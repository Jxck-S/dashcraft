#!/usr/bin/env python3
"""Generate the ground-truth query corpora (reference/groundtruth/corpus/*.json; ADR-0011).

Each corpus is a list of DAX queries that the recorder (Record-GroundTruth.ps1) runs against
Power BI Desktop and that `crates/pbi-session/tests/groundtruth.rs` runs against DashCraft. The
inputs are committed files only: the Acme Outfitters model (TMDL), its `expected.json` (report,
page and slicer filters), the DAX function cases of `crates/pbi-dax/tests/functions.rs` and the
function catalog of the DAX language service (`crates/pbi-dax/src/service/catalog.txt`; the
`dax-arity` suite, checked by `crates/pbi-dax/tests/language_service.rs`, not by groundtruth.rs).

Case ids are stable: they are derived from the case content, never from list positions, so
adding a case does not invalidate the other recordings. A recording is stale when its query
text differs from the corpus text.

Usage: python scripts/groundtruth/gen_corpus.py [--check]
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ACME = ROOT / "reference" / "examples" / "acme-outfitters"
ACME_DEF = ACME / "AcmeOutfitters.SemanticModel" / "definition"
FUNCTIONS_RS = ROOT / "crates" / "pbi-dax" / "tests" / "functions.rs"
CATALOG_TXT = ROOT / "crates" / "pbi-dax" / "src" / "service" / "catalog.txt"
OUT = ROOT / "reference" / "groundtruth" / "corpus"

ACME_TABLES = [
    "Orders",
    "Customers",
    "Products",
    "Stores",
    "Targets",
    "Returns",
    "Web Sessions",
    "Date",
    "Time Calc",
    "Products Never Sold",
    "Last Refreshed",
]
DAX_LAB_TABLES = ["Product", "Sales", "Date", "Orders", "Emp"]

GROUP_BY = [
    ("year", "'Date'[Year]"),
    ("year-month", "'Date'[Year Month]"),
    ("day-type", "'Date'[Day Type]"),
    ("region", "'Stores'[Region]"),
    ("channel", "'Stores'[Channel]"),
    ("store", "'Stores'[Store Name]"),
    ("category", "'Products'[Category]"),
    ("price-band", "'Products'[Price Band]"),
    ("segment", "'Customers'[Segment]"),
    ("loyalty-tier", "'Customers'[Loyalty Tier]"),
    ("value-tier", "'Customers'[Value Tier]"),
    ("device", "'Web Sessions'[Device]"),
    ("status", "'Orders'[Status]"),
    ("order-size", "'Orders'[Order Size]"),
]

def desktop_role(role: str, user: str) -> str:
    """Desktop's local engine refuses an EffectiveUserName that is not a Windows account, so the
    Desktop variant carries a static copy of each dynamic role with USERPRINCIPALNAME() replaced by
    the user (make_desktop_variant.py uses the same name); the recorder connects with that role."""
    return f"GT {role} {user.split('@')[0]}"


def role_is_dynamic(role: str) -> bool:
    return "USERPRINCIPALNAME()" in (ACME_DEF / "roles" / f"{role}.tmdl").read_text(encoding="utf-8")


def slug(s: str) -> str:
    return re.sub(r"[^a-z0-9]+", "-", s.lower()).strip("-")


def short_hash(s: str) -> str:
    return hashlib.sha1(s.encode("utf-8")).hexdigest()[:8]


def dax_string(s: str) -> str:
    return '"' + s.replace('"', '""') + '"'


def measure_ref(name: str) -> str:
    return "[" + name.replace("]", "]]") + "]"


def table_ref(name: str) -> str:
    return "'" + name.replace("'", "''") + "'"


# --- Acme model --------------------------------------------------------------------------------


def tmdl_name(raw: str) -> str:
    raw = raw.strip()
    if raw.startswith("'"):
        return raw[1:-1].replace("''", "'")
    return raw


def acme_measures() -> list[tuple[str, str]]:
    """(table, measure) in model order."""
    model = (ACME_DEF / "model.tmdl").read_text(encoding="utf-8")
    order = [tmdl_name(m) for m in re.findall(r"^ref table (.+)$", model, re.M)]
    out = []
    for table in order:
        text = (ACME_DEF / "tables" / f"{table}.tmdl").read_text(encoding="utf-8")
        for m in re.finditer(r"^\tmeasure ('(?:[^']|'')+'|[^=\s]+)\s*=", text, re.M):
            out.append((table, tmdl_name(m.group(1))))
    return out


def acme_measure_formats() -> dict[str, str]:
    """Static format string per measure (measures with a dynamic one are left out)."""
    out = {}
    for f in sorted((ACME_DEF / "tables").glob("*.tmdl")):
        blocks = re.split(r"^(?=\t(?:measure|column|partition|calculationGroup) )", f.read_text(encoding="utf-8"), flags=re.M)
        for b in blocks:
            m = re.match(r"\tmeasure ('(?:[^']|'')+'|[^=\s]+)", b)
            fs = re.search(r"^\t\tformatString: (.+)$", b, re.M)
            if m and fs and "formatStringDefinition" not in b:
                out[tmdl_name(m.group(1))] = fs.group(1).strip()
    return out


def acme_format_strings() -> list[str]:
    seen: dict[str, None] = {}
    for f in sorted((ACME_DEF / "tables").glob("*.tmdl")):
        for m in re.finditer(r"^\t+formatString: (.+)$", f.read_text(encoding="utf-8"), re.M):
            seen.setdefault(m.group(1).strip(), None)
    return list(seen)


FILTER_RE = re.compile(r"^('(?:[^']|'')+'\[[^\]]+\]) (.*)$")


def dax_literal(v: str) -> str:
    v = v.strip()
    if v.startswith("'") and v.endswith("'"):
        return dax_string(v[1:-1])
    m = re.fullmatch(r"(\d{4})-(\d{2})-(\d{2})", v)
    if m:
        return f"DATE({int(m.group(1))}, {int(m.group(2))}, {int(m.group(3))})"
    if re.fullmatch(r"-?\d+(\.\d+)?", v):
        return v
    raise SystemExit(f"unsupported filter literal: {v}")


def dax_filter(expr: str) -> str | None:
    """expected.json filter text -> a CALCULATE boolean filter; None for relative dates.

    Relative date filters ("in the last 10 years") depend on today's date; on the example's
    2024-2025 data they keep every row, so they are left out.
    """
    m = FILTER_RE.match(expr)
    if not m:
        raise SystemExit(f"unsupported filter: {expr}")
    col, rest = m.groups()
    if "relative date" in rest:
        return None
    m = re.fullmatch(r"(not )?in \[(.*)\]", rest)
    if m:
        values = [dax_literal(v) for v in re.findall(r"'(?:[^']|'')*'|[^,\s]+", m.group(2))]
        cond = f"{col} IN {{{', '.join(values)}}}"
        return f"NOT ({cond})" if m.group(1) else cond
    m = re.fullmatch(r"(>=|>|<=|<) (\S+) and (>=|>|<=|<) (\S+)", rest)
    if m:
        return f"{col} {m.group(1)} {dax_literal(m.group(2))} && {col} {m.group(3)} {dax_literal(m.group(4))}"
    raise SystemExit(f"unsupported filter: {expr}")


def with_filters(table_expr: str, filters: list[str]) -> str:
    dax = [f for f in (dax_filter(x) for x in filters) if f]
    if not dax:
        return table_expr
    return "CALCULATETABLE(\n    " + table_expr + ",\n    " + ",\n    ".join(dax) + "\n)"


def suite(name: str, title: str, fixture: str, tables: list[str], cases: list[dict]) -> dict:
    ids = [c["id"] for c in cases]
    dupes = {i for i in ids if ids.count(i) > 1}
    if dupes:
        raise SystemExit(f"{name}: duplicate case ids {sorted(dupes)}")
    return {
        "schemaVersion": 1,
        "suite": name,
        "title": title,
        "fixture": fixture,
        "requiresTables": tables,
        "cases": cases,
    }


def acme_tables_suite() -> dict:
    cases = []
    for t in ACME_TABLES:
        case = {"id": slug(t), "query": f"EVALUATE {table_ref(t)}"}
        if t == "Last Refreshed":
            case["volatile"] = "refresh time"
        cases.append(case)
    return suite(
        "acme-tables",
        "Every Acme Outfitters table after refresh (M, types, calculated columns and tables)",
        "acme",
        ACME_TABLES,
        cases,
    )


def acme_measures_suite(expected: dict) -> dict:
    measures = acme_measures()
    cases = []
    contexts = [("all", [])] + [
        (slug(r["report"].split(" - ")[-1]), r["reportFilters"]) for r in expected["reports"]
    ]
    for _, m in measures:
        for ctx, filters in contexts:
            cases.append(
                {
                    "id": f"{slug(m)}/{ctx}",
                    "query": "EVALUATE " + with_filters(f'ROW("v", {measure_ref(m)})', filters),
                }
            )
    for _, m in measures:
        for key, col in GROUP_BY:
            cases.append(
                {
                    "id": f"{slug(m)}/by-{key}",
                    "query": f'EVALUATE SUMMARIZECOLUMNS({col}, "v", {measure_ref(m)})',
                }
            )
    # Every card and visual measure of every page, under the report, page and slicer filters.
    for r in expected["reports"]:
        rep = slug(r["report"].split(" - ")[-1])
        for p in r["pages"]:
            filters = r["reportFilters"] + p["pageFilters"] + [s["selection"] for s in p["slicers"]]
            names: dict[str, None] = {}
            for c in p["cards"]:
                if "calculationItem" not in c:
                    names.setdefault(c["measure"], None)
            if names:
                cols = ", ".join(f"{dax_string(n)}, {measure_ref(n)}" for n in names)
                cases.append(
                    {
                        "id": f"page/{rep}/{slug(p['page'])}/cards",
                        "query": "EVALUATE " + with_filters(f"ROW({cols})", filters),
                    }
                )
    return suite(
        "acme-measures",
        "Every Acme Outfitters measure unfiltered, under each report's filters, grouped by the "
        "main dimensions, and every page's cards under the report, page and slicer filters",
        "acme",
        ACME_TABLES,
        cases,
    )


def acme_visuals_suite(expected: dict) -> dict:
    """The DAX that matches each asserted visual of expected.json (grouped values and totals)."""
    cases = []
    overview = expected["reports"][0]["reportFilters"]
    stores = next(r for r in expected["reports"] if r["report"].endswith("Stores"))["reportFilters"]
    online = next(r for r in expected["reports"] if r["report"].endswith("Online"))["reportFilters"]
    product_slicers = ["'Products'[Category] in ['Camping', 'Hiking']", "'Products'[List Price] >= 50 and <= 400"]
    cases.append(
        {
            "id": "overview/product-detail/matrix",
            "query": "EVALUATE "
            + with_filters(
                "SUMMARIZECOLUMNS(ROLLUPADDISSUBTOTAL('Products'[Category], \"IsTotal\"), "
                '"Total Sales", [Total Sales], "Units Sold", [Units Sold], "Margin %", [Margin %])',
                overview + product_slicers,
            ),
        }
    )
    cases.append(
        {
            "id": "overview/product-detail/top10",
            "query": "EVALUATE "
            + with_filters(
                'TOPN(10, SUMMARIZECOLUMNS(\'Products\'[Product Name], "Total Sales", [Total Sales], '
                '"Units Sold", [Units Sold], "Margin %", [Margin %]), [Total Sales], DESC)',
                overview + product_slicers,
            ),
        }
    )
    cases.append(
        {
            "id": "overview/executive-summary/kpi-trend",
            "query": "EVALUATE "
            + with_filters(
                "SUMMARIZECOLUMNS('Date'[Year Month], \"Total Sales\", [Total Sales], \"Target\", [Target])",
                overview + ["'Date'[Year] in [2025]"],
            ),
        }
    )
    store_slicers = ["'Stores'[Region] in ['West']", "'Date'[Date] >= 2025-01-01 and < 2026-01-01"]
    cases.append(
        {
            "id": "stores/store-performance/stores",
            "query": "EVALUATE "
            + with_filters(
                "SUMMARIZECOLUMNS(ROLLUPADDISSUBTOTAL('Stores'[Store Name], \"IsTotal\"), \"Total Sales\", [Total Sales], "
                '"Target", [Target], "Target Attainment %", [Target Attainment %], "Target Stretch", [Target Stretch])',
                stores + store_slicers,
            ),
        }
    )
    cases.append(
        {
            "id": "stores/store-detail/stores",
            "query": "EVALUATE "
            + with_filters(
                "SUMMARIZECOLUMNS('Stores'[Store Name], 'Stores'[Store Manager], 'Stores'[City], "
                '"Total Sales", [Total Sales])',
                stores + ["'Stores'[Region] in ['East']"],
            ),
        }
    )
    cases.append(
        {
            "id": "stores/store-detail/returns",
            "query": "EVALUATE "
            + with_filters(
                "SUMMARIZECOLUMNS(ROLLUPADDISSUBTOTAL('Returns'[Reason], \"IsTotal\"), "
                '"Returns Count", [Returns Count], "Refunds", [Refunds])',
                stores + ["'Stores'[Region] in ['East']"],
            ),
        }
    )
    cases.append(
        {
            "id": "online/online-overview/devices",
            "query": "EVALUATE "
            + with_filters(
                "SUMMARIZECOLUMNS('Web Sessions'[Device], \"Sessions\", [Sessions])",
                online + ["'Customers'[Loyalty Tier] not in ['Unassigned']"],
            ),
        }
    )
    cases.append(
        {
            "id": "online/customer-detail/table",
            "query": "EVALUATE "
            + with_filters(
                "SUMMARIZECOLUMNS('Customers'[Customer Name], \"Total Sales\", [Total Sales])",
                online + ["'Customers'[Segment] in ['Corporate']"],
            ),
        }
    )
    cases.append(
        {
            "id": "online/customer-detail/matrix",
            "query": "EVALUATE "
            + with_filters(
                "SUMMARIZECOLUMNS(ROLLUPADDISSUBTOTAL('Customers'[Loyalty Tier], \"IsTotal\"), "
                '"Total Sales", [Total Sales])',
                online + ["'Customers'[Segment] in ['Corporate']"],
            ),
        }
    )
    return suite(
        "acme-visuals",
        "The grouped values behind the asserted Acme Outfitters visuals (expected.json), as DAX",
        "acme",
        ACME_TABLES,
        cases,
    )


def acme_calcgroup_rls_suite(expected: dict) -> dict:
    cg = expected["calculationGroup"]
    item_col = f"{table_ref(cg['table'])}[{cg['column']}]"
    overview = expected["reports"][0]["reportFilters"]
    month = f"'Date'[Year Month] in ['{cg['month']}']"
    cases = []
    for m in ["Total Sales", "Order Count", "Margin %", "Units Sold"]:
        cases.append(
            {
                "id": f"calc-group/{slug(m)}/month",
                "query": "EVALUATE "
                + with_filters(f'SUMMARIZECOLUMNS({item_col}, "v", {measure_ref(m)})', overview + [month]),
            }
        )
        cases.append(
            {
                "id": f"calc-group/{slug(m)}/by-year",
                "query": "EVALUATE "
                + with_filters(
                    f"SUMMARIZECOLUMNS('Date'[Year], {item_col}, \"v\", {measure_ref(m)})",
                    overview,
                ),
            }
        )
    cases.append(
        {
            "id": "calc-group/total-sales/by-year-month",
            "query": f"EVALUATE SUMMARIZECOLUMNS('Date'[Year Month], {item_col}, \"v\", [Total Sales])",
        }
    )
    cases.append(
        {
            "id": "calc-group/items",
            "query": f"EVALUATE {table_ref(cg['table'])}",
        }
    )
    rls_measures = ["Total Sales", "Order Count", "Target", "Returns Count", "Sessions", "Customer Count"]
    for spec in expected["rls"]:
        roles = spec["roles"]
        view_as: dict = {"roles": roles}
        desktop_roles = roles
        key = slug("-".join(roles))
        if spec.get("userName"):
            view_as["userName"] = spec["userName"]
            desktop_roles = [desktop_role(r, spec["userName"]) if role_is_dynamic(r) else r for r in roles]
            key += "/" + slug(spec["userName"].split("@")[0])
        counts = ", ".join(f"{dax_string(t)}, COUNTROWS({table_ref(t)})" for t in ACME_TABLES)
        base = {"viewAs": view_as}
        if desktop_roles != roles:
            base["desktopRoles"] = desktop_roles
        cases.append({"id": f"rls/{key}/visible-rows", "query": f"EVALUATE ROW({counts})", **base})
        cols = ", ".join(f"{dax_string(m)}, {measure_ref(m)}" for m in rls_measures)
        cases.append({"id": f"rls/{key}/measures", "query": f"EVALUATE ROW({cols})", **base})
        cases.append(
            {
                "id": f"rls/{key}/by-store",
                "query": "EVALUATE SUMMARIZECOLUMNS('Stores'[Store Name], \"Total Sales\", [Total Sales], \"Target\", [Target])",
                **base,
            }
        )
        cases.append({"id": f"rls/{key}/stores", "query": "EVALUATE 'Stores'", **base})
    return suite(
        "acme-calcgroup-rls",
        "Acme Outfitters calculation-group items by month and year, and the two roles "
        "(static Region West; dynamic Store Manager for one user)",
        "acme",
        ACME_TABLES,
        cases,
    )


def metadata_queries() -> dict[str, str]:
    queries = {
        "tables": "EVALUATE SELECTCOLUMNS(INFO.VIEW.TABLES(), \"Name\", [Name], \"DataCategory\", [DataCategory], "
        '"Description", [Description], "IsHidden", [IsHidden], "CalculationGroupPrecedence", [CalculationGroupPrecedence])',
        "columns": "EVALUATE SELECTCOLUMNS(FILTER(INFO.VIEW.COLUMNS(), [Type] <> \"RowNumber\"), \"Table\", [Table], \"Name\", [Name], "
        '"DataType", [DataType], "IsHidden", [IsHidden], "FormatString", [FormatString], '
        '"SortByColumn", [SortByColumn], "SummarizeBy", [SummarizeBy], "DataCategory", [DataCategory], '
        '"DisplayFolder", [DisplayFolder], "Description", [Description], "Expression", [Expression])',
        "measures": "EVALUATE SELECTCOLUMNS(INFO.VIEW.MEASURES(), \"Table\", [Table], \"Name\", [Name], "
        '"IsHidden", [IsHidden], "DisplayFolder", [DisplayFolder], "Description", [Description], '
        '"Expression", [Expression])',
        # INFO.VIEW.MEASURES leaves FormatString empty; the static one is in INFO.MEASURES.
        "measure-formats": 'EVALUATE SELECTCOLUMNS(INFO.MEASURES(), "Name", [Name], "FormatString", [FormatString])',
        "relationships": "EVALUATE SELECTCOLUMNS(INFO.VIEW.RELATIONSHIPS(), \"FromTable\", [FromTable], "
        '"FromColumn", [FromColumn], "FromCardinality", [FromCardinality], "ToTable", [ToTable], '
        '"ToColumn", [ToColumn], "ToCardinality", [ToCardinality], "IsActive", [IsActive], '
        '"CrossFilteringBehavior", [CrossFilteringBehavior], "SecurityFilteringBehavior", [SecurityFilteringBehavior])',
    }
    return queries


def metadata_cases() -> list[dict]:
    return [{"id": k, "query": q, "metadata": k} for k, q in metadata_queries().items()]


def acme_metadata_suite() -> dict:
    return suite(
        "acme-metadata",
        "The Acme Outfitters model as Desktop sees it (INFO.VIEW functions) vs the model DashCraft reads",
        "acme",
        ACME_TABLES,
        metadata_cases(),
    )


# --- Microsoft samples (MIT) -------------------------------------------------------------------

# The committed PBIX samples (reference/samples/microsoft, MIT). The PBIT is left out: opening a
# template makes Desktop refresh it from the web.
SAMPLES = [
    ("revenue-opportunities", "Revenue Opportunities.pbix"),
    ("corporate-spend", "Corporate Spend.pbix"),
    ("regional-sales", "Regional Sales Sample.pbix"),
]
SAMPLE_DUMP_MAX_ROWS = 500


def sample_model(file: str) -> dict:
    """The sample's model as DashCraft reads it (`dashcraft inspect --json`): the corpus lists its
    tables, columns and measures; Desktop's own metadata is checked by the metadata cases."""
    out = subprocess.run(
        ["cargo", "run", "-q", "-p", "dashcraft-cli", "--bin", "dashcraft", "--", "inspect",
         str(ROOT / "reference" / "samples" / "microsoft" / file), "--json"],
        cwd=ROOT, check=True, capture_output=True,
    )
    return json.loads(out.stdout)["model"]


