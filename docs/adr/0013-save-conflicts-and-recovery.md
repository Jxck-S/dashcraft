# ADR-0013: Save, external changes and recovery

- Status: Accepted
- Date: 2026-11-05
- Deciders: DashCraft maintainers
- Related: [ADR-0004](0004-desktop-ready-architecture.md), [ADR-0007](0007-command-registry.md),
  [ADR-0012](0012-revisioned-document-state.md), [ADR-0014](0014-serialization-contract.md)

## Context

DashCraft can write a PBIP project or export a PBIT, but it does not yet save edits to the opened
project. A safe save must not overwrite edits made by another tool after DashCraft opened the
project, drop unknown files, or imply that unsupported embedded data was preserved. Recovery
must also stay outside the user's project and remain opt-in.

## Decision

1. **Save targets.** In-place `doc.save` is supported only for an opened PBIP project. Other
   readable sources use Save As to PBIP; PBIT export remains an explicit separate operation.
   Writing PBIX is out of scope. Saving a PBIX-derived document as PBIP does not carry embedded
   VertiPaq data, and the result and UI must state that plainly.
2. **Conflict detection.** At open and after each successful save, retain fingerprints for the
   source project files, including unmodelled files. Before an in-place save, compare the current
   project with that baseline. Added, removed or changed files are conflicts; the default is to
   refuse the save and return the affected relative paths. DashCraft does not silently merge or
   discard external edits.
3. **Explicit resolution.** The user may inspect/reload the external version or explicitly
   choose to keep DashCraft's version and overwrite the reported conflicts. Overwrite is an
   explicit per-operation choice, never the default. Reload replaces the open revision and
   clears its history only after it succeeds; unsaved edits are not silently discarded.
   Save As refuses to replace an existing target unless the caller explicitly requests
   overwrite.
4. **Write guarantees.** A save writes only changed, modeled files and preserves unmodelled
   project files. Each destination is staged in its own directory and atomically replaced.
   Multi-file saves are not claimed to be one filesystem transaction: if a later file fails,
   report the completed and failed paths, retain the dirty state, and allow a subsequent save to
   reconcile the project. The saved baseline and dirty marker advance only after all required
   writes succeed.
5. **Recovery boundary.** Autosave journals and recovery data, when implemented in M8, live in
   the operating system's per-user application data directory, never in the project. They contain
   only the minimum document-edit state needed for recovery, not credentials or external data.
   A recovered version is offered for explicit inspection and acceptance; it is never applied
   silently. Successful save or explicit discard removes its journal. Undo history is not
   persisted across sessions.
6. **Caller policy.** Every save target is checked through the registry `PathPolicy`, including
   Save As and explicit conflict overwrite. The app must explain data that cannot be carried
   forward before completing Save As.

## Alternatives considered

| Option | Why not chosen |
|---|---|
| Overwrite external changes by default | Could destroy edits made by another editor without the user's knowledge. |
| Attempt automatic three-way merging | TMDL, PBIR and unknown content need format-aware merge rules that do not yet exist. |
| Save only the known model and discard other files | Violates the lossless round-trip and unknown-file preservation requirements. |
| Put recovery files beside the project | Pollutes projects, risks publishing local state and can expose user-specific paths or data. |
| Treat a set of file replacements as atomic | General filesystems do not provide a portable multi-file transaction. |

## Consequences

- In-place save requires a stable baseline over the project tree and a clear, typed conflict
  result; the caller must not reduce it to a generic I/O error.
- Atomic replacement is per file. Partial success is reported explicitly and must not make the
  document appear clean.
- Recovery storage and UX are deferred to M8, but their location, consent and non-silent behavior
  are fixed here.
- PBIX/PBIT sources remain readable but are not in-place save targets.
