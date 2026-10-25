#!/usr/bin/env python3
"""License inventory of the third-party code shipped in DashCraft's binaries and app bundle.

Rust: the normal (non-dev, non-build) dependency closure of the `dashcraft-cli` and `dashcraft-app`
crates (the `dashcraft` CLI and `dashcraft-app`),
from `cargo tree -e normal --locked`, unioned over the four release targets (the `[graph]
targets` in deny.toml). Proc-macro crates are included. Workspace crates are not listed.

npm: the packages whose code ends up in the front-end bundle (app/dist). They are read from
the source maps of a throw-away `vite build --sourcemap` (output in target/, deleted
afterwards), so runtime code of dev dependencies (the Svelte runtime) is listed and build
tooling is not. Needs `npm ci` in app/ first.

Usage (from anywhere; Python 3.9+, no third-party modules):
  scripts/license-inventory.py            rewrite the generated blocks in THIRD_PARTY_NOTICES.md
  scripts/license-inventory.py --check    exit 1 if THIRD_PARTY_NOTICES.md is out of date
  scripts/license-inventory.py --texts F  also write the full license/notice files of every
                                          listed package to F (release bundles ship it)

The output is deterministic: sorted, no dates, no local paths.
"""

from __future__ import annotations

import argparse
import difflib
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
NOTICES = ROOT / "THIRD_PARTY_NOTICES.md"
APP = ROOT / "app"
TARGETS = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
]
ROOT_CRATES = ["dashcraft-cli", "dashcraft-app"]
MARKERS = {
    "rust": ("<!-- license-inventory:rust:begin -->", "<!-- license-inventory:rust:end -->"),
    "npm": ("<!-- license-inventory:npm:begin -->", "<!-- license-inventory:npm:end -->"),
}
LICENSE_FILE_RE = re.compile(r"^(licen[cs]e|copying|notice|copyright|unlicense)([-._].*)?$", re.I)
MAX_TEXT_BYTES = 256 * 1024


def run(cmd: list[str], cwd: Path) -> str:
    exe = shutil.which(cmd[0])
    if exe is None:
        sys.exit(f"error: `{cmd[0]}` not found on PATH")
    result = subprocess.run([exe, *cmd[1:]], cwd=cwd, capture_output=True, text=True)
    if result.returncode != 0:
        sys.stderr.write(result.stderr)
        sys.exit(f"error: `{' '.join(cmd)}` failed with exit code {result.returncode}")
    return result.stdout


def md_escape(text: str) -> str:
    return text.replace("|", "\\|").replace("\n", " ").strip()


def normalize_license(expr: str | None) -> str:
    if not expr:
        return "UNKNOWN"
    # Old crates.io style "MIT/Apache-2.0" means "MIT OR Apache-2.0".
    expr = re.sub(r"\s*/\s*", " OR ", expr.strip())
    return re.sub(r"\s+", " ", expr)


def normalize_url(url: str | None) -> str:
    if not url:
        return ""
    url = url.strip()
    url = re.sub(r"^git\+", "", url)
    url = re.sub(r"^git://", "https://", url)
    url = re.sub(r"^ssh://git@", "https://", url)
    url = re.sub(r"^git@github\.com:", "https://github.com/", url)
    url = re.sub(r"^github:", "https://github.com/", url)
    url = re.sub(r"\.git$", "", url)
    if re.fullmatch(r"[\w.-]+/[\w.-]+", url):
        url = f"https://github.com/{url}"
    return url


def version_key(version: str) -> list:
    return [(0, int(p), "") if p.isdigit() else (1, 0, p) for p in re.split(r"[.+-]", version)]


def license_files(directory: Path) -> list[Path]:
    if not directory.is_dir():
        return []
    files = [p for p in directory.iterdir() if p.is_file() and LICENSE_FILE_RE.match(p.name)]
    return sorted(files, key=lambda p: p.name.lower())


# ---------------------------------------------------------------- Rust


def rust_packages() -> list[dict]:
    """Packages that `cargo tree -e normal` reaches from ROOT_CRATES, on any release target.

    `cargo tree` applies the real feature resolver, so features that only dev-dependencies
    enable (and the crates behind them) are not counted; `cargo metadata` supplies the details.
    """
    meta = json.loads(run(["cargo", "metadata", "--format-version", "1", "--locked"], ROOT))
    workspace = {p["name"] for p in meta["packages"] if p["id"] in meta["workspace_members"]}
    missing = set(ROOT_CRATES) - workspace
    if missing:
        sys.exit(f"error: workspace crates not found: {', '.join(sorted(missing))}")
    by_key = {(p["name"], p["version"]): p for p in meta["packages"] if p["name"] not in workspace}
    shipped: dict[tuple[str, str], dict] = {}
    for target in TARGETS:
        cmd = ["cargo", "tree", "--locked", "-e", "normal", "--target", target]
        for crate in ROOT_CRATES:
            cmd += ["-p", crate]
        tree = run([*cmd, "--prefix", "none", "--format", "{p}"], ROOT)
        for line in tree.splitlines():
            match = re.match(r"^(\S+) v(\S+)", line.strip())
            if not match or match.group(1) in workspace:
                continue
            key = (match.group(1), match.group(2))
            if key not in by_key:
                sys.exit(f"error: `cargo tree` package {key[0]} {key[1]} not in cargo metadata")
            shipped[key] = by_key[key]
    return list(shipped.values())


