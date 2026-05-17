# Render Fixtures

Render fixtures live here as JSON `.ntshot` files consumed by:

```bash
cargo run -p noctrail-cli -- render-fixtures
```

Each fixture describes:

- a `TerminalSnapshot`
- a render surface and damage set
- optional active/inactive pane border style
- optional glyph raster config overrides
- structured expectations for prepared rows, raster job counts, paint rects, and
  border segments

This is the deterministic software/golden path for Phase 3 before GPU screenshots
become stable enough to gate in CI.