def sample_volatile(model: dict) -> set[str]:
    """Columns, calculated tables and measures (`table`, `table\\0column`) whose values depend on the
    clock: the expression calls NOW/TODAY/… or refers to something volatile, to a fixpoint. Desktop
    recalculates them when it opens the file, DashCraft reads the stored values, so they are
    compared for shape only."""
    exprs: dict[str, str] = {}
    names: dict[str, str] = {}
    for t in model["tables"]:
        for p in t.get("partitions", []):
            if p.get("kind") == "calculated":
                exprs[t["name"]] = p.get("expression", "")
        for c in t.get("columns", []):
            key = t["name"] + "\0" + c["name"]
            exprs[key] = c.get("expression", "")
            names[key] = c["name"]
        for m in t.get("measures", []):
            key = t["name"] + "\0" + m["name"]
            exprs[key] = m.get("expression", "")
            names[key] = m["name"]
    volatile = {k for k, e in exprs.items() if VOLATILE_FUNCTIONS.search(e or "")}
    while True:
        refs = [re.escape("[" + names[k].replace("]", "]]") + "]") for k in volatile if k in names]
        refs += [re.escape(k) for k in volatile if k not in names]
        pattern = re.compile("|".join(refs), re.I) if refs else None
        grown = set(volatile)
        if pattern:
            grown |= {k for k, e in exprs.items() if e and pattern.search(e)}
        grown |= {k for k in names if k.split("\0", 1)[0] in volatile}
        if grown == volatile:
            return volatile
        volatile = grown


