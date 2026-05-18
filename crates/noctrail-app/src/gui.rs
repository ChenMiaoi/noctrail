use std::{
    collections::HashMap,
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use noctrail_agent::{CommandPermission, CommandProposal, CommandRisk, ProviderRequestPreview};
use noctrail_config::{
    AgentConfig, ConfigReloader, FontConfig, KeymapConfig, LayoutConfig, LayoutSplitAxis,
    ThemeConfig,
};
use noctrail_layout::{FocusDirection, LayoutRect, SplitAxis, WorkspaceId};
use noctrail_pty::PtySize;
use noctrail_render::{
    FontPreferences, GlyphRasterConfig, GpuRenderer, PaneBorderStyle, RenderBackend, RenderGlyph,
    RenderPlan, RenderRect, RenderRow, Rgba, SoftwareRenderPalette, rasterize_software_frame,
};
use noctrail_runtime::PaneId;
use noctrail_term::{Color, Cursor, DamageSet, MouseTrackingMode, Position, SelectionMode, Style};
use tracing::{debug, error, info, warn};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize},
    event::{ElementState, Ime, MouseButton as WinitMouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    keyboard::{Key, ModifiersState, NamedKey},
    window::{CursorIcon, ResizeDirection, Window, WindowId},
};

#[cfg(target_os = "macos")]
use winit::platform::macos::WindowAttributesExtMacOS;
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

use crate::{DesktopApp, DesktopFrame, PaneChromeConfig, clipboard::ClipboardBridge, input};

