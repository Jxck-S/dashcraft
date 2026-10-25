# `dashcraft` CLI reference

The `dashcraft` binary (crate `dashcraft-cli`) works offline: no network access (except the opt-in `ai.*` commands, [AI.md](AI.md)), no telemetry, and embedded scripts, custom visuals and connectors are never run. Inputs are detected by content, so any of these work: a `.pbip` file or project folder, a `*.SemanticModel` or `*.Report` folder, a `.pbix` (or an unzipped PBIX folder), a `.pbit`, a `model.bim`, a TMDL folder, and the other formats in [FORMATS.md](../FORMATS.md).

```sh
cargo build --release -p dashcraft-cli      # target/release/dashcraft
dashcraft --help                          # and dashcraft <command> --help
```

For task-oriented recipes (CI checks, golden comparisons, exports), see the [user guide](USER-GUIDE.md#cli-recipes).

## Commands

| Command | What it does |
|---|---|
| `dashcraft inspect <path> [--json] [--pretty] [--no-expressions]` | Model, reports and reader diagnostics, as a tree or as JSON with a stable schema |
| `dashcraft detect <path> [--json] [--pretty]` | Format detection only (signals and the items found) |
| `dashcraft open <path>` | Opens the file in the desktop app (`dashcraft-app`) |
| `dashcraft convert <path> --to pbip --out <dir> [--force]` | Writes a PBIP project (TMDL + PBIR) |
| `dashcraft convert <path> --to pbit --out <file> [--report <name>] [--force]` | Writes a PBIT template (model schema and one report, no data) |
| `dashcraft lint <path> [--json] [--pretty] [--deny errors\|warnings\|infos] [--disable <RULE>]… [--no-layout]` | Broken-reference linter: visual fields, `queryRef`s, filters, sorts, interactions, DAX/M references, relationships, query groups, layout ([rules](../crates/pbi-core/src/lint/RULES.md)) |
| `dashcraft refresh <path> [--source auto\|m\|cache] [--blob-map <file>] [--tables <t1,t2>] [--json] [--pretty]` | Loads the data headlessly (a PBIP's local data cache, M partitions, embedded PBIX data, calculated columns and tables) and prints rows, time and status per table, with hints |
| `dashcraft measures <path> [--source …] [--blob-map <file>] [--report <name>] [--json] [--pretty]` | Refreshes, then prints every measure's formatted value; `--report` applies that report's filters. Failed measures show ⚠ and their DAX |
| `dashcraft export <path> --table <name> --format csv\|parquet --out <file> [--source …] [--blob-map <file>] [--formatted] [--bom] [--excel-safe] [--delimiter <c>] [--compression zstd\|snappy\|none] [--force]` | Refreshes, then writes one table's data |
| `dashcraft export <path> --model --format json\|md --out <file\|-> [--no-expressions] [--no-hidden] [--title <text>] [--force]` | Writes the model documentation: a JSON model doc or a Markdown data dictionary (no refresh needed) |
| `dashcraft render <path> (--png <file> \| --svg <file> \| --out-dir <dir> [--format png\|svg\|both]) [--report <name>] [--page <page> \| --all-pages] [--scale <n>] [--slicer <slicer=values>]… [--now <rfc3339>] [--source …] [--blob-map <file>] [--force] [--json] [--pretty]` | Refreshes, then renders report pages headlessly to PNG (resvg, bundled fonts) or SVG, with per-visual status |
| `dashcraft commands [<id>] [--json] [--schemas] [--markdown] [--agent-safe] [--pretty]` | Lists the command catalog of the registry (`pbi-ops`): ids, summaries, flags; one command's parameters, result and examples; JSON Schemas; or the Markdown of [COMMANDS.md](COMMANDS.md) |
| `dashcraft run <id> [--params <json>] [-p <key=value>]… [--doc <path>] [--blob-map <file>] [--pretty]` | Runs one catalog command and prints its JSON result |
| `dashcraft run --batch <file\|-> [--doc <path>] [--blob-map <file>] [--stop-on-error true\|false] [--pretty]` | Runs ordered steps on one workspace, with `${name.field}` references to earlier results |
| `dashcraft serve [--stdio] [--read-root <dir>]… [--write-root <dir>]… [--blob-map <file>] [--max-concurrent <n>] [--agent]` | Serves the catalog as JSON-lines RPC (JSON-RPC 2.0) on stdin/stdout with a persistent workspace |
| `dashcraft mcp [--read-root <dir>]… [--write-root <dir>]… [--blob-map <file>] [--allow <category\|id>]… [--deny <category\|id>]… [--tool-names underscore\|dotted] [--max-concurrent <n>]` | Model Context Protocol server on stdin/stdout: the agent-safe commands as MCP tools, confined to read/write roots ([MCP.md](MCP.md)) |
| `dashcraft ai login --provider <id> [--stdin]` / `dashcraft ai logout --provider <id>` | Stores (prompted without echo, or the first line of stdin) or deletes an AI provider's API key in the OS keychain; the `ai.*` commands themselves run through `run`/`serve`/`mcp` ([AI.md](AI.md)) |

Flags shared by every command that reads a model (`inspect`, `convert`, `lint`, `refresh`, `measures`, `export`, `render`):

- `--include-system-tables` keeps the internal VertiPaq tables (`H$`, `R$`, `U$`) of PBIX/ABF models.
- `--no-pbix-schema` skips decompressing the PBIX `DataModel` (or a PBIP's `.pbi/cache.abf`) to read the model schema. It's faster, but the model is reported as not loaded.

Global flags:

- `-v`/`--verbose` logs progress on stderr. Repeat it for more detail (`-vv` debug, `-vvv` trace). Without it, `RUST_LOG` is honoured.
- `-q`/`--quiet` prints only errors on stderr.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success. Warning and Info diagnostics don't change the exit code (`lint --deny` can make them fail). |
| 1 | Fatal error: missing, unreadable, refused or unsupported input, usage error, output already exists, app not found. The error is printed on stderr as `dashcraft: error: …`. |
| 2 | The input was read, but it has Error diagnostics (or, for `lint`, findings at or above the `--deny` severity). The output is still complete; with `--json` the errors are in `diagnostics`. For `refresh`, `measures`, `export --table` and `render`: a table or calculated column failed to refresh, a measure failed, or (`render`) a visual's query failed. The output is still written. If the exported table itself failed, `export` exits with 1 and writes nothing. |

## Flag details

### `inspect`, `detect`

- `--pretty` implies `--json`.
- `inspect --no-expressions` leaves out all DAX, M and query text: measures, calculated columns and tables, partitions, parameter values and role filters. Use it when sharing output.
- `inspect` reports reader diagnostics only. Run `lint` for broken references.

### `convert`

- Never overwrites a non-empty output folder or an existing file unless you pass `--force`.
- A PBIT holds one report. `--to pbit` writes the first report, or the one named by `--report` (exact name), and warns if other reports were left out.

### `lint`

- Prints findings grouped by file, each with its JSON pointer or line, plus a fix hint where there is one.
- Exits with 2 on Error findings or reader errors. `--deny warnings` (or `infos`) also fails on lower severities. `--disable LINT016` skips a rule; `--no-layout` skips LINT015 and LINT016.
- `--json` prints `{ schemaVersion: 1, path, summary { errors, warnings, infos, byRule, notes? }, findings[] { rule, severity, message, origin? { path, pointer?, line? }, object?, suggestion? }, readerErrors[] }`.

### Data commands: `refresh`, `measures`, `export --table`, `render`

- These load data offline. M partitions that read `AzureStorage.Blobs` need `--blob-map <file>`: a `blob-map.json` that maps blob URLs to local files, relative to the map. Without it those tables fail with an "offline" hint. Other data-access functions (`Sql.Database`, `Web.Contents`, `File.Contents`, …) are never evaluated; their tables fail with an "unsupported" status.
- PBIX, `.abf` and Power Pivot files use their embedded data.
- A failed table is left empty, and what depends on it is skipped with the reason.
- `--source` picks where a PBIP's data comes from:
  - `auto` (the default) loads the local data cache `<Model>.SemanticModel/.pbi/cache.abf` (the data of Power BI Desktop's last refresh) when there is one: no M is evaluated and no blob map is needed, and a note on stderr says so. Without a cache, `auto` evaluates the M queries.
  - `m` always evaluates the M queries.
  - `cache` requires the cache (exit 1 without one).
  - Cache tables and columns are matched to the model by name. Calculated columns and tables missing from the cache are computed; a table missing from it fails (refresh it from the source); a data column missing from it loads as blank with a warning.
- `refresh --tables` refreshes the named tables plus the calculated columns and tables that depend on them.
- `refresh --json` prints `{ schemaVersion: 1, source, ok, durationMs, summary { tables, ok, failed, skipped }, tables[] { name, kind: m|calculated|pbix|cache|manual|skipped, rows, durationMs, status { state: ok|error|skipped, message?, unsupported?, hint?, csvFallback? }, columns[] { name, durationMs, status } }, diagnostics[] }`.

### `measures`

- `--report <name>` applies the report-level filters of that report (exact name, or a unique part of it, case-insensitive). Filters that can't be translated (Top N, relative date, …) are listed as warnings and not applied.
- `--json` prints `{ schemaVersion: 1, source, report?, filters[] { table, column, values[], exclude }, unsupportedFilters[], refresh { tables, ok, failed, skipped }, measures[] { table, name, value, formatted?, formatString?, error?, unsupported, dax, hidden, displayFolder? } }`. `value` is the raw value (BLANK is `null`, dates are ISO text) and `formatted` uses the measure's format string.

### `export`

- The format comes from `--format`, or else from the `--out` extension (`.csv`, `.parquet`, `.json`, `.md`). `--out -` writes CSV, JSON or Markdown to stdout.
- Never overwrites an existing file unless you pass `--force`. Files are written to a temporary file and renamed into place, so a failed export leaves nothing behind.
- `--table` writes raw values by default (`0.5`, `1.2500` for fixed decimals, `2026-10-21T14:05:00`, BLANK as an empty field). `--formatted` applies each column's model format string instead, as Power BI shows it. Fields are quoted only when they contain the delimiter, `"` or a line break.
- `--bom` adds a UTF-8 BOM so Excel detects the encoding.
- `--excel-safe` guards against spreadsheet formula injection: text cells (and header names) that start with `=`, `+`, `-`, `@`, TAB or CR get a leading `'`, so a spreadsheet shows them as text instead of running them. Numbers, dates and booleans are unchanged.
- Parquet files are Zstd-compressed by default and carry the model metadata: Arrow field metadata (`dashcraft.dataType`, `dashcraft.kind`, `dashcraft.description`, `dashcraft.formatString`, `dashcraft.sourceColumn`, and `dashcraft.derived` for session-derived columns) and file key-value metadata (`dashcraft.schemaVersion`, `dashcraft.table`, `dashcraft.tableDescription`, `dashcraft.culture`, and `dashcraft.columns` as a JSON array).
- `--model --format json` writes the model doc (`schemaVersion: 1`, defined in `crates/pbi-session/src/export/model_doc.rs`): `{ name, description?, culture?, compatibilityLevel?, includesExpressions, includesHidden, summary {…counts}, tables[] { name, description?, kind, hidden, columns[] { name, dataType, kind, hidden, key, description?, sourceColumn?, expression?, formatString?, … }, measures[] { name, expression?, formatString?, description?, displayFolder?, hidden }, partitions[] { name, kind, language?: m|dax|sql, mode, expression?, … }, hierarchies[] }, relationships[] { name, from { table, column }, to { table, column }, fromCardinality, toCardinality, crossFilteringBehavior, active, securityFilteringBehavior }, parameters[] { name, type?, required, description?, value? }, expressions[], roles[] }`. Descriptions are the TMDL `///` comments.
- `--model --format md` renders the same content as a Markdown data dictionary: summary counts, a relationship table, then per table its description, a columns table (Name, Type, Description, Source), measures with their DAX in `dax` code blocks, calculated columns, and the M partition in a `powerquery` code block; then parameters. The title defaults to the file or project name (`--title` overrides it).
- `--no-expressions` leaves out all DAX and M; `--no-hidden` leaves out hidden objects.
- Exported data can be internal: don't commit it.

### `render`

- Refreshes like `refresh` (same `--source`/`--blob-map`), then queries and draws pages.
- `--report` picks the report (exact name or a unique part of it; default: the first one, with a note when there are several).
- `--page` picks the page by internal name, display name (case-insensitive), 1-based index or a unique part of the display name; default: the report's active page. `--all-pages` renders every page.
- `--png FILE` and `--svg FILE` (both allowed) write one page. `--out-dir DIR` writes `<NN>-<page name>.png` (or `.svg`/both with `--format`) per page, `NN` being the 1-based page index.
- `--scale` is PNG pixels per page unit (default 2: a 1280×720 page becomes 2560×1440, like the service's exports; greater than 0 and at most 16).
- PNGs have a white background and use only the bundled Selawik fonts, so the same inputs give byte-identical files on every OS.
- `--now 2026-10-21T21:35:00Z` pins the time M's `DateTime.LocalNow()`/`DateTimeZone.UtcNow()` return (for example a "Last Refresh" table); otherwise the system clock is used.
- `--slicer "SLICER=V1,V2"` selects values in a slicer; `SLICER!=V1` selects everything else, `SLICER=` selects all (clears it), `(Blank)` is BLANK. SLICER is the slicer's visual name or its field (display name, column, or `Table[Column]`), case-insensitive. It replaces the selection saved in the report. Repeatable.
- Existing output files are never overwritten without `--force`. Outputs, pages and slicers are checked before the refresh.
- A visual whose query fails is drawn with ⚠ and reported on stderr; the exit code is then 2 (also when the refresh fails).
- Reports stored as PBIX `Report/Layout` or PBIR-Legacy `report.json` render with the built-in default theme in the CLI (the app resolves their theme files).
- `--json` prints `{ schemaVersion: 1, source, report, refresh { tables, ok, failed, skipped } | null, now?, ok, pages[] { index, name, displayName, width, height, outputs[] { format: png|svg, path, width?, height?, bytes }, durationMs, slicers[] { visual, values[], inverted, condition, origin: report|cli }, summary { visuals, ok, warnings, errors, hidden, truncated }, visuals[] { name, visualType, status: ok|warning|error|hidden, error?, warnings[], truncated, queryMicros, renderMicros } } }`.
- Rendered pages show the data: don't commit them.

### `commands`, `run`, `serve`

These three expose the **command registry** (`pbi-ops`, [ADR-0007](adr/0007-command-registry.md)): every capability as a named command with typed parameters and results described by JSON Schema, the same commands the app and AI agents (MCP) use. The generated catalog, the batch format and the RPC protocol are in [COMMANDS.md](COMMANDS.md).

- `commands` prints the catalog grouped by category (`--agent-safe`: only the commands offered to agents). `commands <id>` describes one command (parameters, result, examples); with `--json` it prints `{ id, title, summary, category, mutatesDocument, longRunning, agentSafe, writesFiles, since, examples[] { title, params }, paramsSchema, outputSchema }`. `commands --json` prints `{ count, commands[] }` (schemas only with `--schemas`). `commands --markdown` prints the content of `docs/COMMANDS.md` (a test fails when the committed file is stale).
- `run <id>` takes parameters as a JSON object (`--params`) and/or `-p key=value` pairs (repeatable, override `--params`; the value is parsed as JSON when it is valid JSON, else used as text; dotted keys nest: `-p sort.column=Name`). Parameters are camelCase and unknown ones are rejected.
- `--doc PATH` opens the file first (`doc.open`, with `--blob-map`) and passes its `docId` to the command if it takes one and none was given; in a batch it is injected into every step that takes a `docId`, and `${doc.docId}` (or any field of the `doc.open` result) is available. `--blob-map` is also the default blob map of `doc.open`.
- Data commands load the document's data on first use (as `refresh --source auto` would); `data.refresh` reloads it, `doc.open` with `"autoRefresh": false` turns the automatic load off.
- `run` prints the result on stdout as compact JSON (`--pretty` to indent). Errors are printed on stderr as `dashcraft: error: [<code>] <message>` (and `dashcraft: details: <json>`).
- Exit codes: 0 success; 1 for errors with codes `invalidParams`, `unknownCommand`, `notFound`, `noDocument`, `unsupported`, `pathDenied`, `io`, `cancelled`, `busy`, `internal`; 2 for `query` errors (DAX, SQL, M or CSV evaluation) and for results with `"ok": false` (failed tables, ⚠ measures, failed visuals, lint findings at the `deny` level).
- `run --batch FILE` (`-` reads stdin) takes `[{ "id", "params"?, "as"? }]` or `{ "steps": [...], "stopOnError"?: bool }`. A string that is exactly `"${name.path}"` becomes the referenced value of the step named `name` (any JSON type); inside a longer string the value's text is spliced in; `$${` is a literal `${`. The batch stops at the first error unless `--stop-on-error false` (or `"stopOnError": false`); it is not atomic. It prints the JSON array `[{ index, id, as?, ok, result?, error? { code, message, data? } }]` and exits with the code of the first error, else 2 if a result has `"ok": false`.
- `serve` reads one JSON-RPC 2.0 request per line on stdin and writes one response or notification per line on stdout (logs go to stderr) until end of input, then waits for running requests. Methods are command ids; `progress` notifications report long-running requests; `$/cancel {"id": …}` cancels one. Requests run concurrently (`--max-concurrent`); mutating commands run in arrival order per document. Errors carry the JSON-RPC code and `data.code` (the string code).
- `serve --read-root DIR` / `--write-root DIR` (repeatable) confine every path the commands read or write to those folders: paths are resolved (relative ones against the first root), symlink and `..` escapes and writes through symlinks are refused (`pathDenied`), and blob maps may only map files inside the read roots. Without roots, paths are unrestricted (as for the other subcommands). `--agent` serves only the agent-safe commands, as an MCP host would.

```sh
$ dashcraft run doc.detect -p path=model.pbip --pretty
$ dashcraft run measures.values --doc model.pbip --blob-map blob-map.json -p report=Overview \
    | jq -r '.measures[] | "\(.name)\t\(.display)"'
$ dashcraft run dax.query --doc model.pbip --blob-map blob-map.json \
    --params '{"query": "EVALUATE ROW(\"n\", [Total Sales])"}'
$ dashcraft run render.page --doc model.pbip -p report=Overview -p format=png -p outputPath=target/tmp/overview.png -p overwrite=true
$ cat steps.json
[{"id": "doc.open", "params": {"path": "model.pbip"}, "as": "d"},
 {"id": "report.pages", "params": {"docId": "${d.docId}", "report": "Overview"}}]
$ dashcraft run --batch steps.json --pretty
$ printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"doc.open","params":{"path":"model.pbip"}}' | dashcraft serve
{"jsonrpc":"2.0","id":1,"result":{"docId":"d1",…}}
```

### `mcp`

A [Model Context Protocol](https://modelcontextprotocol.io/specification) server for AI agents (Claude Desktop, VS Code, GitHub Copilot CLI, …) over stdin/stdout. Full reference, security model, client configuration and a transcript: [MCP.md](MCP.md).

- Tools are the **agent-safe** commands of the catalog, generated from the registry: `doc.open` becomes the tool `doc_open` (`--tool-names dotted` keeps `doc.open`), with the command's parameter and result JSON Schemas and read-only/destructive/idempotent hints from its flags. Calls run as an agent (`CallerKind::Agent`) on one workspace kept for the session.
- Paths are always confined: `--read-root DIR` (repeatable; default the current folder, with a notice on stderr, unless it is the filesystem root) for reads, `--write-root DIR` for writes (without it every write is refused with `pathDenied`). Symlink and `..` escapes are refused; `--blob-map` and the files it maps must be inside the read roots.
- `--allow` / `--deny` take a category (`inspect`, `render`, …), a command id or a tool name, and narrow the tool set; an entry that matches nothing is an error (exit 1).
- Results are the command's JSON as text plus `structuredContent`; inline PNG renders are returned as `image` content; text over 1 MiB is cut with a note. Command failures are tool results with `isError: true` and `{"code", "message"}`; unknown tools are JSON-RPC `-32602`.
- Protocol revisions `2025-11-25`, `2025-06-18`, `2025-03-26`, `2024-11-05` (`initialize`) and the stateless `2026-07-28` (`server/discover`, per-request `_meta`); `ping`, `tools/list` with cursors, `notifications/cancelled`, `notifications/progress` for calls with a `progressToken`, `logging/setLevel`. stdout carries only MCP messages; logs go to stderr. End of input cancels running calls and exits 0.

```sh
$ dashcraft mcp --read-root ~/PowerBI --blob-map ~/PowerBI/blob-map.json
$ dashcraft mcp --read-root . --write-root ./out --allow inspect --allow dax --allow workspace
```

### `ai`

The opt-in AI layer ([AI.md](AI.md), [ADR-0008](adr/0008-ai-layer.md)) is off by default. Its `ai.*` commands (`ai.status`, `ai.preview`, `ai.explainMeasure`, `ai.generateMeasure`, consent and settings commands) are part of the catalog of `run`, `serve` and `commands`; `mcp` offers only the agent-safe ones (never consent, settings or keys). `commands --markdown` prints the core catalog only (docs/COMMANDS.md); the AI commands are documented in AI.md.

- `dashcraft ai login --provider <id>` prompts for the key without echo; `--stdin` reads the first line of stdin instead (scripts). The key is never accepted as an argument.
- `dashcraft ai logout --provider <id>` deletes the stored key (`DASHCRAFT_AI_API_KEY` is not affected).

```sh
$ dashcraft run ai.status
$ dashcraft run ai.preview --doc model.pbip -p capability=dax.explain -p "measure=Total Sales"
$ dashcraft ai login --provider openai
```

### `open`

The app is looked up in this order:

1. `$DASHCRAFT_APP`;
2. next to the `dashcraft` executable;
3. on macOS, in a `DashCraft.app` bundle next to it, in `/Applications` or in `~/Applications`;
4. on `PATH`.

The path is passed to the app as a single argument, never through a shell.

## Examples

The examples use the synthetic Acme Outfitters example in this repository
(`reference/examples/acme-outfitters/`: a PBIP with three reports, CSV data and a blob map) or a
generic `model.pbip`. Output is abbreviated (`…`); the example's expected values are listed in
[EXPECTED.md](../reference/examples/acme-outfitters/EXPECTED.md).

```text
$ dashcraft inspect reference/examples/acme-outfitters/AcmeOutfitters.pbip
reference/examples/acme-outfitters/AcmeOutfitters.pbip
  Format: PBIP project (pbip), confidence certain
  Signals:
    - `$schema` https://developer.microsoft.com/json-schemas/fabric/pbip/pbipProperties/1.0.0/schema.json
    - report `Acme Outfitters - Online.Report` (PBIR; byPath `../AcmeOutfitters.SemanticModel`)
    …
Model: Model (TMDL in AcmeOutfitters.SemanticModel, culture en-US, compatibility level 1601)
  Table: Orders (table)
    One row per order line (synthetic). …
    Columns (15):
      Order ID : string, key — Order number (SO-…). Unique after Table.Distinct.
      …
      Is Returned : string, calculated = IF(ISEMPTY(RELATEDTABLE(Returns)), "No", "Yes") — …
    Measures (20):
      Total Sales = SUM(Orders[Line Amount])  [format \$#,0]
      Margin % = DIVIDE([Gross Margin], [Total Sales])  [format 0.0%]
      …
  …
  Relationships (8):
    Orders[Customer ID] (many) → Customers[Customer ID] (one), cross-filter bothDirections
    …
    Targets[Region] (many) → Stores[Region] (many), cross-filter oneDirection
  …
Reports (3):
  Report: Acme Outfitters - Overview (model byPath ../AcmeOutfitters.SemanticModel)
    Report filters (1):
      Orders[Status] (Categorical): NOT Orders[Status] IN ('Cancelled')
    Page: Executive Summary [ef796b39c816a4afa4f1] (1280x720, FitToPage, 14 visuals)
      …
      cardVisual at (24, 152) 1000x112 [f7a9c9413fef68532313]
        Data: Orders[Total Sales]
        Data: Orders[Order Count]
        …

Diagnostics: errors 0, warnings 0, infos 2
  …

Totals: tables 11, columns 81, measures 33, relationships 8, reports 3, pages 8, visuals 80
```

```sh
$ dashcraft inspect --json reference/examples/acme-outfitters/AcmeOutfitters.pbip | jq -c .counts
{"tables":11,"columns":81,"calculatedColumns":11,"calculatedTables":2,"measures":33,"relationships":8,"expressions":3,"parameters":1,"reports":3,"pages":8,"visuals":80,"filters":12,"errors":0,"warnings":0,"infos":2}

$ dashcraft inspect --json --no-expressions model.pbix > model-outline.json   # safe to share: no DAX/M
$ dashcraft convert reference/examples/acme-outfitters/AcmeOutfitters.pbip --to pbit \
    --out out/AcmeOutfitters.pbit --report "Acme Outfitters - Stores"
$ dashcraft convert report.pbix --to pbip --out ./report-project
$ dashcraft open reference/examples/acme-outfitters/AcmeOutfitters.pbip
```

```text
$ dashcraft lint reference/examples/acme-outfitters/AcmeOutfitters.pbip
reference/examples/acme-outfitters/AcmeOutfitters.pbip: no problems found

$ dashcraft lint model.pbip
Sales.Report/definition/pages/<page>/visuals/<visual>/visual.json
  warning LINT003 #/visual/query/queryState/Data/projections/0/queryRef cardVisual `<visual>` (Data): queryRef `Orders.Total Sale` does not match the field `Orders.Total Sales` (stale queryRef)
      hint: set queryRef to `Orders.Total Sales`
…
```

```text
$ dashcraft refresh reference/examples/acme-outfitters/AcmeOutfitters.pbip \
    --blob-map reference/examples/acme-outfitters/blob-map.json
TABLE                    SOURCE            ROWS       TIME  STATUS
Orders                   m                 3900      82 ms  ok
  Order Size             column                      20 ms  ok
  …
Customers                m                  400      26 ms  ok
…
Web Sessions             m                 1600      16 ms  ok
Date                     calculated         731      34 ms  ok
  Day Type               column                       2 ms  ok
Time Calc                calculated           4       0 ms  ok
Products Never Sold      calculated           6       2 ms  ok
Last Refreshed           m                    1       0 ms  ok

11 tables in 474 ms: 11 ok, 0 failed, 0 skipped

$ dashcraft refresh reference/examples/acme-outfitters/AcmeOutfitters.pbip      # no blob map: exit 2
TABLE                    SOURCE            ROWS       TIME  STATUS
Orders                   m                    0       1 ms  ERROR: blob storage: offline: no blob mapping configured for …
    hint: offline: pass --blob-map FILE (blob URLs → local files), or load a CSV in the app
…

$ dashcraft measures reference/examples/acme-outfitters/AcmeOutfitters.pbip \
    --blob-map reference/examples/acme-outfitters/blob-map.json --report Overview
Report: Acme Outfitters - Overview
Filter: Orders[Status] NOT IN ("Cancelled")

Orders
  Total Sales                …
  …
  Order Count                3,629
  …
```

```sh
$ dashcraft export reference/examples/acme-outfitters/AcmeOutfitters.pbip --table Orders --out orders.csv \
    --blob-map reference/examples/acme-outfitters/blob-map.json
dashcraft: wrote 3900 rows × 15 columns of 'Orders' to orders.csv
$ dashcraft export model.pbip --table Orders --out orders-excel.csv --formatted --bom --excel-safe --blob-map blob-map.json
$ dashcraft export model.pbip --table Returns --format parquet --out returns.parquet --blob-map blob-map.json
$ dashcraft export model.pbip --model --format md --out - > DATA-DICTIONARY.md
$ dashcraft refresh saved-in-desktop.pbip                  # has .pbi/cache.abf: loads it, no blob map needed
$ dashcraft refresh saved-in-desktop.pbip --source m --blob-map blob-map.json   # run the queries instead
```

```text
$ dashcraft render reference/examples/acme-outfitters/AcmeOutfitters.pbip \
    --blob-map reference/examples/acme-outfitters/blob-map.json \
    --report Overview --png target/tmp/overview.png --now 2026-01-01T00:00:00Z
Executive Summary: target/tmp/overview.png (2560x1440, 14 visuals, 0 errors)

$ dashcraft render reference/examples/acme-outfitters/AcmeOutfitters.pbip \
    --blob-map reference/examples/acme-outfitters/blob-map.json \
    --report Stores --all-pages --out-dir target/tmp/stores --format both
Store Performance: target/tmp/stores/01-Store-Performance.png (2560x1440, 14 visuals, 0 errors)
Store Performance: target/tmp/stores/01-Store-Performance.svg (14 visuals, 0 errors)
Store Detail: target/tmp/stores/02-Store-Detail.png (2560x1800, 5 visuals, 0 errors)
…
```

```sh
$ dashcraft render model.pbip --blob-map blob-map.json --svg target/tmp/west.svg \
    --slicer "Region=West" --slicer "Year=2024"
$ dashcraft render model.pbip --blob-map blob-map.json --png target/tmp/page.png --page 2 --scale 1 --json \
    | jq -r '.pages[].visuals[] | select(.status != "ok") | "\(.name) \(.status) \(.error // "")"'
```

## `inspect --json` schema (`schemaVersion: 1`)

The schema is defined in `crates/pbi-ops/src/inspect_model.rs` (shared with the `doc.inspect` command; the CLI re-exports it). It's separate from the internal model types. Within version 1, fields are only ever added. Renaming, removing or retyping a field bumps `schemaVersion`.

- Arrays and booleans are always present.
- Optional fields (marked `?` below) are omitted when absent.
- `model` is `null` when no model is available locally.
- Enumerations use the TMDL/TMSL/PBIR spellings.
- `source.path` is the path as given on the command line. Every other path is relative to the opened folder or archive.

```text
schemaVersion: 1
source        { path, format, detection { format, label, confidence, signals[] }, packageVersion? }
modelStatus   { status: local|remote|missing, description? }
model         { name, description?, culture?, compatibilityLevel?, source? { kind: tmdl|tmsl|abf, folder },
                tables[] { name, description?, isHidden, kind: regular|calculatedTable|calculationGroup|remote, dataCategory?,
                           columns[] { name, dataType, kind: data|calculated|calculatedTableColumn|rowNumber, isHidden, isKey,
                                       description?, sourceColumn?, expression?, formatString?, summarizeBy?,
                                       sortByColumn?, displayFolder?, dataCategory? },
                           measures[] { name, expression?, formatString?, description?, isHidden, displayFolder? },
                           partitions[] { name, kind: m|calculated|entity|query|…, mode, expression?, queryGroup?,
                                          entityName?, expressionSource? },
                           hierarchies[] { name, isHidden, description?, levels[] { name, column } } },
                relationships[] { name, from { table, column }, to { table, column }, fromCardinality, toCardinality,
                                  crossFilteringBehavior, isActive, securityFilteringBehavior },
                expressions[] { name, kind: m|dax, isParameter, parameterType?, description?, queryGroup?, expression? },
                roles[] { name, description?, modelPermission?, tablePermissions[] { table, filterExpression? } },
                cacheAbf? { path, size, tableCount, tables[], matchesDefinition?, metadataFormat?, compatibilityLevel? } } | null
extraModels[] (same shape as model: other models of a multi-model project)
reports[]     { name, datasetReference { kind: byPath|byConnection|embedded|none, path?, connectionString?, modelId?,
                                         databaseName? },
                baseTheme?, customTheme?, activePage?, filters[],
                pages[] { name, displayName, width, height, displayOption, isHidden, filters[],
                          visuals[] { name, visualType, isGroup, displayName?, isHidden, parentGroupName?,
                                      position { x, y, z, width, height, tabOrder? },
                                      fields[] { role, kind, table?, name?, aggregation?, hierarchy?, ref, queryRef,
                                                 nativeQueryRef?, displayName? },
                                      filters[] } } }
filters[]     { name, displayName?, kind: Categorical|Advanced|TopN|RelativeDate|…,
                field? { kind: column|measure|aggregation|hierarchyLevel|literal|unknown, table?, name?, aggregation?,
                         hierarchy?, ref },
                condition?, isHiddenInViewMode, isLockedInViewMode, howCreated? }
diagnostics[] { severity: error|warning|info, code, message, origin? { path, line?, pointer? } }
counts        { tables, columns, calculatedColumns, calculatedTables, measures, relationships, expressions, parameters,
                reports, pages, visuals, filters, errors, warnings, infos }   (totals over model + extraModels + reports)
```

Additive sections for the formats in [FORMATS.md §5](../FORMATS.md#inspect-output-for-5-formats): `vpaxStats`, `paginatedReport`, `mashup`, `customVisual`, `connector`.

Notes:

- `ref` is a DAX-style reference, for example `'Order Lines'[Quantity]` or `CountNonNull(Orders[Status])`.
- `condition` is a one-line summary, for example `Stores[Region] IN ('West')`. It's omitted when the filter card isn't filtering.
- `--no-expressions` removes every `expression` and `filterExpression` field (and, for §5 formats, query text and textbox values).

`dashcraft detect --json` prints `{ schemaVersion, path, sourceFormat, detection {…as above}, items { pbipFile?, semanticModels[] { path, flavour, hasCacheAbf, displayName? }, reports[] { path, flavour, dataset, datasetPath?, displayName? }, otherItems[] { path, itemType } } }`.
