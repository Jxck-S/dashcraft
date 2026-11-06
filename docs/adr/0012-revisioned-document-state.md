# ADR-0012: Revisioned document state and edit transactions

- Status: Accepted
- Date: 2026-11-05
- Deciders: DashCraft maintainers
- Related: [ADR-0004](0004-desktop-ready-architecture.md), [ADR-0006](0006-interaction-state.md),
  [ADR-0007](0007-command-registry.md), [ADR-0011](0011-desktop-ground-truth.md)

## Context

`pbi-model` already represents edits as commands with inverses and change events, and its
`History` supports undo, redo and save points. The open-document path has not yet made this the
authoritative state: `OpenedDocument`, `Session`, the model used by the engine and registry-side
`DocEntry` state can each retain a copy or derivative. Editing one copy without rebuilding the
others would let readers, queries and writers observe different models.

## Decision

1. **One authoritative revision.** Each open document has one current revision containing its
   semantic `Document` and the format-specific writer sources needed to save it (including the
   lossless TMDL source and typed PBIR inputs). The opened container and its original file
   fingerprints remain attached as the read/save baseline. `Session` models and engine snapshots
   are derived state, never independent authorities for semantic edits.
2. **Atomic publication.** A transaction applies commands to a private working copy of the
   current revision. Validation or application failure discards that copy and leaves the current
   revision, history, dirty state and published change events untouched. A successful changed
   transaction publishes one new revision and one history entry; grouped commands are one undo
   step. Callers never observe a partially applied group.
3. **Revision and history identity.** The document revision is a monotonically increasing
   `u64`, advanced only when the committed semantic or writer state changes. Undo and redo
   publish new revisions; they do not rewind the revision number. No-op edits return
   `changed: false` and do not advance it. Dirty state compares the current content identity
   with the last successfully saved content identity, so undoing back to the saved content clears
   dirty state even though the revision number is newer. History is session-local and is not
   persisted across reopen.
4. **Query consistency.** A query captures one revision and the corresponding derived engine
   snapshot before it runs, and its result identifies that revision. A later edit cannot change
   the inputs of an in-flight query. Document revisions and data generations are separate:
   refreshes, CSV loads and derived data changes advance the data generation but do not by
   themselves mark the semantic document dirty.
5. **Serialization and events.** Existing per-document operation ordering remains the
   serialization boundary for mutations. Each committed result carries its revision, dirty
   state and `ChangeEvent`s. Failed and no-op transactions publish no change notification.
   `serve` and MCP notify clients only after a committed transaction; the app receives the same
   committed events through its registry adapter.
6. **Agent safety.** In-memory document edits and history navigation may be `agent_safe` when
   exposed through typed registry commands. Save commands may be `agent_safe` only when every
   output passes the caller's `PathPolicy`. Reload, recovery and any command that bypasses those
   checks are not agent-safe. Command metadata and validation remain the authority for access.

## Alternatives considered

| Option | Why not chosen |
|---|---|
| Mutate the existing `Session` and separately update the writer model | Creates multiple authorities and permits queries or saves to see stale state. |
| Apply compound edits directly and undo on error | Intermediate state can leak to concurrent readers and rollback itself can fail. |
| Use the history cursor as the public revision | Undo would make revisions go backwards, obscuring which state an in-flight query observed. |
| Persist undo history in the project | Adds private, non-format project content and makes recovery/versioning part of the interchange format. |

## Consequences

- The implementation must define a revision value that owns both the semantic document and
  writer inputs, and derive or invalidate session/engine state from committed revisions.
- Editing transactions need a working copy or an equivalent rollback-safe staging mechanism;
  applying inverses after a partial failure is not sufficient.
- Reads must capture a revision consistently, and all mutation entry points must use the
  per-document serialization boundary.
- History is cleared on close/reopen. Recovery is a separate, explicit feature covered by
  [ADR-0013](0013-save-conflicts-and-recovery.md).
