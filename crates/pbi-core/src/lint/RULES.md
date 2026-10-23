# DashCraft lint rules

`pbi_core::lint::lint` (an opened document) and `pbi_core::lint_document` (a bare `Document`) return a `LintReport`: findings with a rule id, severity, message, origin (container-relative file plus a JSON pointer for PBIR/TMSL or a line for TMDL), the object id, and an optional suggestion.

**How reports are bound to models:**

- A report with `byPath` is checked against the semantic model in that folder. That can be the document model, an extra model of a multi-model project, or the sibling model of a lone report folder.
- An embedded report (PBIX/PBIT) is checked against the document model.
- For a `byConnection` report, or when the model was not loaded, model-dependent checks are skipped. The skip is recorded in `summary.notes`.
- `queryRef` consistency, interactions, duplicate names and layout are checked on every report.
- Report-level measures from PBIR `definition/reportExtensions.json` count as defined. `lint_document` has no container, so it cannot see them.

`LintOptions` can disable rules by id (`disabled_rules`), drop findings below a severity (`min_severity`), and turn the layout rules off (`layout: false`). `LintReport::to_json()` is the stable machine format (`schemaVersion: 1`). `LintReport::to_diagnostics()` converts findings to `pbi_model::Diagnostic`, with the rule id as the code and the suggestion appended to the message.

| Rule | Severity | What it flags |
|---|---|---|
| LINT001 | Error | A visual field or formatting expression references a table missing from the model |
| LINT002 | Error | A visual field or formatting expression references a missing column, measure, hierarchy or level |
| LINT003 | Warning / Info | Stale `queryRef` / `nativeQueryRef` |
| LINT004 | Error | A report, page or visual filter references a missing table or field |
| LINT005 | Error | A visual sort references a missing table or field |
| LINT006 | Warning | `visualInteractions` names a visual that is not on the page |
| LINT007 | Error | DAX references an unknown table, column or measure |
| LINT008 | Error | DAX does not parse |
| LINT009 | Error | A relationship references a missing table or column |
| LINT010 | Warning | Relationship key columns have different data types |
| LINT011 | Error | A partition or expression uses an undefined query group |
| LINT012 | Error / Info | The report's dataset reference does not resolve, or is remote |
| LINT013 | Warning / Info | An M query references an undefined step, parameter or query |
| LINT014 | Error / Warning | Duplicate page or visual names |
| LINT015 | Warning | A visual is outside its page or has zero size |
| LINT016 | Info | Two visuals overlap |
| LINT017 | Error | `sortByColumn` or a hierarchy level references a missing column |

## LINT001 / LINT002: visual field missing from the model

LINT001 and LINT002 check every projection (`queryState.<role>.projections[i].field`). They also check the `Column`/`Measure` references inside `visual.objects` and `visualContainerObjects`, such as conditional formatting and selectors.

- **Column, Measure, Aggregation:** the table must exist (LINT001) and the column or measure must be on it (LINT002).
  - A measure bound as a column, or a column bound as a measure, is flagged.
  - So is a measure bound to the wrong home table.
- **HierarchyLevel:** the hierarchy and level must exist. For auto date/time variations, only the base column is checked.
- **Literals and unknown expressions** are not checked.

Origin: `…/visual.json#/visual/query/queryState/<role>/projections/<i>/field`, or the pointer inside `objects`.

Suggestions, in order:

- the closest field on the same table (case-insensitive, small edit distance, or a word prefix/suffix such as `Category` → `Category Name`);
- the same name on another table;
- the closest field anywhere.

## LINT003: stale `queryRef` / `nativeQueryRef`

Power BI generates `queryRef` from the field: `Table.Column`, `Table.Measure`, `Sum(Table.Column)`, `CountNonNull(T.C)` (Count), `Count(T.C)` (Count distinct), and `Table.Hierarchy.Level` (with the variation column for auto date/time). Selectors in `objects` use it as their key. When a field is renamed in the model but the visual keeps its old `queryRef`, the visual can break.

- **Warning:** the `queryRef` names a different field, for example `Orders.Total Sales count` while the field is `Orders[Total Sales]`, or the `queryRef` is missing.
- **Info:**
  - the same field with a different aggregation function name;
  - a `nativeQueryRef` that is neither the field name nor `<Aggregation> of <field>`;
  - a formatting `selector.metadata` that matches no projection's `queryRef` (that formatting is ignored).

These are Power BI conventions, so they are not flagged:

