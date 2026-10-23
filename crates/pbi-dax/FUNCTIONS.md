# DAX function coverage (`pbi-dax`)

Hand-maintained. Every function the evaluator registers
(`pbi_dax::supported_functions()`) must have a row here — the test
`functions_md_lists_every_supported_function` checks it — and at least one
test expression in `tests/functions.rs` (`every_supported_function_is_tested`).

Status:

- **supported** — DAX semantics as documented by Microsoft, including BLANK
  handling;
- **partial** — works for the common forms; the notes say what differs or
  is missing.

Any other function name fails with `DaxError::UnsupportedFunction` (shown as
⚠ + the DAX text; never a crash).

## Logical and information

| Function | Status | Notes |
|---|---|---|
| `IF` | supported | |
| `SWITCH` | supported | `SWITCH(TRUE(), cond, result, …)` works (DAX `=` matching). |
| `COALESCE` | supported | |
| `AND` | supported | |
| `OR` | supported | |
| `NOT` | supported | |
| `TRUE` | supported | |
| `FALSE` | supported | |
| `BLANK` | supported | |
| `ISBLANK` | supported | |
| `IFERROR` | supported | Cancellation, depth and cycle errors are not swallowed. |
| `ISERROR` | supported | |
| `ISNUMBER` | supported | |
| `ISTEXT` | supported | |
| `ISNONTEXT` | supported | |
| `ISLOGICAL` | supported | |
| `ISEVEN` | supported | |
| `ISODD` | supported | |
| `CONVERT` | supported | `INTEGER` rounds half away from zero. |
| `CONTAINS` | supported | DAX `=` comparison per column. |
| `CONTAINSROW` | supported | Strict (`==`) comparison, like `IN`. |
| `IF.EAGER` | supported | Evaluates both branches (an error in either is raised), then chooses like `IF`. |
| `ERROR` | supported | Raises an evaluation error with the text (caught by `IFERROR`). |
| `EVALUATEANDLOG` | supported | Returns the value; the label goes to the `tracing` debug log instead of a trace event. |
| `USERCULTURE` | supported | Always `en-US` (the culture `FORMAT` implements). |
| `ISAFTER` | supported | Scalar form: `(value, start[, ASC\|DESC], …)` compared pair by pair. |
| `ISONORAFTER` | supported | As `ISAFTER`; equal tuples count. |
| `BITAND` | supported | 64-bit integers (arguments truncated). |
| `BITOR` | supported | |
| `BITXOR` | supported | |
| `BITLSHIFT` | supported | A negative shift goes right; shifts of 64 bits or more give 0 (−1 for negative numbers shifted right). |
| `BITRSHIFT` | supported | Arithmetic (sign-keeping) shift. |

## Math and statistics

