//! Render plan and backend boundary for Noctrail.

use std::{collections::BTreeSet, sync::Arc};

use cosmic_text::{
    Attrs as FontAttrs, Buffer as FontBuffer, CacheKey, Family as FontFamily, FontSystem,
    LayoutGlyph, Metrics, Shaping,
    fontdb::{self, Query},
};
use noctrail_term::{
    Cell, Color, Cursor, DamageSet, ScreenRowSnapshot, Selection, Style, TerminalSnapshot,
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

const DEFAULT_FONT_FAMILY: &str = "JetBrainsMono Nerd Font";
const DEFAULT_FONT_SIZE: f32 = 14.0;
const DEFAULT_FONT_FALLBACKS: [&str; 4] = [
    "Noto Sans CJK SC",
    "Noto Color Emoji",
    "Apple Color Emoji",
    "Segoe UI Emoji",
];
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
            family: DEFAULT_FONT_FAMILY.to_string(),
            size: DEFAULT_FONT_SIZE,
            fallback: default_font_fallbacks(),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFamilyDiagnostics {
    pub requested_family: String,
    pub resolution: FontFamilyResolution,
    pub resolved_family: Option<String>,
    pub resolved_post_script_name: Option<String>,
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct GlyphRasterConfig {
    pub font: FontPreferences,
    pub scale: f32,
    pub cell_width: f32,
    pub line_height: f32,
}

impl Default for GlyphRasterConfig {
    fn default() -> Self {
        Self {
            font: FontPreferences::default(),
            scale: 1.0,
            cell_width: DEFAULT_FONT_SIZE,
            line_height: DEFAULT_FONT_SIZE * 1.4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedGlyph {
    pub row: usize,
    pub col: usize,
    pub text: String,
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
pub struct FrameStats {
    pub full_frame: bool,
    pub scrollback_rows: usize,
    pub dirty_rows: usize,
    pub glyphs_prepared: usize,
    pub atlas_uploads: usize,
    pub paint_rects: usize,
    pub border_segments: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRenderFrame {
    pub glyphs: PreparedGlyphFrame,
    pub paint: CellPaintFrame,
    pub border: BorderFrame,
    pub stats: FrameStats,
}

pub fn probe_font_diagnostics(preferences: &FontPreferences) -> FontDiagnostics {
    let mut font_system = FontSystem::new();
    collect_font_diagnostics(&mut font_system, preferences)
}

pub fn prepare_glyph_frame(
    plan: &RenderPlan,
    config: &GlyphRasterConfig,
) -> Result<PreparedGlyphFrame, GlyphPrepareError> {
    let mut font_system = FontSystem::new();
    prepare_glyph_frame_with_font_system(&mut font_system, plan, config)
}

pub fn prepare_render_frame(
    plan: &RenderPlan,
    config: &GlyphRasterConfig,
) -> Result<PreparedRenderFrame, GlyphPrepareError> {
    let mut font_system = FontSystem::new();
    let glyphs = prepare_glyph_frame_with_font_system(&mut font_system, plan, config)?;
    let paint = prepare_cell_paint_frame(plan);
    let border = prepare_border_frame(plan);
    let stats = FrameStats {
        full_frame: plan.damage.full_frame,
        scrollback_rows: plan.scrollback_rows,
        dirty_rows: glyphs.prepared_rows.len(),
        glyphs_prepared: glyphs.glyphs.len(),
        atlas_uploads: glyphs.raster_jobs(),
        paint_rects: paint.rects.len(),
        border_segments: border.segments.len(),
    };

    Ok(PreparedRenderFrame {
        glyphs,
        paint,
        border,
        stats,
    })
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
                monospaced: Some(face.monospaced),
            },
            Some(FontCandidate {
                id: face.id,
                label: font_face_label(face),
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
                monospaced: Some(face.monospaced),
            },
            Some(FontCandidate {
                id: face.id,
                label: font_face_label(face),
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
            monospaced: None,
        },
        None,
        vec![format!(
            "requested primary family {:?} unavailable and no system monospace face was found",
            requested_family
        )],
    )
}

fn default_font_fallbacks() -> Vec<String> {
    let mut fallback = DEFAULT_FONT_FALLBACKS
        .iter()
        .map(|family| (*family).to_string())
        .collect::<Vec<_>>();

    #[cfg(target_os = "macos")]
    fallback.insert(1, "PingFang SC".to_string());

    #[cfg(windows)]
    fallback.insert(1, "Microsoft YaHei UI".to_string());

    fallback
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
                monospaced: Some(face.monospaced),
            },
            Some(FontCandidate {
                id: face.id,
                label: font_face_label(face),
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
    for family in preferred_system_monospace_families() {
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

#[cfg(target_os = "macos")]
fn preferred_system_monospace_families() -> &'static [&'static str] {
    &["SF Mono", "Menlo", "Monaco"]
}

#[cfg(windows)]
fn preferred_system_monospace_families() -> &'static [&'static str] {
    &["Cascadia Mono", "Consolas", "Lucida Console"]
}

#[cfg(not(any(target_os = "macos", windows)))]
fn preferred_system_monospace_families() -> &'static [&'static str] {
    &["Noto Sans Mono", "DejaVu Sans Mono", "Liberation Mono"]
}

fn font_face_label(face: &fontdb::FaceInfo) -> String {
    let family = face
        .families
        .first()
        .map(|(family, _)| family.as_str())
        .unwrap_or(face.post_script_name.as_str());
    format!("{family} ({})", face.post_script_name)
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
    } else if used_fallback || primary_candidate.is_none() {
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
                row.row as f32 * config.line_height,
            );

            for layout_glyph in shaped_glyphs {
                let physical = layout_glyph.physical(offset, config.scale);
                unique_cache_keys.insert(physical.cache_key);
                glyphs.push(PreparedGlyph {
                    row: row.row,
                    col: glyph.col,
                    text: glyph.text.clone(),
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

fn font_attrs_for_render_glyph<'a>(style: Style, config: &'a GlyphRasterConfig) -> FontAttrs<'a> {
    let metrics = Metrics::new(config.font.size, config.line_height);
    let mut attrs = FontAttrs::new()
        .family(FontFamily::Name(&config.font.family))
        .metrics(metrics);

    if style.bold {
        attrs = attrs.weight(fontdb::Weight::BOLD);
    }

    if style.italic {
        attrs = attrs.style(fontdb::Style::Italic);
    }

    if !style.bold && !style.italic {
        attrs = attrs.weight(fontdb::Weight::NORMAL);
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

        assert_eq!(preferences.family, DEFAULT_FONT_FAMILY);
        assert_eq!(preferences.size, DEFAULT_FONT_SIZE);
        for family in DEFAULT_FONT_FALLBACKS {
            assert!(preferences.fallback.iter().any(|item| item == family));
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
        assert_eq!(diagnostics.fallbacks.len(), default_font_fallbacks().len());
        assert!(
            diagnostics
                .fallbacks
                .iter()
                .all(|fallback| fallback.resolution == FontFamilyResolution::Missing)
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
            active: false,
            border: PaneBorderStyle::default(),
            corner_radius: 9,
        });

        assert_eq!(plan.pane_rect, RenderRect::new(1, 2, 6, 7));
        assert_eq!(plan.viewport, RenderRect::new(4, 5, 6, 7));
        assert_eq!(plan.damage, damage);
        assert!(!plan.active);
        assert_eq!(plan.border, PaneBorderStyle::default());
        assert_eq!(plan.corner_radius, 9);
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
