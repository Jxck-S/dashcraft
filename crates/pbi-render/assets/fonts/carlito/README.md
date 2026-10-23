# Carlito (bundled font, Calibri metrics)

`pbi-render` embeds these fonts with `include_bytes!` so that text in
**Calibri** (and `Calibri Light`) is measured (`cosmic-text`), declared in the
SVG `@font-face` block and rasterized (`resvg`) with glyphs whose advance
widths and vertical metrics match Calibri's, on every OS (ADR-0005).

| File | Style | Weight used by pbi-render |
|---|---|---|
| `Carlito-Regular.ttf` | Regular | 400 |
| `Carlito-Italic.ttf` | Italic | 400 |
| `Carlito-Bold.ttf` | Bold | 700 |
| `Carlito-BoldItalic.ttf` | Bold Italic | 700 |

All four faces are registered under the family name **`Carlito`**. Calibri
Light has no Carlito counterpart and uses Regular (the closest weight).

Vertical metrics (2048 units per em): `usWinAscent` 1950, `usWinDescent` 550,
`hhea` 1950 / −550 / 0, so `line-height: normal` is 2500/2048 ≈ 1.22 em, as
for Calibri.

## Source

- Project: <https://github.com/googlefonts/carlito> (Version 1.104), as
  published by Google Fonts in <https://github.com/google/fonts/tree/main/ofl/carlito>
  (source commit `3a810cab78ebd6e2e4eed42af9e8453c4f9b850a`).
- SHA-256 of the files (unmodified):
  - `Carlito-Regular.ttf` `f6418f708baede9789daef5d458c0f53d2a888af9820e8062934e504fedc6595`
  - `Carlito-Bold.ttf` `bb5d20f79b82599ec72983597437373a80f2d2085fa91fc144fd74e876a594db`
  - `Carlito-Italic.ttf` `0b019225e58d702bfedcbd35c21696769f8ee115cb6343f84c2f240312450d1c`
  - `Carlito-BoldItalic.ttf` `b32928186c119599e03ca6a1ffc680fdcb7fac95772f4b95d989cf6cd3861517`

## License and attribution

Copyright 2013 The Carlito Project Authors
(https://github.com/googlefonts/carlito), with Reserved Font Name "Carlito".
Carlito is based on Lato by Łukasz Dziedzic.

This Font Software is licensed under the SIL Open Font License, Version 1.1
(see `OFL.txt` in this folder, also at <https://openfontlicense.org>).

Carlito is metric-compatible with Calibri (a Microsoft font that DashCraft
cannot redistribute); it is not Calibri and its glyph shapes differ. The fonts
are bundled unmodified and are not sold by themselves, as the OFL requires.
