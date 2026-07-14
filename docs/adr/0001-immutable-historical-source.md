# ADR-0001: Immutable, pinned historical source boundary

- Status: Accepted
- Date: 2026-07-13

## Context

ApolloRS must distinguish historical evidence from modern reconstruction.  The
upstream Apollo-11 corpus can change, contains transcription adaptations, and
currently has two Luminary include tokens that do not exactly match filenames.
Editing source in place would destroy the ability to identify whether behavior
came from historical bytes, a compatibility repair, or ApolloRS semantics.

## Decision

Treat `historical/Apollo-11/` as a read-only input pinned by repository URL,
commit, and per-file SHA-256.  Assembly workarounds, include aliases, normalized
copies, and patches live outside the historical tree in versioned overlays.
Every derived artifact records source and overlay digests.

Changing the pinned source revision requires a provenance review and new
assembly/trace baselines.

## Alternatives considered

- **Copy selected `.agc` files into Rust crates.** Rejected because provenance
  and upstream diffing become ambiguous.
- **Edit upstream files until they assemble.** Rejected because compatibility
  changes become indistinguishable from source evidence.
- **Fetch the default branch during each build.** Rejected because builds cease
  to be reproducible and may change without review.
- **Store only hashes and fetch on demand.** Viable for distribution, but not as
  the sole development model because offline inspection and stable tooling are
  required.

## Consequences

- Tooling needs an explicit source-root input and overlay mechanism.
- The project carries or obtains a pinned historical checkout.
- Upstream corrections are adopted deliberately, not automatically.
- Reports can link every derived word/event to exact source bytes.

## Risks introduced

- A pinned revision may retain known transcription errors.
- Nested repository or submodule handling may complicate packaging.
- Large source-derived artifacts could be duplicated if overlays are careless.

## Validation

- Regenerate `artifacts/forensics/source-manifest.sha256` and run inventory
  `--check` in CI.
- Refuse reproducible assembly when the checkout is dirty or hashes differ.
- Test that overlays cannot write under the historical root.
- For a baseline update, compare source hashes, emitted words, symbols, and
  golden trace digests before accepting the change.

