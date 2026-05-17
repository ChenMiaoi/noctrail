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

`backend`
- Type: string
- Default: `"gpu"`
- Accepted values: `"gpu"`, `"software"`

## Font

```toml
[font]
family = "JetBrainsMono Nerd Font"
size = 14.0
fallback = ["Noto Sans CJK SC", "Noto Color Emoji"]
```

`family`
- Type: string
- Default: `"JetBrainsMono Nerd Font"`

`size`
- Type: float
- Default: `14.0`
- Constraint: `> 0`

`fallback`
- Type: string array
- Default:
  `["Noto Sans CJK SC", "Noto Color Emoji", "Apple Color Emoji",
  "Segoe UI Emoji"]`

## Theme

```toml
[theme]
opacity = 0.92

[theme.color]
background = "#050a0f"
foreground = "#c0caf5"

[theme.border]
active = "#7aa2f7"
inactive = "#3b4261"
width = 1

[theme.pane]
gap = 4
padding = 2
radius = 8

[theme.blur]
enabled = false
fallback-tint-opacity = 0.92

[theme.animation]
enabled = true
duration-ms = 120

[theme.low-power]
enabled = false

[theme.cursor]
color = "#c0caf5"
blink-interval-ms = 600

[theme.selection]
background = "#264f78cc"
foreground = "#ffffff"
```

`theme.opacity`
- Type: float
- Default: `1.0`
- Constraint: `0.0..=1.0`

`theme.color.background`
- Type: color string
- Default: `"#050a0f"`

`theme.color.foreground`
- Type: color string
- Default: `"#c0caf5"`

`theme.border.active`
- Type: color string
- Default: `"#7aa2f7"`

`theme.border.inactive`
- Type: color string
- Default: `"#3b4261"`

Color-valued fields
- Type: string
- Format: `#RRGGBB` or `#RRGGBBAA`

`theme.border.width`
- Type: integer
- Default: `1`

`theme.pane.gap`
- Type: integer
- Default: `4`

`theme.pane.padding`
- Type: integer
- Default: `2`

`theme.pane.radius`
- Type: integer
- Default: `8`

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
- Default: `"#c0caf5"`

`theme.cursor.blink-interval-ms`
- Type: integer
- Default: `600`
- Constraint: `> 0`

`theme.selection.background`
- Type: color string
- Default: `"#264f78cc"`

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

The layout table controls how the current pane tree behaves at startup
and how keyboard-driven layout mutations behave.

```toml
[layout]
default-split-axis = "auto"
resize-step = 5
scratch-height-percent = 33
startup-workspace = 1
```

`default-split-axis`
- Type: string
- Default: `"auto"`
- Accepted values: `"auto"`, `"horizontal"`, `"vertical"`
- Effect:
  `split_active_pane_shell()` follows auto BSP behavior in `"auto"`
  mode, or always uses the configured axis otherwise

`resize-step`
- Type: integer
- Default: `5`
- Constraint: `> 0`
- Effect: command-palette resize operations move splits by this delta

`scratch-height-percent`
- Type: integer
- Default: `33`
- Constraint: `1..=100`
- Effect: dropdown scratch pane height as a percentage of window height

`startup-workspace`
- Type: integer
- Default: `1`
- Constraint: `1..=9`
- Effect: GUI launches focused on that workspace

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
- Example:
  `command = ["sh", "-lc", "printf '{\\\"commands\\\":[]}'"]`

Provider validation:
- If `agent.enabled = true`, `agent.provider` must exist
- `openai-compatible` requires `model` and `endpoint`
- `local` requires `model`
- `cli` requires `command`

## Full Example

See [examples/noctrail.example.toml](../examples/noctrail.example.toml).
