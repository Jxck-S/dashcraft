# ADR-0015: Accessibility target and verification

- Status: Accepted
- Date: 2026-11-05
- Deciders: DashCraft maintainers
- Related: [ADR-0001](0001-ui-tauri-svelte.md), [ADR-0012](0012-revisioned-document-state.md)

## Context

The desktop shell is adding interactive editing surfaces. Accessibility cannot be postponed
until the final audit: keyboard, focus, names, contrast and reduced-motion behavior are
properties of each new control and interaction, while screen-reader behavior differs across
the platform webviews used by the app.

## Decision

1. **Target.** New and materially changed app surfaces target WCAG 2.2 AA. This is a product
   acceptance target, not a claim that every existing surface has already passed an independent
   conformance audit.
2. **Per-surface checklist.** Each milestone that adds or changes UI tests and records:
   - keyboard-only operation, no keyboard traps and documented shortcuts;
   - logical focus order and visible focus;
   - accessible names and roles, plus automated checks with no serious or critical violations;
   - text contrast of at least 4.5:1 (3:1 for large text) and component contrast of at least 3:1
     in each available shell theme;
   - respect for the operating system's reduced-motion preference.
3. **Screen-reader smoke tests.** Before a milestone is considered complete, its changed
   interaction paths receive a scripted smoke test with VoiceOver on macOS/WKWebView, Narrator
   and NVDA on Windows/WebView2, and Orca on Linux/WebKitGTK. Record the platform, webview,
   task, outcome and any issue in the milestone's verification notes. A missing platform is a
   documented incomplete gate, not a pass.
4. **Audit.** M31.8 performs an independent accessibility audit before 1.0. It supplements,
   rather than replaces, the per-milestone checks.
5. **Scope.** The checklist applies to the app's new or changed controls and interactions; it
   does not require the Rust renderer to provide assistive semantics through SVG beyond the
   separate canvas and visual-accessibility work planned in M25.

## Alternatives considered

| Option | Why not chosen |
|---|---|
| Defer accessibility until M31 | Defects in keyboard and focus behavior become more expensive to retrofit after editing workflows are established. |
| Require an independent audit for every milestone | Duplicates substantial review effort; automated checks and scripted smoke tests provide continuous evidence, with an independent audit before release. |
| Claim whole-app WCAG conformance from automated checks | Automated tools cannot establish all keyboard and screen-reader behavior, and existing surfaces have not yet been audited. |

## Consequences

- New app controls need accessible names, keyboard behavior and focus management as part of
  their implementation, not as optional polish.
- Verification results must identify platform and webview; untested platforms remain visible
  rather than being represented as passing.
- M25 and M31.8 remain responsible for visual-canvas semantics and the independent audit.