def sample_suite(sample_id: str, file: str, rows: dict[str, int]) -> dict:
    model = sample_model(file)
    volatile = sample_volatile(model)
    reason = "depends on the clock (Desktop recalculates on open; DashCraft reads the stored values)"
    cases = metadata_cases()
    tables = []
    for t in model["tables"]:
        name = t["name"]
        tables.append(name)
        ref = table_ref(name)
        cases.append({"id": f"rows/{slug(name)}", "query": f'EVALUATE ROW("rows", COUNTROWS({ref}))'})
        columns = [c for c in t.get("columns", []) if c.get("kind") != "rowNumber"]
        for c in columns:
            dtype = c.get("dataType", "")
            if dtype in ("binary", "unknown", "variant"):
                continue
            col = f"{ref}[{c['name'].replace(']', ']]')}]"
            parts = [f'"distinct", DISTINCTCOUNT({col})', f'"blanks", COUNTBLANK({col})']
            if dtype != "boolean":
                parts += [f'"min", MIN({col})', f'"max", MAX({col})']
            if dtype in ("int64", "double", "decimal"):
                parts.append(f'"sum", SUM({col})')
            case = {
                "id": f"column/{slug(name)}/{slug(c['name'])}-{short_hash(name + chr(0) + c['name'])}",
                "query": f"EVALUATE ROW({', '.join(parts)})",
            }
            if name + "\0" + c["name"] in volatile:
                case["volatile"] = reason
            cases.append(case)
        dumpable = all(c.get("dataType") not in ("binary", "unknown", "variant") for c in columns)
        if dumpable and rows.get(name, SAMPLE_DUMP_MAX_ROWS + 1) <= SAMPLE_DUMP_MAX_ROWS:
            case = {"id": f"table/{slug(name)}", "query": f"EVALUATE {ref}"}
            if name in volatile or any(name + "\0" + c["name"] in volatile for c in columns):
                case["volatile"] = reason
            cases.append(case)
        for m in t.get("measures", []):
            case = {
                "id": f"measure/{slug(m['name'])}-{short_hash(name + chr(0) + m['name'])}",
                "query": f'EVALUATE ROW("v", {measure_ref(m["name"])})',
            }
            if name + "\0" + m["name"] in volatile:
                case["volatile"] = reason
            cases.append(case)
    return suite(
        f"samples-{sample_id}",
        f"Microsoft sample {file} (MIT): metadata, row counts, column aggregates, small tables and "
        "every measure, Desktop's engine vs DashCraft's VertiPaq decoder and DAX engine",
        f"sample:{file}",
        tables,
        cases,
    )


