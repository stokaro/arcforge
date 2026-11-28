//! Native window decoration belongs to the OS; the workspace paints its content.

pub(crate) fn native_options() -> eframe::NativeOptions {
    let viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([900.0, 440.0])
        .with_app_id("org.stokaro.arcforge")
        .with_icon(eframe::egui::IconData::default())
        .with_decorations(true);

    // egui-winit maps these to NSWindow's transparent titlebar and full-size
    // content view. Keep the real close/minimize/zoom controls and native window
    // semantics; ui-egui reserves their leading space in the integrated header.
    #[cfg(target_os = "macos")]
    let viewport = viewport
        .with_fullsize_content_view(true)
        .with_titlebar_shown(false)
        .with_title_shown(false)
        .with_titlebar_buttons_shown(true)
        .with_movable_by_background(false);

    eframe::NativeOptions {
        viewport,
        ..Default::default()
    }
}
