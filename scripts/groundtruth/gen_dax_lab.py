#!/usr/bin/env python3
"""Generate the "dax-lab" PBIP fixture of the Desktop ground truth (ADR-0011).

The DAX function tests (`crates/pbi-dax/tests/functions.rs`) run on two small hand-built models:
`fx()` (`Sales` -> `Product`, an unmatched product key P9, a blank quantity and date, the
calculated column `Line`) and `dates_fx()` (`Date` 2023-2024, `Orders` with an active and an
inactive date relationship, the parent-child table `Emp`). This script writes the same tables,
columns, measures and relationships as one PBIP with M `#table` partitions, so Power BI Desktop
and DashCraft evaluate the extracted cases (reference/groundtruth/corpus/dax-functions.json)
on identical data. The intentionally broken test measures (cycles, depth, syntax errors) are left
out: Desktop would refuse to load them.

Deterministic: stable ids are uuid5 of the object path.

Usage: python scripts/groundtruth/gen_dax_lab.py [--out DIR]
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_OUT = ROOT / "reference" / "groundtruth" / "fixtures" / "dax-lab"
NS = uuid.UUID("6f1c2a52-6d0a-4b8e-9a59-0d1f3c5b7a10")
T = "\t"


def tag(*path: str) -> str:
    return str(uuid.uuid5(NS, "/".join(path)))


def q(name: str) -> str:
    """TMDL name, quoted when needed."""
    return name if name.replace("_", "").isalnum() else "'" + name.replace("'", "''") + "'"


def m_value(v) -> str:
    if v is None:
        return "null"
    if isinstance(v, str):
        return '"' + v.replace('"', '""') + '"'
    if isinstance(v, tuple):
        y, mo, d = v
        return f"#datetime({y}, {mo}, {d}, 0, 0, 0)"
    if isinstance(v, float):
        return repr(v)
    return str(v)


M_TYPES = {"string": "text", "int64": "Int64.Type", "double": "number", "dateTime": "datetime"}
SUMMARIZE = {"string": "none", "int64": "sum", "double": "sum", "dateTime": "none"}


def literal_partition(columns: list[tuple[str, str]], rows: list[list]) -> list[str]:
    typ = ", ".join(f"#\"{c}\" = {M_TYPES[t]}" if " " in c else f"{c} = {M_TYPES[t]}" for c, t in columns)
    body = ", ".join("{" + ", ".join(m_value(v) for v in r) + "}" for r in rows)
    return ["let", f"    Source = #table(type table [{typ}], {{{body}}})", "in", "    Source"]


def table_tmdl(
    name: str,
    description: str,
    columns: list[tuple[str, str]],
    m: list[str],
    measures: list[tuple[str, str]] = (),
    calculated: list[tuple[str, str, str]] = (),
) -> str:
    out = [f"/// {description}", f"table {q(name)}", f"{T}lineageTag: {tag(name)}", ""]
    for mname, expr in measures:
        out += [f"{T}measure {q(mname)} = {expr}", f"{T}{T}lineageTag: {tag(name, 'measure', mname)}", ""]
    for cname, ctype in columns:
        out += [
            f"{T}column {q(cname)}",
            f"{T}{T}dataType: {ctype}",
            f"{T}{T}lineageTag: {tag(name, 'column', cname)}",
            f"{T}{T}summarizeBy: {SUMMARIZE[ctype]}",
            f"{T}{T}sourceColumn: {cname}",
            "",
            f"{T}{T}annotation SummarizationSetBy = Automatic",
            "",
        ]
    for cname, ctype, expr in calculated:
        out += [
            f"{T}column {q(cname)} = {expr}",
            f"{T}{T}dataType: {ctype}",
            f"{T}{T}lineageTag: {tag(name, 'column', cname)}",
            f"{T}{T}summarizeBy: {SUMMARIZE[ctype]}",
            "",
            f"{T}{T}annotation SummarizationSetBy = Automatic",
            "",
        ]
    out += [f"{T}partition {q(name)} = m", f"{T}{T}mode: import", f"{T}{T}source ="]
    out += [f"{T}{T}{T}{T}{line}" for line in m]
    out += ["", f"{T}annotation PBI_ResultType = Table", ""]
    return "\n".join(out)


def tables() -> dict[str, str]:
    product_cols = [("ProductKey", "string"), ("Name", "string"), ("Color", "string"), ("Category", "string")]
    product_rows = [
        ["P1", "Apple", "Red", "Fruit"],
        ["P2", "Banana", "Yellow", "Fruit"],
        ["P3", "Carrot", "Orange", "Veg"],
        ["P4", "Cherry", "Red", "Fruit"],
    ]
    sales_cols = [("Id", "int64"), ("ProductKey", "string"), ("Qty", "int64"), ("Price", "double"), ("Date", "dateTime")]
    sales_rows = [
        [1, "P1", 3, 1.5, (2024, 1, 5)],
        [2, "P1", 2, 1.5, (2024, 2, 10)],
        [3, "P2", 5, 0.25, (2024, 1, 20)],
        [4, "P3", 1, 2.0, (2024, 3, 1)],
        [5, "P9", 4, 3.0, (2024, 3, 15)],
        [6, "P2", None, 0.5, None],
    ]
    orders_cols = [
        ("OrderId", "int64"),
        ("OrderDate", "dateTime"),
        ("ShipDate", "dateTime"),
        ("Amount", "int64"),
        ("Region", "string"),
    ]
    order_dates = [(2023, 1, 10), (2023, 2, 20), (2023, 12, 31), (2024, 1, 15), (2024, 2, 10), (2024, 2, 29), (2024, 3, 31), (2024, 6, 30)]
    ship_dates = [(2023, 1, 15), (2023, 3, 2), (2024, 1, 2), (2024, 2, 1), (2024, 2, 12), (2024, 3, 1), (2024, 4, 2), (2024, 7, 1)]
    orders_rows = [
        [i + 1, order_dates[i], ship_dates[i], (i + 1) * 10, "East" if i % 2 == 0 else "West"] for i in range(8)
    ]
    date_m = [
        "let",
        "    Days = List.Dates(#date(2023, 1, 1), 731, #duration(1, 0, 0, 0)),",
        '    Source = Table.FromList(Days, Splitter.SplitByNothing(), {"Date"}),',
        '    Typed = Table.TransformColumnTypes(Source, {{"Date", type datetime}}),',
        '    Year = Table.AddColumn(Typed, "Year", each Date.Year([Date]), Int64.Type),',
        '    Month = Table.AddColumn(Year, "Month", each Date.Month([Date]), Int64.Type)',
        "in",
        "    Month",
    ]
    return {
        "Product": table_tmdl(
            "Product",
            "functions.rs fx(): four products; Sales has an unmatched key (P9), so Product gets a blank row.",
            product_cols,
            literal_partition(product_cols, product_rows),
            measures=[("Product Count", "COUNTROWS(Product)")],
        ),
        "Sales": table_tmdl(
            "Sales",
            "functions.rs fx(): six sales lines, one blank quantity and one blank date.",
            sales_cols,
            literal_partition(sales_cols, sales_rows),
            measures=[
                ("Total Qty", "SUM(Sales[Qty])"),
                ("Rows", "COUNTROWS(Sales)"),
                ("Red Qty", 'CALCULATE([Total Qty], Product[Color] = "Red")'),
            ],
            calculated=[("Line", "double", "Sales[Qty] * [Price]")],
        ),
        "Date": table_tmdl(
            "Date",
            "functions.rs dates_fx(): every day of 2023 and 2024.",
            [("Date", "dateTime"), ("Year", "int64"), ("Month", "int64")],
            date_m,
        ),
        "Orders": table_tmdl(
            "Orders",
            "functions.rs dates_fx(): eight orders; OrderDate is the active date relationship, ShipDate the inactive one.",
            orders_cols,
            literal_partition(orders_cols, orders_rows),
            measures=[("Amt", "SUM(Orders[Amount])")],
        ),
        "Emp": table_tmdl(
            "Emp",
            "functions.rs dates_fx(): a parent-child hierarchy for PATH.",
            [("Id", "int64"), ("Parent", "int64")],
            literal_partition([("Id", "int64"), ("Parent", "int64")], [[1, None], [2, 1], [3, 2], [4, 1]]),
        ),
    }


RELATIONSHIPS = [
    ("Sales", "ProductKey", "Product", "ProductKey", True),
    ("Orders", "OrderDate", "Date", "Date", True),
    ("Orders", "ShipDate", "Date", "Date", False),
]


def relationships_tmdl() -> str:
    out = []
    for ft, fc, tt, tc, active in RELATIONSHIPS:
        out.append(f"relationship {tag('rel', ft, fc, tt, tc)}")
        if not active:
            out.append(f"{T}isActive: false")
        out += [f"{T}fromColumn: {q(ft)}.{q(fc)}", f"{T}toColumn: {q(tt)}.{q(tc)}", ""]
    return "\n".join(out)


def write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8", newline="\n")


def write_json(path: Path, doc) -> None:
    write(path, json.dumps(doc, indent=2) + "\n")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT)
    out: Path = ap.parse_args().out.resolve()
    if out.exists():
        shutil.rmtree(out)
    model = out / "DaxLab.SemanticModel"
    report = out / "DaxLab.Report"
    schemas = "https://developer.microsoft.com/json-schemas/fabric"

    write(out / ".gitignore", "**/.pbi/localSettings.json\n**/.pbi/cache.abf\n")
    write_json(
        out / "DaxLab.pbip",
        {
            "$schema": f"{schemas}/pbip/pbipProperties/1.0.0/schema.json",
            "version": "1.0",
            "artifacts": [{"report": {"path": "DaxLab.Report"}}],
            "settings": {"enableAutoRecovery": False},
        },
    )
    write_json(
        model / ".platform",
        {
            "$schema": f"{schemas}/gitIntegration/platformProperties/2.0.0/schema.json",
            "metadata": {"type": "SemanticModel", "displayName": "DaxLab"},
            "config": {"version": "2.0", "logicalId": tag("model")},
        },
    )
    write_json(
        model / "definition.pbism",
        {"$schema": f"{schemas}/item/semanticModel/definitionProperties/1.0.0/schema.json", "version": "4.2", "settings": {}},
    )
    definition = model / "definition"
    write(definition / "database.tmdl", "database\n\tcompatibilityLevel: 1601\n")
    tbls = tables()
    model_lines = [
        "/// DaxLab: the DAX function test models of crates/pbi-dax/tests/functions.rs (ground truth).",
        "model Model",
        f"{T}culture: en-US",
        f"{T}defaultPowerBIDataSourceVersion: powerBI_V3",
        f"{T}discourageImplicitMeasures",
        f"{T}sourceQueryCulture: en-US",
        f"{T}dataAccessOptions",
        f"{T}{T}legacyRedirects",
        f"{T}{T}returnErrorValuesAsNull",
        "",
        "annotation __PBI_TimeIntelligenceEnabled = 0",
        "",
    ] + [f"ref table {q(n)}" for n in tbls] + [""]
    write(definition / "model.tmdl", "\n".join(model_lines))
    write(definition / "relationships.tmdl", relationships_tmdl())
    for name, text in tbls.items():
        write(definition / "tables" / f"{name}.tmdl", text)

    write_json(
        report / ".platform",
        {
            "$schema": f"{schemas}/gitIntegration/platformProperties/2.0.0/schema.json",
            "metadata": {"type": "Report", "displayName": "DaxLab"},
            "config": {"version": "2.0", "logicalId": tag("report")},
        },
    )
    write_json(
        report / "definition.pbir",
        {
            "$schema": f"{schemas}/item/report/definitionProperties/2.0.0/schema.json",
            "version": "4.0",
            "datasetReference": {"byPath": {"path": "../DaxLab.SemanticModel"}},
        },
    )
    rdef = report / "definition"
    write_json(rdef / "version.json", {"$schema": f"{schemas}/item/report/definition/versionMetadata/1.0.0/schema.json", "version": "2.0.0"})
    write_json(
        rdef / "report.json",
        {
            "$schema": f"{schemas}/item/report/definition/report/3.3.0/schema.json",
            "themeCollection": {
                "customTheme": {
                    "name": "DaxLab.json",
                    "reportVersionAtImport": {"visual": "2.4.0", "report": "3.3.0", "page": "2.1.0"},
                    "type": "RegisteredResources",
                }
            },
            "resourcePackages": [
                {
                    "name": "RegisteredResources",
                    "type": "RegisteredResources",
                    "items": [{"name": "DaxLab.json", "path": "DaxLab.json", "type": "CustomTheme"}],
                }
            ],
            "settings": {"useStylableVisualContainerHeader": True, "useDefaultAggregateDisplayName": True},
        },
    )
    write_json(report / "StaticResources" / "RegisteredResources" / "DaxLab.json", {"name": "DaxLab"})
    page_id = "a0d1f2e3c4b5a6978899"
    write_json(
        rdef / "pages" / "pages.json",
        {
            "$schema": f"{schemas}/item/report/definition/pagesMetadata/1.1.0/schema.json",
            "pageOrder": [page_id],
            "activePageName": page_id,
        },
    )
    write_json(
        rdef / "pages" / page_id / "page.json",
        {
            "$schema": f"{schemas}/item/report/definition/page/2.1.0/schema.json",
            "name": page_id,
            "displayName": "Lab",
            "displayOption": "FitToPage",
            "height": 720,
            "width": 1280,
        },
    )
    print(f"wrote {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