def sample_suites() -> list[dict]:
    expected = json.loads((ROOT / "reference" / "samples" / "microsoft" / "expected.json").read_text(encoding="utf-8"))
    rows = {
        f["file"].rsplit("/", 1)[-1]: {t["name"]: t["rows"] for t in f.get("model", {}).get("tables", []) if t.get("rows") is not None}
        for f in expected["files"]
    }
    return [sample_suite(i, f, rows.get(f, {})) for i, f in SAMPLES]


# --- FORMAT matrix -----------------------------------------------------------------------------

FORMAT_VALUES = [
    ("int", "1234"),
    ("neg", "-1234.5678"),
    ("frac", "0.125"),
    ("zero", "0"),
    ("big", "1234567890.5"),
    ("small", "0.000456"),
    ("half", "2.5"),
    ("currency", "CURRENCY(1234.5)"),
    ("blank", "BLANK()"),
    ("date", "DATE(2025, 3, 7)"),
    ("datetime", "DATE(2025, 3, 7) + TIME(14, 5, 9)"),
    ("dec31", "DATE(2024, 12, 31)"),
    ("text", '"abc"'),
    ("bool", "TRUE()"),
]

NAMED_FORMATS = [
    "General Number",
    "Currency",
    "Fixed",
    "Standard",
    "Percent",
    "Scientific",
    "Yes/No",
    "True/False",
    "On/Off",
    "General Date",
    "Long Date",
    "Medium Date",
    "Short Date",
    "Long Time",
    "Medium Time",
    "Short Time",
]

