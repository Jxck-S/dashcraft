# Packaging and releases

How the DashCraft desktop app and the `dashcraft` CLI are bundled, signed, verified and published.
Names follow [ADR-0009](adr/0009-naming-dashcraft.md) (DashCraft, formerly pbiview).
The CI side (jobs, permissions, caching) is in [ci.md](ci.md#releaseyml-tags-v); this page
is the packaging reference.

## What a release contains

`.github/workflows/release.yml` builds everything on a `v*` tag, or on a manual run
([below](#release-workflow)), and attaches it to a **draft** GitHub release. `<ver>` is the
version without the `v`; `<tag>` is the tag.

| Platform | File | Notes |
|---|---|---|
| macOS arm64 | `DashCraft_<ver>_aarch64.dmg` | Drag `DashCraft.app` to Applications |
| macOS x64 | `DashCraft_<ver>_x64.dmg` | Cross-compiled on Apple Silicon |
| Windows x64 | `DashCraft_<ver>_x64_en-US.msi` | WiX; per-machine install (needs admin) |
| Windows x64 | `DashCraft_<ver>_x64-setup.exe` | NSIS; per-user install, no admin |
| Windows arm64 | `DashCraft_<ver>_arm64-setup.exe` | NSIS only (Tauri's WiX MSI targets x86/x64); built on a native Arm64 runner |
| Linux x64, arm64 | `DashCraft_<ver>_amd64.AppImage`, `…_aarch64.AppImage` | Portable; `chmod +x` and run |
| Linux x64, arm64 | `DashCraft_<ver>_amd64.deb`, `…_arm64.deb` | Debian/Ubuntu (package `dash-craft`, Tauri's kebab case of the product name; [why](#linux-package-name-dash-craft-decided)); registers MIME types and file associations |
| Linux x64, arm64 | `…-<ver>-1.x86_64.rpm`, `…-<ver>-1.aarch64.rpm` | Fedora, RHEL, openSUSE; the exact names come from the Tauri bundler |
| CLI | `dashcraft-<tag>-<target>.tar.gz` (`.zip` on Windows) | `dashcraft` and its `pbiview` alias, `README.md`, `NOTICE`, `THIRD_PARTY_NOTICES.md`, `THIRD_PARTY_LICENSES.txt` |
| All | `<file>.sha256` | One per binary artifact (`sha256sum -c` / `shasum -a 256 -c`) |
| All | `SHA256SUMS.txt` | Checksums of every asset |
| All | `dashcraft-<tag>-sbom.zip` | CycloneDX 1.5 SBOMs (see [below](#license-inventory-and-sbom)) |
| All | `THIRD_PARTY_NOTICES.md`, `THIRD_PARTY_LICENSES.txt` | License inventory and full license texts |

CLI targets: `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-pc-windows-msvc` and
`aarch64-pc-windows-msvc` (static CRT), `x86_64-unknown-linux-gnu` and
`aarch64-unknown-linux-gnu` (built on Ubuntu 22.04, glibc ≥ 2.35).

The release notes are the version's section of [CHANGELOG.md](../CHANGELOG.md), plus a
download/verification footer and, for unsigned builds, how to get past Gatekeeper or
SmartScreen.

## Bundle configuration

Everything is in [`app/src-tauri/tauri.conf.json`](../app/src-tauri/tauri.conf.json).

| Setting | Value | Why |
|---|---|---|
| `productName` | `DashCraft` | App name, window title, `.app`/installer/menu names |
| `mainBinaryName` | `dashcraft-app` | The GUI executable (`DashCraft.app/Contents/MacOS/dashcraft-app`, `dashcraft-app.exe`, `/usr/bin/dashcraft-app`); also the Cargo `[[bin]]` of the `pbiview-app` crate. Keeps `dashcraft` free for the CLI, so both can be on `PATH`. |
| `identifier` | `io.github.jxck-s.dashcraft` | Bundle ID (macOS), also used for the app config and data folders ([ADR-0009](adr/0009-naming-dashcraft.md)). Set before the first public release; changing it later moves File → Recent and `ai.json` to new folders, so it then needs a settings migration. |
| `version` | *(not set)* | Taken from the `pbiview-app` crate, i.e. `[workspace.package] version` in the root `Cargo.toml`. The release workflow fails if the tag, `Cargo.toml` and `app/package.json` disagree. |
| `bundle.targets` | `app`, `dmg`, `msi`, `nsis`, `appimage`, `deb` | Tauri builds the subset that applies to the host OS |
| `bundle.icon` | `icons/*` | Generated from `icons/app-icon.svg` (see [Icons](#icons)) |
| `bundle.resources` | `licenses/…` | `NOTICE` (trademark and non-affiliation notice), `THIRD_PARTY_NOTICES.md`, the Selawik `OFL.txt` (as `Selawik-OFL.txt`) and the Fluent UI System Icons MIT `LICENSE` (as `FluentUI-System-Icons-LICENSE.txt`). Release builds add `THIRD_PARTY_LICENSES.txt`. |
| `bundle.fileAssociations` | `.pbix`, `.pbit`, `.pbip`, `.bim`, `.vpax` | Role *Viewer*, rank *Alternate* (see [File associations](#file-associations)) |
| `bundle.windows.wix.upgradeCode` | `070cde76-…-f64f8824fa32` | Pinned (Tauri's default for the former name `pbiview`), so the rename to DashCraft is an MSI upgrade of an installed pbiview and later renames can't break upgrades either. NSIS, macOS and Linux installs of pbiview stay separate apps (remove them by hand). |
| `bundle.windows.nsis.installMode` | `currentUser` | NSIS installs per user without admin rights; use the MSI for machine-wide installs |
| `bundle.linux.deb`, `bundle.linux.appimage` | `desktopTemplate`, `files` | `.desktop` file (`DashCraft.desktop`) with `Exec=dashcraft-app %F` (the AppImage reuses the deb's), and the shared-mime-info XML at `/usr/share/mime/packages/dashcraft.xml` (both in [`scripts/packaging/linux/`](../scripts/packaging/linux/)) |

Where the license resources end up:

| OS | Path |
|---|---|
| macOS | `DashCraft.app/Contents/Resources/licenses/` |
| Windows | `<install dir>\licenses\` (MSI: `C:\Program Files\DashCraft`; NSIS: `%LOCALAPPDATA%\DashCraft`) |
| Linux (deb) | `/usr/lib/DashCraft/licenses/` |
| Linux (AppImage) | `usr/lib/DashCraft/licenses/` inside the image |

At runtime they are under Tauri's resource directory (`app.path().resource_dir()` +
`licenses/…`, or `resolveResource('licenses/THIRD_PARTY_NOTICES.md')` from
`@tauri-apps/api/path`). Help → About currently names the source files; linking it to these
paths is a front-end follow-up.

### Linux package name: `dash-craft` (decided)

The Debian package is **`dash-craft`**: `sudo apt install ./DashCraft_<ver>_amd64.deb`,
`sudo apt remove dash-craft`, `dpkg -s dash-craft`. The binary stays `dashcraft-app` and the
menu entry "DashCraft". This is kept deliberately:

- Tauri 2.12 derives the package name from `productName` by kebab-casing it (`DashCraft` →
  `dash-craft`; an RPM would get the same name), and `bundle.linux.deb` has no package-name
  option (it offers `depends`, `files`, `section`, `priority`, `desktopTemplate`, …).
- The only lever is a Linux-only `app/src-tauri/tauri.linux.conf.json` with `"productName":
  "dashcraft"` (merged over `tauri.conf.json` on Linux). It would also lowercase the app name on
  Linux: the menu entry (the `.desktop` template's `Name={{name}}` would need a hard-coded
  `DashCraft`), the `.desktop` file, the `/usr/lib/DashCraft/` resource folder and the
  `.deb`/AppImage file names, so Linux would no longer match the other platforms and this page.
- Nothing depends on the package name: no repository, updater or dependent package uses it yet.

Revisit if Tauri adds a package-name option or DashCraft is published to an APT repository
where users would look for `dashcraft`; a rename then needs `Replaces: dash-craft`,
`Conflicts: dash-craft` and `Provides: dash-craft` so existing installs upgrade.

### The CLI is not a sidecar

The CLI ships as separate per-platform archives, not inside the app bundle
(`bundle.externalBin`). Reasons:

- **Size.** The CLI is about 28 MB (arm64 release build) and shares no code with the app at
  runtime, so a sidecar would add that much to every installer.
- **No `PATH` integration.** A sidecar lives inside the bundle (`Contents/MacOS/`, the install
  folder, `/usr/bin`) under a target-suffixed name. Users would still need a symlink or
  `PATH` change to run `dashcraft`, which installers don't do.
- **Simpler builds and signing.** `externalBin` needs a `dashcraft-<target-triple>` binary
  built before every `tauri build`. On macOS each nested binary must also be signed and
  notarized with the app.
- **Independent use.** CI, servers and WSL users want the CLI without a webview runtime.

Revisit this if the app ever needs to run the CLI itself. If the app later puts a launcher on
`PATH`, use the name `dashcraft` for the CLI; that is why the GUI binary is `dashcraft-app`.

## File associations

Configured once in `bundle.fileAssociations` (one entry per extension so each type has its
own name and MIME type):

| Extension | Name | Linux MIME type |
|---|---|---|
| `.pbix` | Power BI report (PBIX) | `application/x-dashcraft-pbix` (sub-class of `application/zip`) |
| `.pbit` | Power BI template (PBIT) | `application/x-dashcraft-pbit` (zip) |
| `.pbip` | Power BI project (PBIP) | `application/x-dashcraft-pbip` (sub-class of `application/json`) |
| `.bim` | Tabular model (BIM) | `application/x-dashcraft-bim` (json) |
| `.vpax` | VertiPaq Analyzer export (VPAX) | `application/x-dashcraft-vpax` (zip) |

(`application/x-pbiview-*` before the rename.)

DashCraft registers as a **viewer**. It doesn't try to become the default app where another
one (Power BI Desktop, Tabular Editor) owns the type:

- **macOS:** `CFBundleDocumentTypes` with `CFBundleTypeRole = Viewer` and
  `LSHandlerRank = Alternate`. No `UTExportedTypeDeclarations`: we don't own these formats.
  Finder lists DashCraft under *Open With*, and uses it for double-clicks only when no other
  app claims the type.
- **Windows (MSI and NSIS):** Tauri's installers register a `ProgId` per extension with an
  *open* verb that runs `dashcraft-app.exe "%1"`. Windows 10/11 keep any default the user
  chose (`UserChoice`). When DashCraft is installed next to Power BI Desktop, Explorer offers
  both under *Open with*. Uninstalling removes the registration.
- **Linux (deb):** the `.desktop` file lists the MIME types and runs `dashcraft-app %F`. The
  package installs `/usr/share/mime/packages/dashcraft.xml`, and the shared-mime-info dpkg
  trigger runs `update-mime-database`.
- **Linux (AppImage):** no system integration by itself. Tools such as AppImageLauncher or
  `appimaged` add the `.desktop` entry. The MIME XML is inside the image but is not
  registered.

The app opens the first non-flag command-line argument at startup, which is how Windows
and Linux pass the file. macOS delivers files through an Apple Event (`RunEvent::Opened`):
the app forwards it to the UI as an `open-file-requested` event (or hands it out through
`startup_path` when the UI is not listening yet), so Finder double-clicks and *Open With*
open the file, including in an app that is already running (since v0.2.0).

## Building locally

All commands run from the repository root unless noted. Bundles land in
`target/release/bundle/<kind>/`, or `target/<triple>/release/bundle/` with `--target`. Never
commit them: `target/` is ignored.

### macOS

Requires Xcode Command Line Tools, plus Node and Rust as in [ci.md](ci.md#running-the-same-checks-locally).

```sh
cd app && npm ci
# CI=true skips the Finder AppleScript that lays out the DMG window (needs a GUI session).
# APPLE_SIGNING_IDENTITY=- gives an ad-hoc signature (unsigned arm64 binaries won't run).
CI=true APPLE_SIGNING_IDENTITY=- npx tauri build --bundles app,dmg
# Intel build from Apple Silicon:
rustup target add x86_64-apple-darwin
CI=true APPLE_SIGNING_IDENTITY=- npx tauri build --target x86_64-apple-darwin --bundles app,dmg
```

A universal app (`--target universal-apple-darwin`, both targets installed) also works. It
is about twice the size and twice the build time, so releases ship two thin DMGs instead.

### Windows

Requires the MSVC build tools ("Desktop development with C++") and WebView2, which is
preinstalled on Windows 11 and current Windows 10. The bundler downloads WiX 3 and NSIS on first use.

```powershell
cd app; npm ci
npx tauri build --bundles msi,nsis
```

If WebView2 is missing, the installers download its bootstrapper (the default
`webviewInstallMode`). Offline installs need `bundle.windows.webviewInstallMode` set to `offlineInstaller` (about
+130 MB).

### Linux

Ubuntu 22.04+ / Debian 12+ (WebKitGTK 4.1):

```sh
sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
  librsvg2-dev libxdo-dev patchelf file xdg-utils
cd app && npm ci
npx tauri build --bundles appimage,deb
```

The AppImage step downloads `linuxdeploy` and needs FUSE to run it, or
`APPIMAGE_EXTRACT_AND_RUN=1`. Build on the oldest distribution you want to support: the
binaries need at least the build host's glibc.

### CLI

```sh
cargo build --release --locked -p pbiview-cli         # target/release/dashcraft and pbiview (alias)
cargo build --release --locked -p pbiview-cli --target x86_64-apple-darwin
```

### Release-identical local build

To reproduce a release job, including `THIRD_PARTY_LICENSES.txt` in the bundle:

```sh
python3 scripts/license-inventory.py --texts dist/licenses/THIRD_PARTY_LICENSES.txt
echo '{"bundle":{"resources":{"../../dist/licenses/THIRD_PARTY_LICENSES.txt":"licenses/THIRD_PARTY_LICENSES.txt"}}}' > target/tauri.release.json
cd app && CI=true APPLE_SIGNING_IDENTITY=- npx tauri build --ci --target aarch64-apple-darwin \
  --bundles app,dmg --config ../target/tauri.release.json
cd .. && rm -rf dist
```

## Verifying bundles

The release workflow runs the macOS and Linux checks below after each build (*Verify
bundles* step).

**macOS**

```sh
app=target/release/bundle/macos/DashCraft.app
codesign --verify --deep --strict --verbose=2 "$app"   # valid on disk, satisfies its DR
codesign -dv "$app"                                    # Signature=adhoc, or Authority=Developer ID…
spctl -a -vv -t exec "$app"                            # "rejected" unless signed + notarized
lipo -archs "$app/Contents/MacOS/dashcraft-app"        # arm64 or x86_64
plutil -p "$app/Contents/Info.plist"                   # CFBundleDocumentTypes: 5 entries
ls "$app/Contents/Resources/licenses"                  # NOTICE, THIRD_PARTY_NOTICES.md, …
dmg=$(ls target/release/bundle/dmg/*.dmg)
hdiutil verify "$dmg"
hdiutil attach -nobrowse -readonly "$dmg"              # DashCraft.app + Applications link
hdiutil detach /Volumes/DashCraft
xcrun stapler validate "$app"                          # notarized builds only
```

**Windows** (PowerShell)

```powershell
Get-AuthenticodeSignature .\DashCraft_*_x64-setup.exe   # Valid when signed, NotSigned otherwise
msiexec /i DashCraft_<ver>_x64_en-US.msi /qn /l*v install.log   # silent install (admin)
# Then: %ProgramFiles%\DashCraft\dashcraft-app.exe, licenses\, and Explorer "Open with" for .pbix
```

**Linux**

```sh
dpkg-deb --info DashCraft_<ver>_amd64.deb              # Package: dash-craft; Depends: libwebkit2gtk-4.1-0, libgtk-3-0
dpkg-deb -x DashCraft_<ver>_amd64.deb out/ && cat out/usr/share/applications/DashCraft.desktop
desktop-file-validate out/usr/share/applications/DashCraft.desktop
./DashCraft_<ver>_amd64.AppImage --appimage-extract >/dev/null && ls squashfs-root/usr/lib/DashCraft/licenses
xdg-mime query filetype report.pbix                    # after installing the deb: application/x-dashcraft-pbix
```

**Checksums:** `sha256sum -c SHA256SUMS.txt --ignore-missing` (Linux/Windows Git Bash) or
`shasum -a 256 -c DashCraft_<ver>_aarch64.dmg.sha256` (macOS).

**Provenance:** when attestations are enabled (public repository, or the repository variable
`PBIVIEW_ATTEST=true` on GitHub Enterprise Cloud), run
`gh attestation verify <file> -R <owner>/<repo>`.

### Local result (macOS arm64, v0.1.0 config, before the rename)

| Check | Result |
|---|---|
| `pbiview.app` | 94.7 MiB; `pbiview-app` Mach-O arm64 |
| `pbiview_0.1.0_aarch64.dmg` | 30.3 MiB; `hdiutil verify` VALID; mounts with `pbiview.app` and the `Applications` link |
| `codesign` | `Signature=adhoc`, `flags=adhoc,runtime`, valid on disk; `spctl` rejected (expected without notarization) |
| `Info.plist` | `CFBundleExecutable=pbiview-app`, 5 `CFBundleDocumentTypes` (Viewer, Alternate), `LSMinimumSystemVersion=10.13` |
| CLI `target/release/pbiview` | 28.5 MB arm64; `--version` and `inspect … --json` OK |

The app binary is larger than the CLI because of symbols. Setting
`CARGO_PROFILE_RELEASE_STRIP=symbols` saves about 17 MB, but release backtraces then lose
function names, so it is not enabled.

## Release workflow

1. `scripts/set-version.sh <ver>` sets the version in `Cargo.toml` (`[workspace.package]`),
   `Cargo.lock`, `app/package.json`, `app/package-lock.json` and, if it has one,
   `tauri.conf.json`.
2. Move the CHANGELOG `[Unreleased]` entries under `## [<ver>] — <date> — <title>`.
3. Run `scripts/license-inventory.py` and commit (CI warns if you forget).
4. Start the release, either way:
   - **Manual (Actions → Release → Run workflow)** on the commit's branch. It releases the
     version in `Cargo.toml` and creates the tag `v<ver>` on that commit itself.
   - **Tag push:** tag `v<ver>` and push the tag.
5. Review the draft release (assets, notes), then publish it.

Manual run inputs:

| Input | Default | Effect |
|---|---|---|
| `version` | empty | Overrides the version from `Cargo.toml`, e.g. `0.3.0-rc.1` for a release candidate. The build jobs write it into the manifests with `scripts/set-version.sh` on the runner; nothing is committed. The tag is `v<version>` on the selected commit. |
| `dry_run` | off | Same builds and checks, artifacts on the run page (14 days); no tag and no release. The label is `v<ver>-dev.<sha7>`. |

A version with a pre-release part (`0.3.0-rc.1`, `0.3.0-beta.2`) makes a release marked
**pre-release**, and its notes fall back to `[Unreleased]` when CHANGELOG has no section for
it; a stable version needs its CHANGELOG section. A manual run refuses a tag that already
exists on another commit. Tags created by the workflow don't start a second, tag-push run.

Jobs:

| Job | Runs on | Does |
|---|---|---|
| `prepare` | ubuntu-latest | Resolves the version: a pushed tag must match `Cargo.toml` and `app/package.json`; a manual run takes `Cargo.toml` or the `version` input. Extracts the release notes (a stable tag needs a CHANGELOG section; pre-releases fall back to `[Unreleased]`). Regenerates the license inventory and `THIRD_PARTY_LICENSES.txt`. Builds the SBOM zip. |
| `create-release` | ubuntu-latest | On a manual run, creates the tag. Creates or updates the draft (refuses if the tag's release is already published). Uploads notices, licenses and the SBOM zip. |
| `bundle` ×6 | macos-14 (arm64, x64), windows-latest (x64), windows-11-arm (arm64), ubuntu-22.04 (x64), ubuntu-22.04-arm (arm64) | `tauri build --ci --target … --bundles …` with the release resources config, optional signing, *Verify bundles*, `.sha256` files, upload |
| `cli` ×6 | same | `cargo build --release -p pbiview-cli` (`dashcraft` and the `pbiview` alias), smoke test (`--version` of both, `inspect` on the committed Acme Outfitters example PBIP), archive + `.sha256`, upload |
| `finalize` | ubuntu-latest | `SHA256SUMS.txt` of all assets, optional build-provenance attestation |

Pre-release versions: WiX only accepts a numeric pre-release (`1.2.0-1`). For any other
pre-release (`1.2.0-rc.1`) the Windows job builds the NSIS installer only and logs a
warning.

## Signing and notarization

Without certificates every build still succeeds, unsigned:

- macOS gets an ad-hoc signature.
- Windows installers are unsigned.
- Linux artifacts are never signed; verify them with the checksums and attestations.

Each platform's signing switches on when its secrets are present. Add them under
**Settings → Secrets and variables → Actions** (repository or `release` environment
secrets); the full table is in [ci.md](ci.md#signing-secrets-optional).

### Apple (Developer ID + notarization)

1. Join the Apple Developer Program as an organization (yearly fee; D-U-N-S number
   required).
2. Create a **Developer ID Application** certificate. Either use Xcode → Settings → Accounts →
   Manage Certificates, or create a CSR in Keychain Access and upload it at
   developer.apple.com → Certificates. Only the Account Holder can create Developer ID
   certificates.
3. Export the certificate **with its private key** from Keychain Access as `.p12`, with a
   strong password.
4. Set the secrets:
   - `APPLE_CERTIFICATE`: `base64 -i cert.p12 | pbcopy`
   - `APPLE_CERTIFICATE_PASSWORD`
   - optionally `APPLE_SIGNING_IDENTITY`, e.g. `Developer ID Application: Org Name (TEAMID)`
     (`security find-identity -v -p codesigning`)
5. Notarization:
   - `APPLE_ID`: the developer account's email.
   - `APPLE_PASSWORD`: an **app-specific password** created at account.apple.com → Sign-In
     and Security.
   - `APPLE_TEAM_ID`: Membership details.

   Tauri submits the `.app` with `notarytool`, waits, and staples the ticket. The first
   notarization of a new app can take hours.

Tauri also supports App Store Connect API keys (`APPLE_API_KEY`, `APPLE_API_ISSUER`,
`APPLE_API_KEY_PATH`) instead of an Apple ID. The workflow doesn't wire them yet.

Signed builds use the hardened runtime. If a future feature needs entitlements (for
example JIT), add `bundle.macOS.entitlements`.

### Windows (Authenticode)

Since June 2023, publicly trusted code-signing keys (OV and EV) must be generated and kept
on an HSM or hardware token. A new certificate usually **can't be exported as a `.pfx`**.
Two options:

- **Cloud signing (recommended):** Azure Trusted Signing, or a CA's cloud HSM (DigiCert
  KeyLocker, SSL.com eSigner). Set `bundle.windows.signCommand` to the vendor's CLI (for
  Azure, `trusted-signing-cli -e <endpoint> -a <account> -c <profile> -d DashCraft %1`), pass
  its credentials as secrets, and replace the *Enable Windows code signing* step. Tauri runs
  the command for the app executable, the NSIS installer and uninstaller, and the MSI.
- **Legacy exportable `.pfx`:** set `WINDOWS_CERTIFICATE` (base64 of the `.pfx`:
  `[Convert]::ToBase64String([IO.File]::ReadAllBytes('cert.pfx'))`) and
  `WINDOWS_CERTIFICATE_PASSWORD`. The workflow imports it and passes its thumbprint, SHA-256
  and DigiCert timestamping to Tauri.

An OV certificate still starts with SmartScreen warnings until the publisher builds up
reputation. EV certificates no longer get instant reputation either (Microsoft changed that
in 2024).

### Updater

The Tauri updater is not enabled, so there are no `latest.json` or `.sig` artifacts. Enabling
it needs `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` secrets,
`bundle.createUpdaterArtifacts` and the updater plugin.

## License inventory and SBOM

[`scripts/license-inventory.py`](../scripts/license-inventory.py) (Python 3.9+, standard
library only) regenerates two marked blocks in
[`THIRD_PARTY_NOTICES.md`](../THIRD_PARTY_NOTICES.md):

- **Rust:** `cargo tree -e normal --locked` from `pbiview-cli` and `pbiview-app` for the four
  release targets. That is every crate compiled into a shipped binary, including
  proc-macros; build-only and dev-only dependencies are excluded. Each row has crate,
  versions, SPDX expression and repository, and a summary counts crates per license.
- **npm:** a throw-away `vite build --sourcemap` (in `target/`, deleted afterwards). The
  packages whose files appear in the source maps are exactly what is in `app/dist`: today
  `@tauri-apps/api`, `@tauri-apps/plugin-dialog`, `svelte` (runtime) and `clsx`. Build tools
  are excluded.

```sh
(cd app && npm ci)
python3 scripts/license-inventory.py                       # rewrite the blocks
python3 scripts/license-inventory.py --check               # exit 1 + diff if stale
python3 scripts/license-inventory.py --texts out.txt       # also collect full license texts
```

The output is deterministic: sorted, with no dates or local paths. Policy is still enforced
by `cargo deny check licenses` (`deny.toml`); the script only lists. CI's `app` job (Linux)
warns when the blocks are stale. The release job regenerates them, so shipped notices
always match the lockfiles.

`--texts` writes `THIRD_PARTY_LICENSES.txt`: every `LICENSE*`, `COPYING*` and `NOTICE*` file
of the listed crates and npm packages, with identical texts printed once (about 0.9 MB). A
short list at the end names packages that publish no license file; their SPDX expression
applies. Release builds bundle this file under `licenses/` and attach it to the release.
Local builds ship only the notices.

**SBOMs** (`dashcraft-<tag>-sbom.zip`, CycloneDX 1.5 JSON):

- `dashcraft_bin_<target>.cdx.json`, `pbiview_bin_<target>.cdx.json` (the alias) and `dashcraft-app_bin_<target>.cdx.json`: one per binary
  and target. Made with [`cargo-cyclonedx`](https://github.com/CycloneDX/cyclonedx-rust-cargo)
  (Apache-2.0, maintained by the CycloneDX project), `--describe binaries --no-build-deps`,
  with timestamps pinned to the commit (`SOURCE_DATE_EPOCH`).
- `dashcraft-app_npm.cdx.json`: `npm sbom --sbom-format cyclonedx` of `app/package-lock.json`.
  Build-only packages are flagged `cdx:npm:package:development=true`.

Locally: `cargo install cargo-cyclonedx --locked`, then run the commands of the *SBOMs*
step in `release.yml`. The files are written next to each `Cargo.toml`; move or delete
them.

## Icons

`app/src-tauri/icons/app-icon.svg` is original artwork: a teal rounded square with a bar
chart and trend line. It contains no Microsoft or Power BI marks. Regenerate all sizes
(PNG, `icon.icns`, `icon.ico`, Windows `Square*Logo`) after editing it:

```sh
(cd app && npm ci) && scripts/make-app-icon.sh
```

The script rasterizes a 1024×1024 master (`target/app-icon/1024x1024.png`) with the Tauri
CLI, runs `tauri icon` on it, and removes the mobile icon sets.

## Known limitations

- **Unsigned builds trigger OS warnings.** macOS:
  - Gatekeeper blocks the first launch. Allow it under System Settings → Privacy & Security
    → *Open Anyway*, or run `xattr -dr com.apple.quarantine /Applications/DashCraft.app`.
  - macOS 15 removed the Control-click → Open bypass.

  Windows:
  - SmartScreen shows "Windows protected your PC" → *More info* → *Run anyway*.
  - Some managed machines block unsigned MSIs entirely.
- **Linux needs WebKitGTK 4.1** (`libwebkit2gtk-4.1-0`): Ubuntu 22.04+, Debian 12+. Older
  distributions (Ubuntu 20.04, Debian 11) can't run the deb or the AppImage.
  The AppImage also needs glibc ≥ 2.35 (the Ubuntu 22.04 build host) and FUSE 2 (or
  `--appimage-extract-and-run`).
- The AppImage doesn't register file associations by itself (see [File associations](#file-associations)).
- The macOS x64 app and CLI are cross-compiled. The release only smoke-tests the native
  targets; the Windows and Linux arm64 builds run on native Arm64 runners and are tested.
- Windows arm64 has no MSI, only the NSIS `-setup.exe`.
- No auto-update; users download new versions from the releases page.
- Linux artifacts have no GPG signature; use `SHA256SUMS.txt` and attestations.
