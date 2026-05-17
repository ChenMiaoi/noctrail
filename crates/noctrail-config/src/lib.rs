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
const DEFAULT_FONT_LINE_HEIGHT: f32 = 1.55;
const DEFAULT_FONT_WEIGHT: u16 = 450;
const DEFAULT_FONT_BOLD_WEIGHT: u16 = 650;
const DEFAULT_FONT_FALLBACKS: [&str; 4] = [
    "Noto Sans CJK SC",
    "Noto Color Emoji",
    "Apple Color Emoji",
    "Segoe UI Emoji",
];
const DEFAULT_CURSOR_BLINK_INTERVAL_MS: u64 = 600;
const DEFAULT_PANE_GAP: u16 = 12;
const DEFAULT_PANE_PADDING: u16 = 8;
const DEFAULT_PANE_RADIUS: u16 = 14;
const DEFAULT_BLUR_FALLBACK_TINT_OPACITY: f32 = 0.92;
const DEFAULT_ANIMATION_DURATION_MS: u64 = 120;
const DEFAULT_LAYOUT_RESIZE_STEP: u16 = 5;
const DEFAULT_SCRATCH_HEIGHT_PERCENT: u8 = 33;
const DEFAULT_STARTUP_WORKSPACE: u8 = 1;
const MIN_WORKSPACE_ID: u8 = 1;
const MAX_WORKSPACE_ID: u8 = 9;

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
    #[serde(rename = "line-height")]
    pub line_height: f32,
    pub weight: u16,
    #[serde(rename = "bold-weight")]
    pub bold_weight: u16,
    pub fallback: Vec<String>,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: DEFAULT_FONT_FAMILY.to_string(),
            size: DEFAULT_FONT_SIZE,
            line_height: DEFAULT_FONT_LINE_HEIGHT,
            weight: DEFAULT_FONT_WEIGHT,
            bold_weight: DEFAULT_FONT_BOLD_WEIGHT,
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
    #[serde(rename = "chrome-background")]
    pub chrome_background: RgbaColor,
    #[serde(rename = "chrome-foreground")]
    pub chrome_foreground: RgbaColor,
    #[serde(rename = "chrome-muted")]
    pub chrome_muted: RgbaColor,
    #[serde(rename = "chrome-accent")]
    pub chrome_accent: RgbaColor,
    #[serde(rename = "chrome-danger")]
    pub chrome_danger: RgbaColor,
}

