# Clean-machine qualification

ApolloRS has one local qualification entry point:

```sh
sh tools/qualify.sh
```

It performs, in order:

1. formatting, workspace Clippy with warnings denied, all-target tests, and a
   release build;
2. byte-for-byte historical source verification;
3. generation and local execution of the complete Block II conformance rope;
4. the 300,000-instruction Luminary P63 mission run and streaming coverage;
5. the declared 11-arm paired fault matrix;
6. schema/provenance validation of every new report.

By default, outputs go to a fresh `mktemp` directory and existing tracked
artifacts are untouched. Set `APOLLORS_QUALIFY_OUTPUT` to require a specific
new output directory.

To include the independently built yaAGC comparison:

```sh
YAAGC=$(sh tools/build-yaagc-conformance.sh /tmp/apollors-virtualagc)
APOLLORS_YAAGC="$YAAGC" sh tools/qualify.sh
```

The build helper refuses to overwrite an existing checkout, checks out the
pinned VirtualAGC revision, applies only the dedicated conformance logger, and
prints the executable path after a successful build.

## Continuous integration

`.github/workflows/qualification.yml` runs two clean Ubuntu jobs:

- `rust-and-mission` runs the complete local qualification with the pinned
  Rust 1.97.0 toolchain and recursive historical submodule;
- `independent-yaagc` clones and compiles pinned VirtualAGC separately, then
  requires the 252-event exact conformance match.

Both jobs retain generated reports and traces as CI artifacts, including on
failure. The workflow uses official `actions/checkout@v6` and
`actions/upload-artifact@v7` releases and grants only `contents: read`.

## Inputs expected on a new machine

- Git with submodule and network access;
- `rustup` capable of installing Rust 1.97.0, Clippy, and rustfmt;
- a C compiler and `make` only for the optional external yaAGC job.

No Python environment, database, service account, or pre-existing VirtualAGC
checkout is required.