CUSTOM_FORMATS = [
    "0",
    "0.00",
    "#,0",
    "#,0.00",
    "#,##0.0#",
    "0%",
    "0.0%",
    "0.00%",
    "+0.0%;-0.0%;0.0%",
    "$#,0.00;($#,0.00);\"-\"",
    "\\$#,0.00",
    "#,0,K",
    "#,0.0,,\"M\"",
    "0.00E+00",
    "000000",
    "#",
    "@",
    ">",
    "yyyy-mm-dd",
    "yyyy-mm",
    "dd/mm/yyyy",
    "m/d/yyyy",
    "mmm yyyy",
    "mmmm d, yyyy",
    "ddd",
    "dddd",
    "q",
    "\"Q\"q yyyy",
    "hh:nn:ss",
    "h:nn AM/PM",
    "yyyy-mm-dd hh:nn",
    "ww",
    "y",
]


def format_suite() -> dict:
    formats: dict[str, None] = {}
    for f in NAMED_FORMATS + CUSTOM_FORMATS:
        formats.setdefault(f, None)
    for f in acme_format_strings():
        # TMDL writes a leading backslash escape for '$' and keeps the rest verbatim.
        formats.setdefault(f, None)
    cases = []
    for fmt in formats:
        for key, value in FORMAT_VALUES:
            cases.append(
                {
                    "id": f"{slug(fmt) or 'fmt'}-{short_hash(fmt)}/{key}",
                    "query": f'EVALUATE ROW("v", FORMAT({value}, {dax_string(fmt)}))',
                }
            )
    formats_by_measure = acme_measure_formats()
    for _, m in acme_measures():
        fmt = formats_by_measure.get(m, "")
        cases.append(
            {
                "id": f"measure/{slug(m)}",
                "query": f'EVALUATE ROW("v", FORMAT({measure_ref(m)}, {dax_string(fmt)}))',
            }
        )
    return suite(
        "format",
        "FORMAT(value, format) for the named and common custom formats and every Acme format string (en-US)",
        "acme",
        ACME_TABLES,
        cases,
    )