impl Default for ThemeColors {
    fn default() -> Self {
        Self {
            background: RgbaColor::from_rgb(0x0b, 0x10, 0x14),
            foreground: RgbaColor::from_rgb(0xe6, 0xeb, 0xef),
            chrome_background: RgbaColor::from_rgb(0x14, 0x1c, 0x24),
            chrome_foreground: RgbaColor::from_rgb(0xf5, 0xf7, 0xfa),
            chrome_muted: RgbaColor::from_rgb(0x8c, 0x99, 0xa5),
            chrome_accent: RgbaColor::from_rgb(0x73, 0xc9, 0xa7),
            chrome_danger: RgbaColor::from_rgb(0xf0, 0x76, 0x62),
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
    pub keymap: KeymapConfig,
    pub layout: LayoutConfig,
    pub agent: AgentConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum LayoutSplitAxis {
    #[default]
    Auto,
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct LayoutConfig {
    #[serde(rename = "default-split-axis")]
    pub default_split_axis: LayoutSplitAxis,
    #[serde(rename = "resize-step")]
    pub resize_step: u16,
    #[serde(rename = "scratch-height-percent")]
    pub scratch_height_percent: u8,
    #[serde(rename = "startup-workspace")]
    pub startup_workspace: u8,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            default_split_axis: LayoutSplitAxis::Auto,
            resize_step: DEFAULT_LAYOUT_RESIZE_STEP,
            scratch_height_percent: DEFAULT_SCRATCH_HEIGHT_PERCENT,
            startup_workspace: DEFAULT_STARTUP_WORKSPACE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct KeymapConfig {
    pub copy: Vec<String>,
    pub paste: Vec<String>,
    #[serde(rename = "command-palette")]
    pub command_palette: Vec<String>,
    #[serde(rename = "block-browser")]
    pub block_browser: Vec<String>,
    #[serde(rename = "patch-preview")]
    pub patch_preview: Vec<String>,
    #[serde(rename = "review-panel")]
    pub review_panel: Vec<String>,
    #[serde(rename = "agent-context")]
    pub agent_context: Vec<String>,
    #[serde(rename = "agent-audit")]
    pub agent_audit: Vec<String>,
    #[serde(rename = "focus-left")]
    pub focus_left: Vec<String>,
    #[serde(rename = "focus-right")]
    pub focus_right: Vec<String>,
    #[serde(rename = "focus-up")]
    pub focus_up: Vec<String>,
    #[serde(rename = "focus-down")]
    pub focus_down: Vec<String>,
}

impl Default for KeymapConfig {
    fn default() -> Self {
        Self {
            copy: vec!["ctrl-shift-c".to_string(), "ctrl-insert".to_string()],
            paste: vec!["ctrl-shift-v".to_string(), "shift-insert".to_string()],
            command_palette: vec!["ctrl-shift-p".to_string()],
            block_browser: vec!["ctrl-shift-b".to_string()],
            patch_preview: vec!["ctrl-shift-d".to_string()],
            review_panel: vec!["ctrl-shift-r".to_string()],
            agent_context: vec!["ctrl-shift-a".to_string()],
            agent_audit: vec!["ctrl-shift-l".to_string()],
            focus_left: vec!["alt-h".to_string()],
            focus_right: vec!["alt-l".to_string()],
            focus_up: vec!["alt-k".to_string()],
            focus_down: vec!["alt-j".to_string()],
        }
    }
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
    if config.font.line_height <= 1.0 {
        return Err(ConfigError::Validation {
            path: path.to_path_buf(),
            reason: format!(
                "font.line-height must be greater than 1.0, got {}",
                config.font.line_height
            ),
        });
    }
    if !(100..=900).contains(&config.font.weight) {
        return Err(ConfigError::Validation {
            path: path.to_path_buf(),
            reason: format!(
                "font.weight must be within 100..=900, got {}",
                config.font.weight
            ),
        });
    }
    if !(100..=900).contains(&config.font.bold_weight) {
        return Err(ConfigError::Validation {
            path: path.to_path_buf(),
            reason: format!(
                "font.bold-weight must be within 100..=900, got {}",
                config.font.bold_weight
            ),
        });
    }
    if config.font.bold_weight < config.font.weight {
        return Err(ConfigError::Validation {
            path: path.to_path_buf(),
            reason: format!(
                "font.bold-weight must be greater than or equal to font.weight, got {} < {}",
                config.font.bold_weight, config.font.weight
            ),
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
    if config.layout.resize_step == 0 {
        return Err(ConfigError::Validation {
            path: path.to_path_buf(),
            reason: "layout.resize-step must be greater than 0".to_string(),
        });
    }
    if !(1..=100).contains(&config.layout.scratch_height_percent) {
        return Err(ConfigError::Validation {
            path: path.to_path_buf(),
            reason: format!(
                "layout.scratch-height-percent must be within 1..=100, got {}",
                config.layout.scratch_height_percent
            ),
        });
    }
    if !(MIN_WORKSPACE_ID..=MAX_WORKSPACE_ID).contains(&config.layout.startup_workspace) {
        return Err(ConfigError::Validation {
            path: path.to_path_buf(),
            reason: format!(
                "layout.startup-workspace must be within {}..={}, got {}",
                MIN_WORKSPACE_ID, MAX_WORKSPACE_ID, config.layout.startup_workspace
            ),
        });
    }
    validate_keymap(path, &config.keymap)?;
    if config.agent.enabled && config.agent.provider.is_none() {
        return Err(ConfigError::Validation {
            path: path.to_path_buf(),
            reason: "agent.provider must be configured when agent.enabled = true".to_string(),
        });
    }
    if let Some(provider) = config.agent.provider.as_ref() {
        match provider.kind {
            AgentProviderKind::OpenAiCompatible => {
                if provider
                    .endpoint
                    .as_deref()
                    .is_none_or(|endpoint| endpoint.trim().is_empty())
                {
                    return Err(ConfigError::Validation {
                        path: path.to_path_buf(),
                        reason: "agent.provider.endpoint must be set for openai-compatible"
                            .to_string(),
                    });
                }
                if provider
                    .model
                    .as_deref()
                    .is_none_or(|model| model.trim().is_empty())
                {
                    return Err(ConfigError::Validation {
                        path: path.to_path_buf(),
                        reason: "agent.provider.model must be set for openai-compatible"
                            .to_string(),
                    });
                }
            }
            AgentProviderKind::Local => {
                if provider
                    .model
                    .as_deref()
                    .is_none_or(|model| model.trim().is_empty())
                {
                    return Err(ConfigError::Validation {
                        path: path.to_path_buf(),
                        reason: "agent.provider.model must be set for local".to_string(),
                    });
                }
            }
            AgentProviderKind::Cli => {
                if provider.command.is_empty() {
                    return Err(ConfigError::Validation {
                        path: path.to_path_buf(),
                        reason: "agent.provider.command must be set for cli".to_string(),
                    });
                }
            }
        }
    }
    Ok(())
}

fn validate_keymap(path: &Path, keymap: &KeymapConfig) -> Result<(), ConfigError> {
    use std::collections::BTreeMap;

    let actions = [
        ("keymap.copy", &keymap.copy),
        ("keymap.paste", &keymap.paste),
        ("keymap.command-palette", &keymap.command_palette),
        ("keymap.block-browser", &keymap.block_browser),
        ("keymap.patch-preview", &keymap.patch_preview),
        ("keymap.review-panel", &keymap.review_panel),
        ("keymap.agent-context", &keymap.agent_context),
        ("keymap.agent-audit", &keymap.agent_audit),
        ("keymap.focus-left", &keymap.focus_left),
        ("keymap.focus-right", &keymap.focus_right),
        ("keymap.focus-up", &keymap.focus_up),
        ("keymap.focus-down", &keymap.focus_down),
    ];
    let mut seen = BTreeMap::<String, &'static str>::new();

    for (action, bindings) in actions {
        for binding in bindings {
            let normalized =
                parse_shortcut_binding(binding).map_err(|reason| ConfigError::Validation {
                    path: path.to_path_buf(),
                    reason: format!("{action} contains invalid shortcut {binding:?}: {reason}"),
                })?;
            if let Some(previous) = seen.insert(normalized.clone(), action) {
                return Err(ConfigError::Validation {
                    path: path.to_path_buf(),
                    reason: format!(
                        "shortcut {binding:?} is assigned to both {previous} and {action}"
                    ),
                });
            }
        }
    }

    Ok(())
}

fn parse_shortcut_binding(raw: &str) -> Result<String, String> {
    let binding = raw.trim().to_ascii_lowercase();
    if binding.is_empty() {
        return Err("shortcut may not be empty".to_string());
    }

    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut key = None::<String>;

    for token in binding.split('-') {
        match token {
            "ctrl" | "control" => {
                if ctrl {
                    return Err("duplicate ctrl modifier".to_string());
                }
                ctrl = true;
            }
            "alt" => {
                if alt {
                    return Err("duplicate alt modifier".to_string());
                }
                alt = true;
            }
            "shift" => {
                if shift {
                    return Err("duplicate shift modifier".to_string());
                }
                shift = true;
            }
            "insert" => {
                if key.replace("insert".to_string()).is_some() {
                    return Err("shortcut must contain exactly one key".to_string());
                }
            }
            value if value.chars().count() == 1 => {
                if key.replace(value.to_string()).is_some() {
                    return Err("shortcut must contain exactly one key".to_string());
                }
            }
            "super" | "cmd" | "meta" => {
                return Err("super/cmd/meta shortcuts are not supported".to_string());
            }
            other => return Err(format!("unknown shortcut token {other:?}")),
        }
    }

    let key = key.ok_or_else(|| "shortcut must contain a key token".to_string())?;
    let mut normalized = Vec::new();
    if ctrl {
        normalized.push("ctrl".to_string());
    }
    if alt {
        normalized.push("alt".to_string());
    }
    if shift {
        normalized.push("shift".to_string());
    }
    normalized.push(key);
    Ok(normalized.join("-"))
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
        assert_eq!(config.font.line_height, DEFAULT_FONT_LINE_HEIGHT);
        assert_eq!(config.font.weight, DEFAULT_FONT_WEIGHT);
        assert_eq!(config.font.bold_weight, DEFAULT_FONT_BOLD_WEIGHT);
        assert_eq!(config.theme.opacity, DEFAULT_THEME_OPACITY);
        assert_eq!(
            config.theme.cursor.blink_interval_ms,
            DEFAULT_CURSOR_BLINK_INTERVAL_MS
        );
        assert_eq!(
            config.theme.color.background,
            RgbaColor::from_rgb(0x0b, 0x10, 0x14)
        );
        assert_eq!(
            config.theme.color.chrome_accent,
            RgbaColor::from_rgb(0x73, 0xc9, 0xa7)
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
        assert_eq!(config.layout.default_split_axis, LayoutSplitAxis::Auto);
        assert_eq!(config.layout.resize_step, DEFAULT_LAYOUT_RESIZE_STEP);
        assert_eq!(
            config.layout.scratch_height_percent,
            DEFAULT_SCRATCH_HEIGHT_PERCENT
        );
        assert_eq!(config.layout.startup_workspace, DEFAULT_STARTUP_WORKSPACE);
        assert_eq!(config.keymap.copy, vec!["ctrl-shift-c", "ctrl-insert"]);
        assert_eq!(config.keymap.paste, vec!["ctrl-shift-v", "shift-insert"]);
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
            "[renderer]\nbackend = \"software\"\n\n[font]\nfamily = \"Iosevka\"\nsize = 16.5\nline-height = 1.7\nweight = 430\nbold-weight = 700\nfallback = [\"Noto Sans CJK SC\"]\n\n[theme]\nopacity = 0.8\n\n[theme.color]\nbackground = \"#112233\"\nforeground = \"#abcdef\"\nchrome-background = \"#171f28\"\nchrome-foreground = \"#f6f8fa\"\nchrome-muted = \"#8693a0\"\nchrome-accent = \"#73c9a7\"\nchrome-danger = \"#f07662\"\n\n[theme.border]\nactive = \"#7aa2f7\"\ninactive = \"#3b4261\"\nwidth = 2\n\n[theme.pane]\ngap = 10\npadding = 4\nradius = 12\n\n[theme.blur]\nenabled = true\nfallback-tint-opacity = 0.94\n\n[theme.animation]\nenabled = false\nduration-ms = 180\n\n[theme.low-power]\nenabled = true\n\n[theme.cursor]\ncolor = \"#ffeeaa\"\nblink-interval-ms = 450\n\n[theme.selection]\nbackground = \"#264f78cc\"\nforeground = \"#ffffff\"\n\n[layout]\ndefault-split-axis = \"horizontal\"\nresize-step = 7\nscratch-height-percent = 40\nstartup-workspace = 3\n\n[keymap]\ncopy = [\"ctrl-shift-y\"]\npaste = [\"ctrl-shift-u\"]\nfocus-left = [\"alt-a\"]\nfocus-right = [\"alt-d\"]\nfocus-up = [\"alt-w\"]\nfocus-down = [\"alt-s\"]\n\n[agent]\nenabled = true\nread-env = true\nread-history = true\n\n[agent.provider]\ntype = \"openai-compatible\"\nmodel = \"gpt-5\"\nendpoint = \"https://example.invalid/v1\"\n",
        )
        .expect("write config");

        let config = Config::load_from_path(&path).expect("load config");
        assert_eq!(config.renderer.backend, RendererBackend::Software);
        assert_eq!(config.font.family, "Iosevka");
        assert_eq!(config.font.size, 16.5);
        assert_eq!(config.font.line_height, 1.7);
        assert_eq!(config.font.weight, 430);
        assert_eq!(config.font.bold_weight, 700);
        assert_eq!(config.font.fallback, vec!["Noto Sans CJK SC".to_string()]);
        assert_eq!(config.theme.opacity, 0.8);
        assert_eq!(
            config.theme.color.background,
            RgbaColor::from_rgb(0x11, 0x22, 0x33)
        );
        assert_eq!(
            config.theme.color.chrome_background,
            RgbaColor::from_rgb(0x17, 0x1f, 0x28)
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
            config.layout.default_split_axis,
            LayoutSplitAxis::Horizontal
        );
        assert_eq!(config.layout.resize_step, 7);
        assert_eq!(config.layout.scratch_height_percent, 40);
        assert_eq!(config.layout.startup_workspace, 3);
        assert_eq!(config.keymap.copy, vec!["ctrl-shift-y".to_string()]);
        assert_eq!(config.keymap.focus_left, vec!["alt-a".to_string()]);
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
    fn openai_provider_requires_endpoint_and_model() {
        let path = temp_config_path("agent-openai-endpoint");
        fs::write(
            &path,
            "[agent]\nenabled = true\n\n[agent.provider]\ntype = \"openai-compatible\"\nmodel = \"gpt-5\"\n",
        )
        .expect("write config");

        let error = Config::load_from_path(&path).expect_err("config should fail");
        match error {
            ConfigError::Validation { reason, .. } => {
                assert!(reason.contains("agent.provider.endpoint"));
            }
            other => panic!("unexpected error: {other:?}"),
        }

        let _ = fs::remove_file(path);
    }

    #[test]
    fn cli_provider_requires_command() {
        let path = temp_config_path("agent-cli-command");
        fs::write(
            &path,
            "[agent]\nenabled = true\n\n[agent.provider]\ntype = \"cli\"\n",
        )
        .expect("write config");

        let error = Config::load_from_path(&path).expect_err("config should fail");
        match error {
            ConfigError::Validation { reason, .. } => {
                assert!(reason.contains("agent.provider.command"));
            }
            other => panic!("unexpected error: {other:?}"),
        }

        let _ = fs::remove_file(path);
    }

    #[test]
    fn invalid_layout_values_are_rejected() {
        let path = temp_config_path("layout-invalid");
        fs::write(
            &path,
            "[layout]\nresize-step = 0\nscratch-height-percent = 101\n",
        )
        .expect("write config");

        let error = Config::load_from_path(&path).expect_err("config should fail");
        match error {
            ConfigError::Validation { reason, .. } => {
                assert!(reason.contains("layout.resize-step"));
            }
            other => panic!("unexpected error: {other:?}"),
        }

        let _ = fs::remove_file(path);
    }

    #[test]
    fn duplicate_keymap_bindings_are_rejected() {
        let path = temp_config_path("keymap-duplicate");
        fs::write(
            &path,
            "[keymap]\ncopy = [\"ctrl-shift-c\"]\npaste = [\"ctrl-shift-c\"]\n",
        )
        .expect("write config");

        let error = Config::load_from_path(&path).expect_err("config should fail");
        match error {
            ConfigError::Validation { reason, .. } => {
                assert!(reason.contains("assigned to both"));
            }
            other => panic!("unexpected error: {other:?}"),
        }

        let _ = fs::remove_file(path);
    }

    #[test]
    fn local_provider_requires_model() {
        let path = temp_config_path("agent-local-model");
        fs::write(
            &path,
            "[agent]\nenabled = true\n\n[agent.provider]\ntype = \"local\"\n",
        )
        .expect("write config");

        let error = Config::load_from_path(&path).expect_err("config should fail");
        match error {
            ConfigError::Validation { reason, .. } => {
                assert!(reason.contains("agent.provider.model"));
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
