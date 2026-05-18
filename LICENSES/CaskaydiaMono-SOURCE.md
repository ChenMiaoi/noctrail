# Bundled Caskaydia Mono Source Notes

Noctrail bundles four static `Caskaydia` mono faces as its default terminal font:

- `CaskaydiaMonoNerdFontMono-Regular.ttf`
- `CaskaydiaMonoNerdFontMono-Bold.ttf`
- `CaskaydiaMonoNerdFontMono-Italic.ttf`
- `CaskaydiaMonoNerdFontMono-BoldItalic.ttf`

The runtime family name resolved by `fontdb` is `CaskaydiaMono NFM`.

Current asset provenance:
- copied from the local Windows font installation at `C:\Windows\Fonts\`

Upstream references:
- Cascadia Code repository: <https://github.com/microsoft/cascadia-code>
- Cascadia releases: <https://github.com/microsoft/cascadia-code/releases>
- Microsoft Learn overview: <https://learn.microsoft.com/en-us/windows/terminal/cascadia-code>

License:
- `LICENSES/CascadiaCode-OFL-1.1.txt`

Notes:
- The bundled set is intentionally limited to the mono regular/bold/italic/bold-italic run set used as Noctrail's primary terminal font.
- CJK and emoji glyph coverage is still expected to come from system fallback families such as `Microsoft YaHei UI` and `Segoe UI Emoji`.