# --- DAX functions (functions.rs) --------------------------------------------------------------

RUST_STR = re.compile(r'"((?:[^"\\]|\\.)*)"', re.S)
CASE_START = re.compile(r'\(\s*"((?:[^"\\]|\\.)*)"\s*,', re.S)


def rust_unescape(s: str) -> str:
    out = []
    i = 0
    while i < len(s):
        c = s[i]
        if c != "\\":
            out.append(c)
            i += 1
            continue
        n = s[i + 1]
        if n == "\n":
            i += 2
            while i < len(s) and s[i] in " \t\r\n":
                i += 1
            continue
        mapping = {"n": "\n", "t": "\t", "r": "\r", "\\": "\\", '"': '"', "'": "'", "0": "\0"}
        if n in mapping:
            out.append(mapping[n])
            i += 2
            continue
        if n == "u":
            end = s.index("}", i)
            out.append(chr(int(s[i + 3 : end], 16)))
            i = end + 1
            continue
        raise SystemExit(f"unsupported Rust escape \\{n}")
    return "".join(out)


def fn_body(src: str, name: str) -> str:
    m = re.search(rf"^fn {name}\(\)[^{{]*\{{", src, re.M)
    if not m:
        raise SystemExit(f"{FUNCTIONS_RS}: fn {name} not found")
    depth = 1
    i = m.end()
    in_str = False
    while depth:
        c = src[i]
        if in_str:
            if c == "\\":
                i += 1
            elif c == '"':
                in_str = False
        elif c == '"':
            in_str = True
        elif c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
        i += 1
    return src[m.end() : i - 1]