const DEFAULT_WINDOW_WIDTH: u32 = 1280;
const DEFAULT_WINDOW_HEIGHT: u32 = 800;
const ANIMATION_FRAME_INTERVAL: Duration = Duration::from_millis(16);
const STARTUP_DEBUG_WINDOW: Duration = Duration::from_secs(3);
const STABLE_DEBUG_SAMPLE_INTERVAL: Duration = Duration::from_secs(2);
const WINDOW_RESIZE_HANDLE_PX: f64 = 6.0;
const MAX_OUTPUT_DRAINS_PER_EVENT: usize = 8;
const STARTUP_FOCUS_RETRY_WINDOW: Duration = Duration::from_secs(2);
const STARTUP_FOCUS_RETRY_INTERVAL: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuiEvent {
    RuntimeOutput,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct VisualEffectsPolicy {
    low_power_enabled: bool,
    requested_opacity: f32,
    effective_opacity: f32,
    window_transparent: bool,
    transparency_fallback_reason: Option<&'static str>,
    blur_mode: BlurMode,
    blur_fallback_reason: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlurMode {
    Disabled,
    TintedSolid,
}

pub fn run() -> Result<(), Box<dyn Error>> {
    run_with_options(GuiLaunchOptions::default())
}

#[derive(Debug, Clone, PartialEq)]
pub struct GuiLaunchOptions {
    pub safe_mode: bool,
    pub debug_logging: bool,
    pub renderer_backend: RenderBackend,
    pub config_path: Option<PathBuf>,
    pub theme: ThemeConfig,
    pub font: FontConfig,
    pub keymap: KeymapConfig,
    pub layout: LayoutConfig,
    pub agent: AgentConfig,
}

impl Default for GuiLaunchOptions {
    fn default() -> Self {
        Self {
            safe_mode: false,
            debug_logging: false,
            renderer_backend: RenderBackend::Gpu,
            config_path: None,
            theme: ThemeConfig::default(),
            font: FontConfig::default(),
            keymap: KeymapConfig::default(),
            layout: LayoutConfig::default(),
            agent: AgentConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdleScheduleProbe {
    pub premature_redraw: bool,
    pub next_wakeup: Duration,
}

pub fn idle_schedule_probe(theme: &ThemeConfig) -> IdleScheduleProbe {
    let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(10, 3));
    let mut gui = GuiApp::new(
        app,
        GuiLaunchOptions {
            theme: theme.clone(),
            ..GuiLaunchOptions::default()
        },
    );
    let now = Instant::now();
    gui.window_focused = true;
    gui.cursor_visible = true;
    gui.next_cursor_blink_at = now + gui.frame_interval;

    IdleScheduleProbe {
        premature_redraw: gui.advance_cursor_blink(now + gui.frame_interval / 2),
        next_wakeup: gui.next_cursor_blink_at.saturating_duration_since(now),
    }
}

pub fn agent_audit_smoke() -> Result<(), Box<dyn Error>> {
    let app = DesktopApp::spawn_shell(LayoutRect::new(0, 0, 120, 80), PtySize::new(80, 24))?;
    let mut gui = GuiApp::new(app, GuiLaunchOptions::default());
    gui.app.set_block_observer_enabled(true);
    gui.app.advance_output(&shell_integration_probe_bytes(
        "cargo test -p noctrail-app",
        "/tmp/noctrail-agent-audit",
        0,
        19,
        "audit context\n",
    ));
    let _ = gui.app.select_newest_command_block();
    gui.app
        .set_agent_explicit_files(vec![PathBuf::from("/tmp/noctrail/Cargo.toml")]);
    gui.app.select_viewport_range(
        Position { row: 0, col: 0 },
        Position { row: 0, col: 4 },
        SelectionMode::Normal,
    );

    let preview = crate::redaction::redact_agent_context_preview(&gui.app.agent_context_preview());
    gui.app.record_agent_context_access(&preview);
    let prompt = format!(
        "cwd: {}\ncommand: {}\nselection: {}\nfiles: {}",
        preview
            .cwd
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "none".to_string()),
        preview
            .current_block
            .as_ref()
            .and_then(|block| block.command.as_deref())
            .unwrap_or("none"),
        preview.selection.as_deref().unwrap_or("none"),
        preview.explicit_files.len(),
    );
    gui.app.record_agent_read(&ProviderRequestPreview {
        kind: "cli",
        endpoint: None,
        model: None,
        command: vec!["sh".to_string(), "-lc".to_string(), "echo".to_string()],
        prompt_chars: prompt.chars().count(),
    });
    gui.app.set_agent_command_proposals(vec![CommandProposal {
        command: review_output_command("NOCTRAIL_AUDIT_EXECUTE"),
        reason: "Verify the shell remains interactive after reviewed execution.".to_string(),
        risk: CommandRisk::Low,
        permission: CommandPermission::Review,
    }]);
    gui.toggle_review_panel();
    let _ = gui.confirm_review_selection()?;
    gui.toggle_agent_audit_browser();

    let title = gui.title_text();
    if !title.contains("agent-audit") || !title.contains("execute") {
        return Err("audit browser title did not expose the latest execution entry".into());
    }

    let kinds = gui
        .app
        .agent_audit_entries()
        .iter()
        .map(|entry| entry.kind.label())
        .collect::<Vec<_>>();
    for required in ["context", "read", "suggest", "review", "execute"] {
        if !kinds.contains(&required) {
            return Err(format!("audit ledger did not record {required}").into());
        }
    }

    gui.app.write_input(shell_exit_bytes().as_slice())?;
    std::thread::sleep(Duration::from_millis(100));
    let output = read_all_runtime_output_for_gui(&mut gui.app)?;
    let _ = gui.app.close_runtime()?;
    let text = String::from_utf8_lossy(&output);
    if !text.contains("NOCTRAIL_AUDIT_EXECUTE") {
        return Err("audit smoke did not preserve reviewed shell execution".into());
    }

    println!("audit_entries={} latest={}", kinds.len(), kinds.join(","));
    println!("agent audit smoke ok");
    Ok(())
}

pub fn review_panel_smoke() -> Result<(), Box<dyn Error>> {
    let app = DesktopApp::spawn_shell(LayoutRect::new(0, 0, 120, 80), PtySize::new(80, 24))?;
    let mut gui = GuiApp::new(app, GuiLaunchOptions::default());
    let high_marker =
        std::env::temp_dir().join(format!("noctrail-review-high-{}", std::process::id()));
    let _ = std::fs::remove_file(&high_marker);

    gui.app
        .set_agent_command_proposals(vec![noctrail_agent::CommandProposal {
            command: review_output_command("NOCTRAIL_REVIEW_LOW"),
            reason: "Inspect the shell before changing files.".to_string(),
            risk: CommandRisk::Low,
            permission: noctrail_agent::CommandPermission::Review,
        }]);
    gui.toggle_review_panel();
    let _ = gui.confirm_review_selection()?;

    gui.app
        .set_agent_command_proposals(vec![noctrail_agent::CommandProposal {
            command: review_file_command(&high_marker),
            reason: "Delete or rewrite shell-visible state.".to_string(),
            risk: CommandRisk::High,
            permission: noctrail_agent::CommandPermission::StrongReview,
        }]);
    gui.toggle_review_panel();
    let _ = gui.confirm_review_selection()?;
    if high_marker.exists() {
        let _ = std::fs::remove_file(&high_marker);
        return Err("high-risk proposal executed before strong confirmation".into());
    }
    let _ = gui.confirm_review_with_text("y")?;

    gui.app.write_input(
        shell_submission_bytes(&review_output_command("NOCTRAIL_REVIEW_DONE")).as_slice(),
    )?;
    gui.app.write_input(shell_exit_bytes().as_slice())?;
    std::thread::sleep(Duration::from_millis(100));
    let output = read_all_runtime_output_for_gui(&mut gui.app)?;
    let _ = gui.app.close_runtime()?;

    let text = String::from_utf8_lossy(&output);
    if !text.contains("NOCTRAIL_REVIEW_LOW") {
        return Err("low-risk review confirmation did not reach the shell".into());
    }
    if !text.contains("NOCTRAIL_REVIEW_DONE") {
        return Err("review smoke did not preserve shell output after execution".into());
    }
    if !high_marker.exists() {
        return Err("high-risk review confirmation did not execute after strong confirm".into());
    }
    let _ = std::fs::remove_file(&high_marker);

    println!("low=review high=strong-review shell=ok");
    println!("agent review smoke ok");
    Ok(())
}

pub fn patch_preview_smoke() -> Result<(), Box<dyn Error>> {
    let app = DesktopApp::spawn_shell(LayoutRect::new(0, 0, 120, 80), PtySize::new(80, 24))?;
    let mut gui = GuiApp::new(app, GuiLaunchOptions::default());
    let original_path =
        std::env::temp_dir().join(format!("noctrail-patch-preview-{}.txt", std::process::id()));
    std::fs::write(&original_path, "original\n")?;
    let fixture_path = std::env::temp_dir().join(format!(
        "noctrail-patch-preview-payload-{}.json",
        std::process::id()
    ));

    let diff = format!(
        "--- a/{0}\n+++ b/{0}\n@@ -1 +1 @@\n-original\n+patched\n",
        original_path.display()
    );
    std::fs::write(
        &fixture_path,
        serde_json::json!({
            "patches": [
                {
                    "path": original_path.display().to_string(),
                    "reason": "Preview a one-line patch without applying it.",
                    "diff": diff
                }
            ]
        })
        .to_string(),
    )?;

    let mut preview_app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(80, 24));
    preview_app.set_block_observer_enabled(true);
    preview_app.advance_output(&shell_integration_probe_bytes(
        "cargo test -p noctrail-app",
        "/tmp/noctrail-agent-patch",
        0,
        21,
        "patch context\n",
    ));
    let _ = preview_app.select_newest_command_block();
    preview_app.set_agent_explicit_files(vec![original_path.clone()]);
    let prompt =
        crate::redaction::redact_agent_context_preview(&preview_app.agent_context_preview());
    let prompt = format!(
        "cwd: {}\nfiles:\n- {}\ncommand: {}\noutput:\n{}",
        prompt
            .cwd
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "/tmp/noctrail-agent-patch".to_string()),
        original_path.display(),
        prompt
            .current_block
            .as_ref()
            .and_then(|block| block.command.as_deref())
            .unwrap_or("cargo test -p noctrail-app"),
        prompt
            .current_block
            .as_ref()
            .map(|block| block.output.as_str())
            .unwrap_or("")
    );

    let adapter = noctrail_agent::ProviderAdapter::from_provider_config(
        &noctrail_config::AgentProviderConfig {
            kind: noctrail_config::AgentProviderKind::Cli,
            model: None,
            endpoint: None,
            command: review_patch_cli_command(&fixture_path),
        },
    )?;
    let previews = adapter.propose_patches(&prompt)?;
    gui.app.set_agent_patch_previews(previews);
    gui.toggle_patch_preview_browser();
    let title = gui.title_text();
    if !title.contains("patch-preview") || !title.contains("diff --- a/") {
        return Err("patch preview title did not expose the unified diff".into());
    }

    gui.app.write_input(
        shell_submission_bytes(&review_output_command("NOCTRAIL_PATCH_PREVIEW_OK")).as_slice(),
    )?;
    gui.app.write_input(shell_exit_bytes().as_slice())?;
    std::thread::sleep(Duration::from_millis(100));
    let output = read_all_runtime_output_for_gui(&mut gui.app)?;
    let _ = gui.app.close_runtime()?;

    let text = String::from_utf8_lossy(&output);
    if !text.contains("NOCTRAIL_PATCH_PREVIEW_OK") {
        return Err("patch preview flow broke foreground shell output".into());
    }
    let contents = std::fs::read_to_string(&original_path)?;
    if contents != "original\n" {
        return Err("patch preview unexpectedly modified the target file".into());
    }

    let _ = std::fs::remove_file(&original_path);
    let _ = std::fs::remove_file(&fixture_path);
    println!("patches=1 file_unchanged=yes shell=ok");
    println!("agent patch preview smoke ok");
    Ok(())
}

pub fn run_with_options(options: GuiLaunchOptions) -> Result<(), Box<dyn Error>> {
    let event_loop = EventLoop::<GuiEvent>::with_user_event().build()?;
    let event_proxy = event_loop.create_proxy();
    let initial_surface = LayoutRect::new(
        0,
        0,
        DEFAULT_WINDOW_WIDTH as u16,
        DEFAULT_WINDOW_HEIGHT as u16,
    );
    let initial_terminal = terminal_size_from_surface(
        PhysicalSize::new(DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT),
        &options.font,
        1.0,
    );
    let app = DesktopApp::spawn_shell(initial_surface, initial_terminal)?;
    let mut gui = GuiApp::new(app, options);
    gui.set_event_proxy(event_proxy);
    event_loop.run_app(&mut gui)?;
    Ok(())
}

pub(crate) fn terminal_size_from_surface(
    size: PhysicalSize<u32>,
    font: &FontConfig,
    scale_factor: f64,
) -> PtySize {
    let cell = cell_dimensions(font);
    let scale = (scale_factor as f32).max(1.0);
    let logical_width = size.width as f32 / scale;
    let logical_height = size.height as f32 / scale;
    let cols = ((logical_width / cell.width).floor() as u32).max(1);
    let rows = ((logical_height / cell.height).floor() as u32).max(1);

    PtySize::new(saturating_u32_to_u16(cols), saturating_u32_to_u16(rows))
}

pub(crate) fn layout_rect_from_surface(size: PhysicalSize<u32>) -> LayoutRect {
    LayoutRect::new(
        0,
        0,
        saturating_u32_to_u16(size.width),
        saturating_u32_to_u16(size.height),
    )
}

fn cell_dimensions(font: &FontConfig) -> CellDimensions {
    CellDimensions {
        width: (font.size * 0.62).max(8.0),
        height: (font.size * font.line_height).max((font.size + 6.0).max(18.0)),
    }
}

pub(crate) fn frame_title(frame: &DesktopFrame, cursor_visible: bool) -> String {
    let backend = match frame.render_plan.backend {
        noctrail_render::RenderBackend::Gpu => "gpu",
        noctrail_render::RenderBackend::Software => "software",
    };
    let pid = frame
        .process_id
        .map(|pid| pid.to_string())
        .unwrap_or_else(|| "starting".to_string());
    let cursor = if cursor_visible { "on" } else { "off" };
    let pane_label = if frame.is_scratch {
        format!("scratch {}", frame.pane_id.0)
    } else {
        format!("pane {}", frame.pane_id.0)
    };

    let mut title = format!(
        "Noctrail | {pane_label} | pid {pid} | {}x{} px | {}x{} cells | rows {} | {backend} | cursor {cursor}",
        frame.surface.width,
        frame.surface.height,
        frame.terminal_size.cols,
        frame.terminal_size.rows,
        frame.render_plan.rows.len(),
    );
    if let Some(shell) = frame.status_line.shell.as_deref() {
        title.push_str(" | shell ");
        title.push_str(shell);
    }
    if let Some(cwd) = frame.status_line.cwd.as_deref() {
        title.push_str(" | cwd ");
        title.push_str(&display_status_path(cwd));
    }
    if let Some(branch) = frame.status_line.git_branch.as_deref() {
        title.push_str(" | git ");
        title.push_str(branch);
    }
    if let Some(exit_status) = frame.status_line.exit_status.as_deref() {
        title.push_str(" | exit ");
        title.push_str(exit_status);
    }

    title
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StatusRuns {
    left: Vec<StatusRun>,
    right: Vec<StatusRun>,
}

fn compose_status_runs(
    frame: &DesktopFrame,
    cols: usize,
    palette: StatusBarPalette,
    input_mode: InputMode,
) -> StatusRuns {
    let shell = frame.status_line.shell.as_deref().unwrap_or("shell");
    let shell_label = shell.to_ascii_uppercase();
    let cwd = frame
        .status_line
        .cwd
        .as_deref()
        .map(display_status_path)
        .unwrap_or_else(|| "workspace".to_string());
    let branch = frame
        .status_line
        .git_branch
        .as_deref()
        .map(|branch| format!("git:{branch}"));
    let pane_label = if frame.is_scratch {
        "SCRATCH".to_string()
    } else {
        format!("WS{}", frame.workspace_id.0)
    };

    let mut right = Vec::new();

    if let Some(exit_status) = frame.status_line.exit_status.as_deref() {
        let exit_background = if exit_status == "code 0" {
            palette.accent
        } else {
            palette.chip_danger_background
        };
        right.push(status_chip(
            exit_status,
            chip_foreground(exit_background),
            exit_background,
            true,
        ));
        right.push(StatusRun {
            text: "  ".to_string(),
            style: status_style(palette.muted, None, false),
        });
    }

    right.extend([
        status_chip(
            input_mode.label(),
            if input_mode == InputMode::Editor {
                chip_foreground(palette.chip_accent_background)
            } else {
                palette.foreground
            },
            if input_mode == InputMode::Editor {
                palette.chip_accent_background
            } else {
                palette.chip_background
            },
            input_mode == InputMode::Editor,
        ),
        StatusRun {
            text: "  ".to_string(),
            style: status_style(palette.muted, None, false),
        },
        status_chip(
            &pane_label,
            palette.foreground,
            palette.chip_background,
            false,
        ),
        StatusRun {
            text: "  ".to_string(),
            style: status_style(palette.muted, None, false),
        },
        status_chip(
            &format!("{}x{}", frame.terminal_size.cols, frame.terminal_size.rows),
            palette.foreground,
            palette.chip_background,
            false,
        ),
    ]);

    let right_len = status_runs_len(&right);
    let available_left = cols.saturating_sub(right_len.saturating_add(1));
    let branch_len = branch
        .as_ref()
        .map_or(0, |branch| branch.chars().count() + 4);
    let mut left = vec![status_chip(
        &shell_label,
        chip_foreground(palette.chip_accent_background),
        palette.chip_accent_background,
        true,
    )];
    let shell_len = status_runs_len(&left);

    if available_left > shell_len.saturating_add(2) {
        let reserve_branch = if branch.is_some() && available_left > shell_len.saturating_add(14) {
            branch_len
        } else {
            0
        };
        let cwd_budget = available_left
            .saturating_sub(shell_len)
            .saturating_sub(3)
            .saturating_sub(reserve_branch);
        left.push(StatusRun {
            text: "  ".to_string(),
            style: status_style(palette.muted, None, false),
        });
        let cwd_label = truncate_middle(&cwd, cwd_budget.saturating_sub(2).max(1));
        left.push(status_chip(
            &cwd_label,
            palette.foreground,
            palette.chip_background,
            false,
        ));

        if let Some(branch) = branch
            && status_runs_len(&left).saturating_add(branch_len) < available_left
        {
            left.push(StatusRun {
                text: "  ".to_string(),
                style: status_style(palette.muted, None, false),
            });
            left.push(status_chip(
                &branch,
                palette.accent,
                palette.chip_background,
                false,
            ));
        }
    }

    StatusRuns { left, right }
}

fn compose_status_row(left: &[StatusRun], right: &[StatusRun], cols: usize) -> Vec<RenderGlyph> {
    let mut glyphs = render_glyphs_for_runs(left, 0, cols);
    let right_len = status_runs_len(right);
    let start = cols.saturating_sub(right_len);
    glyphs.extend(render_glyphs_for_runs(right, start, cols));
    glyphs
}

fn render_glyphs_for_runs(runs: &[StatusRun], start_col: usize, cols: usize) -> Vec<RenderGlyph> {
    let mut glyphs = Vec::new();
    let mut col = start_col;
    for run in runs {
        for ch in run.text.chars() {
            if col >= cols {
                return glyphs;
            }
            if ch != ' ' || run.style.background != Color::Default {
                glyphs.push(RenderGlyph {
                    col,
                    text: ch.to_string(),
                    style: run.style,
                    span: 1,
                    wide_continuation: false,
                });
            }
            col += 1;
        }
    }
    glyphs
}

fn status_runs_len(runs: &[StatusRun]) -> usize {
    runs.iter().map(|run| run.text.chars().count()).sum()
}

fn status_style(color: Rgba, background: Option<Rgba>, bold: bool) -> Style {
    Style {
        foreground: Color::Rgb(color.red, color.green, color.blue),
        background: background
            .map(|background| Color::Rgb(background.red, background.green, background.blue))
            .unwrap_or(Color::Default),
        bold,
        italic: false,
        underline: false,
    }
}

fn status_chip(text: &str, foreground: Rgba, background: Rgba, bold: bool) -> StatusRun {
    StatusRun {
        text: format!(" {text} "),
        style: status_style(foreground, Some(background), bold),
    }
}

fn truncate_middle(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    if max_chars <= 3 {
        return "...".chars().take(max_chars).collect();
    }

    let head = (max_chars - 3) / 2;
    let tail = max_chars.saturating_sub(head + 3);
    let prefix = text.chars().take(head).collect::<String>();
    let suffix = text
        .chars()
        .rev()
        .take(tail)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{prefix}...{suffix}")
}

fn normalize_editor_text(text: &str) -> String {
    text.replace("\r\n", " ")
        .replace(['\r', '\n'], " ")
}

fn nth_char_boundary(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .map(|(index, _)| index)
        .nth(char_index)
        .unwrap_or(text.len())
}

fn previous_word_boundary(text: &str, cursor_chars: usize) -> usize {
    let chars = text.chars().collect::<Vec<_>>();
    if chars.is_empty() || cursor_chars == 0 {
        return 0;
    }
    let mut index = cursor_chars.min(chars.len());
    while index > 0 && chars[index - 1].is_whitespace() {
        index -= 1;
    }
    while index > 0 && !chars[index - 1].is_whitespace() {
        index -= 1;
    }
    index
}

fn next_word_boundary(text: &str, cursor_chars: usize) -> usize {
    let chars = text.chars().collect::<Vec<_>>();
    let mut index = cursor_chars.min(chars.len());
    while index < chars.len() && chars[index].is_whitespace() {
        index += 1;
    }
    while index < chars.len() && !chars[index].is_whitespace() {
        index += 1;
    }
    index
}

fn mix_rgba(foreground: Rgba, background: Rgba, background_ratio: f32) -> Rgba {
    let foreground_ratio = (1.0 - background_ratio).clamp(0.0, 1.0);
    let background_ratio = background_ratio.clamp(0.0, 1.0);
    Rgba {
        red: ((f32::from(foreground.red) * foreground_ratio)
            + (f32::from(background.red) * background_ratio))
            .round() as u8,
        green: ((f32::from(foreground.green) * foreground_ratio)
            + (f32::from(background.green) * background_ratio))
            .round() as u8,
        blue: ((f32::from(foreground.blue) * foreground_ratio)
            + (f32::from(background.blue) * background_ratio))
            .round() as u8,
        alpha: u8::MAX,
    }
}

fn rgba_from_hex(rgb: u32) -> Rgba {
    Rgba::opaque(
        ((rgb >> 16) & 0xff) as u8,
        ((rgb >> 8) & 0xff) as u8,
        (rgb & 0xff) as u8,
    )
}

fn chip_foreground(background: Rgba) -> Rgba {
    let brightness = (0.299 * f32::from(background.red)
        + 0.587 * f32::from(background.green)
        + 0.114 * f32::from(background.blue))
        / 255.0;
    if brightness >= 0.52 {
        rgba_from_hex(0x091017)
    } else {
        rgba_from_hex(0xf5f7fb)
    }
}

fn blit_software_frame(
    target: &mut noctrail_render::SoftwareRenderFrame,
    overlay: &noctrail_render::SoftwareRenderFrame,
    origin_x: usize,
    origin_y: usize,
) {
    for row in 0..overlay.height as usize {
        for col in 0..overlay.width as usize {
            let dst_x = origin_x.saturating_add(col);
            let dst_y = origin_y.saturating_add(row);
            if dst_x >= target.width as usize || dst_y >= target.height as usize {
                continue;
            }
            let src_index = (row * overlay.width as usize + col) * 4;
            let dst_index = (dst_y * target.width as usize + dst_x) * 4;
            target.pixels[dst_index..dst_index + 4]
                .copy_from_slice(&overlay.pixels[src_index..src_index + 4]);
        }
    }
}

fn solid_software_frame(
    width: u32,
    height: u32,
    color: Rgba,
) -> noctrail_render::SoftwareRenderFrame {
    let mut pixels = vec![0; width as usize * height as usize * 4];
    for pixel in pixels.chunks_exact_mut(4) {
        pixel[0] = color.red;
        pixel[1] = color.green;
        pixel[2] = color.blue;
        pixel[3] = color.alpha;
    }
    noctrail_render::SoftwareRenderFrame {
        width,
        height,
        pixels,
        stats: noctrail_render::FrameStats::default(),
    }
}

fn display_status_path(path: &Path) -> String {
    let compact = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home| path.starts_with(home))
        .and_then(|home| path.strip_prefix(home).ok())
        .map(|suffix| {
            if suffix.as_os_str().is_empty() {
                "~".to_string()
            } else {
                format!("~/{}", suffix.display())
            }
        })
        .unwrap_or_else(|| path.display().to_string());

    let parts = compact.split('/').collect::<Vec<_>>();
    if parts.len() <= 4 {
        compact
    } else if compact.starts_with("~/") {
        format!("~/{}/.../{}", parts[1], parts[parts.len() - 1])
    } else {
        format!("{}/.../{}", parts[0], parts[parts.len() - 1])
    }
}

fn saturating_u32_to_u16(value: u32) -> u16 {
    value.min(u16::MAX as u32) as u16
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct MouseSelectionDrag {
    anchor: Position,
    cursor: Position,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransitionKind {
    Pane,
    Workspace,
}

impl TransitionKind {
    fn label(self) -> &'static str {
        match self {
            Self::Pane => "pane",
            Self::Workspace => "workspace",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TransitionRect {
    pane_id: noctrail_runtime::PaneId,
    from: Option<LayoutRect>,
    to: Option<LayoutRect>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct TransitionSnapshot {
    workspace_id: Option<WorkspaceId>,
    scratch_visible: bool,
    panes: Vec<(noctrail_runtime::PaneId, LayoutRect)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveTransition {
    kind: TransitionKind,
    started_at: Instant,
    next_frame_at: Instant,
    duration: Duration,
    panes: Vec<TransitionRect>,
}

impl ActiveTransition {
    fn deadline(&self) -> Instant {
        self.started_at + self.duration
    }

    fn progress(&self, now: Instant) -> f32 {
        let total = self.duration.as_secs_f32();
        if total <= 0.0 {
            return 1.0;
        }
        ((now - self.started_at).as_secs_f32() / total).clamp(0.0, 1.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaletteCommand {
    NewPane,
    SplitHorizontal,
    SplitVertical,
    Focus(FocusDirection),
    Resize(FocusDirection),
    Swap(FocusDirection),
    ClosePane,
    Workspace(WorkspaceId),
    ToggleScratch,
}

impl PaletteCommand {
    fn all() -> Vec<Self> {
        let mut commands = vec![
            Self::NewPane,
            Self::SplitHorizontal,
            Self::SplitVertical,
            Self::Focus(FocusDirection::Left),
            Self::Focus(FocusDirection::Right),
            Self::Focus(FocusDirection::Up),
            Self::Focus(FocusDirection::Down),
            Self::Resize(FocusDirection::Left),
            Self::Resize(FocusDirection::Right),
            Self::Resize(FocusDirection::Up),
            Self::Resize(FocusDirection::Down),
            Self::Swap(FocusDirection::Left),
            Self::Swap(FocusDirection::Right),
            Self::Swap(FocusDirection::Up),
            Self::Swap(FocusDirection::Down),
            Self::ClosePane,
            Self::ToggleScratch,
        ];
        commands.extend(
            (WorkspaceId::MIN..=WorkspaceId::MAX).map(|id| Self::Workspace(WorkspaceId::new(id))),
        );
        commands
    }

    fn label(self) -> String {
        match self {
            Self::NewPane => "new pane".to_string(),
            Self::SplitHorizontal => "split horizontal".to_string(),
            Self::SplitVertical => "split vertical".to_string(),
            Self::Focus(direction) => format!("focus {}", direction_name(direction)),
            Self::Resize(direction) => format!("resize {}", direction_name(direction)),
            Self::Swap(direction) => format!("move {}", direction_name(direction)),
            Self::ClosePane => "close pane".to_string(),
            Self::Workspace(workspace_id) => format!("workspace {}", workspace_id.0),
            Self::ToggleScratch => "scratch show hide".to_string(),
        }
    }

    fn haystack(self) -> String {
        match self {
            Self::NewPane => "new pane split create shell".to_string(),
            Self::SplitHorizontal => "split horizontal top bottom".to_string(),
            Self::SplitVertical => "split vertical left right".to_string(),
            Self::Focus(direction) => format!("focus {}", direction_name(direction)),
            Self::Resize(direction) => format!("resize {}", direction_name(direction)),
            Self::Swap(direction) => format!("move swap pane {}", direction_name(direction)),
            Self::ClosePane => "close pane kill active".to_string(),
            Self::Workspace(workspace_id) => {
                format!("workspace {} switch session", workspace_id.0)
            }
            Self::ToggleScratch => "scratch show hide dropdown terminal".to_string(),
        }
    }

    fn matches_query(self, query: &str) -> bool {
        if query.trim().is_empty() {
            return true;
        }

        let haystack = self.haystack();
        query
            .split_whitespace()
            .all(|token| haystack.contains(&token.to_ascii_lowercase()))
    }

    fn execute(self, app: &mut DesktopApp, resize_step: u16) -> Result<(), Box<dyn Error>> {
        match self {
            Self::NewPane => {
                let _ = app.split_active_pane_shell()?;
            }
            Self::SplitHorizontal => {
                let _ = app.split_active_pane_shell_with_axis(SplitAxis::Horizontal)?;
            }
            Self::SplitVertical => {
                let _ = app.split_active_pane_shell_with_axis(SplitAxis::Vertical)?;
            }
            Self::Focus(direction) => {
                let _ = app.focus_direction(direction)?;
            }
            Self::Resize(direction) => {
                app.resize_active_split(direction, resize_step)?;
            }
            Self::Swap(direction) => {
                let _ = app.swap_active_pane(direction)?;
            }
            Self::ClosePane => {
                let _ = app.close_active_pane()?;
            }
            Self::Workspace(workspace_id) => {
                let _ = app.switch_workspace(workspace_id)?;
            }
            Self::ToggleScratch => {
                let _ = app.toggle_scratch()?;
            }
        }

        Ok(())
    }

    fn transition_kind(self) -> Option<TransitionKind> {
        match self {
            Self::NewPane
            | Self::SplitHorizontal
            | Self::SplitVertical
            | Self::Resize(_)
            | Self::Swap(_)
            | Self::ClosePane
            | Self::ToggleScratch => Some(TransitionKind::Pane),
            Self::Workspace(_) => Some(TransitionKind::Workspace),
            Self::Focus(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandPalette {
    query: String,
    selected: usize,
}

impl CommandPalette {
    fn new() -> Self {
        Self {
            query: String::new(),
            selected: 0,
        }
    }

    fn filtered_commands(&self) -> Vec<PaletteCommand> {
        PaletteCommand::all()
            .into_iter()
            .filter(|command| command.matches_query(&self.query))
            .collect()
    }

    fn selected_command(&self) -> Option<PaletteCommand> {
        let commands = self.filtered_commands();
        commands
            .get(self.selected.min(commands.len().saturating_sub(1)))
            .copied()
    }

    fn push_query_text(&mut self, text: &str) {
        for ch in text.chars().filter(|ch| !ch.is_control()) {
            self.query.push(ch);
        }
        self.selected = 0;
    }

    fn pop_query_char(&mut self) {
        self.query.pop();
        self.selected = 0;
    }

    fn select_next(&mut self) {
        let len = self.filtered_commands().len();
        if len > 0 {
            self.selected = (self.selected + 1) % len;
        }
    }

    fn select_previous(&mut self) {
        let len = self.filtered_commands().len();
        if len > 0 {
            self.selected = (self.selected + len - 1) % len;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct AgentContextBrowser;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct AgentAuditBrowser;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct PatchPreviewBrowser;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct BlockBrowser;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct ReviewPanel {
    strong_confirm_index: Option<usize>,
}

struct GuiApp {
    app: DesktopApp,
    launch_options: GuiLaunchOptions,
    event_proxy: Option<EventLoopProxy<GuiEvent>>,
    config_reloader: Option<ConfigReloader>,
    window: Option<Arc<Window>>,
    renderer: Option<GpuRenderer>,
    gpu_fallback_error: Option<String>,
    theme_reload_error: Option<String>,
    theme: ThemeConfig,
    font: FontConfig,
    font_preferences: FontPreferences,
    ime_preedit: Option<String>,
    agent_audit_browser: Option<AgentAuditBrowser>,
    agent_context_browser: Option<AgentContextBrowser>,
    patch_preview_browser: Option<PatchPreviewBrowser>,
    review_panel: Option<ReviewPanel>,
    block_browser: Option<BlockBrowser>,
    command_palette: Option<CommandPalette>,
    mouse_position: Option<PhysicalPosition<f64>>,
    mouse_selection: Option<MouseSelectionDrag>,
    mouse_button: Option<input::MouseButton>,
    output_thread: Option<JoinHandle<()>>,
    transition: Option<ActiveTransition>,
    started_at: Instant,
    last_frame_log_at: Option<Instant>,
    last_frame_log_signature: Option<FrameLogSignature>,
    next_cursor_blink_at: Instant,
    cursor_visible: bool,
    frame_interval: Duration,
    window_focused: bool,
    startup_focus_retry_until: Option<Instant>,
    next_startup_focus_retry_at: Instant,
    cached_software_frame: Option<noctrail_render::SoftwareRenderFrame>,
    requested_input_mode: InputMode,
    editor_states: HashMap<PaneId, EditorBufferState>,
    latency_probe: Option<LatencyProbe>,
    modifiers: ModifiersState,
    clipboard: ClipboardBridge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FrameLogSignature {
    full_frame: bool,
    dirty_rows: usize,
    glyphs: usize,
    paint_rects: usize,
    border_segments: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct CellDimensions {
    width: f32,
    height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StatusBarPalette {
    background: Rgba,
    foreground: Rgba,
    muted: Rgba,
    accent: Rgba,
    danger: Rgba,
    chip_background: Rgba,
    chip_accent_background: Rgba,
    chip_danger_background: Rgba,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StatusRun {
    text: String,
    style: Style,
}

#[derive(Debug)]
struct LatencyProbe {
    output_path: PathBuf,
    pending_started_at: Option<Instant>,
    pending_kind: Option<&'static str>,
    samples: Vec<LatencyProbeSample>,
}

#[derive(Debug)]
struct LatencyProbeSample {
    kind: &'static str,
    latency_ms: f64,
}

impl LatencyProbe {
    fn from_env() -> Option<Self> {
        let output_path = env::var_os("NOCTRAIL_INPUT_LATENCY_LOG")?;
        Some(Self {
            output_path: PathBuf::from(output_path),
            pending_started_at: None,
            pending_kind: None,
            samples: Vec::new(),
        })
    }

    fn begin(&mut self, kind: &'static str) {
        self.pending_started_at = Some(Instant::now());
        self.pending_kind = Some(kind);
    }

    fn finish(&mut self) {
        let Some(started_at) = self.pending_started_at.take() else {
            return;
        };
        let kind = self.pending_kind.take().unwrap_or("unknown");
        self.samples.push(LatencyProbeSample {
            kind,
            latency_ms: started_at.elapsed().as_secs_f64() * 1000.0,
        });
    }

    fn flush(&self) {
        let mut values = self
            .samples
            .iter()
            .map(|sample| sample.latency_ms)
            .collect::<Vec<_>>();
        values.sort_by(|left, right| left.total_cmp(right));
        let average = if values.is_empty() {
            None
        } else {
            Some(values.iter().sum::<f64>() / values.len() as f64)
        };
        let payload = serde_json::json!({
            "samples": self.samples.iter().map(|sample| serde_json::json!({
                "kind": sample.kind,
                "latency_ms": sample.latency_ms,
            })).collect::<Vec<_>>(),
            "summary": {
                "count": values.len(),
                "p50_ms": percentile(&values, 0.5),
                "p95_ms": percentile(&values, 0.95),
                "avg_ms": average,
            }
        });
        let _ = fs::write(
            &self.output_path,
            serde_json::to_vec_pretty(&payload).unwrap_or_default(),
        );
    }
}

fn percentile(values: &[f64], ratio: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let index = ((values.len() - 1) as f64 * ratio).ceil() as usize;
    values.get(index.min(values.len() - 1)).copied()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputMode {
    Editor,
    Terminal,
}

impl InputMode {
    fn label(self) -> &'static str {
        match self {
            Self::Editor => "EDIT",
            Self::Terminal => "TERM",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct EditorBufferState {
    text: String,
    cursor_chars: usize,
    history: Vec<String>,
    history_index: Option<usize>,
    history_restore: Option<String>,
    anchor: Option<Cursor>,
}

impl EditorBufferState {
    fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    fn clear(&mut self) {
        self.text.clear();
        self.cursor_chars = 0;
        self.history_index = None;
        self.history_restore = None;
        self.anchor = None;
    }

    fn sync_anchor(&mut self, anchor: Cursor) {
        if self.is_empty() {
            self.anchor = Some(anchor);
        }
    }

    fn cursor_byte_index(&self) -> usize {
        nth_char_boundary(&self.text, self.cursor_chars)
    }

    fn char_len(&self) -> usize {
        self.text.chars().count()
    }

    fn set_text(&mut self, text: String) {
        self.text = normalize_editor_text(&text);
        self.cursor_chars = self.char_len();
    }

    fn insert_text(&mut self, text: &str) {
        let text = normalize_editor_text(text);
        if text.is_empty() {
            return;
        }
        let byte_index = self.cursor_byte_index();
        self.text.insert_str(byte_index, &text);
        self.cursor_chars += text.chars().count();
        self.history_index = None;
        self.history_restore = None;
    }

    fn move_left(&mut self) {
        self.cursor_chars = self.cursor_chars.saturating_sub(1);
    }

    fn move_right(&mut self) {
        self.cursor_chars = (self.cursor_chars + 1).min(self.char_len());
    }

    fn move_home(&mut self) {
        self.cursor_chars = 0;
    }

    fn move_end(&mut self) {
        self.cursor_chars = self.char_len();
    }

    fn move_word_left(&mut self) {
        self.cursor_chars = previous_word_boundary(&self.text, self.cursor_chars);
    }

    fn move_word_right(&mut self) {
        self.cursor_chars = next_word_boundary(&self.text, self.cursor_chars);
    }

    fn delete_backward(&mut self) {
        if self.cursor_chars == 0 {
            return;
        }
        let end = self.cursor_byte_index();
        let start = nth_char_boundary(&self.text, self.cursor_chars - 1);
        self.text.replace_range(start..end, "");
        self.cursor_chars -= 1;
        self.history_index = None;
        self.history_restore = None;
    }

    fn delete_forward(&mut self) {
        if self.cursor_chars >= self.char_len() {
            return;
        }
        let start = self.cursor_byte_index();
        let end = nth_char_boundary(&self.text, self.cursor_chars + 1);
        self.text.replace_range(start..end, "");
        self.history_index = None;
        self.history_restore = None;
    }

    fn delete_to_start(&mut self) {
        let end = self.cursor_byte_index();
        self.text.replace_range(0..end, "");
        self.cursor_chars = 0;
        self.history_index = None;
        self.history_restore = None;
    }

    fn delete_to_end(&mut self) {
        let start = self.cursor_byte_index();
        self.text.truncate(start);
        self.history_index = None;
        self.history_restore = None;
    }

    fn delete_previous_word(&mut self) {
        let start_chars = previous_word_boundary(&self.text, self.cursor_chars);
        let start = nth_char_boundary(&self.text, start_chars);
        let end = self.cursor_byte_index();
        self.text.replace_range(start..end, "");
        self.cursor_chars = start_chars;
        self.history_index = None;
        self.history_restore = None;
    }

    fn push_history(&mut self, command: &str) {
        let command = command.trim();
        if command.is_empty() {
            return;
        }
        if self.history.last().is_some_and(|last| last == command) {
            return;
        }
        self.history.push(command.to_string());
        if self.history.len() > 200 {
            let overflow = self.history.len() - 200;
            self.history.drain(0..overflow);
        }
        self.history_index = None;
        self.history_restore = None;
    }

    fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let next_index = match self.history_index {
            Some(index) => index.saturating_sub(1),
            None => {
                self.history_restore = Some(self.text.clone());
                self.history.len() - 1
            }
        };
        self.history_index = Some(next_index);
        self.set_text(self.history[next_index].clone());
    }

    fn history_next(&mut self) {
        let Some(index) = self.history_index else {
            return;
        };
        if index + 1 < self.history.len() {
            let next_index = index + 1;
            self.history_index = Some(next_index);
            self.set_text(self.history[next_index].clone());
            return;
        }
        let restore = self.history_restore.take().unwrap_or_default();
        self.history_index = None;
        self.set_text(restore);
    }
}

impl GuiApp {
    fn new(mut app: DesktopApp, launch_options: GuiLaunchOptions) -> Self {
        let now = Instant::now();
        let config_reloader = launch_options
            .config_path
            .as_ref()
            .and_then(|path| ConfigReloader::from_path(path).ok());
        let theme = launch_options.theme.clone();
        let font = launch_options.font.clone();
        app.set_default_split_axis(split_axis_from_config(
            launch_options.layout.default_split_axis,
        ));
        app.set_scratch_height_percent(launch_options.layout.scratch_height_percent)
            .expect("app should accept scratch height updates");
        app.set_pane_chrome(pane_chrome_from_theme(&theme, &font))
            .expect("app should accept pane chrome updates");
        if launch_options.layout.startup_workspace != 1 {
            let _ = app
                .switch_workspace(WorkspaceId::new(launch_options.layout.startup_workspace))
                .expect("validated startup workspace should switch cleanly");
        }
        apply_debug_startup_layout(&mut app);
        Self {
            app,
            launch_options,
            event_proxy: None,
            config_reloader,
            window: None,
            renderer: None,
            gpu_fallback_error: None,
            theme_reload_error: None,
            theme: theme.clone(),
            font: font.clone(),
            font_preferences: font_preferences_from_config(&font),
            ime_preedit: None,
            agent_audit_browser: None,
            agent_context_browser: None,
            patch_preview_browser: None,
            review_panel: None,
            block_browser: None,
            command_palette: None,
            mouse_position: None,
            mouse_selection: None,
            mouse_button: None,
            output_thread: None,
            transition: None,
            started_at: now,
            last_frame_log_at: None,
            last_frame_log_signature: None,
            next_cursor_blink_at: now + Duration::from_millis(theme.cursor.blink_interval_ms),
            cursor_visible: true,
            frame_interval: Duration::from_millis(theme.cursor.blink_interval_ms),
            window_focused: false,
            startup_focus_retry_until: Some(now + STARTUP_FOCUS_RETRY_WINDOW),
            next_startup_focus_retry_at: now,
            cached_software_frame: None,
            requested_input_mode: InputMode::Editor,
            editor_states: HashMap::new(),
            latency_probe: LatencyProbe::from_env(),
            modifiers: ModifiersState::empty(),
            clipboard: ClipboardBridge::new(),
        }
    }

    fn set_event_proxy(&mut self, event_proxy: EventLoopProxy<GuiEvent>) {
        self.event_proxy = Some(event_proxy);
    }

    fn window_id(&self) -> Option<WindowId> {
        self.window.as_ref().map(|window| window.id())
    }

    fn attach_output_pump(&mut self) -> Result<(), Box<dyn Error>> {
        if self.output_thread.is_some() {
            return Ok(());
        }

        let Some(runtime) = self.app.pane_mut().runtime_mut() else {
            return Ok(());
        };
        let Some(rx) = runtime.take_output_notification_receiver()? else {
            return Ok(());
        };
        if let Some(proxy) = self.event_proxy.clone() {
            self.output_thread = Some(thread::spawn(move || {
                while rx.recv().is_ok() {
                    if proxy.send_event(GuiEvent::RuntimeOutput).is_err() {
                        break;
                    }
                }
            }));
        }
        info!("attached PTY output pump");
        Ok(())
    }

    fn should_attempt_gpu_renderer(&self) -> bool {
        !self.launch_options.safe_mode && self.launch_options.renderer_backend == RenderBackend::Gpu
    }

    fn low_power_enabled(&self) -> bool {
        self.theme.low_power.enabled
    }

    fn animation_duration(&self) -> Option<Duration> {
        if self.theme.animation.enabled && !self.low_power_enabled() {
            Some(Duration::from_millis(self.theme.animation.duration_ms))
        } else {
            None
        }
    }

    fn transition_snapshot(&self) -> TransitionSnapshot {
        let mut panes = self
            .app
            .pane_layouts()
            .into_iter()
            .map(|layout| (layout.pane_id, layout.rect))
            .collect::<Vec<_>>();
        if self.app.scratch_visible()
            && let Some(scratch_id) = self.app.scratch_pane_id()
        {
            panes.push((scratch_id, self.app.frame().pane_surface));
        }
        panes.sort_by_key(|(pane_id, _)| pane_id.0);

        TransitionSnapshot {
            workspace_id: Some(self.app.active_workspace_id()),
            scratch_visible: self.app.scratch_visible(),
            panes,
        }
    }

    fn apply_palette_command(&mut self, command: PaletteCommand) -> Result<(), Box<dyn Error>> {
        let before = self.transition_snapshot();
        command.execute(&mut self.app, self.launch_options.layout.resize_step)?;
        self.start_transition(command.transition_kind(), before);
        Ok(())
    }

    fn shortcut_action(&self, logical_key: &winit::keyboard::Key) -> Option<input::ShortcutAction> {
        input::shortcut_action(logical_key, self.modifiers, &self.launch_options.keymap)
    }

    fn toggle_input_mode(&mut self) {
        self.requested_input_mode = match self.requested_input_mode {
            InputMode::Editor => InputMode::Terminal,
            InputMode::Terminal => InputMode::Editor,
        };
        self.touch_cursor_blink();
        self.request_redraw();
    }

    fn editor_state_mut(&mut self, pane_id: PaneId) -> &mut EditorBufferState {
        self.editor_states.entry(pane_id).or_default()
    }

    fn effective_input_mode(&self, frame: &DesktopFrame) -> InputMode {
        if self.requested_input_mode == InputMode::Terminal {
            return InputMode::Terminal;
        }

        let Some(pane) = self.app.pane_by_id(frame.pane_id) else {
            return InputMode::Terminal;
        };
        let shell = frame.status_line.shell.as_deref().unwrap_or_default();
        let shell_supported = matches!(
            shell.to_ascii_lowercase().as_str(),
            "pwsh.exe" | "pwsh" | "powershell.exe" | "powershell"
        );
        if !shell_supported {
            return InputMode::Terminal;
        }
        if frame.render_plan.alternate_screen {
            return InputMode::Terminal;
        }
        if pane.mouse_tracking_mode() != MouseTrackingMode::Disabled {
            return InputMode::Terminal;
        }
        if !pane.prompt_ready() {
            return InputMode::Terminal;
        }

        InputMode::Editor
    }

    fn active_input_mode(&self) -> InputMode {
        let frame = self.app.frame();
        self.effective_input_mode(&frame)
    }

    fn mark_latency_probe(&mut self, kind: &'static str) {
        if let Some(probe) = self.latency_probe.as_mut() {
            probe.begin(kind);
        }
    }

    fn complete_latency_probe(&mut self) {
        if let Some(probe) = self.latency_probe.as_mut() {
            probe.finish();
        }
    }

    fn present_now(&mut self) -> Result<(), Box<dyn Error>> {
        if let Err(error) = self.render_current_frame() {
            self.record_gpu_fallback(error.to_string());
        }
        self.complete_latency_probe();
        self.update_title();
        Ok(())
    }

    fn refresh_editor_feedback(
        &mut self,
        latency_kind: &'static str,
    ) -> Result<(), Box<dyn Error>> {
        self.mark_latency_probe(latency_kind);
        self.touch_cursor_blink();
        if self.present_editor_row_feedback()? {
            self.complete_latency_probe();
            self.update_title();
            Ok(())
        } else {
            self.present_now()
        }
    }

    fn present_editor_row_feedback(&mut self) -> Result<bool, Box<dyn Error>> {
        let Some(window) = self.window.as_ref() else {
            return Ok(false);
        };
        let scale_factor = window.scale_factor();
        let mut frame = self.app.frame();
        if self.effective_input_mode(&frame) != InputMode::Editor {
            return Ok(false);
        }
        self.apply_editor_overlay(&mut frame);
        let Some(row) = frame
            .render_plan
            .rows
            .get(frame.render_plan.cursor.row)
            .cloned()
        else {
            return Ok(false);
        };

        let config = self.glyph_raster_config(&frame, scale_factor);
        let strip_origin_y =
            frame.render_plan.viewport.y + (frame.render_plan.cursor.row as f32 * config.line_height).round() as usize;
        let max_strip_height = usize::from(frame.pane_surface.height).saturating_sub(strip_origin_y);
        if max_strip_height == 0 {
            return Ok(false);
        }
        let strip_height = (config.line_height.ceil() as usize).max(1).min(max_strip_height);
        let strip_plan = RenderPlan {
            backend: frame.render_plan.backend,
            pane_rect: RenderRect::new(0, 0, usize::from(frame.pane_surface.width.max(1)), strip_height),
            viewport: RenderRect::new(
                frame.render_plan.viewport.x,
                0,
                frame.render_plan.viewport.width,
                strip_height,
            ),
            damage: DamageSet {
                dirty_rows: vec![0],
                full_frame: true,
            },
            scrollback_rows: 0,
            cursor: Cursor {
                row: 0,
                col: frame.render_plan.cursor.col,
            },
            alternate_screen: false,
            selection: None,
            chrome: Vec::new(),
            active: true,
            border: PaneBorderStyle::default(),
            corner_radius: 0,
            rows: vec![RenderRow {
                row: 0,
                wrapped: row.wrapped,
                glyphs: row.glyphs,
            }],
        };
        let strip = rasterize_software_frame(
            &strip_plan,
            &config,
            &self.software_palette(),
            self.cursor_visible,
        )?;
        let Some(cached) = self.cached_software_frame.as_mut() else {
            return Ok(false);
        };
        blit_software_frame(
            cached,
            &strip,
            usize::from(frame.pane_surface.x),
            usize::from(frame.pane_surface.y).saturating_add(strip_origin_y),
        );
        let Some(renderer) = self.renderer.as_mut() else {
            return Ok(false);
        };
        renderer.render_software_frame(cached)?;
        Ok(true)
    }

    fn submit_editor_buffer(&mut self) -> Result<bool, Box<dyn Error>> {
        let Some(pane_id) = self.app.active_pane_id() else {
            return Ok(false);
        };
        let state = self.editor_state_mut(pane_id);
        if state.text.is_empty() {
            self.app.write_input(b"\r")?;
            return Ok(true);
        }
        let command = state.text.clone();
        state.push_history(&command);
        state.clear();
        let mut bytes = command.into_bytes();
        bytes.push(b'\r');
        self.app.write_input(&bytes)?;
        Ok(true)
    }

    fn insert_editor_text(&mut self, text: &str) -> bool {
        let Some(pane_id) = self.app.active_pane_id() else {
            return false;
        };
        self.editor_state_mut(pane_id).insert_text(text);
        true
    }

    fn handle_editor_key(
        &mut self,
        event: &winit::event::KeyEvent,
    ) -> Result<bool, Box<dyn Error>> {
        if event.state != ElementState::Pressed {
            return Ok(false);
        }
        if self.active_input_mode() != InputMode::Editor {
            return Ok(false);
        }

        let Some(pane_id) = self.app.active_pane_id() else {
            return Ok(false);
        };
        let control = self.modifiers.control_key();
        let alt = self.modifiers.alt_key();
        let latency_kind = match event.logical_key.as_ref() {
            Key::Named(NamedKey::Backspace) | Key::Named(NamedKey::Delete) => "delete",
            Key::Named(NamedKey::Space) => "insert",
            Key::Character(_) => "insert",
            _ => "move",
        };
        match event.logical_key.as_ref() {
            Key::Named(NamedKey::Enter) => return self.submit_editor_buffer(),
            Key::Named(NamedKey::ArrowLeft) => {
                self.editor_state_mut(pane_id).move_left();
            }
            Key::Named(NamedKey::ArrowRight) => {
                self.editor_state_mut(pane_id).move_right();
            }
            Key::Named(NamedKey::ArrowUp) => {
                self.editor_state_mut(pane_id).history_prev();
            }
            Key::Named(NamedKey::ArrowDown) => {
                self.editor_state_mut(pane_id).history_next();
            }
            Key::Named(NamedKey::Home) => {
                self.editor_state_mut(pane_id).move_home();
            }
            Key::Named(NamedKey::End) => {
                self.editor_state_mut(pane_id).move_end();
            }
            Key::Named(NamedKey::Backspace) => {
                self.editor_state_mut(pane_id).delete_backward();
            }
            Key::Named(NamedKey::Delete) => {
                self.editor_state_mut(pane_id).delete_forward();
            }
            Key::Named(NamedKey::Space) => {
                self.editor_state_mut(pane_id).insert_text(" ");
            }
            Key::Character(text) if control => match text.to_ascii_lowercase().as_str() {
                "a" => self.editor_state_mut(pane_id).move_home(),
                "e" => self.editor_state_mut(pane_id).move_end(),
                "u" => self.editor_state_mut(pane_id).delete_to_start(),
                "k" => self.editor_state_mut(pane_id).delete_to_end(),
                "w" => self.editor_state_mut(pane_id).delete_previous_word(),
                "c" => self.editor_state_mut(pane_id).clear(),
                "l" => {
                    self.app.write_input(&[0x0c])?;
                    self.editor_state_mut(pane_id).anchor = None;
                }
                _ => return Ok(false),
            },
            Key::Character(text) if alt => match text.to_ascii_lowercase().as_str() {
                "b" => self.editor_state_mut(pane_id).move_word_left(),
                "f" => self.editor_state_mut(pane_id).move_word_right(),
                _ => return Ok(false),
            },
            Key::Character(text)
                if !control
                    && !alt
                    && !text.is_empty() =>
            {
                self.editor_state_mut(pane_id).insert_text(text);
            }
            _ => return Ok(false),
        }

        self.refresh_editor_feedback(latency_kind)?;
        Ok(true)
    }

    fn start_transition(&mut self, kind: Option<TransitionKind>, before: TransitionSnapshot) {
        let Some(kind) = kind else {
            return;
        };
        let Some(duration) = self.animation_duration() else {
            self.transition = None;
            return;
        };

        let after = self.transition_snapshot();
        let mut pane_ids = before
            .panes
            .iter()
            .map(|(pane_id, _)| *pane_id)
            .collect::<Vec<_>>();
        for (pane_id, _) in &after.panes {
            if !pane_ids.contains(pane_id) {
                pane_ids.push(*pane_id);
            }
        }
        pane_ids.sort_by_key(|pane_id| pane_id.0);

        let panes = pane_ids
            .into_iter()
            .filter_map(|pane_id| {
                let from = before
                    .panes
                    .iter()
                    .find_map(|(current, rect)| (*current == pane_id).then_some(*rect));
                let to = after
                    .panes
                    .iter()
                    .find_map(|(current, rect)| (*current == pane_id).then_some(*rect));
                (from != to).then_some(TransitionRect { pane_id, from, to })
            })
            .collect::<Vec<_>>();

        if panes.is_empty()
            && before.workspace_id == after.workspace_id
            && before.scratch_visible == after.scratch_visible
        {
            self.transition = None;
            return;
        }

        let now = Instant::now();
        self.transition = Some(ActiveTransition {
            kind,
            started_at: now,
            next_frame_at: now,
            duration,
            panes,
        });
    }

    fn visual_effects_policy(&self) -> VisualEffectsPolicy {
        let requested_opacity = self.theme.opacity;
        let low_power_enabled = self.low_power_enabled();

        if self.launch_options.safe_mode {
            return VisualEffectsPolicy {
                low_power_enabled,
                requested_opacity,
                effective_opacity: 1.0,
                window_transparent: false,
                transparency_fallback_reason: Some("safe-mode"),
                blur_mode: if low_power_enabled {
                    BlurMode::Disabled
                } else if self.theme.blur.enabled {
                    BlurMode::TintedSolid
                } else {
                    BlurMode::Disabled
                },
                blur_fallback_reason: if low_power_enabled && self.theme.blur.enabled {
                    Some("low-power")
                } else if self.theme.blur.enabled {
                    Some("safe-mode")
                } else {
                    None
                },
            };
        }

        if self.app.backend() != RenderBackend::Gpu {
            return VisualEffectsPolicy {
                low_power_enabled,
                requested_opacity,
                effective_opacity: 1.0,
                window_transparent: false,
                transparency_fallback_reason: Some("software-backend"),
                blur_mode: if low_power_enabled {
                    BlurMode::Disabled
                } else if self.theme.blur.enabled {
                    BlurMode::TintedSolid
                } else {
                    BlurMode::Disabled
                },
                blur_fallback_reason: if low_power_enabled && self.theme.blur.enabled {
                    Some("low-power")
                } else if self.theme.blur.enabled {
                    Some("software-backend")
                } else {
                    None
                },
            };
        }

        if low_power_enabled {
            return VisualEffectsPolicy {
                low_power_enabled,
                requested_opacity,
                effective_opacity: requested_opacity,
                window_transparent: requested_opacity < 1.0,
                transparency_fallback_reason: None,
                blur_mode: BlurMode::Disabled,
                blur_fallback_reason: self.theme.blur.enabled.then_some("low-power"),
            };
        }

        if requested_opacity >= 1.0 {
            return VisualEffectsPolicy {
                low_power_enabled,
                requested_opacity,
                effective_opacity: 1.0,
                window_transparent: false,
                transparency_fallback_reason: None,
                blur_mode: BlurMode::Disabled,
                blur_fallback_reason: None,
            };
        }

        if self.theme.blur.enabled {
            return VisualEffectsPolicy {
                low_power_enabled,
                requested_opacity,
                effective_opacity: self.theme.blur.fallback_tint_opacity.max(requested_opacity),
                window_transparent: false,
                transparency_fallback_reason: None,
                blur_mode: BlurMode::TintedSolid,
                blur_fallback_reason: Some("unsupported-platform"),
            };
        }

        VisualEffectsPolicy {
            low_power_enabled,
            requested_opacity,
            effective_opacity: requested_opacity,
            window_transparent: true,
            transparency_fallback_reason: None,
            blur_mode: BlurMode::Disabled,
            blur_fallback_reason: None,
        }
    }

    fn create_window(&mut self, event_loop: &ActiveEventLoop) -> Result<(), Box<dyn Error>> {
        let requested_transparency = self.theme.opacity < 1.0 && !self.launch_options.safe_mode;
        #[allow(unused_mut)]
        let mut attributes = Window::default_attributes()
            .with_title("Noctrail")
            .with_inner_size(LogicalSize::new(
                f64::from(DEFAULT_WINDOW_WIDTH),
                f64::from(DEFAULT_WINDOW_HEIGHT),
            ))
            .with_active(true)
            .with_resizable(true)
            .with_decorations(false)
            .with_transparent(requested_transparency);
        #[cfg(target_os = "macos")]
        {
            attributes = attributes
                .with_titlebar_hidden(true)
                .with_fullsize_content_view(true)
                .with_movable_by_window_background(true);
        }
        info!(
            safe_mode = self.launch_options.safe_mode,
            backend = ?self.launch_options.renderer_backend,
            gpu_raster = self.should_attempt_gpu_renderer(),
            transparency = requested_transparency,
            "creating noctrail window"
        );
        let window = Arc::new(event_loop.create_window(attributes)?);
        self.window = Some(window.clone());
        window.focus_window();
        let size = window.inner_size();
        self.sync_surface(size, window.scale_factor())?;
        if self.launch_options.safe_mode {
            self.renderer = None;
            self.gpu_fallback_error = Some("safe-mode".to_string());
            self.app.set_backend(RenderBackend::Software);
            info!("safe mode enabled; GPU presenter disabled");
        } else {
            match GpuRenderer::new(window.clone(), size) {
                Ok(renderer) => {
                    self.renderer = Some(renderer);
                    self.gpu_fallback_error = None;
                    self.app.set_backend(self.launch_options.renderer_backend);
                    info!(
                        backend = ?self.launch_options.renderer_backend,
                        width = size.width,
                        height = size.height,
                        "render presenter initialized"
                    );
                    self.apply_theme_visuals();
                }
                Err(error) => {
                    self.record_gpu_fallback(error.to_string());
                }
            }
        }
        self.apply_theme_visuals();
        self.update_title();
        self.request_redraw();
        Ok(())
    }

    fn sync_surface(
        &mut self,
        size: PhysicalSize<u32>,
        scale_factor: f64,
    ) -> Result<(), Box<dyn Error>> {
        let surface = layout_rect_from_surface(size);
        let terminal_size = terminal_size_from_surface(size, &self.font, scale_factor);
        self.app.resize(surface, terminal_size)?;
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.resize(size);
        }
        debug!(
            width = size.width,
            height = size.height,
            cols = terminal_size.cols,
            rows = terminal_size.rows,
            "synced surface and terminal size"
        );
        Ok(())
    }

    fn update_title(&self) {}

    fn concise_title_text(&self, frame: &DesktopFrame) -> String {
        let shell = frame.status_line.shell.as_deref().unwrap_or("shell");
        let cwd = frame
            .status_line
            .cwd
            .as_deref()
            .map(display_status_path)
            .unwrap_or_else(|| "workspace".to_string());
        format!("Noctrail - {} - {cwd}", shell.to_ascii_uppercase())
    }

    fn glyph_raster_config(&self, frame: &DesktopFrame, scale_factor: f64) -> GlyphRasterConfig {
        let cols = f32::from(frame.terminal_size.cols.max(1));
        let rows = f32::from(frame.terminal_size.rows.max(1));
        let nominal = cell_dimensions(&self.font);
        let measured_cell_width = f32::from(frame.surface.width.max(1)) / cols;
        let measured_line_height = f32::from(frame.surface.height.max(1)) / rows;
        GlyphRasterConfig {
            font: self.font_preferences.clone(),
            scale: scale_factor as f32,
            cell_width: measured_cell_width
                .clamp(nominal.width * 0.94, nominal.width * 1.08)
                .max(1.0),
            line_height: measured_line_height
                .clamp(nominal.height * 0.96, nominal.height * 1.1)
                .max(1.0),
            weight: self.font.weight,
            bold_weight: self.font.bold_weight,
        }
    }

    fn software_palette(&self) -> SoftwareRenderPalette {
        SoftwareRenderPalette {
            background: rgba_from_config(self.theme.color.background),
            foreground: rgba_from_config(self.theme.color.foreground),
            selection_background: rgba_from_config(self.theme.selection.background),
            selection_foreground: rgba_from_config(self.theme.selection.foreground),
            cursor: rgba_from_config(self.theme.cursor.color),
        }
    }

    fn status_bar_palette(&self, active: bool) -> StatusBarPalette {
        let colors = &self.theme.color;
        let chrome_background = rgba_from_config(colors.chrome_background);
        let background = if active {
            mix_rgba(
                rgba_from_config(colors.chrome_foreground),
                chrome_background,
                0.88,
            )
        } else {
            mix_rgba(
                rgba_from_config(colors.chrome_foreground),
                chrome_background,
                0.92,
            )
        };
        let foreground = rgba_from_config(colors.chrome_foreground);
        let muted = if active {
            rgba_from_config(colors.chrome_muted)
        } else {
            mix_rgba(rgba_from_config(colors.chrome_muted), background, 0.25)
        };
        let accent = if active {
            rgba_from_config(colors.chrome_accent)
        } else {
            mix_rgba(rgba_from_config(colors.chrome_accent), background, 0.35)
        };
        let danger = if active {
            rgba_from_config(colors.chrome_danger)
        } else {
            mix_rgba(rgba_from_config(colors.chrome_danger), background, 0.35)
        };
        let chip_background = if active {
            mix_rgba(foreground, background, 0.84)
        } else {
            mix_rgba(foreground, background, 0.93)
        };
        let chip_accent_background = if active {
            mix_rgba(accent, background, 0.52)
        } else {
            mix_rgba(accent, background, 0.82)
        };
        let chip_danger_background = if active {
            mix_rgba(danger, background, 0.54)
        } else {
            mix_rgba(danger, background, 0.8)
        };

        StatusBarPalette {
            background,
            foreground,
            muted,
            accent,
            danger,
            chip_background,
            chip_accent_background,
            chip_danger_background,
        }
    }

    fn render_current_frame(&mut self) -> Result<(), Box<dyn Error>> {
        let Some(window) = self.window.as_ref() else {
            return Ok(());
        };
        let scale_factor = window.scale_factor();
        let size = window.inner_size();
        let palette = self.software_palette();
        let frames = self.visible_frames()?;
        let mut raster =
            solid_software_frame(size.width.max(1), size.height.max(1), palette.background);

        for mut frame in frames {
            self.apply_editor_overlay(&mut frame);
            let mut pane_raster = rasterize_software_frame(
                &frame.render_plan,
                &self.glyph_raster_config(&frame, scale_factor),
                &palette,
                self.cursor_visible && frame.render_plan.active,
            )?;
            self.render_status_bar(&frame, scale_factor, &mut pane_raster)?;
            if frame.render_plan.active {
                self.maybe_log_render_frame(&frame, &pane_raster);
            }
            blit_software_frame(
                &mut raster,
                &pane_raster,
                usize::from(frame.pane_surface.x),
                usize::from(frame.pane_surface.y),
            );
        }

        if let Some(renderer) = self.renderer.as_mut() {
            renderer.render_software_frame(&raster)?;
        }
        self.cached_software_frame = Some(raster);

        Ok(())
    }

    fn visible_frames(&self) -> Result<Vec<DesktopFrame>, Box<dyn Error>> {
        let mut pane_ids = self
            .app
            .pane_layouts()
            .into_iter()
            .map(|layout| layout.pane_id)
            .collect::<Vec<_>>();
        if self.app.scratch_visible()
            && let Some(scratch_id) = self.app.scratch_pane_id()
        {
            pane_ids.push(scratch_id);
        }

        pane_ids
            .into_iter()
            .map(|pane_id| self.app.frame_for_pane(pane_id).map_err(Into::into))
            .collect()
    }

    fn apply_editor_overlay(&mut self, frame: &mut DesktopFrame) {
        if !frame.render_plan.active || self.effective_input_mode(frame) != InputMode::Editor {
            return;
        }

        let anchor = {
            let state = self.editor_state_mut(frame.pane_id);
            state.sync_anchor(frame.render_plan.cursor);
            state.anchor.unwrap_or(frame.render_plan.cursor)
        };

        let state = self.editor_state_mut(frame.pane_id).clone();
        frame.render_plan.cursor = Cursor {
            row: anchor.row.min(frame.render_plan.rows.len().saturating_sub(1)),
            col: (anchor.col + state.cursor_chars)
                .min(usize::from(frame.terminal_size.cols).saturating_sub(1)),
        };

        if state.text.is_empty() {
            return;
        }

        let Some(row) = frame.render_plan.rows.get_mut(anchor.row) else {
            return;
        };
        let row_capacity = usize::from(frame.terminal_size.cols);
        let mut glyphs = row
            .glyphs
            .iter()
            .filter(|glyph| glyph.col < anchor.col)
            .cloned()
            .collect::<Vec<_>>();
        let overlay_style = row
            .glyphs
            .iter()
            .rev()
            .find(|glyph| glyph.col < anchor.col)
            .map(|glyph| glyph.style)
            .unwrap_or_default();

        for (index, ch) in state.text.chars().enumerate() {
            let col = anchor.col + index;
            if col >= row_capacity {
                break;
            }
            glyphs.push(RenderGlyph {
                col,
                text: ch.to_string(),
                style: overlay_style,
                span: 1,
                wide_continuation: false,
            });
        }
        row.glyphs = glyphs;
    }

    fn render_status_bar(
        &self,
        frame: &DesktopFrame,
        scale_factor: f64,
        target: &mut noctrail_render::SoftwareRenderFrame,
    ) -> Result<(), noctrail_render::GlyphPrepareError> {
        if frame.status_surface.height == 0 || frame.status_surface.width == 0 {
            return Ok(());
        }
        let palette = self.status_bar_palette(frame.render_plan.active);
        let status_width = usize::from(frame.status_surface.width.max(1));
        let status_height = usize::from(frame.status_surface.height.max(1));
        let origin_x = usize::from(frame.status_surface.x.saturating_sub(frame.pane_surface.x));
        let origin_y = usize::from(frame.status_surface.y.saturating_sub(frame.pane_surface.y));
        let mut config = self.glyph_raster_config(frame, scale_factor);
        config.font.size = (config.font.size * 1.12).max(self.font.size);
        config.line_height = (self.font.size * self.font.line_height)
            .min((status_height as f32 - 8.0).max(1.0))
            .max(1.0);

        let horizontal_padding = ((self.font.size * 1.45).round() as usize).max(28);
        let usable_width = status_width.saturating_sub(horizontal_padding.saturating_mul(2));
        let cols = ((usable_width as f32 / config.cell_width).floor() as usize).max(1);
        let runs = compose_status_runs(frame, cols, palette, self.effective_input_mode(frame));
        let row = compose_status_row(&runs.left, &runs.right, cols);
        let top_padding =
            (((status_height as f32) - config.line_height).max(0.0) / 2.0).floor() as usize;
        let viewport = RenderRect::new(
            horizontal_padding.min(status_width / 2),
            top_padding,
            usable_width.max(1),
            status_height,
        );
        let overlay = rasterize_software_frame(
            &RenderPlan {
                backend: frame.render_plan.backend,
                pane_rect: RenderRect::new(0, 0, status_width, status_height),
                viewport,
                damage: DamageSet {
                    dirty_rows: vec![0],
                    full_frame: true,
                },
                scrollback_rows: 0,
                cursor: Cursor::default(),
                alternate_screen: false,
                selection: None,
                chrome: Vec::new(),
                active: frame.render_plan.active,
                border: PaneBorderStyle::default(),
                corner_radius: 0,
                rows: vec![RenderRow {
                    row: 0,
                    wrapped: false,
                    glyphs: row,
                }],
            },
            &config,
            &SoftwareRenderPalette {
                background: palette.background,
                foreground: palette.foreground,
                selection_background: palette.background,
                selection_foreground: palette.foreground,
                cursor: palette.accent,
            },
            false,
        )?;

        blit_software_frame(target, &overlay, origin_x, origin_y);
        Ok(())
    }

    fn maybe_log_render_frame(
        &mut self,
        frame: &DesktopFrame,
        raster: &noctrail_render::SoftwareRenderFrame,
    ) {
        let signature = FrameLogSignature {
            full_frame: raster.stats.full_frame,
            dirty_rows: raster.stats.dirty_rows,
            glyphs: raster.stats.glyphs_prepared,
            paint_rects: raster.stats.paint_rects,
            border_segments: raster.stats.border_segments,
        };
        let now = Instant::now();
        let in_startup_window = now.duration_since(self.started_at) <= STARTUP_DEBUG_WINDOW;
        let signature_changed = self.last_frame_log_signature != Some(signature);
        let interval_elapsed = self
            .last_frame_log_at
            .is_none_or(|previous| now.duration_since(previous) >= STABLE_DEBUG_SAMPLE_INTERVAL);

        if in_startup_window || signature_changed || interval_elapsed {
            debug!(
                backend = ?frame.render_plan.backend,
                rows = frame.render_plan.rows.len(),
                dirty_rows = raster.stats.dirty_rows,
                glyphs = raster.stats.glyphs_prepared,
                paint_rects = raster.stats.paint_rects,
                border_segments = raster.stats.border_segments,
                full_frame = raster.stats.full_frame,
                startup_window = in_startup_window,
                "prepared terminal frame"
            );
            self.last_frame_log_at = Some(now);
            self.last_frame_log_signature = Some(signature);
        }
    }

    fn title_text(&self) -> String {
        let frame = self.app.frame();
        if !self.launch_options.debug_logging
            && self.gpu_fallback_error.is_none()
            && self.theme_reload_error.is_none()
            && self.app.agent_command_proposals().is_empty()
            && self.block_browser.is_none()
            && self.agent_context_browser.is_none()
            && self.agent_audit_browser.is_none()
            && self.patch_preview_browser.is_none()
            && self.review_panel.is_none()
            && self.command_palette.is_none()
        {
            return self.concise_title_text(&frame);
        }

        let mut title = frame_title(&frame, self.cursor_visible);
        let effects = self.visual_effects_policy();
        title.push_str(" | font ");
        title.push_str(&self.font.family);
        title.push(' ');
        title.push_str(&format!("{:.1}", self.font.size));
        title.push_str(" | power ");
        title.push_str(if effects.low_power_enabled {
            "low"
        } else {
            "normal"
        });
        title.push_str(" | opacity ");
        title.push_str(&format!("{:.2}", effects.effective_opacity));
        match effects.blur_mode {
            BlurMode::Disabled => title.push_str(" | blur off"),
            BlurMode::TintedSolid => title.push_str(" | blur tinted-solid"),
        }
        if let Some(reason) = effects.transparency_fallback_reason {
            title.push_str(" | transparency-fallback ");
            title.push_str(reason);
        }
        if let Some(reason) = effects.blur_fallback_reason {
            title.push_str(" | blur-fallback ");
            title.push_str(reason);
        }
        if let Some(transition) = self.transition.as_ref() {
            title.push_str(" | anim ");
            title.push_str(transition.kind.label());
            title.push(' ');
            title.push_str(&format!("{:.2}", transition.progress(Instant::now())));
        }
        if let Some(error) = self.gpu_fallback_error.as_deref() {
            title.push_str(" | gpu-fallback ");
            title.push_str(error);
        }
        if let Some(error) = self.theme_reload_error.as_deref() {
            title.push_str(" | theme-reload ");
            title.push_str(error);
        }
        if let Some(preedit) = self.ime_preedit.as_deref()
            && !preedit.is_empty()
        {
            title.push_str(" | ime ");
            title.push_str(preedit);
        }
        if self.block_browser.is_some() {
            title.push_str(" | blocks ");
            title.push_str(if self.app.block_observer_enabled() {
                "on"
            } else {
                "off"
            });
            title.push(' ');
            title.push_str(&self.app.command_blocks().len().to_string());
            title.push('/');
            title.push_str("100");
            let failed = self.app.failed_command_blocks_count();
            if failed > 0 {
                title.push_str(" | failures ");
                title.push_str(&failed.to_string());
            }
            if let Some(index) = self.app.selected_command_block_index() {
                title.push_str(" sel ");
                title.push_str(&(index + 1).to_string());
            }
            if let Some(block) = self.app.selected_command_block() {
                if block.failed() {
                    title.push_str(" | FAIL");
                }
                if block.folded {
                    title.push_str(" | folded");
                }
                if let Some(lens) = block.structured_output.as_ref() {
                    title.push_str(" | lens ");
                    title.push_str(&lens.summary);
                }
                if let Some(command) = block.command.as_deref() {
                    title.push_str(" | cmd ");
                    title.push_str(&preview_text(command, 32));
                }
                if let Some(exit_code) = block.exit_code {
                    title.push_str(" | code ");
                    title.push_str(&exit_code.to_string());
                }
                if let Some(duration_ms) = block.duration_ms {
                    title.push_str(" | dur ");
                    title.push_str(&duration_ms.to_string());
                    title.push_str("ms");
                }
                if !block.folded && !block.output.is_empty() {
                    title.push_str(" | out ");
                    title.push_str(&preview_text(&block.output, 32));
                }
            }
        }
        if self.agent_context_browser.is_some() {
            let preview =
                crate::redaction::redact_agent_context_preview(&self.app.agent_context_preview());
            title.push_str(" | agent-context");
            if let Some(block) = preview.current_block.as_ref() {
                if let Some(command) = block.command.as_deref() {
                    title.push_str(" | block ");
                    title.push_str(&preview_text(command, 32));
                }
                if !block.output.is_empty() {
                    title.push_str(" | output ");
                    title.push_str(&preview_text(&block.output, 32));
                }
            }
            if let Some(selection) = preview.selection.as_deref() {
                title.push_str(" | selection ");
                title.push_str(&preview_text(selection, 32));
            }
            if let Some(cwd) = preview.cwd.as_deref() {
                title.push_str(" | cwd ");
                title.push_str(&display_status_path(cwd));
            }
            if !preview.explicit_files.is_empty() {
                title.push_str(" | files ");
                title.push_str(&preview.explicit_files.len().to_string());
                title.push(' ');
                title.push_str(&preview_paths(&preview.explicit_files, 48));
            }
        }
        if self.agent_audit_browser.is_some() {
            title.push_str(" | agent-audit");
            title.push_str(" | entries ");
            title.push_str(&self.app.agent_audit_entries().len().to_string());
            if let Some(index) = self.app.selected_agent_audit_entry_index() {
                title.push_str(" sel ");
                title.push_str(&(index + 1).to_string());
            }
            if let Some(entry) = self.app.selected_agent_audit_entry() {
                title.push_str(" | ");
                title.push_str(entry.kind.label());
                title.push(' ');
                title.push_str(&preview_text(&entry.summary, 48));
            }
        }
        if let Some(proposal) = self.app.agent_command_proposals().first() {
            title.push_str(" | agent-proposal");
            title.push_str(" | risk ");
            title.push_str(proposal.risk.label());
            title.push_str(" | permission ");
            title.push_str(proposal.permission.label());
            title.push_str(" | cmd ");
            title.push_str(&preview_text(&proposal.command, 32));
            title.push_str(" | reason ");
            title.push_str(&preview_text(&proposal.reason, 32));
        }
        if let Some(review_panel) = self.review_panel.as_ref() {
            title.push_str(" | review");
            if let Some(index) = self.app.selected_agent_command_proposal_index() {
                title.push_str(" sel ");
                title.push_str(&(index + 1).to_string());
            }
            if let Some(proposal) = self.app.selected_agent_command_proposal() {
                title.push_str(" | risk ");
                title.push_str(proposal.risk.label());
                title.push_str(" | permission ");
                title.push_str(proposal.permission.label());
                title.push_str(" | cmd ");
                title.push_str(&preview_text(&proposal.command, 32));
                title.push_str(" | reason ");
                title.push_str(&preview_text(&proposal.reason, 32));
                match proposal.risk {
                    CommandRisk::High | CommandRisk::Critical => {
                        if review_panel.strong_confirm_index
                            == self.app.selected_agent_command_proposal_index()
                        {
                            title.push_str(" | confirm y");
                        } else {
                            title.push_str(" | press enter to arm");
                        }
                    }
                    CommandRisk::Low | CommandRisk::Medium => {
                        title.push_str(" | press enter to execute");
                    }
                }
            }
        }
        if self.patch_preview_browser.is_some() {
            title.push_str(" | patch-preview");
            if let Some(index) = self.app.selected_agent_patch_preview_index() {
                title.push_str(" sel ");
                title.push_str(&(index + 1).to_string());
            }
            if let Some(preview) = self.app.selected_agent_patch_preview() {
                title.push_str(" | file ");
                title.push_str(&display_status_path(&preview.path));
                title.push_str(" | reason ");
                title.push_str(&preview_text(&preview.reason, 32));
                title.push_str(" | diff ");
                title.push_str(&preview_diff(&preview.diff, 48));
            }
        }
        if let Some(palette) = self.command_palette.as_ref() {
            title.push_str(" | palette ");
            if palette.query.is_empty() {
                title.push_str("(all)");
            } else {
                title.push_str(&palette.query);
            }
            if let Some(command) = palette.selected_command() {
                title.push_str(" -> ");
                title.push_str(&command.label());
            }
        }
        title
    }

    fn apply_theme_visuals(&mut self) {
        self.frame_interval = Duration::from_millis(self.theme.cursor.blink_interval_ms);
        self.app.invalidate_visuals();
        if self.animation_duration().is_none() {
            self.transition = None;
        }
        let effects = self.visual_effects_policy();
        if let Some(window) = self.window.as_ref() {
            window.set_transparent(effects.window_transparent);
        }
        if let Some(renderer) = self.renderer.as_mut() {
            let background = self.theme.color.background;
            renderer.set_clear_color(
                srgb_component(background.red),
                srgb_component(background.green),
                srgb_component(background.blue),
                f64::from(effects.effective_opacity) * background.alpha_factor(),
            );
        }
    }

    fn request_redraw(&self) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    fn touch_cursor_blink(&mut self) {
        self.cursor_visible = true;
        self.next_cursor_blink_at = Instant::now() + self.frame_interval;
    }

    fn advance_cursor_blink(&mut self, now: Instant) -> bool {
        if !self.window_focused || now < self.next_cursor_blink_at {
            return false;
        }

        self.cursor_visible = !self.cursor_visible;
        self.next_cursor_blink_at = now + self.frame_interval;
        true
    }

    fn advance_transition(&mut self, now: Instant) -> bool {
        let Some(transition) = self.transition.as_mut() else {
            return false;
        };
        if now >= transition.deadline() {
            self.transition = None;
            return true;
        }
        if now < transition.next_frame_at {
            return false;
        }

        transition.next_frame_at = now + ANIMATION_FRAME_INTERVAL;
        true
    }

    fn reschedule(&mut self, event_loop: &ActiveEventLoop) {
        let transition_deadline = self
            .transition
            .as_ref()
            .map(|transition| transition.next_frame_at.min(transition.deadline()));
        match (self.window_focused, transition_deadline) {
            (true, Some(deadline)) => {
                event_loop.set_control_flow(ControlFlow::WaitUntil(
                    self.next_cursor_blink_at.min(deadline),
                ));
            }
            (true, None) => {
                event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_cursor_blink_at));
            }
            (false, Some(deadline)) => {
                event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
            }
            (false, None) => {
                event_loop.set_control_flow(ControlFlow::Wait);
            }
        }
    }

    fn record_gpu_fallback(&mut self, error: String) {
        warn!(reason = %error, "render presenter unavailable; falling back to degraded mode");
        self.renderer = None;
        self.gpu_fallback_error = Some(error);
        self.app
            .set_backend(noctrail_render::RenderBackend::Software);
    }

    fn drain_output_events(&mut self) -> bool {
        if let Some(error) = self.app.take_runtime_output_error() {
            error!(reason = %error, "PTY output pump failed");
        }
        if self.app.buffered_runtime_output_bytes() == 0 {
            return false;
        }

        let mut drained_any = false;
        let mut total_drained_bytes = 0;
        let mut remaining_bytes = 0;
        let mut drain_count = 0;

        while drain_count < MAX_OUTPUT_DRAINS_PER_EVENT
            && self.app.buffered_runtime_output_bytes() > 0
        {
            let Ok(drain) = self.app.drain_runtime_output_budget() else {
                break;
            };
            if drain.drained_bytes == 0 {
                remaining_bytes = drain.remaining_bytes;
                break;
            }

            for chunk in drain.chunks {
                self.app.advance_output(&chunk);
            }

            drained_any = true;
            total_drained_bytes += drain.drained_bytes;
            remaining_bytes = drain.remaining_bytes;
            drain_count += 1;
        }

        if !drained_any {
            return false;
        }

        debug!(
            drained_bytes = total_drained_bytes,
            remaining_bytes, drain_count, "drained PTY output into terminal state"
        );
        self.touch_cursor_blink();
        self.request_redraw();
        if remaining_bytes > 0 {
            self.requeue_runtime_output_event();
        }

        true
    }

    fn requeue_runtime_output_event(&self) {
        if let Some(proxy) = self.event_proxy.as_ref() {
            let _ = proxy.send_event(GuiEvent::RuntimeOutput);
        }
    }

    fn handle_ime_event(&mut self, ime: Ime) -> Result<(), Box<dyn Error>> {
        match ime {
            Ime::Enabled | Ime::Disabled => Ok(()),
            Ime::Preedit(text, _cursor) => {
                self.ime_preedit = if text.is_empty() { None } else { Some(text) };
                self.touch_cursor_blink();
                self.request_redraw();
                Ok(())
            }
            Ime::Commit(text) => {
                self.ime_preedit = None;
                if !text.is_empty() {
                    if self.active_input_mode() == InputMode::Editor {
                        self.insert_editor_text(&text);
                        self.refresh_editor_feedback("ime-commit")?;
                    } else {
                        self.app.write_input(text.as_bytes())?;
                        self.touch_cursor_blink();
                        self.request_redraw();
                    }
                }
                Ok(())
            }
        }
    }

    fn handle_window_resize_interaction(&self) {
        if let (Some(window), Some(position)) = (self.window.as_ref(), self.mouse_position)
            && let Some(direction) = self.resize_direction_for_position(position)
        {
            let _ = window.drag_resize_window(direction);
        }
    }

    fn resize_direction_for_position(
        &self,
        position: PhysicalPosition<f64>,
    ) -> Option<ResizeDirection> {
        let window = self.window.as_ref()?;
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return None;
        }

        let x = position.x;
        let y = position.y;
        let width = f64::from(size.width);
        let height = f64::from(size.height);
        let left = x <= WINDOW_RESIZE_HANDLE_PX;
        let right = x >= width - WINDOW_RESIZE_HANDLE_PX;
        let top = y <= WINDOW_RESIZE_HANDLE_PX;
        let bottom = y >= height - WINDOW_RESIZE_HANDLE_PX;

        match (left, right, top, bottom) {
            (true, false, true, false) => Some(ResizeDirection::NorthWest),
            (false, true, true, false) => Some(ResizeDirection::NorthEast),
            (true, false, false, true) => Some(ResizeDirection::SouthWest),
            (false, true, false, true) => Some(ResizeDirection::SouthEast),
            (true, false, false, false) => Some(ResizeDirection::West),
            (false, true, false, false) => Some(ResizeDirection::East),
            (false, false, true, false) => Some(ResizeDirection::North),
            (false, false, false, true) => Some(ResizeDirection::South),
            _ => None,
        }
    }

    fn cell_position_at(&self, position: PhysicalPosition<f64>) -> Option<Position> {
        let logical = if let Some(window) = self.window.as_ref() {
            position.to_logical::<f64>(window.scale_factor())
        } else {
            LogicalPosition::new(position.x, position.y)
        };
        if logical.x.is_sign_negative() || logical.y.is_sign_negative() {
            return None;
        }

        let frame = self.app.frame();
        let terminal_size = frame.terminal_size;
        let cell = self.glyph_raster_config(&frame, 1.0);
        let relative_x = logical.x - f64::from(frame.surface.x);
        let relative_y = logical.y - f64::from(frame.surface.y);
        if relative_x.is_sign_negative() || relative_y.is_sign_negative() {
            return None;
        }

        let col = (relative_x / f64::from(cell.cell_width)).floor() as usize;
        let row = (relative_y / f64::from(cell.line_height)).floor() as usize;
        if row >= usize::from(terminal_size.rows) || col >= usize::from(terminal_size.cols) {
            return None;
        }

        Some(Position { row, col })
    }

    fn handle_cursor_moved(
        &mut self,
        position: PhysicalPosition<f64>,
    ) -> Result<(), Box<dyn Error>> {
        self.mouse_position = Some(position);
        if let Some(window) = self.window.as_ref() {
            let cursor = self
                .resize_direction_for_position(position)
                .map(CursorIcon::from)
                .unwrap_or(CursorIcon::Text);
            window.set_cursor(cursor);
        }

        if self.app.mouse_reporting_enabled() {
            if self.app.mouse_tracking_mode() == MouseTrackingMode::Motion
                && self.mouse_button.is_none()
                && let Some(cell) = self.cell_position_at(position)
            {
                self.write_mouse_report(input::MouseReportKind::Move, cell)?;
            } else if matches!(
                self.app.mouse_tracking_mode(),
                MouseTrackingMode::Drag | MouseTrackingMode::Motion
            ) && let (Some(button), Some(cell)) =
                (self.mouse_button, self.cell_position_at(position))
            {
                self.write_mouse_report(input::MouseReportKind::Drag(button), cell)?;
            }
            return Ok(());
        }

        let cell = self.cell_position_at(position);
        if let (Some(selection), Some(cell)) = (self.mouse_selection.as_mut(), cell) {
            selection.cursor = cell;
            self.app.select_viewport_range(
                selection.anchor,
                selection.cursor,
                SelectionMode::Normal,
            );
            self.touch_cursor_blink();
            self.request_redraw();
        }

        Ok(())
    }

    fn handle_mouse_input(
        &mut self,
        state: ElementState,
        button: WinitMouseButton,
    ) -> Result<(), Box<dyn Error>> {
        let button = mouse_button(button);
        let Some(button) = button else {
            return Ok(());
        };

        let cell = self
            .mouse_position
            .and_then(|position| self.cell_position_at(position));

        if self.app.mouse_reporting_enabled() {
            self.app.clear_selection();
            self.mouse_selection = None;
            match state {
                ElementState::Pressed => {
                    self.mouse_button = Some(button);
                    if let Some(cell) = cell {
                        self.write_mouse_report(input::MouseReportKind::Press(button), cell)?;
                    }
                }
                ElementState::Released => {
                    self.mouse_button = None;
                    if let Some(cell) = cell {
                        self.write_mouse_report(input::MouseReportKind::Release(button), cell)?;
                    }
                }
            }
            self.touch_cursor_blink();
            self.request_redraw();
            self.update_title();
            return Ok(());
        }

        if button != input::MouseButton::Left {
            return Ok(());
        }

        match state {
            ElementState::Pressed => {
                if self
                    .mouse_position
                    .and_then(|position| self.resize_direction_for_position(position))
                    .is_some()
                {
                    self.handle_window_resize_interaction();
                    return Ok(());
                }
                #[cfg(not(target_os = "macos"))]
                if self.modifiers.alt_key() {
                    if let Some(window) = self.window.as_ref() {
                        let _ = window.drag_window();
                    }
                    return Ok(());
                }
                if self.active_input_mode() == InputMode::Editor
                    && let Some(anchor) = self.editor_state_mut(self.app.active_pane_id().unwrap()).anchor
                    && let Some(cell) = cell
                    && cell.row == anchor.row
                {
                    let cursor = cell.col.saturating_sub(anchor.col);
                    let state = self.editor_state_mut(self.app.active_pane_id().unwrap());
                    state.cursor_chars = cursor.min(state.char_len());
                    self.touch_cursor_blink();
                    self.request_redraw();
                    return Ok(());
                }
                if let Some(cell) = cell {
                    self.mouse_selection = Some(MouseSelectionDrag {
                        anchor: cell,
                        cursor: cell,
                    });
                    self.app
                        .select_viewport_range(cell, cell, SelectionMode::Normal);
                    self.touch_cursor_blink();
                    self.request_redraw();
                }
            }
            ElementState::Released => {
                self.mouse_selection = None;
            }
        }

        Ok(())
    }

    fn handle_mouse_wheel(&mut self, delta: MouseScrollDelta) -> Result<(), Box<dyn Error>> {
        let lines = scroll_lines(delta);
        if lines == 0 {
            return Ok(());
        }

        if self.app.mouse_reporting_enabled() {
            if let Some(cell) = self
                .mouse_position
                .and_then(|position| self.cell_position_at(position))
            {
                let kind = if lines > 0 {
                    input::MouseReportKind::WheelUp
                } else {
                    input::MouseReportKind::WheelDown
                };
                for _ in 0..lines.unsigned_abs() {
                    self.write_mouse_report(kind, cell)?;
                }
            }
            self.touch_cursor_blink();
        } else {
            self.app.scroll_scrollback(lines);
            self.touch_cursor_blink();
            self.request_redraw();
            self.update_title();
        }

        Ok(())
    }

    fn write_mouse_report(
        &mut self,
        kind: input::MouseReportKind,
        cell: Position,
    ) -> Result<(), Box<dyn Error>> {
        let bytes = input::mouse_report_bytes(kind, cell.row, cell.col, self.app.sgr_mouse_mode());
        self.app.write_input(&bytes)?;
        Ok(())
    }

    fn poll_config_reload(&mut self) -> bool {
        let Some(reloader) = self.config_reloader.as_mut() else {
            return false;
        };

        match reloader.reload_if_changed() {
            Ok(Some(config)) => {
                self.theme = config.theme;
                self.font = config.font;
                self.font_preferences = font_preferences_from_config(&self.font);
                if let Err(error) = self
                    .app
                    .set_pane_chrome(pane_chrome_from_theme(&self.theme, &self.font))
                {
                    self.theme_reload_error = Some(error.to_string());
                    self.update_title();
                    return false;
                }
                self.theme_reload_error = None;
                self.apply_theme_visuals();
                self.touch_cursor_blink();
                self.update_title();
                self.request_redraw();
                true
            }
            Ok(None) => false,
            Err(error) => {
                self.theme_reload_error = Some(error.to_string());
                self.update_title();
                false
            }
        }
    }

    fn toggle_command_palette(&mut self) {
        if self.command_palette.is_some() {
            self.command_palette = None;
        } else {
            self.command_palette = Some(CommandPalette::new());
        }
        self.touch_cursor_blink();
        self.update_title();
        self.request_redraw();
    }

    fn toggle_review_panel(&mut self) {
        if self.review_panel.is_some() {
            self.review_panel = None;
        } else if !self.app.agent_command_proposals().is_empty() {
            if self.app.selected_agent_command_proposal_index().is_none() {
                let _ = self.app.select_oldest_agent_command_proposal();
            }
            if let Some(proposal) = self.app.selected_agent_command_proposal() {
                self.app
                    .record_agent_review(format!("open {}", preview_text(&proposal.command, 48)));
            }
            self.review_panel = Some(ReviewPanel::default());
        }
        self.touch_cursor_blink();
        self.update_title();
        self.request_redraw();
    }

    fn toggle_patch_preview_browser(&mut self) {
        if self.patch_preview_browser.is_some() {
            self.patch_preview_browser = None;
        } else if !self.app.agent_patch_previews().is_empty() {
            if self.app.selected_agent_patch_preview_index().is_none() {
                let _ = self.app.select_oldest_agent_patch_preview();
            }
            self.patch_preview_browser = Some(PatchPreviewBrowser);
        }
        self.touch_cursor_blink();
        self.update_title();
        self.request_redraw();
    }

    fn toggle_agent_context_preview(&mut self) {
        if self.agent_context_browser.is_some() {
            self.agent_context_browser = None;
        } else {
            self.agent_context_browser = Some(AgentContextBrowser);
        }
        self.touch_cursor_blink();
        self.update_title();
        self.request_redraw();
    }

    fn toggle_agent_audit_browser(&mut self) {
        if self.agent_audit_browser.is_some() {
            self.agent_audit_browser = None;
        } else if !self.app.agent_audit_entries().is_empty() {
            if self.app.selected_agent_audit_entry_index().is_none() {
                let _ = self.app.select_newest_agent_audit_entry();
            }
            self.agent_audit_browser = Some(AgentAuditBrowser);
        }
        self.touch_cursor_blink();
        self.update_title();
        self.request_redraw();
    }

    fn toggle_block_browser(&mut self) {
        if self.block_browser.is_some() {
            self.block_browser = None;
        } else {
            self.app.set_block_observer_enabled(true);
            let _ = self.app.select_newest_command_block();
            self.block_browser = Some(BlockBrowser);
        }
        self.touch_cursor_blink();
        self.update_title();
        self.request_redraw();
    }

    fn confirm_review_selection(&mut self) -> Result<bool, Box<dyn Error>> {
        let Some(review_panel) = self.review_panel.as_mut() else {
            return Ok(false);
        };
        let Some(index) = self.app.selected_agent_command_proposal_index() else {
            return Ok(true);
        };
        let Some(proposal) = self.app.selected_agent_command_proposal() else {
            return Ok(true);
        };

        match proposal.risk {
            CommandRisk::High | CommandRisk::Critical => {
                if review_panel.strong_confirm_index == Some(index) {
                    self.app.record_agent_review(format!(
                        "confirm {}",
                        preview_text(&proposal.command, 48)
                    ));
                    self.app.submit_selected_agent_command_proposal()?;
                    self.review_panel = None;
                } else {
                    self.app.record_agent_review(format!(
                        "arm {}",
                        preview_text(&proposal.command, 48)
                    ));
                    review_panel.strong_confirm_index = Some(index);
                }
            }
            CommandRisk::Low | CommandRisk::Medium => {
                self.app.record_agent_review(format!(
                    "approve {}",
                    preview_text(&proposal.command, 48)
                ));
                self.app.submit_selected_agent_command_proposal()?;
                self.review_panel = None;
            }
        }

        self.touch_cursor_blink();
        self.update_title();
        self.request_redraw();
        Ok(true)
    }

    fn confirm_review_with_text(&mut self, text: &str) -> Result<bool, Box<dyn Error>> {
        let Some(review_panel) = self.review_panel.as_mut() else {
            return Ok(false);
        };
        let Some(index) = self.app.selected_agent_command_proposal_index() else {
            return Ok(true);
        };
        if review_panel.strong_confirm_index != Some(index) {
            return Ok(true);
        }

        if text.eq_ignore_ascii_case("y") {
            if let Some(proposal) = self.app.selected_agent_command_proposal() {
                self.app.record_agent_review(format!(
                    "confirm {}",
                    preview_text(&proposal.command, 48)
                ));
            }
            self.app.submit_selected_agent_command_proposal()?;
            self.review_panel = None;
        }

        self.touch_cursor_blink();
        self.update_title();
        self.request_redraw();
        Ok(true)
    }

    fn handle_command_palette_key(
        &mut self,
        event: &winit::event::KeyEvent,
    ) -> Result<bool, Box<dyn Error>> {
        if !event.state.is_pressed() {
            return Ok(self.command_palette.is_some());
        }

        let Some(palette) = self.command_palette.as_mut() else {
            return Ok(false);
        };

        match event.logical_key.as_ref() {
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape) => {
                self.command_palette = None;
            }
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Enter) => {
                let command = palette.selected_command();
                self.command_palette = None;
                if let Some(command) = command {
                    self.apply_palette_command(command)?;
                }
            }
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::ArrowDown)
            | winit::keyboard::Key::Named(winit::keyboard::NamedKey::Tab) => {
                palette.select_next();
            }
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::ArrowUp) => {
                palette.select_previous();
            }
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Backspace) => {
                palette.pop_query_char();
            }
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Space)
                if !self.modifiers.control_key()
                    && !self.modifiers.alt_key()
                    && !self.modifiers.super_key() =>
            {
                palette.push_query_text(" ");
            }
            _ if !self.modifiers.control_key()
                && !self.modifiers.alt_key()
                && !self.modifiers.super_key() =>
            {
                if let Some(text) = event.text.as_deref() {
                    palette.push_query_text(text);
                }
            }
            _ => {}
        }

        self.touch_cursor_blink();
        self.update_title();
        self.request_redraw();
        Ok(true)
    }

    fn handle_review_panel_key(
        &mut self,
        event: &winit::event::KeyEvent,
    ) -> Result<bool, Box<dyn Error>> {
        if self.review_panel.is_none() {
            return Ok(false);
        }
        if !event.state.is_pressed() {
            return Ok(true);
        }

        match event.logical_key.as_ref() {
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape) => {
                self.review_panel = None;
            }
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::ArrowUp) => {
                let _ = self.app.select_previous_agent_command_proposal();
                if let Some(review_panel) = self.review_panel.as_mut() {
                    review_panel.strong_confirm_index = None;
                }
            }
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::ArrowDown)
            | winit::keyboard::Key::Named(winit::keyboard::NamedKey::Tab) => {
                let _ = self.app.select_next_agent_command_proposal();
                if let Some(review_panel) = self.review_panel.as_mut() {
                    review_panel.strong_confirm_index = None;
                }
            }
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Home) => {
                let _ = self.app.select_oldest_agent_command_proposal();
                if let Some(review_panel) = self.review_panel.as_mut() {
                    review_panel.strong_confirm_index = None;
                }
            }
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::End) => {
                let _ = self.app.select_newest_agent_command_proposal();
                if let Some(review_panel) = self.review_panel.as_mut() {
                    review_panel.strong_confirm_index = None;
                }
            }
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Enter) => {
                return self.confirm_review_selection();
            }
            _ if !self.modifiers.control_key()
                && !self.modifiers.alt_key()
                && !self.modifiers.super_key() =>
            {
                let Some(text) = event.text.as_deref() else {
                    self.touch_cursor_blink();
                    self.update_title();
                    self.request_redraw();
                    return Ok(true);
                };
                return self.confirm_review_with_text(text);
            }
            _ => {}
        }

        self.touch_cursor_blink();
        self.update_title();
        self.request_redraw();
        Ok(true)
    }

    fn handle_patch_preview_key(
        &mut self,
        event: &winit::event::KeyEvent,
    ) -> Result<bool, Box<dyn Error>> {
        if self.patch_preview_browser.is_none() {
            return Ok(false);
        }
        if !event.state.is_pressed() {
            return Ok(true);
        }

        match event.logical_key.as_ref() {
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape) => {
                self.patch_preview_browser = None;
            }
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::ArrowUp) => {
                let _ = self.app.select_previous_agent_patch_preview();
            }
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::ArrowDown)
            | winit::keyboard::Key::Named(winit::keyboard::NamedKey::Tab) => {
                let _ = self.app.select_next_agent_patch_preview();
            }
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Home) => {
                let _ = self.app.select_oldest_agent_patch_preview();
            }
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::End) => {
                let _ = self.app.select_newest_agent_patch_preview();
            }
            _ => {}
        }

        self.touch_cursor_blink();
        self.update_title();
        self.request_redraw();
        Ok(true)
    }

    fn handle_agent_audit_key(
        &mut self,
        event: &winit::event::KeyEvent,
    ) -> Result<bool, Box<dyn Error>> {
        if self.agent_audit_browser.is_none() {
            return Ok(false);
        }
        if !event.state.is_pressed() {
            return Ok(true);
        }

        match event.logical_key.as_ref() {
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape) => {
                self.agent_audit_browser = None;
            }
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::ArrowUp) => {
                let _ = self.app.select_previous_agent_audit_entry();
            }
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::ArrowDown)
            | winit::keyboard::Key::Named(winit::keyboard::NamedKey::Tab) => {
                let _ = self.app.select_next_agent_audit_entry();
            }
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Home) => {
                let _ = self.app.select_oldest_agent_audit_entry();
            }
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::End)
            | winit::keyboard::Key::Named(winit::keyboard::NamedKey::Enter) => {
                let _ = self.app.select_newest_agent_audit_entry();
            }
            _ => {}
        }

        self.touch_cursor_blink();
        self.update_title();
        self.request_redraw();
        Ok(true)
    }

    fn handle_block_browser_key(
        &mut self,
        event: &winit::event::KeyEvent,
    ) -> Result<bool, Box<dyn Error>> {
        if self.block_browser.is_none() {
            return Ok(false);
        }
        if !event.state.is_pressed() {
            return Ok(true);
        }

        match event.logical_key.as_ref() {
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape) => {
                self.block_browser = None;
            }
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::ArrowUp) => {
                let _ = self.app.select_previous_command_block();
            }
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::ArrowDown)
            | winit::keyboard::Key::Named(winit::keyboard::NamedKey::Tab) => {
                let _ = self.app.select_next_command_block();
            }
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Home) => {
                let _ = self.app.select_oldest_command_block();
            }
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::End)
            | winit::keyboard::Key::Named(winit::keyboard::NamedKey::Enter) => {
                let _ = self.app.select_newest_command_block();
            }
            _ if !self.modifiers.control_key()
                && !self.modifiers.alt_key()
                && !self.modifiers.super_key() =>
            {
                let Some(text) = event.text.as_deref() else {
                    self.touch_cursor_blink();
                    self.update_title();
                    self.request_redraw();
                    return Ok(true);
                };
                match text.to_ascii_lowercase().as_str() {
                    "c" => {
                        if let Some(command) = self.app.copy_selected_command_block_command() {
                            self.clipboard.set_text(command);
                        }
                    }
                    "f" => {
                        let _ = self.app.toggle_selected_command_block_fold();
                    }
                    "j" => {
                        let _ = self.app.select_next_command_block();
                    }
                    "k" => {
                        let _ = self.app.select_previous_command_block();
                    }
                    "o" => {
                        if let Some(output) = self.app.copy_selected_command_block_output() {
                            self.clipboard.set_text(output);
                        }
                    }
                    "s" => {
                        if let Some(output) =
                            self.app.copy_selected_command_block_structured_output()
                        {
                            self.clipboard.set_text(output);
                        }
                    }
                    "x" => {
                        let _ = self.app.select_newest_failed_command_block();
                    }
                    _ => {}
                }
            }
            _ => {}
        }

        self.touch_cursor_blink();
        self.update_title();
        self.request_redraw();
        Ok(true)
    }
}

