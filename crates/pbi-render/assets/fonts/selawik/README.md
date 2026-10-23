# Selawik (bundled font)

`pbi-render` embeds these fonts with `include_bytes!` so that text measurement
(`cosmic-text`), the SVG `@font-face` block and headless PNG rendering
(`resvg`) use exactly the same glyphs on every OS (ADR-0005).

| File | Style | Weight used by pbi-render |
|---|---|---|
| `Selawik-Light.ttf` | Light | 300 |
| `Selawik-Semilight.ttf` | Semilight | 350 (the file says 300; remapped so it does not collide with Light) |
| `Selawik-Regular.ttf` | Regular | 400 |
| `Selawik-Semibold.ttf` | Semibold | 600 |
| `Selawik-Bold.ttf` | Bold | 700 |

All five faces are registered under the single family name **`Selawik`**.

## Source

- Project: <https://github.com/microsoft/Selawik>
- Release: `1.01`, asset `Selawik_Release.zip`
  (SHA-256 `3f62c51e05e3b5a1e6241cf92a371f0be2ea1183aa87b30718bbd40832a8d423`)
- Files renamed from `selawk.ttf`, `selawksl.ttf`, `selawkl.ttf`,
  `selawksb.ttf`, `selawkb.ttf`; contents unmodified.

## License and attribution

Copyright 2015, Microsoft Corporation (www.microsoft.com), with Reserved Font
Name Selawik. All Rights Reserved. Selawik is a trademark of Microsoft
Corporation in the United States and/or other countries.

This Font Software is licensed under the SIL Open Font License, Version 1.1
(see `OFL.txt` in this folder, also at <https://openfontlicense.org>).

Selawik is an open-source replacement for Segoe UI with matching metrics; it
is not Segoe UI and lacks Segoe-matching kerning. The fonts are bundled
unmodified and are not sold by themselves, as the OFL requires.
