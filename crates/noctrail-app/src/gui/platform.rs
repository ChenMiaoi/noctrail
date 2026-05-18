use std::path::Path;

use winit::window::WindowAttributes;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(windows))]
mod unix;
#[cfg(windows)]
mod windows;

pub(super) fn configure_window_attributes(attributes: WindowAttributes) -> WindowAttributes {
    #[cfg(target_os = "macos")]
    {
        macos::configure_window_attributes(attributes)
    }

    #[cfg(not(target_os = "macos"))]
    {
        attributes
    }
}

pub(super) fn allows_alt_drag_window() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::allows_alt_drag_window()
    }

    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

pub(super) fn review_output_command(marker: &str) -> String {
    #[cfg(windows)]
    {
        windows::review_output_command(marker)
    }

    #[cfg(not(windows))]
    {
        unix::review_output_command(marker)
    }
}

pub(super) fn review_file_command(path: &Path) -> String {
    #[cfg(windows)]
    {
        windows::review_file_command(path)
    }

    #[cfg(not(windows))]
    {
        unix::review_file_command(path)
    }
}

pub(super) fn review_patch_cli_command(path: &Path) -> Vec<String> {
    #[cfg(windows)]
    {
        windows::review_patch_cli_command(path)
    }

    #[cfg(not(windows))]
    {
        unix::review_patch_cli_command(path)
    }
}