fn apply_debug_startup_layout(app: &mut DesktopApp) {
    let Some(layout) = std::env::var("NOCTRAIL_DEBUG_LAYOUT")
        .ok()
        .map(|layout| layout.trim().to_ascii_lowercase())
    else {
        return;
    };

    let split = match layout.as_str() {
        "split-horizontal" | "horizontal" => Some(SplitAxis::Horizontal),
        "split-vertical" | "vertical" | "two-pane" => Some(SplitAxis::Vertical),
        _ => None,
    };

    let Some(axis) = split else {
        warn!(layout, "ignoring unknown NOCTRAIL_DEBUG_LAYOUT");
        return;
    };

    if let Err(error) = app.split_active_pane_shell_with_axis(axis) {
        warn!(?error, layout, "failed to apply debug startup layout");
    }
}

fn direction_name(direction: FocusDirection) -> &'static str {
    match direction {
        FocusDirection::Left => "left",
        FocusDirection::Right => "right",
        FocusDirection::Up => "up",
        FocusDirection::Down => "down",
    }
}

fn split_axis_from_config(axis: LayoutSplitAxis) -> Option<SplitAxis> {
    match axis {
        LayoutSplitAxis::Auto => None,
        LayoutSplitAxis::Horizontal => Some(SplitAxis::Horizontal),
        LayoutSplitAxis::Vertical => Some(SplitAxis::Vertical),
    }
}

