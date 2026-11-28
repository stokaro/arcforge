//! Audited AppKit ABI/target-action boundary. See ../README.md.

use std::cell::{OnceCell, RefCell};
use std::collections::VecDeque;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
#[cfg(feature = "native-capture")]
use objc2_app_kit::NSOpenPanel;
use objc2_app_kit::{
    NSApplication, NSEventModifierFlags, NSImage, NSMenu, NSMenuItem, NSMenuToolbarItem,
    NSSearchField, NSSearchToolbarItem, NSToolbar, NSToolbarDelegate, NSToolbarDisplayMode,
    NSToolbarFlexibleSpaceItemIdentifier, NSToolbarItem, NSToolbarItemGroup,
    NSToolbarItemGroupControlRepresentation, NSToolbarItemIdentifier, NSWindow,
    NSWindowTitleVisibility, NSWindowToolbarStyle,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSObject, NSObjectProtocol, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

#[cfg(feature = "native-capture")]
use crate::OpenPanelInfo;
use crate::{AttachError, Command, State};

const SIDEBAR: &str = "org.stokaro.arcforge.sidebar";
const ARCHIVE_GROUP: &str = "org.stokaro.arcforge.archive";
const EDIT_GROUP: &str = "org.stokaro.arcforge.edit";
const MODE_GROUP: &str = "org.stokaro.arcforge.mode";
const SHARE: &str = "org.stokaro.arcforge.share";
const SEARCH: &str = "org.stokaro.arcforge.search";
const INSPECTOR: &str = "org.stokaro.arcforge.inspector";

#[derive(Clone, Copy)]
#[repr(isize)]
enum Action {
    Unavailable = 0,
    Open = 1,
    Close,
    ExtractAll,
    ExtractSelected,
    Test,
    Outline,
    Flat,
    Favorites,
    Inspector,
    Search,
    SelectAll,
    Cut,
    Copy,
    Paste,
    Cancel,
    IconsOnly,
    IconsAndLabels,
    Preview,
    OpenByPath,
    Back,
    Forward,
    Settings,
}

impl Action {
    fn from_tag(tag: isize) -> Option<Self> {
        Some(match tag {
            0 => Self::Unavailable,
            1 => Self::Open,
            2 => Self::Close,
            3 => Self::ExtractAll,
            4 => Self::ExtractSelected,
            5 => Self::Test,
            6 => Self::Outline,
            7 => Self::Flat,
            8 => Self::Favorites,
            9 => Self::Inspector,
            10 => Self::Search,
            11 => Self::SelectAll,
            12 => Self::Cut,
            13 => Self::Copy,
            14 => Self::Paste,
            15 => Self::Cancel,
            16 => Self::IconsOnly,
            17 => Self::IconsAndLabels,
            18 => Self::Preview,
            19 => Self::OpenByPath,
            20 => Self::Back,
            21 => Self::Forward,
            22 => Self::Settings,
            _ => return None,
        })
    }

    fn enabled(self, state: &State) -> bool {
        match self {
            Self::Unavailable => false,
            Self::Open | Self::OpenByPath | Self::Settings => state.can_open,
            Self::Close => state.can_close,
            Self::ExtractAll => state.can_extract_all,
            Self::ExtractSelected => state.can_extract_selected,
            Self::Test => state.can_test,
            Self::Preview => state.can_preview,
            Self::Outline | Self::Flat => state.can_change_view,
            Self::Search => state.can_search,
            // The Rust dispatcher may have a text editor focused even when an
            // archive selection is unavailable; it owns that distinction.
            Self::SelectAll => true,
            Self::Back => state.can_go_back,
            Self::Forward => state.can_go_forward,
            Self::Cancel => state.can_cancel,
            Self::Favorites
            | Self::Inspector
            | Self::Cut
            | Self::Copy
            | Self::Paste
            | Self::IconsOnly
            | Self::IconsAndLabels => true,
        }
    }

    fn command(self) -> Option<Command> {
        Some(match self {
            Self::Open => Command::OpenArchive,
            Self::OpenByPath => Command::OpenByPath,
            Self::Settings => Command::OpenSettings,
            Self::Close => Command::CloseArchive,
            Self::ExtractAll => Command::ExtractAll,
            Self::ExtractSelected => Command::ExtractSelected,
            Self::Test => Command::TestArchive,
            Self::Preview => Command::PreviewSelected,
            Self::Outline => Command::ViewOutline,
            Self::Flat => Command::ViewFlat,
            Self::Back => Command::GoBack,
            Self::Forward => Command::GoForward,
            Self::Favorites => Command::ToggleFavorites,
            Self::Inspector => Command::ToggleInspector,
            Self::Search => Command::FocusSearch,
            Self::SelectAll => Command::SelectAll,
            Self::Cut => Command::Cut,
            Self::Copy => Command::Copy,
            Self::Paste => Command::Paste,
            Self::Cancel => Command::CancelOperation,
            Self::Unavailable | Self::IconsOnly | Self::IconsAndLabels => return None,
        })
    }

    fn edit_selector(self) -> Option<Sel> {
        match self {
            Self::Cut => Some(sel!(cut:)),
            Self::Copy => Some(sel!(copy:)),
            Self::Paste => Some(sel!(paste:)),
            Self::SelectAll => Some(sel!(selectAll:)),
            _ => None,
        }
    }
}

struct TargetIvars {
    queue: RefCell<VecDeque<Command>>,
    state: RefCell<State>,
    wake: Box<dyn Fn()>,
    toolbar: OnceCell<Retained<NSToolbar>>,
    menu_items: RefCell<Vec<(Action, Retained<NSMenuItem>)>>,
}

define_class!(
    // SAFETY: NSObject has no subclass invariants; this class has no custom Drop.
    // MainThreadOnly prevents AppKit state and callbacks crossing thread bounds.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = TargetIvars]
    struct ShellTarget;

    // SAFETY: NSObjectProtocol imposes no additional safety requirements.
    unsafe impl NSObjectProtocol for ShellTarget {}

    // SAFETY: These signatures exactly match pinned NSToolbarDelegate. Each
    // request constructs a fresh item; no AppKit lifecycle delegate is replaced.
    unsafe impl NSToolbarDelegate for ShellTarget {
        #[unsafe(method_id(toolbar:itemForItemIdentifier:willBeInsertedIntoToolbar:))]
        fn toolbar_item(
            &self,
            _toolbar: &NSToolbar,
            identifier: &NSToolbarItemIdentifier,
            _inserted: bool,
        ) -> Option<Retained<NSToolbarItem>> {
            self.make_item(&identifier.to_string())
        }

        #[unsafe(method_id(toolbarDefaultItemIdentifiers:))]
        fn default_identifiers(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSString>> {
            default_identifiers()
        }

        #[unsafe(method_id(toolbarAllowedItemIdentifiers:))]
        fn allowed_identifiers(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSString>> {
            default_identifiers()
        }
    }

    impl ShellTarget {
        // SAFETY: A target-action receives one Objective-C object. A toolbar
        // overflow representation may send an NSMenuItem instead of the toolbar
        // item, so the object is checked before using either concrete API.
        #[unsafe(method(runToolbarCommand:))]
        fn toolbar_command(&self, sender: &AnyObject) {
            let tag = sender.downcast_ref::<NSToolbarItem>().map(|item| item.tag())
                .or_else(|| sender.downcast_ref::<NSMenuItem>().map(|item| item.tag()));
            if let Some(action) = tag.and_then(Action::from_tag) {
                self.perform(action);
            }
        }

        // SAFETY: Installed only on NSMenuItem target-actions below.
        #[unsafe(method(runMenuCommand:))]
        fn menu_command(&self, sender: &NSMenuItem) {
            let Some(action) = Action::from_tag(sender.tag()) else {
                return;
            };
            if let Some(selector) = action.edit_selector() {
                // SAFETY: These are standard one-sender AppKit editing actions.
                // A nil target uses the native responder chain, which handles
                // an active search field or file-panel editor before egui.
                let handled = unsafe {
                    NSApplication::sharedApplication(self.mtm())
                        .sendAction_to_from(selector, None, Some(sender))
                };
                if handled {
                    return;
                }
            }
            self.perform(action);
        }

        // SAFETY: Installed only on our NSSearchField target-action below.
        #[unsafe(method(searchChanged:))]
        fn search_changed(&self, sender: &NSSearchField) {
            if self.ivars().state.borrow().can_search {
                self.enqueue(Command::SearchQuery(sender.stringValue().to_string()));
            }
        }
    }
);

impl ShellTarget {
    fn new(mtm: MainThreadMarker, wake: impl Fn() + 'static) -> Retained<Self> {
        let allocated = Self::alloc(mtm).set_ivars(TargetIvars {
            queue: RefCell::new(VecDeque::new()),
            state: RefCell::new(State::default()),
            wake: Box::new(wake),
            toolbar: OnceCell::new(),
            menu_items: RefCell::new(Vec::new()),
        });
        // SAFETY: NSObject.init has this exact signature; ivars are initialized
        // before the superclass initializer can expose the object to AppKit.
        unsafe { msg_send![super(allocated), init] }
    }

    fn enqueue(&self, command: Command) {
        {
            let mut queue = self.ivars().queue.borrow_mut();
            // Native typing may deliver several changes before a renderer frame.
            // Keep their final text without reordering unrelated user commands.
            if matches!(command, Command::SearchQuery(_))
                && matches!(queue.back(), Some(Command::SearchQuery(_)))
            {
                queue.pop_back();
            }
            queue.push_back(command);
        }
        (self.ivars().wake)();
    }

    fn perform(&self, action: Action) {
        if !action.enabled(&self.ivars().state.borrow()) {
            return;
        }
        if let Some(toolbar) = self.ivars().toolbar.get() {
            let display = match action {
                Action::IconsOnly => Some(NSToolbarDisplayMode::IconOnly),
                Action::IconsAndLabels => Some(NSToolbarDisplayMode::IconAndLabel),
                _ => None,
            };
            if let Some(display) = display {
                toolbar.setDisplayMode(display);
                (self.ivars().wake)();
                return;
            }
        }
        if let Some(command) = action.command() {
            self.enqueue(command);
        }
    }

    fn make_item(&self, identifier: &str) -> Option<Retained<NSToolbarItem>> {
        let item = match identifier {
            SIDEBAR => {
                let item = self.button(SIDEBAR, "Favorites", "sidebar.left", Action::Favorites);
                item.setNavigational(true);
                item
            }
            ARCHIVE_GROUP => self.group(
                ARCHIVE_GROUP,
                "Archive",
                &[
                    self.extract_item(),
                    self.button("save", "Save", "doc.zipper", Action::Unavailable),
                    self.button("test", "Test", "checkmark.square", Action::Test),
                    self.button(
                        "move",
                        "Move",
                        "folder.badge.arrowshape.right",
                        Action::Unavailable,
                    ),
                ],
            ),
            EDIT_GROUP => self.group(
                EDIT_GROUP,
                "Files",
                &[
                    self.button("add", "Add", "archivebox", Action::Unavailable),
                    self.button("view", "View", "doc.text.magnifyingglass", Action::Preview),
                    self.button("delete", "Delete", "trash", Action::Unavailable),
                    self.button(
                        "new-folder",
                        "New Folder",
                        "folder.badge.plus",
                        Action::Unavailable,
                    ),
                ],
            ),
            MODE_GROUP => self.group(
                MODE_GROUP,
                "Modes",
                &[
                    self.button(
                        "collect",
                        "Collect",
                        "tray.and.arrow.down",
                        Action::Unavailable,
                    ),
                    self.button(
                        "direct",
                        "Direct Mode",
                        "archivebox.fill",
                        Action::Unavailable,
                    ),
                ],
            ),
            SHARE => self.button(SHARE, "Share", "square.and.arrow.up", Action::Unavailable),
            SEARCH => self.search_item(),
            INSPECTOR => self.button(INSPECTOR, "Inspector", "sidebar.right", Action::Inspector),
            _ => return None,
        };
        update_toolbar_item(&item, &self.ivars().state.borrow());
        Some(item)
    }

    fn button(
        &self,
        identifier: &str,
        title: &str,
        symbol: &str,
        action: Action,
    ) -> Retained<NSToolbarItem> {
        let item = NSToolbarItem::initWithItemIdentifier(
            NSToolbarItem::alloc(self.mtm()),
            &NSString::from_str(identifier),
        );
        configure_item(&item, title, symbol, action);
        // SAFETY: ShellTarget is retained by NativeShell until all installed
        // controls are detached. The selector accepts one checked ObjC sender.
        unsafe {
            item.setTarget(Some(self));
            item.setAction(Some(sel!(runToolbarCommand:)));
        }
        // Supply a separately typed menu action and explicit tag for AppKit's
        // narrow-window overflow instead of relying on its generated defaults.
        let overflow = self.make_menu_item(title, "", action);
        item.setMenuFormRepresentation(Some(&overflow));
        item
    }

    fn group(
        &self,
        identifier: &str,
        label: &str,
        items: &[Retained<NSToolbarItem>],
    ) -> Retained<NSToolbarItem> {
        let group = NSToolbarItemGroup::initWithItemIdentifier(
            NSToolbarItemGroup::alloc(self.mtm()),
            &NSString::from_str(identifier),
        );
        group.setLabel(&NSString::from_str(label));
        group.setPaletteLabel(&NSString::from_str(label));
        group.setSubitems(&NSArray::from_retained_slice(items));
        group.setControlRepresentation(NSToolbarItemGroupControlRepresentation::Expanded);
        group.setAutovalidates(false);
        group.into_super()
    }

    fn extract_item(&self) -> Retained<NSToolbarItem> {
        let item = NSMenuToolbarItem::initWithItemIdentifier(
            NSMenuToolbarItem::alloc(self.mtm()),
            &NSString::from_str("extract"),
        );
        configure_item(
            &item,
            "Extract",
            "archivebox.and.arrow.up",
            Action::ExtractAll,
        );
        item.setShowsIndicator(true);
        let menu = new_menu(self.mtm(), "Extract");
        self.add_menu_item(&menu, "Extract All", "", Action::Unavailable);
        for destination in ["Downloads", "Documents", "Desktop"] {
            let destination = self.add_menu_item(&menu, destination, "", Action::Unavailable);
            destination.setToolTip(Some(&NSString::from_str("Quick destinations are planned")));
        }
        self.add_menu_item(&menu, "Other Folder…", "r", Action::ExtractAll);
        menu.addItem(&NSMenuItem::separatorItem(self.mtm()));
        self.add_menu_item(&menu, "Extract Selected…", "", Action::ExtractSelected);
        menu.addItem(&NSMenuItem::separatorItem(self.mtm()));
        self.add_menu_item(&menu, "Extract with Preset", "", Action::Unavailable);
        for preset in ["Extract & Trash", "Extract with Cleaning", "Edit Presets…"] {
            let preset = self.add_menu_item(&menu, preset, "", Action::Unavailable);
            preset.setToolTip(Some(&NSString::from_str("Extraction presets are planned")));
        }
        item.setMenu(&menu);
        item.into_super()
    }

    fn search_item(&self) -> Retained<NSToolbarItem> {
        let item = NSSearchToolbarItem::initWithItemIdentifier(
            NSSearchToolbarItem::alloc(self.mtm()),
            &NSString::from_str(SEARCH),
        );
        item.setLabel(&NSString::from_str("Search"));
        item.setTag(Action::Search as isize);
        item.setAutovalidates(false);
        item.setPreferredWidthForSearchField(220.0);
        let field = item.searchField();
        field.setPlaceholderString(Some(&NSString::from_str("Search")));
        field.setSendsWholeSearchString(false);
        field.setSendsSearchStringImmediately(true);
        // SAFETY: This field is the only sender installed for searchChanged:;
        // the target is retained for at least as long as the toolbar.
        unsafe {
            field.setTarget(Some(self));
            field.setAction(Some(sel!(searchChanged:)));
        }
        item.into_super()
    }

    fn add_menu_item(
        &self,
        menu: &NSMenu,
        title: &str,
        key: &str,
        action: Action,
    ) -> Retained<NSMenuItem> {
        let item = self.make_menu_item(title, key, action);
        menu.addItem(&item);
        item
    }

    fn make_menu_item(&self, title: &str, key: &str, action: Action) -> Retained<NSMenuItem> {
        // SAFETY: runMenuCommand: is the method above with an NSMenuItem sender.
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(self.mtm()),
                &NSString::from_str(title),
                Some(sel!(runMenuCommand:)),
                &NSString::from_str(key),
            )
        };
        item.setTag(action as isize);
        item.setEnabled(
            action.edit_selector().is_some() || action.enabled(&self.ivars().state.borrow()),
        );
        // SAFETY: The retained ShellTarget implements this item's exact action.
        unsafe { item.setTarget(Some(self)) };
        self.ivars()
            .menu_items
            .borrow_mut()
            .push((action, item.clone()));
        item
    }
}

fn configure_item(item: &NSToolbarItem, title: &str, symbol: &str, action: Action) {
    let title = NSString::from_str(title);
    item.setLabel(&title);
    item.setPaletteLabel(&title);
    item.setToolTip(Some(&title));
    item.setTag(action as isize);
    item.setAutovalidates(false);
    item.setBordered(true);
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(symbol),
        Some(&title),
    );
    if let Some(image) = image {
        item.setImage(Some(&image));
    } else {
        // A symbol may be absent on an older installed macOS. Preserve a
        // usable native text item instead of an invisible or invented icon.
        item.setTitle(&title);
    }
}

