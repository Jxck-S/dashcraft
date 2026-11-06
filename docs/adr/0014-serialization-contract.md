# ADR-0014: Serialization contract and generated identifiers

- Status: Accepted
- Date: 2026-11-05
- Deciders: DashCraft maintainers
- Related: [ADR-0003](0003-vertipaq-xpress9-cleanroom.md), [ADR-0004](0004-desktop-ready-architecture.md),
  [ADR-0011](0011-desktop-ground-truth.md), [ADR-0012](0012-revisioned-document-state.md),
  [ADR-0013](0013-save-conflicts-and-recovery.md)

## Context

The TMDL, TMSL and PBIR readers preserve content they do not model, and existing writers have
round-trip tests. Editing adds new serialization obligations: unchanged bytes must remain stable,
unknown content must survive, and new objects need identifiers that are deterministic in tests
without becoming predictable in normal use. Comparing against a self-authored expected output
alone does not establish Desktop compatibility.

## Decision

1. **No-op saves.** Saving a document with no content changes emits no file changes. A full
   no-op round trip of a supported project is byte-identical; it creates no new identifiers and
   does not advance the saved revision.
2. **Unchanged content.** In an edited project, unchanged files and unchanged regions of edited
   files remain byte-stable. Unknown properties, annotations, files and format-specific content
   not modeled by the edit are preserved from the input. Writers must not silently drop them.
3. **New or changed content.** Generated TMDL content is canonical only where its form is
   verified against a Desktop-saved fixture; PBIR/TMSL content is compared as JSON values until
   that canonical evidence exists. A non-equal output is permitted only as an explicit,
   reviewed entry in a deviation list naming the format/field, reason and Desktop version.
   No-op saves and preserved-from-input content can never be deviations. The deviation list is
   capped at 20 entries for the 1.0 output contract and may only shrink between releases, except
   for reviewed changes in a new Desktop version.
4. **Compatibility.** `$schema` and compatibility-level values are selected from supported
   versions recorded by the reader/writer tests. The writer does not invent an upgrade path or
   silently upgrade a project's compatibility level. Unknown source values remain preserved
   unless a specific edit requires a validated replacement.
5. **Identifiers.** Newly generated serialized identifiers use an injectable generator.
   Tests supply a fixed sequence and assert stable output; normal application use supplies
   unpredictable identifiers. Once assigned, identifiers are preserved across edits and saves.
   Re-saving an unchanged object never regenerates its identifier.
6. **Verification.** Writer tests cover no-op byte equality, preservation, edited-region
   stability and parse-after-write equivalence on synthetic and licensed fixtures. Desktop
   canonical comparisons use only black-box outputs collected under [ADR-0011](0011-desktop-ground-truth.md).

## Alternatives considered

| Option | Why not chosen |
|---|---|
| Re-serialize every file from typed models | Reorders or normalizes unknown and untouched content, creating noisy diffs and data loss risk. |
| Treat JSON value equality as the permanent guarantee | Does not protect users' diffs or establish Desktop's canonical form for edited output. |
| Use deterministic identifiers in production | Makes generated object identifiers predictable and can collide across independently created projects. |
| Regenerate identifiers on every save | Breaks stable references and makes repeated saves produce unrelated diffs. |

## Consequences

- Writers need to retain original source material and enough spans/structure to limit edits.
- New output formats and constructs need synthetic round-trip tests; claims of canonical Desktop
  output require a Desktop-saved fixture.
- The identifier generator is a test seam and a production security boundary; callers cannot
  choose predictable IDs in normal application use.
- Deviations are temporary, auditable and bounded rather than an alternate serialization mode.