def rust_section(packages: list[dict]) -> str:
    by_name: dict[str, list[dict]] = defaultdict(list)
    for p in packages:
        by_name[p["name"]].append(p)
    rows = []
    counts: dict[str, int] = defaultdict(int)
    for name in sorted(by_name, key=str.lower):
        versions = sorted(by_name[name], key=lambda p: version_key(p["version"]))
        licenses = sorted({normalize_license(p["license"]) for p in versions})
        for lic in licenses:
            counts[lic] += 1
        latest = versions[-1]
        url = normalize_url(latest.get("repository") or latest.get("homepage"))
        if not url:
            url = f"https://crates.io/crates/{name}"
        rows.append(
            f"| `{name}` | {', '.join(p['version'] for p in versions)} "
            f"| {md_escape(' / '.join(licenses))} | {url} |"
        )
    out = [
        "Generated by `scripts/license-inventory.py` from `Cargo.lock`. Do not edit by hand.",
        "",
        f"{len(by_name)} crates ({len(packages)} crate versions): the normal dependency closure of "
        "`dashcraft-cli` and `dashcraft-app` on macOS (arm64, x64), Windows x64 and Linux x64, "
        "including proc-macro crates. Build-only and dev-only dependencies are not shipped "
        "and not listed.",
        "",
        "| License (SPDX expression) | Crates |",
        "|---|---:|",
    ]
    for lic in sorted(counts, key=lambda k: (-counts[k], k)):
        out.append(f"| {md_escape(lic)} | {counts[lic]} |")
    out += ["", "| Crate | Version(s) | License | Source |", "|---|---|---|---|", *rows]
    return "\n".join(out)


def rust_texts(packages: list[dict]) -> list[tuple[str, str, Path | None]]:
    """(label, license, directory) per crate version, for --texts."""
    entries = []
    for p in sorted(packages, key=lambda p: (p["name"].lower(), version_key(p["version"]))):
        directory = Path(p["manifest_path"]).parent
        entries.append(
            (f"{p['name']} {p['version']} (crate)", normalize_license(p["license"]), directory)
        )
    return entries


# ---------------------------------------------------------------- npm

NODE_MODULES_RE = re.compile(r"^(.*/node_modules/(?:@[^/]+/)?[^/]+)/")


def npm_package_dirs() -> list[Path]:
    """Package directories whose files appear in the source maps of a production build."""
    if not (APP / "node_modules").is_dir():
        sys.exit("error: app/node_modules is missing; run `npm ci` in app/ first")
    out_dir = ROOT / "target" / "license-inventory" / "dist"
    shutil.rmtree(out_dir, ignore_errors=True)
    try:
        run(
            [
                "npx",
                "--no-install",
                "vite",
                "build",
                "--sourcemap",
                "--emptyOutDir",
                "--outDir",
                str(out_dir),
                "--logLevel",
                "error",
            ],
            APP,
        )
        dirs: set[Path] = set()
        for map_file in sorted(out_dir.rglob("*.map")):
            source_map = json.loads(map_file.read_text(encoding="utf-8"))
            root = source_map.get("sourceRoot") or ""
            for source in source_map.get("sources", []):
                path = os.path.normpath(os.path.join(map_file.parent, root, source))
                path = path.replace(os.sep, "/")
                # Helpers that the bundler injects (e.g. `\0vite/preload-helper`) are Vite's code.
                if "node_modules/" not in path and re.search(r"(^|/|\x00)vite/", source):
                    dirs.add(APP / "node_modules" / "vite")
                    continue
                match = NODE_MODULES_RE.match(path + "/")
                if match:
                    dirs.add(Path(match.group(1)))
        if not dirs:
            sys.exit("error: no node_modules sources found in the build's source maps")
        return sorted(dirs)
    finally:
        shutil.rmtree(out_dir.parent, ignore_errors=True)


def npm_license(manifest: dict) -> str:
    lic = manifest.get("license")
    if isinstance(lic, dict):
        lic = lic.get("type")
    if not lic and isinstance(manifest.get("licenses"), list):
        types = [l.get("type") if isinstance(l, dict) else str(l) for l in manifest["licenses"]]
        lic = " OR ".join(t for t in types if t)
    return normalize_license(lic)