fn font_preferences_from_config(config: &FontConfig) -> FontPreferences {
    FontPreferences {
        family: config.family.clone(),
        size: config.size,
        fallback: config.fallback.clone(),
    }
}

fn srgb_component(value: u8) -> f64 {
    f64::from(value) / f64::from(u8::MAX)
}

fn preview_text(text: &str, max_chars: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut preview = normalized.chars().take(max_chars).collect::<String>();
    if normalized.chars().count() > max_chars {
        preview.push_str("...");
    }
    preview
}

fn preview_paths(paths: &[PathBuf], max_chars: usize) -> String {
    let joined = paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    preview_text(&joined, max_chars)
}

fn preview_diff(diff: &str, max_chars: usize) -> String {
    let normalized = diff
        .lines()
        .filter(|line| !line.trim().is_empty())
        .take(4)
        .collect::<Vec<_>>()
        .join(" | ");
    preview_text(&normalized, max_chars)
}

fn shell_submission_bytes(command: &str) -> Vec<u8> {
    let mut bytes = command.as_bytes().to_vec();
    bytes.push(b'\r');
    bytes
}

fn shell_exit_bytes() -> Vec<u8> {
    b"exit\r\n".to_vec()
}

fn review_output_command(marker: &str) -> String {
    #[cfg(windows)]
    {
        format!("echo {marker}")
    }

    #[cfg(not(windows))]
    {
        format!("printf '{marker}\\n'")
    }
}

