# Accessibility results: M7 formula bar

The accessibility checklist A1–A6 of [DESKTOP-PLAN.md W5](../DESKTOP-PLAN.md#w5-accessibility)
([ADR-0015](../adr/0015-accessibility-target.md), WCAG 2.2 AA), run on the formula bar that M7.5
turns into a DAX editor (acceptance criterion 5 of M7):

- token colours and wavy squiggles under errors and warnings;
- the suggestion list with its description panel;
- parameter info after `(` and `,`;
- descriptions on Ctrl+K Ctrl+I or pointing;
- Shift+Alt+F formatting;
- Desktop's formula editor keys (below), including Ctrl+M, which makes Tab move focus instead of
  indenting.

The surfaces M6 added around it (Commit, Cancel, expand, the Quick Access Toolbar, the Properties
fields) are in [m6-editing.md](m6-editing.md); their checks still pass.

## Run

| | |
|---|---|
| Date | 2026-11-07 |
| Build | debug build of the M7.5b branch (`npx tauri build --debug --no-bundle`) |
| Platform | Windows 11 Pro (build 26300), x64 |
| Webview | WebView2 157.0.4341.42 |
| Shell themes | one (light); every contrast number below is for it |
| Document | a copy of the synthetic Acme Outfitters example, under `target/tmp/app-checks/` |

## Method

[`scripts/app-checks/`](../../scripts/app-checks/) drives the real app over the WebView2 DevTools
protocol, as for M6. Keys and text go in as input events, so the browser's own commands (cut,
copy, undo) run as they do for a user.

- `formula-intellisense.mjs` checks the IntelliSense of M7.5a (20 checks).
- `formula-keys.mjs` checks Desktop's editor keys of M7.5b and runs A1–A5 on the formula bar with
  the suggestion list, parameter info, a description and a squiggle open (43 checks). It uses the
  IBM Equal Access checker (`accessibility-checker-engine` 4.0.34, Apache-2.0, `WCAG_2_2`
  policy), computed styles and `uia.ps1` (the UI Automation tree), through the shared helpers in
  `a11y.mjs`.
- `editing-a11y.mjs` and `editing-smoke.mjs` (M6) run again unchanged in what they check.

```sh
(cd app && npx tauri build --debug --no-bundle)
npm install --prefix target/tmp/a11y --no-save accessibility-checker-engine
node scripts/app-checks/formula-intellisense.mjs   # 20 passed, 0 failed
node scripts/app-checks/formula-keys.mjs           # 43 passed, 0 failed
node scripts/app-checks/editing-a11y.mjs           # 17 passed, 0 failed
node scripts/app-checks/editing-smoke.mjs          # 23 passed, 0 failed
```

## Results

