use std::{
    error::Error,
    io::Read,
    path::PathBuf,
    sync::Arc,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use noctrail_config::{ConfigReloader, FontConfig, ThemeConfig};
use noctrail_layout::{FocusDirection, LayoutRect, SplitAxis, WorkspaceId};
use noctrail_pty::{PtyOutputReader, PtySize};
use noctrail_render::{FontPreferences, GpuRenderer, PaneBorderStyle, RenderBackend, Rgba};
use noctrail_term::{MouseTrackingMode, Position, SelectionMode};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize},
    event::{ElementState, Ime, MouseButton as WinitMouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::ModifiersState,
    window::{Window, WindowId},
};

use crate::{DesktopApp, DesktopFrame, PaneChromeConfig, clipboard::ClipboardBridge, input};

const DEFAULT_WINDOW_WIDTH: u32 = 1280;
const DEFAULT_WINDOW_HEIGHT: u32 = 800;
const DEFAULT_CELL_WIDTH: u32 = 8;
const DEFAULT_CELL_HEIGHT: u32 = 16;
const PALETTE_RESIZE_DELTA: u16 = 5;
const ANIMATION_FRAME_INTERVAL: Duration = Duration::from_millis(16);

#[derive(Debug, Clone, Copy, PartialEq)]
struct VisualEffectsPolicy {
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
    pub renderer_backend: RenderBackend,
    pub config_path: Option<PathBuf>,
    pub theme: ThemeConfig,
    pub font: FontConfig,
}

impl Default for GuiLaunchOptions {
    fn default() -> Self {
        Self {
            safe_mode: false,
            renderer_backend: RenderBackend::Gpu,
            config_path: None,
            theme: ThemeConfig::default(),
            font: FontConfig::default(),
        }
    }
}

pub fn run_with_options(options: GuiLaunchOptions) -> Result<(), Box<dyn Error>> {
    let event_loop = EventLoop::new()?;
    let initial_surface = LayoutRect::new(
        0,
        0,
        DEFAULT_WINDOW_WIDTH as u16,
        DEFAULT_WINDOW_HEIGHT as u16,
    );
    let initial_terminal = terminal_size_from_surface(PhysicalSize::new(
        DEFAULT_WINDOW_WIDTH,
        DEFAULT_WINDOW_HEIGHT,
    ));
    let app = DesktopApp::spawn_shell(initial_surface, initial_terminal)?;
    let mut gui = GuiApp::new(app, options);
    event_loop.run_app(&mut gui)?;
    Ok(())
}

