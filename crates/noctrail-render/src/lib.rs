//! Render plan and backend boundary for Noctrail.

use std::sync::Arc;

use noctrail_term::{
    Cell, Cursor, DamageSet, ScreenRowSnapshot, Selection, Style, TerminalSnapshot,
};
use thiserror::Error;
use wgpu::CurrentSurfaceTexture;
use winit::{dpi::PhysicalSize, window::Window};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderBackend {
    Gpu,
    #[default]
    Software,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RenderRect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl RenderRect {
    pub const fn new(x: usize, y: usize, width: usize, height: usize) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderGlyph {
    pub col: usize,
    pub text: String,
    pub style: Style,
    pub span: usize,
    pub wide_continuation: bool,
}

impl RenderGlyph {
    fn from_cell(col: usize, cells: &[Cell]) -> Self {
        let cell = &cells[col];
        let wide_continuation = cell.wide_continuation;
        let span = if wide_continuation {
            0
        } else if col + 1 < cells.len() && cells[col + 1].wide_continuation {
            2
        } else {
            1
        };

        Self {
            col,
            text: cell.text.clone(),
            style: cell.style,
            span,
            wide_continuation,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderRow {
    pub row: usize,
    pub wrapped: bool,
    pub glyphs: Vec<RenderGlyph>,
}

impl RenderRow {
    fn from_snapshot(row: usize, snapshot: &ScreenRowSnapshot) -> Self {
        let glyphs = snapshot
            .cells
            .iter()
            .enumerate()
            .map(|(col, _)| RenderGlyph::from_cell(col, &snapshot.cells))
            .collect();

        Self {
            row,
            wrapped: snapshot.wrapped,
            glyphs,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RenderInput<'a> {
    pub viewport: RenderRect,
    pub backend: RenderBackend,
    pub snapshot: &'a TerminalSnapshot,
    pub damage: &'a DamageSet,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RenderPlan {
    pub backend: RenderBackend,
    pub viewport: RenderRect,
    pub damage: DamageSet,
    pub scrollback_rows: usize,
    pub cursor: Cursor,
    pub alternate_screen: bool,
    pub selection: Option<Selection>,
    pub active: bool,
    pub rows: Vec<RenderRow>,
}

impl RenderPlan {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_terminal(
        viewport: RenderRect,
        backend: RenderBackend,
        snapshot: &TerminalSnapshot,
    ) -> Self {
        Self::from_input(RenderInput {
            viewport,
            backend,
            snapshot,
            damage: &DamageSet {
                dirty_rows: (0..snapshot.rows.len()).collect(),
                full_frame: true,
            },
            active: true,
        })
    }

    pub fn from_input(input: RenderInput<'_>) -> Self {
        Self {
            backend: input.backend,
            viewport: input.viewport,
            damage: input.damage.clone(),
            scrollback_rows: input.snapshot.scrollback.len(),
            cursor: input.snapshot.cursor,
            alternate_screen: input.snapshot.alternate_screen,
            selection: input.snapshot.selection.clone(),
            active: input.active,
            rows: input
                .snapshot
                .rows
                .iter()
                .enumerate()
                .map(|(row, snapshot)| RenderRow::from_snapshot(row, snapshot))
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty() && self.scrollback_rows == 0
    }
}

#[derive(Debug, Default)]
pub struct RenderSurface;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuDiagnostics {
    pub adapter_name: String,
    pub backend: wgpu::Backend,
    pub device_type: wgpu::DeviceType,
    pub surface_format: wgpu::TextureFormat,
    pub present_mode: wgpu::PresentMode,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderOutcome {
    Presented,
    Skipped,
}

#[derive(Debug, Error)]
pub enum GpuRendererError {
    #[error("failed to create GPU surface: {0}")]
    CreateSurface(#[source] wgpu::CreateSurfaceError),
    #[error("failed to request GPU adapter: {0}")]
    RequestAdapter(#[source] wgpu::RequestAdapterError),
    #[error("surface does not expose a default configuration")]
    MissingSurfaceConfiguration,
    #[error("failed to request GPU device: {0}")]
    RequestDevice(#[source] wgpu::RequestDeviceError),
    #[error("surface validation failed while acquiring the next frame")]
    SurfaceValidation,
}

pub struct GpuRenderer {
    instance: wgpu::Instance,
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_config: wgpu::SurfaceConfiguration,
    clear_color: wgpu::Color,
    diagnostics: GpuDiagnostics,
}

impl std::fmt::Debug for GpuRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuRenderer")
            .field("surface_config", &self.surface_config)
            .field("diagnostics", &self.diagnostics)
            .finish()
    }
}

impl GpuRenderer {
    pub fn new(window: Arc<Window>, size: PhysicalSize<u32>) -> Result<Self, GpuRendererError> {
        pollster::block_on(Self::new_async(window, size))
    }

    async fn new_async(
        window: Arc<Window>,
        size: PhysicalSize<u32>,
    ) -> Result<Self, GpuRendererError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let surface = instance
            .create_surface(window.clone())
            .map_err(GpuRendererError::CreateSurface)?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .map_err(GpuRendererError::RequestAdapter)?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .map_err(GpuRendererError::RequestDevice)?;
        let mut surface_config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or(GpuRendererError::MissingSurfaceConfiguration)?;
        surface_config.width = size.width.max(1);
        surface_config.height = size.height.max(1);
        surface.configure(&device, &surface_config);

        let adapter_info = adapter.get_info();
        let diagnostics = GpuDiagnostics {
            adapter_name: adapter_info.name,
            backend: adapter_info.backend,
            device_type: adapter_info.device_type,
            surface_format: surface_config.format,
            present_mode: surface_config.present_mode,
            width: surface_config.width,
            height: surface_config.height,
        };

        Ok(Self {
            instance,
            window,
            surface,
            adapter,
            device,
            queue,
            surface_config,
            clear_color: wgpu::Color {
                r: 0.02,
                g: 0.04,
                b: 0.06,
                a: 1.0,
            },
            diagnostics,
        })
    }

    pub fn diagnostics(&self) -> &GpuDiagnostics {
        &self.diagnostics
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        self.surface_config.width = size.width.max(1);
        self.surface_config.height = size.height.max(1);
        self.surface.configure(&self.device, &self.surface_config);
        self.diagnostics.width = self.surface_config.width;
        self.diagnostics.height = self.surface_config.height;
    }

    pub fn render_clear(&mut self) -> Result<RenderOutcome, GpuRendererError> {
        let (frame, reconfigure_after_present) = match self.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(frame) => (frame, false),
            CurrentSurfaceTexture::Suboptimal(frame) => (frame, true),
            CurrentSurfaceTexture::Timeout | CurrentSurfaceTexture::Occluded => {
                return Ok(RenderOutcome::Skipped);
            }
            CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.surface_config);
                return Ok(RenderOutcome::Skipped);
            }
            CurrentSurfaceTexture::Lost => {
                self.recreate_surface()?;
                return Ok(RenderOutcome::Skipped);
            }
            CurrentSurfaceTexture::Validation => return Err(GpuRendererError::SurfaceValidation),
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("noctrail-clear-frame"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("noctrail-clear-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.clear_color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        self.queue.submit([encoder.finish()]);
        frame.present();

        if reconfigure_after_present {
            self.surface.configure(&self.device, &self.surface_config);
        }

        Ok(RenderOutcome::Presented)
    }

    fn recreate_surface(&mut self) -> Result<(), GpuRendererError> {
        self.surface = self
            .instance
            .create_surface(self.window.clone())
            .map_err(GpuRendererError::CreateSurface)?;
        self.surface.configure(&self.device, &self.surface_config);
        self.adapter =
            pollster::block_on(self.instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&self.surface),
                force_fallback_adapter: false,
            }))
            .map_err(GpuRendererError::RequestAdapter)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use noctrail_term::{Color, ScreenRowSnapshot, SelectionMode};

    fn cell(text: &str) -> Cell {
        Cell {
            text: text.to_owned(),
            style: Style {
                foreground: Color::Indexed(7),
                background: Color::Indexed(0),
                bold: false,
                italic: false,
                underline: false,
            },
            wide_continuation: false,
        }
    }

    fn wide_continuation_cell() -> Cell {
        Cell {
            text: String::new(),
            style: Style {
                foreground: Color::Indexed(7),
                background: Color::Indexed(0),
                bold: false,
                italic: false,
                underline: false,
            },
            wide_continuation: true,
        }
    }

    #[test]
    fn from_terminal_copies_snapshot_metadata() {
        let snapshot = TerminalSnapshot {
            rows: vec![ScreenRowSnapshot {
                cells: vec![cell("h"), cell("i")],
                wrapped: false,
            }],
            scrollback: vec![ScreenRowSnapshot {
                cells: vec![cell("o"), cell("l")],
                wrapped: true,
            }],
            cursor: Cursor { row: 1, col: 2 },
            alternate_screen: true,
            bracketed_paste: false,
            selection: Some(Selection {
                mode: SelectionMode::Line,
                start: noctrail_term::Position { row: 0, col: 0 },
                end: noctrail_term::Position { row: 0, col: 1 },
            }),
        };

        let plan =
            RenderPlan::from_terminal(RenderRect::new(1, 2, 80, 24), RenderBackend::Gpu, &snapshot);

        assert_eq!(plan.backend, RenderBackend::Gpu);
        assert_eq!(plan.viewport, RenderRect::new(1, 2, 80, 24));
        assert!(plan.damage.full_frame);
        assert_eq!(plan.damage.dirty_rows, vec![0]);
        assert_eq!(plan.scrollback_rows, 1);
        assert_eq!(plan.cursor, snapshot.cursor);
        assert!(plan.alternate_screen);
        assert_eq!(plan.selection, snapshot.selection);
        assert!(plan.active);
        assert_eq!(plan.rows.len(), 1);
        assert_eq!(plan.rows[0].row, 0);
        assert!(!plan.rows[0].wrapped);
    }

    #[test]
    fn glyph_path_marks_wide_cells() {
        let snapshot = TerminalSnapshot {
            rows: vec![ScreenRowSnapshot {
                cells: vec![cell("字"), wide_continuation_cell(), cell("x")],
                wrapped: false,
            }],
            ..TerminalSnapshot::default()
        };

        let plan = RenderPlan::from_terminal(
            RenderRect::new(0, 0, 3, 1),
            RenderBackend::Software,
            &snapshot,
        );

        let glyphs = &plan.rows[0].glyphs;
        assert_eq!(glyphs.len(), 3);
        assert_eq!(glyphs[0].text, "字");
        assert_eq!(glyphs[0].span, 2);
        assert!(!glyphs[0].wide_continuation);
        assert_eq!(glyphs[1].text, "");
        assert_eq!(glyphs[1].span, 0);
        assert!(glyphs[1].wide_continuation);
        assert_eq!(glyphs[2].text, "x");
        assert_eq!(glyphs[2].span, 1);
    }

    #[test]
    fn glyph_path_preserves_combining_marks() {
        let snapshot = TerminalSnapshot {
            rows: vec![ScreenRowSnapshot {
                cells: vec![cell("e\u{301}"), cell(" ")],
                wrapped: false,
            }],
            ..TerminalSnapshot::default()
        };

        let plan = RenderPlan::from_terminal(
            RenderRect::new(0, 0, 2, 1),
            RenderBackend::Software,
            &snapshot,
        );

        assert_eq!(plan.rows[0].glyphs[0].text, "e\u{301}");
        assert_eq!(plan.rows[0].glyphs[0].span, 1);
    }

    #[test]
    fn from_input_preserves_damage_metadata() {
        let snapshot = TerminalSnapshot {
            rows: vec![ScreenRowSnapshot {
                cells: vec![cell("a"), cell("b"), cell("c")],
                wrapped: false,
            }],
            ..TerminalSnapshot::default()
        };
        let damage = DamageSet {
            dirty_rows: vec![0],
            full_frame: false,
        };

        let plan = RenderPlan::from_input(RenderInput {
            viewport: RenderRect::new(4, 5, 6, 7),
            backend: RenderBackend::Software,
            snapshot: &snapshot,
            damage: &damage,
            active: false,
        });

        assert_eq!(plan.viewport, RenderRect::new(4, 5, 6, 7));
        assert_eq!(plan.damage, damage);
        assert!(!plan.active);
        assert_eq!(plan.rows[0].glyphs.len(), 3);
    }

    #[test]
    fn gpu_diagnostics_track_surface_size() {
        let mut diagnostics = GpuDiagnostics {
            adapter_name: "adapter".to_string(),
            backend: wgpu::Backend::Metal,
            device_type: wgpu::DeviceType::IntegratedGpu,
            surface_format: wgpu::TextureFormat::Bgra8UnormSrgb,
            present_mode: wgpu::PresentMode::AutoVsync,
            width: 80,
            height: 24,
        };

        diagnostics.width = 100;
        diagnostics.height = 30;

        assert_eq!(diagnostics.width, 100);
        assert_eq!(diagnostics.height, 30);
    }
}
