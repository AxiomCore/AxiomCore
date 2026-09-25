# AxiomCore platform release control

## Local release dashboard

Run `just release web` and open the printed `http://127.0.0.1:8716/` address.
The launcher injects Infisical `prod` using the checked-in project ID from
`docs/.infisical.json`; do not print or export secrets manually. Authenticate
the Infisical CLI before starting the dashboard. Flutter package builds and
pub.dev publication use `fvm flutter` / `fvm dart`.
The Python server binds only to loopback; its browser UI is in
[`web/index.html`](./web/index.html). Select components with a selective builder,
enter the train summary, each component's change type/note/version, and review
the generated sequence. Existing uncommitted files can be inspected and
committed explicitly from the workspace panel. The production confirmation is
`PUBLISH <train-id>`; it authorizes the dashboard to create the next train,
prepare version mirrors and owned changelog fragments, commit only those
managed paths, push the required tracked branches without force, build each
candidate in dependency order, and publish each only after **all** builds pass.
Selected-only rollout ordering places the backend API, managed runner and
contract executor before the release worker, and the dashboard origin before
its edge proxy, without forcing an unchanged component into the train.
Each publisher still verifies remote bytes. A failed step stops the run;
there is no automatic rollback of an already published component.
When Swift or React needs newly published runtime or `atmx-web` bytes, one
dashboard confirmation runs two internal phases. It builds and remotely
verifies the producers first; only then does it update the exact Swift
XCFramework checksum and React npm lockfile, commit and push those pins, and
create a separate immutable SDK train automatically. The SDKs are then built
from those committed bytes and remotely verified. A producer failure stops
before changing consumer pins. The preview shows both phases and the extra
source commits before authorization. Nothing invents a future checksum or
lockfile resolution, and an interrupted phase remains checkpointed for review
or explicit resume.

For versioned components, the form suggests the existing candidate unless a
locally recorded, remotely verified release already owns it; in that case it
suggests the next patch version. Editing the version updates its local safety
message immediately. A stale staged candidate means its saved artifact no
longer matches current source or version; it is preserved, not relabeled, and
a new candidate build is required. Remote tag/package checks still happen at
the publisher gate.

The run and log are checkpointed under the external release root's `ui-runs/`
directory. Reopen the dashboard to inspect or explicitly resume a stopped
run. A resumed run never rewrites completed release evidence; if a partial
step cannot be verified, it fails closed for manual inspection. The dashboard
does not create or format the external SSD image.

Former CI-only targets now have selective candidate builders: package and
framework builds use tracked Git sources staged on the release SSD; container
targets submit only staged sources to Cloud Build under immutable candidate
tags. The build writes the same source-bound receipt as the existing local
adapters. Production deployment and stable tags remain exclusively in the
publisher step. Required platform tools, cloud credentials and provisioned
destinations still need to be available; a missing prerequisite stops the run.

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
- `just release update` changes the active selection without editing JSON by
  hand. It preselects unfinished active components, retains their answers,
  and lets you add queued components with arrows and Space. Published components
  are deselected by default. To preserve prepared and published evidence, it
  automatically creates a successor train; it refuses to strand an interrupted
  or staged-but-unpublished candidate. An interrupted update resumes from its
  own SSD draft; `just release update restart` archives that draft.
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

The *publisher* paths are wired for all catalog targets, but no destination
adapter has completed a disposable production-like rehearsal. The selective
builders have unit-tested receipt and isolation behavior; they are not a
claim that every platform tool or cloud destination has been exercised on this
machine. This work does not claim a signed whole-train baseline or finalize
changelogs. Keep the old mutating deploy scripts out of this operator path.
The checked-in GitHub Actions release-control workflow currently runs tests;
it is not a production publisher. A candidate gate can only say
`candidate-ready-for-publisher`, never `published`.