| Item | Result | Evidence |
|---|---|---|
| A1 Keyboard | ✅ Pass | Everything is done from the keyboard: the suggestion list (typing, Ctrl+Space, ↑ ↓ PgUp PgDn, Enter or Tab, Esc), parameter info (Ctrl+Shift+Space), descriptions (Ctrl+K Ctrl+I, Esc), formatting (Shift+Alt+F) and every key in the table below. Tab indents, as in Desktop; Ctrl+M switches Tab to moving focus and back (Monaco's *Toggle Tab Key Moves Focus*), so Tab is not a keyboard trap. The editor's accessible description says which mode is on ("Tab indents; Ctrl+M makes Tab move focus."), and the switch is announced. The mode holds for every formula editor in the window until switched back. Enter commits and Esc cancels, as before. The keys are listed in [desktop-ui-reference.md](../../app/docs/desktop-ui-reference.md#keyboard). |
| A2 Focus order | ✅ Pass | Focus stays in the editor while the suggestion list is open; the list follows the ARIA combobox pattern (`aria-activedescendant` names the selected suggestion, `aria-controls` the list). After Ctrl+M, Tab leaves the editor for the next control (*Collapse the formula bar*). The editor's focus ring is a solid 2 px line at 5.21:1. |
| A3 Names and roles | ✅ Pass | The checker finds no violation with the suggestion list and a squiggle open, with parameter info open, or with a description open. UI Automation shows the editor as *Measure formula*, Edit, with its value, and the list as *Suggestions*, List. The status live region announces the number of suggestions, the inserted suggestion, formatting and the Tab mode. The problem at the caret, the parameter info (a `tooltip`) and the Tab mode are part of the editor's accessible description, and `aria-invalid` is set while there is an error. |
| A4 Contrast | ✅ Pass | Suggestion text at least 5.4:1, the selected suggestion included; the selected suggestion's outline 5.39:1; the error squiggle 5.86:1 and the warning squiggle 5.73:1 on the editor background; parameter info text 6.19:1; description text 6.07:1. Token colours are at least 4.5:1 (M7.5a); the lowest text in the formula bar is 5.98:1. |
| A5 Reduced motion | ✅ Pass | No animation or transition in the formula bar or its pop-ups. |
| A6 Screen readers | ⚠️ Incomplete | Not run yet with any of the four screen readers; the Windows evidence is the UI Automation tree (A3), not a spoken run. The task is below. Per ADR-0015, this is a documented incomplete gate, not a pass. |

### Left for review by hand

The checker reports some rules as *potential* issues:

- `style_focus_visible`: covered by the focus-ring measurement in A2.
- `input_label_visible`: the editor is named *Measure formula* (or *Calculated column formula*,
  *Calculated table formula*) by `aria-label`; like Desktop's formula bar it has no visible label.
- `aria_keyboard_handler_exists` and `widget_tabbable_exists`: the suggestion list is not in the
  tab order on purpose. In the combobox pattern the editor keeps focus and handles the list's keys
  itself.

## Desktop's formula editor keys

Microsoft documents the keys of Desktop's formula editor in *Formula editor in Power BI Desktop*.
The formula bar supports those that a single caret can do:

| Keys | Action | Here |
|---|---|---|
| Tab, Shift+Tab | Indent (to the next tab stop, or the selected lines), outdent | ✅ |
| Ctrl+M | Tab moves focus, and back | ✅ |
| Ctrl+], Ctrl+[ | Indent, outdent the lines | ✅ |
| Shift+Enter | New line (keeps the indentation; the bar expands) | ✅ |
| Ctrl+Shift+Enter | Insert line above | ✅ |
| Alt+↑, Alt+↓ | Move the lines up, down | ✅ |
| Shift+Alt+↑, Shift+Alt+↓ | Copy the lines up, down | ✅ |
| Ctrl+Shift+K | Delete the lines | ✅ |
| Ctrl+L | Select the line (again: the next one too) | ✅ |
| Ctrl+X, Ctrl+C without a selection | Cut, copy the line | ✅ |
| Ctrl+Shift+\ | Jump to the matching bracket | ✅ |
| Shift+Alt+→, Shift+Alt+← | Expand, shrink the selection (word, brackets, line, all) | ✅ |
| Ctrl+↑, Ctrl+↓, Alt+PgUp, Alt+PgDn | Scroll a line, a page | ✅ |
| Home, End, Ctrl+Home, Ctrl+End | Line, text start and end | ✅ (the browser's) |
| Ctrl+Z, Ctrl+Y | Undo, redo in the editor, each key edit one step | ✅ (the browser's) |
| Ctrl+G | Go to line | ❌ |
| Ctrl+U | Undo the last cursor move | ❌ |
| Alt+Click, Ctrl+Alt+↑ ↓, Shift+Alt+I, Ctrl+Shift+L, Ctrl+F2 | Extra cursors | ❌ |
| Shift+Alt+drag, Ctrl+Shift+Alt+arrows | Column (box) selection | ❌ |

The editor is a text field with one caret, so extra cursors and column selection are out of reach
without a different editor component; they are listed as a gap in [GAPS.md](../GAPS.md).

## A6 task (for each screen reader)

Run it on a copy of Acme Outfitters with VoiceOver on macOS (WKWebView), Narrator and NVDA on
Windows (WebView2) and Orca on Linux (WebKitGTK):

1. Select **Total Sales** in the Data pane and move to the formula bar. Expected: "Measure
   formula, edit", the formula, and "Tab indents; Ctrl+M makes Tab move focus."
2. Go to the end and type ` + COUNTR`. Expected: the number of suggestions, then the selected
   suggestion as ↓ moves through the list.
3. Press Tab. Expected: "Inserted COUNTROWS" and the parameter info.
4. Type `Orders) + [Nope]`. Expected: the formula is reported invalid, with "unknown measure or
   column" in its description.
5. Press Ctrl+K Ctrl+I with the caret on `Nope`. Expected: the description is read.
6. Press Shift+Enter, then Alt+↑. Expected: the new line moves up; the text is read again.
7. Press Ctrl+M, then Tab. Expected: "Tab now moves focus", then the *Collapse the formula bar*
   button. Shift+Tab back, press Ctrl+M again.
8. Press Esc. Expected: the edit is cancelled and the formula read as before.

For each screen reader, record whether the task completed and any announcement that was missing
or wrong.

## Known limits

- **Shift+Enter.** Microsoft's table calls it "Insert line below". Here it breaks the line at the
  caret and keeps its indentation, as Enter does in an editor where Enter does not commit; the
  difference from Desktop, if any, is unconfirmed.
- **No extra cursors or column selection** (table above).
- **One shell theme.** Contrast is measured for the light theme only, the only one so far.
