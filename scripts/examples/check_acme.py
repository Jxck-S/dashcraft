#!/usr/bin/env python3
"""Checks DashCraft against the Acme Outfitters expected values.

Runs the `dashcraft` CLI on reference/examples/acme-outfitters (refresh, measures, lint,
render as SVG, and the security registry commands) and compares the results with
expected.json. Expectations marked "knownEngineGap" are reported but do not fail.

Usage: python3 scripts/examples/check_acme.py [--bin PATH] [--out DIR]

Without --bin it runs `cargo run -q -p pbiview-cli --bin dashcraft --`. Scratch output goes
to target/tmp/acme-check. Python 3 standard library only.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EXAMPLE = ROOT / "reference" / "examples" / "acme-outfitters"
SVG = "{http://www.w3.org/2000/svg}"


class Checker:
    def __init__(self, cli: list[str], out: Path):
        self.cli, self.out = cli, out
        self.exp = json.loads((EXAMPLE / "expected.json").read_text(encoding="utf-8"))
        self.pbip = str(EXAMPLE / self.exp["pbip"])
        self.blob_map = str(EXAMPLE / self.exp["blobMap"])
        self.passed = 0
        self.failures: list[str] = []
        self.known: list[str] = []

    def run(self, *args: str, ok_codes=(0,)) -> str:
        proc = subprocess.run([*self.cli, *args], cwd=ROOT, capture_output=True, text=True)
        if proc.returncode not in ok_codes:
            raise SystemExit(f"dashcraft {' '.join(args)} exited {proc.returncode}:\n{proc.stderr}")
        return proc.stdout

    def check(self, label: str, got, want, gap: str | None = None) -> None:
        if got == want:
            self.passed += 1
        elif gap:
            self.known.append(f"{label}: got {got!r}, expected {want!r} (known gap: {gap})")
        else:
            self.failures.append(f"{label}: got {got!r}, expected {want!r}")

    # -------------------------------------------------------------------------
    def refresh(self) -> None:
        data = json.loads(self.run("refresh", self.pbip, "--blob-map", self.blob_map, "--json"))
        rows = {t["name"]: t["rows"] for t in data["tables"]}
        for table, n in self.exp["rowCounts"].items():
            self.check(f"refresh rows {table}", rows.get(table), n)
        self.check("refresh ok", data["ok"], True)

    def measures(self) -> None:
        cases = [(None, self.exp["measures"]["unfiltered"])]
        cases += [(name, v["values"]) for name, v in self.exp["measures"]["byReport"].items()]
        for report, want in cases:
            args = ["measures", self.pbip, "--blob-map", self.blob_map, "--json"]
            if report:
                args += ["--report", report]
            got = {m["name"]: m.get("formatted") for m in json.loads(self.run(*args))["measures"]}
            for name, v in want.items():
                self.check(f"measures[{report or 'unfiltered'}] {name}", got.get(name), v["formatted"])

    def lint(self) -> None:
        data = json.loads(self.run("lint", self.pbip, "--json", ok_codes=(0, 2)))
        self.check("lint errors", data["summary"]["errors"], 0)
        self.check("lint reader errors", len(data["readerErrors"]), 0)

    def render(self) -> None:
        for report in self.exp["reports"]:
            out = self.out / "render" / report["report"]
            meta = json.loads(self.run("render", self.pbip, "--blob-map", self.blob_map, "--report",
                                       report["report"], "--all-pages", "--out-dir", str(out), "--format",
                                       "svg", "--force", "--json", ok_codes=(0, 2)))
            svgs = {p["name"]: Path(p["outputs"][0]["path"]) for p in meta["pages"]}
            status = {v["name"]: v for p in meta["pages"] for v in p["visuals"]}
            for page in report["pages"]:
                texts = visual_texts(svgs[page["pageId"]])
                where = f"{report['report']} / {page['page']}"
                for card in page["cards"]:
                    label = f"{where} / {card['visualKey']} [{card['measure']}]"
                    self.check(label + " status", status[card["visual"]]["status"], "ok")
                    got = card["formatted"] if card["formatted"] in texts.get(card["visual"], []) else \
                        texts.get(card["visual"], [])
                    self.check(label, got, card["formatted"], card.get("knownEngineGap"))
                for vis in page["visuals"]:
                    self.visual(f"{where} / {vis['visualKey']}", vis, texts.get(vis["visual"], []))

    def visual(self, label: str, vis: dict, texts: list[str]) -> None:
        gap = vis.get("knownEngineGap")
        wanted: list[str] = []
        if "indicator" in vis:
            wanted += [vis["indicator"]["formatted"]]
        for key in ("total",):
            wanted += [x["formatted"] for x in vis.get(key, {}).values()]
        for row in vis.get("rows", []) + vis.get("columns", []) + vis.get("categories", []):
            wanted += [x["formatted"] if isinstance(x, dict) else str(x) for x in row.values()
                       if x is not None and not isinstance(x, int)]
        # Tables and matrices leave BLANK cells empty; a long table's total row scrolls out of view
        # (checked through measures.values instead).
        wanted = [w for w in wanted if w != "(Blank)"]
        if "rowCount" in vis:
            wanted = []
        if vis["visualType"] == "gauge":
            wanted = []  # gauges apply display units (K); covered by the card values
        if vis["visualType"] == "pieChart":
            wanted = [c["Device"] for c in vis["categories"]]
            self.check(label + " only these categories", sorted(set(texts) & {"Desktop", "Mobile", "Tablet"}),
                       sorted(wanted))
        for w in wanted:
            self.check(f"{label} shows {w}", w if w in texts else None, w, gap)
        for row in vis.get("rows", []):
            cells = [x["formatted"] if isinstance(x, dict) else str(x) for x in row.values()
                     if x is not None and not (isinstance(x, dict) and x["formatted"] == "(Blank)")]
            self.check(f"{label} row {cells[0]}", cells if in_order(texts, cells) else None, cells, gap)
        if vis["visualKey"] == "calc_matrix":
            order = [t for t in texts if t in ("Current", "YTD", "PY", "YoY %")]
            self.check(label + " column order", order[:4], ["Current", "YTD", "PY", "YoY %"], gap)

    def security(self) -> None:
        steps = []
        for i, case in enumerate(self.exp["rls"]):
            params = {"roles": case["roles"]}
            if case["userName"]:
                params["userName"] = case["userName"]
            steps += [{"id": "security.viewAs", "params": params, "as": f"v{i}"},
                      {"id": "measures.values", "params": {}, "as": f"m{i}"}]
        batch = self.out / "rls-batch.json"
        batch.parent.mkdir(parents=True, exist_ok=True)
        batch.write_text(json.dumps({"steps": steps, "stopOnError": False}), encoding="utf-8")
        results = json.loads(self.run("run", "--batch", str(batch), "--doc", self.pbip, "--blob-map",
                                      self.blob_map, ok_codes=(0, 2)))
        roles = json.loads(self.run("run", "security.roles", "--doc", self.pbip))
        self.check("security.roles names", [r["name"] for r in roles["roles"]], ["Region West", "Store Manager"])
        self.check("security.roles dynamic", [r["dynamic"] for r in roles["roles"]], [False, True])
        for i, case in enumerate(self.exp["rls"]):
            label = f"viewAs {case['roles']} {case['userName'] or ''}".strip()
            view, values = results[2 * i]["result"], results[2 * i + 1]["result"]
            rows = {t["name"]: t.get("visibleRows", t.get("rows")) for t in view["tables"]}
            for table, n in case["visibleRows"].items():
                self.check(f"{label} rows {table}", rows.get(table), n)
            for m in values["measures"]:
                v = case["measures"].get(m["name"])
                if v is not None:
                    got = m.get("value") if v["value"] is None else m.get("formatted")
                    self.check(f"{label} {m['name']}", got, v["value"] if v["value"] is None else v["formatted"])

    def page_measures(self) -> None:
        """Pages without slicer selections: measures.values with the report and page filters gives
        the card values and the totals of visuals without visual filters."""
        for report in self.exp["reports"]:
            for page in report["pages"]:
                if page["slicers"]:
                    continue
                got = json.loads(self.run("run", "measures.values", "--doc", self.pbip, "--blob-map", self.blob_map,
                                          "-p", f"report={report['report']}", "-p", f"page={page['page']}"))
                values = {m["name"]: m.get("formatted") for m in got["measures"]}
                where = f"measures.values {report['report']} / {page['page']}"
                for card in page["cards"]:
                    self.check(f"{where} [{card['measure']}]", values.get(card["measure"]), card["formatted"])
                for vis in page["visuals"]:
                    for name, v in vis.get("total", {}).items():
                        self.check(f"{where} {vis['visualKey']} total [{name}]", values.get(name), v["formatted"])

    def report(self) -> int:
        for line in self.known:
            print("KNOWN  " + line)
        for line in self.failures:
            print("FAIL   " + line)
        print(f"{self.passed} passed, {len(self.failures)} failed, {len(self.known)} known engine gaps")
        return 1 if self.failures else 0


def in_order(texts: list[str], cells: list[str]) -> bool:
    """`cells` appear in `texts` in this order, close together (one table or card row)."""
    for i, t in enumerate(texts):
        if t != cells[0]:
            continue
        window, pos = texts[i + 1:i + 1 + 3 * len(cells)], 0
        for c in cells[1:]:
            try:
                pos = window.index(c, pos) + 1
            except ValueError:
                break
        else:
            return True
    return False


def visual_texts(svg: Path) -> dict[str, list[str]]:
    """Text of every visual group (`data-hit="<visual id>|visual"`) of a rendered page."""
    out: dict[str, list[str]] = {}
    root = ET.parse(svg).getroot()

    def walk(el, current):
        hit = el.get("data-hit", "")
        if hit.endswith("|visual"):
            current = hit.split("|")[0]
            out.setdefault(current, [])
        if el.tag == SVG + "text" and current:
            text = "".join(el.itertext()).strip()
            if text:
                out[current].append(text)
        for child in el:
            walk(child, current)

    walk(root, None)
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--bin", help="dashcraft binary (default: cargo run)")
    ap.add_argument("--out", type=Path, default=ROOT / "target" / "tmp" / "acme-check")
    args = ap.parse_args()
    cli = [args.bin] if args.bin else ["cargo", "run", "-q", "-p", "pbiview-cli", "--bin", "dashcraft", "--"]
    c = Checker(cli, args.out)
    c.refresh()
    c.measures()
    c.lint()
    c.render()
    c.page_measures()
    c.security()
    return c.report()


if __name__ == "__main__":
    sys.exit(main())