fn default_identifiers() -> Retained<NSArray<NSString>> {
    // SAFETY: AppKit exports this immutable NSString constant for all supported
    // macOS versions. Reading it does not transfer or extend a raw lifetime.
    let flexible = unsafe { NSToolbarFlexibleSpaceItemIdentifier };
    NSArray::from_retained_slice(&[
        NSString::from_str(SIDEBAR),
        NSString::from_str(&flexible.to_string()),
        NSString::from_str(ARCHIVE_GROUP),
        NSString::from_str(EDIT_GROUP),
        NSString::from_str(MODE_GROUP),
        NSString::from_str(SHARE),
        NSString::from_str(SEARCH),
        NSString::from_str(INSPECTOR),
    ])
}

fn new_menu(mtm: MainThreadMarker, title: &str) -> Retained<NSMenu> {
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title));
    menu.setAutoenablesItems(false);
    menu
}

fn add_submenu(parent: &NSMenu, title: &str, submenu: &NSMenu, mtm: MainThreadMarker) {
    // SAFETY: A submenu header has no action selector.
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            None,
            &NSString::new(),
        )
    };
    item.setSubmenu(Some(submenu));
    parent.addItem(&item);
}

fn add_standard(
    menu: &NSMenu,
    title: &str,
    key: &str,
    selector: Sel,
    mtm: MainThreadMarker,
) -> Retained<NSMenuItem> {
    // SAFETY: Call sites supply only documented, one-sender AppKit action
    // selectors. A nil target uses the standard native responder chain.
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            Some(selector),
            &NSString::from_str(key),
        )
    };
    menu.addItem(&item);
    item
}