fn review_file_command(path: &Path) -> String {
    #[cfg(windows)]
    {
        format!("cmd /C echo review-high>\"{}\"", path.display())
    }

    #[cfg(not(windows))]
    {
        format!("sh -lc 'printf review-high > \"{}\"'", path.display())
    }
}

fn review_patch_cli_command(path: &Path) -> Vec<String> {
    #[cfg(windows)]
    {
        vec![
            "cmd".to_string(),
            "/C".to_string(),
            format!("type \"{}\"", path.display()),
        ]
    }

    #[cfg(not(windows))]
    {
        vec![
            "sh".to_string(),
            "-lc".to_string(),
            format!("cat \"{}\"", path.display()),
        ]
    }
}

fn shell_integration_probe_bytes(
    command: &str,
    cwd: &str,
    exit_code: i32,
    duration_ms: u64,
    output: &str,
) -> Vec<u8> {
    [
        osc_marker_bytes("Prompt").as_slice(),
        osc_marker_bytes("CommandStart").as_slice(),
        osc_marker_pair_bytes("CommandText", command).as_slice(),
        osc_marker_pair_bytes("Cwd", cwd).as_slice(),
        output.as_bytes(),
        osc_marker_pair_bytes("ExitCode", exit_code.to_string().as_str()).as_slice(),
        osc_marker_pair_bytes("DurationMs", duration_ms.to_string().as_str()).as_slice(),
        osc_marker_bytes("CommandEnd").as_slice(),
    ]
    .concat()
}