def extract_cases(src: str, name: str) -> list[str]:
    body = fn_body(src, name)
    body = re.sub(r"^\s*//.*$", "", body, flags=re.M)
    return [rust_unescape(m.group(1)) for m in CASE_START.finditer(body)]


FX_GROUPS = [
    ("logical", "logical_cases"),
    ("math", "math_cases"),
    ("text", "text_cases"),
    ("date", "date_cases"),
    ("aggregate", "aggregate_cases"),
    ("context", "context_cases"),
    ("table", "table_cases"),
    ("date-more", "date_more_cases"),
    ("stats", "stats_cases"),
    ("window", "window_cases"),
    ("misc", "misc_cases"),
    ("visual", "visual_cases"),
    ("library", "library_cases"),
    ("library-approx", "library_approx_cases"),
]

FEB_2024 = "'Date'[Year] = 2024, 'Date'[Month] = 2"
MAR_2024 = "'Date'[Year] = 2024, 'Date'[Month] = 3"
VOLATILE_FUNCTIONS = re.compile(
    r"\b(NOW|TODAY|UTCNOW|UTCTODAY|RAND|RANDBETWEEN|USERNAME|USERPRINCIPALNAME|USEROBJECTID|USERCULTURE|CUSTOMDATA|NEWGUID)\s*\(",
    re.I,
)


def dax_case(group: str, expr: str, filt: str | None = None) -> dict:
    row = f'ROW("v", {expr})'
    query = f"EVALUATE {row}" if not filt else f"EVALUATE CALCULATETABLE({row}, {filt})"
    case = {"id": f"{group}/{short_hash(query)}", "expr": expr, "query": query}
    if VOLATILE_FUNCTIONS.search(expr):
        case["volatile"] = "depends on the clock, randomness or the signed-in user"
    return case


def dax_functions_suite() -> dict:
    src = FUNCTIONS_RS.read_text(encoding="utf-8")
    cases: list[dict] = []
    seen: set[str] = set()

    def add(case: dict) -> None:
        if case["id"] not in seen:
            seen.add(case["id"])
            cases.append(case)

    for group, fn in FX_GROUPS:
        exprs = extract_cases(src, fn)
        if not exprs:
            raise SystemExit(f"no cases found in {fn}")
        for e in exprs:
            add(dax_case(group, e))
    for e in extract_cases(src, "time_intelligence_cases"):
        add(dax_case("time-intelligence", e, FEB_2024))
    extra = [
        ("OPENINGBALANCEMONTH([Amt], 'Date'[Date])", MAR_2024),
        ("CALCULATE([Amt], PREVIOUSMONTH('Date'[Date]))", MAR_2024),
        ("TOTALYTD([Amt], 'Date'[Date])", None),
        ("COUNTROWS(DATESYTD('Date'[Date]))", None),
        ("CALCULATE([Amt], SAMEPERIODLASTYEAR('Date'[Date]))", None),
        ("MINX(VALUES('Date'[Year]), TOTALYTD([Amt], 'Date'[Date]))", None),
    ]
    for e, f in extra:
        add(dax_case("time-intelligence", e, f))
    return suite(
        "dax-functions",
        "Every case of crates/pbi-dax/tests/functions.rs as EVALUATE ROW(\"v\", <expr>) on the dax-lab fixture",
        "dax-lab",
        DAX_LAB_TABLES,
        cases,
    )


def dax_lab_tables_suite() -> dict:
    cases = [{"id": slug(t), "query": f"EVALUATE {table_ref(t)}"} for t in DAX_LAB_TABLES]
    return suite("dax-lab-tables", "Every dax-lab table after refresh", "dax-lab", DAX_LAB_TABLES, cases)


CATALOG_LINE = re.compile(r"^([A-Z][A-Z0-9.]*)\((.*)\)\s*(?:\{(\d+)\.\.(\d+|\*)\})?\s*->\s*(\w+)\s*\|")