fn build_menus(target: &ShellTarget) -> (Retained<NSMenu>, Retained<NSMenu>) {
    let mtm = target.mtm();
    let main = new_menu(mtm, "ArcForge");
    let application = new_menu(mtm, "ArcForge");
    add_standard(
        &application,
        "About ArcForge",
        "",
        sel!(orderFrontStandardAboutPanel:),
        mtm,
    );
    application.addItem(&NSMenuItem::separatorItem(mtm));
    target.add_menu_item(&application, "Settings…", ",", Action::Settings);
    application.addItem(&NSMenuItem::separatorItem(mtm));
    add_standard(&application, "Hide ArcForge", "h", sel!(hide:), mtm);
    let hide_others = add_standard(
        &application,
        "Hide Others",
        "h",
        sel!(hideOtherApplications:),
        mtm,
    );
    hide_others
        .setKeyEquivalentModifierMask(NSEventModifierFlags::Command | NSEventModifierFlags::Option);
    add_standard(
        &application,
        "Show All",
        "",
        sel!(unhideAllApplications:),
        mtm,
    );
    application.addItem(&NSMenuItem::separatorItem(mtm));
    add_standard(&application, "Quit ArcForge", "q", sel!(terminate:), mtm);
    add_submenu(&main, "ArcForge", &application, mtm);

    let file = new_menu(mtm, "File");
    target.add_menu_item(&file, "Open…", "o", Action::Open);
    target.add_menu_item(&file, "Open by Path…", "", Action::OpenByPath);
    target.add_menu_item(&file, "Close Archive", "w", Action::Close);
    file.addItem(&NSMenuItem::separatorItem(mtm));
    target.add_menu_item(&file, "Extract All…", "r", Action::ExtractAll);
    let selected = target.add_menu_item(&file, "Extract Selected…", "r", Action::ExtractSelected);
    selected
        .setKeyEquivalentModifierMask(NSEventModifierFlags::Command | NSEventModifierFlags::Shift);
    target.add_menu_item(&file, "Save", "s", Action::Unavailable);
    target.add_menu_item(&file, "Test Archive", "", Action::Test);
    file.addItem(&NSMenuItem::separatorItem(mtm));
    target.add_menu_item(&file, "Cancel Operation", ".", Action::Cancel);
    add_submenu(&main, "File", &file, mtm);

    let edit = new_menu(mtm, "Edit");
    target.add_menu_item(&edit, "Cut", "x", Action::Cut);
    target.add_menu_item(&edit, "Copy", "c", Action::Copy);
    target.add_menu_item(&edit, "Paste", "v", Action::Paste);
    target.add_menu_item(&edit, "Select All", "a", Action::SelectAll);
    edit.addItem(&NSMenuItem::separatorItem(mtm));
    target.add_menu_item(&edit, "Find…", "f", Action::Search);
    add_submenu(&main, "Edit", &edit, mtm);

    let view = new_menu(mtm, "View");
    target.add_menu_item(&view, "as Outline", "2", Action::Outline);
    target.add_menu_item(&view, "as List", "1", Action::Flat);
    view.addItem(&NSMenuItem::separatorItem(mtm));
    target.add_menu_item(&view, "Show Favorites", "b", Action::Favorites);
    target.add_menu_item(&view, "Show Inspector", "i", Action::Inspector);
    view.addItem(&NSMenuItem::separatorItem(mtm));
    target.add_menu_item(&view, "Toolbar Icons Only", "", Action::IconsOnly);
    target.add_menu_item(&view, "Toolbar Icons and Text", "", Action::IconsAndLabels);
    add_submenu(&main, "View", &view, mtm);

    let go = new_menu(mtm, "Go");
    target.add_menu_item(&go, "Back", "[", Action::Back);
    target.add_menu_item(&go, "Forward", "]", Action::Forward);
    add_submenu(&main, "Go", &go, mtm);

    let window = new_menu(mtm, "Window");
    add_standard(&window, "Minimize", "m", sel!(performMiniaturize:), mtm);
    add_standard(&window, "Zoom", "", sel!(performZoom:), mtm);
    window.addItem(&NSMenuItem::separatorItem(mtm));
    add_standard(
        &window,
        "Bring All to Front",
        "",
        sel!(arrangeInFront:),
        mtm,
    );
    add_submenu(&main, "Window", &window, mtm);

    let help = new_menu(mtm, "Help");
    target.add_menu_item(&help, "ArcForge Help", "", Action::Unavailable);
    add_submenu(&main, "Help", &help, mtm);
    (main, window)
}

