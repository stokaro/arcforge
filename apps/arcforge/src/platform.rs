//! Native window decoration belongs to the OS; the workspace paints its content.

use arcforge_ui_egui::ArchiveWorkspace;

/// The host owns the native adapter for the same lifetime as the desktop app.
/// Other platforms retain the portable toolbar and their real OS decorations.
#[derive(Default)]
pub(crate) struct NativeShell {
    #[cfg(target_os = "macos")]
    attempted: bool,
    #[cfg(target_os = "macos")]
    inner: Option<arcforge_platform_macos::NativeShell>,
}

impl NativeShell {
    /// Install once, then deliver native intents before rendering the workspace.
    pub(crate) fn before_frame(
        &mut self,
        workspace: &mut ArchiveWorkspace,
        context: &eframe::egui::Context,
        frame: &eframe::Frame,
    ) {
        #[cfg(target_os = "macos")]
        {
            if !self.attempted {
                self.attempted = true;
                let repaint = context.clone();
                match arcforge_platform_macos::NativeShell::attach(frame, move || {
                    repaint.request_repaint()
                }) {
                    Ok(shell) => {
                        self.inner = Some(shell);
                        workspace.set_native_toolbar_attached(true);
                    }
                    Err(error) => {
                        eprintln!("arcforge: native macOS shell unavailable: {error}");
                    }
                }
            }
            if let Some(shell) = &self.inner {
                for command in shell.drain_commands() {
                    if command == arcforge_platform_macos::Command::FocusSearch {
                        shell.focus_search();
                    } else {
                        workspace.dispatch_command(workspace_command(command), context);
                    }
                }
                shell.update(&native_state(workspace.toolbar_state()));
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (workspace, context, frame);
    }

    /// Jobs and the portable content may have changed capabilities this frame.
    pub(crate) fn after_frame(&self, workspace: &ArchiveWorkspace) {
        #[cfg(target_os = "macos")]
        if let Some(shell) = &self.inner {
            shell.update(&native_state(workspace.toolbar_state()));
        }
        #[cfg(not(target_os = "macos"))]
        let _ = workspace;
    }

    #[cfg(feature = "native-capture")]
    pub(crate) fn window_id(&self) -> Option<u32> {
        #[cfg(target_os = "macos")]
        {
            self.inner.as_ref()?.window_id()
        }
        #[cfg(not(target_os = "macos"))]
        None
    }

    #[cfg(feature = "native-capture")]
    pub(crate) fn active_open_panel(&self) -> Option<OpenPanelInfo> {
        #[cfg(target_os = "macos")]
        {
            let panel = self.inner.as_ref()?.active_open_panel()?;
            Some(OpenPanelInfo {
                window_id: panel.window_id,
                parent_window_id: panel.parent_window_id,
            })
        }
        #[cfg(not(target_os = "macos"))]
        None
    }

    #[cfg(feature = "native-capture")]
    pub(crate) fn cancel_open_panel(&self, expected_window_id: u32) -> bool {
        #[cfg(target_os = "macos")]
        {
            self.inner
                .as_ref()
                .is_some_and(|shell| shell.cancel_open_panel(expected_window_id))
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = expected_window_id;
            false
        }
    }
}

#[cfg(feature = "native-capture")]
pub(crate) struct OpenPanelInfo {
    pub window_id: u32,
    pub parent_window_id: u32,
}

#[cfg(target_os = "macos")]
fn workspace_command(
    command: arcforge_platform_macos::Command,
) -> arcforge_ui_egui::WorkspaceCommand {
    use arcforge_platform_macos::Command as Native;
    use arcforge_ui_egui::WorkspaceCommand as Workspace;
    match command {
        Native::OpenArchive => Workspace::OpenArchive,
        Native::OpenByPath => Workspace::OpenByPath,
        Native::OpenSettings => Workspace::OpenSettings,
        Native::CloseArchive => Workspace::CloseArchive,
        Native::ExtractAll => Workspace::ExtractAll,
        Native::ExtractSelected => Workspace::ExtractSelected,
        Native::TestArchive => Workspace::TestArchive,
        Native::PreviewSelected => Workspace::PreviewSelected,
        Native::ViewOutline => Workspace::ViewOutline,
        Native::ViewFlat => Workspace::ViewFlat,
        Native::GoBack => Workspace::GoBack,
        Native::GoForward => Workspace::GoForward,
        Native::ToggleFavorites => Workspace::ToggleFavorites,
        Native::ToggleInspector => Workspace::ToggleInspector,
        Native::FocusSearch => Workspace::FocusSearch,
        Native::SearchQuery(query) => Workspace::SearchQuery(query),
        Native::SelectAll => Workspace::SelectAll,
        Native::Cut => Workspace::Cut,
        Native::Copy => Workspace::Copy,
        Native::Paste => Workspace::Paste,
        Native::CancelOperation => Workspace::CancelOperation,
    }
}

#[cfg(target_os = "macos")]
fn native_state(state: arcforge_ui_egui::ToolbarState) -> arcforge_platform_macos::State {
    arcforge_platform_macos::State {
        title: state.title,
        can_open: state.can_open,
        can_close: state.can_close,
        can_extract_all: state.can_extract_all,
        can_extract_selected: state.can_extract_selected,
        can_test: state.can_test,
        can_preview: state.can_preview,
        can_select_all: state.can_select_all,
        can_cancel: state.can_cancel,
        can_search: state.can_search,
        can_change_view: state.can_change_view,
        can_go_back: state.can_go_back,
        can_go_forward: state.can_go_forward,
        favorites_visible: state.favorites_visible,
        inspector_visible: state.inspector_visible,
        outline_view: state.outline_view,
        search_query: state.search_query,
    }
}

pub(crate) fn native_options() -> eframe::NativeOptions {
    let viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([900.0, 440.0])
        .with_app_id("org.stokaro.arcforge")
        .with_icon(eframe::egui::IconData::default())
        .with_decorations(true);

    // AppKit owns the titlebar and toolbar. Keep egui's content below their
    // layout rectangle instead of painting an imitation over native controls.
    #[cfg(target_os = "macos")]
    let viewport = viewport
        .with_fullsize_content_view(false)
        .with_titlebar_shown(true)
        .with_title_shown(true)
        .with_titlebar_buttons_shown(true)
        .with_movable_by_background(false);

    eframe::NativeOptions {
        viewport,
        ..Default::default()
    }
}