fn osc_marker_bytes(marker: &str) -> Vec<u8> {
    format!("\x1b]1337;Noctrail;{marker}\x07").into_bytes()
}

fn osc_marker_pair_bytes(marker: &str, value: &str) -> Vec<u8> {
    format!("\x1b]1337;Noctrail;{marker};{value}\x07").into_bytes()
}

fn read_all_runtime_output_for_gui(app: &mut DesktopApp) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut output = Vec::new();
    let mut chunk = [0_u8; 1024];

    loop {
        let drain = {
            let runtime = app
                .pane_mut()
                .runtime_mut()
                .ok_or("active pane is missing a runtime")?;
            runtime.drain_output_budget()
        };
        for bytes in drain.chunks {
            app.advance_output(&bytes);
            output.extend_from_slice(&bytes);
        }
        if drain.remaining_bytes > 0 {
            continue;
        }

        let count = {
            let runtime = app
                .pane_mut()
                .runtime_mut()
                .ok_or("active pane is missing a runtime")?;
            runtime.read_output(&mut chunk)?
        };
        if count == 0 {
            break;
        }
        let bytes = chunk[..count].to_vec();
        app.advance_output(&bytes);
        output.extend_from_slice(&bytes);
    }

    Ok(output)
}

pub fn pane_chrome_from_theme(theme: &ThemeConfig, font: &FontConfig) -> PaneChromeConfig {
    let terminal_background = rgba_from_config(theme.color.background);
    let chrome_background = rgba_from_config(theme.color.chrome_background);
    let chrome_foreground = rgba_from_config(theme.color.chrome_foreground);
    let background = mix_rgba(chrome_background, terminal_background, 0.22);
    let status_background = mix_rgba(chrome_foreground, background, 0.95);
    let accent = rgba_from_config(theme.color.chrome_accent);
    let inactive = rgba_from_config(theme.border.inactive);
    let status_height = ((font.size * 1.78).round() as u16).clamp(20, 30);
    let status_spacing = ((font.size * 0.30).round() as u16).clamp(4, 8);
    PaneChromeConfig {
        border: PaneBorderStyle {
            width: usize::from(theme.border.width),
            active: rgba_from_config(theme.border.active),
            inactive: rgba_from_config(theme.border.inactive),
        },
        background,
        status_background,
        status_separator: mix_rgba(accent, status_background, 0.58),
        active_indicator: accent,
        inactive_indicator: mix_rgba(accent, inactive, 0.72),
        gap: theme.pane.gap,
        padding: theme.pane.padding,
        radius: theme.pane.radius,
        status_height,
        status_spacing,
    }
}

fn rgba_from_config(color: noctrail_config::RgbaColor) -> Rgba {
    Rgba {
        red: color.red,
        green: color.green,
        blue: color.blue,
        alpha: color.alpha,
    }
}

fn mouse_button(button: WinitMouseButton) -> Option<input::MouseButton> {
    match button {
        WinitMouseButton::Left => Some(input::MouseButton::Left),
        WinitMouseButton::Middle => Some(input::MouseButton::Middle),
        WinitMouseButton::Right => Some(input::MouseButton::Right),
        _ => None,
    }
}

fn scroll_lines(delta: MouseScrollDelta) -> i32 {
    match delta {
        MouseScrollDelta::LineDelta(_, y) => y.round() as i32,
        MouseScrollDelta::PixelDelta(delta) => {
            let lines = delta.y / 20.0;
            if lines.abs() >= 1.0 {
                lines.round() as i32
            } else {
                delta.y.signum() as i32
            }
        }
    }
}

fn exit_on_error<T, E>(event_loop: &ActiveEventLoop, result: Result<T, E>) {
    if result.is_err() {
        event_loop.exit();
    }
}