fn update_toolbar_item(item: &NSToolbarItem, state: &State) {
    if let Some(group) = item.downcast_ref::<NSToolbarItemGroup>() {
        for subitem in group.subitems() {
            update_toolbar_item(&subitem, state);
        }
        return;
    }
    if let Some(action) = Action::from_tag(item.tag()) {
        let enabled = if item.downcast_ref::<NSMenuToolbarItem>().is_some() {
            state.can_extract_all || state.can_extract_selected
        } else {
            action.enabled(state)
        };
        item.setEnabled(enabled);
    }
    if let Some(search) = item.downcast_ref::<NSSearchToolbarItem>() {
        let field = search.searchField();
        if field.stringValue().to_string() != state.search_query {
            field.setStringValue(&NSString::from_str(&state.search_query));
        }
    }
}

/// Owns the exact desktop window's native presentation. Main-thread-only by its
/// retained AppKit objects; construction never dereferences a raw handle.
pub struct NativeShell {
    window: Retained<NSWindow>,
    toolbar: Retained<NSToolbar>,
    target: Retained<ShellTarget>,
    main_menu: Retained<NSMenu>,
    previous_menu: Option<Retained<NSMenu>>,
    previous_windows_menu: Option<Retained<NSMenu>>,
    previous_toolbar: Option<Retained<NSToolbar>>,
    previous_toolbar_style: NSWindowToolbarStyle,
    previous_title_visibility: NSWindowTitleVisibility,
    previous_titlebar_transparent: bool,
}

