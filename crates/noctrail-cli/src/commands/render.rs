use std::{
    env,
    path::{Path, PathBuf},
};

use noctrail_render::{
    GlyphRasterConfig, PaintLayer, PaneBorderStyle, RenderBackend, RenderPlan, RenderRect, Rgba,
    prepare_render_frame,
};
use noctrail_term::{Cell, Color, Cursor, DamageSet, ScreenRowSnapshot, Style, TerminalSnapshot};
use serde::Deserialize;

pub(crate) fn replay_fixtures(patterns: &[String]) -> Result<(), String> {
    let mut paths = Vec::new();
    for pattern in patterns {
        if contains_glob_meta(pattern) {
            let entries = glob::glob(pattern)
                .map_err(|error| format!("failed to parse glob pattern {pattern:?}: {error}"))?;
            for entry in entries {
                let path = entry.map_err(|error| format!("failed to read glob entry: {error}"))?;
                let path = canonicalize_fixture_path(path)?;
                paths.push(path);
            }
        } else {
            paths.push(canonicalize_fixture_path(PathBuf::from(pattern))?);
        }
    }

    if paths.is_empty() {
        return Err("no fixtures matched the provided patterns".to_string());
    }

    paths.sort();
    paths.dedup();

    for path in paths {
        noctrail_term::recording::replay_recording_file(&path)
            .map_err(|error| error.to_string())?;
        println!("replayed {}", path.display());
    }

    Ok(())
}

#[derive(Debug, Deserialize)]
struct RenderFixture {
    surface: RenderFixtureRect,
    #[serde(default)]
    backend: FixtureBackend,
    #[serde(default = "default_active")]
    active: bool,
    snapshot: TerminalSnapshot,
    damage: RenderFixtureDamage,
    #[serde(default)]
    border: FixtureBorder,
    #[serde(default)]
    glyph_raster: FixtureGlyphRaster,
    expect: RenderFixtureExpect,
}

