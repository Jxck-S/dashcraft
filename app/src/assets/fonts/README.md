# Selawik (UI font)

The app's UI font stack is `"Segoe UI", Selawik, system-ui, sans-serif`: Segoe UI where the OS
has it (Windows), else the bundled Selawik, Microsoft's open-source, metric-compatible fallback
for Segoe UI. The files are unmodified copies of `crates/pbi-render/assets/fonts/selawik/`
(Selawik 1.01 from <https://github.com/microsoft/Selawik>), so the app chrome and the report
renderer use the same glyphs.

Licensed under the SIL Open Font License 1.1 (`OFL.txt`). Copyright 2015, Microsoft
Corporation (www.microsoft.com), with Reserved Font Name Selawik.

`app.css` registers the faces with `unicode-range: U+0000-2025, U+2027-10FFFF`: Selawik 1.01's
U+2026 (HORIZONTAL ELLIPSIS) glyph is a single dot, so the next font in the stack draws `…`.

# Carlito (report text in Calibri)

`carlito/` holds Carlito 1.104 (Regular, Italic, Bold, Bold Italic), unmodified copies of
`crates/pbi-render/assets/fonts/carlito/` (from <https://github.com/googlefonts/carlito> via
Google Fonts). Carlito is metric-compatible with Calibri: the report renderer measures Calibri
text with it and writes `font-family="Carlito, Calibri, sans-serif"` into the page SVG, and
`app.css` registers the four faces so the canvas draws the glyphs that were measured.

Licensed under the SIL Open Font License 1.1 (`carlito/OFL.txt`). Copyright 2013 The Carlito
Project Authors (https://github.com/googlefonts/carlito), with Reserved Font Name "Carlito".
