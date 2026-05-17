# Render Fixtures

This document records the deterministic render verification path.

## Entry Points

- `cargo test -p noctrail-render --all-targets`
- `cargo test -p noctrail-cli --all-targets`
- `cargo run -p noctrail-cli -- render-fixtures`
- `cargo run -p noctrail-cli -- render-smoke`

## Fixture Format

Render fixtures live under `tests/fixtures/render/` as `.ntshot` JSON
files.

Each fixture currently describes:

- a `TerminalSnapshot`
- viewport and pane geometry
- a `DamageSet`
- optional pane border style
- optional glyph raster config overrides
- structured expectations for prepared rows, raster jobs, paint rects,
  and border overlays

## Current Fixture Set

The current deterministic fixture set is:

- `ascii.ntshot`
- `cursor.ntshot`
- `selection.ntshot`
- `underline.ntshot`
- `pane-border.ntshot`

Together they verify:

- basic row preparation
- cursor overlays
- selection paint overlays
- underline paint
- active and inactive pane border preparation

## GPU and Fallback Checks

The fixture path is intentionally software-first and deterministic.
Current GPU-adjacent smoke remains separate:

- `cargo run -p noctrail-cli -- doctor gpu`
- `cargo run -p noctrail-app -- smoke`

That split keeps snapshot expectations stable while the GUI still proves
the real GPU init path.