| Function | Status | Notes |
|---|---|---|
| `DIVIDE` | supported | |
| `ABS` | supported | |
| `INT` | supported | |
| `ROUND` | supported | |
| `ROUNDUP` | supported | |
| `ROUNDDOWN` | supported | |
| `TRUNC` | supported | |
| `MOD` | supported | |
| `POWER` | supported | |
| `SQRT` | supported | |
| `SIGN` | supported | |
| `CURRENCY` | supported | Rounds to 4 decimals (fixed decimal). |
| `PI` | supported | |
| `SQRTPI` | supported | |
| `EXP` | supported | |
| `LN` | supported | `LN(BLANK())` is `LN(0)`, an error (as every math function, BLANK counts as 0). |
| `LOG` | supported | Base 10 by default; base 2 and 10 use exact `log2`/`log10`. |
| `LOG10` | supported | |
| `SIN` | supported | |
| `COS` | supported | |
| `TAN` | supported | |
| `COT` | supported | `\|x\|` must be non-zero and below 2^27. |
| `ASIN` | supported | |
| `ACOS` | supported | |
| `ATAN` | supported | |
| `ACOT` | supported | Result in `(0, π)`. |
| `SINH` | supported | |
| `COSH` | supported | |
| `TANH` | supported | |
| `COTH` | supported | |
| `ASINH` | supported | |
| `ACOSH` | supported | |
| `ATANH` | supported | |
| `ACOTH` | supported | |
| `DEGREES` | supported | |
| `RADIANS` | supported | |
| `FACT` | supported | Truncates; 0 to 170 (`Double`). |
| `COMBIN` | supported | Whole number (`Double` past 64 bits). |
| `COMBINA` | supported | |
| `PERMUT` | supported | |
| `GCD` | supported | Two arguments (as in DAX), non-negative and below 2^53. |
| `LCM` | supported | |
| `QUOTIENT` | supported | |
| `EVEN` | supported | |
| `ODD` | supported | |
| `MROUND` | supported | Same signs required; BLANK number gives BLANK. |
| `CEILING` | supported | Result typed like the significance (currency numbers stay currency). |
| `ISO.CEILING` | supported | Result typed like the number. |
| `FLOOR` | supported | Same signs required. |
| `RAND` | supported | Deterministic: SplitMix64 seeded with a fixed value per evaluation (query, measure, calculated column). |
| `RANDBETWEEN` | supported | As `RAND`. |
| `PRODUCT` | supported | |
| `PRODUCTX` | supported | |
| `MEDIAN` | supported | Dates in, date out. |
| `MEDIANX` | supported | |
| `PERCENTILE.INC` | supported | |
| `PERCENTILE.EXC` | supported | |
| `PERCENTILEX.INC` | supported | |
| `PERCENTILEX.EXC` | supported | |
| `STDEV.S` | supported | BLANK for fewer than two values. |
| `STDEV.P` | supported | |
| `STDEVX.S` | supported | |
| `STDEVX.P` | supported | |
| `VAR.S` | supported | BLANK for fewer than two values. |
| `VAR.P` | supported | |
| `VARX.S` | supported | |
| `VARX.P` | supported | |
| `RANKX` | supported | `ASC`/`DESC` (or 1/0), `SKIP`/`DENSE`, optional value; BLANK compares as 0. |
| `RANK.EQ` | supported | Over the column's visible rows; BLANK when the value is not in the column. |
| `GEOMEAN` | supported | Values must be positive. |
| `GEOMEANX` | supported | |

## Statistical distributions

Own implementations (`src/eval/functions/special.rs`, no dependencies):
regularized incomplete gamma (series below `a + 1`, Legendre continued
fraction above) and beta (continued fraction on the fast-converging side,
`x` and `1 − x` passed separately), both with modified Lentz evaluation and
cancellation-free prefactors for large shape parameters; Stirling-series
`ln Γ`; Halley's method for the normal quantile and safeguarded Newton
(bisection inside a shrinking bracket) for the other inverses. Accuracy: a
relative error of about 1e-13 or better (tests compare 120+ points against
closed forms and an independent implementation at 1e-12). Degrees of
freedom, `alpha`/`beta` and Poisson `x`/`mean` are limited to 10¹⁰ so every
evaluation stays bounded (a deviation for larger values); results are
always finite.

| Function | Status | Notes |
|---|---|---|
| `NORM.DIST` | supported | |
| `NORM.INV` | supported | |
| `NORM.S.DIST` | supported | |
| `NORM.S.INV` | supported | |
| `EXPON.DIST` | supported | |
| `POISSON.DIST` | supported | |
| `BETA.DIST` | supported | Optional bounds `A`, `B`. |
| `BETA.INV` | supported | |
| `CHISQ.DIST` | supported | |
| `CHISQ.DIST.RT` | supported | |
| `CHISQ.INV` | supported | |
| `CHISQ.INV.RT` | supported | |
| `T.DIST` | supported | |
| `T.DIST.2T` | supported | |
| `T.DIST.RT` | supported | |
| `T.INV` | supported | |
| `T.INV.2T` | supported | |
| `CONFIDENCE.NORM` | supported | |
| `CONFIDENCE.T` | supported | |

## Text

| Function | Status | Notes |
|---|---|---|
| `UPPER` | supported | |
| `LOWER` | supported | |
| `LEN` | supported | |
| `LEFT` | supported | |
| `RIGHT` | supported | |
| `MID` | supported | |
| `TRIM` | supported | |
| `CONCATENATE` | supported | |
| `COMBINEVALUES` | supported | |
| `SUBSTITUTE` | supported | |
| `REPLACE` | supported | |
| `REPT` | supported | Result capped at 1 Mi characters. |
| `SEARCH` | supported | Case-insensitive, `?`/`*`/`~` wildcards. |
| `FIND` | supported | Case-sensitive. |
| `CONTAINSSTRING` | supported | |
| `CONTAINSSTRINGEXACT` | supported | |
| `EXACT` | supported | |
| `VALUE` | supported | |
| `FORMAT` | partial | Full format-string support (`pbi_dax::format_value`); cultures other than `en-US` fall back to `en-US`. |
| `FIXED` | supported | |
| `UNICHAR` | supported | |
| `UNICODE` | supported | |