impl NativeShell {
    pub fn attach<W: HasWindowHandle + ?Sized>(
        window: &W,
        wake: impl Fn() + 'static,
    ) -> Result<Self, AttachError> {
        let mtm = MainThreadMarker::new().ok_or(AttachError::NotMainThread)?;
        if !objc2::available!(macos = 11.0) {
            return Err(AttachError::UnsupportedSystem);
        }
        let borrowed = window
            .window_handle()
            .map_err(|_| AttachError::UnavailableWindowHandle)?;
        let RawWindowHandle::AppKit(handle) = borrowed.as_raw() else {
            return Err(AttachError::NotAppKitWindow);
        };
        let app = NSApplication::sharedApplication(mtm);
        let window = app
            .windows()
            .into_iter()
            .find(|window| {
                window.contentView().is_some_and(|view| {
                    Retained::as_ptr(&view).cast::<std::ffi::c_void>()
                        == handle.ns_view.as_ptr().cast_const()
                })
            })
            .ok_or(AttachError::WindowNotFound)?;
        let previous_menu = app.mainMenu();
        let previous_windows_menu = app.windowsMenu();
        let previous_toolbar = window.toolbar();
        let previous_toolbar_style = window.toolbarStyle();
        let previous_title_visibility = window.titleVisibility();
        let previous_titlebar_transparent = window.titlebarAppearsTransparent();

        let target = ShellTarget::new(mtm, wake);
        let toolbar = NSToolbar::initWithIdentifier(
            NSToolbar::alloc(mtm),
            &NSString::from_str("org.stokaro.arcforge.main-toolbar"),
        );
        let _ = target.ivars().toolbar.set(toolbar.clone());
        toolbar.setDelegate(Some(ProtocolObject::from_ref(&*target)));
        toolbar.setDisplayMode(NSToolbarDisplayMode::IconOnly);
        toolbar.setAllowsUserCustomization(false);
        toolbar.setAutosavesConfiguration(false);
        window.setTitleVisibility(NSWindowTitleVisibility::Visible);
        window.setTitlebarAppearsTransparent(false);
        window.setToolbarStyle(NSWindowToolbarStyle::Unified);
        window.setToolbar(Some(&toolbar));
        toolbar.setVisible(true);

        let (main_menu, windows_menu) = build_menus(&target);
        app.setMainMenu(Some(&main_menu));
        app.setWindowsMenu(Some(&windows_menu));
        let shell = Self {
            window,
            toolbar,
            target,
            main_menu,
            previous_menu,
            previous_windows_menu,
            previous_toolbar,
            previous_toolbar_style,
            previous_title_visibility,
            previous_titlebar_transparent,
        };
        shell.update(&State::default());
        Ok(shell)
    }

