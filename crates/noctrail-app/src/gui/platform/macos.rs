use winit::{platform::macos::WindowAttributesExtMacOS, window::WindowAttributes};

pub(super) fn configure_window_attributes(attributes: WindowAttributes) -> WindowAttributes {
    attributes
        .with_titlebar_hidden(true)
        .with_fullsize_content_view(true)
        .with_movable_by_window_background(true)
}

pub(super) fn allows_alt_drag_window() -> bool {
    false
}