#[derive(Debug, Deserialize)]
struct RenderFixtureRect {
    #[serde(default)]
    x: usize,
    #[serde(default)]
    y: usize,
    width: usize,
    height: usize,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum FixtureBackend {
    Gpu,
    #[default]
    Software,
}

#[derive(Debug, Deserialize)]
struct RenderFixtureDamage {
    dirty_rows: Vec<usize>,
    #[serde(default)]
    full_frame: bool,
}

#[derive(Debug, Deserialize)]
struct FixtureBorder {
    #[serde(default)]
    width: usize,
    #[serde(default = "default_active_border")]
    active: FixtureColor,
    #[serde(default = "default_inactive_border")]
    inactive: FixtureColor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
struct FixtureColor(HexColor);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
struct HexColor {
    red: u8,
    green: u8,
    blue: u8,
    alpha: u8,
}

impl Default for FixtureBorder {
    fn default() -> Self {
        Self {
            width: 0,
            active: default_active_border(),
            inactive: default_inactive_border(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct FixtureGlyphRaster {
    #[serde(default = "default_scale")]
    scale: f32,
    #[serde(default = "default_cell_width")]
    cell_width: f32,
    #[serde(default = "default_line_height")]
    line_height: f32,
}

impl Default for FixtureGlyphRaster {
    fn default() -> Self {
        Self {
            scale: default_scale(),
            cell_width: default_cell_width(),
            line_height: default_line_height(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct RenderFixtureExpect {
    #[serde(default)]
    prepared_rows: Vec<usize>,
    glyph_rows: Option<Vec<usize>>,
    raster_jobs: Option<usize>,
    glyphs_prepared: Option<usize>,
    paint_rects: Option<usize>,
    full_frame: Option<bool>,
    background_rects: Option<Vec<ExpectedRect>>,
    selection_rects: Option<Vec<ExpectedRect>>,
    underline_rects: Option<Vec<ExpectedRect>>,
    cursor_rects: Option<Vec<ExpectedRect>>,
    border_segments: Option<Vec<ExpectedBorderSegment>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct ExpectedRect {
    row: usize,
    col: usize,
    span: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct ExpectedBorderSegment {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    color: FixtureColor,
}

fn default_scale() -> f32 {
    1.0
}

fn default_active() -> bool {
    true
}

fn default_cell_width() -> f32 {
    14.0
}

fn default_line_height() -> f32 {
    19.6
}

fn default_active_border() -> FixtureColor {
    FixtureColor(HexColor {
        red: 0x7a,
        green: 0xa2,
        blue: 0xf7,
        alpha: u8::MAX,
    })
}

fn default_inactive_border() -> FixtureColor {
    FixtureColor(HexColor {
        red: 0x3b,
        green: 0x42,
        blue: 0x61,
        alpha: u8::MAX,
    })
}

impl TryFrom<String> for HexColor {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let Some(hex) = value.strip_prefix('#') else {
            return Err(format!("expected #RRGGBB or #RRGGBBAA, got {value:?}"));
        };
        let bytes = match hex.len() {
            6 => [
                parse_hex_byte(&hex[0..2])?,
                parse_hex_byte(&hex[2..4])?,
                parse_hex_byte(&hex[4..6])?,
                u8::MAX,
            ],
            8 => [
                parse_hex_byte(&hex[0..2])?,
                parse_hex_byte(&hex[2..4])?,
                parse_hex_byte(&hex[4..6])?,
                parse_hex_byte(&hex[6..8])?,
            ],
            _ => return Err(format!("expected 6 or 8 hex digits, got {value:?}")),
        };

        Ok(Self {
            red: bytes[0],
            green: bytes[1],
            blue: bytes[2],
            alpha: bytes[3],
        })
    }
}

impl From<FixtureColor> for Rgba {
    fn from(value: FixtureColor) -> Self {
        Self {
            red: value.0.red,
            green: value.0.green,
            blue: value.0.blue,
            alpha: value.0.alpha,
        }
    }
}

fn parse_hex_byte(raw: &str) -> Result<u8, String> {
    u8::from_str_radix(raw, 16).map_err(|error| format!("invalid hex byte {raw:?}: {error}"))
}

pub(crate) fn run_render_fixtures(patterns: &[String]) -> Result<(), String> {
    let owned_patterns = if patterns.is_empty() {
        default_render_fixture_patterns()
    } else {
        patterns.to_vec()
    };
    let paths = resolve_paths(&owned_patterns)?;

    for path in paths {
        run_render_fixture(&path)?;
        println!("rendered {}", path.display());
    }

    Ok(())
}

fn run_render_fixture(path: &Path) -> Result<(), String> {
    let fixture: RenderFixture =
        serde_json::from_slice(&std::fs::read(path).map_err(|error| {
            format!("failed to read render fixture {}: {error}", path.display())
        })?)
        .map_err(|error| format!("failed to parse render fixture {}: {error}", path.display()))?;
    let damage = DamageSet {
        dirty_rows: fixture.damage.dirty_rows.clone(),
        full_frame: fixture.damage.full_frame,
    };
    let backend = match fixture.backend {
        FixtureBackend::Gpu => RenderBackend::Gpu,
        FixtureBackend::Software => RenderBackend::Software,
    };
    let plan = RenderPlan::from_input(noctrail_render::RenderInput {
        pane_rect: RenderRect::new(
            fixture.surface.x,
            fixture.surface.y,
            fixture.surface.width,
            fixture.surface.height,
        ),
        viewport: RenderRect::new(
            fixture.surface.x,
            fixture.surface.y,
            fixture.surface.width,
            fixture.surface.height,
        ),
        backend,
        snapshot: &fixture.snapshot,
        damage: &damage,
        chrome: &[],
        active: fixture.active,
        border: PaneBorderStyle {
            width: fixture.border.width,
            active: fixture.border.active.into(),
            inactive: fixture.border.inactive.into(),
        },
        corner_radius: 0,
    });
    let prepared = prepare_render_frame(
        &plan,
        &GlyphRasterConfig {
            scale: fixture.glyph_raster.scale,
            cell_width: fixture.glyph_raster.cell_width,
            line_height: fixture.glyph_raster.line_height,
            ..GlyphRasterConfig::default()
        },
    )
    .map_err(|error| {
        format!(
            "failed to prepare render fixture {}: {error}",
            path.display()
        )
    })?;

    assert_render_fixture(path, &fixture.expect, &prepared)
}

fn assert_render_fixture(
    path: &Path,
    expect: &RenderFixtureExpect,
    prepared: &noctrail_render::PreparedRenderFrame,
) -> Result<(), String> {
    if !expect.prepared_rows.is_empty() && prepared.glyphs.prepared_rows != expect.prepared_rows {
        return Err(format!(
            "{} prepared rows mismatch: expected {:?}, got {:?}",
            path.display(),
            expect.prepared_rows,
            prepared.glyphs.prepared_rows
        ));
    }

    if let Some(glyph_rows) = &expect.glyph_rows {
        let actual_rows = prepared
            .glyphs
            .glyphs
            .iter()
            .map(|glyph| glyph.row)
            .collect::<Vec<_>>();
        if &actual_rows != glyph_rows {
            return Err(format!(
                "{} glyph rows mismatch: expected {:?}, got {:?}",
                path.display(),
                glyph_rows,
                actual_rows
            ));
        }
    }

    if let Some(raster_jobs) = expect.raster_jobs
        && prepared.glyphs.raster_jobs() != raster_jobs
    {
        return Err(format!(
            "{} raster jobs mismatch: expected {}, got {}",
            path.display(),
            raster_jobs,
            prepared.glyphs.raster_jobs()
        ));
    }

    if let Some(glyphs_prepared) = expect.glyphs_prepared
        && prepared.stats.glyphs_prepared != glyphs_prepared
    {
        return Err(format!(
            "{} glyph count mismatch: expected {}, got {}",
            path.display(),
            glyphs_prepared,
            prepared.stats.glyphs_prepared
        ));
    }

    if let Some(paint_rects) = expect.paint_rects
        && prepared.stats.paint_rects != paint_rects
    {
        return Err(format!(
            "{} paint rect count mismatch: expected {}, got {}",
            path.display(),
            paint_rects,
            prepared.stats.paint_rects
        ));
    }

    if let Some(full_frame) = expect.full_frame
        && prepared.stats.full_frame != full_frame
    {
        return Err(format!(
            "{} full_frame mismatch: expected {}, got {}",
            path.display(),
            full_frame,
            prepared.stats.full_frame
        ));
    }

    assert_expected_rects(
        path,
        "background",
        PaintLayer::Background,
        expect.background_rects.as_deref(),
        prepared,
    )?;
    assert_expected_rects(
        path,
        "selection",
        PaintLayer::Selection,
        expect.selection_rects.as_deref(),
        prepared,
    )?;
    assert_expected_rects(
        path,
        "underline",
        PaintLayer::Underline,
        expect.underline_rects.as_deref(),
        prepared,
    )?;
    assert_expected_rects(
        path,
        "cursor",
        PaintLayer::Cursor,
        expect.cursor_rects.as_deref(),
        prepared,
    )?;
    assert_expected_border_segments(path, expect.border_segments.as_deref(), prepared)?;

    Ok(())
}

fn assert_expected_rects(
    path: &Path,
    label: &str,
    layer: PaintLayer,
    expected: Option<&[ExpectedRect]>,
    prepared: &noctrail_render::PreparedRenderFrame,
) -> Result<(), String> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let actual = prepared
        .paint
        .rects
        .iter()
        .filter(|rect| rect.layer == layer)
        .map(|rect| ExpectedRect {
            row: rect.row,
            col: rect.col,
            span: rect.span,
        })
        .collect::<Vec<_>>();

    if actual != expected {
        return Err(format!(
            "{} {} rects mismatch: expected {:?}, got {:?}",
            path.display(),
            label,
            expected,
            actual
        ));
    }

    Ok(())
}

fn assert_expected_border_segments(
    path: &Path,
    expected: Option<&[ExpectedBorderSegment]>,
    prepared: &noctrail_render::PreparedRenderFrame,
) -> Result<(), String> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let actual = prepared
        .border
        .segments
        .iter()
        .map(|segment| ExpectedBorderSegment {
            x: segment.x,
            y: segment.y,
            width: segment.width,
            height: segment.height,
            color: FixtureColor(HexColor {
                red: segment.color.red,
                green: segment.color.green,
                blue: segment.color.blue,
                alpha: segment.color.alpha,
            }),
        })
        .collect::<Vec<_>>();

    if actual != expected {
        return Err(format!(
            "{} border segments mismatch: expected {:?}, got {:?}",
            path.display(),
            expected,
            actual
        ));
    }

    Ok(())
}

fn resolve_paths(patterns: &[String]) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    for pattern in patterns {
        if contains_glob_meta(pattern) {
            let entries = glob::glob(pattern)
                .map_err(|error| format!("failed to parse glob pattern {pattern:?}: {error}"))?;
            for entry in entries {
                let path = entry.map_err(|error| format!("failed to read glob entry: {error}"))?;
                paths.push(canonicalize_fixture_path(path)?);
            }
        } else {
            paths.push(canonicalize_fixture_path(PathBuf::from(pattern))?);
        }
    }

    if paths.is_empty() {
        return Err("no fixtures matched the provided patterns".to_string());
    }

    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn default_render_fixture_patterns() -> Vec<String> {
    let workspace_pattern =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/render/*.ntshot");
    vec![
        "tests/fixtures/render/*.ntshot".to_string(),
        workspace_pattern.to_string_lossy().into_owned(),
    ]
}

fn canonicalize_fixture_path(path: PathBuf) -> Result<PathBuf, String> {
    path.canonicalize().map_err(|error| {
        format!(
            "failed to canonicalize fixture path {}: {error}",
            path.display()
        )
    })
}

pub(crate) fn contains_glob_meta(pattern: &str) -> bool {
    pattern.chars().any(|ch| matches!(ch, '*' | '?' | '['))
}

pub(crate) fn run_render_smoke() -> Result<(), String> {
    let snapshot = TerminalSnapshot {
        rows: vec![ScreenRowSnapshot {
            cells: vec![
                Cell {
                    text: "A".to_string(),
                    style: Style {
                        foreground: Color::Indexed(2),
                        background: Color::Default,
                        bold: true,
                        italic: false,
                        underline: false,
                    },
                    wide_continuation: false,
                },
                Cell {
                    text: "界".to_string(),
                    style: Style::default(),
                    wide_continuation: false,
                },
                Cell::wide_continuation(Style::default()),
            ],
            wrapped: false,
        }],
        cursor: Cursor { row: 0, col: 2 },
        ..TerminalSnapshot::default()
    };

    let plan = RenderPlan::from_terminal(
        RenderRect::new(0, 0, 96, 32),
        RenderBackend::Software,
        &snapshot,
    );

    if plan.rows.len() != 1 {
        return Err(format!(
            "render smoke expected 1 row, got {}",
            plan.rows.len()
        ));
    }

    let glyphs = &plan.rows[0].glyphs;
    if glyphs.len() != 3 {
        return Err(format!(
            "render smoke expected 3 glyph entries, got {}",
            glyphs.len()
        ));
    }

    if glyphs[0].text != "A" || !glyphs[0].style.bold {
        return Err("render smoke did not preserve ASCII glyph style".to_string());
    }

    if glyphs[1].text != "界" || glyphs[1].span != 2 {
        return Err("render smoke did not preserve wide glyph metadata".to_string());
    }

    if !glyphs[2].wide_continuation || glyphs[2].span != 0 {
        return Err("render smoke did not preserve wide continuation cell".to_string());
    }

    println!("render smoke ok");
    Ok(())
}