# A placeholder per parameter name: plausible enough that Desktop checks the
# argument count; whether it then accepts the types does not matter.
ARITY_PLACEHOLDERS = [
    (re.compile(r"^dates$"), "'Date'[Date]"),
    (re.compile(r"^(idColumn)$"), "Emp[Id]"),
    (re.compile(r"^(parentColumn)$"), "Emp[Parent]"),
    (re.compile(r"(?i)table|^relation$|^data$"), "Sales"),
    (re.compile(r"(?i)column|^orderByExpression$"), "Sales[Qty]"),
    (re.compile(r"^measure$"), "[Total Qty]"),
    (re.compile(r"^orderBy$"), "ORDERBY(Sales[Qty])"),
    (re.compile(r"^partitionBy$"), "PARTITIONBY(Sales[Line])"),
    (re.compile(r"^matchBy$"), "MATCHBY(Sales[Id])"),
    (re.compile(r"^yearEndDate$"), '"12-31"'),
    (re.compile(r"(?i)date$|^datetime$"), "DATE(2024, 1, 1)"),
    (re.compile(r"^dateText$"), '"2024-01-01"'),
    (re.compile(r"^timeText$"), '"12:00"'),
    (re.compile(r"^interval$"), "MONTH"),
    (re.compile(r"^order$"), "ASC"),
    (re.compile(r"(?i)^(filter|logical\d?|logicalTest|cumulative)$"), "TRUE()"),
    (re.compile(r"(?i)name$|text\d?$|^(label|delimiter|formatString|path|item|localeName)$"), '"a"'),
]


def catalog_entries() -> list[tuple[str, list[str], int, int | None, str]]:
    """(name, flattened parameter names, min, max, returns) of every catalog.txt function.

    Mirrors crates/pbi-dax/src/service/catalog.rs; the test compares Desktop with the Rust
    catalog, so a difference here only moves the probes.
    """
    out = []
    for line in CATALOG_TXT.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        m = CATALOG_LINE.match(line)
        if not m:
            raise SystemExit(f"catalog.txt: cannot parse {line!r}")
        name, params, lo, hi, returns = m.groups()
        items, depth, cur = [], 0, ""
        for ch in params:
            if ch == "," and depth == 0:
                items.append(cur.strip())
                cur = ""
                continue
            depth += ch == "["
            depth -= ch == "]"
            cur += ch
        if cur.strip():
            items.append(cur.strip())
        flat = [w for i in items for w in re.findall(r"[A-Za-z][A-Za-z0-9]*", i)]
        repeating = any(i.endswith("...") for i in items)
        min_args = sum(1 for i in items if not i.startswith("["))
        max_args = None if repeating else len(flat)
        if lo is not None:
            min_args, max_args = int(lo), (None if hi == "*" else int(hi))
        out.append((name, flat, min_args, max_args, returns))
    return out


def arity_placeholder(param: str) -> str:
    for pattern, value in ARITY_PLACEHOLDERS:
        if pattern.search(param):
            return value
    return "1"


def dax_arity_suite() -> dict:
    cases = []
    for name, params, lo, hi, returns in catalog_entries():
        counts = {lo, lo + 4} if hi is None else {lo, hi, hi + 1}
        if lo > 0:
            counts.add(lo - 1)
        for n in sorted(counts):
            # Past the listed parameters, repeat the last ones.
            args = [arity_placeholder(params[min(i, len(params) - 1)]) if params else "1" for i in range(n)]
            call = f"{name}({', '.join(args)})"
            inner = f"COUNTROWS({call})" if returns == "table" else call
            cases.append({"id": f"{name}/{n}", "function": name, "args": n, "query": f'EVALUATE ROW("v", {inner})'})
    doc = suite(
        "dax-arity",
        "Every catalog.txt function called with too few, the fewest, the most and too many arguments on the dax-lab fixture",
        "dax-lab",
        DAX_LAB_TABLES,
        cases,
    )
    # Only whether Desktop accepts the argument count matters: never record values (USERNAME…).
    doc["recordValues"] = False
    return doc


def render(doc: dict) -> str:
    return json.dumps(doc, indent=2, ensure_ascii=False) + "\n"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--check", action="store_true", help="fail if a committed corpus is out of date")
    args = ap.parse_args()
    expected = json.loads((ACME / "expected.json").read_text(encoding="utf-8"))
    suites = [
        acme_metadata_suite(),
        acme_tables_suite(),
        acme_measures_suite(expected),
        acme_visuals_suite(expected),
        acme_calcgroup_rls_suite(expected),
        format_suite(),
        dax_lab_tables_suite(),
        dax_functions_suite(),
        dax_arity_suite(),
        *sample_suites(),
    ]
    stale = []
    OUT.mkdir(parents=True, exist_ok=True)
    for s in suites:
        path = OUT / f"{s['suite']}.json"
        text = render(s)
        if args.check:
            if not path.exists() or path.read_text(encoding="utf-8") != text:
                stale.append(path.name)
        else:
            path.write_text(text, encoding="utf-8", newline="\n")
            print(f"{path.relative_to(ROOT)}: {len(s['cases'])} cases")
    if stale:
        print("stale corpora (run scripts/groundtruth/gen_corpus.py): " + ", ".join(stale))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
