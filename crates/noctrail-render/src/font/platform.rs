pub(crate) fn default_font_fallback_families(
    _linux_default: &'static [&'static str],
) -> &'static [&'static str] {
    #[cfg(target_os = "macos")]
    {
        &[
            "Menlo",
            "PingFang SC",
            "Apple Color Emoji",
            "Segoe UI Emoji",
        ]
    }

    #[cfg(target_os = "windows")]
    {
        &["Microsoft YaHei UI", "Segoe UI Emoji", "Noto Color Emoji"]
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        _linux_default
    }
}

pub(crate) fn preferred_system_monospace_families() -> &'static [&'static str] {
    #[cfg(target_os = "macos")]
    {
        &["SF Mono", "Menlo", "Monaco"]
    }

    #[cfg(windows)]
    {
        &["Cascadia Mono", "Consolas", "Lucida Console"]
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    {
        &["Noto Sans Mono", "DejaVu Sans Mono", "Liberation Mono"]
    }
}
