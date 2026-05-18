# Noctrail Config Reference

Noctrail accepts an optional TOML config via:

```sh
cargo run -p noctrail-app -- gui --config /path/to/noctrail.toml
```

`--safe-mode` ignores config load errors and forces software rendering.

## Top-Level Tables

```toml
[renderer]
[font]
[theme]
[theme.color]
[theme.border]
[theme.pane]
[theme.blur]
[theme.animation]
[theme.low-power]
[theme.cursor]
[theme.selection]
[keymap]
[layout]
[agent]
[agent.provider]
```

## Renderer

```toml
[renderer]
backend = "gpu" # or "software"
```

`renderer.backend`
- Type: string
- Default: `"gpu"`
- Accepted values: `"gpu"`, `"software"`

## Font

```toml
[font]
family = "CaskaydiaMono NFM"
size = 15.0
line-height = 1.55
weight = 450
bold-weight = 650
fallback = ["Microsoft YaHei UI", "Segoe UI Emoji"]
```

`font.family`
- Type: string
- Default: `"CaskaydiaMono NFM"`
- Notes:
  Noctrail now bundles the primary `Caskaydia` Mono run set in-repo, so the default family does not rely on a system install. `doctor font` reports this as `source=bundled`.

`font.size`
- Type: float
- Default: `15.0`
- Constraint: `> 0`

`font.line-height`
- Type: float
- Default: `1.55`
- Constraint: `> 0`

`font.weight`
- Type: integer
- Default: `450`

`font.bold-weight`
- Type: integer
- Default: `650`

`font.fallback`
- Type: string array
- Default on Windows: `["Microsoft YaHei UI", "Segoe UI Emoji", "Noto Color Emoji"]`
- Notes:
  Fallback families remain system-resolved. Use them for CJK, emoji, or any glyph range not covered by bundled `Caskaydia`.

## Theme

```toml
[theme]
opacity = 1.0

[theme.color]
background = "#070d12"
foreground = "#e8eef2"
chrome-background = "#121a22"
chrome-foreground = "#f7fafc"
chrome-muted = "#97a5b2"
chrome-accent = "#7ad9b2"
chrome-danger = "#f28779"

[theme.border]
active = "#91b4ff"
inactive = "#313d49"
width = 1

[theme.pane]
gap = 12
padding = 8
radius = 18

[theme.blur]
enabled = false
fallback-tint-opacity = 0.92

[theme.animation]
enabled = true
duration-ms = 120

[theme.low-power]
enabled = false

[theme.cursor]
color = "#ffd885"
blink-interval-ms = 600

[theme.selection]
background = "#2d5a84d8"
foreground = "#ffffff"
```

`theme.opacity`
- Type: float
- Default: `1.0`
- Constraint: `0.0..=1.0`

Color-valued fields
- Type: string
- Format: `#RRGGBB` or `#RRGGBBAA`

`theme.border.width`
- Type: integer
- Default: `1`

`theme.pane.gap`
- Type: integer
- Default: `12`

`theme.pane.padding`
- Type: integer
- Default: `8`

`theme.pane.radius`
- Type: integer
- Default: `18`

`theme.blur.enabled`
- Type: bool
- Default: `false`

`theme.blur.fallback-tint-opacity`
- Type: float
- Default: `0.92`
- Constraint: `0.0..=1.0`

`theme.animation.enabled`
- Type: bool
- Default: `true`

`theme.animation.duration-ms`
- Type: integer
- Default: `120`
- Constraint: `> 0`

`theme.low-power.enabled`
- Type: bool
- Default: `false`

`theme.cursor.color`
- Type: color string
- Default: `"#ffd885"`

`theme.cursor.blink-interval-ms`
- Type: integer
- Default: `600`
- Constraint: `> 0`

`theme.selection.background`
- Type: color string
- Default: `"#2d5a84d8"`

`theme.selection.foreground`
- Type: color string
- Default: `"#ffffff"`

## Keymap

The current keymap surface is intentionally small and only covers
existing app-level shortcuts. Each field accepts an array of shortcut
strings. Use `[]` to disable an action.

```toml
[keymap]
copy = ["ctrl-shift-c", "ctrl-insert"]
paste = ["ctrl-shift-v", "shift-insert"]
command-palette = ["ctrl-shift-p"]
block-browser = ["ctrl-shift-b"]
patch-preview = ["ctrl-shift-d"]
review-panel = ["ctrl-shift-r"]
agent-context = ["ctrl-shift-a"]
agent-audit = ["ctrl-shift-l"]
focus-left = ["alt-h"]
focus-right = ["alt-l"]
focus-up = ["alt-k"]
focus-down = ["alt-j"]
```

Shortcut grammar:
- Supported modifiers: `ctrl`, `alt`, `shift`
- Supported keys: one printable character, or `insert`
- Examples: `ctrl-shift-c`, `alt-h`, `shift-insert`
- Not supported: `super`, `cmd`, `meta`

Validation rules:
- Every shortcut must parse
- The same shortcut cannot be assigned to two different actions

## Layout

```toml
[layout]
default-split-axis = "auto"
resize-step = 5
scratch-height-percent = 33
startup-workspace = 1
```

`layout.default-split-axis`
- Type: string
- Default: `"auto"`
- Accepted values: `"auto"`, `"horizontal"`, `"vertical"`

`layout.resize-step`
- Type: integer
- Default: `5`
- Constraint: `> 0`

`layout.scratch-height-percent`
- Type: integer
- Default: `33`
- Constraint: `1..=100`

`layout.startup-workspace`
- Type: integer
- Default: `1`
- Constraint: `1..=9`

## Agent

Agent features are still default-off.

```toml
[agent]
enabled = false
read-env = false
read-history = false

[agent.provider]
type = "openai-compatible"
model = "gpt-5"
endpoint = "https://example.invalid/v1"
command = []
```

`agent.enabled`
- Type: bool
- Default: `false`

`agent.read-env`
- Type: bool
- Default: `false`

`agent.read-history`
- Type: bool
- Default: `false`

`agent.provider.type`
- Type: string
- Accepted values: `"openai-compatible"`, `"local"`, `"cli"`

`agent.provider.model`
- Type: optional string
- Required for: `"openai-compatible"`, `"local"`

`agent.provider.endpoint`
- Type: optional string
- Required for: `"openai-compatible"`

`agent.provider.command`
- Type: string array
- Required for: `"cli"`

## Full Example

See [examples/noctrail.example.toml](../examples/noctrail.example.toml).