pub(crate) fn terminal_size_from_surface(size: PhysicalSize<u32>) -> PtySize {
    let cols = (size.width / DEFAULT_CELL_WIDTH).max(1);
    let rows = (size.height / DEFAULT_CELL_HEIGHT).max(1);

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

    format!(
        "Noctrail | {pane_label} | pid {pid} | {}x{} px | {}x{} cells | rows {} | {backend} | cursor {cursor}",
        frame.surface.width,
        frame.surface.height,
        frame.terminal_size.cols,
        frame.terminal_size.rows,
        frame.render_plan.rows.len(),
    )
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

    fn execute(self, app: &mut DesktopApp) -> Result<(), Box<dyn Error>> {
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
                app.resize_active_split(direction, PALETTE_RESIZE_DELTA)?;
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

struct GuiApp {
    app: DesktopApp,
    launch_options: GuiLaunchOptions,
    config_reloader: Option<ConfigReloader>,
    window: Option<Arc<Window>>,
    renderer: Option<GpuRenderer>,
    gpu_fallback_error: Option<String>,
    theme_reload_error: Option<String>,
    theme: ThemeConfig,
    font: FontConfig,
    font_preferences: FontPreferences,
    ime_preedit: Option<String>,
    command_palette: Option<CommandPalette>,
    mouse_position: Option<PhysicalPosition<f64>>,
    mouse_selection: Option<MouseSelectionDrag>,
    mouse_button: Option<input::MouseButton>,
    output_rx: Option<Receiver<OutputPumpEvent>>,
    output_thread: Option<JoinHandle<()>>,
    transition: Option<ActiveTransition>,
    next_cursor_blink_at: Instant,
    cursor_visible: bool,
    frame_interval: Duration,
    window_focused: bool,
    modifiers: ModifiersState,
    clipboard: ClipboardBridge,
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
        app.set_pane_chrome(pane_chrome_from_theme(&theme))
            .expect("app should accept pane chrome updates");
        Self {
            app,
            launch_options,
            config_reloader,
            window: None,
            renderer: None,
            gpu_fallback_error: None,
            theme_reload_error: None,
            theme: theme.clone(),
            font: font.clone(),
            font_preferences: font_preferences_from_config(&font),
            ime_preedit: None,
            command_palette: None,
            mouse_position: None,
            mouse_selection: None,
            mouse_button: None,
            output_rx: None,
            output_thread: None,
            transition: None,
            next_cursor_blink_at: now + Duration::from_millis(theme.cursor.blink_interval_ms),
            cursor_visible: true,
            frame_interval: Duration::from_millis(theme.cursor.blink_interval_ms),
            window_focused: true,
            modifiers: ModifiersState::empty(),
            clipboard: ClipboardBridge::new(),
        }
    }

    fn window_id(&self) -> Option<WindowId> {
        self.window.as_ref().map(|window| window.id())
    }

    fn attach_output_pump(&mut self) -> Result<(), Box<dyn Error>> {
        if self.output_rx.is_some() {
            return Ok(());
        }

        let Some(runtime) = self.app.pane().runtime() else {
            return Ok(());
        };
        let reader = runtime.session().clone_output_reader()?;
        let (tx, rx) = mpsc::channel();
        let handle = thread::spawn(move || pump_output(reader, tx));
        self.output_rx = Some(rx);
        self.output_thread = Some(handle);
        Ok(())
    }

    fn should_attempt_gpu_renderer(&self) -> bool {
        !self.launch_options.safe_mode && self.launch_options.renderer_backend == RenderBackend::Gpu
    }

    fn animation_duration(&self) -> Option<Duration> {
        if self.theme.animation.enabled {
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
        command.execute(&mut self.app)?;
        self.start_transition(command.transition_kind(), before);
        Ok(())
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
        if requested_opacity >= 1.0 {
            return VisualEffectsPolicy {
                requested_opacity,
                effective_opacity: 1.0,
                window_transparent: false,
                transparency_fallback_reason: None,
                blur_mode: BlurMode::Disabled,
                blur_fallback_reason: None,
            };
        }

        if self.launch_options.safe_mode {
            return VisualEffectsPolicy {
                requested_opacity,
                effective_opacity: 1.0,
                window_transparent: false,
                transparency_fallback_reason: Some("safe-mode"),
                blur_mode: if self.theme.blur.enabled {
                    BlurMode::TintedSolid
                } else {
                    BlurMode::Disabled
                },
                blur_fallback_reason: if self.theme.blur.enabled {
                    Some("safe-mode")
                } else {
                    None
                },
            };
        }

        if self.app.backend() != RenderBackend::Gpu {
            return VisualEffectsPolicy {
                requested_opacity,
                effective_opacity: 1.0,
                window_transparent: false,
                transparency_fallback_reason: Some("software-backend"),
                blur_mode: if self.theme.blur.enabled {
                    BlurMode::TintedSolid
                } else {
                    BlurMode::Disabled
                },
                blur_fallback_reason: if self.theme.blur.enabled {
                    Some("software-backend")
                } else {
                    None
                },
            };
        }

        if self.theme.blur.enabled {
            return VisualEffectsPolicy {
                requested_opacity,
                effective_opacity: self.theme.blur.fallback_tint_opacity.max(requested_opacity),
                window_transparent: false,
                transparency_fallback_reason: None,
                blur_mode: BlurMode::TintedSolid,
                blur_fallback_reason: Some("unsupported-platform"),
            };
        }

        VisualEffectsPolicy {
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
        let attributes = Window::default_attributes()
            .with_title("Noctrail")
            .with_inner_size(LogicalSize::new(
                f64::from(DEFAULT_WINDOW_WIDTH),
                f64::from(DEFAULT_WINDOW_HEIGHT),
            ))
            .with_resizable(true)
            .with_transparent(requested_transparency);
        let window = Arc::new(event_loop.create_window(attributes)?);
        let size = window.inner_size();
        self.sync_surface(size)?;
        if self.launch_options.safe_mode {
            self.renderer = None;
            self.gpu_fallback_error = Some("safe-mode".to_string());
            self.app.set_backend(RenderBackend::Software);
        } else if self.should_attempt_gpu_renderer() {
            match GpuRenderer::new(window.clone(), size) {
                Ok(renderer) => {
                    self.renderer = Some(renderer);
                    self.gpu_fallback_error = None;
                    self.app.set_backend(RenderBackend::Gpu);
                    self.apply_theme_visuals();
                }
                Err(error) => {
                    self.record_gpu_fallback(error.to_string());
                }
            }
        } else {
            self.renderer = None;
            self.gpu_fallback_error = None;
            self.app.set_backend(RenderBackend::Software);
        }
        self.window = Some(window);
        self.apply_theme_visuals();
        self.update_title();
        self.request_redraw();
        Ok(())
    }

    fn sync_surface(&mut self, size: PhysicalSize<u32>) -> Result<(), Box<dyn Error>> {
        let surface = layout_rect_from_surface(size);
        let terminal_size = terminal_size_from_surface(size);
        self.app.resize(surface, terminal_size)?;
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.resize(size);
        }
        Ok(())
    }

    fn update_title(&self) {
        if let Some(window) = self.window.as_ref() {
            let mut title = frame_title(&self.app.frame(), self.cursor_visible);
            let effects = self.visual_effects_policy();
            title.push_str(" | font ");
            title.push_str(&self.font.family);
            title.push(' ');
            title.push_str(&format!("{:.1}", self.font.size));
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
            window.set_title(&title);
        }
    }

    fn apply_theme_visuals(&mut self) {
        self.frame_interval = Duration::from_millis(self.theme.cursor.blink_interval_ms);
        self.app.invalidate_visuals();
        if !self.theme.animation.enabled {
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
        eprintln!("GPU renderer unavailable, falling back to software: {error}");
        self.renderer = None;
        self.gpu_fallback_error = Some(error);
        self.app
            .set_backend(noctrail_render::RenderBackend::Software);
    }

    fn drain_output_events(&mut self) -> bool {
        let Some(rx) = self.output_rx.as_ref() else {
            return false;
        };

        let mut received_output = false;
        loop {
            match rx.try_recv() {
                Ok(OutputPumpEvent::Bytes(bytes)) => {
                    self.app.advance_output(&bytes);
                    received_output = true;
                }
                Ok(OutputPumpEvent::Error(error)) => {
                    eprintln!("PTY output pump failed: {error}");
                    break;
                }
                Ok(OutputPumpEvent::Eof) => break,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }

        if received_output {
            self.touch_cursor_blink();
            self.update_title();
            self.request_redraw();
        }

        received_output
    }

    fn handle_ime_event(&mut self, ime: Ime) -> Result<(), Box<dyn Error>> {
        match ime {
            Ime::Enabled | Ime::Disabled => Ok(()),
            Ime::Preedit(text, _cursor) => {
                self.ime_preedit = if text.is_empty() { None } else { Some(text) };
                self.touch_cursor_blink();
                self.update_title();
                self.request_redraw();
                Ok(())
            }
            Ime::Commit(text) => {
                self.ime_preedit = None;
                if !text.is_empty() {
                    self.app.write_input(text.as_bytes())?;
                    self.touch_cursor_blink();
                    self.request_redraw();
                }
                self.update_title();
                Ok(())
            }
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

        let terminal_size = self.app.pane().terminal_size();
        let col = (logical.x / f64::from(DEFAULT_CELL_WIDTH)).floor() as usize;
        let row = (logical.y / f64::from(DEFAULT_CELL_HEIGHT)).floor() as usize;
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
            self.update_title();
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
                if let Some(cell) = cell {
                    self.mouse_selection = Some(MouseSelectionDrag {
                        anchor: cell,
                        cursor: cell,
                    });
                    self.app
                        .select_viewport_range(cell, cell, SelectionMode::Normal);
                    self.touch_cursor_blink();
                    self.request_redraw();
                    self.update_title();
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
                    .set_pane_chrome(pane_chrome_from_theme(&self.theme))
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
}

fn direction_name(direction: FocusDirection) -> &'static str {
    match direction {
        FocusDirection::Left => "left",
        FocusDirection::Right => "right",
        FocusDirection::Up => "up",
        FocusDirection::Down => "down",
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

fn pane_chrome_from_theme(theme: &ThemeConfig) -> PaneChromeConfig {
    PaneChromeConfig {
        border: PaneBorderStyle {
            width: usize::from(theme.border.width),
            active: rgba_from_config(theme.border.active),
            inactive: rgba_from_config(theme.border.inactive),
        },
        gap: theme.pane.gap,
        padding: theme.pane.padding,
        radius: theme.pane.radius,
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

enum OutputPumpEvent {
    Bytes(Vec<u8>),
    Eof,
    Error(String),
}

fn pump_output(mut reader: PtyOutputReader, tx: mpsc::Sender<OutputPumpEvent>) {
    let mut chunk = [0_u8; 4096];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => {
                let _ = tx.send(OutputPumpEvent::Eof);
                break;
            }
            Ok(count) => {
                if tx
                    .send(OutputPumpEvent::Bytes(chunk[..count].to_vec()))
                    .is_err()
                {
                    break;
                }
            }
            Err(error) => {
                let _ = tx.send(OutputPumpEvent::Error(error.to_string()));
                break;
            }
        }
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
            let lines = delta.y / f64::from(DEFAULT_CELL_HEIGHT);
            if lines.abs() >= 1.0 {
                lines.round() as i32
            } else {
                delta.y.signum() as i32
            }
        }
    }
}

impl ApplicationHandler for GuiApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
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
            WindowEvent::Ime(ime) => {
                if self.handle_ime_event(ime).is_err() {
                    event_loop.exit();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if self.handle_cursor_moved(position).is_err() {
                    event_loop.exit();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if self.handle_mouse_input(state, button).is_err() {
                    event_loop.exit();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if self.handle_mouse_wheel(delta).is_err() {
                    event_loop.exit();
                }
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                if is_synthetic {
                    return;
                }
                if matches!(
                    input::shortcut_action(&event.logical_key, self.modifiers),
                    Some(input::ShortcutAction::ToggleCommandPalette)
                ) {
                    self.toggle_command_palette();
                    return;
                }
                match self.handle_command_palette_key(&event) {
                    Ok(true) => return,
                    Ok(false) => {}
                    Err(_) => {
                        event_loop.exit();
                        return;
                    }
                }
                if let Some(action) = input::shortcut_action(&event.logical_key, self.modifiers) {
                    match action {
                        input::ShortcutAction::ToggleCommandPalette => unreachable!(),
                        input::ShortcutAction::Copy => {
                            if let Some(text) = self.app.copy_selection_text() {
                                self.clipboard.set_text(text);
                            }
                        }
                        input::ShortcutAction::Paste => {
                            if let Some(text) = self.clipboard.get_text() {
                                if self.app.paste_text(&text).is_err() {
                                    event_loop.exit();
                                    return;
                                }
                                self.touch_cursor_blink();
                                self.request_redraw();
                                self.update_title();
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
                if let Some(bytes) = input::key_event_to_pty_bytes(
                    event.state,
                    &event.logical_key,
                    event.text.as_deref(),
                    self.modifiers,
                ) {
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
                if self.sync_surface(size).is_err() {
                    event_loop.exit();
                    return;
                }
                self.touch_cursor_blink();
                self.update_title();
                self.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                if let Some(renderer) = self.renderer.as_mut()
                    && let Err(error) = renderer.render_clear()
                {
                    self.record_gpu_fallback(error.to_string());
                    self.update_title();
                }
                self.update_title();
            }
            WindowEvent::Focused(true) => {
                self.window_focused = true;
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
        if self.advance_transition(Instant::now()) {
            self.update_title();
            self.request_redraw();
        }
        if self.advance_cursor_blink(Instant::now()) {
            self.update_title();
            self.request_redraw();
        }
        self.reschedule(event_loop);
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        let _ = self.app.close_runtime();
        self.output_rx.take();
        if let Some(handle) = self.output_thread.take() {
            let _ = handle.join();
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
        assert_eq!(
            terminal_size_from_surface(PhysicalSize::new(7, 15)),
            PtySize::new(1, 1)
        );
        assert_eq!(
            terminal_size_from_surface(PhysicalSize::new(320, 160)),
            PtySize::new(40, 10)
        );
    }

    #[test]
    fn frame_title_reflects_state() {
        let frame = DesktopFrame {
            workspace_id: WorkspaceId::new(1),
            is_scratch: false,
            pane_id: PaneId::new(7),
            pane_surface: LayoutRect::new(0, 0, 120, 80),
            surface: LayoutRect::new(0, 0, 120, 80),
            terminal_size: PtySize::new(80, 24),
            process_id: Some(1234),
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
                config_path: None,
                theme: ThemeConfig::default(),
                font: FontConfig::default(),
            },
        );

        assert!(!gui.should_attempt_gpu_renderer());
    }

    #[test]
    fn software_backend_launch_options_skip_gpu_attempts() {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(10, 3));
        let gui = GuiApp::new(
            app,
            GuiLaunchOptions {
                safe_mode: false,
                renderer_backend: RenderBackend::Software,
                config_path: None,
                theme: ThemeConfig::default(),
                font: FontConfig::default(),
            },
        );

        assert!(!gui.should_attempt_gpu_renderer());
    }

    #[test]
    fn config_reload_updates_theme_font_and_cursor_timing() {
        let path = temp_config_path("theme-reload");
        fs::write(
            &path,
            "[font]\nfamily = \"JetBrainsMono Nerd Font\"\nsize = 14.0\n\n[theme]\nopacity = 1.0\n\n[theme.pane]\ngap = 8\npadding = 6\nradius = 8\n\n[theme.animation]\nenabled = true\nduration-ms = 120\n\n[theme.cursor]\nblink-interval-ms = 600\n",
        )
        .expect("write initial config");

        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(10, 3));
        let mut gui = GuiApp::new(
            app,
            GuiLaunchOptions {
                safe_mode: false,
                renderer_backend: RenderBackend::Software,
                config_path: Some(path.clone()),
                theme: ThemeConfig::default(),
                font: FontConfig::default(),
            },
        );

        fs::write(
            &path,
            "[font]\nfamily = \"Iosevka\"\nsize = 16.0\nfallback = [\"Noto Sans CJK SC\"]\n\n[theme]\nopacity = 0.75\n\n[theme.pane]\ngap = 10\npadding = 4\nradius = 12\n\n[theme.animation]\nenabled = false\nduration-ms = 200\n\n[theme.cursor]\nblink-interval-ms = 250\n",
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
        assert!(!gui.theme.animation.enabled);
        assert_eq!(gui.theme.animation.duration_ms, 200);
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

        PaletteCommand::SplitHorizontal.execute(&mut gui.app)?;

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

        PaletteCommand::Workspace(WorkspaceId::new(2)).execute(&mut gui.app)?;
        assert_eq!(gui.app.active_workspace_id(), WorkspaceId::new(2));

        PaletteCommand::ToggleScratch.execute(&mut gui.app)?;
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

        gui.handle_ime_event(Ime::Preedit("zhong".to_string(), None))?;
        assert_eq!(gui.ime_preedit.as_deref(), Some("zhong"));

        gui.handle_ime_event(Ime::Preedit(String::new(), None))?;
        assert!(gui.ime_preedit.is_none());
        Ok(())
    }

    #[test]
    fn mouse_drag_updates_selection() -> Result<(), Box<dyn Error>> {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(8, 2));
        let mut gui = GuiApp::new(app, GuiLaunchOptions::default());
        gui.app.advance_output(b"hello");

        gui.handle_cursor_moved(PhysicalPosition::new(1.0, 1.0))?;
        gui.handle_mouse_input(ElementState::Pressed, WinitMouseButton::Left)?;
        gui.handle_cursor_moved(PhysicalPosition::new(25.0, 1.0))?;
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
        gui.output_rx.take();
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
    fn ime_commit_writes_text_to_shell() -> Result<(), Box<dyn Error>> {
        let app = DesktopApp::spawn_shell(LayoutRect::new(0, 0, 120, 80), PtySize::new(80, 24))?;
        let mut gui = GuiApp::new(app, GuiLaunchOptions::default());
        gui.attach_output_pump()?;

        gui.handle_ime_event(Ime::Commit("NOCTRAIL_IME".to_string()))?;
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
                if text.contains("NOCTRAIL_IME") {
                    observed = true;
                    break;
                }
            }
            thread::sleep(Duration::from_millis(20));
        }

        gui.app.write_input(shell_exit_bytes().as_slice())?;
        let _ = gui.app.close_runtime()?;
        gui.output_rx.take();
        if let Some(handle) = gui.output_thread.take() {
            let _ = handle.join();
        }

        assert!(observed, "ime commit did not reach the shell");
        Ok(())
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

        gui.handle_cursor_moved(PhysicalPosition::new(9.0, 17.0))?;
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

    fn shell_command_bytes(marker: &str) -> Vec<u8> {
        shell_command_text(marker).into_bytes()
    }

    fn shell_exit_bytes() -> Vec<u8> {
        b"exit\r\n".to_vec()
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
