# AxiomCore platform release control

The root `justfile` exposes one operator interface: `just release`. See the
[operator guide](./OPERATIONS.md) for its short workflow and the
[production migration](./PUBLISHER_MIGRATION.md) for what is still missing.
These controls manage AxiomCore's own tools and services, not customer
contract releases or end-user application bundles.

The source of truth is split deliberately:

- [catalog.toml](./catalog.toml) defines the 20 deliverables, owning Git
  repositories, build inputs, dependency edges, adapters, and destinations.
- [versions.json](./versions.json) is the single editable ledger of candidate
  SemVer numbers. The owning Cargo/npm/pubspec manifests are the native
  package-manager mirrors; `prepare` updates them. A null candidate for a
  service or site means its identity is a future immutable digest/deployment.
- [intent.json](./intent.json) groups one wave's user-visible changes and
  release-note summaries. Its `queued` list records affected dependent
  components awaiting an earlier artifact. A new wave gets a new train ID.
- `scan.py` inventories local committed and working build-input changes against
  each repository's upstream. This is **not** a production baseline. An
  authenticated signed published manifest must supply the actual baseline.
- `flow.py`, `ctl.py`, `gate.py`, and `train.py` perform backed-up preparation,
  deterministic planning, local builds, receipts, staging, and validation.
  They do not publish.

The local release root is the existing SSD-backed APFS image at
`/Volumes/AxiomReleaseBuild/axiom-release`; the operator script mounts it
only for actions that write evidence. Train evidence is grouped under
`trains/TRAIN/`, with per-component candidate evidence in
`trains/TRAIN/components/COMPONENT/`. The CLI and three UI Host targets have
SSD-safe local builders. The remaining components require selective CI
builders. The old repository-specific scripts are not silently invoked by
this interface because some mutate source or overwrite release assets.

`just release publish COMPONENT` deliberately stops until a publisher can
consume a verified receipt and prove the remote bytes/deployment match.
The checked-in GitHub Actions release-control workflow currently runs tests;
it is not a production publisher. A candidate gate can only say
`candidate-ready-for-publisher`, never `published`.