    pub fn drain_commands(&self) -> Vec<Command> {
        self.target.ivars().queue.borrow_mut().drain(..).collect()
    }

    pub fn update(&self, state: &State) {
        *self.target.ivars().state.borrow_mut() = state.clone();
        if self.window.title().to_string() != state.title {
            self.window.setTitle(&NSString::from_str(&state.title));
        }
        for item in self.toolbar.items() {
            update_toolbar_item(&item, state);
        }
        // Clone retained handles before native calls so an AppKit callback can
        // enqueue commands without a reentrant borrow of the menu collection.
        let items = self.target.ivars().menu_items.borrow().clone();
        for (action, item) in items {
            // Keep native text editing available in an Open sheet even while
            // the archive workspace itself cannot change its selection.
            item.setEnabled(action.edit_selector().is_some() || action.enabled(state));
            let checked = match action {
                Action::Outline => state.outline_view,
                Action::Flat => !state.outline_view,
                Action::Favorites => state.favorites_visible,
                Action::Inspector => state.inspector_visible,
                Action::IconsOnly => self.toolbar.displayMode() == NSToolbarDisplayMode::IconOnly,
                Action::IconsAndLabels => {
                    self.toolbar.displayMode() == NSToolbarDisplayMode::IconAndLabel
                }
                _ => false,
            };
            item.setState(if checked { 1 } else { 0 });
            match action {
                Action::Favorites => {
                    item.setTitle(&NSString::from_str(if state.favorites_visible {
                        "Hide Favorites"
                    } else {
                        "Show Favorites"
                    }))
                }
                Action::Inspector => {
                    item.setTitle(&NSString::from_str(if state.inspector_visible {
                        "Hide Inspector"
                    } else {
                        "Show Inspector"
                    }))
                }
                _ => {}
            }
        }
    }

