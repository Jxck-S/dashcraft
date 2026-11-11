# Accessibility results: M6 editing surfaces

The accessibility checklist A1–A6 of [DESKTOP-PLAN.md W5](../DESKTOP-PLAN.md#w5-accessibility)
([ADR-0015](../adr/0015-accessibility-target.md), WCAG 2.2 AA), run on the editing surfaces that
M6.8 adds to the app:

- the Quick Access Toolbar's Save, Undo and Redo;
- the shortcuts Ctrl+S, Ctrl+Shift+S, Ctrl+Z and Ctrl+Y (⌘ on macOS);
- the `*` unsaved marker in the title;
- the editable formula bar;
- the Properties pane fields Description, Display folder and Format string;
- File → Save and Save as;
- the *Unsaved changes* question (Save, Don't save, Cancel).

## Run

| | |
|---|---|
| Date | 2026-11-07 |
| Build | debug build of the M6.8 branch (`npx tauri build --debug --no-bundle`) |
| Platform | Windows 11 Pro (build 26300), x64 |
| Webview | WebView2 157.0.4341.42 |
| Shell themes | one (light); every contrast number below is for it |
| Document | a copy of the synthetic Acme Outfitters example, under `target/tmp/app-checks/` |

## Method

[`scripts/app-checks/`](../../scripts/app-checks/) drives the real app over the WebView2 DevTools
protocol. `cdp.mjs` launches the app with `--remote-debugging-port`; keys and text go in as
input events.

- `editing-smoke.mjs` runs the editing task end to end (23 checks).
- `editing-a11y.mjs` runs A1–A5 (21 checks). It uses:
  - the IBM Equal Access checker (`accessibility-checker-engine` 4.0.34, Apache-2.0, `WCAG_2_2`
    policy), installed under `target/tmp/a11y` and not a dependency of the repository;
  - computed styles for the focus rings and contrast ratios;
  - `uia.ps1`, which reads the Windows UI Automation tree. That tree is what Narrator reads, and
    NVDA reads the same names and roles through IAccessible2.
- `screen-reader.mjs` runs A6 in CI with NVDA, VoiceOver and Orca
  ([A6 below](#a6-screen-reader-runs); 2026-11-11, GitHub-hosted runners).

```sh
(cd app && npx tauri build --debug --no-bundle)
npm install --prefix target/tmp/a11y --no-save accessibility-checker-engine
node scripts/app-checks/editing-smoke.mjs     # 23 passed, 0 failed
node scripts/app-checks/editing-a11y.mjs      # 21 passed, 0 failed
```

## Results

| Item | Result | Evidence |
|---|---|---|
| A1 Keyboard | ✅ Pass | Everything is done from the keyboard. The formula is typed and committed with Enter; Shift+Enter starts a new line, and Esc cancels a draft. A Properties field commits on Enter (Ctrl+Enter in Description) or when it loses focus; Esc cancels and keeps focus in the field. Ctrl+S, Ctrl+Shift+S, Ctrl+Z and Ctrl+Y work; Ctrl+Z and Ctrl+Y stay with a text field while it has focus. In the question, Esc answers Cancel. Tab leaves every new control, so there is no keyboard trap. The shortcuts are listed in [desktop-ui-reference.md](../../app/docs/desktop-ui-reference.md#keyboard). |
| A2 Focus order | ✅ Pass | Tab moves through the toolbar as Save → Undo → Search. A disabled Redo is skipped, which is correct. In the Properties pane, Tab moves Description → Display folder → Format string. The question takes focus when it opens. Every focus ring is a solid 2 px line: 4.94:1 on the toolbar buttons, 5.39:1 on the fields and 5.21:1 on the formula editor. |
| A3 Names and roles | ✅ Pass | Every new control has a name: Save, Undo, Redo, Cancel, Commit, Measure formula, Explain with AI, Expand the formula bar, Description, Display folder and Format string. The checker finds no violation on the toolbar, formula bar, Properties fields, File menu or question. UI Automation shows the toolbar as a ToolBar; the buttons as Button, with their enabled state; the formula and fields as Edit, with their values; and the question as a Window with its buttons. The status bar message, which reports each edit, undo, redo and save, is a polite live region. |
| A4 Contrast | ✅ Pass | The lowest ratios are 5.98:1 for text, 6.19:1 for icons and input borders, and 5.39:1 for the question's text and buttons. The checks found three failures, all now fixed. The gutter line numbers were 3.34:1 and now use the muted text colour. The input borders were 1.53:1 and now have a `--color-border-accessible` bottom border (#616161), as Fluent 2 inputs do. The focus rings were 1 px and are now 2 px. |
| A5 Reduced motion | ✅ Pass | The new controls have no animation and no transition. |
| A6 Screen readers | ✅ Pass (ADR-0030) | Run in CI with each change to the app ([A6 below](#a6-screen-reader-runs)). NVDA (Windows, WebView2) passes every required step and gates the job. VoiceOver (macOS, WKWebView) and Orca (Linux, WebKitGTK) complete the journey and are informational; their misses are listed below. Narrator has no automation interface, so its evidence is the UI Automation tree (A3). |

### Left for review by hand

The checker reports some rules as *potential* issues. These need a person to judge them:

- `style_focus_visible`: covered by the focus-ring measurements in A2.
- `input_label_visible`: the fields have visible `<label>`s.
- `element_tabbable_unobscured`: neither the File menu nor the question hides a focused control.
- `aria_descendant_valid`: the File menu's command buttons contain `<strong>` (existing markup).

## A6 screen-reader runs

The task runs in CI, per [ADR-0030](../adr/0030-screen-readers-in-ci.md): the `Screen readers`
workflow ([ci.md](../ci.md#screen-readeryml-screen-readers-weekly-or-on-app-prs)) runs
`scripts/app-checks/screen-reader.mjs`, which drives the debug app with a real screen reader
through [Guidepup](https://github.com/guidepup/guidepup), on a copy of Acme Outfitters. The
script presses the keys a user would and checks what the screen reader says. Steps S1–S11 and
S23–S32 are this milestone's; S12–S22 are the M7 formula bar's
([m7-formula-bar.md](m7-formula-bar.md#a6-screen-reader-runs)).

| Screen reader | Platform, webview | Gate | Result |
|---|---|---|---|
| NVDA | Windows (`windows-latest`), WebView2 | Required | ✅ All 27 required steps pass (runs 4 and 5 on PR #66); the 5 optional steps are recorded as notes. |
| VoiceOver | macOS 15, WKWebView | Informational | ⚠️ The journey completes and 24 to 26 of 32 steps pass (runs 4 and 5). For M6: no VoiceOver command reads the window title, so the `*` of unsaved changes is not checked (S1, S8, S32); the first phrase after start is said but not captured (S2); applying the description is not always announced, and the field is read again instead (S25 failed in run 4, passed in run 5). |
| Orca | Ubuntu 26.04, WebKitGTK under Xvfb | Informational | ⚠️ The journey completes and 20 of 32 steps pass. Names, focus, the File menu, the question and the title are read. The polite status messages (edit, undo, redo, description, save) are not heard (S7, S10, S11, S25, S31), and Orca's "where am I" leaves out the field's description (S6). |
| Narrator | Windows, WebView2 | Not automated | Narrator has no automation interface. Its evidence is the UI Automation tree that it reads (A3); a person listens before the 1.0 audit (M31.8). |

A6 passes for M6 by ADR-0030's rule: every required step passes with NVDA, the informational
readers complete the journey with their misses listed above, and the UI Automation check passes.
Each run's report, with everything the reader said, is in the job summary and the
`screen-reader-<reader>` artifact.

Steps (Ctrl and Alt are ⌘ and ⌥ on macOS; End is ⌘→):

1. Open `AcmeOutfitters.pbip`. Expected: the window title, "AcmeOutfitters.pbip - DashCraft"
   (S1).
2. Press Alt+Q, type `Total Sales`, press Enter and Tab to the formula bar. Expected: "Search
   commands and fields", then "Measure formula, edit" and the formula (S2–S5).
3. Press End, type ` * 1` and press Enter. Expected: "Edit measure 'Total Sales'"; the title then
   starts with `*` (S7, S8).
4. Press Alt+Q, then Shift+Tab. Expected: "Quick access toolbar, tool bar", then "Undo, button"
   with the edit it undoes (S9).
5. Press Ctrl+Z, then Ctrl+Y. Expected: "Undid Edit measure 'Total Sales'" and "Redid …" (S10,
   S11).
6. Open the Properties pane through the search, Shift+Tab to **Description**, type text and press
   Ctrl+Enter. Expected: "Description", then "Change the description of measure 'Total Sales'"
   (S23–S25).
7. Press Alt+Q and Tab to **File**, press Enter, Tab to **Close** and press Enter. Expected:
   "File, dialog", "Close", then the "Unsaved changes" dialog. Esc cancels it, and focus goes
   back to File (S26–S30).
8. Press Ctrl+S. Expected: "Saved AcmeOutfitters.pbip: 1 file written", and a title without `*`
   (S31, S32).

The runs found two faults, both fixed with tests. Tab left the File backstage for the page
behind it; now the backstage, `Modal` and the Power Query dialog keep Tab inside (a shared
`use:modal` action), and focus goes back to File when the backstage closes. Option+Q did nothing on
macOS, where it types `œ`; Alt+Q now also matches the physical key.

## Known limits

- **macOS quit.** Cmd+Q and Quit in the Dock end the app without the *Unsaved changes* question:
  Tauri does not let an app intercept them yet
  ([tauri-apps/tauri#9198](https://github.com/tauri-apps/tauri/issues/9198),
  [tauri-apps/tauri#12978](https://github.com/tauri-apps/tauri/issues/12978)). Closing the window
  asks first on every platform, whether by the title bar, Alt+F4 or Cmd+W.
- **Ctrl+S in a field.** Pressing Ctrl+S while a formula or Properties field holds an
  uncommitted draft saves the document without that draft. Commit the draft with Enter first.
