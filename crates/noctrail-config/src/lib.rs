//! Configuration boundary for Noctrail.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Deserializer};
use thiserror::Error;

const DEFAULT_THEME_OPACITY: f32 = 1.0;
const DEFAULT_FONT_FAMILY: &str = "JetBrainsMono Nerd Font";
const DEFAULT_FONT_SIZE: f32 = 14.0;
const DEFAULT_FONT_FALLBACKS: [&str; 4] = [
    "Noto Sans CJK SC",
    "Noto Color Emoji",
    "Apple Color Emoji",
    "Segoe UI Emoji",
];
const DEFAULT_CURSOR_BLINK_INTERVAL_MS: u64 = 600;
const DEFAULT_PANE_GAP: u16 = 4;
const DEFAULT_PANE_PADDING: u16 = 2;
const DEFAULT_PANE_RADIUS: u16 = 8;
const DEFAULT_BLUR_FALLBACK_TINT_OPACITY: f32 = 0.92;
const DEFAULT_ANIMATION_DURATION_MS: u64 = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RendererBackend {
    #[default]
    Gpu,
    Software,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(default)]
pub struct RendererConfig {
    pub backend: RendererBackend,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RgbaColor {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

impl RgbaColor {
    pub const fn from_rgba(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }

    pub const fn from_rgb(red: u8, green: u8, blue: u8) -> Self {
        Self::from_rgba(red, green, blue, u8::MAX)
    }

    pub fn alpha_factor(self) -> f64 {
        f64::from(self.alpha) / f64::from(u8::MAX)
    }
}

impl<'de> Deserialize<'de> for RgbaColor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        parse_rgba_color(&raw).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct FontConfig {
    pub family: String,
    pub size: f32,
    pub fallback: Vec<String>,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: DEFAULT_FONT_FAMILY.to_string(),
            size: DEFAULT_FONT_SIZE,
            fallback: DEFAULT_FONT_FALLBACKS
                .iter()
                .map(|family| (*family).to_string())
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct ThemeColors {
    pub background: RgbaColor,
    pub foreground: RgbaColor,
}

impl Default for ThemeColors {
    fn default() -> Self {
        Self {
            background: RgbaColor::from_rgb(0x05, 0x0a, 0x0f),
            foreground: RgbaColor::from_rgb(0xc0, 0xca, 0xf5),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct BorderTheme {
    pub active: RgbaColor,
    pub inactive: RgbaColor,
    pub width: u16,
}

impl Default for BorderTheme {
    fn default() -> Self {
        Self {
            active: RgbaColor::from_rgb(0x7a, 0xa2, 0xf7),
            inactive: RgbaColor::from_rgb(0x3b, 0x42, 0x61),
            width: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct CursorTheme {
    pub color: RgbaColor,
    #[serde(rename = "blink-interval-ms")]
    pub blink_interval_ms: u64,
}

impl Default for CursorTheme {
    fn default() -> Self {
        Self {
            color: RgbaColor::from_rgb(0xc0, 0xca, 0xf5),
            blink_interval_ms: DEFAULT_CURSOR_BLINK_INTERVAL_MS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct SelectionTheme {
    pub background: RgbaColor,
    pub foreground: RgbaColor,
}

impl Default for SelectionTheme {
    fn default() -> Self {
        Self {
            background: RgbaColor::from_rgb(0x26, 0x4f, 0x78),
            foreground: RgbaColor::from_rgb(0xff, 0xff, 0xff),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct PaneTheme {
    pub gap: u16,
    pub padding: u16,
    pub radius: u16,
}

impl Default for PaneTheme {
    fn default() -> Self {
        Self {
            gap: DEFAULT_PANE_GAP,
            padding: DEFAULT_PANE_PADDING,
            radius: DEFAULT_PANE_RADIUS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct BlurTheme {
    pub enabled: bool,
    #[serde(rename = "fallback-tint-opacity")]
    pub fallback_tint_opacity: f32,
}

impl Default for BlurTheme {
    fn default() -> Self {
        Self {
            enabled: false,
            fallback_tint_opacity: DEFAULT_BLUR_FALLBACK_TINT_OPACITY,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct AnimationTheme {
    pub enabled: bool,
    #[serde(rename = "duration-ms")]
    pub duration_ms: u64,
}

impl Default for AnimationTheme {
    fn default() -> Self {
        Self {
            enabled: true,
            duration_ms: DEFAULT_ANIMATION_DURATION_MS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(default)]
pub struct LowPowerTheme {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct ThemeConfig {
    pub opacity: f32,
    pub color: ThemeColors,
    pub border: BorderTheme,
    pub pane: PaneTheme,
    pub blur: BlurTheme,
    pub animation: AnimationTheme,
    #[serde(rename = "low-power")]
    pub low_power: LowPowerTheme,
    pub cursor: CursorTheme,
    pub selection: SelectionTheme,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            opacity: DEFAULT_THEME_OPACITY,
            color: ThemeColors::default(),
            border: BorderTheme::default(),
            pane: PaneTheme::default(),
            blur: BlurTheme::default(),
            animation: AnimationTheme::default(),
            low_power: LowPowerTheme::default(),
            cursor: CursorTheme::default(),
            selection: SelectionTheme::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub renderer: RendererConfig,
    pub font: FontConfig,
    pub theme: ThemeConfig,
    pub agent: AgentConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AgentProviderKind {
    #[serde(rename = "openai-compatible")]
    #[default]
    OpenAiCompatible,
    Local,
    Cli,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(default)]
pub struct AgentProviderConfig {
    #[serde(rename = "type")]
    pub kind: AgentProviderKind,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    pub command: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(default)]
pub struct AgentConfig {
    pub enabled: bool,
    #[serde(rename = "read-env")]
    pub read_env: bool,
    #[serde(rename = "read-history")]
    pub read_history: bool,
    pub provider: Option<AgentProviderConfig>,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config at {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("invalid config at {path}: {reason}")]
    Validation { path: PathBuf, reason: String },
}

#[derive(Debug, Clone)]
pub struct ConfigReloader {
    path: PathBuf,
    last_raw: String,
}

impl Config {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let raw = read_config_raw(path)?;
        parse_config_raw(path, &raw)
    }
}

impl ConfigReloader {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let raw = read_config_raw(path)?;
        Ok(Self {
            path: path.to_path_buf(),
            last_raw: raw,
        })
    }

    pub fn reload_if_changed(&mut self) -> Result<Option<Config>, ConfigError> {
        let raw = read_config_raw(&self.path)?;
        if raw == self.last_raw {
            return Ok(None);
        }

        let config = parse_config_raw(&self.path, &raw)?;
        self.last_raw = raw;
        Ok(Some(config))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn read_config_raw(path: &Path) -> Result<String, ConfigError> {
    fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })
}

fn parse_config_raw(path: &Path, raw: &str) -> Result<Config, ConfigError> {
    let config = toml::from_str::<Config>(raw).map_err(|source| ConfigError::Parse {
        path: path.to_path_buf(),
        source,
    })?;
    validate_config(path, &config)?;
    Ok(config)
}

fn validate_config(path: &Path, config: &Config) -> Result<(), ConfigError> {
    if !(0.0..=1.0).contains(&config.theme.opacity) {
        return Err(ConfigError::Validation {
            path: path.to_path_buf(),
            reason: format!(
                "theme.opacity must be within 0.0..=1.0, got {}",
                config.theme.opacity
            ),
        });
    }
    if config.font.size <= 0.0 {
        return Err(ConfigError::Validation {
            path: path.to_path_buf(),
            reason: format!("font.size must be greater than 0, got {}", config.font.size),
        });
    }
    if config.theme.cursor.blink_interval_ms == 0 {
        return Err(ConfigError::Validation {
            path: path.to_path_buf(),
            reason: "theme.cursor.blink-interval-ms must be greater than 0".to_string(),
        });
    }
    if !(0.0..=1.0).contains(&config.theme.blur.fallback_tint_opacity) {
        return Err(ConfigError::Validation {
            path: path.to_path_buf(),
            reason: format!(
                "theme.blur.fallback-tint-opacity must be within 0.0..=1.0, got {}",
                config.theme.blur.fallback_tint_opacity
            ),
        });
    }
    if config.theme.animation.duration_ms == 0 {
        return Err(ConfigError::Validation {
            path: path.to_path_buf(),
            reason: "theme.animation.duration-ms must be greater than 0".to_string(),
        });
    }
    if config.agent.enabled && config.agent.provider.is_none() {
        return Err(ConfigError::Validation {
            path: path.to_path_buf(),
            reason: "agent.provider must be configured when agent.enabled = true".to_string(),
        });
    }
    Ok(())
}

fn parse_rgba_color(raw: &str) -> Result<RgbaColor, String> {
    let trimmed = raw.trim();
    let hex = trimmed
        .strip_prefix('#')
        .ok_or_else(|| format!("colors must start with '#', got {trimmed:?}"))?;

    match hex.len() {
        6 => Ok(RgbaColor::from_rgb(
            parse_hex_component(&hex[0..2])?,
            parse_hex_component(&hex[2..4])?,
            parse_hex_component(&hex[4..6])?,
        )),
        8 => Ok(RgbaColor::from_rgba(
            parse_hex_component(&hex[0..2])?,
            parse_hex_component(&hex[2..4])?,
            parse_hex_component(&hex[4..6])?,
            parse_hex_component(&hex[6..8])?,
        )),
        _ => Err(format!(
            "colors must be in #RRGGBB or #RRGGBBAA form, got {trimmed:?}"
        )),
    }
}

fn parse_hex_component(raw: &str) -> Result<u8, String> {
    u8::from_str_radix(raw, 16)
        .map_err(|_| format!("invalid hex component {raw:?} in color literal"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn defaults_cover_renderer_font_and_theme() {
        let config = Config::default();

        assert_eq!(config.renderer.backend, RendererBackend::Gpu);
        assert_eq!(config.font.family, DEFAULT_FONT_FAMILY);
        assert_eq!(config.font.size, DEFAULT_FONT_SIZE);
        assert_eq!(config.theme.opacity, DEFAULT_THEME_OPACITY);
        assert_eq!(
            config.theme.cursor.blink_interval_ms,
            DEFAULT_CURSOR_BLINK_INTERVAL_MS
        );
        assert_eq!(
            config.theme.color.background,
            RgbaColor::from_rgb(0x05, 0x0a, 0x0f)
        );
        assert_eq!(config.theme.pane.gap, DEFAULT_PANE_GAP);
        assert_eq!(config.theme.pane.padding, DEFAULT_PANE_PADDING);
        assert_eq!(config.theme.pane.radius, DEFAULT_PANE_RADIUS);
        assert!(!config.theme.blur.enabled);
        assert_eq!(
            config.theme.blur.fallback_tint_opacity,
            DEFAULT_BLUR_FALLBACK_TINT_OPACITY
        );
        assert!(config.theme.animation.enabled);
        assert_eq!(
            config.theme.animation.duration_ms,
            DEFAULT_ANIMATION_DURATION_MS
        );
        assert!(!config.theme.low_power.enabled);
        assert!(!config.agent.enabled);
        assert!(!config.agent.read_env);
        assert!(!config.agent.read_history);
        assert_eq!(config.agent.provider, None);
    }

    #[test]
    fn loads_theme_and_font_fields_from_toml() {
        let path = temp_config_path("theme-load");
        fs::write(
            &path,
            "[renderer]\nbackend = \"software\"\n\n[font]\nfamily = \"Iosevka\"\nsize = 16.5\nfallback = [\"Noto Sans CJK SC\"]\n\n[theme]\nopacity = 0.8\n\n[theme.color]\nbackground = \"#112233\"\nforeground = \"#abcdef\"\n\n[theme.border]\nactive = \"#7aa2f7\"\ninactive = \"#3b4261\"\nwidth = 2\n\n[theme.pane]\ngap = 10\npadding = 4\nradius = 12\n\n[theme.blur]\nenabled = true\nfallback-tint-opacity = 0.94\n\n[theme.animation]\nenabled = false\nduration-ms = 180\n\n[theme.low-power]\nenabled = true\n\n[theme.cursor]\ncolor = \"#ffeeaa\"\nblink-interval-ms = 450\n\n[theme.selection]\nbackground = \"#264f78cc\"\nforeground = \"#ffffff\"\n\n[agent]\nenabled = true\nread-env = true\nread-history = true\n\n[agent.provider]\ntype = \"openai-compatible\"\nmodel = \"gpt-5\"\nendpoint = \"https://example.invalid/v1\"\n",
        )
        .expect("write config");

        let config = Config::load_from_path(&path).expect("load config");
        assert_eq!(config.renderer.backend, RendererBackend::Software);
        assert_eq!(config.font.family, "Iosevka");
        assert_eq!(config.font.size, 16.5);
        assert_eq!(config.font.fallback, vec!["Noto Sans CJK SC".to_string()]);
        assert_eq!(config.theme.opacity, 0.8);
        assert_eq!(
            config.theme.color.background,
            RgbaColor::from_rgb(0x11, 0x22, 0x33)
        );
        assert_eq!(config.theme.border.width, 2);
        assert_eq!(config.theme.pane.gap, 10);
        assert_eq!(config.theme.pane.padding, 4);
        assert_eq!(config.theme.pane.radius, 12);
        assert!(config.theme.blur.enabled);
        assert_eq!(config.theme.blur.fallback_tint_opacity, 0.94);
        assert!(!config.theme.animation.enabled);
        assert_eq!(config.theme.animation.duration_ms, 180);
        assert!(config.theme.low_power.enabled);
        assert_eq!(
            config.theme.cursor.color,
            RgbaColor::from_rgb(0xff, 0xee, 0xaa)
        );
        assert_eq!(
            config.theme.selection.background,
            RgbaColor::from_rgba(0x26, 0x4f, 0x78, 0xcc)
        );
        assert!(config.agent.enabled);
        assert!(config.agent.read_env);
        assert!(config.agent.read_history);
        let provider = config
            .agent
            .provider
            .as_ref()
            .expect("agent provider should be loaded");
        assert_eq!(provider.kind, AgentProviderKind::OpenAiCompatible);
        assert_eq!(provider.model.as_deref(), Some("gpt-5"));
        assert_eq!(
            provider.endpoint.as_deref(),
            Some("https://example.invalid/v1")
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn validation_errors_keep_the_config_path() {
        let path = temp_config_path("invalid-opacity");
        fs::write(&path, "[theme]\nopacity = 1.5\n").expect("write config");

        let error = Config::load_from_path(&path).expect_err("config should fail");
        match error {
            ConfigError::Validation {
                path: error_path, ..
            } => assert_eq!(error_path, path),
            other => panic!("unexpected error: {other:?}"),
        }

        let _ = fs::remove_file(path);
    }

    #[test]
    fn parse_errors_keep_the_config_path() {
        let path = temp_config_path("parse-error");
        fs::write(&path, "[theme\nopacity = 0.9\n").expect("write config");

        let error = Config::load_from_path(&path).expect_err("config should fail");
        match error {
            ConfigError::Parse {
                path: error_path, ..
            } => assert_eq!(error_path, path),
            other => panic!("unexpected error: {other:?}"),
        }

        let _ = fs::remove_file(path);
    }

    #[test]
    fn agent_enable_without_provider_is_rejected() {
        let path = temp_config_path("agent-provider-required");
        fs::write(&path, "[agent]\nenabled = true\n").expect("write config");

        let error = Config::load_from_path(&path).expect_err("config should fail");
        match error {
            ConfigError::Validation { reason, .. } => {
                assert!(reason.contains("agent.provider must be configured"));
            }
            other => panic!("unexpected error: {other:?}"),
        }

        let _ = fs::remove_file(path);
    }

    #[test]
    fn reloader_only_returns_changed_configs() {
        let path = temp_config_path("reloader");
        fs::write(&path, "[theme]\nopacity = 1.0\n").expect("write initial config");

        let mut reloader = ConfigReloader::from_path(&path).expect("watch config");
        assert_eq!(reloader.reload_if_changed().expect("same config"), None);

        fs::write(&path, "[theme]\nopacity = 0.7\n").expect("write changed config");
        let changed = reloader
            .reload_if_changed()
            .expect("changed config should load")
            .expect("config should be marked changed");
        assert_eq!(changed.theme.opacity, 0.7);
        assert_eq!(
            reloader.reload_if_changed().expect("same changed config"),
            None
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn color_literals_require_hash_prefixed_hex() {
        assert_eq!(
            parse_rgba_color("#112233").expect("rgb should parse"),
            RgbaColor::from_rgb(0x11, 0x22, 0x33)
        );
        assert!(parse_rgba_color("112233").is_err());
        assert!(parse_rgba_color("#12345").is_err());
    }

    fn temp_config_path(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("noctrail-config-{label}-{unique}.toml"))
    }
}
