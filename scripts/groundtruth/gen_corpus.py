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
M_LIBRARY = ROOT / "scripts" / "groundtruth" / "m-library.m"
M_ARITY = ROOT / "scripts" / "groundtruth" / "m-arity.m"
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


def m_suite(name: str, title: str, cases: list[dict]) -> dict:
    """An M suite: pure M expressions that Desktop evaluates against its standard library alone
    (Record-GroundTruth.ps1 runs them in a temporary table of any open model)."""
    doc = suite(name, title, "any", [], cases)
    doc["language"] = "m"
    return doc


def m_library_suite() -> dict:
    text = M_LIBRARY.read_text(encoding="utf-8")
    return m_suite(
        "m-library",
        "Every name of Power BI Desktop's M standard library: kind, parameters, return type, type and constant values",
        [{"id": "library", "query": text}],
    )


def m_arity_suite() -> dict:
    text = M_ARITY.read_text(encoding="utf-8")
    return m_suite(
        "m-arity",
        "The error Power BI Desktop raises when each M library function gets too few or too many arguments",
        [{"id": "arity", "query": text}],
    )


# M expressions for the m-functions suite: (id, expression). Error cases use distinct messages:
# within one probe batch Desktop may reuse the error record of an earlier case with the same
# message (reference/groundtruth/README.md).
# `e(try x)`: x, or "Reason: Message" when x raises. `s(t)`: the name, type name and nullability of
# every column of table t (recordings carry values, not column types).
_E = 'e = (r) => if r[HasError] then r[Error][Reason] & ": " & r[Error][Message] else r[Value]'
# The rows of a table with every cell read through `x{i}` inside its own `try`, so row and cell
# errors show per cell instead of escaping as lazy list items.
_RW = 'rw = (x) => let c = Table.ColumnNames(x) in List.Buffer(List.Transform({0..Table.RowCount(x) - 1}, (i) => List.Transform(c, (k) => e(try Record.Field(x{i}, k)))))'
_S = 's = (t) => Table.ToRows(Table.SelectColumns(Table.Schema(t), {"Name", "TypeName", "IsNullable"}))'
M_FUNCTIONS = [
    # Error.Record and the error records `try` catches.
    ("Error.Record/reason-only", 'Error.Record("r-1")'),
    ("Error.Record/all-arguments", 'Error.Record("r-2", "m-2 #{0}", "d-2", {"x"}, "E2")'),
    ("Error.Record/null-message-with-parameters", 'Error.Record("r-3", null, null, {1})'),
    ("Error.Record/detail-record", 'Error.Record("r-4", "m-4", [a = 1])'),
    ("Error.Record/error-code-not-text", 'Error.Record("r-5", "m-5", null, null, 5)'),
    ("Error.Record/parameters-not-list", 'Error.Record("r-6", "m-6", null, 6)'),
    ("Error.Record/raised", '(try error Error.Record("r-7", "m-7 #{0}", null, {"p7"}, "E7"))[Error]'),
    ("try/error-text", '(try error "boom-8")[Error]'),
    ("try/error-record-format", '(try error [Reason = "R9", Message = "ignored-9", #"Message.Format" = "v9=#{0}", #"Message.Parameters" = {9}, ErrorCode = 9])[Error]'),
    ("try/error-record-no-format", '(try error [Reason = "R10", Message = "m-10", #"Message.Parameters" = {10}])[Error]'),
    ("try/error-record-no-reason", '(try error [Message = "m-11"])[Error]'),
    ("try/error-record-no-message", '(try error [Reason = "R12"])[Error]'),
    ("try/error-record-detail", '(try error [Reason = "R13", Message = "m-13", Detail = [a = 13]])[Error]'),
    ("try/has-error", 'let r = try error "boom-14" in [HasError = r[HasError], Reason = r[Error][Reason]]'),
    ("try/otherwise", 'try error "boom-15" otherwise 15'),
    ("try/engine-error", 'let e = (try 1 + "a")[Error] in [Reason = e[Reason], Message = e[Message]]'),
    ("error/record-message-not-text", 'error [Reason = "R16", Message = 16]'),
    ("error/record-bad-placeholder", 'error [Reason = "R17", #"Message.Format" = "m-17 #{x}"]'),
    ("error/record-unclosed-placeholder", 'error [Reason = "R18", #"Message.Format" = "m-18 #{0"]'),
    ("error/text", 'error "boom-19"'),
    # Error message placeholders (#{n}).
    ("format/basic", 'Error.Record("P", "a #{0} b #{1}", null, {1, "x"})[Message]'),
    ("format/index-spacing", 'Error.Record("P", "#{ 0}|#{+1}|#{00}", null, {"a", "b"})[Message]'),
    ("format/literal-hashes", 'Error.Record("P", "##{0}|#{0}}|# { } #", null, {1})[Message]'),
    ("format/numbers", 'Error.Record("P", "#{0}|#{1}|#{2}|#{3}|#{4}|#{5}|#{6}", null, {null, true, 1.5, -0.5, 0.1 + 0.2, 1e20, 1.2345678901234568e23 + 0})[Message]'),
    # Desktop keeps number literals as System.Decimal until arithmetic; DashCraft numbers are f64.
    ("literals/decimal-precision", '{Text.From(9007199254740993), Text.From(123456789.123456789), Text.From(123456789.123456789 + 0)}'),
    ("format/small-numbers", 'Error.Record("P", "#{0}|#{1}|#{2}|#{3}", null, {1e-7, 2.5e-5, 0.0001, 0.00012})[Message]'),
    ("format/special-numbers", 'Error.Record("P", "#{0}|#{1}|#{2}|#{3}", null, {#nan, #infinity, -#infinity, -0})[Message]'),
    ("format/dates-and-times", 'Error.Record("P", "#{0}|#{1}|#{2}|#{3}", null, {#date(2024, 1, 2), #datetime(2024, 1, 2, 3, 4, 5), #datetimezone(2024, 1, 2, 3, 4, 5, 1, 30), #time(13, 2, 59)})[Message]'),
    ("format/durations", 'Error.Record("P", "#{0}|#{1}", null, {#duration(0, 0, 0, 1.5), #duration(-1, -2, 0, 0)})[Message]'),
    ("format/structured", 'Error.Record("P", "#{0}|#{1}|#{2}|#{3}|#{4}", null, {{1}, [a = 1], #table({"a"}, {}), each _, type text})[Message]'),
    ("format/binary", 'Error.Record("P", "#{0}", null, {#binary({1, 2})})[Message]'),
    ("format/index-not-integer", 'Error.Record("P", "f-20 #{x}", null, {1})[Message]'),
    ("format/index-negative", 'Error.Record("P", "f-21 #{-1}", null, {1})[Message]'),
    ("format/index-out-of-range", 'Error.Record("P", "f-22 #{5}", null, {1})[Message]'),
    ("format/unclosed", 'Error.Record("P", "f-23 #{0} #{", null, {1})[Message]'),
    ("format/error-parameter", 'Error.Record("P", "f-24 #{0}", null, {error "pe-24"})[Message]'),
    ("format/unreferenced-error-parameter", 'Error.Record("P", "f-25 #{0}", null, {25, error "pe-25"})[Message]'),
    # Splitters.
    ("Splitter.SplitTextByDelimiter/csv-default", 'Splitter.SplitTextByDelimiter(",")("a,""b,c"",d")'),
    ("Splitter.SplitTextByDelimiter/quote-none", 'Splitter.SplitTextByDelimiter(",", QuoteStyle.None)("a,""b,c"",d")'),
    ("Splitter.SplitTextByDelimiter/quote-mid-field", 'Splitter.SplitTextByDelimiter(",")("a,x""b,c"",d")'),
    ("Splitter.SplitTextByDelimiter/quote-after-delimiter", 'Splitter.SplitTextByDelimiter(",", QuoteStyle.Csv, CsvStyle.QuoteAfterDelimiter)("a,x""b,c"",""d,e""")'),
    ("Splitter.SplitTextByDelimiter/quote-always", 'Splitter.SplitTextByDelimiter(",", QuoteStyle.Csv, CsvStyle.QuoteAlways)("a,x""b,c"",d")'),
    ("Splitter.SplitTextByDelimiter/unclosed-quote", 'Splitter.SplitTextByDelimiter(",")("a,""b,c")'),
    ("Splitter.SplitTextByDelimiter/text-after-quote", 'Splitter.SplitTextByDelimiter(",")("""a""x,b")'),
    ("Splitter.SplitTextByDelimiter/escaped-quote", 'Splitter.SplitTextByDelimiter(",")("""a""""b"",c")'),
    ("Splitter.SplitTextByDelimiter/line-breaks", 'Splitter.SplitTextByDelimiter(",")("a#(cr,lf)b,c")'),
    ("Splitter.SplitTextByDelimiter/multi-character", 'Splitter.SplitTextByDelimiter("--")("a--b---c")'),
    ("Splitter.SplitTextByDelimiter/null-text", 'Splitter.SplitTextByDelimiter(",")(null)'),
    ("Splitter.SplitTextByDelimiter/empty-text", 'Splitter.SplitTextByDelimiter(",")("")'),
    ("Splitter.SplitTextByDelimiter/empty-delimiter", 'Splitter.SplitTextByDelimiter("")("s-26")'),
    ("Splitter.SplitTextByDelimiter/quote-style-invalid", 'Splitter.SplitTextByDelimiter(",", 2)("s-27")'),
    ("Splitter.SplitTextByDelimiter/quote-none-with-csv-style", 'Splitter.SplitTextByDelimiter(",", QuoteStyle.None, CsvStyle.QuoteAlways)("s-28")'),
    ("Splitter.SplitTextByDelimiter/csv-style-invalid", 'Splitter.SplitTextByDelimiter(",", null, 5)("s-29")'),
    ("Splitter.SplitTextByAnyDelimiter/list-order", 'Splitter.SplitTextByAnyDelimiter({"--", "-"})("a---b")'),
    ("Splitter.SplitTextByAnyDelimiter/csv-default", 'Splitter.SplitTextByAnyDelimiter({",", ";"})("a;""b,c"",d")'),
    ("Splitter.SplitTextByAnyDelimiter/empty-list", 'Splitter.SplitTextByAnyDelimiter({})("a,b")'),
    ("Splitter.SplitTextByAnyDelimiter/start-at-end", 'Splitter.SplitTextByAnyDelimiter({",", ";"}, QuoteStyle.Csv, true)("a,b;""c,d"",e")'),
    ("Splitter.SplitTextByAnyDelimiter/start-at-end-overlap", 'Splitter.SplitTextByAnyDelimiter({"--", "-"}, QuoteStyle.None, true)("a---b")'),
    ("Splitter.SplitTextByAnyDelimiter/start-at-end-not-logical", 'Splitter.SplitTextByAnyDelimiter({","}, QuoteStyle.None, 1)("s-30")'),
    ("Splitter.SplitTextByAnyDelimiter/empty-delimiter", 'Splitter.SplitTextByAnyDelimiter({",", ""})("s-31")'),
    ("Splitter.SplitTextByEachDelimiter/basic", 'Splitter.SplitTextByEachDelimiter({",", ";"})("a,b;c,d")'),
    ("Splitter.SplitByNothing/basic", 'Splitter.SplitByNothing()("a,b")'),
    ("Splitter.SplitTextByPositions/basic", 'Splitter.SplitTextByPositions({0, 2, 5})("abcdefg")'),
    ("Splitter.SplitTextByLengths/basic", 'Splitter.SplitTextByLengths({1, 3})("abcdefg")'),
    ("Splitter.SplitTextByCharacterTransition/basic", 'Splitter.SplitTextByCharacterTransition({"a".."z"}, {"0".."9"})("ab12cd34")'),
    # Library constants.
    ("constants/TraceLevel", '{TraceLevel.Critical, TraceLevel.Error, TraceLevel.Warning, TraceLevel.Information, TraceLevel.Verbose}'),
    ("constants/QuoteStyle-CsvStyle", '{QuoteStyle.None, QuoteStyle.Csv, CsvStyle.QuoteAfterDelimiter, CsvStyle.QuoteAlways}'),
    ("constants/numbers", '{Number.PI, Number.E, Number.Epsilon, Number.PositiveInfinity, Number.NegativeInfinity}'),
    # Text.
    ("Text.Upper/basic", 'Text.Upper("abc é")'),
    ("Text.Proper/basic", 'Text.Proper("hello wORLD o\'neil")'),
    ("Text.Trim/basic", '{Text.Trim("  a b  "), Text.TrimStart("xxaxx", "x"), Text.TrimEnd("xxaxx", {"x", "a"})}'),
    ("Text.PadStart/basic", '{Text.PadStart("7", 3, "0"), Text.PadEnd("ab", 4), Text.PadStart("abcd", 2)}'),
    ("Text.Middle/basic", '{Text.Middle("abcdef", 2, 3), Text.Middle("abc", 2, 10), Text.Range("abcdef", 1, 2), Text.Start("abc", 2), Text.End("abc", 2)}'),
    ("Text.Replace/basic", 'Text.Replace("a-b-c", "-", "+")'),
    ("Text.Split/basic", 'Text.Split("a,b,,c", ",")'),
    ("Text.Combine/basic", '{Text.Combine({"a", null, "b"}, "-"), Text.Combine({})}'),
    ("Text.Contains/basic", '{Text.Contains("Hello", "ell"), Text.Contains("Hello", "ELL"), Text.Contains("Hello", "ELL", Comparer.OrdinalIgnoreCase)}'),
    ("Text.PositionOf/basic", '{Text.PositionOf("abcabc", "c"), Text.PositionOf("abcabc", "c", Occurrence.Last), Text.PositionOf("abcabc", "c", Occurrence.All), Text.PositionOf("abc", "z")}'),
    ("Text.BeforeDelimiter/basic", '{Text.BeforeDelimiter("a-b-c", "-"), Text.BeforeDelimiter("a-b-c", "-", 1), Text.AfterDelimiter("a-b-c", "-", {0, RelativePosition.FromEnd}), Text.BetweenDelimiters("a(b)c", "(", ")")}'),
    ("Text.Select/basic", '{Text.Select("a1b2", {"0".."9"}), Text.Remove("a1b2", {"0".."9"})}'),
    ("Text.Repeat/basic", '{Text.Repeat("ab", 3), Text.Insert("ac", 1, "b"), Text.Length("héllo"), Text.ToList("abc")}'),
    ("Text.From/basic", '{Text.From(1.5), Text.From(#date(2024, 1, 2)), Text.From(true), Text.From(null), Text.From(#duration(1, 2, 3, 4))}'),
    ("Text.Format/basic", '{Text.Format("#{0}+#{1}", {1, "b"}), Text.Format("#[x]!", [x = "hi"])}'),
    ("Text.Clean/basic", 'Text.Clean("a#(lf)b#(tab)c")'),
    ("Text.StartsWith/basic", '{Text.StartsWith("Hello", "He"), Text.EndsWith("Hello", "LO"), Text.EndsWith("Hello", "LO", Comparer.OrdinalIgnoreCase)}'),
    ("Text.NewGuid/length", 'Text.Length(Text.NewGuid())'),
    ("Text.NewGuid/format", 'let g = Text.NewGuid() in {Text.Length(g), Text.At(g, 8), Text.At(g, 14), Text.Lower(g) = g, List.Count(Text.Split(g, "-"))}'),
    ("Text.PositionOfAny/basic", '{Text.PositionOfAny("abcabc", {"c", "b"}), Text.PositionOfAny("abcabc", {"c"}, Occurrence.Last), Text.PositionOfAny("abcabc", {"b", "c"}, Occurrence.All), Text.PositionOfAny("abc", {"z"})}'),
    ("Splitter/more", '{Splitter.SplitTextByRepeatedLengths(2)("abcde"), Splitter.SplitTextByRepeatedLengths(2, true)("abcde"), Splitter.SplitTextByRanges({{0, 2}, {3, 1}})("abcdef"), Splitter.SplitTextByWhitespace()("a  b#(tab)c"), Splitter.SplitTextByAnyDelimiter({",", ";"})("a,b;c")}'),
    ("Splitter/start-at-end", '{Splitter.SplitTextByLengths({1, 2}, true)("abcdef"), Splitter.SplitTextByPositions({0, 2}, true)("abcdef"), Splitter.SplitTextByEachDelimiter({",", ";"}, QuoteStyle.None, true)("a,b;c,d"), Splitter.SplitTextByCharacterTransition({"a".."z"}, {"0".."9"})("ab12cd3")}'),
    # Number.
    ("Number.Round/banker", '{Number.Round(2.5), Number.Round(3.5), Number.Round(-2.5), Number.Round(2.345, 2)}'),
    ("Number.Round/modes", '{Number.Round(2.5, 0, RoundingMode.AwayFromZero), Number.Round(2.5, 0, RoundingMode.Up), Number.Round(-2.5, 0, RoundingMode.Down), Number.Round(2.5, 0, RoundingMode.TowardZero)}'),
    ("Number.RoundUp/basic", '{Number.RoundUp(1.2), Number.RoundDown(-1.2), Number.RoundUp(1.234, 2), Number.RoundTowardZero(-1.7), Number.RoundAwayFromZero(-1.2)}'),
    ("Number.IntegerDivide/basic", '{Number.IntegerDivide(7, 2), Number.IntegerDivide(-7, 2), Number.Mod(-7, 3), Number.Mod(7.5, 2)}'),
    ("Number.Power/basic", '{Number.Power(2, 10), Number.Sqrt(16), Number.Abs(-3), Number.Sign(-2), Number.Log10(1000), Number.Ln(1)}'),
    ("Number.From/basic", '{Number.From("1.5"), Number.From(true), Number.From(#date(2024, 1, 2)), Number.From(#duration(1, 12, 0, 0)), Number.FromText("1e3")}'),
    ("Number.ToText/formats", '{Number.ToText(1234.5, "N2"), Number.ToText(0.5, "P"), Number.ToText(255, "X"), Number.ToText(1234.5678, "0.00"), Number.ToText(1234.5, "F1"), Number.ToText(0.000123, "E2")}'),
    ("Number.ToText/default", '{Number.ToText(1.5), Number.ToText(1e21), Number.ToText(0.1 + 0.2), Number.ToText(-0)}'),
    ("Number.Bitwise/basic", '{Number.BitwiseAnd(12, 10), Number.BitwiseOr(12, 10), Number.BitwiseXor(12, 10), Number.BitwiseShiftLeft(1, 4)}'),
    ("Number.Bitwise/edges", '{Number.BitwiseNot(5), Number.BitwiseShiftRight(-16, 2), Number.BitwiseAnd(null, 1), Number.BitwiseShiftLeft(1, 64), Number.BitwiseXor(-1, 5)}'),
    ("Number.IsEven/basic", '{Number.IsEven(4), Number.IsOdd(4), Number.IsNaN(0 / 0), 1 / 0, -1 / 0}'),
    ("Number.FromText/invalid", 'Number.FromText("n-32")'),
    ("Int64.From/basic", '{Int64.From(2.5), Int64.From(3.5), Int64.From("12"), Currency.From(1.23456), Decimal.From("1.25")}'),
    # Lists.
    ("List.Sum/basic", '{List.Sum({1, 2, null}), List.Average({1, 2, 4}), List.Max({3, 1, 2}), List.Min({3, 1, 2}), List.Count({1, null}), List.NonNullCount({1, null})}'),
    ("List.Sum/empty", '{List.Sum({}), List.Average({}), List.Max({}), List.Min({}, 0)}'),
    ("List.Distinct/basic", '{List.Distinct({1, 1, 2}), List.Distinct({"a", "A"}, Comparer.OrdinalIgnoreCase)}'),
    ("List.Sort/basic", '{List.Sort({3, 1, 2}), List.Sort({3, 1, 2}, Order.Descending), List.Sort({"b", "A", "a"}), List.Sort({"b", "A", "a"}, Comparer.OrdinalIgnoreCase)}'),
    ("List.Range/basic", '{List.Range({1..5}, 1, 2), List.FirstN({1..5}, 2), List.LastN({1..5}, 2), List.Skip({1..5}, 3), List.RemoveFirstN({1..5}, 1), List.Reverse({1, 2, 3})}'),
    ("List.Zip/basic", 'List.Zip({{1, 2}, {"a", "b", "c"}})'),
    ("List.Combine/basic", '{List.Combine({{1}, {2, 3}}), List.Union({{1, 2}, {2, 3}}), List.Intersect({{1, 2}, {2, 3}}), List.Difference({1, 2, 3}, {2})}'),
    ("List.Accumulate/basic", 'List.Accumulate({1, 2, 3}, 0, (s, c) => s + c)'),
    ("List.Generate/basic", 'List.Generate(() => 0, each _ < 3, each _ + 1, each _ * 10)'),
    ("List.Numbers/basic", '{List.Numbers(1, 3, 2), List.Dates(#date(2024, 1, 30), 3, #duration(1, 0, 0, 0)), {1..3}, {"a".."c"}}'),
    ("List.PositionOf/basic", '{List.PositionOf({1, 2, 3, 2}, 2), List.PositionOf({1, 2, 3, 2}, 2, Occurrence.All), List.Contains({1, 2}, 2), List.ContainsAny({1, 2}, {3, 2}), List.ContainsAll({1, 2}, {2, 3})}'),
    ("List.Select/basic", '{List.Select({1..6}, Number.IsEven), List.Transform({1, 2}, each _ * 2), List.RemoveNulls({1, null, 2}), List.RemoveItems({1, 2, 3}, {2})}'),
    ("List.Statistics/basic", '{List.Median({1, 3, 2, 4}), List.Mode({1, 2, 2, 3}), List.StandardDeviation({1, 2, 3, 4}), List.Product({2, 3})}'),
    ("List.Statistics/more", 'let e = (r) => if r[HasError] then r[Error][Reason] & ": " & r[Error][Message] else r[Value] in {e(try List.Modes({1, 2, 2, 3, 3})), e(try List.Covariance({1, 2, 3}, {2, 4, 7})), e(try List.Median({})), e(try List.StandardDeviation({1})), e(try List.Median({"b", "a", "c"})), e(try List.Product({})), e(try List.Mode({}))}'),
    ("List.Percentile/basic", '{List.Percentile({1, 2, 3, 4}, 0.5), List.Percentile({1, 2, 3, 4}, {0.25, 0.75}), List.Percentile({1, 2, 3, 4}, 0.25, [PercentileMode = PercentileMode.ExcelExc]), List.Percentile({1, 2, 3, 4}, 0.3, [PercentileMode = PercentileMode.SqlDisc]), List.Percentile({null, 5}, 0.5), List.Percentile({#date(2024, 1, 1), #date(2024, 1, 4)}, 0.5)}'),
    ("List.Percentile/errors", 'let e = (r) => if r[HasError] then r[Error][Reason] & ": " & r[Error][Message] else r[Value] in {e(try List.Percentile({1, "a"}, 0.5)), e(try List.Percentile({"a", "b"}, 0.5)), e(try List.Percentile({1, 2}, 1.5)), e(try List.Percentile({1, 2}, 0.9, [PercentileMode = PercentileMode.ExcelExc]))}'),
    ("List.Split/basic", '{List.Split({1..5}, 2), List.Alternate({1..10}, 1, 1), List.Repeat({1, 2}, 2), List.Buffer({1})}'),
    ("List.First/basic", '{List.First({}), List.First({}, "d"), List.Last({1, 2}), List.IsEmpty({}), List.Single({7}), List.SingleOrDefault({}, 0)}'),
    ("List.MatchesAll/basic", '{List.MatchesAll({2, 4}, Number.IsEven), List.MatchesAny({1, 3}, Number.IsEven), List.AnyTrue({false, true}), List.AllTrue({})}'),
    ("List.ReplaceValue/basic", '{List.ReplaceValue({"a", "b"}, "a", "z", Replacer.ReplaceValue), List.ReplaceValue({"ab", "b"}, "b", "z", Replacer.ReplaceText), List.InsertRange({1, 4}, 1, {2, 3}), List.ReplaceRange({1, 2, 3}, 1, 1, {9})}'),
    ("List.Edit/more", '{List.RemoveRange({1..5}, 1, 2), List.Positions({"a", "b"}), List.IsDistinct({1, 2, 1}), List.MaxN({3, 1, 4, 1, 5}, 2), List.MinN({3, 1, 4}, each _ < 4), List.RemoveLastN({1..5}, 2), List.RemoveMatchingItems({1, 2, 3, 2}, {2}), List.ReplaceMatchingItems({1, 2, 3}, {{2, 9}}), List.TransformMany({1, 2}, each {_, _ * 10}, (x, y) => x + y), List.PositionOfAny({1, 2, 3}, {3, 2})}'),
    ("List.Item/out-of-range", '{1, 2}{5}'),
    ("List.Item/optional", '{{1, 2}{5}?, [a = 1][b]?}'),
    # Dates and times.
    ("Date.AddMonths/end-of-month", '{Date.AddMonths(#date(2024, 1, 31), 1), Date.AddYears(#date(2024, 2, 29), 1), Date.AddDays(#date(2024, 2, 28), 2), Date.AddWeeks(#date(2024, 1, 1), 2)}'),
    ("Date.StartOfWeek/basic", '{Date.StartOfWeek(#date(2024, 1, 3)), Date.StartOfWeek(#date(2024, 1, 3), Day.Monday), Date.EndOfMonth(#date(2024, 2, 10)), Date.StartOfQuarter(#date(2024, 5, 10)), Date.EndOfYear(#date(2024, 5, 10))}'),
    ("Date.DayOfWeek/basic", '{Date.DayOfWeek(#date(2024, 1, 7)), Date.DayOfWeek(#date(2024, 1, 7), Day.Monday), Date.DayOfYear(#date(2024, 3, 1)), Date.WeekOfYear(#date(2024, 12, 31)), Date.WeekOfMonth(#date(2024, 1, 31)), Date.QuarterOfYear(#date(2024, 8, 1))}'),
    ("Date.Week/edges", '{Date.WeekOfYear(#date(2024, 12, 31), Day.Monday), Date.WeekOfYear(#date(2023, 1, 1), Day.Monday), Date.WeekOfMonth(#date(2024, 3, 31)), Date.WeekOfMonth(#date(2024, 3, 31), Day.Monday), Date.QuarterOfYear(null), Date.QuarterOfYear(#datetime(2024, 12, 1, 0, 0, 0)), Date.WeekOfYear(null)}'),
    ("Date.StartOf/kinds", '{Date.StartOfWeek(#datetimezone(2024, 1, 3, 10, 0, 0, 2, 0)), Date.EndOfWeek(#date(2024, 1, 3), Day.Monday), Date.EndOfWeek(#datetime(2024, 1, 3, 10, 0, 0)), Date.StartOfQuarter(#datetime(2024, 5, 10, 3, 0, 0)), Date.EndOfQuarter(#datetimezone(2024, 5, 10, 3, 0, 0, 1, 0)), Date.EndOfQuarter(#date(2024, 11, 10)), Date.StartOfDay(#datetimezone(2024, 1, 2, 3, 4, 5, -5, 0)), Date.EndOfDay(#datetime(2024, 1, 2, 3, 4, 5)), Date.EndOfDay(#date(2024, 1, 2)), Date.EndOfMonth(#datetime(2024, 2, 3, 4, 5, 6))}'),
    ("Time.StartOfHour/basic", '{Time.StartOfHour(#time(5, 30, 10)), Time.EndOfHour(#time(5, 30, 10)), Time.StartOfHour(#datetime(2024, 1, 2, 5, 30, 10)), Time.EndOfHour(#datetimezone(2024, 1, 2, 5, 30, 10, 2, 0)), Time.StartOfHour(#date(2024, 1, 2)), Time.StartOfHour(null)}'),
    ("Date.AddWeeks/kinds", '{Date.AddWeeks(null, 1), Date.AddWeeks(#datetime(2024, 1, 31, 1, 2, 3), -2), Date.AddQuarters(#date(2024, 1, 31), 1), Date.AddQuarters(#datetimezone(2024, 11, 30, 1, 0, 0, 1, 0), 1), Date.AddQuarters(#date(2024, 1, 1), 2.0)}'),
    ("Date/errors", 'let e = (r) => if r[HasError] then r[Error][Reason] & ": " & r[Error][Message] else r[Value] in {e(try Date.AddWeeks(#date(2024, 1, 1), 1.6)), e(try Date.AddWeeks(#date(9999, 12, 30), 1)), e(try Date.StartOfWeek(#date(2024, 1, 3), 7)), e(try Date.StartOfWeek(#date(1, 1, 1))), e(try Date.StartOfDay(#time(1, 2, 3))), e(try Date.WeekOfYear("2024-01-01")), e(try Date.DayOfWeek(#date(2024, 1, 3), -1)), e(try Date.AddDays(#date(2024, 1, 1), 1.5))}'),
    ("Date.Names/basic", '{Date.MonthName(#date(2024, 3, 1)), Date.DayOfWeekName(#date(2024, 3, 1)), Date.MonthName(#date(2024, 3, 1), "fr-FR")}'),
    ("Date.Names/cultures", '{Date.MonthName(#date(2024, 5, 1), "de-DE"), Date.DayOfWeekName(#date(2024, 5, 3), "es-ES"), Date.MonthName(#date(2024, 2, 1), "ja-JP"), Date.DayOfWeekName(#date(2024, 5, 5), "nl-NL"), Date.MonthName(#date(2024, 2, 1), "pt-BR"), Date.MonthName(#date(2024, 3, 1), "xx-YY"), Date.MonthName(null), Date.DayOfWeekName(#datetimezone(2024, 5, 1, 1, 0, 0, 2, 0)), Date.MonthName(#time(1, 0, 0)), Date.DayOfWeekName(#date(2024, 1, 1), "ru-RU")}'),
    ("Date.IsLeapYear/basic", '{Date.IsLeapYear(#date(2024, 1, 1)), Date.IsLeapYear(#date(1900, 1, 1)), Date.DaysInMonth(#date(2023, 2, 1))}'),
    ("Date.ToText/basic", '{Date.ToText(#date(2024, 1, 2)), Date.ToText(#date(2024, 1, 2), "dd MMM yyyy"), Date.ToText(#date(2024, 1, 2), [Format = "yyyy-MM-dd"]), DateTime.ToText(#datetime(2024, 1, 2, 13, 4, 5))}'),
    ("Date.FromText/basic", '{Date.FromText("2024-03-05"), Date.FromText("3/5/2024"), Date.From(45000), DateTime.From("2024-01-02 03:04"), Time.From(0.75), DateTime.FromText("2024-01-02T03:04:05")}'),
    ("Date.ToRecord/basic", '{Date.ToRecord(#date(2024, 1, 2)), Time.ToRecord(#time(1, 2, 3)), DateTime.Date(#datetime(2024, 1, 2, 3, 4, 5)), DateTime.Time(#datetime(2024, 1, 2, 3, 4, 5))}'),
    ("Date.ToRecord/kinds", '{Time.ToRecord(#time(1, 2, 3.5)), DateTime.ToRecord(#datetime(2024, 1, 2, 3, 4, 5.25)), DateTimeZone.ToRecord(#datetimezone(2024, 1, 2, 3, 4, 5, -5, -30))}'),
    ("Date.ToRecord/errors", 'let e = (r) => if r[HasError] then r[Error][Reason] & ": " & r[Error][Message] else r[Value] in {e(try Date.ToRecord(#datetime(2024, 1, 2, 3, 4, 5))), e(try Date.ToRecord(null)), e(try Time.ToRecord(#datetime(2024, 1, 2, 3, 4, 5))), e(try DateTimeZone.ToRecord(#datetime(2024, 1, 2, 3, 4, 5)))}'),
    ("Date.Arithmetic/basic", '{#date(2024, 3, 1) - #date(2024, 2, 1), #date(2024, 1, 1) + #duration(1, 0, 0, 0), #datetime(2024, 1, 1, 0, 0, 0) - #duration(0, 1, 0, 0), #time(23, 0, 0) + #duration(0, 2, 0, 0)}'),
    ("Duration/basic", '{Duration.TotalHours(#duration(1, 2, 0, 0)), Duration.From(1.5), Duration.Days(#duration(1, 2, 3, 4)), Duration.FromText("1.02:03:04"), Duration.ToText(#duration(0, 0, 0, 1.25))}'),
    ("DateTimeZone/basic", '{DateTime.AddZone(#datetime(2024, 1, 1, 0, 0, 0), 2), DateTimeZone.SwitchZone(#datetimezone(2024, 1, 1, 12, 0, 0, 2, 0), -5), DateTimeZone.RemoveZone(#datetimezone(2024, 1, 1, 12, 0, 0, 2, 0)), DateTimeZone.ZoneHours(#datetimezone(2024, 1, 1, 12, 0, 0, -3, -30))}'),
    ("Date.FromText/invalid", 'Date.FromText("d-33")'),
    # Records.
    ("Record/basic", '{Record.Field([a = 1], "a"), Record.FieldNames([b = 1, a = 2]), Record.AddField([a = 1], "b", 2), Record.Combine({[a = 1], [b = 2], [a = 3]})}'),
    ("Record/transform", '{Record.TransformFields([a = 1, b = 2], {"a", each _ * 10}), Record.RemoveFields([a = 1, b = 2], "a"), Record.RenameFields([a = 1], {"a", "z"}), Record.ReorderFields([a = 1, b = 2], {"b", "a"})}'),
    ("Record.TransformFields/edges", 'let e = (r) => if r[HasError] then r[Error][Reason] & ": " & r[Error][Message] else r[Value] in {e(try Record.TransformFields([a = 1, b = 2], {{"a", each _ + 1}, {"b", Text.From}})), e(try Record.TransformFields([a = 1], {"z", each _}, MissingField.Ignore)), e(try Record.TransformFields([a = 1], {"z", each _}, MissingField.UseNull)), e(try Record.TransformFields([a = 1], {"z", each _})), e(try Record.TransformFields([a = 1], {{"a", each _}, {"a", each _}}))}'),
    ("Record.ReorderFields/edges", 'let e = (r) => if r[HasError] then r[Error][Reason] & ": " & r[Error][Message] else r[Value] in {e(try Record.ReorderFields([a = 1, b = 2, c = 3], {"c", "a"})), e(try Record.ReorderFields([a = 1, b = 2], {"z", "b"}, MissingField.UseNull)), e(try Record.ReorderFields([a = 1, b = 2], {"z", "b"}, MissingField.Ignore)), e(try Record.ReorderFields([a = 1], {"q"}))}'),
    ("Record.ToTable/basic", 'Record.ToTable([a = 1, b = "x"])'),
    ("Record.Field/missing", 'Record.Field([a = 1], "missing-34")'),
    # Tables.
    ("Table.FromRecords/basic", 'Table.FromRecords({[a = 1, b = "x"], [a = 2, b = "y"]})'),
    ("Table.FromRecords/missing", f'let {_E}, {_RW} in {{e(try rw(Table.FromRecords({{[a = 1, b = 2], [a = 3]}}))), e(try rw(Table.FromRecords({{[a = 1, b = 2], [a = 3]}}, null, MissingField.UseNull))), e(try rw(Table.FromRecords({{[a = 1], [a = 3, b = 4]}}))), e(try rw(Table.FromRecords({{[a = 1, b = 2]}}, {{"b", "z"}}))), e(try rw(Table.FromRecords({{[a = 1, b = 2]}}, {{"b", "z"}}, MissingField.Ignore)))}}'),
    ("Table.RowCount/convert", f'let {_E} in {{e(try Table.RowCount(1)), e(try Table.RowCount("x")), e(try Table.RowCount(() => 1)), e(try Table.RowCount({{1}})), e(try Table.RowCount([a = 1])), e(try Table.RowCount(type table)), e(try Table.RowCount(#binary({{1}}))), e(try List.Count(#table({{}}, {{}}))), e(try Record.FieldNames({{1}})), e(try Number.Abs([a = 1])), e(try Text.Length({{1}})), e(try Text.Length(() => 1)), e(try Text.Length(type text)), e(try Number.Abs(type number)), e(try List.Count(type list)), e(try Record.FieldNames(type record))}}'),
    ("Table.SelectRows/basic", 'Table.SelectRows(Table.FromRecords({[a = 1], [a = 2], [a = 3]}), each [a] > 1)'),
    ("Table.AddColumn/basic", 'Table.AddColumn(#table({"a"}, {{1}, {2}}), "c", each [a] * 2)'),
    ("Table.Group/basic", 'Table.Group(#table({"k", "v"}, {{"x", 1}, {"y", 2}, {"x", 3}}), {"k"}, {{"n", Table.RowCount}, {"s", each List.Sum([v])}})'),
    ("Table.Group/local", 'Table.Group(#table({"k", "v"}, {{"x", 1}, {"x", 2}, {"y", 3}, {"x", 4}}), {"k"}, {{"s", each List.Sum([v])}}, GroupKind.Local)'),
    ("Table.Sort/basic", 'Table.Sort(#table({"a", "b"}, {{2, "x"}, {1, "y"}, {2, "a"}}), {{"a", Order.Descending}, "b"})'),
    ("Table.Unpivot/basic", 'Table.UnpivotOtherColumns(#table({"id", "q1", "q2"}, {{1, 10, 20}, {2, null, 40}}), {"id"}, "q", "v")'),
    ("Table.Pivot/basic", 'Table.Pivot(#table({"id", "q", "v"}, {{1, "q1", 10}, {1, "q2", 20}, {2, "q1", 30}}), {"q1", "q2"}, "q", "v", List.Sum)'),
    ("Table.TransformColumnTypes/basic", 'Table.TransformColumnTypes(#table({"a", "b"}, {{"1.5", "2024-01-02"}}), {{"a", type number}, {"b", type date}})'),
    # Desktop's engine-raised errors carry an ErrorCode ("10041") DashCraft does not reproduce.
    ("Table.TransformColumnTypes/error-cell", 'let t = Table.TransformColumnTypes(#table({"a"}, {{"x35"}}), {{"a", type number}}), r = try t{0}[a] in [HasError = r[HasError], Error = Record.SelectFields(r[Error], {"Reason", "Message", "Detail"})]'),
    ("Table.NestedJoin/basic", 'Table.ExpandTableColumn(Table.NestedJoin(#table({"k", "a"}, {{1, "x"}, {2, "y"}}), {"k"}, #table({"k", "b"}, {{1, "p"}, {1, "q"}}), {"k"}, "j", JoinKind.LeftOuter), "j", {"b"})'),
    ("Table.Join/inner", 'Table.Join(#table({"k", "a"}, {{1, "x"}, {2, "y"}}), "k", #table({"k2", "b"}, {{2, "p"}}), "k2")'),
    ("Table.Distinct/basic", '{Table.Distinct(#table({"a"}, {{1}, {1}, {2}})), Table.FillDown(#table({"a"}, {{1}, {null}, {2}}), {"a"}), Table.FillUp(#table({"a"}, {{null}, {2}}), {"a"})}'),
    ("Table.PromoteHeaders/basic", '{Table.PromoteHeaders(#table({"Column1", "Column2"}, {{"x", "y"}, {1, 2}})), Table.DemoteHeaders(#table({"a"}, {{1}}))}'),
    ("Table.Transpose/basic", 'Table.Transpose(#table({"a", "b"}, {{1, 2}, {3, 4}}))'),
    ("Table.ReplaceValue/basic", 'Table.ReplaceValue(#table({"a", "b"}, {{"xa", "x"}}), "x", "z", Replacer.ReplaceText, {"a"})'),
    ("Table.SplitColumn/basic", 'Table.SplitColumn(#table({"a"}, {{"p-q"}, {"r"}}), "a", Splitter.SplitTextByDelimiter("-"), {"a1", "a2"})'),
    ("Table.Combine/basic", 'Table.Combine({#table({"a"}, {{1}}), #table({"b"}, {{2}})})'),
    ("Table.Columns/basic", '{Table.ColumnNames(Table.RenameColumns(#table({"a", "b"}, {}), {"a", "z"})), Table.ColumnNames(Table.ReorderColumns(#table({"a", "b"}, {}), {"b", "a"})), Table.ColumnNames(Table.RemoveColumns(#table({"a", "b"}, {}), "a")), Table.RowCount(#table({"a"}, {{1}, {2}}))}'),
    ("Table.Max/basic", '{Table.Max(#table({"a"}, {{1}, {3}}), "a"), Table.First(#table({"a"}, {})), Table.Column(#table({"a"}, {{1}, {2}}), "a")}'),
    ("Table.ToRows/basic", '{Table.ToRows(#table({"a", "b"}, {{1, 2}})), Table.ToRecords(#table({"a"}, {{1}})), Table.ToColumns(#table({"a", "b"}, {{1, 2}}))}'),
    ("Table.AddIndexColumn/basic", 'Table.AddIndexColumn(#table({"a"}, {{"x"}, {"y"}}), "i", 1, 10)'),
    ("Table.Schema/basic", 'Table.SelectColumns(Table.Schema(#table(type table [a = number, b = text], {})), {"Name", "Kind", "IsNullable"})'),
    ("Table.SelectRows/missing-column", 'Table.SelectRows(#table({"a"}, {{1}}), each [missing36] = 1)'),
    # Table operations (M7.3): joins, grouping, pivots, expansions, splitting, ordering, row edits.
    ("Table.Join/kinds", f'let {_E}, a = #table({{"k", "x"}}, {{{{1, "a"}}, {{2, "b"}}, {{3, "c"}}}}), b = #table({{"k2", "y"}}, {{{{2, "p"}}, {{3, "q"}}, {{3, "r"}}, {{4, "s"}}}}), j = (kind) => Table.Sort(Table.Join(a, "k", b, "k2", kind), {{"k", "k2", "y"}}) in {{j(JoinKind.Inner), j(JoinKind.LeftOuter), j(JoinKind.RightOuter), j(JoinKind.FullOuter), e(try j(JoinKind.LeftAnti)), e(try j(JoinKind.RightAnti)), e(try j(JoinKind.LeftSemi)), e(try j(JoinKind.RightSemi))}}'),
    ("Table.Join/keys", f'let {_E} in {{Table.Join(#table({{"k", "m", "x"}}, {{{{1, "a", 1}}, {{1, "b", 2}}}}), {{"k", "m"}}, #table({{"k2", "m2"}}, {{{{1, "b"}}}}), {{"k2", "m2"}}), Table.Join(#table({{"k", "v1"}}, {{{{1, 2}}}}), "k", #table({{"k", "w1"}}, {{{{1, 3}}}}), "k"), e(try Table.RowCount(Table.Join(#table({{"k"}}, {{{{1}}}}), "k", () => #table({{"j"}}, {{{{1}}}}), "j"))), e(try Table.RowCount(Table.NestedJoin(#table({{"k"}}, {{{{1}}}}), "k", () => #table({{"j"}}, {{{{1}}}}), "j", "n"))), e(try Table.RowCount(Table.Join(#table({{"k1", "v2"}}, {{{{1, 2}}}}), "k1", #table({{"k1", "w2"}}, {{{{1, 3}}}}), "k1", JoinKind.LeftOuter))), e(try Table.RowCount(Table.Join(#table({{"k4", "v4"}}, {{{{1, 2}}}}), "k4", #table({{"j4", "v4"}}, {{{{1, 3}}}}), "j4"))), e(try Table.RowCount(Table.Join(#table({{"k3"}}, {{{{1}}}}), "k3", #table({{"j3"}}, {{{{1}}}}), "j3", JoinKind.Inner, null, {{Comparer.OrdinalIgnoreCase}}))), e(try Table.RowCount(Table.Join(#table({{"k5"}}, {{{{1}}}}), "k5", #table({{"j5"}}, {{{{1}}}}), "missing5")))}}'),
    ("Table.NestedJoin/kinds", f'let {_E}, a = #table({{"k", "x"}}, {{{{1, "a"}}, {{2, "b"}}}}), b = #table({{"k2", "y"}}, {{{{2, "p"}}, {{3, "q"}}}}), n = (kind) => Table.ToRows(Table.TransformColumns(Table.NestedJoin(a, "k", b, "k2", "j", kind), {{"j", Table.ToRows}})) in {{n(JoinKind.Inner), n(JoinKind.LeftOuter), n(JoinKind.LeftAnti), e(try n(JoinKind.LeftSemi)), Table.ToRows(Table.TransformColumns(Table.NestedJoin(a, "k", b, "k2", "j"), {{"j", Table.ToRows}})), e(try Table.NestedJoin(a, "k", b, "k2", "x", JoinKind.Inner))}}'),
    ("Table.NestedJoin/right", f'let a = #table({{"k", "x"}}, {{{{1, "a"}}, {{2, "b"}}}}), b = #table({{"k2", "y"}}, {{{{2, "p"}}, {{3, "q"}}}}), n = (kind) => List.Sort(Table.ToRows(Table.ExpandTableColumn(Table.NestedJoin(a, "k", b, "k2", "j", kind), "j", {{"k2", "y"}})), (p, q) => Value.Compare(Text.From(p{{3}}) & Text.From(p{{0}}), Text.From(q{{3}}) & Text.From(q{{0}}))) in {{n(JoinKind.RightOuter), n(JoinKind.FullOuter), n(JoinKind.RightAnti)}}'),
    ("Table.AddJoinColumn/basic", 'Table.ToRows(Table.TransformColumns(Table.AddJoinColumn(#table({"k"}, {{1}, {2}}), "k", #table({"k2", "v"}, {{1, "a"}, {1, "b"}}), "k2", "j"), {"j", Table.ToRows}))'),
    ("Table.Group/options", f'let t = #table({{"g", "v"}}, {{{{"a", 1}}, {{"A", 2}}, {{"b", 3}}, {{"a", 4}}}}) in {{Table.Group(t, "g", {{"n", each Table.RowCount(_)}}), Table.Group(t, "g", {{"n", each Table.RowCount(_)}}, GroupKind.Local), Table.Group(t, "g", {{"s", each List.Sum([v])}}, GroupKind.Global, Comparer.OrdinalIgnoreCase), Table.Group(t, {{"g"}}, {{{{"s", each List.Sum([v]), type number}}, {{"m", each List.Max([v])}}}}), Table.Group(t, {{}}, {{"n", each Table.RowCount(_)}}), Table.Group(t, "g", {{}})}}'),
    ("Table.Group/custom-comparer", '{Table.Group(#table({"v"}, {{1}, {2}, {5}, {6}, {9}}), "v", {"n", each Table.RowCount(_)}, GroupKind.Local, (a, b) => Number.From(b[v] - a[v] > 1)), Table.Group(#table({"v"}, {{1}, {2}, {1}}), "v", {"n", each Table.RowCount(_)}, GroupKind.Global, (a, b) => 0)}'),
    ("Table.Group/types", f'let {_S}, t = #table(type table [g = text, v = number], {{{{"a", 1}}}}) in s(Table.Group(t, "g", {{{{"n", each Table.RowCount(_)}}, {{"s", each List.Sum([v])}}, {{"t", each _}}, {{"x", each 1, Int64.Type}}, {{"c", Table.RowCount}}, {{"z", each Text.Combine([g])}}}}))'),
    ("Table.Pivot/options", f'let {_E}, t = #table({{"r", "attr", "val"}}, {{{{1, "x", 10}}, {{1, "y", 20}}, {{2, "x", 30}}, {{2, "x", 31}}, {{2, "z", 40}}, {{3, null, 50}}}}) in {{Table.Pivot(t, {{"x", "y"}}, "attr", "val", List.Sum), Table.Pivot(Table.FirstN(t, 3), {{"x", "y", "w"}}, "attr", "val"), e(try Table.Pivot(t, {{"x", "y"}}, "attr", "val"){{1}}[x]), e(try Table.Pivot(#table({{"a", "v"}}, {{{{1, 2}}}}), {{1}}, "a", "v")), e(try Table.Pivot(t, {{"x", "x"}}, "attr", "val"))}}'),
    ("Table.Pivot/types", f'let {_S}, t = #table(type table [r = number, a = text, v = Int64.Type], {{{{1, "x", 2}}}}) in {{s(Table.Pivot(t, {{"x"}}, "a", "v")), s(Table.Pivot(t, {{"x"}}, "a", "v", List.Sum))}}'),
    ("Table.Unpivot/options", f'let {_E}, {_S}, t = #table(type table [id = number, a = number, b = nullable number, c = text], {{{{1, 2, null, "p"}}, {{2, 3, 4, "q"}}}}) in {{Table.Unpivot(t, {{"b", "a"}}, "Attribute", "Value"), s(Table.Unpivot(t, {{"a", "b"}}, "Attribute", "Value")), s(Table.Unpivot(t, {{"a", "c"}}, "Attribute", "Value")), Table.UnpivotOtherColumns(t, {{"id", "c"}}, "k", "v"), e(try Table.UnpivotOtherColumns(t, {{"id", "zz6"}}, "k", "v")), e(try Table.Unpivot(t, {{"a", "zz7"}}, "k", "v")), e(try Table.Unpivot(t, {{"a"}}, "id", "v"))}}'),
    ("Table.ExpandTableColumn/options", f'let {_E}, {_S}, t = #table({{"k", "n"}}, {{{{1, #table(type table [a = number, b = text], {{{{1, "p"}}, {{3, "q"}}}})}}, {{2, #table({{"a", "b"}}, {{}})}}, {{3, null}}}}), u = #table(type table [k = number, n = table [a = number, b = text]], {{}}) in {{Table.ExpandTableColumn(t, "n", {{"a"}}), Table.ExpandTableColumn(t, "n", {{"a", "b"}}, {{"x", "y"}}), s(Table.ExpandTableColumn(u, "n", {{"a", "b"}})), e(try Table.ExpandTableColumn(t, "n", {{"a"}}, {{"k"}})), e(try Table.ExpandTableColumn(t, "n", {{"a", "q8"}}))}}'),
    ("Table.ExpandRecordColumn/basic", f'let {_E}, {_S} in {{Table.ExpandRecordColumn(#table({{"k", "r"}}, {{{{1, [a = 1, b = 2]}}, {{2, [a = 3]}}, {{3, null}}}}), "r", {{"a", "b"}}), Table.ExpandRecordColumn(#table({{"r"}}, {{{{[a = 1]}}}}), "r", {{"a"}}, {{"z"}}), s(Table.ExpandRecordColumn(#table(type table [r = [a = number, b = text]], {{}}), "r", {{"a", "b"}})), e(try Table.ExpandRecordColumn(#table({{"r", "a"}}, {{}}), "r", {{"a"}}))}}'),
    ("Table.ExpandListColumn/basic", f'let {_E} in {{Table.ExpandListColumn(#table({{"k", "l"}}, {{{{1, {{1, 2}}}}, {{2, {{}}}}, {{3, null}}}}), "l"), e(try Table.ExpandListColumn(#table({{"l"}}, {{{{1}}}}), "l"){{0}}[l])}}'),
    ("Table.AggregateTableColumn/basic", f'let {_S}, t = #table({{"k", "n"}}, {{{{1, #table({{"v"}}, {{{{1}}, {{2}}}})}}, {{2, #table({{"v"}}, {{}})}}}}) in {{Table.AggregateTableColumn(t, "n", {{{{"v", List.Sum, "sum"}}, {{"v", List.Count, "count"}}}}), s(Table.AggregateTableColumn(t, "n", {{{{"v", List.Sum, "sum"}}}}))}}'),
    ("Table.Transpose/options", f'let {_E} in {{Table.Transpose(#table({{"a", "b"}}, {{{{1, 2}}, {{3, 4}}, {{5, 6}}}}), {{"x", "y", "z"}}), Table.Transpose(#table({{"a", "b"}}, {{{{1, 2}}}})), Table.Transpose(#table({{"a"}}, {{}})), e(try Table.Transpose(#table({{"a"}}, {{{{1}}, {{2}}}}), {{"x7"}}))}}'),
    ("Table.SplitColumn/options", f'let {_E}, {_S}, t = #table({{"s", "k"}}, {{{{"a,b,c", 1}}, {{"d", 2}}, {{null, 3}}}}), d = Splitter.SplitTextByDelimiter(",") in {{Table.SplitColumn(t, "s", d), Table.SplitColumn(t, "s", d, 2), Table.SplitColumn(t, "s", d, {{"x", "y"}}, "-"), Table.SplitColumn(t, "s", d, 2, null, ExtraValues.List), e(try Record.Field(Table.SplitColumn(t, "s", d, 2, null, ExtraValues.Error){{0}}, "s.2")), e(try Record.Field(Table.SplitColumn(t, "s", d, 2, null, ExtraValues.Error){{0}}, "s.1")), s(Table.SplitColumn(t, "s", d, 2))}}'),
    ("Table.CombineColumns/basic", '{Table.CombineColumns(#table({"a", "b", "c"}, {{"x", "y", 1}, {null, "z", 2}}), {"a", "b"}, Combiner.CombineTextByDelimiter("-"), "ab"), Table.CombineColumnsToRecord(#table({"a", "b", "c"}, {{1, 2, 3}}), "r", {"a", "c"}), Table.CombineColumnsToRecord(#table({"a", "b", "c"}, {{1, 2, 3}}), "r", {"b", "a"}), Table.CombineColumnsToRecord(#table({"a", "b", "c"}, {{1, 2, 3}}), "r", {"c"}), Table.CombineColumnsToRecord(#table({"a", "b", "c", "d"}, {{1, 2, 3, 4}}), "r", {"c", "a"}), Table.CombineColumns(#table({"a", "b", "c", "d"}, {{"1", "2", "3", "4"}}), {"c", "a"}, Combiner.CombineTextByDelimiter("+"), "ca"), Table.CombineColumns(#table({"a", "b", "c", "d"}, {{"1", "2", "3", "4"}}), {"a", "c"}, Combiner.CombineTextByDelimiter("+"), "ac"), Table.CombineColumnsToRecord(#table({"a", "b", "c", "d"}, {{1, 2, 3, 4}}), "r", {"b", "d", "a"})}'),
    ("Table.Max/options", 'let t = #table({"a", "b"}, {{1, "x"}, {3, "y"}, {3, "z"}, {2, "w"}}) in {Table.Max(t, "a"), Table.Min(t, "a"), Table.Min(#table({"a", "b"}, {{1, "x"}, {1, "y"}}), "a"), Table.Max(#table({"a"}, {}), "a", 0), Table.MaxN(t, "a", 2), Table.MinN(t, "a", 2), Table.MaxN(t, "a", each [a] > 2), Table.Min(t, {"a", Order.Descending})}'),
    ("Table.AddRankColumn/kinds", f'let {_S}, t = #table({{"n", "v"}}, {{{{"a", 3}}, {{"b", 1}}, {{"c", 3}}, {{"d", 2}}}}) in {{Table.AddRankColumn(t, "r", {{"v", Order.Descending}}), Table.AddRankColumn(t, "r", {{"v", Order.Descending}}, [RankKind = RankKind.Dense]), Table.AddRankColumn(t, "r", "v", [RankKind = RankKind.Ordinal]), s(Table.AddRankColumn(t, "r", "v"))}}'),
    ("Table.Schema/types", 'Table.Schema(#table(type table [a = Int64.Type, b = nullable text, c = any, d = date, e = table, f = list, g = Currency.Type, h = record], {}))'),
    ("Table.ColumnsOfType/basic", 'let t = #table(type table [a = number, b = nullable number, c = text, d = Int64.Type, e = any], {}) in {Table.ColumnsOfType(t, {type number}), Table.ColumnsOfType(t, {type nullable number}), Table.ColumnsOfType(t, {type text, Int64.Type}), Table.ColumnsOfType(t, {type any})}'),
    ("Table.ToList/basic", f'let {_E} in {{Table.ApproximateRowCount(#table({{"a"}}, {{{{1}}, {{2}}}})), Table.ToList(#table({{"a", "b"}}, {{{{"x", "y"}}, {{"p", null}}}})), Table.ToList(#table({{"a", "b"}}, {{{{1, 2}}}}), each List.Sum(_)), e(try Table.ToList(#table({{"a", "b"}}, {{{{18, 2}}}})){{0}}), Table.FirstValue(#table({{"a", "b"}}, {{{{1, 2}}}})), Table.FirstValue(#table({{"a"}}, {{}}), "d"), Table.PrefixColumns(#table({{"a", "b"}}, {{{{1, 2}}}}), "p")}}'),
    ("Table.FromValue/kinds", '{Table.FromValue([a = 1, b = "x"]), Table.FromValue({1, 2}), Table.FromValue(5), Table.FromValue(#table({"q"}, {{1}})), Table.FromValue({1}, [DefaultColumnName = "c"]), Table.FromValue([a = 1], [DefaultColumnName = "c"])}'),
    ("Table.Rows/reorganize", 'let t = #table({"a"}, {{1}, {2}, {3}, {4}, {5}}) in {Table.ReverseRows(t), Table.Repeat(#table({"a"}, {{1}, {2}}), 2), Table.Repeat(t, 0), Table.AlternateRows(t, 1, 1, 2), Table.AlternateRows(t, 0, 2, 1), Table.RemoveLastN(t, 2), Table.RemoveLastN(t, each [a] > 3), Table.SingleRow(#table({"a"}, {{7}})), List.Transform(Table.Split(t, 2), Table.ToRows), List.Transform(Table.SplitAt(t, 2), Table.ToRows)}'),
    ("Table.Partition/basic", 'List.Transform(Table.Partition(#table({"a"}, {{1}, {2}, {3}, {4}, {5}, {-1}, {2.5}}), "a", 2, each _), Table.ToRows)'),
    ("Table.Rows/errors", f'let {_E}, t = #table({{"a"}}, {{{{1}}, {{2}}}}) in {{e(try Table.Split(t, 0)), e(try Table.SingleRow(t)), e(try Table.SingleRow(#table({{"a"}}, {{}}))), e(try Table.Repeat(t, -1)), e(try Table.AlternateRows(t, -1, 1, 1)), e(try Table.Partition(t, "a", 0, each _))}}'),
    ("Table.InsertRows/edges", f'let {_E}, {_RW}, t = #table({{"a", "b"}}, {{{{1, 2}}, {{3, 4}}}}) in {{e(try rw(Table.InsertRows(t, 1, {{[a = 9, b = 8]}}))), e(try rw(Table.InsertRows(t, 2, {{[b = 7, a = 6]}}))), e(try rw(Table.InsertRows(t, 1, {{[b = 7, a = 6, c = 5]}}))), e(try rw(Table.InsertRows(t, 3, {{[a = 1, b = 1]}}))), e(try rw(Table.InsertRows(t, 0, {{[a = 10]}}))), e(try rw(Table.RemoveRows(t, 0))), e(try rw(Table.RemoveRows(t, 1, 1))), e(try rw(Table.RemoveRows(t, 1, 5))), e(try rw(Table.RemoveRows(t, 2))), e(try rw(Table.RemoveRows(t, 2, 0))), e(try rw(Table.ReplaceRows(t, 0, 1, {{[a = 0, b = 0]}}))), e(try rw(Table.ReplaceRows(t, 1, 2, {{}}))), e(try rw(Table.ReplaceRows(t, 2, 0, {{[a = 5, b = 5]}})))}}'),
    ("Table.MatchingRows/basic", f'let {_E}, {_RW}, t = #table({{"a", "b"}}, {{{{1, "x"}}, {{2, "Y"}}, {{1, "x"}}}}) in {{e(try rw(Table.RemoveMatchingRows(t, {{[a = 1, b = "x"]}}))), e(try rw(Table.RemoveMatchingRows(t, {{[a = 1]}}))), e(try rw(Table.RemoveMatchingRows(t, {{[a = 1]}}, "a"))), e(try rw(Table.RemoveMatchingRows(t, {{[b = "y"]}}, {{"b", Comparer.OrdinalIgnoreCase}}))), e(try rw(Table.ReplaceMatchingRows(t, {{[a = 1, b = "x"], [a = 5, b = "z"]}}))), e(try rw(Table.ReplaceMatchingRows(t, {{{{[a = 2], [a = 6]}}}}, "a"))), e(try rw(Table.ReplaceMatchingRows(t, {{{{[a = 2, b = "Y"], [b = "q"]}}}}))), e(try rw(Table.ReplaceMatchingRows(t, {{{{[a = 2], [a = 6, b = "w"]}}}}, "a"))), e(try rw(Table.ReplaceMatchingRows(t, {{{{[a = 2], [b = "w", a = 6, c = 0]}}}}, "a"))), e(try rw(Table.ReplaceMatchingRows(t, {{{{[a = 2, b = "Y"], [a = 7, b = "v"]}}}})))}}'),
    ("Table.Contains/basic", f'let {_E}, t = #table({{"a", "b"}}, {{{{1, "x"}}, {{2, "Y"}}, {{1, "z"}}}}) in {{Table.Contains(t, [a = 1]), Table.Contains(t, [a = 1, b = "y"]), Table.Contains(t, [b = "y"], {{"b", Comparer.OrdinalIgnoreCase}}), Table.Contains(t, [c = 1]), Table.Contains(t, [a = 2, c = 1], "a"), e(try Table.Contains(t, [a = 2], "zz9")), e(try Table.Contains(t, [a = 2], "b")), e(try Table.Contains(t, [zz = 1], "zz")), e(try Table.Contains(t, [a = 1, b = "x"], {{"a", "zz2"}})), Table.ContainsAll(t, {{[a = 1], [a = 2]}}), Table.ContainsAll(t, {{[a = 1], [a = 3]}}), Table.ContainsAny(t, {{[a = 3], [b = "z"]}}), Table.ContainsAny(t, {{}}), Table.ContainsAll(t, {{}})}}'),
    ("Table.PositionOf/basic", 'let t = #table({"a", "b"}, {{1, "x"}, {2, "Y"}, {1, "z"}}) in {Table.PositionOf(t, [a = 1]), Table.PositionOf(t, [a = 1], Occurrence.Last), Table.PositionOf(t, [a = 1], Occurrence.All), Table.PositionOf(t, [a = 9]), Table.PositionOf(t, [b = "y"], Occurrence.First, {"b", Comparer.OrdinalIgnoreCase}), Table.PositionOfAny(t, {[a = 2], [b = "z"]}), Table.PositionOfAny(t, {[a = 2], [b = "z"]}, Occurrence.All), Table.PositionOf(t, [a = 1, b = "z"], Occurrence.First, "a")}'),
    ("Table.FindText/kinds", 'Table.ToRows(Table.FindText(#table({"i", "v"}, {{0, "x1"}, {1, 1}, {2, {"1"}}, {3, {{"1"}}}, {4, [f = "1"]}, {5, #table({"q"}, {{"1"}})}, {6, Text.ToBinary("1")}, {7, #date(2001, 1, 1)}, {8, "1" meta [m = 1]}, {9, {2, "a1b"}}}), "1"))'),
    ("Table.TransformColumnTypes/structured", f'let {_E}, t = #table({{"n"}}, {{{{1}}}}) in {{e(try Table.RowCount(Table.TransformColumnTypes(t, {{"n", type table [a = number]}}))), e(try Table.TransformColumnTypes(t, {{"n", type list}}){{0}}[n]), e(try Table.TransformColumnTypes(t, {{"n", type any}}){{0}}[n]), e(try Table.TransformColumnTypes(t, {{"n", type record}}){{0}}[n]), e(try Table.TransformColumnTypes(t, {{"n", type function}}){{0}}[n])}}'),
    ("Table.Partition/hash", 'List.Transform(Table.Partition(#table({"a"}, {{"b"}, {"cc"}, {"d"}}), "a", 3, Text.Length), Table.ToRows)'),
    ("Table.IsDistinct/basic", 'let t = #table({"a", "b"}, {{1, "x"}, {2, "Y"}, {1, "y"}}) in {Table.IsDistinct(t), Table.IsDistinct(t, "a"), Table.IsDistinct(t, {"b", Comparer.OrdinalIgnoreCase}), Table.MatchesAllRows(t, each [a] > 0), Table.MatchesAnyRows(t, each [a] > 1), Table.MatchesAllRows(#table({"a"}, {}), each false), Table.FindText(#table({"a", "b"}, {{1, "abc"}, {12, "x"}, {null, #date(2024, 1, 2)}, {true, {"1"}}}), "1"), Table.FindText(t, "Y")}'),
    # Values, logicals, types, binary, JSON.
    ("Value/basic", '{Value.Is(1, type number), Value.Equals(1, 1.0), Value.Compare(1, 2), Value.Type(1), Value.Type("x"), Logical.FromText("TRUE"), Logical.ToText(false)}'),
    ("Value.Metadata/basic", 'Value.Metadata(1 meta [a = 1])'),
    ("Type/basic", '{Type.IsNullable(type nullable text), Type.NonNullable(type nullable number), Type.TableColumn(type table [a = text], "a"), Type.Is(type number, type any)}'),
    ("Binary/basic", '{Text.ToBinary("hé"), Binary.ToText(Text.ToBinary("abc"), BinaryEncoding.Hex), Binary.FromText("AQI="), Binary.Length(#binary({1, 2, 3})), Text.FromBinary(#binary({104, 105}))}'),
    ("Binary/functions", '{Binary.ToText(#binary({1, 2, 255})), Binary.ToText(#binary({1, 2, 255}), BinaryEncoding.Hex), Binary.ToList(Binary.FromText("0A0b", BinaryEncoding.Hex)), Binary.ToList(Binary.FromText(" AQ I= ")), Binary.ToList(Binary.From("AQI=")), Binary.From(null), Binary.ToList(Binary.Combine({#binary({1}), #binary({2, 3})})), Binary.ToList(Binary.Range(#binary({1, 2, 3, 4}), 1, 2)), Binary.ToList(Binary.Range(#binary({1, 2, 3, 4}), 2, 5)), List.Transform(Binary.Split(#binary({1, 2, 3, 4, 5}), 2), Binary.ToList), Binary.ApproximateLength(#binary({1, 2, 3})), Binary.ToList(Binary.Buffer(#binary({9}))), Binary.ToList(Binary.FromList({0, 255}))}'),
    ("Binary/errors", 'let e = (r) => if r[HasError] then r[Error][Reason] & ": " & r[Error][Message] else r[Value] in {e(try Binary.Length(Binary.FromText("012", BinaryEncoding.Hex))), e(try Binary.Length(Binary.FromText("0g", BinaryEncoding.Hex))), e(try Binary.Length(Binary.FromText("AQI"))), e(try Binary.Length(Binary.From(1))), e(try Binary.Length(Binary.FromList({1, 256}))), e(try Binary.Length(Binary.FromList({1.5}))), e(try Binary.Length(Binary.Range(#binary({1}), -1))), e(try Binary.Split(#binary({1}), 0)), e(try Binary.Length(Binary.Combine({null}))), e(try Binary.Length(Binary.ToText(#binary({1}), 5)))}'),
    ("Json.Document/basic", 'Json.Document("{""a"":[1,2.5,{""b"":null}],""c"":true}")'),
    ("Json.FromValue/basic", 'Text.FromBinary(Json.FromValue([a = {1, "x"}, b = null, c = #date(2024, 1, 2)]))'),
    ("Json.FromValue/kinds", 'Text.FromBinary(Json.FromValue({1e21, 1e-7, -0, 1.5, 0 / 0, "a/b\\""c", "é#(lf)", #datetime(2024, 1, 2, 3, 4, 5.25), #time(1, 2, 3), #datetimezone(2024, 1, 2, 3, 4, 5, 1, 0), #duration(1, 2, 3, 4), -#duration(0, 23, 59, 59.5), #binary({1, 2}), true, #table({"a"}, {{1}, {2}})}))'),
    ("Json.FromValue/utf16", 'Binary.ToText(Json.FromValue("é", TextEncoding.Utf16), BinaryEncoding.Hex)'),
    ("Json.FromValue/errors", 'let e = (r) => if r[HasError] then r[Error][Reason] & ": " & r[Error][Message] else r[Value] in {e(try Binary.Length(Json.FromValue(each _))), e(try Binary.Length(Json.FromValue(type text)))}'),
    ("constants/enums", '{JoinKind.LeftSemi, JoinKind.RightSemi, JoinAlgorithm.SortMerge, JoinSide.Right, PercentileMode.SqlCont, Occurrence.Repeating, BinaryOccurrence.Required, BinaryEncoding.Hex, Type.Is(Day.Type, type number)}'),
    ("Comparer/basic", '{Comparer.Ordinal("a", "B"), Comparer.OrdinalIgnoreCase("a", "B"), Comparer.Equals(Comparer.OrdinalIgnoreCase, "a", "A")}'),
    ("Lines/basic", '{Lines.FromText("a#(cr,lf)b#(lf)c"), Lines.ToText({"a", "b"}, "|")}'),
    ("Csv.Document/basic", 'Csv.Document("a,b#(lf)1,""x,y""#(lf)", [Delimiter = ",", Columns = 2, QuoteStyle = QuoteStyle.Csv])'),
    # Operators and language.
    ("operators/arithmetic", '{1 + 2 * 3, 7 / 2, "a" & "b", {1} & {2}, [a = 1] & [b = 2], null + 1, 0.1 + 0.2}'),
    ("operators/comparison", '{1 < 2, "a" < "B", null = null, null < 1, #date(2024, 1, 1) < #date(2024, 1, 2), {1, 2} = {1, 2}, [a = 1, b = 2] = [b = 2, a = 1]}'),
    ("operators/logical", '{true and null, false and (error "never-37"), true or (error "never-38"), not true}'),
    # `error` is an expression, not an operand: Desktop rejects it unparenthesized.
    ("syntax/error-operand", 'false and error "never-37b"'),
    ("operators/type-error", '1 + "x39"'),
    ("language/let-each", 'let f = each _ + 1, g = (x, optional y) => x + (if y = null then 0 else y) in {f(1), g(1), g(1, 2)}'),
    ("language/recursion", 'let fact = (n) => if n <= 1 then 1 else n * @fact(n - 1) in fact(10)'),
    ("language/field-access", 'let r = [a = [b = 2]] in {r[a][b], r[[a]], #table({"x", "y"}, {{1, 2}}){0}[[x]]}'),
    ("language/meta", 'Value.Metadata(Value.ReplaceMetadata(1, [m = "x"]))'),
    ("language/is-as", '{1 is number, null is nullable text, "a" as text, 1 is any}'),
    ("language/as-error", '"a40" as number'),
    # Names: what Desktop does with names that are not defined, M's exclusive scoping (a let step
    # or record field cannot see itself without `@`) and cycles. Each case is its own expression:
    # an undefined name may fail the whole expression.
    ("names/unknown", 'Undefined1'),
    ("names/unknown-unused-step", 'let a = Undefined2 in 2'),
    ("names/unknown-in-try", 'try Undefined3 otherwise 3'),
    ("names/unknown-in-untaken-branch", 'if true then 4 else Undefined4'),
    ("names/unknown-in-function-body", 'let f = (x) => Undefined5 + x in 5'),
    ("names/unknown-in-each", 'let f = each Undefined6 in 6'),
    ("names/unknown-in-catch", 'try Undefined7 catch (e) => e[Message]'),
    ("names/unknown-in-type", 'type table [a = Undefined8]'),
    ("names/unknown-library-member", 'Table.RowCnt'),
    ("names/case-sensitive", 'text.upper("a")'),
    ("names/sibling-fields", '[a = 1, b = a + 1][b]'),
    ("names/exclusive-let", 'let a = 1, r = let a = a + 1 in a in r'),
    ("names/exclusive-record", 'let a = 1 in [a = a + 1][a]'),
    ("names/self-reference", 'let x9 = x9 in x9'),
    ("names/self-reference-unused", 'let x10 = x10 in 10'),
    ("names/recursion-without-at", 'let f11 = (n) => if n = 0 then 1 else n * f11(n - 1) in f11(3)'),
    ("names/inclusive-cycle", 'let x12 = @x12 + 1 in x12'),
    ("names/let-cycle", 'let a13 = b13, b13 = a13 in a13'),
    ("names/let-cycle-unused", 'let a14 = b14, b14 = a14 in 14'),
    ("names/step-shadows-library", 'let Text.Upper = (x) => x & "!" in Text.Upper("a", "b", "c")'),
    ("names/parameter-shadows-library", '((List.Sum) => List.Sum)(15)'),
    ("names/each-underscore", 'List.Transform({1, 2}, each _ + 1)'),
    # Argument counts.
    ("arity/too-few", 'Text.Upper()'),
    ("arity/too-many", 'Text.Upper("a", "en-US", 3)'),
    ("arity/unused", 'let f = () => Text.Upper() in 16'),
    ("arity/in-try", 'try Text.Lower("a", "b", "c") otherwise 17'),
    ("arity/exact", 'Number.Round(1.25, 1, RoundingMode.Up)'),
    ("arity/local-function", 'let f = (a, optional b) => a in f(1, 2, 3, 4)'),
    ("arity/each", '(each _)(1, 2)'),
]


def m_functions_suite() -> dict:
    return m_suite(
        "m-functions",
        "M standard library functions, operators and errors evaluated by Power BI Desktop's mashup engine",
        [{"id": i, "query": q} for i, q in M_FUNCTIONS],
    )


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
        m_library_suite(),
        m_arity_suite(),
        m_functions_suite(),
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
