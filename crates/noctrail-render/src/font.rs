use cosmic_text::{FontSystem, fontdb};

mod platform;

struct BundledFontFile {
    bytes: &'static [u8],
}

const BUNDLED_FONT_FILES: [BundledFontFile; 4] = [
    BundledFontFile {
        bytes: include_bytes!(
            "../../../assets/fonts/caskaydia-mono/CaskaydiaMonoNerdFontMono-Regular.ttf"
        ),
    },
    BundledFontFile {
        bytes: include_bytes!(
            "../../../assets/fonts/caskaydia-mono/CaskaydiaMonoNerdFontMono-Bold.ttf"
        ),
    },
    BundledFontFile {
        bytes: include_bytes!(
            "../../../assets/fonts/caskaydia-mono/CaskaydiaMonoNerdFontMono-Italic.ttf"
        ),
    },
    BundledFontFile {
        bytes: include_bytes!(
            "../../../assets/fonts/caskaydia-mono/CaskaydiaMonoNerdFontMono-BoldItalic.ttf"
        ),
    },
];

const DEFAULT_UNIX_FONT_FALLBACKS: &[&str] = &[
    "Noto Sans Mono",
    "Noto Color Emoji",
    "DejaVu Sans Mono",
    "Liberation Mono",
];

pub(crate) fn default_font_family() -> &'static str {
    super::DEFAULT_FONT_FAMILY
}

pub(crate) fn default_font_fallbacks() -> Vec<String> {
    platform::default_font_fallback_families(DEFAULT_UNIX_FONT_FALLBACKS)
        .iter()
        .map(|family| (*family).to_string())
        .collect()
}

pub(crate) fn preferred_system_monospace_families() -> &'static [&'static str] {
    platform::preferred_system_monospace_families()
}

pub(crate) fn configured_font_system() -> FontSystem {
    let locale = FontSystem::new().locale().to_string();
    FontSystem::new_with_locale_and_db(locale, configured_font_database())
}

pub(crate) fn configured_font_database() -> fontdb::Database {
    let mut db = fontdb::Database::new();
    load_bundled_fonts(&mut db);
    db.load_system_fonts();
    db.set_monospace_family(default_font_family());
    db
}

fn load_bundled_fonts(db: &mut fontdb::Database) {
    for font in BUNDLED_FONT_FILES {
        db.load_font_data(font.bytes.to_vec());
    }
}

#[cfg(test)]
pub(crate) fn bundled_font_file_count() -> usize {
    BUNDLED_FONT_FILES.len()
}

#[cfg(test)]
pub(crate) fn bundled_fonts_present() -> bool {
    BUNDLED_FONT_FILES.iter().all(|font| !font.bytes.is_empty())
}