    pub fn focus_search(&self) {
        if !self.target.ivars().state.borrow().can_search {
            return;
        }
        for item in self.toolbar.items() {
            if let Some(search) = item.downcast_ref::<NSSearchToolbarItem>() {
                search.beginSearchInteraction();
                return;
            }
        }
    }

    #[cfg(feature = "native-capture")]
    pub fn window_id(&self) -> Option<u32> {
        u32::try_from(self.window.windowNumber())
            .ok()
            .filter(|id| *id != 0)
    }

    #[cfg(feature = "native-capture")]
    fn owned_open_panel(&self) -> Option<Retained<NSOpenPanel>> {
        let sheet = self
            .window
            .attachedSheet()?
            .downcast::<NSOpenPanel>()
            .ok()?;
        let parent = sheet.sheetParent()?;
        std::ptr::eq(&*parent, &*self.window).then_some(sheet)
    }

    #[cfg(feature = "native-capture")]
    pub fn active_open_panel(&self) -> Option<OpenPanelInfo> {
        let panel = self.owned_open_panel()?;
        let window_id = u32::try_from(panel.windowNumber())
            .ok()
            .filter(|id| *id != 0)?;
        Some(OpenPanelInfo {
            window_id,
            parent_window_id: self.window_id()?,
        })
    }