impl ApplicationHandler<GuiEvent> for GuiApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        info!("GUI application resumed");
        if self.window.is_none() && self.create_window(event_loop).is_err() {
            event_loop.exit();
            return;
        }
        if self.attach_output_pump().is_err() {
            event_loop.exit();
            return;
        }

        self.touch_cursor_blink();
        self.reschedule(event_loop);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if Some(window_id) != self.window_id() {
            return;
        }

        match event {
            WindowEvent::CloseRequested => {
                let _ = self.app.close_runtime();
                event_loop.exit();
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::Ime(ime) => exit_on_error(event_loop, self.handle_ime_event(ime)),
            WindowEvent::CursorMoved { position, .. } => {
                exit_on_error(event_loop, self.handle_cursor_moved(position));
            }
            WindowEvent::MouseInput { state, button, .. } => {
                exit_on_error(event_loop, self.handle_mouse_input(state, button));
            }
            WindowEvent::MouseWheel { delta, .. } => {
                exit_on_error(event_loop, self.handle_mouse_wheel(delta));
            }
            WindowEvent::KeyboardInput {
                event,
                ..
            } => {
                let key_pressed = event.state == ElementState::Pressed;
                if matches!(
                    self.shortcut_action(&event.logical_key),
                    Some(input::ShortcutAction::ToggleAgentAuditBrowser)
                ) && key_pressed
                {
                    self.toggle_agent_audit_browser();
                    return;
                }
                if matches!(
                    self.shortcut_action(&event.logical_key),
                    Some(input::ShortcutAction::ToggleAgentContextPreview)
                ) && key_pressed
                {
                    self.toggle_agent_context_preview();
                    return;
                }
                if matches!(
                    self.shortcut_action(&event.logical_key),
                    Some(input::ShortcutAction::ToggleBlockBrowser)
                ) && key_pressed
                {
                    self.toggle_block_browser();
                    return;
                }
                if matches!(
                    self.shortcut_action(&event.logical_key),
                    Some(input::ShortcutAction::TogglePatchPreview)
                ) && key_pressed
                {
                    self.toggle_patch_preview_browser();
                    return;
                }
                if matches!(
                    self.shortcut_action(&event.logical_key),
                    Some(input::ShortcutAction::ToggleReviewPanel)
                ) && key_pressed
                {
                    self.toggle_review_panel();
                    return;
                }
                if matches!(
                    self.shortcut_action(&event.logical_key),
                    Some(input::ShortcutAction::ToggleInputMode)
                ) && key_pressed
                {
                    self.toggle_input_mode();
                    return;
                }
                if matches!(
                    self.shortcut_action(&event.logical_key),
                    Some(input::ShortcutAction::ToggleCommandPalette)
                ) && key_pressed
                {
                    self.toggle_command_palette();
                    return;
                }
                match self.handle_review_panel_key(&event) {
                    Ok(true) => return,
                    Ok(false) => {}
                    Err(_) => {
                        event_loop.exit();
                        return;
                    }
                }
                match self.handle_patch_preview_key(&event) {
                    Ok(true) => return,
                    Ok(false) => {}
                    Err(_) => {
                        event_loop.exit();
                        return;
                    }
                }
                match self.handle_agent_audit_key(&event) {
                    Ok(true) => return,
                    Ok(false) => {}
                    Err(_) => {
                        event_loop.exit();
                        return;
                    }
                }
                match self.handle_block_browser_key(&event) {
                    Ok(true) => return,
                    Ok(false) => {}
                    Err(_) => {
                        event_loop.exit();
                        return;
                    }
                }
                match self.handle_command_palette_key(&event) {
                    Ok(true) => return,
                    Ok(false) => {}
                    Err(_) => {
                        event_loop.exit();
                        return;
                    }
                }
                if key_pressed
                    && let Some(action) = self.shortcut_action(&event.logical_key)
                {
                    match action {
                        input::ShortcutAction::ToggleInputMode => unreachable!(),
                        input::ShortcutAction::ToggleAgentAuditBrowser => unreachable!(),
                        input::ShortcutAction::ToggleAgentContextPreview => unreachable!(),
                        input::ShortcutAction::ToggleBlockBrowser => unreachable!(),
                        input::ShortcutAction::ToggleCommandPalette => unreachable!(),
                        input::ShortcutAction::TogglePatchPreview => unreachable!(),
                        input::ShortcutAction::ToggleReviewPanel => unreachable!(),
                        input::ShortcutAction::Copy => {
                            if let Some(text) = self.app.copy_selection_text() {
                                self.clipboard.set_text(text);
                            }
                        }
                        input::ShortcutAction::Paste => {
                            if let Some(text) = self.clipboard.get_text() {
                                let editor_mode = self.active_input_mode() == InputMode::Editor;
                                let write_result = if editor_mode {
                                    self.insert_editor_text(&text);
                                    Ok(0)
                                } else {
                                    self.app.paste_text(&text).map(|_| 0)
                                };
                                if write_result.is_err() {
                                    event_loop.exit();
                                    return;
                                }
                                if editor_mode {
                                    if self.refresh_editor_feedback("paste").is_err() {
                                        event_loop.exit();
                                        return;
                                    }
                                } else {
                                    self.touch_cursor_blink();
                                    self.request_redraw();
                                    self.update_title();
                                }
                            }
                        }
                        input::ShortcutAction::Focus(direction) => {
                            if self.app.focus_direction(direction).is_err() {
                                event_loop.exit();
                                return;
                            }
                            self.touch_cursor_blink();
                            self.request_redraw();
                            self.update_title();
                        }
                    }
                    return;
                }
                match self.handle_editor_key(&event) {
                    Ok(true) => return,
                    Ok(false) => {}
                    Err(_) => {
                        event_loop.exit();
                        return;
                    }
                }
                let key_without_modifiers = event.key_without_modifiers();
                let key_without_modifiers = match key_without_modifiers.as_ref() {
                    winit::keyboard::Key::Character(text) => Some(&text[..]),
                    _ => None,
                };
                if let Some(bytes) = input::encode_key_event(input::KeyboardEncodeRequest {
                    state: event.state,
                    logical_key: &event.logical_key,
                    text: event.text.as_deref(),
                    key_without_modifiers,
                    modifiers: self.modifiers,
                    repeat: event.repeat,
                }) {
                    if self.app.write_input(&bytes).is_err() {
                        event_loop.exit();
                        return;
                    }
                    self.touch_cursor_blink();
                    self.request_redraw();
                    self.update_title();
                }
            }
            WindowEvent::Resized(size) => {
                let scale_factor = self
                    .window
                    .as_ref()
                    .map(|window| window.scale_factor())
                    .unwrap_or(1.0);
                if self.sync_surface(size, scale_factor).is_err() {
                    event_loop.exit();
                    return;
                }
                self.touch_cursor_blink();
                self.update_title();
                self.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.render_current_frame() {
                    self.record_gpu_fallback(error.to_string());
                    self.update_title();
                }
                self.complete_latency_probe();
                self.update_title();
            }
            WindowEvent::Focused(true) => {
                self.window_focused = true;
                self.startup_focus_retry_until = None;
                self.touch_cursor_blink();
                self.request_redraw();
            }
            WindowEvent::Focused(false) => {
                self.window_focused = false;
                self.cursor_visible = true;
                self.request_redraw();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            event_loop.exit();
            return;
        }

        let _ = self.poll_config_reload();
        let _ = self.drain_output_events();
        if matches!(self.app.refresh_runtime_statuses(), Ok(true)) {
            self.update_title();
        }
        let now = Instant::now();
        if !self.window_focused && self.startup_focus_retry_until.is_some() {
            if let Some(until) = self.startup_focus_retry_until {
                if now <= until {
                    if now >= self.next_startup_focus_retry_at {
                        if let Some(window) = self.window.as_ref() {
                            window.focus_window();
                        }
                        self.next_startup_focus_retry_at = now + STARTUP_FOCUS_RETRY_INTERVAL;
                    }
                } else {
                    self.startup_focus_retry_until = None;
                }
            }
        }
        if self.advance_transition(now) {
            self.update_title();
            self.request_redraw();
        }
        if self.advance_cursor_blink(now) {
            self.update_title();
            self.request_redraw();
        }
        self.reschedule(event_loop);
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        let _ = self.app.close_runtime();
        if let Some(handle) = self.output_thread.take() {
            let _ = handle.join();
        }
        if let Some(probe) = self.latency_probe.as_ref() {
            probe.flush();
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: GuiEvent) {
        match event {
            GuiEvent::RuntimeOutput => {
                let _ = self.drain_output_events();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DesktopFrame;
    use noctrail_layout::WorkspaceId;
    use noctrail_render::{PaneBorderStyle, RenderBackend, RenderPlan, RenderRect};
    use noctrail_runtime::PaneId;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn surface_size_is_clamped_to_terminal_cells() {
        let font = FontConfig::default();
        assert_eq!(
            terminal_size_from_surface(PhysicalSize::new(7, 15), &font, 1.0),
            PtySize::new(1, 1)
        );
        assert_eq!(
            terminal_size_from_surface(PhysicalSize::new(320, 160), &font, 1.0),
            PtySize::new(34, 6)
        );
        assert_eq!(
            terminal_size_from_surface(PhysicalSize::new(320, 160), &font, 2.0),
            PtySize::new(17, 3)
        );
    }

    #[test]
    fn frame_title_reflects_state() {
        let frame = DesktopFrame {
            workspace_id: WorkspaceId::new(1),
            is_scratch: false,
            pane_id: PaneId::new(7),
            pane_surface: LayoutRect::new(0, 0, 120, 80),
            status_surface: LayoutRect::new(0, 0, 120, 22),
            surface: LayoutRect::new(0, 0, 120, 80),
            terminal_size: PtySize::new(80, 24),
            process_id: Some(1234),
            status_line: crate::PaneStatusLine {
                shell: Some("zsh".to_string()),
                cwd: Some(PathBuf::from("/tmp/noctrail")),
                git_branch: Some("main".to_string()),
                exit_status: Some("code 0".to_string()),
            },
            render_plan: RenderPlan {
                backend: RenderBackend::Gpu,
                pane_rect: RenderRect::new(0, 0, 120, 80),
                viewport: RenderRect::new(0, 0, 120, 80),
                damage: noctrail_term::DamageSet {
                    dirty_rows: vec![1],
                    full_frame: false,
                },
                scrollback_rows: 9,
                cursor: noctrail_term::Cursor { row: 1, col: 2 },
                alternate_screen: false,
                selection: None,
                chrome: Vec::new(),
                active: true,
                border: PaneBorderStyle::default(),
                corner_radius: 0,
                rows: Vec::new(),
            },
        };

        let title = frame_title(&frame, true);
        assert!(title.contains("pane 7"));
        assert!(title.contains("pid 1234"));
        assert!(title.contains("rows 0"));
        assert!(title.contains("gpu"));
        assert!(title.contains("cursor on"));
        assert!(title.contains("shell zsh"));
        assert!(title.contains("cwd /tmp/noctrail"));
        assert!(title.contains("git main"));
        assert!(title.contains("exit code 0"));
    }

    #[test]
    fn gpu_fallback_switches_backend_without_exiting() {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(10, 3));
        let mut gui = GuiApp::new(app, GuiLaunchOptions::default());
        gui.app.set_backend(RenderBackend::Gpu);

        gui.record_gpu_fallback("adapter missing".to_string());

        assert_eq!(gui.app.backend(), RenderBackend::Software);
        assert!(gui.renderer.is_none());
        assert_eq!(gui.gpu_fallback_error.as_deref(), Some("adapter missing"));
    }

    #[test]
    fn visual_effects_policy_keeps_requested_opacity_without_blur() {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(10, 3));
        let theme = ThemeConfig {
            opacity: 0.72,
            ..ThemeConfig::default()
        };
        let mut gui = GuiApp::new(
            app,
            GuiLaunchOptions {
                renderer_backend: RenderBackend::Gpu,
                theme,
                ..GuiLaunchOptions::default()
            },
        );
        gui.app.set_backend(RenderBackend::Gpu);

        let effects = gui.visual_effects_policy();

        assert_eq!(effects.requested_opacity, 0.72);
        assert_eq!(effects.effective_opacity, 0.72);
        assert!(effects.window_transparent);
        assert_eq!(effects.transparency_fallback_reason, None);
        assert!(!effects.low_power_enabled);
        assert_eq!(effects.blur_mode, BlurMode::Disabled);
        assert_eq!(effects.blur_fallback_reason, None);
    }

    #[test]
    fn visual_effects_policy_uses_tinted_solid_when_blur_is_requested() {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(10, 3));
        let mut theme = ThemeConfig {
            opacity: 0.72,
            ..ThemeConfig::default()
        };
        theme.blur.enabled = true;
        theme.blur.fallback_tint_opacity = 0.9;
        let mut gui = GuiApp::new(
            app,
            GuiLaunchOptions {
                renderer_backend: RenderBackend::Gpu,
                theme,
                ..GuiLaunchOptions::default()
            },
        );
        gui.app.set_backend(RenderBackend::Gpu);

        let effects = gui.visual_effects_policy();

        assert!(!effects.low_power_enabled);
        assert_eq!(effects.effective_opacity, 0.9);
        assert!(!effects.window_transparent);
        assert_eq!(effects.transparency_fallback_reason, None);
        assert_eq!(effects.blur_mode, BlurMode::TintedSolid);
        assert_eq!(effects.blur_fallback_reason, Some("unsupported-platform"));
    }

    #[test]
    fn visual_effects_policy_falls_back_in_safe_mode() {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(10, 3));
        let mut theme = ThemeConfig {
            opacity: 0.72,
            ..ThemeConfig::default()
        };
        theme.blur.enabled = true;
        let gui = GuiApp::new(
            app,
            GuiLaunchOptions {
                safe_mode: true,
                renderer_backend: RenderBackend::Gpu,
                theme,
                ..GuiLaunchOptions::default()
            },
        );

        let effects = gui.visual_effects_policy();

        assert!(!effects.low_power_enabled);
        assert_eq!(effects.effective_opacity, 1.0);
        assert!(!effects.window_transparent);
        assert_eq!(effects.transparency_fallback_reason, Some("safe-mode"));
        assert_eq!(effects.blur_mode, BlurMode::TintedSolid);
        assert_eq!(effects.blur_fallback_reason, Some("safe-mode"));
    }

    #[test]
    fn visual_effects_policy_falls_back_on_software_backend() {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(10, 3));
        let mut theme = ThemeConfig {
            opacity: 0.72,
            ..ThemeConfig::default()
        };
        theme.blur.enabled = true;
        let gui = GuiApp::new(
            app,
            GuiLaunchOptions {
                renderer_backend: RenderBackend::Software,
                theme,
                ..GuiLaunchOptions::default()
            },
        );

        let effects = gui.visual_effects_policy();

        assert!(!effects.low_power_enabled);
        assert_eq!(effects.effective_opacity, 1.0);
        assert!(!effects.window_transparent);
        assert_eq!(
            effects.transparency_fallback_reason,
            Some("software-backend")
        );
        assert_eq!(effects.blur_mode, BlurMode::TintedSolid);
        assert_eq!(effects.blur_fallback_reason, Some("software-backend"));
    }

    #[test]
    fn safe_mode_launch_options_skip_gpu_attempts() {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(10, 3));
        let gui = GuiApp::new(
            app,
            GuiLaunchOptions {
                safe_mode: true,
                renderer_backend: RenderBackend::Gpu,
                ..GuiLaunchOptions::default()
            },
        );

        assert!(!gui.should_attempt_gpu_renderer());
    }

    #[test]
    fn visual_effects_policy_disables_blur_in_low_power_mode() {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(10, 3));
        let mut theme = ThemeConfig {
            opacity: 0.72,
            ..ThemeConfig::default()
        };
        theme.blur.enabled = true;
        theme.low_power.enabled = true;
        let mut gui = GuiApp::new(
            app,
            GuiLaunchOptions {
                renderer_backend: RenderBackend::Gpu,
                theme,
                ..GuiLaunchOptions::default()
            },
        );
        gui.app.set_backend(RenderBackend::Gpu);

        let effects = gui.visual_effects_policy();

        assert!(effects.low_power_enabled);
        assert_eq!(effects.effective_opacity, 0.72);
        assert!(effects.window_transparent);
        assert_eq!(effects.transparency_fallback_reason, None);
        assert_eq!(effects.blur_mode, BlurMode::Disabled);
        assert_eq!(effects.blur_fallback_reason, Some("low-power"));
        assert!(gui.animation_duration().is_none());
    }

    #[test]
    fn software_backend_launch_options_skip_gpu_attempts() {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(10, 3));
        let gui = GuiApp::new(
            app,
            GuiLaunchOptions {
                safe_mode: false,
                renderer_backend: RenderBackend::Software,
                ..GuiLaunchOptions::default()
            },
        );

        assert!(!gui.should_attempt_gpu_renderer());
    }

    #[test]
    fn config_reload_updates_theme_font_and_cursor_timing() {
        let path = temp_config_path("theme-reload");
        fs::write(
            &path,
            "[font]\nfamily = \"JetBrainsMono Nerd Font\"\nsize = 14.0\n\n[theme]\nopacity = 1.0\n\n[theme.pane]\ngap = 8\npadding = 6\nradius = 8\n\n[theme.animation]\nenabled = true\nduration-ms = 120\n\n[theme.low-power]\nenabled = false\n\n[theme.cursor]\nblink-interval-ms = 600\n",
        )
        .expect("write initial config");

        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(10, 3));
        let mut gui = GuiApp::new(
            app,
            GuiLaunchOptions {
                safe_mode: false,
                renderer_backend: RenderBackend::Software,
                config_path: Some(path.clone()),
                ..GuiLaunchOptions::default()
            },
        );

        fs::write(
            &path,
            "[font]\nfamily = \"Iosevka\"\nsize = 16.0\nfallback = [\"Noto Sans CJK SC\"]\n\n[theme]\nopacity = 0.75\n\n[theme.pane]\ngap = 10\npadding = 4\nradius = 12\n\n[theme.animation]\nenabled = true\nduration-ms = 200\n\n[theme.low-power]\nenabled = true\n\n[theme.cursor]\nblink-interval-ms = 250\n",
        )
        .expect("write changed config");

        assert!(gui.poll_config_reload());
        assert_eq!(gui.font.family, "Iosevka");
        assert_eq!(gui.font.size, 16.0);
        assert_eq!(gui.font_preferences.family, "Iosevka");
        assert_eq!(gui.frame_interval, Duration::from_millis(250));
        assert_eq!(gui.theme.opacity, 0.75);
        assert_eq!(gui.app.pane_chrome().gap, 10);
        assert_eq!(gui.app.pane_chrome().padding, 4);
        assert_eq!(gui.app.pane_chrome().radius, 12);
        assert!(gui.theme.animation.enabled);
        assert_eq!(gui.theme.animation.duration_ms, 200);
        assert!(gui.theme.low_power.enabled);
        assert!(gui.animation_duration().is_none());
        assert!(gui.theme_reload_error.is_none());
        assert!(!gui.poll_config_reload());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn command_palette_filters_commands_by_query() {
        let mut palette = CommandPalette::new();
        palette.push_query_text("workspace 2");

        assert_eq!(
            palette.selected_command(),
            Some(PaletteCommand::Workspace(WorkspaceId::new(2)))
        );
    }

    #[test]
    fn block_browser_opens_and_selects_the_newest_block() {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(12, 4));
        let mut gui = GuiApp::new(app, GuiLaunchOptions::default());
        gui.app.set_block_observer_enabled(true);
        gui.app.advance_output(&block_probe_bytes(
            "echo first",
            "/tmp/noctrail-first",
            0,
            10,
            "first output",
        ));
        gui.app.advance_output(&block_probe_bytes(
            "echo second",
            "/tmp/noctrail-second",
            0,
            11,
            "second output",
        ));
        gui.app.set_block_observer_enabled(false);

        gui.toggle_block_browser();

        assert!(gui.block_browser.is_some());
        assert!(gui.app.block_observer_enabled());
        assert_eq!(gui.app.selected_command_block_index(), Some(1));
        assert_eq!(
            gui.app
                .selected_command_block()
                .and_then(|block| block.command.as_deref()),
            Some("echo second")
        );
    }

    #[test]
    fn agent_context_preview_title_shows_block_selection_cwd_and_files() {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(12, 4));
        let mut gui = GuiApp::new(app, GuiLaunchOptions::default());
        gui.app.set_block_observer_enabled(true);
        gui.app.advance_output(&block_probe_bytes(
            "cargo test -p noctrail-app",
            "/tmp/noctrail-agent",
            0,
            17,
            "alpha beta\r\ngamma delta\r\n",
        ));
        let _ = gui.app.select_newest_command_block();
        gui.app.select_viewport_range(
            Position { row: 0, col: 0 },
            Position { row: 0, col: 4 },
            SelectionMode::Normal,
        );
        gui.app.set_agent_explicit_files(vec![
            PathBuf::from("/tmp/noctrail/Cargo.toml"),
            PathBuf::from("/tmp/noctrail/crates/noctrail-app/src/lib.rs"),
        ]);

        gui.toggle_agent_context_preview();

        let title = gui.title_text();
        assert!(gui.agent_context_browser.is_some());
        assert!(title.contains("agent-context"));
        assert!(title.contains("block cargo test -p noctrail-app"));
        assert!(title.contains("output alpha beta gamma delta"));
        assert!(
            gui.app.agent_context_preview().selection.is_some(),
            "agent context preview should expose a selection"
        );
        assert!(title.contains(" | selection "), "{title}");
        assert!(title.contains("cwd /tmp/noctrail-agent"));
        assert!(title.contains("files 2"));
        assert!(title.contains("/tmp/noctrail/Cargo.toml"));
    }

    #[test]
    fn audit_browser_title_reflects_selected_entry() {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(12, 4));
        let mut gui = GuiApp::new(app, GuiLaunchOptions::default());
        let preview = crate::AgentContextPreview {
            current_block: Some(crate::AgentContextBlock {
                command: Some("echo token=sk-live-secretvalue12345".to_string()),
                output: "ok".to_string(),
                exit_code: Some(0),
            }),
            selection: None,
            cwd: Some(PathBuf::from("/tmp/noctrail-agent-audit")),
            explicit_files: Vec::new(),
        };
        gui.app.record_agent_context_access(&preview);
        gui.app.record_agent_review("approve git status");

        gui.toggle_agent_audit_browser();
        let title = gui.title_text();

        assert!(title.contains("agent-audit"));
        assert!(title.contains("entries 2"));
        assert!(title.contains("review approve git status"));
    }

    #[test]
    fn title_reflects_agent_command_proposals() {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(12, 4));
        let mut gui = GuiApp::new(app, GuiLaunchOptions::default());
        gui.app
            .set_agent_command_proposals(vec![noctrail_agent::CommandProposal {
                command: "git status".to_string(),
                reason: "Inspect the repository state.".to_string(),
                risk: noctrail_agent::CommandRisk::Low,
                permission: noctrail_agent::CommandPermission::Review,
            }]);

        let title = gui.title_text();

        assert!(title.contains("agent-proposal"));
        assert!(title.contains("risk low"));
        assert!(title.contains("permission review"));
        assert!(title.contains("cmd git status"));
        assert!(title.contains("reason Inspect the repository state."));
    }

    #[test]
    fn patch_preview_title_reflects_selected_diff() {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(12, 4));
        let mut gui = GuiApp::new(app, GuiLaunchOptions::default());
        gui.app
            .set_agent_patch_previews(vec![noctrail_agent::PatchPreview {
                path: PathBuf::from("src/lib.rs"),
                reason: "Preview a one-line patch.".to_string(),
                diff: "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,2 @@\n-foo\n+foo\n+bar\n"
                    .to_string(),
            }]);

        gui.toggle_patch_preview_browser();
        let title = gui.title_text();

        assert!(title.contains("patch-preview"));
        assert!(title.contains("file src/lib.rs"));
        assert!(title.contains("reason Preview a one-line patch."));
        assert!(title.contains("diff --- a/src/lib.rs"));
    }

    #[test]
    fn review_panel_title_reflects_selected_proposal_and_arm_state() -> Result<(), Box<dyn Error>> {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(12, 4));
        let mut gui = GuiApp::new(app, GuiLaunchOptions::default());
        gui.app
            .set_agent_command_proposals(vec![noctrail_agent::CommandProposal {
                command: "rm -rf build".to_string(),
                reason: "Remove an inconsistent build directory.".to_string(),
                risk: noctrail_agent::CommandRisk::High,
                permission: noctrail_agent::CommandPermission::StrongReview,
            }]);

        gui.toggle_review_panel();
        let before = gui.title_text();
        assert!(before.contains("review"));
        assert!(before.contains("press enter to arm"));

        let _ = gui.confirm_review_selection()?;
        let armed = gui.title_text();
        assert!(armed.contains("confirm y"));
        assert_eq!(
            gui.review_panel
                .as_ref()
                .and_then(|panel| panel.strong_confirm_index),
            Some(0)
        );
        Ok(())
    }

    #[test]
    fn block_browser_title_reflects_preview_and_fold_state() {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(12, 4));
        let mut gui = GuiApp::new(app, GuiLaunchOptions::default());
        gui.app.set_block_observer_enabled(true);
        gui.app.advance_output(&block_probe_bytes(
            "cargo test -p noctrail-app",
            "/tmp/noctrail-blocks",
            7,
            1200,
            "{\"line\":\"one\",\"count\":2}\n",
        ));
        gui.block_browser = Some(BlockBrowser);
        let _ = gui.app.select_newest_command_block();

        let unfolded = gui.title_text();
        assert!(unfolded.contains("blocks on 1/100 | failures 1 sel 1"));
        assert!(unfolded.contains("lens json object 2 keys"));
        assert!(unfolded.contains("cmd cargo test -p noctrail-app"));
        assert!(unfolded.contains("code 7"));
        assert!(unfolded.contains("dur 1200ms"));
        assert!(unfolded.contains("out {\"line\":\"one\",\"count\":2}"));

        let _ = gui.app.toggle_selected_command_block_fold();
        let folded = gui.title_text();
        assert!(folded.contains("| folded"));
        assert!(!folded.contains("out {\"line\":\"one\",\"count\":2}"));
    }

    #[test]
    fn block_browser_title_highlights_failures() {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(12, 4));
        let mut gui = GuiApp::new(app, GuiLaunchOptions::default());
        gui.app.set_block_observer_enabled(true);
        gui.app.advance_output(&block_probe_bytes(
            "echo ok",
            "/tmp/noctrail-ok",
            0,
            1,
            "ok output\n",
        ));
        gui.app.advance_output(&block_probe_bytes(
            "echo fail",
            "/tmp/noctrail-fail",
            7,
            2,
            "failure output\n",
        ));
        gui.block_browser = Some(BlockBrowser);
        let _ = gui.app.select_newest_failed_command_block();

        let title = gui.title_text();
        assert!(title.contains("failures 1"));
        assert!(title.contains("| FAIL"));
        assert!(title.contains("code 7"));
    }

    #[test]
    fn block_browser_can_copy_structured_output() {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(12, 4));
        let mut gui = GuiApp::new(app, GuiLaunchOptions::default());
        gui.app.set_block_observer_enabled(true);
        gui.app.advance_output(&block_probe_bytes(
            "cat structured",
            "/tmp/noctrail-structured",
            0,
            18,
            "name,count\nalpha,1\nbeta,2\n",
        ));
        gui.block_browser = Some(BlockBrowser);
        let _ = gui.app.select_newest_command_block();

        if let Some(output) = gui.app.copy_selected_command_block_structured_output() {
            gui.clipboard.set_text(output);
        }
        assert_eq!(
            gui.clipboard.get_text().as_deref(),
            Some("name,count\nalpha,1\nbeta,2\n")
        );
    }

    #[test]
    fn command_palette_executes_split_horizontal() -> Result<(), Box<dyn Error>> {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(12, 4));
        let mut theme = ThemeConfig::default();
        theme.pane.gap = 0;
        theme.pane.padding = 0;
        theme.pane.radius = 0;
        let mut gui = GuiApp::new(
            app,
            GuiLaunchOptions {
                theme,
                ..GuiLaunchOptions::default()
            },
        );

        PaletteCommand::SplitHorizontal
            .execute(&mut gui.app, gui.launch_options.layout.resize_step)?;

        assert_eq!(
            gui.app.frame_for_pane(PaneId::new(1))?.surface,
            LayoutRect::new(0, 0, 120, 20)
        );
        let split = gui
            .app
            .active_pane_id()
            .expect("split pane should be active");
        assert_eq!(
            gui.app.frame_for_pane(split)?.surface,
            LayoutRect::new(0, 20, 120, 20)
        );
        let split_status = gui
            .app
            .pane_mut_by_id(split)
            .expect("split pane should exist")
            .close_runtime()?;
        assert!(split_status.is_some());
        Ok(())
    }

    #[test]
    fn command_palette_executes_workspace_and_scratch_commands() -> Result<(), Box<dyn Error>> {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(12, 4));
        let mut gui = GuiApp::new(app, GuiLaunchOptions::default());

        PaletteCommand::Workspace(WorkspaceId::new(2))
            .execute(&mut gui.app, gui.launch_options.layout.resize_step)?;
        assert_eq!(gui.app.active_workspace_id(), WorkspaceId::new(2));

        PaletteCommand::ToggleScratch
            .execute(&mut gui.app, gui.launch_options.layout.resize_step)?;
        assert!(gui.app.scratch_visible());
        assert!(gui.app.frame().is_scratch);

        let scratch_pane = gui
            .app
            .scratch_pane_id()
            .expect("scratch pane should exist");
        let scratch_status = gui
            .app
            .pane_mut_by_id(scratch_pane)
            .expect("scratch pane should exist")
            .close_runtime()?;
        assert!(scratch_status.is_some());

        let workspace_pane = gui.app.toggle_scratch()?;
        let workspace_status = gui
            .app
            .pane_mut_by_id(workspace_pane)
            .expect("workspace pane should exist")
            .close_runtime()?;
        assert!(workspace_status.is_some());
        Ok(())
    }

    #[test]
    fn split_command_starts_pane_transition_when_animation_is_enabled() -> Result<(), Box<dyn Error>>
    {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(12, 4));
        let mut gui = GuiApp::new(app, GuiLaunchOptions::default());

        gui.apply_palette_command(PaletteCommand::SplitHorizontal)?;

        let transition = gui.transition.as_ref().expect("transition should start");
        assert_eq!(transition.kind, TransitionKind::Pane);
        assert_eq!(transition.duration, Duration::from_millis(120));
        assert!(!transition.panes.is_empty());
        Ok(())
    }

    #[test]
    fn workspace_command_starts_workspace_transition() -> Result<(), Box<dyn Error>> {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(12, 4));
        let mut gui = GuiApp::new(app, GuiLaunchOptions::default());

        gui.apply_palette_command(PaletteCommand::Workspace(WorkspaceId::new(2)))?;

        let transition = gui.transition.as_ref().expect("transition should start");
        assert_eq!(transition.kind, TransitionKind::Workspace);
        assert!(!transition.panes.is_empty());
        Ok(())
    }

    #[test]
    fn animation_off_switch_skips_transition() -> Result<(), Box<dyn Error>> {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(12, 4));
        let mut theme = ThemeConfig::default();
        theme.animation.enabled = false;
        let mut gui = GuiApp::new(
            app,
            GuiLaunchOptions {
                theme,
                ..GuiLaunchOptions::default()
            },
        );

        gui.apply_palette_command(PaletteCommand::SplitHorizontal)?;

        assert!(gui.transition.is_none());
        Ok(())
    }

    #[test]
    fn low_power_mode_skips_transition_even_when_animation_is_enabled() -> Result<(), Box<dyn Error>>
    {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(12, 4));
        let mut theme = ThemeConfig::default();
        theme.low_power.enabled = true;
        let mut gui = GuiApp::new(
            app,
            GuiLaunchOptions {
                theme,
                ..GuiLaunchOptions::default()
            },
        );

        gui.apply_palette_command(PaletteCommand::SplitHorizontal)?;

        assert!(gui.transition.is_none());
        Ok(())
    }

    #[test]
    fn advance_transition_clears_finished_animation() -> Result<(), Box<dyn Error>> {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(12, 4));
        let mut gui = GuiApp::new(app, GuiLaunchOptions::default());

        gui.apply_palette_command(PaletteCommand::SplitHorizontal)?;
        let deadline = gui
            .transition
            .as_ref()
            .expect("transition should start")
            .deadline();

        assert!(gui.advance_transition(deadline));
        assert!(gui.transition.is_none());
        Ok(())
    }

    #[test]
    fn cursor_blink_only_flips_after_deadline() {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(10, 3));
        let mut gui = GuiApp::new(app, GuiLaunchOptions::default());
        let now = Instant::now();
        gui.window_focused = true;
        gui.next_cursor_blink_at = now + Duration::from_millis(50);

        assert!(!gui.advance_cursor_blink(now + Duration::from_millis(20)));
        assert!(gui.cursor_visible);

        assert!(gui.advance_cursor_blink(now + Duration::from_millis(50)));
        assert!(!gui.cursor_visible);
        assert!(gui.next_cursor_blink_at > now + Duration::from_millis(50));
    }

    #[test]
    fn cursor_blink_stops_when_window_is_unfocused() {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(10, 3));
        let mut gui = GuiApp::new(app, GuiLaunchOptions::default());
        let now = Instant::now();
        gui.window_focused = false;
        gui.next_cursor_blink_at = now;

        assert!(!gui.advance_cursor_blink(now + Duration::from_secs(1)));
        assert!(gui.cursor_visible);
    }

    #[test]
    fn ime_preedit_updates_gui_state() -> Result<(), Box<dyn Error>> {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(10, 3));
        let mut gui = GuiApp::new(app, GuiLaunchOptions::default());

        gui.handle_ime_event(Ime::Preedit("中🙂".to_string(), None))?;
        assert_eq!(gui.ime_preedit.as_deref(), Some("中🙂"));
        assert_eq!(gui.app.frame().render_plan.cursor.col, 0);
        assert!(rendered_text(&gui.app.frame()).trim().is_empty());

        gui.handle_ime_event(Ime::Preedit(String::new(), None))?;
        assert!(gui.ime_preedit.is_none());
        Ok(())
    }

    #[test]
    fn mouse_drag_updates_selection() -> Result<(), Box<dyn Error>> {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(8, 2));
        let mut gui = GuiApp::new(app, GuiLaunchOptions::default());
        gui.app.set_pane_chrome(PaneChromeConfig::default())?;
        gui.app.advance_output(b"hello");

        gui.handle_cursor_moved(test_cell_position(&gui, 0, 0))?;
        gui.handle_mouse_input(ElementState::Pressed, WinitMouseButton::Left)?;
        gui.handle_cursor_moved(test_cell_position(&gui, 3, 0))?;
        gui.handle_mouse_input(ElementState::Released, WinitMouseButton::Left)?;

        assert_eq!(gui.app.copy_selection_text().as_deref(), Some("hell"));
        assert!(gui.app.frame().render_plan.selection.is_some());
        Ok(())
    }

    #[test]
    fn wheel_scroll_moves_scrollback_view() -> Result<(), Box<dyn Error>> {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(8, 2));
        let mut theme = ThemeConfig::default();
        theme.pane.gap = 0;
        theme.pane.padding = 0;
        theme.pane.radius = 0;
        let mut gui = GuiApp::new(
            app,
            GuiLaunchOptions {
                theme,
                ..GuiLaunchOptions::default()
            },
        );
        gui.app.advance_output(b"one\r\ntwo\r\nthree");

        gui.handle_mouse_wheel(MouseScrollDelta::LineDelta(0.0, 1.0))?;
        let scrolled = rendered_text(&gui.app.frame());
        assert!(scrolled.contains("one"));
        assert!(scrolled.contains("two"));

        gui.handle_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0))?;
        let live = rendered_text(&gui.app.frame());
        assert!(live.contains("two"));
        assert!(live.contains("three"));
        Ok(())
    }

    #[test]
    fn output_pump_feeds_shell_output_into_render_plan() -> Result<(), Box<dyn Error>> {
        let app = DesktopApp::spawn_shell(LayoutRect::new(0, 0, 120, 80), PtySize::new(80, 24))?;
        let mut gui = GuiApp::new(app, GuiLaunchOptions::default());
        gui.attach_output_pump()?;

        gui.app
            .write_input(shell_command_bytes("NOCTRAIL_GUI_PUMP").as_slice())?;

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut observed = false;
        while Instant::now() < deadline {
            if gui.drain_output_events() {
                let frame = gui.app.frame();
                let text = frame
                    .render_plan
                    .rows
                    .iter()
                    .map(|row| {
                        row.glyphs
                            .iter()
                            .map(|glyph| glyph.text.as_str())
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if text.contains("NOCTRAIL_GUI_PUMP") {
                    observed = true;
                    break;
                }
            }
            thread::sleep(Duration::from_millis(20));
        }

        gui.app.write_input(shell_exit_bytes().as_slice())?;
        let _ = gui.app.close_runtime()?;
        if let Some(handle) = gui.output_thread.take() {
            let _ = handle.join();
        }

        assert!(
            observed,
            "output pump did not feed shell output into render plan"
        );
        Ok(())
    }

    #[test]
    fn drain_output_events_consumes_multiple_runtime_budgets_in_one_pass()
    -> Result<(), Box<dyn Error>> {
        let app = DesktopApp::spawn_shell(LayoutRect::new(0, 0, 120, 80), PtySize::new(80, 24))?;
        let mut gui = GuiApp::new(app, GuiLaunchOptions::default());
        gui.attach_output_pump()?;
        let marker = "NOCTRAIL_MULTI_DRAIN";

        gui.app
            .write_input(shell_large_output_command_bytes(marker).as_slice())?;

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && gui.app.buffered_runtime_output_bytes() == 0 {
            thread::sleep(Duration::from_millis(20));
        }

        assert!(
            gui.drain_output_events(),
            "expected buffered output to start draining"
        );

        let drain_deadline = Instant::now() + Duration::from_secs(5);
        let mut rendered = String::new();
        while Instant::now() < drain_deadline {
            rendered = rendered_text(&gui.app.frame()).replace('\n', "");
            if rendered.contains(marker) {
                break;
            }
            let _ = gui.drain_output_events();
            thread::sleep(Duration::from_millis(20));
        }
        gui.app.write_input(shell_exit_bytes().as_slice())?;
        let _ = gui.app.close_runtime()?;
        if let Some(handle) = gui.output_thread.take() {
            let _ = handle.join();
        }

        assert!(
            rendered.contains(marker),
            "multi-budget drain did not preserve trailing output: {rendered:?}"
        );
        Ok(())
    }

    #[test]
    fn review_panel_requires_strong_confirmation_for_high_risk_commands()
    -> Result<(), Box<dyn Error>> {
        let app = DesktopApp::spawn_shell(LayoutRect::new(0, 0, 120, 80), PtySize::new(80, 24))?;
        let mut gui = GuiApp::new(app, GuiLaunchOptions::default());
        let high_marker =
            std::env::temp_dir().join(format!("noctrail-review-panel-test-{}", std::process::id()));
        let _ = std::fs::remove_file(&high_marker);

        gui.app.set_agent_command_proposals(vec![
            noctrail_agent::CommandProposal {
                command: review_output_command("NOCTRAIL_REVIEW_LOW_TEST"),
                reason: "Inspect the shell before changing files.".to_string(),
                risk: noctrail_agent::CommandRisk::Low,
                permission: noctrail_agent::CommandPermission::Review,
            },
            noctrail_agent::CommandProposal {
                command: review_file_command(&high_marker),
                reason: "Rewrite shell-visible state.".to_string(),
                risk: noctrail_agent::CommandRisk::High,
                permission: noctrail_agent::CommandPermission::StrongReview,
            },
        ]);

        gui.toggle_review_panel();
        let _ = gui.confirm_review_selection()?;
        let _ = gui.app.select_next_agent_command_proposal();
        gui.toggle_review_panel();
        let _ = gui.confirm_review_selection()?;
        assert!(gui.review_panel.is_some());
        assert!(!high_marker.exists());

        let _ = gui.confirm_review_with_text("y")?;
        gui.app.write_input(
            shell_submission_bytes(&review_output_command("NOCTRAIL_REVIEW_DONE_TEST")).as_slice(),
        )?;
        gui.app.write_input(shell_exit_bytes().as_slice())?;
        std::thread::sleep(Duration::from_millis(100));
        let output = read_all_runtime_output_for_gui(&mut gui.app)?;
        let _ = gui.app.close_runtime()?;

        let text = String::from_utf8_lossy(&output);
        assert!(text.contains("NOCTRAIL_REVIEW_LOW_TEST"));
        assert!(text.contains("NOCTRAIL_REVIEW_DONE_TEST"));
        assert!(high_marker.exists());
        let _ = std::fs::remove_file(high_marker);
        Ok(())
    }

    #[test]
    fn ime_commit_writes_text_to_shell() -> Result<(), Box<dyn Error>> {
        let app = DesktopApp::spawn_shell(LayoutRect::new(0, 0, 120, 80), PtySize::new(80, 24))?;
        let mut gui = GuiApp::new(app, GuiLaunchOptions::default());
        gui.attach_output_pump()?;
        let marker = "中🙂e\u{301}";

        gui.handle_ime_event(Ime::Commit(marker.to_string()))?;
        gui.app.write_input(b"\r")?;

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut observed = false;
        while Instant::now() < deadline {
            if gui.drain_output_events() {
                let frame = gui.app.frame();
                let text = frame
                    .render_plan
                    .rows
                    .iter()
                    .map(|row| {
                        row.glyphs
                            .iter()
                            .map(|glyph| glyph.text.as_str())
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if text.contains(marker) {
                    observed = true;
                    break;
                }
            }
            thread::sleep(Duration::from_millis(20));
        }

        gui.app.write_input(shell_exit_bytes().as_slice())?;
        let _ = gui.app.close_runtime()?;
        if let Some(handle) = gui.output_thread.take() {
            let _ = handle.join();
        }

        assert!(observed, "ime commit did not reach the shell");
        Ok(())
    }

    #[test]
    fn idle_schedule_probe_waits_for_the_blink_deadline() {
        let probe = idle_schedule_probe(&ThemeConfig::default());

        assert!(!probe.premature_redraw);
        assert!(probe.next_wakeup >= Duration::from_millis(500));
    }

    #[cfg(not(windows))]
    #[test]
    fn mouse_reporting_writes_reports_to_the_pty() -> Result<(), Box<dyn Error>> {
        let app = DesktopApp::spawn(
            LayoutRect::new(0, 0, 120, 80),
            mouse_report_hex_dump_command(),
            PtySize::new(80, 24),
        )?;
        let mut gui = GuiApp::new(app, GuiLaunchOptions::default());
        gui.app.advance_output(b"\x1b[?1000h\x1b[?1006h");

        gui.handle_cursor_moved(test_cell_position(&gui, 1, 1))?;
        gui.handle_mouse_input(ElementState::Pressed, WinitMouseButton::Left)?;

        let output = read_all_runtime_output(&mut gui.app)?;
        let text = String::from_utf8_lossy(&output);
        let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");

        let _ = gui.app.close_runtime()?;
        assert!(
            normalized.contains("1b 5b 3c 30 3b 32 3b 32 4d"),
            "mouse report bytes did not reach the foreground process: {text:?}"
        );
        Ok(())
    }

    fn shell_command_text(marker: &str) -> String {
        #[cfg(windows)]
        {
            format!("echo {marker}\r\n")
        }

        #[cfg(not(windows))]
        {
            format!("printf '{marker}\\n'\r")
        }
    }

    fn test_cell_position(gui: &GuiApp, col: usize, row: usize) -> PhysicalPosition<f64> {
        let frame = gui.app.frame();
        let metrics = gui.glyph_raster_config(&frame, 1.0);
        PhysicalPosition::new(
            f64::from(frame.surface.x) + f64::from(metrics.cell_width) * (col as f64 + 0.5),
            f64::from(frame.surface.y) + f64::from(metrics.line_height) * (row as f64 + 0.5),
        )
    }

    fn shell_command_bytes(marker: &str) -> Vec<u8> {
        shell_command_text(marker).into_bytes()
    }

    fn shell_large_output_command_bytes(marker: &str) -> Vec<u8> {
        #[cfg(windows)]
        {
            format!(
                "powershell -NoProfile -Command \"$payload='x'*40000; Write-Output $payload; Write-Output '{marker}'\"\r\n"
            )
            .into_bytes()
        }

        #[cfg(not(windows))]
        {
            format!(
                "i=0; while [ $i -lt 40000 ]; do printf x; i=$((i+1)); done; printf '\\n{marker}\\n'\r"
            )
            .into_bytes()
        }
    }

    fn shell_exit_bytes() -> Vec<u8> {
        b"exit\r\n".to_vec()
    }

    fn block_probe_bytes(
        command: &str,
        cwd: &str,
        exit_code: i32,
        duration_ms: u64,
        output: &str,
    ) -> Vec<u8> {
        [
            osc_marker_bytes("Prompt").as_slice(),
            osc_marker_bytes("CommandStart").as_slice(),
            osc_marker_pair_bytes("CommandText", command).as_slice(),
            osc_marker_pair_bytes("Cwd", cwd).as_slice(),
            output.as_bytes(),
            osc_marker_pair_bytes("ExitCode", exit_code.to_string().as_str()).as_slice(),
            osc_marker_pair_bytes("DurationMs", duration_ms.to_string().as_str()).as_slice(),
            osc_marker_bytes("CommandEnd").as_slice(),
        ]
        .concat()
    }

    fn osc_marker_bytes(marker: &str) -> Vec<u8> {
        format!("\x1b]1337;Noctrail;{marker}\x07").into_bytes()
    }

    fn osc_marker_pair_bytes(marker: &str, value: &str) -> Vec<u8> {
        format!("\x1b]1337;Noctrail;{marker};{value}\x07").into_bytes()
    }

    #[cfg(not(windows))]
    fn read_all_runtime_output(app: &mut DesktopApp) -> Result<Vec<u8>, Box<dyn Error>> {
        let runtime = app
            .pane_mut()
            .runtime_mut()
            .ok_or("active pane is missing a runtime")?;
        let mut output = Vec::new();
        let mut chunk = [0_u8; 1024];

        loop {
            let drain = runtime.drain_output_budget();
            for bytes in drain.chunks {
                output.extend_from_slice(&bytes);
            }
            if drain.remaining_bytes > 0 {
                continue;
            }

            let count = runtime.read_output(&mut chunk)?;
            if count == 0 {
                break;
            }
            output.extend_from_slice(&chunk[..count]);
        }

        Ok(output)
    }

    #[cfg(not(windows))]
    fn mouse_report_hex_dump_command() -> noctrail_pty::PtyCommand {
        let mut command = noctrail_pty::PtyCommand::new("sh");
        command.args(["-lc", "stty raw -echo; od -An -tx1 -N9"]);
        command
    }

    fn rendered_text(frame: &DesktopFrame) -> String {
        frame
            .render_plan
            .rows
            .iter()
            .map(|row| {
                row.glyphs
                    .iter()
                    .map(|glyph| glyph.text.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn temp_config_path(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("noctrail-gui-{label}-{unique}.toml"))
    }
}
