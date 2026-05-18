//! Render plan and backend boundary for Noctrail.

use std::{cell::RefCell, collections::BTreeSet, sync::Arc};

use cosmic_text::{
    Attrs as FontAttrs, Buffer as FontBuffer, CacheKey, Family as FontFamily, FontSystem,
    LayoutGlyph, Metrics, Shaping, SwashCache, SwashContent,
    fontdb::{self, Query},
};
use noctrail_term::{
    Cell, Color, Cursor, DamageSet, ScreenRowSnapshot, Selection, Style, TerminalSnapshot,
};
use thiserror::Error;
use wgpu::CurrentSurfaceTexture;
use winit::{dpi::PhysicalSize, window::Window};

thread_local! {
    static SOFTWARE_FONT_SYSTEM: RefCell<FontSystem> = RefCell::new(font::configured_font_system());
    static SOFTWARE_SWASH_CACHE: RefCell<SwashCache> = RefCell::new(SwashCache::new());
}

mod font;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rgba {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

impl Rgba {
    pub const fn opaque(red: u8, green: u8, blue: u8) -> Self {
        Self {
            red,
            green,
            blue,
            alpha: u8::MAX,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PaneBorderStyle {
    pub width: usize,
    pub active: Rgba,
    pub inactive: Rgba,
}

impl PaneBorderStyle {
    pub fn color_for(self, active: bool) -> Rgba {
        if active { self.active } else { self.inactive }
    }

    pub fn enabled(self) -> bool {
        self.width > 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromeLayer {
    PaneBackground,
    StatusBackground,
    StatusIndicator,
    StatusSeparator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChromeRect {
    pub layer: ChromeLayer,
    pub rect: RenderRect,
    pub color: Rgba,
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
    pub pane_rect: RenderRect,
    pub viewport: RenderRect,
    pub backend: RenderBackend,
    pub snapshot: &'a TerminalSnapshot,
    pub damage: &'a DamageSet,
    pub chrome: &'a [ChromeRect],
    pub active: bool,
    pub border: PaneBorderStyle,
    pub corner_radius: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RenderPlan {
    pub backend: RenderBackend,
    pub pane_rect: RenderRect,
    pub viewport: RenderRect,
    pub damage: DamageSet,
    pub scrollback_rows: usize,
    pub cursor: Cursor,
    pub alternate_screen: bool,
    pub selection: Option<Selection>,
    pub chrome: Vec<ChromeRect>,
    pub active: bool,
    pub border: PaneBorderStyle,
    pub corner_radius: usize,
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
            pane_rect: viewport,
            viewport,
            backend,
            snapshot,
            damage: &DamageSet {
                dirty_rows: (0..snapshot.rows.len()).collect(),
                full_frame: true,
            },
            chrome: &[],
            active: true,
            border: PaneBorderStyle::default(),
            corner_radius: 0,
        })
    }

    pub fn from_input(input: RenderInput<'_>) -> Self {
        Self {
            backend: input.backend,
            pane_rect: input.pane_rect,
            viewport: input.viewport,
            damage: input.damage.clone(),
            scrollback_rows: input.snapshot.scrollback.len(),
            cursor: input.snapshot.cursor,
            alternate_screen: input.snapshot.alternate_screen,
            selection: input.snapshot.selection.clone(),
            chrome: input.chrome.to_vec(),
            active: input.active,
            border: input.border,
            corner_radius: input.corner_radius,
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

const DEFAULT_FONT_SIZE: f32 = 15.0;
const DEFAULT_FONT_FAMILY: &str = "CaskaydiaMono NFM";
const FONT_SAMPLE_ASCII: &str = "abcXYZ 0123";
const FONT_SAMPLE_CJK: &str = "你好，世界";
const FONT_SAMPLE_EMOJI: &str = "🙂🧪🚀";
const FONT_SAMPLE_NERD: &str = "\u{e0b0} \u{f417} \u{f120}";

#[derive(Debug, Clone, PartialEq)]
pub struct FontPreferences {
    pub family: String,
    pub size: f32,
    pub fallback: Vec<String>,
}

impl Default for FontPreferences {
    fn default() -> Self {
        Self {
            family: font::default_font_family().to_string(),
            size: DEFAULT_FONT_SIZE,
            fallback: font::default_font_fallbacks(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontFamilyResolution {
    Requested,
    SystemMonospaceFallback,
    Missing,
}

impl FontFamilyResolution {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::SystemMonospaceFallback => "system-monospace-fallback",
            Self::Missing => "missing",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontFaceSource {
    Bundled,
    System,
}

impl FontFaceSource {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Bundled => "bundled",
            Self::System => "system",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFamilyDiagnostics {
    pub requested_family: String,
    pub resolution: FontFamilyResolution,
    pub resolved_family: Option<String>,
    pub resolved_post_script_name: Option<String>,
    pub resolved_source: Option<FontFaceSource>,
    pub monospaced: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontSampleStatus {
    Primary,
    Fallback,
    Missing,
}

impl FontSampleStatus {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Fallback => "fallback",
            Self::Missing => "missing",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontSampleDiagnostics {
    pub label: &'static str,
    pub text: String,
    pub status: FontSampleStatus,
    pub fonts: Vec<String>,
    pub missing_glyphs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FontDiagnostics {
    pub locale: String,
    pub preferences: FontPreferences,
    pub primary: FontFamilyDiagnostics,
    pub fallbacks: Vec<FontFamilyDiagnostics>,
    pub samples: Vec<FontSampleDiagnostics>,
    pub logs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FontCandidate {
    id: fontdb::ID,
    label: String,
    source: FontFaceSource,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GlyphRasterConfig {
    pub font: FontPreferences,
    pub scale: f32,
    pub cell_width: f32,
    pub line_height: f32,
    pub weight: u16,
    pub bold_weight: u16,
}

impl Default for GlyphRasterConfig {
    fn default() -> Self {
        Self {
            font: FontPreferences::default(),
            scale: 1.0,
            cell_width: DEFAULT_FONT_SIZE,
            line_height: DEFAULT_FONT_SIZE * 1.4,
            weight: 450,
            bold_weight: 650,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedGlyph {
    pub row: usize,
    pub col: usize,
    pub text: String,
    pub style: Style,
    pub cache_key: CacheKey,
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PreparedGlyphFrame {
    pub glyphs: Vec<PreparedGlyph>,
    pub unique_cache_keys: Vec<CacheKey>,
    pub prepared_rows: Vec<usize>,
}

impl PreparedGlyphFrame {
    pub fn raster_jobs(&self) -> usize {
        self.unique_cache_keys.len()
    }
}

#[derive(Debug, Error)]
pub enum GlyphPrepareError {
    #[error("no fonts available for glyph preparation")]
    NoFonts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PaintLayer {
    Background,
    Selection,
    Underline,
    Cursor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaintRect {
    pub layer: PaintLayer,
    pub row: usize,
    pub col: usize,
    pub span: usize,
    pub color: Option<Color>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CellPaintFrame {
    pub rects: Vec<PaintRect>,
    pub prepared_rows: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BorderSegment {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    pub color: Rgba,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BorderFrame {
    pub corner_radius: usize,
    pub segments: Vec<BorderSegment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChromeFrame {
    pub rects: Vec<ChromeRect>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FrameStats {
    pub full_frame: bool,
    pub scrollback_rows: usize,
    pub dirty_rows: usize,
    pub glyphs_prepared: usize,
    pub atlas_uploads: usize,
    pub chrome_rects: usize,
    pub paint_rects: usize,
    pub border_segments: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRenderFrame {
    pub glyphs: PreparedGlyphFrame,
    pub chrome: ChromeFrame,
    pub paint: CellPaintFrame,
    pub border: BorderFrame,
    pub stats: FrameStats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoftwareRenderPalette {
    pub background: Rgba,
    pub foreground: Rgba,
    pub selection_background: Rgba,
    pub selection_foreground: Rgba,
    pub cursor: Rgba,
}

impl Default for SoftwareRenderPalette {
    fn default() -> Self {
        Self {
            background: Rgba::opaque(0x07, 0x0d, 0x12),
            foreground: Rgba::opaque(0xe8, 0xee, 0xf2),
            selection_background: Rgba::opaque(0x2d, 0x5a, 0x84),
            selection_foreground: Rgba::opaque(0xff, 0xff, 0xff),
            cursor: Rgba::opaque(0xff, 0xd8, 0x85),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoftwareRenderFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub stats: FrameStats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PixelRect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

pub fn probe_font_diagnostics(preferences: &FontPreferences) -> FontDiagnostics {
    SOFTWARE_FONT_SYSTEM.with(|font_system| {
        let mut font_system = font_system.borrow_mut();
        collect_font_diagnostics(&mut font_system, preferences)
    })
}

pub fn prepare_glyph_frame(
    plan: &RenderPlan,
    config: &GlyphRasterConfig,
) -> Result<PreparedGlyphFrame, GlyphPrepareError> {
    SOFTWARE_FONT_SYSTEM.with(|font_system| {
        let mut font_system = font_system.borrow_mut();
        prepare_glyph_frame_with_font_system(&mut font_system, plan, config)
    })
}

pub fn prepare_render_frame(
    plan: &RenderPlan,
    config: &GlyphRasterConfig,
) -> Result<PreparedRenderFrame, GlyphPrepareError> {
    SOFTWARE_FONT_SYSTEM.with(|font_system| {
        let mut font_system = font_system.borrow_mut();
        prepare_render_frame_with_font_system(&mut font_system, plan, config)
    })
}

pub fn rasterize_software_frame(
    plan: &RenderPlan,
    config: &GlyphRasterConfig,
    palette: &SoftwareRenderPalette,
    cursor_visible: bool,
) -> Result<SoftwareRenderFrame, GlyphPrepareError> {
    SOFTWARE_FONT_SYSTEM.with(|font_system| {
        SOFTWARE_SWASH_CACHE.with(|swash_cache| {
            let mut font_system = font_system.borrow_mut();
            let mut swash_cache = swash_cache.borrow_mut();
            let prepared = prepare_render_frame_with_font_system(&mut font_system, plan, config)?;
            let width = plan.pane_rect.width.max(1) as u32;
            let height = plan.pane_rect.height.max(1) as u32;
            let mut pixels = vec![0; width as usize * height as usize * 4];
            fill_rgba(&mut pixels, palette.background);

            let selection = plan
                .selection
                .as_ref()
                .map(|selection| selection_ranges(&plan.rows, &selection.clone().normalized()));

            for rect in &prepared.chrome.rects {
                fill_rect_rgba(
                    &mut pixels,
                    width,
                    height,
                    PixelRect {
                        x: rect.rect.x as i32,
                        y: rect.rect.y as i32,
                        width: rect.rect.width as u32,
                        height: rect.rect.height as u32,
                    },
                    rect.color,
                );
            }

            for segment in &prepared.border.segments {
                fill_rect_rgba(
                    &mut pixels,
                    width,
                    height,
                    PixelRect {
                        x: segment.x as i32,
                        y: segment.y as i32,
                        width: segment.width as u32,
                        height: segment.height as u32,
                    },
                    segment.color,
                );
            }

            for rect in &prepared.paint.rects {
                if matches!(rect.layer, PaintLayer::Cursor) && !cursor_visible {
                    continue;
                }

                let x =
                    plan.viewport.x as i32 + (rect.col as f32 * config.cell_width).round() as i32;
                let y =
                    plan.viewport.y as i32 + (rect.row as f32 * config.line_height).round() as i32;
                let cell_width = ((rect.span as f32) * config.cell_width).ceil().max(1.0) as u32;
                let cell_height = config.line_height.ceil().max(1.0) as u32;
                match rect.layer {
                    PaintLayer::Background => {
                        fill_rect_rgba(
                            &mut pixels,
                            width,
                            height,
                            PixelRect {
                                x,
                                y,
                                width: cell_width,
                                height: cell_height,
                            },
                            resolve_color(rect.color.unwrap_or(Color::Default), palette, false),
                        );
                    }
                    PaintLayer::Selection => {
                        fill_rect_rgba(
                            &mut pixels,
                            width,
                            height,
                            PixelRect {
                                x,
                                y,
                                width: cell_width,
                                height: cell_height,
                            },
                            palette.selection_background,
                        );
                    }
                    PaintLayer::Cursor => {
                        fill_rect_rgba(
                            &mut pixels,
                            width,
                            height,
                            PixelRect {
                                x,
                                y,
                                width: cell_width,
                                height: cell_height,
                            },
                            palette.cursor,
                        );
                    }
                    PaintLayer::Underline => {
                        let underline_height = ((config.line_height / 14.0).ceil() as u32).max(1);
                        let underline_y = y + cell_height.saturating_sub(underline_height) as i32;
                        fill_rect_rgba(
                            &mut pixels,
                            width,
                            height,
                            PixelRect {
                                x,
                                y: underline_y,
                                width: cell_width,
                                height: underline_height,
                            },
                            resolve_color(rect.color.unwrap_or(Color::Default), palette, false),
                        );
                    }
                }
            }

            for glyph in &prepared.glyphs.glyphs {
                let selected = selection
                    .as_ref()
                    .is_some_and(|ranges| glyph_intersects_selection(glyph.row, glyph.col, ranges));
                let base = resolve_color(glyph.style.foreground, palette, selected);
                if let Some(image) = swash_cache
                    .get_image(&mut font_system, glyph.cache_key)
                    .as_ref()
                {
                    draw_swash_image(
                        &mut pixels,
                        width,
                        height,
                        plan.viewport.x as i32 + glyph.x,
                        plan.viewport.y as i32 + glyph.y,
                        image,
                        base,
                    );
                }
            }

            Ok(SoftwareRenderFrame {
                width,
                height,
                pixels,
                stats: prepared.stats,
            })
        })
    })
}

fn prepare_render_frame_with_font_system(
    font_system: &mut FontSystem,
    plan: &RenderPlan,
    config: &GlyphRasterConfig,
) -> Result<PreparedRenderFrame, GlyphPrepareError> {
    let glyphs = prepare_glyph_frame_with_font_system(font_system, plan, config)?;
    let chrome = prepare_chrome_frame(plan);
    let paint = prepare_cell_paint_frame(plan);
    let border = prepare_border_frame(plan);
    let stats = FrameStats {
        full_frame: plan.damage.full_frame,
        scrollback_rows: plan.scrollback_rows,
        dirty_rows: glyphs.prepared_rows.len(),
        glyphs_prepared: glyphs.glyphs.len(),
        atlas_uploads: glyphs.raster_jobs(),
        chrome_rects: chrome.rects.len(),
        paint_rects: paint.rects.len(),
        border_segments: border.segments.len(),
    };

    Ok(PreparedRenderFrame {
        glyphs,
        chrome,
        paint,
        border,
        stats,
    })
}

pub fn prepare_chrome_frame(plan: &RenderPlan) -> ChromeFrame {
    ChromeFrame {
        rects: plan
            .chrome
            .iter()
            .filter(|rect| rect.rect.width > 0 && rect.rect.height > 0)
            .cloned()
            .collect(),
    }
}

pub fn prepare_cell_paint_frame(plan: &RenderPlan) -> CellPaintFrame {
    let prepared_rows = effective_render_rows(plan);
    let mut rects = Vec::new();

    for row in &plan.rows {
        if !prepared_rows.contains(&row.row) {
            continue;
        }

        for glyph in &row.glyphs {
            if glyph.wide_continuation {
                continue;
            }

            let span = glyph.span.max(1);
            if glyph.style.background != Color::Default {
                push_paint_rect(
                    &mut rects,
                    PaintRect {
                        layer: PaintLayer::Background,
                        row: row.row,
                        col: glyph.col,
                        span,
                        color: Some(glyph.style.background),
                    },
                );
            }

            if glyph.style.underline {
                push_paint_rect(
                    &mut rects,
                    PaintRect {
                        layer: PaintLayer::Underline,
                        row: row.row,
                        col: glyph.col,
                        span,
                        color: Some(glyph.style.foreground),
                    },
                );
            }
        }
    }

    if let Some(selection) = plan.selection.as_ref() {
        let selection = selection.clone().normalized();
        for (row, start_col, end_col) in selection_ranges(&plan.rows, &selection) {
            if prepared_rows.contains(&row) && end_col > start_col {
                rects.push(PaintRect {
                    layer: PaintLayer::Selection,
                    row,
                    col: start_col,
                    span: end_col - start_col,
                    color: None,
                });
            }
        }
    }

    if prepared_rows.contains(&plan.cursor.row)
        && let Some(row) = plan.rows.get(plan.cursor.row)
    {
        let span = row
            .glyphs
            .get(plan.cursor.col)
            .map(|glyph| glyph.span.max(1))
            .unwrap_or(1);
        rects.push(PaintRect {
            layer: PaintLayer::Cursor,
            row: plan.cursor.row,
            col: plan.cursor.col,
            span,
            color: None,
        });
    }

    CellPaintFrame {
        rects,
        prepared_rows,
    }
}

pub fn prepare_border_frame(plan: &RenderPlan) -> BorderFrame {
    if !plan.border.enabled() || plan.pane_rect.width == 0 || plan.pane_rect.height == 0 {
        return BorderFrame::default();
    }

    let thickness = plan
        .border
        .width
        .min(plan.pane_rect.width)
        .min(plan.pane_rect.height);
    if thickness == 0 {
        return BorderFrame::default();
    }

    let mut segments = Vec::with_capacity(4);
    let color = plan.border.color_for(plan.active);
    segments.push(BorderSegment {
        x: plan.pane_rect.x,
        y: plan.pane_rect.y,
        width: plan.pane_rect.width,
        height: thickness,
        color,
    });
    segments.push(BorderSegment {
        x: plan.pane_rect.x,
        y: plan.pane_rect.y + plan.pane_rect.height.saturating_sub(thickness),
        width: plan.pane_rect.width,
        height: thickness,
        color,
    });

    let side_height = plan
        .pane_rect
        .height
        .saturating_sub(thickness.saturating_mul(2));
    if side_height > 0 {
        segments.push(BorderSegment {
            x: plan.pane_rect.x,
            y: plan.pane_rect.y + thickness,
            width: thickness,
            height: side_height,
            color,
        });
        segments.push(BorderSegment {
            x: plan.pane_rect.x + plan.pane_rect.width.saturating_sub(thickness),
            y: plan.pane_rect.y + thickness,
            width: thickness,
            height: side_height,
            color,
        });
    }

    BorderFrame {
        corner_radius: plan.corner_radius,
        segments,
    }
}

fn collect_font_diagnostics(
    font_system: &mut FontSystem,
    preferences: &FontPreferences,
) -> FontDiagnostics {
    let locale = font_system.locale().to_string();
    let (primary, primary_candidate, mut logs) =
        resolve_primary_family(font_system.db(), &preferences.family);

    let mut fallbacks = Vec::with_capacity(preferences.fallback.len());
    for family in &preferences.fallback {
        let (diagnostics, candidate, fallback_logs) =
            resolve_requested_family(font_system.db(), family);
        fallbacks.push(diagnostics);
        let _ = candidate;
        logs.extend(fallback_logs);
    }

    let samples = if font_system.db().faces().next().is_none() {
        sample_definitions()
            .iter()
            .map(|(label, text)| FontSampleDiagnostics {
                label,
                text: (*text).to_string(),
                status: FontSampleStatus::Missing,
                fonts: Vec::new(),
                missing_glyphs: text
                    .chars()
                    .filter(|ch| !ch.is_whitespace())
                    .map(|ch| ch.to_string())
                    .collect(),
            })
            .collect()
    } else {
        sample_definitions()
            .iter()
            .map(|(label, text)| {
                diagnose_font_sample(
                    font_system,
                    preferences,
                    primary_candidate.as_ref(),
                    label,
                    text,
                )
            })
            .collect()
    };

    FontDiagnostics {
        locale,
        preferences: preferences.clone(),
        primary,
        fallbacks,
        samples,
        logs,
    }
}

fn resolve_primary_family(
    db: &fontdb::Database,
    requested_family: &str,
) -> (FontFamilyDiagnostics, Option<FontCandidate>, Vec<String>) {
    if let Some(face) = query_family_face(db, FontFamily::Name(requested_family)) {
        return (
            FontFamilyDiagnostics {
                requested_family: requested_family.to_string(),
                resolution: FontFamilyResolution::Requested,
                resolved_family: face.families.first().map(|(family, _)| family.clone()),
                resolved_post_script_name: Some(face.post_script_name.clone()),
                resolved_source: Some(font_face_source(face)),
                monospaced: Some(face.monospaced),
            },
            Some(FontCandidate {
                id: face.id,
                label: font_face_label(face),
                source: font_face_source(face),
            }),
            Vec::new(),
        );
    }

    if let Some(face) = preferred_system_monospace_face(db) {
        return (
            FontFamilyDiagnostics {
                requested_family: requested_family.to_string(),
                resolution: FontFamilyResolution::SystemMonospaceFallback,
                resolved_family: face.families.first().map(|(family, _)| family.clone()),
                resolved_post_script_name: Some(face.post_script_name.clone()),
                resolved_source: Some(font_face_source(face)),
                monospaced: Some(face.monospaced),
            },
            Some(FontCandidate {
                id: face.id,
                label: font_face_label(face),
                source: font_face_source(face),
            }),
            vec![format!(
                "requested primary family {:?} unavailable; using system monospace {:?}",
                requested_family,
                face.families
                    .first()
                    .map(|(family, _)| family.as_str())
                    .unwrap_or(face.post_script_name.as_str())
            )],
        );
    }

    (
        FontFamilyDiagnostics {
            requested_family: requested_family.to_string(),
            resolution: FontFamilyResolution::Missing,
            resolved_family: None,
            resolved_post_script_name: None,
            resolved_source: None,
            monospaced: None,
        },
        None,
        vec![format!(
            "requested primary family {:?} unavailable and no system monospace face was found",
            requested_family
        )],
    )
}

fn resolve_requested_family(
    db: &fontdb::Database,
    requested_family: &str,
) -> (FontFamilyDiagnostics, Option<FontCandidate>, Vec<String>) {
    if let Some(face) = query_family_face(db, FontFamily::Name(requested_family)) {
        return (
            FontFamilyDiagnostics {
                requested_family: requested_family.to_string(),
                resolution: FontFamilyResolution::Requested,
                resolved_family: face.families.first().map(|(family, _)| family.clone()),
                resolved_post_script_name: Some(face.post_script_name.clone()),
                resolved_source: Some(font_face_source(face)),
                monospaced: Some(face.monospaced),
            },
            Some(FontCandidate {
                id: face.id,
                label: font_face_label(face),
                source: font_face_source(face),
            }),
            Vec::new(),
        );
    }

    (
        FontFamilyDiagnostics {
            requested_family: requested_family.to_string(),
            resolution: FontFamilyResolution::Missing,
            resolved_family: None,
            resolved_post_script_name: None,
            resolved_source: None,
            monospaced: None,
        },
        None,
        vec![format!(
            "requested fallback family {:?} unavailable",
            requested_family
        )],
    )
}

fn query_family_face<'a>(
    db: &'a fontdb::Database,
    family: FontFamily<'_>,
) -> Option<&'a fontdb::FaceInfo> {
    let attrs = FontAttrs::new().family(family);
    let query = Query {
        families: &[attrs.family],
        weight: attrs.weight,
        stretch: attrs.stretch,
        style: attrs.style,
    };

    db.query(&query).and_then(|id| db.face(id))
}

fn preferred_system_monospace_face(db: &fontdb::Database) -> Option<&fontdb::FaceInfo> {
    for family in font::preferred_system_monospace_families() {
        if let Some(face) = query_family_face(db, FontFamily::Name(family)) {
            return Some(face);
        }
    }

    db.faces()
        .filter(|face| face.monospaced && !face.post_script_name.contains("Emoji"))
        .min_by_key(|face| {
            (
                !face.families[0].0.contains("Mono"),
                face.post_script_name.as_str(),
            )
        })
}

fn font_face_label(face: &fontdb::FaceInfo) -> String {
    let family = face
        .families
        .first()
        .map(|(family, _)| family.as_str())
        .unwrap_or(face.post_script_name.as_str());
    format!(
        "{family} ({}) [{}]",
        face.post_script_name,
        font_face_source(face).label()
    )
}

fn font_face_source(face: &fontdb::FaceInfo) -> FontFaceSource {
    match &face.source {
        fontdb::Source::Binary(_) => FontFaceSource::Bundled,
        _ => FontFaceSource::System,
    }
}

fn sample_definitions() -> [(&'static str, &'static str); 4] {
    [
        ("ascii", FONT_SAMPLE_ASCII),
        ("cjk", FONT_SAMPLE_CJK),
        ("emoji", FONT_SAMPLE_EMOJI),
        ("nerd", FONT_SAMPLE_NERD),
    ]
}

fn diagnose_font_sample(
    font_system: &mut FontSystem,
    preferences: &FontPreferences,
    primary_candidate: Option<&FontCandidate>,
    label: &'static str,
    text: &str,
) -> FontSampleDiagnostics {
    let metrics = Metrics::new(preferences.size, preferences.size * 1.4);
    let attrs = FontAttrs::new()
        .family(FontFamily::Name(&preferences.family))
        .metrics(metrics);
    let glyphs = {
        let mut buffer = FontBuffer::new(font_system, metrics);
        let mut buffer = buffer.borrow_with(font_system);
        let width = (text.chars().count().max(1) as f32) * preferences.size * 2.0;
        buffer.set_size(Some(width), Some(metrics.line_height * 2.0));
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer
            .layout_runs()
            .flat_map(|run| {
                run.glyphs
                    .iter()
                    .map(|glyph| (glyph.start, glyph.end, glyph.font_id, glyph.glyph_id))
            })
            .collect::<Vec<_>>()
    };
    let mut fonts = Vec::new();
    let mut missing_glyphs = Vec::new();
    let mut used_fallback = false;

    for (start, ch) in text.char_indices() {
        if ch.is_whitespace() {
            continue;
        }

        let glyph = ch.to_string();
        let end = start + ch.len_utf8();
        let matched_glyph = glyphs.iter().find(|(glyph_start, glyph_end, _, glyph_id)| {
            *glyph_start <= start && *glyph_end >= end && *glyph_id != 0
        });

        if let Some((_, _, font_id, _)) = matched_glyph {
            if Some(*font_id) != primary_candidate.map(|candidate| candidate.id) {
                used_fallback = true;
            }
            let font_label = font_system
                .db()
                .face(*font_id)
                .map(font_face_label)
                .unwrap_or_else(|| format!("unknown-face-{font_id}"));
            if !fonts.iter().any(|font| font == &font_label) {
                fonts.push(font_label);
            }
        } else if !missing_glyphs.iter().any(|missing| missing == &glyph) {
            missing_glyphs.push(glyph);
        }
    }

    let status = if !missing_glyphs.is_empty() {
        FontSampleStatus::Missing
    } else if used_fallback
        || primary_candidate.is_none()
        || primary_candidate.is_some_and(|candidate| candidate.source != FontFaceSource::Bundled)
    {
        FontSampleStatus::Fallback
    } else {
        FontSampleStatus::Primary
    };

    FontSampleDiagnostics {
        label,
        text: text.to_string(),
        status,
        fonts,
        missing_glyphs,
    }
}

fn prepare_glyph_frame_with_font_system(
    font_system: &mut FontSystem,
    plan: &RenderPlan,
    config: &GlyphRasterConfig,
) -> Result<PreparedGlyphFrame, GlyphPrepareError> {
    if font_system.db().faces().next().is_none() {
        return Err(GlyphPrepareError::NoFonts);
    }

    let prepared_rows = effective_render_rows(plan);
    let mut glyphs = Vec::new();
    let mut unique_cache_keys = BTreeSet::new();
    let baseline_y = glyph_baseline_offset(config.font.size, config.line_height);

    for row in &plan.rows {
        if !prepared_rows.contains(&row.row) {
            continue;
        }

        for glyph in &row.glyphs {
            if glyph.wide_continuation || glyph.text.is_empty() {
                continue;
            }

            let attrs = font_attrs_for_render_glyph(glyph.style, config);
            let shaped_glyphs = shape_cluster(
                font_system,
                &glyph.text,
                attrs,
                Metrics::new(config.font.size, config.line_height),
                config.cell_width.max(config.font.size * 2.0),
                config.line_height,
            );
            let offset = (
                glyph.col as f32 * config.cell_width,
                row.row as f32 * config.line_height + baseline_y,
            );

            for layout_glyph in shaped_glyphs {
                let physical = layout_glyph.physical(offset, config.scale);
                unique_cache_keys.insert(physical.cache_key);
                glyphs.push(PreparedGlyph {
                    row: row.row,
                    col: glyph.col,
                    text: glyph.text.clone(),
                    style: glyph.style,
                    cache_key: physical.cache_key,
                    x: physical.x,
                    y: physical.y,
                });
            }
        }
    }

    Ok(PreparedGlyphFrame {
        glyphs,
        unique_cache_keys: unique_cache_keys.into_iter().collect(),
        prepared_rows,
    })
}

fn glyph_baseline_offset(font_size: f32, line_height: f32) -> f32 {
    let leading = (line_height - font_size).max(0.0);
    (leading * 0.5) + (font_size * 0.8)
}

fn font_attrs_for_render_glyph<'a>(style: Style, config: &'a GlyphRasterConfig) -> FontAttrs<'a> {
    let metrics = Metrics::new(config.font.size, config.line_height);
    let mut attrs = FontAttrs::new()
        .family(FontFamily::Name(&config.font.family))
        .metrics(metrics);

    if style.bold {
        attrs = attrs.weight(fontdb::Weight(config.bold_weight));
    } else {
        attrs = attrs.weight(fontdb::Weight(config.weight));
    }

    if style.italic {
        attrs = attrs.style(fontdb::Style::Italic);
    }

    attrs
}

fn shape_cluster(
    font_system: &mut FontSystem,
    text: &str,
    attrs: FontAttrs<'_>,
    metrics: Metrics,
    width: f32,
    height: f32,
) -> Vec<LayoutGlyph> {
    let mut buffer = FontBuffer::new(font_system, metrics);
    let mut buffer = buffer.borrow_with(font_system);
    buffer.set_size(Some(width), Some(height));
    buffer.set_text(text, &attrs, Shaping::Advanced, None);
    buffer
        .layout_runs()
        .flat_map(|run| run.glyphs.iter().cloned())
        .collect()
}

fn effective_render_rows(plan: &RenderPlan) -> Vec<usize> {
    if plan.damage.full_frame {
        return plan.rows.iter().map(|row| row.row).collect();
    }

    let mut rows = BTreeSet::new();
    for row in &plan.damage.dirty_rows {
        if *row < plan.rows.len() {
            rows.insert(*row);
        }
    }

    if plan.cursor.row < plan.rows.len() {
        rows.insert(plan.cursor.row);
    }

    if let Some(selection) = plan.selection.as_ref() {
        let selection = selection.clone().normalized();
        for (row, _, _) in selection_ranges(&plan.rows, &selection) {
            rows.insert(row);
        }
    }

    rows.into_iter().collect()
}

fn push_paint_rect(rects: &mut Vec<PaintRect>, next: PaintRect) {
    if let Some(last) = rects.last_mut()
        && last.layer == next.layer
        && last.row == next.row
        && last.color == next.color
        && last.col + last.span == next.col
    {
        last.span += next.span;
        return;
    }

    rects.push(next);
}

fn glyph_intersects_selection(row: usize, col: usize, ranges: &[(usize, usize, usize)]) -> bool {
    ranges.iter().any(|(selection_row, start_col, end_col)| {
        *selection_row == row && col >= *start_col && col < *end_col
    })
}

fn fill_rgba(pixels: &mut [u8], color: Rgba) {
    for chunk in pixels.chunks_exact_mut(4) {
        chunk[0] = color.red;
        chunk[1] = color.green;
        chunk[2] = color.blue;
        chunk[3] = color.alpha;
    }
}

fn fill_rect_rgba(pixels: &mut [u8], width: u32, height: u32, rect: PixelRect, color: Rgba) {
    let x_start = rect.x.max(0) as u32;
    let y_start = rect.y.max(0) as u32;
    let x_end = (rect.x.saturating_add(rect.width as i32)).max(0) as u32;
    let y_end = (rect.y.saturating_add(rect.height as i32)).max(0) as u32;
    let x_end = x_end.min(width);
    let y_end = y_end.min(height);

    for row in y_start..y_end {
        for col in x_start..x_end {
            blend_rgba_pixel(pixels, width, height, col, row, color);
        }
    }
}

fn draw_swash_image(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    glyph_x: i32,
    glyph_y: i32,
    image: &cosmic_text::SwashImage,
    base: Rgba,
) {
    let placement = image.placement;
    let left = glyph_x + placement.left;
    let top = glyph_y - placement.top;
    match image.content {
        SwashContent::Mask => {
            let mut index = 0;
            for offset_y in 0..placement.height as i32 {
                for offset_x in 0..placement.width as i32 {
                    let alpha = image.data[index];
                    index += 1;
                    let color = Rgba {
                        red: base.red,
                        green: base.green,
                        blue: base.blue,
                        alpha: multiply_alpha(base.alpha, alpha),
                    };
                    blend_rgba_pixel(
                        pixels,
                        width,
                        height,
                        (left + offset_x) as u32,
                        (top + offset_y) as u32,
                        color,
                    );
                }
            }
        }
        SwashContent::Color => {
            let mut index = 0;
            for offset_y in 0..placement.height as i32 {
                for offset_x in 0..placement.width as i32 {
                    let color = Rgba {
                        red: image.data[index],
                        green: image.data[index + 1],
                        blue: image.data[index + 2],
                        alpha: multiply_alpha(base.alpha, image.data[index + 3]),
                    };
                    index += 4;
                    blend_rgba_pixel(
                        pixels,
                        width,
                        height,
                        (left + offset_x) as u32,
                        (top + offset_y) as u32,
                        color,
                    );
                }
            }
        }
        SwashContent::SubpixelMask => {}
    }
}

fn blend_rgba_pixel(pixels: &mut [u8], width: u32, height: u32, x: u32, y: u32, source: Rgba) {
    if source.alpha == 0 || x >= width || y >= height {
        return;
    }

    let index = ((y * width + x) * 4) as usize;
    if index + 3 >= pixels.len() {
        return;
    }

    let destination = Rgba {
        red: pixels[index],
        green: pixels[index + 1],
        blue: pixels[index + 2],
        alpha: pixels[index + 3],
    };
    let blended = alpha_blend(source, destination);
    pixels[index] = blended.red;
    pixels[index + 1] = blended.green;
    pixels[index + 2] = blended.blue;
    pixels[index + 3] = blended.alpha;
}

fn alpha_blend(source: Rgba, destination: Rgba) -> Rgba {
    let source_alpha = f32::from(source.alpha) / 255.0;
    let destination_alpha = f32::from(destination.alpha) / 255.0;
    let out_alpha = source_alpha + destination_alpha * (1.0 - source_alpha);
    if out_alpha <= f32::EPSILON {
        return Rgba::default();
    }

    let red = ((f32::from(source.red) * source_alpha
        + f32::from(destination.red) * destination_alpha * (1.0 - source_alpha))
        / out_alpha)
        .round()
        .clamp(0.0, 255.0) as u8;
    let green = ((f32::from(source.green) * source_alpha
        + f32::from(destination.green) * destination_alpha * (1.0 - source_alpha))
        / out_alpha)
        .round()
        .clamp(0.0, 255.0) as u8;
    let blue = ((f32::from(source.blue) * source_alpha
        + f32::from(destination.blue) * destination_alpha * (1.0 - source_alpha))
        / out_alpha)
        .round()
        .clamp(0.0, 255.0) as u8;

    Rgba {
        red,
        green,
        blue,
        alpha: (out_alpha * 255.0).round().clamp(0.0, 255.0) as u8,
    }
}

fn multiply_alpha(lhs: u8, rhs: u8) -> u8 {
    ((u16::from(lhs) * u16::from(rhs)) / 255) as u8
}

fn resolve_color(color: Color, palette: &SoftwareRenderPalette, selected: bool) -> Rgba {
    if selected {
        return palette.selection_foreground;
    }

    match color {
        Color::Default => palette.foreground,
        Color::Rgb(red, green, blue) => Rgba::opaque(red, green, blue),
        Color::Indexed(index) => indexed_color(index),
    }
}

fn indexed_color(index: u8) -> Rgba {
    const ANSI: [Rgba; 16] = [
        Rgba::opaque(0x00, 0x00, 0x00),
        Rgba::opaque(0xcd, 0x31, 0x31),
        Rgba::opaque(0x0d, 0xbc, 0x79),
        Rgba::opaque(0xe5, 0xe5, 0x10),
        Rgba::opaque(0x24, 0x72, 0xc8),
        Rgba::opaque(0xbc, 0x3f, 0xbc),
        Rgba::opaque(0x11, 0xa8, 0xcd),
        Rgba::opaque(0xe5, 0xe5, 0xe5),
        Rgba::opaque(0x66, 0x66, 0x66),
        Rgba::opaque(0xf1, 0x4c, 0x4c),
        Rgba::opaque(0x23, 0xd1, 0x8b),
        Rgba::opaque(0xf5, 0xf5, 0x43),
        Rgba::opaque(0x3b, 0x8e, 0xff),
        Rgba::opaque(0xd6, 0x70, 0xd6),
        Rgba::opaque(0x29, 0xb8, 0xdb),
        Rgba::opaque(0xff, 0xff, 0xff),
    ];

    if let Some(color) = ANSI.get(index as usize) {
        return *color;
    }

    if (16..=231).contains(&index) {
        let cube = index - 16;
        let red = cube / 36;
        let green = (cube % 36) / 6;
        let blue = cube % 6;
        return Rgba::opaque(
            cube_component(red),
            cube_component(green),
            cube_component(blue),
        );
    }

    let level = 8 + (index.saturating_sub(232) * 10);
    Rgba::opaque(level, level, level)
}

fn cube_component(value: u8) -> u8 {
    if value == 0 { 0 } else { 55 + value * 40 }
}

fn selection_ranges(rows: &[RenderRow], selection: &Selection) -> Vec<(usize, usize, usize)> {
    if rows.is_empty() {
        return Vec::new();
    }

    let mut start_row = selection.start.row.min(rows.len() - 1);
    let mut end_row = selection.end.row.min(rows.len() - 1);

    if matches!(selection.mode, noctrail_term::SelectionMode::Line) {
        while start_row > 0 && rows[start_row].wrapped {
            start_row -= 1;
        }

        while end_row + 1 < rows.len() && rows[end_row + 1].wrapped {
            end_row += 1;
        }
    }

    let mut ranges = Vec::new();
    for (row_idx, row) in rows.iter().enumerate().take(end_row + 1).skip(start_row) {
        let row_len = row.glyphs.len();
        let (start_col, end_col) = match selection.mode {
            noctrail_term::SelectionMode::Block => (
                selection.start.col.min(selection.end.col),
                selection.start.col.max(selection.end.col) + 1,
            ),
            noctrail_term::SelectionMode::Line => (0, row_len),
            noctrail_term::SelectionMode::Normal => {
                if row_idx == start_row && row_idx == end_row {
                    (
                        selection.start.col.min(selection.end.col),
                        selection.start.col.max(selection.end.col) + 1,
                    )
                } else if row_idx == start_row {
                    (selection.start.col, row_len)
                } else if row_idx == end_row {
                    (0, selection.end.col + 1)
                } else {
                    (0, row_len)
                }
            }
        };
        ranges.push((row_idx, start_col.min(row_len), end_col.min(row_len)));
    }

    ranges
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuBackendDiagnostics {
    pub adapter_name: String,
    pub backend: wgpu::Backend,
    pub device_type: wgpu::DeviceType,
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

const BLIT_SHADER: &str = r#"
@group(0) @binding(0)
var frame_texture: texture_2d<f32>;

@group(0) @binding(1)
var frame_sampler: sampler;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0),
    );
    var uvs = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 2.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(2.0, 0.0),
    );

    var out: VertexOut;
    out.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    out.uv = uvs[vertex_index];
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    return textureSample(frame_texture, frame_sampler, in.uv);
}
"#;

pub struct GpuRenderer {
    instance: wgpu::Instance,
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_config: wgpu::SurfaceConfiguration,
    clear_color: wgpu::Color,
    frame_bind_group_layout: wgpu::BindGroupLayout,
    frame_sampler: wgpu::Sampler,
    frame_pipeline: wgpu::RenderPipeline,
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
        let frame_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("noctrail-frame-bind-group-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let frame_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("noctrail-frame-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..wgpu::SamplerDescriptor::default()
        });
        let frame_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("noctrail-frame-pipeline-layout"),
                bind_group_layouts: &[Some(&frame_bind_group_layout)],
                immediate_size: 0,
            });
        let frame_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("noctrail-frame-shader"),
            source: wgpu::ShaderSource::Wgsl(BLIT_SHADER.into()),
        });
        let frame_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("noctrail-frame-pipeline"),
            layout: Some(&frame_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &frame_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &frame_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        });

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
            frame_bind_group_layout,
            frame_sampler,
            frame_pipeline,
            diagnostics,
        })
    }

    pub fn diagnostics(&self) -> &GpuDiagnostics {
        &self.diagnostics
    }

    pub fn set_clear_color(&mut self, red: f64, green: f64, blue: f64, alpha: f64) {
        self.clear_color = wgpu::Color {
            r: red.clamp(0.0, 1.0),
            g: green.clamp(0.0, 1.0),
            b: blue.clamp(0.0, 1.0),
            a: alpha.clamp(0.0, 1.0),
        };
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

    pub fn render_software_frame(
        &mut self,
        frame: &SoftwareRenderFrame,
    ) -> Result<RenderOutcome, GpuRendererError> {
        let (surface_frame, reconfigure_after_present) = match self.surface.get_current_texture() {
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

        let texture_extent = wgpu::Extent3d {
            width: frame.width.max(1),
            height: frame.height.max(1),
            depth_or_array_layers: 1,
        };
        let frame_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("noctrail-frame-texture"),
            size: texture_extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &frame_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &frame.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(frame.width.max(1) * 4),
                rows_per_image: Some(frame.height.max(1)),
            },
            texture_extent,
        );

        let frame_view = frame_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let frame_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("noctrail-frame-bind-group"),
            layout: &self.frame_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&frame_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.frame_sampler),
                },
            ],
        });

        let surface_view = surface_frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("noctrail-frame-encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("noctrail-frame-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface_view,
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
            pass.set_pipeline(&self.frame_pipeline);
            pass.set_bind_group(0, &frame_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        surface_frame.present();

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

pub fn probe_gpu_backend() -> Result<GpuBackendDiagnostics, GpuRendererError> {
    pollster::block_on(async {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .map_err(GpuRendererError::RequestAdapter)?;
        let _device_and_queue = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .map_err(GpuRendererError::RequestDevice)?;
        let adapter_info = adapter.get_info();

        Ok(GpuBackendDiagnostics {
            adapter_name: adapter_info.name,
            backend: adapter_info.backend,
            device_type: adapter_info.device_type,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic_text::fontdb;
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
        assert_eq!(plan.pane_rect, RenderRect::new(1, 2, 80, 24));
        assert_eq!(plan.viewport, RenderRect::new(1, 2, 80, 24));
        assert!(plan.damage.full_frame);
        assert_eq!(plan.damage.dirty_rows, vec![0]);
        assert_eq!(plan.scrollback_rows, 1);
        assert_eq!(plan.cursor, snapshot.cursor);
        assert!(plan.alternate_screen);
        assert_eq!(plan.selection, snapshot.selection);
        assert!(plan.chrome.is_empty());
        assert!(plan.active);
        assert_eq!(plan.corner_radius, 0);
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
    fn default_font_preferences_match_rendering_note() {
        let preferences = FontPreferences::default();

        assert_eq!(preferences.family, font::default_font_family());
        assert_eq!(preferences.size, DEFAULT_FONT_SIZE);
        for family in font::default_font_fallbacks() {
            assert!(preferences.fallback.iter().any(|item| item == &family));
        }
    }

    #[test]
    fn font_diagnostics_report_missing_fonts_in_empty_database() {
        let mut font_system =
            FontSystem::new_with_locale_and_db("en-US".to_string(), fontdb::Database::new());

        let diagnostics = collect_font_diagnostics(&mut font_system, &FontPreferences::default());

        assert_eq!(
            diagnostics.primary.resolution,
            FontFamilyResolution::Missing
        );
        assert_eq!(diagnostics.primary.resolved_source, None);
        assert_eq!(
            diagnostics.fallbacks.len(),
            font::default_font_fallbacks().len()
        );
        assert!(
            diagnostics
                .fallbacks
                .iter()
                .all(|fallback| fallback.resolution == FontFamilyResolution::Missing)
        );
        assert!(
            diagnostics
                .fallbacks
                .iter()
                .all(|fallback| fallback.resolved_source.is_none())
        );
        assert!(
            diagnostics
                .samples
                .iter()
                .all(|sample| sample.status == FontSampleStatus::Missing)
        );
        assert!(!diagnostics.logs.is_empty());
    }

    #[test]
    fn bundled_default_font_resolves_from_bundled_source() {
        let diagnostics = probe_font_diagnostics(&FontPreferences::default());

        assert_eq!(diagnostics.primary.requested_family, DEFAULT_FONT_FAMILY);
        assert_eq!(
            diagnostics.primary.resolution,
            FontFamilyResolution::Requested
        );
        assert_eq!(
            diagnostics.primary.resolved_source,
            Some(FontFaceSource::Bundled)
        );
        assert!(
            diagnostics
                .samples
                .iter()
                .find(|sample| sample.label == "nerd")
                .is_some_and(|sample| sample.status != FontSampleStatus::Missing)
        );
    }

    #[test]
    fn bundled_font_file_inventory_is_complete() {
        assert_eq!(font::bundled_font_file_count(), 4);
        assert!(font::bundled_fonts_present());
    }

    #[test]
    fn glyph_frame_dedupes_repeated_ascii_raster_jobs() {
        let snapshot = TerminalSnapshot {
            rows: vec![ScreenRowSnapshot {
                cells: vec![cell("A"), cell("A"), cell("A"), cell("A")],
                wrapped: false,
            }],
            ..TerminalSnapshot::default()
        };
        let plan = RenderPlan::from_terminal(
            RenderRect::new(0, 0, 4, 1),
            RenderBackend::Software,
            &snapshot,
        );

        let prepared = prepare_glyph_frame(&plan, &GlyphRasterConfig::default()).unwrap();

        assert_eq!(prepared.glyphs.len(), 4);
        assert_eq!(prepared.raster_jobs(), 1);
    }

    #[test]
    fn glyph_frame_scale_changes_cache_keys() {
        let snapshot = TerminalSnapshot {
            rows: vec![ScreenRowSnapshot {
                cells: vec![cell("A")],
                wrapped: false,
            }],
            ..TerminalSnapshot::default()
        };
        let plan = RenderPlan::from_terminal(
            RenderRect::new(0, 0, 1, 1),
            RenderBackend::Software,
            &snapshot,
        );
        let frame_at_1x = prepare_glyph_frame(&plan, &GlyphRasterConfig::default()).unwrap();
        let frame_at_2x = prepare_glyph_frame(
            &plan,
            &GlyphRasterConfig {
                scale: 2.0,
                ..GlyphRasterConfig::default()
            },
        )
        .unwrap();

        assert_eq!(frame_at_1x.raster_jobs(), 1);
        assert_eq!(frame_at_2x.raster_jobs(), 1);
        assert_ne!(frame_at_1x.unique_cache_keys, frame_at_2x.unique_cache_keys);
    }

    #[test]
    fn glyph_frame_offsets_first_row_below_the_top_edge() {
        let snapshot = TerminalSnapshot {
            rows: vec![ScreenRowSnapshot {
                cells: vec![cell("A")],
                wrapped: false,
            }],
            ..TerminalSnapshot::default()
        };
        let plan = RenderPlan::from_terminal(
            RenderRect::new(0, 0, 1, 1),
            RenderBackend::Software,
            &snapshot,
        );

        let prepared = prepare_glyph_frame(&plan, &GlyphRasterConfig::default()).unwrap();

        assert!(!prepared.glyphs.is_empty());
        assert!(prepared.glyphs.iter().all(|glyph| glyph.y > 0));
    }

    #[test]
    fn cell_paint_frame_merges_backgrounds_and_marks_cursor() {
        let snapshot = TerminalSnapshot {
            rows: vec![ScreenRowSnapshot {
                cells: vec![
                    Cell {
                        text: "A".to_string(),
                        style: Style {
                            foreground: Color::Indexed(7),
                            background: Color::Indexed(1),
                            bold: false,
                            italic: false,
                            underline: false,
                        },
                        wide_continuation: false,
                    },
                    Cell {
                        text: "B".to_string(),
                        style: Style {
                            foreground: Color::Indexed(7),
                            background: Color::Indexed(1),
                            bold: false,
                            italic: false,
                            underline: true,
                        },
                        wide_continuation: false,
                    },
                    cell("C"),
                ],
                wrapped: false,
            }],
            cursor: Cursor { row: 0, col: 1 },
            ..TerminalSnapshot::default()
        };
        let plan = RenderPlan::from_terminal(
            RenderRect::new(0, 0, 3, 1),
            RenderBackend::Software,
            &snapshot,
        );

        let paint = prepare_cell_paint_frame(&plan);

        assert_eq!(
            paint.rects,
            vec![
                PaintRect {
                    layer: PaintLayer::Background,
                    row: 0,
                    col: 0,
                    span: 2,
                    color: Some(Color::Indexed(1)),
                },
                PaintRect {
                    layer: PaintLayer::Underline,
                    row: 0,
                    col: 1,
                    span: 1,
                    color: Some(Color::Indexed(7)),
                },
                PaintRect {
                    layer: PaintLayer::Background,
                    row: 0,
                    col: 2,
                    span: 1,
                    color: Some(Color::Indexed(0)),
                },
                PaintRect {
                    layer: PaintLayer::Cursor,
                    row: 0,
                    col: 1,
                    span: 1,
                    color: None,
                },
            ]
        );
    }

    #[test]
    fn cell_paint_frame_marks_selection_range() {
        let snapshot = TerminalSnapshot {
            rows: vec![ScreenRowSnapshot {
                cells: vec![cell("A"), cell("B"), cell("C"), cell("D")],
                wrapped: false,
            }],
            selection: Some(Selection {
                mode: noctrail_term::SelectionMode::Normal,
                start: noctrail_term::Position { row: 0, col: 1 },
                end: noctrail_term::Position { row: 0, col: 2 },
            }),
            ..TerminalSnapshot::default()
        };
        let plan = RenderPlan::from_terminal(
            RenderRect::new(0, 0, 4, 1),
            RenderBackend::Software,
            &snapshot,
        );

        let paint = prepare_cell_paint_frame(&plan);
        let selection_rects = paint
            .rects
            .iter()
            .filter(|rect| rect.layer == PaintLayer::Selection)
            .cloned()
            .collect::<Vec<_>>();

        assert_eq!(
            selection_rects,
            vec![PaintRect {
                layer: PaintLayer::Selection,
                row: 0,
                col: 1,
                span: 2,
                color: None,
            }]
        );
    }

    #[test]
    fn partial_damage_limits_prepared_rows_and_keeps_cursor_overlay() {
        let snapshot = TerminalSnapshot {
            rows: vec![
                ScreenRowSnapshot {
                    cells: vec![cell("A"), cell("A")],
                    wrapped: false,
                },
                ScreenRowSnapshot {
                    cells: vec![cell("B"), cell("B")],
                    wrapped: false,
                },
                ScreenRowSnapshot {
                    cells: vec![cell("C"), cell("C")],
                    wrapped: false,
                },
            ],
            cursor: Cursor { row: 2, col: 1 },
            ..TerminalSnapshot::default()
        };
        let damage = DamageSet {
            dirty_rows: vec![1],
            full_frame: false,
        };
        let plan = RenderPlan::from_input(RenderInput {
            pane_rect: RenderRect::new(0, 0, 2, 3),
            viewport: RenderRect::new(0, 0, 2, 3),
            backend: RenderBackend::Software,
            snapshot: &snapshot,
            damage: &damage,
            chrome: &[],
            active: true,
            border: PaneBorderStyle::default(),
            corner_radius: 0,
        });

        let prepared = prepare_render_frame(&plan, &GlyphRasterConfig::default()).unwrap();

        assert_eq!(prepared.glyphs.prepared_rows, vec![1, 2]);
        assert_eq!(prepared.paint.prepared_rows, vec![1, 2]);
        assert_eq!(prepared.stats.dirty_rows, 2);
        assert_eq!(
            prepared
                .glyphs
                .glyphs
                .iter()
                .map(|glyph| glyph.row)
                .collect::<Vec<_>>(),
            vec![1, 1, 2, 2]
        );
        assert!(
            prepared
                .paint
                .rects
                .iter()
                .any(|rect| rect.layer == PaintLayer::Cursor && rect.row == 2)
        );
        assert!(prepared.paint.rects.iter().all(|rect| rect.row != 0));
    }

    #[test]
    fn selection_rows_are_included_even_when_not_dirty() {
        let snapshot = TerminalSnapshot {
            rows: vec![
                ScreenRowSnapshot {
                    cells: vec![cell("A"), cell("B"), cell("C")],
                    wrapped: false,
                },
                ScreenRowSnapshot {
                    cells: vec![cell("D"), cell("E"), cell("F")],
                    wrapped: false,
                },
                ScreenRowSnapshot {
                    cells: vec![cell("G"), cell("H"), cell("I")],
                    wrapped: false,
                },
            ],
            cursor: Cursor { row: 2, col: 0 },
            selection: Some(Selection {
                mode: noctrail_term::SelectionMode::Normal,
                start: noctrail_term::Position { row: 0, col: 1 },
                end: noctrail_term::Position { row: 0, col: 2 },
            }),
            ..TerminalSnapshot::default()
        };
        let damage = DamageSet {
            dirty_rows: vec![1],
            full_frame: false,
        };
        let plan = RenderPlan::from_input(RenderInput {
            pane_rect: RenderRect::new(0, 0, 3, 3),
            viewport: RenderRect::new(0, 0, 3, 3),
            backend: RenderBackend::Software,
            snapshot: &snapshot,
            damage: &damage,
            chrome: &[],
            active: true,
            border: PaneBorderStyle::default(),
            corner_radius: 0,
        });

        let prepared = prepare_render_frame(&plan, &GlyphRasterConfig::default()).unwrap();

        assert_eq!(prepared.glyphs.prepared_rows, vec![0, 1, 2]);
        assert!(prepared.paint.rects.iter().any(|rect| {
            rect.layer == PaintLayer::Selection && rect.row == 0 && rect.col == 1 && rect.span == 2
        }));
    }

    #[test]
    fn software_frame_rasterizes_text_into_pixels() {
        let mut terminal = noctrail_term::TerminalState::new(4, 1);
        let _ = terminal.advance_bytes(b"hi");
        let snapshot = terminal.snapshot();
        let plan =
            RenderPlan::from_terminal(RenderRect::new(0, 0, 64, 20), RenderBackend::Gpu, &snapshot);
        let palette = SoftwareRenderPalette::default();
        let frame = rasterize_software_frame(
            &plan,
            &GlyphRasterConfig {
                cell_width: 16.0,
                line_height: 20.0,
                ..GlyphRasterConfig::default()
            },
            &palette,
            true,
        )
        .expect("software frame should rasterize");

        assert_eq!(frame.width, 64);
        assert_eq!(frame.height, 20);
        assert!(frame.pixels.chunks_exact(4).any(|chunk| {
            chunk[0] != palette.background.red
                || chunk[1] != palette.background.green
                || chunk[2] != palette.background.blue
        }));
    }

    #[test]
    fn border_frame_uses_active_color_on_all_edges() {
        let plan = RenderPlan {
            backend: RenderBackend::Software,
            pane_rect: RenderRect::new(10, 20, 120, 80),
            viewport: RenderRect::new(10, 20, 120, 80),
            damage: DamageSet {
                dirty_rows: vec![0],
                full_frame: true,
            },
            scrollback_rows: 0,
            cursor: Cursor::default(),
            alternate_screen: false,
            selection: None,
            chrome: Vec::new(),
            active: true,
            border: PaneBorderStyle {
                width: 2,
                active: Rgba::opaque(0x7a, 0xa2, 0xf7),
                inactive: Rgba::opaque(0x3b, 0x42, 0x61),
            },
            corner_radius: 6,
            rows: Vec::new(),
        };

        let border = prepare_border_frame(&plan);

        assert_eq!(
            border.segments,
            vec![
                BorderSegment {
                    x: 10,
                    y: 20,
                    width: 120,
                    height: 2,
                    color: Rgba::opaque(0x7a, 0xa2, 0xf7),
                },
                BorderSegment {
                    x: 10,
                    y: 98,
                    width: 120,
                    height: 2,
                    color: Rgba::opaque(0x7a, 0xa2, 0xf7),
                },
                BorderSegment {
                    x: 10,
                    y: 22,
                    width: 2,
                    height: 76,
                    color: Rgba::opaque(0x7a, 0xa2, 0xf7),
                },
                BorderSegment {
                    x: 128,
                    y: 22,
                    width: 2,
                    height: 76,
                    color: Rgba::opaque(0x7a, 0xa2, 0xf7),
                },
            ]
        );
        assert_eq!(border.corner_radius, 6);
    }

    #[test]
    fn border_frame_switches_to_inactive_color() {
        let plan = RenderPlan {
            backend: RenderBackend::Software,
            pane_rect: RenderRect::new(0, 0, 8, 4),
            viewport: RenderRect::new(0, 0, 8, 4),
            damage: DamageSet {
                dirty_rows: vec![0],
                full_frame: true,
            },
            scrollback_rows: 0,
            cursor: Cursor::default(),
            alternate_screen: false,
            selection: None,
            chrome: Vec::new(),
            active: false,
            border: PaneBorderStyle {
                width: 1,
                active: Rgba::opaque(0x7a, 0xa2, 0xf7),
                inactive: Rgba::opaque(0x3b, 0x42, 0x61),
            },
            corner_radius: 0,
            rows: Vec::new(),
        };

        let prepared = prepare_render_frame(&plan, &GlyphRasterConfig::default()).unwrap();

        assert_eq!(prepared.stats.border_segments, 4);
        assert!(
            prepared
                .border
                .segments
                .iter()
                .all(|segment| segment.color == Rgba::opaque(0x3b, 0x42, 0x61))
        );
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
            pane_rect: RenderRect::new(1, 2, 6, 7),
            viewport: RenderRect::new(4, 5, 6, 7),
            backend: RenderBackend::Software,
            snapshot: &snapshot,
            damage: &damage,
            chrome: &[ChromeRect {
                layer: ChromeLayer::PaneBackground,
                rect: RenderRect::new(1, 2, 6, 7),
                color: Rgba::opaque(0x14, 0x1c, 0x24),
            }],
            active: false,
            border: PaneBorderStyle::default(),
            corner_radius: 9,
        });

        assert_eq!(plan.pane_rect, RenderRect::new(1, 2, 6, 7));
        assert_eq!(plan.viewport, RenderRect::new(4, 5, 6, 7));
        assert_eq!(plan.damage, damage);
        assert_eq!(
            plan.chrome,
            vec![ChromeRect {
                layer: ChromeLayer::PaneBackground,
                rect: RenderRect::new(1, 2, 6, 7),
                color: Rgba::opaque(0x14, 0x1c, 0x24),
            }]
        );
        assert!(!plan.active);
        assert_eq!(plan.border, PaneBorderStyle::default());
        assert_eq!(plan.corner_radius, 9);
        assert_eq!(plan.rows[0].glyphs.len(), 3);
    }

    #[test]
    fn chrome_frame_preserves_pane_surface_rects() {
        let plan = RenderPlan {
            backend: RenderBackend::Software,
            pane_rect: RenderRect::new(0, 0, 120, 80),
            viewport: RenderRect::new(10, 32, 100, 38),
            damage: DamageSet {
                dirty_rows: vec![0],
                full_frame: true,
            },
            scrollback_rows: 0,
            cursor: Cursor::default(),
            alternate_screen: false,
            selection: None,
            chrome: vec![
                ChromeRect {
                    layer: ChromeLayer::PaneBackground,
                    rect: RenderRect::new(0, 0, 120, 80),
                    color: Rgba::opaque(0x14, 0x1c, 0x24),
                },
                ChromeRect {
                    layer: ChromeLayer::StatusIndicator,
                    rect: RenderRect::new(10, 10, 100, 2),
                    color: Rgba::opaque(0x73, 0xc9, 0xa7),
                },
            ],
            active: true,
            border: PaneBorderStyle::default(),
            corner_radius: 0,
            rows: Vec::new(),
        };

        let chrome = prepare_chrome_frame(&plan);

        assert_eq!(chrome.rects, plan.chrome);
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
