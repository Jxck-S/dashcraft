"""Compare two sets of ground-truth recordings case by case (suite S9, writer compatibility).

Record the same corpora on two Desktop models: the source fixture (the committed recordings) and a
file DashCraft wrote (`dashcraft convert --to pbip|pbit`, opened in Desktop), then:

    python scripts/groundtruth/compare_recordings.py reference/groundtruth/recordings \
        target/groundtruth/s9/recordings

Every suite present in the second folder is compared with the same suite in the first: same ids,
same columns, same rows (or both errors). Exit code 0 when they agree, 1 otherwise.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path


def cases(path: Path) -> dict[str, dict]:
    doc = json.loads(path.read_text(encoding="utf-8"))
    return {c["id"]: {k: v for k, v in c.items() if k != "id"} for c in doc["cases"]}


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(__doc__)
        return 2
    base, other = (Path(a) for a in argv)
    total = 0
    for path in sorted(other.glob("*.json")):
        want, got = cases(base / path.name), cases(path)
        differ = [k for k in want if want[k] != got.get(k)] + [k for k in got if k not in want]
        for k in differ[:10]:
            print(f"  {path.stem} {k}: {json.dumps(want.get(k))[:200]} | {json.dumps(got.get(k))[:200]}")
        print(f"{path.stem}: {len(want)} cases, {len(differ)} differ")
        total += len(differ)
    return 1 if total else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
