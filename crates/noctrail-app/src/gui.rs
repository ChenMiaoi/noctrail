use std::{
    error::Error,
    time::{Duration, Instant},
};

use noctrail_layout::LayoutRect;
use noctrail_pty::PtySize;
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalSize},
    event::WindowEvent,
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
    window: Option<Window>,
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
            next_frame_at: now,
            cursor_visible: true,
            frame_interval: FRAME_INTERVAL,
            modifiers: ModifiersState::empty(),
            clipboard: ClipboardBridge::new(),
        }
    }

    fn window_id(&self) -> Option<WindowId> {
        self.window.as_ref().map(Window::id)
    }

    fn create_window(&mut self, event_loop: &ActiveEventLoop) -> Result<(), Box<dyn Error>> {
        let attributes = Window::default_attributes()
            .with_title("Noctrail")
            .with_inner_size(LogicalSize::new(
                f64::from(DEFAULT_WINDOW_WIDTH),
                f64::from(DEFAULT_WINDOW_HEIGHT),
            ))
            .with_resizable(true);
        let window = event_loop.create_window(attributes)?;
        let size = window.inner_size();
        self.sync_surface(size)?;
        self.window = Some(window);
        self.update_title();
        self.request_redraw();
        Ok(())
    }

    fn sync_surface(&mut self, size: PhysicalSize<u32>) -> Result<(), Box<dyn Error>> {
        let surface = layout_rect_from_surface(size);
        let terminal_size = terminal_size_from_surface(size);
        self.app.resize(surface, terminal_size)?;
        Ok(())
    }

    fn update_title(&self) {
        if let Some(window) = self.window.as_ref() {
            window.set_title(&frame_title(&self.app.frame(), self.cursor_visible));
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
}

impl ApplicationHandler for GuiApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() && self.create_window(event_loop).is_err() {
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

        self.reschedule(event_loop);
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        let _ = self.app.close_runtime();
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
                scrollback_rows: 9,
                cursor: noctrail_term::Cursor { row: 1, col: 2 },
                alternate_screen: false,
                selection: None,
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
}