- case-only differences;
- `Sum(T.C)` on a plain column, and `T.C` on an aggregation. "Don't summarize" and switching the aggregation keep the original `queryRef`;
- numbered duplicates (`T.C1`) when the same field appears more than once.

Origin: `…/projections/<i>/queryRef` (or `/nativeQueryRef`). Suggestion: the expected value.

## LINT004: filter references a missing field

Report filters (`report.json#/filterConfig/filters/<i>`), page filters (`page.json`) and visual filters (`visual.json`) are checked the same way as LINT001/LINT002, covering both the filter `field` and every field in its `Where` condition.

## LINT005: sort references a missing field

Entries of `visual.query.sortDefinition.sort` are checked. Origin: `…/visual.json#/visual/query/sortDefinition/sort/<i>`.

## LINT006: dangling `visualInteractions`

The `source` and `target` of each `page.json` `visualInteractions` entry must name a visual on the same page. When you delete a visual, remove its interactions too. Origin: `page.json#/visualInteractions/<i>/source` (or `/target`).

## LINT007 / LINT008: DAX references and syntax

The DAX is parsed with `pbi_dax::syntax::parse_expression`. LINT007 checks the references and LINT008 reports syntax errors. Checked expressions:

- measures;
- calculated columns;
- calculated tables (partition `source`);
- role filter expressions. A missing role table is also LINT007.

How references resolve:

- **`Table`:** must be a model table. Names of `VAR`s are skipped.
- **`Table[Name]`:** must be a column or measure of that table.
- **`[Name]`:** must be a measure, or a column of any table (row context).
- **Names created in the same expression** (string literals in `ADDCOLUMNS`/`SELECTCOLUMNS`/`SUMMARIZE`) also count.
- **Function names** are not checked.

The message gives the line and column inside the expression. For TMDL measures, calculated columns and expressions, the origin line is the file line. A single-line value is on the declaration line, and a multi-line value starts on the next line. For partitions and role filters, the origin line is the object's line.

## LINT009 / LINT010: relationships

- **LINT009:** `fromColumn` and `toColumn` must name an existing table and column. Suggests the closest table or column.
- **LINT010** (Warning): the two key columns have different data types. Columns with `automatic`/`variant`/unknown types are not compared.

## LINT011: undefined query group

A partition's or shared expression's `queryGroup` must be declared in the model, because deploys fail otherwise. Suggests a similar group, or adding the `queryGroup` to `model.tmdl`.

## LINT012: dataset reference

- **Error:** `byPath` does not resolve to a semantic model folder (relative to the report folder) of the opened project, or to the lone report folder's sibling model.
- **Info:** `byConnection`. The model is remote, so field references are not checked.

If the folder exists but its model failed to load, the reader's own error (`CORE003`) covers it. The linter only adds a note.

## LINT013: M references

Partition M sources and M shared expressions are parsed with `pbi_m::parse_expression`.

Identifiers that are not bound are flagged (Warning). Bound means one of:

- a `let` step, a function parameter, `each`'s `_`, a record field, or a `catch` parameter;
- a shared expression or parameter, or another query (table name);
- an undotted library name.

Dotted names (`Table.SelectRows`, `Int64.Type`) are treated as library members and never flagged.

- M is case-sensitive. A case-only mismatch with a query name gets "use `StorageContainerUrl`".
- Other unbound names get the closest step or query name.
- M that does not parse is reported as Info, and its references are not checked.

Column renames inside M steps (renaming `qty` → `Quantity` while an earlier step still uses `qty`) need data and are not checked.

## LINT014: duplicate names

- **Error:** two pages of a report with the same internal `name`, or two visuals on a page with the same `name`.
- **Warning:** two pages with the same display name (case-insensitive).

## LINT015: visual outside its page or zero-size (Warning)

A visible visual must have a positive width and height, and lie within `0..page.width` × `0..page.height`. The tolerance is 0.5 page units. Origin: `visual.json#/position`.

## LINT016: overlapping visuals (Info)

Two visible visuals overlap when their rectangles intersect by more than 0.5 units.

These are skipped:

- groups;
- decorative `shape`/`basicShape`/`image` visuals, such as z-ordered page-frame shapes;
- hidden visuals;
- pairs inside the same group.

Pages with more than 1000 visuals are skipped (noted).

## LINT017: model column references

A column's `sortByColumn` and every hierarchy level's `column` must name a column of the same table.
