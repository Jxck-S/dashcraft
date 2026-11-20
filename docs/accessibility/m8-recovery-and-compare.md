# M8 accessibility: recovery, external changes and Compare

The checks use a copy of the synthetic Acme Outfitters project. The desktop checks
`scripts/app-checks/recovery.mjs` and `external-changes.mjs` exercise A1–A5 on WebView2.
They cover keyboard order, focus when dialogs open and close, status messages, and the IBM
accessibility checker and contrast checks when the checker is installed.

## A6 screen-reader runs

Steps S33–S44 in `scripts/app-checks/screen-reader.mjs` follow the
[ADR-0030](../adr/0030-screen-readers-in-ci.md) procedure:

1. Edit a measure on disk and wait for the external-change announcement. Tab to **Compare**
   and open it. The reader should say the dialog title and then the result, such as
   “1 different.” Tab to the changed measure's TMDL and press Esc to return to **Compare**
   (S33–S38).
2. Shift+Tab to **Reload** and press Enter. The reader should announce the reload (S39).
3. Edit a measure without saving and terminate the app. Reopen it, Tab to **Restore** in
   **Document recovery**, and press Enter. The reader should announce the restored edit
   (S40–S44).

On Windows, S34a, S37a and S42a also check the names and control types of the notification
bar buttons, Compare, and Document recovery in the UI Automation tree that Narrator reads.

The [scheduled run on `0958fd8`](https://github.com/Jxck-S/dashcraft/actions/runs/43879514951)
ran the journey before the Compare announcement change. S36 was informational and cleared the
speech log after the dialog opened. The table records what the reports captured.

| Reader | M8 results on `0958fd8` |
|---|---|
| NVDA, WebView2 | 11 steps passed. S36 did not capture the comparison result. |
| VoiceOver, WKWebView | 8 steps passed. S36 and S44 had informational misses. S40 did not capture the edit announcement and S41 reported focus on the banner. |
| Orca, WebKitGTK | 7 steps passed. S36 had an informational miss. S33, S39, S40 and S43 missed announcements. |
| Narrator, WebView2 | Uses the UI Automation tree per ADR-0030. The M8 tree checks were added after this run. |

Compare now keeps a live region mounted and updates it after the dialog takes focus. The
visible legend also describes the snippet when it receives focus. S36 keeps the speech captured
since the dialog opened and requires NVDA to say the count. A6 completion depends on the
updated hosted runs and UI Automation evidence.
