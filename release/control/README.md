# AxiomCore platform release control

The root `justfile` exposes one operator interface: `just release`. See the
[operator guide](./OPERATIONS.md) for its short workflow and the
[production migration](./PUBLISHER_MIGRATION.md) for what is still missing.
Run `just release capabilities` for the current build/publisher status of all
catalog components; a catalog entry alone does not imply it can publish.
These controls manage AxiomCore's own tools and services, not customer
contract releases or end-user application bundles.

The source of truth is split deliberately:

- [catalog.toml](./catalog.toml) defines the 21 deliverables, owning Git
  repositories, build inputs, dependency edges, adapters, and destinations.
- [versions.json](./versions.json) is the single editable ledger of candidate
  SemVer numbers. The owning Cargo/npm/pubspec manifests are the native
  package-manager mirrors; `prepare` updates them. A null candidate for a
  service or site means its identity is a future immutable digest/deployment.
- `just release new` interactively creates the next train: arrow-key component
  selection, per-component change type and SemVer candidate where applicable,
  and template-or-empty-custom choices for component and cycle summaries. Each
  answer is checkpointed on the release SSD; rerun the same command to resume.
  `just release new restart` archives an unfinished draft before restarting.
  Final confirmation backs up previous source files on the SSD, but does not
  prepare, build, commit, push, or publish.
- [intent.json](./intent.json) groups one wave's user-visible changes and
  release-note summaries. Its `queued` list records affected dependent
  components awaiting an earlier artifact. A new wave gets a new train ID.
- `scan.py` inventories local committed and working build-input changes against
  each repository's upstream. This is **not** a production baseline. An
  authenticated signed published manifest must supply the actual baseline.
- `flow.py`, `ctl.py`, `gate.py`, and `train.py` perform backed-up preparation,
  deterministic planning, local builds, receipts, staging, and validation.
  `publisher.py` and `publish_targets.py` implement destination-specific
  publication from staged receipts; a publisher adapter is not a build receipt.

The local release root is the existing SSD-backed APFS image at
`/Volumes/AxiomReleaseBuild/axiom-release`; the operator script mounts it
only for actions that write evidence. Train evidence is grouped under
`trains/TRAIN/`, with per-component candidate evidence in
`trains/TRAIN/components/COMPONENT/`. The CLI, three UI Host targets, dashboard
proxy, landing, and docs have SSD-safe local builders. The remaining components require selective CI
builders. The old repository-specific scripts are not silently invoked by
this interface because some mutate source or overwrite release assets.
New artifacts live under `artifacts/COMPONENT/FINGERPRINT/`, so another release
never overwrites an earlier candidate. Previously staged root-level receipts
remain readable when their fingerprint matches.
Interactive local builds show elapsed time and keep full logs on the SSD;
press `l` for live logs and `Ctrl-]` to return to progress. Interrupted builds
can resume only when their saved intent and plan still match exactly.
Android signing is preflighted before a grouped Host build and, when needed,
injected only into the Android build child via Infisical's `prod` environment.

`just release publish` opens a multi-select arrow-key picker of staged targets.
Space selects, Enter reviews, and `p` confirms the production publish; `q`
cancels. Non-interactive use must specify `COMPONENT [TRAIN]`. Individual
`ui-host-web`, `ui-host-android`, and `ui-host-ios` names route to the grouped
`ui-host` candidate. Every destination adapter consumes a gated stage, checks
pinned remote source commits, and verifies the resulting remote asset, registry
package, service image, or deployment marker before recording success. GitHub
assets are downloaded and hashed; Pages sites are checked at their deployment
and production alias. No command silently publishes at build time.

The *publisher* paths are wired for all catalog targets, but most are not yet
end-to-end releasable: the CI-only entries have no selective builder in this
checkout, and no destination adapter has completed a disposable production-like
rehearsal. `just release capabilities` shows the builder gap. This work does
not claim a signed whole-train baseline or finalize changelogs. Keep the old
scripts out of this operator path until the missing builders and rehearsal
are completed.
The checked-in GitHub Actions release-control workflow currently runs tests;
it is not a production publisher. A candidate gate can only say
`candidate-ready-for-publisher`, never `published`.
