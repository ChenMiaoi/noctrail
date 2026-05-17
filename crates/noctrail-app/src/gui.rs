use std::{
    error::Error,
    io::Read,
    sync::Arc,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use noctrail_layout::LayoutRect;
use noctrail_pty::{PtyOutputReader, PtySize};
use noctrail_render::GpuRenderer;
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalSize},
    event::{Ime, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::ModifiersState,
    window::{Window, WindowId},
};

use crate::{DesktopApp, DesktopFrame, clipboard::ClipboardBridge, input};

const DEFAULT_WINDOW_WIDTH: u32 = 1280;
const DEFAULT_WINDOW_HEIGHT: u32 = 800;
const DEFAULT_CELL_WIDTH: u32 = 8;
const DEFAULT_CELL_HEIGHT: u32 = 16;
const FRAME_INTERVAL: Duration = Duration::from_millis(250);

pub fn run() -> Result<(), Box<dyn Error>> {
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
    let mut gui = GuiApp::new(app);
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

    format!(
        "Noctrail | pane {} | pid {pid} | {}x{} px | {}x{} cells | rows {} | {backend} | cursor {cursor}",
        frame.pane_id.0,
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

struct GuiApp {
    app: DesktopApp,
    window: Option<Arc<Window>>,
    renderer: Option<GpuRenderer>,
    gpu_fallback_error: Option<String>,
    ime_preedit: Option<String>,
    output_rx: Option<Receiver<OutputPumpEvent>>,
    output_thread: Option<JoinHandle<()>>,
    next_frame_at: Instant,
    cursor_visible: bool,
    frame_interval: Duration,
    modifiers: ModifiersState,
    clipboard: ClipboardBridge,
}

impl GuiApp {
    fn new(app: DesktopApp) -> Self {
        let now = Instant::now();
        Self {
            app,
            window: None,
            renderer: None,
            gpu_fallback_error: None,
            ime_preedit: None,
            output_rx: None,
            output_thread: None,
            next_frame_at: now,
            cursor_visible: true,
            frame_interval: FRAME_INTERVAL,
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

    fn create_window(&mut self, event_loop: &ActiveEventLoop) -> Result<(), Box<dyn Error>> {
        let attributes = Window::default_attributes()
            .with_title("Noctrail")
            .with_inner_size(LogicalSize::new(
                f64::from(DEFAULT_WINDOW_WIDTH),
                f64::from(DEFAULT_WINDOW_HEIGHT),
            ))
            .with_resizable(true);
        let window = Arc::new(event_loop.create_window(attributes)?);
        let size = window.inner_size();
        self.sync_surface(size)?;
        match GpuRenderer::new(window.clone(), size) {
            Ok(renderer) => {
                self.renderer = Some(renderer);
                self.gpu_fallback_error = None;
                self.app.set_backend(noctrail_render::RenderBackend::Gpu);
            }
            Err(error) => {
                self.record_gpu_fallback(error.to_string());
            }
        }
        self.window = Some(window);
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
            if let Some(error) = self.gpu_fallback_error.as_deref() {
                title.push_str(" | gpu-fallback ");
                title.push_str(error);
            }
            if let Some(preedit) = self.ime_preedit.as_deref()
                && !preedit.is_empty()
            {
                title.push_str(" | ime ");
                title.push_str(preedit);
            }
            window.set_title(&title);
        }
    }

    fn request_redraw(&self) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    fn reschedule(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if now >= self.next_frame_at {
            self.request_redraw();
            self.next_frame_at = now + self.frame_interval;
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame_at));
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
                self.update_title();
                self.request_redraw();
                Ok(())
            }
            Ime::Commit(text) => {
                self.ime_preedit = None;
                if !text.is_empty() {
                    self.app.write_input(text.as_bytes())?;
                    self.request_redraw();
                }
                self.update_title();
                Ok(())
            }
        }
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

        self.next_frame_at = Instant::now() + self.frame_interval;
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
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                if is_synthetic {
                    return;
                }
                if let Some(action) = input::shortcut_action(&event.logical_key, self.modifiers) {
                    match action {
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
                                self.request_redraw();
                                self.update_title();
                            }
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
                    self.request_redraw();
                    self.update_title();
                }
            }
            WindowEvent::Resized(size) => {
                if self.sync_surface(size).is_err() {
                    event_loop.exit();
                    return;
                }
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
                self.cursor_visible = !self.cursor_visible;
                self.update_title();
                self.next_frame_at = Instant::now() + self.frame_interval;
            }
            WindowEvent::Focused(true) => {
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

        let _ = self.drain_output_events();
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
    use noctrail_render::{RenderBackend, RenderPlan, RenderRect};
    use noctrail_runtime::PaneId;

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
            pane_id: PaneId::new(7),
            surface: LayoutRect::new(0, 0, 120, 80),
            terminal_size: PtySize::new(80, 24),
            process_id: Some(1234),
            render_plan: RenderPlan {
                backend: RenderBackend::Gpu,
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
        let mut gui = GuiApp::new(app);
        gui.app.set_backend(RenderBackend::Gpu);

        gui.record_gpu_fallback("adapter missing".to_string());

        assert_eq!(gui.app.backend(), RenderBackend::Software);
        assert!(gui.renderer.is_none());
        assert_eq!(gui.gpu_fallback_error.as_deref(), Some("adapter missing"));
    }

    #[test]
    fn ime_preedit_updates_gui_state() -> Result<(), Box<dyn Error>> {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(10, 3));
        let mut gui = GuiApp::new(app);

        gui.handle_ime_event(Ime::Preedit("zhong".to_string(), None))?;
        assert_eq!(gui.ime_preedit.as_deref(), Some("zhong"));

        gui.handle_ime_event(Ime::Preedit(String::new(), None))?;
        assert!(gui.ime_preedit.is_none());
        Ok(())
    }

    #[test]
    fn output_pump_feeds_shell_output_into_render_plan() -> Result<(), Box<dyn Error>> {
        let app = DesktopApp::spawn_shell(LayoutRect::new(0, 0, 120, 80), PtySize::new(80, 24))?;
        let mut gui = GuiApp::new(app);
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
        let mut gui = GuiApp::new(app);
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
}
