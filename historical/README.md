# Historical source boundary

`Apollo-11/` is an upstream checkout of
`https://github.com/chrislgarry/Apollo-11.git` at commit
`247dd7d0d1b0e7f9f270750ec08983e0a72e73e1` on the upstream `master` branch.

The checkout is evidence, not an ApolloRS implementation directory.  ApolloRS
tools may read it, hash it, assemble it in a separate build directory, and
attach provenance to derived artifacts.  They must not edit, format, rename, or
silently repair its `.agc` files.

Any compatibility alias, source normalization, or assembler workaround must be
stored outside this directory and must retain both the original token and the
derived resolution.  This rule is already relevant to two unresolved textual
includes in `Luminary099/MAIN.agc`; see the repository-forensics report.

Before using a different upstream revision, create a provenance review that
records the old and new commits, file-level hash changes, inventory changes,
assembly-output changes, and effects on existing traces.

