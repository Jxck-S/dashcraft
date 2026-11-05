"""Conservatively select full Rust CI or frontend-only validation."""

import os
from pathlib import Path, PurePosixPath
import re
import subprocess


FRONTEND_CONFIG = {
    "app/index.html",
    "app/package.json",
    "app/package-lock.json",
    "app/svelte.config.js",
    "app/tsconfig.json",
    "app/tsconfig.app.json",
    "app/tsconfig.node.json",
    "app/vite.config.ts",
}
FRONTEND_SUFFIXES = {
    ".css", ".gif", ".ico", ".jpeg", ".jpg", ".js", ".mjs", ".otf",
    ".png", ".svelte", ".svg", ".ts", ".ttf", ".webp",
}
TEST_DOCUMENTS = {"docs/AI.md", "docs/COMMANDS.md", "THIRD_PARTY_NOTICES.md"}
EXCLUDED_CONFIG = {
    ".github/dependabot.yml",
    ".github/workflows/fuzz.yml",
    ".github/workflows/release.yml",
}


def ignored_by_ci(path: str) -> bool:
    if path.startswith(("crates/", "reference/")) or path in TEST_DOCUMENTS:
        return False
    return path.startswith("docs/") or path.endswith(".md") or path in EXCLUDED_CONFIG


def requires_rust(paths: list[str]) -> bool:
    relevant = [path for path in paths if not ignored_by_ci(path)]
    if not relevant:
        return True
    for path in relevant:
        if path.endswith((".gen.ts", "/bindings.ts")):
            return True
        if path in FRONTEND_CONFIG:
            continue
        if (
            path.startswith(("app/src/", "app/static/", "app/tests/"))
            and PurePosixPath(path).suffix in FRONTEND_SUFFIXES
        ):
            continue
        return True
    return False


def changed_paths(base: str) -> list[str]:
    if not re.fullmatch(r"[0-9a-f]{40,64}", base):
        raise ValueError("BASE_SHA must be a Git object ID")
    result = subprocess.run(
        ["git", "diff", "--name-only", "--no-renames", "-z", base, "HEAD", "--"],
        check=True,
        stdout=subprocess.PIPE,
    )
    return [os.fsdecode(path) for path in result.stdout.split(b"\0") if path]


def main() -> None:
    base = os.environ.get("BASE_SHA", "")
    app = Path("app/package.json").is_file()
    if not base or not base.strip("0"):
        rust = True
        print("::notice::Running full CI: manual run or no previous revision.")
    else:
        paths = changed_paths(base)
        rust = not app or requires_rust(paths)
        print(f"Changed paths: {len(paths)}; full Rust validation: {rust}")
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
        output.write(f"rust={str(rust).lower()}\napp={str(app).lower()}\n")


if __name__ == "__main__":
    main()
