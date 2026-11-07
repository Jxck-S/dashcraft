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
- `editing-a11y.mjs` runs A1–A5 (17 checks). It uses:
  - the IBM Equal Access checker (`accessibility-checker-engine` 4.0.34, Apache-2.0, `WCAG_2_2`
    policy), installed under `target/tmp/a11y` and not a dependency of the repository;
  - computed styles for the focus rings and contrast ratios;
  - `uia.ps1`, which reads the Windows UI Automation tree. That tree is what Narrator reads, and
    NVDA reads the same names and roles through IAccessible2.

```sh
(cd app && npx tauri build --debug --no-bundle)
npm install --prefix target/tmp/a11y --no-save accessibility-checker-engine
node scripts/app-checks/editing-smoke.mjs     # 23 passed, 0 failed
node scripts/app-checks/editing-a11y.mjs      # 17 passed, 0 failed
```

## Results

| Item | Result | Evidence |
|---|---|---|
| A1 Keyboard | ✅ Pass | Everything is done from the keyboard. The formula is typed and committed with Enter; Shift+Enter starts a new line, and Esc cancels a draft. A Properties field commits on Enter (Ctrl+Enter in Description) or when it loses focus; Esc cancels and keeps focus in the field. Ctrl+S, Ctrl+Shift+S, Ctrl+Z and Ctrl+Y work; Ctrl+Z and Ctrl+Y stay with a text field while it has focus. In the question, Esc answers Cancel. Tab leaves every new control, so there is no keyboard trap. The shortcuts are listed in [desktop-ui-reference.md](../../app/docs/desktop-ui-reference.md#keyboard). |
| A2 Focus order | ✅ Pass | Tab moves through the toolbar as Save → Undo → Search. A disabled Redo is skipped, which is correct. In the Properties pane, Tab moves Description → Display folder → Format string. The question takes focus when it opens. Every focus ring is a solid 2 px line: 4.94:1 on the toolbar buttons, 5.39:1 on the fields and 5.21:1 on the formula editor. |
| A3 Names and roles | ✅ Pass | Every new control has a name: Save, Undo, Redo, Cancel, Commit, Measure formula, Explain with AI, Expand the formula bar, Description, Display folder and Format string. The checker finds no violation on the toolbar, formula bar, Properties fields, File menu or question. UI Automation shows the toolbar as a ToolBar; the buttons as Button, with their enabled state; the formula and fields as Edit, with their values; and the question as a Window with its buttons. The status bar message, which reports each edit, undo, redo and save, is a polite live region. |
| A4 Contrast | ✅ Pass | The lowest ratios are 5.98:1 for text, 6.19:1 for icons and input borders, and 5.39:1 for the question's text and buttons. The checks found three failures, all now fixed. The gutter line numbers were 3.34:1 and now use the muted text colour. The input borders were 1.53:1 and now have a `--color-border-accessible` bottom border (#616161), as Fluent 2 inputs do. The focus rings were 1 px and are now 2 px. |
| A5 Reduced motion | ✅ Pass | The new controls have no animation and no transition. |
| A6 Screen readers | ⚠️ Incomplete | Not run yet with any of the four screen readers. Windows evidence so far is the UI Automation tree (A3), not a spoken run. The task to run is below. Per ADR-0015, this is a documented incomplete gate, not a pass. |

### Left for review by hand

The checker reports some rules as *potential* issues. These need a person to judge them:

- `style_focus_visible`: covered by the focus-ring measurements in A2.
- `input_label_visible`: the fields have visible `<label>`s.
- `element_tabbable_unobscured`: neither the File menu nor the question hides a focused control.
- `aria_descendant_valid`: the File menu's command buttons contain `<strong>` (existing markup).

## A6 task (for each screen reader)

Run it on a copy of Acme Outfitters with:

- VoiceOver on macOS (WKWebView);
- Narrator and NVDA on Windows (WebView2);
- Orca on Linux (WebKitGTK).

Steps:

1. Open `AcmeOutfitters.pbip`. Expected: the window title is read.
2. In the Data pane, select **Total Sales**. Move to the formula bar. Expected: "Measure formula,
   edit" and the formula text.
3. Type ` + 1` and press Enter. Expected: the status message "Edit measure 'Total Sales'", and a
   title that now starts with `*`.
4. Tab to the Quick Access Toolbar. Expected: "Save, button"; "Undo, button" (its tooltip names
   the edit); Redo is skipped.
5. Press Ctrl+Z, then Ctrl+Y. Expected: the status messages "Undid …" and "Redid …".
6. Open View → Properties, Tab to **Description**, type text and press Ctrl+Enter. Expected:
   "Description, edit", then the value.
7. Open File and choose Close. Expected: the "Unsaved changes" dialog, its question and its three
   buttons. Esc cancels.
8. Press Ctrl+S. Expected: the "Saved …" status, and a title without `*`.

For each screen reader, record whether the task completed and any announcement that was missing
or wrong.

## Known limits

- **macOS quit.** Cmd+Q and Quit in the Dock end the app without the *Unsaved changes* question:
  Tauri does not let an app intercept them yet
  ([tauri-apps/tauri#9198](https://github.com/tauri-apps/tauri/issues/9198),
  [tauri-apps/tauri#12978](https://github.com/tauri-apps/tauri/issues/12978)). Closing the window
  asks first on every platform, whether by the title bar, Alt+F4 or Cmd+W.
- **Ctrl+S in a field.** Pressing Ctrl+S while a formula or Properties field holds an
  uncommitted draft saves the document without that draft. Commit the draft with Enter first.
