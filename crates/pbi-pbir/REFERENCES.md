# pbi-pbir references

No third-party code is copied into this crate. These sources were consulted
for facts (field names, enum orders, observed behaviour):

| Source | License | Used for |
|---|---|---|
| [microsoft/json-schemas](https://github.com/microsoft/json-schemas) `fabric/item/report/definition/*` (report 3.3.0, page 2.1.0, visualContainer 2.13.0, visualConfiguration 2.8.0, filterConfiguration 1.3.0, semanticQuery 1.4.0, pagesMetadata 1.1.0, versionMetadata 1.0.0, definitionProperties 2.0.0), `fabric/gitIntegration/platformProperties/2.0.0` | MIT | Property names of the serde types; numeric enum orders used to map legacy layout codes (`displayOption`, interaction `type`, `groupMode`, resource package/item `type`, `howCreated`); `QueryAggregateFunction`, `QueryComparisonKind` and `SortDirection` codes. |
| [d0nk3yhm/pbix-mcp](https://github.com/d0nk3yhm/pbix-mcp) `tests/test_issue119_theme_data_color.py` | MIT | Power BI Desktop measurements for `ThemeDataColor{ColorId, Percent}`: ColorId 0 = white, 1 = black, 2–9 = `dataColors[0..8]`; per-channel shading with half-up rounding. See `src/theme.rs`. |

The legacy `Report/Layout` fixture (`tests/data/legacy_layout.json`) is
synthetic and mirrors part of the Acme Outfitters Stores report
(`reference/examples/acme-outfitters`).