## Date and time

| Function | Status | Notes |
|---|---|---|
| `DATE` | supported | |
| `TIME` | supported | |
| `DATEVALUE` | partial | ISO (`yyyy-mm-dd`) and US (`m/d/yyyy`) text only. |
| `TIMEVALUE` | partial | `hh:mm[:ss] [AM/PM]` text. |
| `NOW` | supported | Local clock. |
| `TODAY` | supported | |
| `UTCNOW` | supported | |
| `UTCTODAY` | supported | |
| `YEAR` | supported | |
| `QUARTER` | supported | BLANK date gives BLANK. |
| `MONTH` | supported | |
| `DAY` | supported | |
| `HOUR` | supported | |
| `MINUTE` | supported | |
| `SECOND` | supported | |
| `WEEKDAY` | supported | Return types 1, 2, 3. |
| `WEEKNUM` | supported | Return types 1, 2, 11–17, 21 (ISO). |
| `EOMONTH` | supported | `months` rounded. |
| `EDATE` | supported | `months` truncated. |
| `DATEDIFF` | supported | Counts interval boundaries; negative when the end is earlier. |
| `YEARFRAC` | supported | Bases 0 (US 30/360 with Excel's end-of-February rules), 1 (actual/actual), 2, 3, 4 (European 30/360); whole days, either order. |
| `NETWORKDAYS` | supported | Weekend codes 1–7, 11–17 (or a 7-character `0`/`1` mask from Monday); holidays as a one-column table; negative when the end is earlier. |
| `CALENDAR` | supported | Capped at 1,000,000 days. |
| `CALENDARAUTO` | supported | Scans the date/time columns of loaded tables, skipping calculated columns and tables. |

## Time intelligence

`dates` is a date column (context transition applies, like
`CALCULATETABLE(DISTINCT(col))`) or a one-column table with the lineage of
a date column. Results are one-column tables of dates (lineage kept, so they
work as `CALCULATE` filters) drawn from all the column's dates. Filtering a
date/time column that keys a date table (the one side of a relationship, or
a table with `dataCategory: Time`) also removes the other filters on that
table, as in Power BI. Periods are whole days; fiscal years via
`year_end_date` (`"6/30"`).

| Function | Status | Notes |
|---|---|---|
| `DATESYTD` | supported | |
| `DATESQTD` | supported | |
| `DATESMTD` | supported | |
| `TOTALYTD` | supported | Optional filter and `year_end_date`. |
| `TOTALQTD` | supported | |
| `TOTALMTD` | supported | |
| `SAMEPERIODLASTYEAR` | supported | |
| `DATEADD` | partial | `DAY`/`MONTH`/`QUARTER`/`YEAR`; a month's last day extends to the end of the target month. Non-contiguous selections are shifted date by date instead of raising an error. |
| `PARALLELPERIOD` | supported | |
| `PREVIOUSDAY` | supported | |
| `PREVIOUSMONTH` | supported | |
| `PREVIOUSQUARTER` | supported | |
| `PREVIOUSYEAR` | supported | |
| `NEXTDAY` | supported | |
| `NEXTMONTH` | supported | |
| `NEXTQUARTER` | supported | |
| `NEXTYEAR` | supported | |
| `STARTOFMONTH` | supported | |
| `STARTOFQUARTER` | supported | |
| `STARTOFYEAR` | supported | |
| `ENDOFMONTH` | supported | |
| `ENDOFQUARTER` | supported | |
| `ENDOFYEAR` | supported | |
| `FIRSTDATE` | supported | |
| `LASTDATE` | supported | |
| `DATESBETWEEN` | supported | BLANK bounds are open. |
| `DATESINPERIOD` | supported | |
| `FIRSTNONBLANK` | partial | Column argument only (not a table expression). |
| `LASTNONBLANK` | partial | Column argument only. |
| `FIRSTNONBLANKVALUE` | partial | Column argument only. |
| `LASTNONBLANKVALUE` | partial | Column argument only. |
| `OPENINGBALANCEMONTH` | supported | |
| `OPENINGBALANCEQUARTER` | supported | |
| `OPENINGBALANCEYEAR` | supported | |
| `CLOSINGBALANCEMONTH` | supported | |
| `CLOSINGBALANCEQUARTER` | supported | |
| `CLOSINGBALANCEYEAR` | supported | |

## Aggregation and iterators

| Function | Status | Notes |
|---|---|---|
| `SUM` | supported | |
| `AVERAGE` | supported | |
| `MIN` | supported | One- and two-argument forms. |
| `MAX` | supported | One- and two-argument forms. |
| `COUNT` | supported | |
| `COUNTA` | supported | |
| `COUNTBLANK` | supported | |
| `COUNTROWS` | supported | No-argument form counts the home table. |
| `DISTINCTCOUNT` | supported | |
| `DISTINCTCOUNTNOBLANK` | supported | |
| `SUMX` | supported | |
| `AVERAGEX` | supported | |
| `MINX` | supported | |
| `MAXX` | supported | |
| `COUNTX` | supported | |
| `COUNTAX` | supported | |
| `CONCATENATEX` | supported | |
| `MAXA` | supported | TRUE = 1, FALSE = 0, text skipped; 0 when there are rows but no usable value, BLANK without rows. |
| `MINA` | supported | As `MAXA`. |
| `AVERAGEA` | supported | Also counts text (numeric text as its number, other text as 0); BLANKs skipped. |

## Filter and context

| Function | Status | Notes |
|---|---|---|
| `CALCULATE` | supported | See the `eval` module docs for the filter-argument rules. |
| `CALCULATETABLE` | supported | |
| `FILTER` | supported | |
| `ALL` | supported | |
| `ALLNOBLANKROW` | supported | |
| `ALLEXCEPT` | partial | `CALCULATE` filter argument only. |
| `REMOVEFILTERS` | supported | |
| `KEEPFILTERS` | supported | |
| `ALLSELECTED` | partial | Restores the *shadow* filter context (`EvalContext::with_allselected`, set per row by `SUMMARIZECOLUMNS`) on the given scope; iterations do not create new shadow contexts. |
| `VALUES` | supported | |
| `DISTINCT` | supported | |
| `FILTERS` | supported | |
| `ISFILTERED` | supported | Direct filters only (expanded-table filters on other tables do not count). |
| `ISCROSSFILTERED` | supported | |
| `HASONEFILTER` | supported | |
| `HASONEVALUE` | supported | |
| `SELECTEDVALUE` | supported | |
| `ISINSCOPE` | supported | Group-by columns of the current level (`SUMMARIZECOLUMNS`, `EvalContext::with_group_by`/`with_data_point`). |
| `ISEMPTY` | supported | |
| `RELATED` | partial | Uses the model's active relationships (ignores `USERELATIONSHIP`). |
| `RELATEDTABLE` | supported | |
| `EARLIER` | supported | |
| `EARLIEST` | supported | |
| `LOOKUPVALUE` | partial | Search columns must belong to the result column's table. |
| `TREATAS` | supported | |
| `USERELATIONSHIP` | supported | `CALCULATE`/`CALCULATETABLE` modifier; the other active relationship between the two tables is deactivated. |
| `CROSSFILTER` | supported | `None`, `OneWay`, `Both`, `OneWay_LeftFiltersRight`, `OneWay_RightFiltersLeft`. |

## Table manipulation

| Function | Status | Notes |
|---|---|---|
| `SUMMARIZE` | partial | No `ROLLUP`. |
| `SUMMARIZECOLUMNS` | partial | Group-by columns, `ROLLUPADDISSUBTOTAL` (subtotal flag columns follow the group-by columns), `ROLLUPGROUP`, filter tables, `IGNORE`. No `NONVISUAL`, no group-level filters in `ROLLUPADDISSUBTOTAL`. |
| `ROLLUPADDISSUBTOTAL` | partial | Inside `SUMMARIZECOLUMNS` only. |
| `ROLLUPGROUP` | partial | Inside `SUMMARIZECOLUMNS`/`ROLLUPADDISSUBTOTAL` only. |
| `IGNORE` | supported | |
| `GROUPBY` | supported | Group-by columns of the table or of related one-side tables. |
| `CURRENTGROUP` | supported | |
| `SELECTCOLUMNS` | supported | |
| `ADDCOLUMNS` | supported | |
| `TOPN` | supported | Ties at the boundary are kept; several order-by keys; default descending. |
| `GENERATE` | supported | |
| `GENERATEALL` | supported | |
| `NATURALINNERJOIN` | partial | Joins on same-name columns whose lineages agree (break lineage with `& ""`/`+ 0` as in DAX); no relationship-based joins. |
| `NATURALLEFTOUTERJOIN` | partial | As `NATURALINNERJOIN`. |
| `CROSSJOIN` | supported | |
| `UNION` | supported | |
| `INTERSECT` | supported | |
| `EXCEPT` | supported | |
| `ROW` | supported | |
| `DATATABLE` | supported | |
| `GENERATESERIES` | supported | |

## Window functions

The relation is split by `PARTITIONBY`, sorted by `ORDERBY`; the current
row is matched from the row context or single-valued filters (see the
`window` module docs). An undetermined current row gives an empty table
(BLANK for `RANK`/`ROWNUMBER`). Without a relation, the `ALLSELECTED`
values of the `ORDERBY`/`PARTITIONBY` columns (one table) are used.

| Function | Status | Notes |
|---|---|---|
| `INDEX` | partial | `blanks` argument ignored (BLANK sorts first). |
| `OFFSET` | partial | As `INDEX`. |
| `WINDOW` | partial | `ABS`/`REL` bounds; as `INDEX`. |
| `RANK` | partial | `SKIP`/`DENSE`; as `INDEX`. |
| `ROWNUMBER` | partial | As `INDEX`. |
| `ORDERBY` | supported | Argument of the window functions only. |
| `PARTITIONBY` | supported | Argument of the window functions only. |
| `MATCHBY` | supported | Argument of the window functions only. |

## Parent-child

| Function | Status | Notes |
|---|---|---|
| `PATH` | supported | Needs a row context on the table; circular chains are errors. |
| `PATHITEM` | supported | |
| `PATHITEMREVERSE` | supported | |
| `PATHLENGTH` | supported | |
| `PATHCONTAINS` | supported | |

## Calculation groups and security

| Function | Status | Notes |
|---|---|---|
| `SELECTEDMEASURE` | supported | In calculation item, selection and dynamic format string expressions: the measure the item applies to (a measure reference: context transition, lower-precedence items apply). Elsewhere an error. |
| `SELECTEDMEASURENAME` | supported | |
| `ISSELECTEDMEASURE` | supported | Arguments must be measure references. |
| `SELECTEDMEASUREFORMATSTRING` | supported | Format string of the selected measure: lower-precedence items' `formatStringDefinition`, else the measure's dynamic or static format string; BLANK when it has none. |
| `USERPRINCIPALNAME` | supported | The effective user of the engine snapshot (`pbi_engine::SecurityContext`, set by `Session::set_effective_roles`); empty text when none. |
| `USERNAME` | supported | Same value as `USERPRINCIPALNAME` (as in the Power BI service). |
| `USEROBJECTID` | supported | The configured object id; empty text when none. |
| `CUSTOMDATA` | supported | The configured custom data; BLANK when none. |

## Not supported yet (selection)

Commonly seen functions that still return `UnsupportedFunction`:
`NONVISUAL`, `SUBSTITUTEWITHINDEX`, `SAMPLE`, `ALLCROSSFILTERED`,
`ROLLUP`/`ROLLUPISSUBTOTAL` (in `SUMMARIZE`), `LINEST`/`LINESTX`,
`TOCSV`/`TOJSON`, `COLUMNSTATISTICS`, the financial functions (`PMT`,
`XIRR`, `XNPV`, …), `EXPAND`/`COLLAPSE`/visual-calculation-only functions
(`RUNNINGSUM`, `MOVINGAVERAGE`, `PREVIOUS`, `NEXT`, `FIRST`, `LAST`),
`INFO.*` DMV functions.

## Numeric conventions

Math, trigonometry and distribution functions convert their arguments like
DAX's implicit conversion (booleans 0/1, numeric text parsed) with BLANK as
0 (FALSE for flags); the rounding-to-a-multiple functions (`MROUND`,
`CEILING`, `FLOOR`, `ISO.CEILING`) keep a BLANK number BLANK, like `ROUND`.
Arguments outside a function's domain are errors ("an argument has an
invalid value (…)"), and so are results that overflow or are not numbers
("the result is too large or too small"): unlike `/`, these functions never
return infinities or `NaN`.

## Known semantic gaps

- **Calculation groups** are applied to measure references (model and query
  measures, not inline aggregations) when the filter context selects one
  item; several or no existing item → `multipleOrEmptySelectionExpression`,
  no filter → `noSelectionExpression` (when defined; with
  `selectionExpressionBehavior: visual` a multiple selection without one
  is BLANK). Highest precedence outermost; an item already applied in the
  evaluation is not applied again (no re-application through
  `SELECTEDMEASURE()` or nested measures; sideways recursion through
  `CALCULATE` works). Gaps: subtotals grouped by a calculation group are
  not hidden for `nonvisual` (a visual-layer rule); dynamic format strings
  are evaluated by `EvalContext::evaluate_measure_formatted` (used by
  `pbi-session` measures), while report visuals still format with static
  format strings.
- **Many-to-many relationships** are limited relationships: filters cross
  them as key values (single direction: `to` filters `from`), no blank row,
  no expanded table (`RELATED` fails; `RELATEDTABLE` works through filter
  propagation). Self-referencing relationships are ignored.
- **Row-level security** is applied by `pbi-session` on a restricted copy of
  the data: role filter expressions are evaluated per row against the
  unrestricted data (not against other roles' security), so a filter that
  reads another secured table sees all of its rows.
- **DirectQuery / Direct Lake / composite models**: no live execution; only
  imported data is evaluated.
- **`ALLSELECTED`**: one shadow context per query/visual data point (and per
  `SUMMARIZECOLUMNS` row); real DAX also creates shadow contexts in
  iterations.
- **Relationship overrides** (`USERELATIONSHIP`, `CROSSFILTER`) change
  filter propagation (so `RELATEDTABLE` and every aggregation) and expanded
  tables, but not `RELATED` navigation or `SUMMARIZE`/`GROUPBY` over related
  columns.
- **Tuple filters** on columns of several tables are applied per table;
  table filters are not projected when one of their columns is overridden.
- **Context transition** filters a stored row by row number: duplicate rows of
  a table without a key count once.
- `DEFINE COLUMN`, `EXPR` (lazy) parameters of user-defined functions,
  `FORMAT` cultures other than `en-US`.
- **Auto-exist** in `SUMMARIZECOLUMNS` is only approximated (cross product
  of the visible values per table, empty rows removed by the measures).

## Fast paths

Some evaluations skip the per-group or per-row work of the general evaluator
when the result is provably the same (`src/eval/fast.rs`; differential tests
there compare both paths on randomized models). They never change results,
only the cost:

- **`SUMMARIZECOLUMNS`** whose extension expressions are plain aggregations
  (`SUM`, `MIN`, `MAX`, `AVERAGE`, `COUNT`, `COUNTA`, `COUNTBLANK`,
  `DISTINCTCOUNT`, `DISTINCTCOUNTNOBLANK` of a column, `COUNTROWS` of a
  table, or measures whose body is one of those) over one table, grouped by
  columns of that table or of its expanded table: one row → group index over
  the visible rows and one pass per aggregation, instead of one filter
  context per value combination. Not used when a calculation item could
  apply to a measure (a group column of a calculation group, a filter on a
  calculation-group table, a `noSelectionExpression`), with `ROLLUPADDISSUBTOTAL`,
  `USERELATIONSHIP`/`CROSSFILTER` in the context, group columns outside the
  expanded table, filter paths that differ from the `RELATED` path, or
  `COUNTBLANK`/`DISTINCTCOUNT` over a table with a blank row.
- **`SUMX`, `AVERAGEX`, `MINX`, `MAXX`, `COUNTX`, `COUNTAX`** over stored
  rows (table reference, `FILTER`, `ALL`, `VALUES` …, no added columns)
  whose row expression combines columns of that table and number literals
  with `+ - * / ^` and unary `-`/`+`: compiled once and evaluated on the
  column dictionaries, without a row context per row. A bare `[Name]` that
  is a measure (context transition) keeps the general path.
- The storage engine answers selective `IN` filters (one data point of a
  visual, context transition) from a per-column inverted index, propagates
  filters on a few one-side rows through the relationship index, and
  aggregates small row sets of high-cardinality columns by sorting their
  dictionary ids, so the general path's per-group cost follows the group's
  size rather than the table's.