    #[cfg(feature = "native-capture")]
    pub fn cancel_open_panel(&self, expected_window_id: u32) -> bool {
        let Some(panel) = self.owned_open_panel() else {
            return false;
        };
        if expected_window_id == 0
            || u32::try_from(panel.windowNumber()).ok() != Some(expected_window_id)
        {
            return false;
        }
        // SAFETY: The retained object was checked to be this exact window's
        // attached NSOpenPanel and still has the requested window number. nil is
        // the documented optional sender for the standard cancel: action.
        unsafe { panel.cancel(None) };
        true
    }
}

impl Drop for NativeShell {
    fn drop(&mut self) {
        // Detach while the weak delegate/targets still have a strong owner.
        if self
            .window
            .toolbar()
            .is_some_and(|current| std::ptr::eq(&*current, &*self.toolbar))
        {
            self.window.setToolbar(self.previous_toolbar.as_deref());
            self.window.setToolbarStyle(self.previous_toolbar_style);
            self.window
                .setTitleVisibility(self.previous_title_visibility);
            self.window
                .setTitlebarAppearsTransparent(self.previous_titlebar_transparent);
        }
        self.toolbar.setDelegate(None);
        let app = NSApplication::sharedApplication(self.target.mtm());
        if app
            .mainMenu()
            .is_some_and(|current| std::ptr::eq(&*current, &*self.main_menu))
        {
            app.setMainMenu(self.previous_menu.as_deref());
            app.setWindowsMenu(self.previous_windows_menu.as_deref());
        }
    }
}