def npm_packages() -> list[dict]:
    packages = {}
    for directory in npm_package_dirs():
        manifest = json.loads((directory / "package.json").read_text(encoding="utf-8"))
        repo = manifest.get("repository")
        if isinstance(repo, dict):
            repo = repo.get("url")
        url = normalize_url(repo or manifest.get("homepage"))
        name, version = manifest["name"], manifest["version"]
        packages[(name, version)] = {
            "name": name,
            "version": version,
            "license": npm_license(manifest),
            "url": url or f"https://www.npmjs.com/package/{name}",
            "dir": directory,
        }
    return [packages[k] for k in sorted(packages, key=lambda k: (k[0], version_key(k[1])))]


def npm_section(packages: list[dict]) -> str:
    out = [
        "Generated by `scripts/license-inventory.py` from `app/package-lock.json` and the "
        "production build's source maps. Do not edit by hand.",
        "",
        f"{len(packages)} packages have code in the shipped front-end bundle (`app/dist`). "
        "Build tooling (Vite, the Svelte compiler, TypeScript, the Tauri CLI) is not shipped "
        "and not listed.",
        "",
        "| Package | Version | License | Source |",
        "|---|---|---|---|",
    ]
    for p in packages:
        out.append(f"| `{p['name']}` | {p['version']} | {md_escape(p['license'])} | {p['url']} |")
    return "\n".join(out)


# ---------------------------------------------------------------- output


def replace_block(text: str, key: str, body: str) -> str:
    begin, end = MARKERS[key]
    pattern = re.compile(re.escape(begin) + r".*?" + re.escape(end), re.S)
    if not pattern.search(text):
        sys.exit(f"error: markers {begin} … {end} not found in {NOTICES.name}")
    return pattern.sub(lambda _: f"{begin}\n{body}\n{end}", text, count=1)


def write_texts(path: Path, entries: list[tuple[str, str, Path | None]]) -> None:
    """Full license and notice files, grouped by identical text."""
    groups: dict[str, dict] = {}
    missing = []
    for label, lic, directory in entries:
        files = license_files(directory) if directory else []
        if not files:
            missing.append(f"{label}: {lic}")
            continue
        for file in files:
            data = file.read_bytes()[:MAX_TEXT_BYTES]
            text = data.decode("utf-8", errors="replace").replace("\r\n", "\n").strip()
            digest = hashlib.sha256(text.encode()).hexdigest()
            group = groups.setdefault(digest, {"text": text, "users": set()})
            group["users"].add(f"{label}, {file.name}")
    out = [
        "DashCraft third-party licenses",
        "",
        "Full license and notice files of the third-party packages listed in",
        "THIRD_PARTY_NOTICES.md (generated by scripts/license-inventory.py --texts).",
        "Identical texts are printed once, followed by every package that ships them.",
        "",
    ]
    for group in sorted(groups.values(), key=lambda g: sorted(g["users"])[0].lower()):
        out.append("=" * 78)
        out.extend(sorted(group["users"], key=str.lower))
        out.append("-" * 78)
        out.append(group["text"])
        out.append("")
    if missing:
        out.append("=" * 78)
        out.append("Packages without a license file in their published package")
        out.append("(the SPDX expression from their manifest applies):")
        out.append("-" * 78)
        out.extend(sorted(missing, key=str.lower))
        out.append("")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(out), encoding="utf-8")
    print(f"wrote {path} ({len(groups)} distinct texts, {len(missing)} packages without one)")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("--check", action="store_true", help="fail if the notices are stale")
    parser.add_argument("--texts", type=Path, help="also write full license texts to this file")
    args = parser.parse_args()

    crates = rust_packages()
    npm = npm_packages()
    current = NOTICES.read_text(encoding="utf-8")
    updated = replace_block(current, "rust", rust_section(crates))
    updated = replace_block(updated, "npm", npm_section(npm))

    if args.texts:
        entries = rust_texts(crates) + [
            (f"{p['name']} {p['version']} (npm)", p["license"], p["dir"]) for p in npm
        ]
        write_texts(args.texts.resolve(), entries)

    if args.check:
        if updated != current:
            sys.stdout.writelines(
                difflib.unified_diff(
                    current.splitlines(keepends=True),
                    updated.splitlines(keepends=True),
                    "THIRD_PARTY_NOTICES.md",
                    "THIRD_PARTY_NOTICES.md (regenerated)",
                )
            )
            print(
                "\nTHIRD_PARTY_NOTICES.md is out of date: run scripts/license-inventory.py",
                file=sys.stderr,
            )
            return 1
        print("THIRD_PARTY_NOTICES.md license inventory is up to date")
        return 0
    if updated != current:
        NOTICES.write_text(updated, encoding="utf-8")
        print(f"updated {NOTICES.relative_to(ROOT)}")
    else:
        print(f"{NOTICES.relative_to(ROOT)} already up to date")
    return 0


if __name__ == "__main__":
    sys.exit(main())
