# ADR-0030: Screen-reader smoke tests in CI

- Status: Accepted
- Date: 2026-11-11
- Deciders: the project owner
- Related: [ADR-0015](0015-accessibility-target.md) (accessibility target; this ADR changes how
  its decision 3 is met), [ci.md](../ci.md#screen-readeryml-screen-readers-weekly-or-on-app-prs)
  (the workflow), [accessibility notes](../accessibility/) (the results per milestone). Numbers
  0016 to 0028 are reserved for the proposals listed in DESKTOP-PLAN.

## Context

ADR-0015 decision 3 asks for a scripted screen-reader smoke test of each milestone's changed
interactions with VoiceOver on macOS (WKWebView), Narrator and NVDA on Windows (WebView2) and Orca
on Linux (WebKitGTK), a missing platform being a documented incomplete gate. M6 and M7 ended with
A6 incomplete: the runs needed a person at three machines with four screen readers, for every
milestone, and they would not repeat when the app changes. The project owner also wants nothing
installed on, or taking over, a developer's machine.

[Guidepup](https://github.com/guidepup/guidepup) (MIT) starts NVDA, VoiceOver and Orca, presses
keys, runs their commands and returns what they said. Its setup package configures GitHub's hosted
runners for them (AppleScript control and the privacy database on macOS, Orca with D-Bus and Xvfb
on Linux; NVDA is a portable copy). Narrator has no automation interface; it reads the UI
Automation tree.

## Decision

1. **A6 runs in CI.** The `Screen readers` workflow (`.github/workflows/screen-reader.yml`) runs
   `scripts/app-checks/screen-reader.mjs` on pull requests that touch the app's front end or the
   check itself, every Monday and on demand. The script opens a copy of Acme Outfitters in the
   debug app, presses the keys a user would, and checks what the screen reader said after each
   step against a pattern. A milestone's A6 adds the steps of its journey to the script.
2. **Gates.** NVDA on Windows (WebView2) gates the job: a required step that fails fails the
   pull request. VoiceOver on macOS (WKWebView) and Orca on Linux (WebKitGTK under Xvfb) are
   informational (`continue-on-error`). Both complete the journey, but in CI VoiceOver drops some
   announcements and has no command that reads the window title, and Orca does not voice
   WebKitGTK's polite live-region announcements. A reader becomes a gate once its required steps
   pass; its misses are tracked in the Accessibility row of GAPS.md until then.
   A step whose wording depends on a reader's verbosity settings is informational on every
   reader.
3. **Narrator.** Its evidence is the UI Automation tree that it reads (`scripts/app-checks/uia.ps1`
   and the checks of A3), not a spoken run.
4. **When A6 passes.** A milestone's A6 passes when every required step passes on the gating
   readers, the informational readers complete the journey with their misses recorded in the
   milestone's accessibility notes, and the UI Automation check passes. This replaces "every
   platform, or a documented incomplete gate" of ADR-0015 decision 3; the rest of ADR-0015 stands.
5. **People.** Before the independent audit of 1.0 (M31.8), a person runs the journeys by hand with
   all four screen readers, Narrator included. A fault found by a person becomes a step in the
   script where a reader can check it.
6. **CI only.** Guidepup is installed by the workflow, at pinned versions, into
   `target/tmp/screen-reader/tools`. It is never a dependency of the app and never installed by
   DashCraft; the script takes over the keyboard and speech, so it is not run on a workstation.

## Alternatives considered

| Option | Why not chosen |
|---|---|
| A person at each machine for every milestone (ADR-0015 as written) | Slow, does not repeat when the app changes, and left M6 and M7 with A6 incomplete. |
| Guidepup on a developer machine | Takes over the keyboard and speech and changes system settings (VoiceOver's AppleScript control, the privacy database). |
| The accessibility tree only (UI Automation, AX, AT-SPI) | Shows names, roles and states, but not what is spoken: live-region announcements, descriptions and their order. Kept as Narrator's evidence. |
| A hosted screen-reader testing service | Costs money and sends builds to a third party, against the offline-first rule. |

## Consequences

- A6 no longer waits for a person, and a change that breaks what a screen reader says fails the
  pull request that makes it, on the gating readers.
- Each run builds the debug app on three hosted runners (about 25 minutes, cold on pull
  requests), so the workflow runs only when the front end changes, plus weekly.
- Screen readers word things differently from version to version: patterns match the words that
  matter, not whole phrases, and the reports keep everything that was said.
- Narrator is not heard until M31.8, and the informational readers' misses stay recorded in the
  accessibility notes until they are fixed or explained.
