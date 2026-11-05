#!/usr/bin/env python3
"""Write a Power BI Desktop-ready copy of the Acme Outfitters PBIP under target/.

The committed example reads its CSVs with `AzureStorage.Blobs(StorageContainerUrl)` from a
placeholder URL that DashCraft resolves offline through `blob-map.json`. Desktop cannot refresh
that, so the copy adds two shared expressions:

- `DataFolder`: the absolute path of the example's `data/` folder (a text parameter);
- `AcmeBlobs`: a `Name`/`Content` table built from `Folder.Contents(DataFolder)` with the blob
  names of `blob-map.json`, so every partition keeps `Source{[Name = "..."]}[Content]` unchanged.

and replaces `AzureStorage.Blobs(StorageContainerUrl)` with `AcmeBlobs` in every partition. Apart
from that one source step the M, DAX, model and reports are byte-identical to the committed
example. `Folder.Contents` is one data source, so no privacy-level prompt appears.

Dynamic roles (USERPRINCIPALNAME()) get one static copy per user of expected.json's `rls` list,
named `GT <role> <user>`, because Desktop's local engine only impersonates Windows accounts.

It also writes one `.pbip` per report so each report can be opened on its own (goldens).

The output contains an absolute local path: it stays under target/ and is never committed
(reference/groundtruth/README.md).

Usage: python scripts/groundtruth/make_desktop_variant.py [--example DIR] [--out DIR]
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_EXAMPLE = ROOT / "reference" / "examples" / "acme-outfitters"
DEFAULT_OUT = ROOT / "target" / "groundtruth" / "acme-desktop"
SOURCE_CALL = "AzureStorage.Blobs(StorageContainerUrl)"


def m_text(s: str) -> str:
    return '"' + s.replace('"', '""') + '"'


def variant_expressions(blob_map: dict, data_dir: Path) -> str:
    containers = blob_map["containers"]
    if len(containers) != 1:
        raise SystemExit("expected exactly one container in blob-map.json")
    (entries,) = containers.values()
    rows = []
    for blob, rel in entries.items():
        rel_path = Path(rel)
        if rel_path.parent != Path("data"):
            raise SystemExit(f"blob {blob} is not in data/: {rel}")
        rows.append("{" + m_text(blob) + ", " + m_text(rel_path.name) + "}")
    t = "\t"
    lines = [
        "/// Ground-truth variant only: absolute path of the example's data folder.",
        f"expression DataFolder = {m_text(str(data_dir))} meta [IsParameterQuery=true, "
        'Type="Text", IsParameterQueryRequired=true]',
        f"{t}queryGroup: Parameters",
        "",
        f"{t}annotation PBI_ResultType = Text",
        "",
        "/// Ground-truth variant only: the blob-map.json names over the local data folder.",
        "expression AcmeBlobs =",
        f"{t}{t}let",
        f"{t}{t}    Files = Folder.Contents(DataFolder),",
        f"{t}{t}    Map = #table(type table [Name = text, File = text], {{" + ", ".join(rows) + "}),",
        f'{t}{t}    WithContent = Table.AddColumn(Map, "Content", each Files{{[Name = [File]]}}[Content], type binary),',
        f'{t}{t}    Result = Table.SelectColumns(WithContent, {{"Name", "Content"}})',
        f"{t}{t}in",
        f"{t}{t}    Result",
        f"{t}queryGroup: Parameters",
        "",
        f"{t}annotation PBI_ResultType = Table",
        "",
    ]
    return "\n".join(lines)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--example", type=Path, default=DEFAULT_EXAMPLE)
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT)
    args = ap.parse_args()
    example: Path = args.example.resolve()
    out: Path = args.out.resolve()
    if ROOT / "target" not in out.parents:
        raise SystemExit(f"--out must be under {ROOT / 'target'} (it holds an absolute local path)")

    expected = json.loads((example / "expected.json").read_text(encoding="utf-8"))
    blob_map = json.loads((example / expected["blobMap"]).read_text(encoding="utf-8"))
    pbip = json.loads((example / expected["pbip"]).read_text(encoding="utf-8"))

    if out.exists():
        shutil.rmtree(out)
    shutil.copytree(
        example,
        out,
        ignore=shutil.ignore_patterns("data", ".pbi", "*.md", "expected.json", "blob-map.json"),
    )

    models = sorted(out.glob("*.SemanticModel"))
    if len(models) != 1:
        raise SystemExit("expected exactly one semantic model")
    definition = models[0] / "definition"
    replaced = 0
    for tmdl in sorted((definition / "tables").glob("*.tmdl")):
        text = tmdl.read_text(encoding="utf-8")
        count = text.count(SOURCE_CALL)
        if count:
            tmdl.write_text(text.replace(SOURCE_CALL, "AcmeBlobs"), encoding="utf-8", newline="")
            replaced += count
    if replaced == 0:
        raise SystemExit(f"no partition uses {SOURCE_CALL}")

    expressions = definition / "expressions.tmdl"
    text = expressions.read_text(encoding="utf-8")
    if not text.endswith("\n"):
        text += "\n"
    text += "\n" + variant_expressions(blob_map, example / "data")
    expressions.write_text(text, encoding="utf-8", newline="")

    roles_added = add_static_roles(definition, expected)

    # The committed .pbip opens one report; add one per report for page exports.
    for report in sorted(out.glob("*.Report")):
        short = report.name.removesuffix(".Report").split(" - ")[-1]
        doc = dict(pbip)
        doc["artifacts"] = [{"report": {"path": report.name}}]
        (out / f"Acme-{short}.pbip").write_text(json.dumps(doc, indent=2) + "\n", encoding="utf-8")

    print(f"wrote {out} ({replaced} partitions use AcmeBlobs; static roles: {', '.join(roles_added)})")
    return 0


def static_role_name(role: str, user: str) -> str:
    # Same name as gen_corpus.py's desktop_role().
    return f"GT {role} {user.split('@')[0]}"


def add_static_roles(definition: Path, expected: dict) -> list[str]:
    """One static copy of each dynamic role per expected.json user: USERPRINCIPALNAME() is
    replaced by the user's address. Desktop's local engine refuses an EffectiveUserName that is
    not a Windows account, so the recorder connects with these roles instead."""
    added = []
    model_tmdl = definition / "model.tmdl"
    model_text = model_tmdl.read_text(encoding="utf-8")
    for spec in expected.get("rls", []):
        user = spec.get("userName")
        if not user:
            continue
        for role in spec["roles"]:
            src = (definition / "roles" / f"{role}.tmdl").read_text(encoding="utf-8")
            if "USERPRINCIPALNAME()" not in src:
                continue
            name = static_role_name(role, user)
            if name in added:
                continue
            body = src.replace("USERPRINCIPALNAME()", '"' + user.replace('"', '""') + '"')
            body = re.sub(r"^role .*$", f"role '{name}'", body, count=1, flags=re.M)
            body = re.sub(r"^///.*\n", f"/// Ground-truth variant only: {role} for {user}.\n", body, count=1, flags=re.M)
            (definition / "roles" / f"{name}.tmdl").write_text(body, encoding="utf-8", newline="")
            model_text = model_text.rstrip("\n") + f"\nref role '{name}'\n"
            added.append(name)
    model_tmdl.write_text(model_text, encoding="utf-8", newline="")
    return added


if __name__ == "__main__":
    sys.exit(main())
