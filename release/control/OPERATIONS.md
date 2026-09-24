# Release operator commands

Run platform-control commands from `/Users/yashmakan/AxiomCore/AxiomCore`.
They prepare reviewed version/note edits, plan, build, verify, and stage;
**they do not publish or deploy**. Do not
substitute `release/justfile`'s `release-all`: it bumps and publishes unrelated
packages and some legacy scripts overwrite release assets.

## Release-intent flow

1. Copy [intent.example.json](./intent.example.json) to a fresh path on the
   release SSD. List only components actually changing. Give each versioned or
   GitHub-tagged component an explicit new stable `X.Y.Z` version. Services,
   docs, and dashboard deployments have no SemVer field; their identities are
   immutable image/deployment digests. Use one host version across all host
   targets in a train.
2. Preview with `just release-prepare INTENT.json`. If the preview is correct,
   run the following with a fresh train-specific output path:

   ```sh
   just release-prepare-apply INTENT.json /Volumes/AxiomReleaseBuild/axiom-release/preparations/TRAIN.json
   ```

   This
   writes only named clean version files and new owned JSON note fragments.
   Byte-for-byte originals go under the SSD's `backups/TRAIN/`. Review every
   repository diff, run its tests, and commit each reviewed change. The command
   never commits, tags, or pushes. If any target file is dirty, it stops;
   resolve that file deliberately rather than discarding work.
3. Make a strict scoped plan against an authenticated published baseline, then
   run `just release-gate INTENT.json PLAN.json`. It checks that the committed
   note for each selected component exactly matches the intent, that planned
   versions equal the source versions, and that source SHAs have not moved.
   A new platform installation without a trusted baseline can make a
   first-build plan, but this is **not** proof of a prior published release.
4. Build each `BUILD` entry; fetch and verify exact published bytes for each
   `REUSE` entry. Local adapters exist for `cli` and the three UI Hosts.
   CI-only entries still need owning build jobs and `release-receipt` records.
   Stage with `python3.12 -B release/control/ctl.py stage --intent INTENT.json
   --plan PLAN.json --train-id TRAIN --receipt RECEIPT.json ... --out STAGE.json`
   and run `gate.py` again
   with the stage and all receipt paths. A passing stage gate is only
   `candidate-ready-for-publisher`.
5. Publish through the owning adapter **after** it has been migrated to consume
   the verified candidate. Independently verify remote bytes/digests and
   deployment revisions, then issue a signed `published` baseline. Publisher
   adapters and signed-baseline generation are **not implemented yet**; the
   current control plane must not be described as a one-command production
   release. Do not rename a staged manifest to `published`.
6. Generate train notes with `just release-notes-save PLAN.json NOTES.md` and
   each owner's draft with
   `just release-notes-owner-save OWNER PLAN.json OWNER-NOTES.md`. After real
   publication, copy the matching entries into the
   owner's changelog and archive its committed fragments. This finalization
   is still manual; the control plane prevents missing fragments but does not
   yet edit `CHANGELOG.md` or attest a registry release.

Version preparation currently edits the CLI's Cargo manifest and lock, npm
package manifests and root lock versions, Flutter pubspecs plus generator
constants and plugin podspecs, FastAPI's Poetry version, and the ATMX web
version constant. The Flutter plugin version and Apple framework pin are now
independent: set `runtimeVersion` in the `sdk-flutter` intent only when adopting
a verified new `runtime-apple` release. Updating either Flutter package also
updates the CLI's generated-project version constants in `axiom-build`; a full
plan will select `cli` as a consumer. Publish the SDK first, then the CLI that
generates its dependency, using separate scoped waves if necessary. React's
`atmx-web` dependency lock and Swift's framework
checksum require the dependency's exact published bytes first; prepare and
publish those as a subsequent dependent step, not by guessing an integrity or
checksum. The editor refuses a target it does not know how to update.

## Before any local release work

```sh
cd /Users/yashmakan/AxiomCore/AxiomCore
just release-mount
just release-preflight
just release-test
just release-plan
```

The APFS image already exists at
`/Volumes/ExternalSSD/AxiomReleaseBuild.sparsebundle`; its build root is
`/Volumes/AxiomReleaseBuild/axiom-release`. No environment export is needed.
`release-mount` is safe to repeat after a reboot; it refuses a conflicting
mount. A `BLOCK` result means the declared inputs are dirty or a versioned
component needs a new version. Commit reviewed changes in each owning repo,
then replan. Do not stash or discard someone else's changes to clear a gate.
Choose a fresh plan/notes/receipt/manifest filename for each attempt: the
control plane refuses to overwrite existing release evidence or an occupied
component artifact directory.

## Plan one changed component

With an authenticated published platform manifest:

```sh
just release-plan-component-against cli PUBLISHED_MANIFEST.json /Volumes/AxiomReleaseBuild/axiom-release/plans/cli.json
just release-gate INTENT.json /Volumes/AxiomReleaseBuild/axiom-release/plans/cli.json
just release-notes-save /Volumes/AxiomReleaseBuild/axiom-release/plans/cli.json /Volumes/AxiomReleaseBuild/axiom-release/notes/cli.md
```

Replace `cli` with the selected component ID from `catalog.toml`.
The plan includes declared dependencies, so inspect every `BUILD` or `REUSE`
entry before acting. Until a signed, verified platform baseline exists, use
`just release-plan-component-save COMPONENT PLAN_PATH` for a scoped
first-build plan; **do not fabricate a `published` baseline**. A source-only
change that does not touch a component's declared inputs should not release
that component. For a build-time configuration change invisible to Git, use
`just release-plan-force PUBLISHED_MANIFEST COMPONENT PLAN_PATH`.

Only four components have SSD-safe local builders today:

```sh
just release-build cli /Volumes/AxiomReleaseBuild/axiom-release/plans/cli.json
just release-verify /Volumes/AxiomReleaseBuild/axiom-release/artifacts/cli/receipt.json
just release-stage-intent-one INTENT.json /Volumes/AxiomReleaseBuild/axiom-release/plans/cli.json 2026.09.23.1 /Volumes/AxiomReleaseBuild/axiom-release/artifacts/cli/receipt.json /Volumes/AxiomReleaseBuild/axiom-release/staged/2026.09.23.1.json
```

`ui-host-web`, `ui-host-android`, and `ui-host-ios` also support `release-build`.
A host publication needs all three targets in the plan and verified receipts,
including fetched/reverified baseline bytes for reused targets. For a
multi-component plan, supply one verified receipt **per planned component**
to `ctl.py stage` (repeat `--receipt` for each component). CI-only products use
their owning builder, then `ctl.py receipt` to register exact output bytes on
the release volume. A staged
manifest is not a published release and cannot be the next baseline.

## Which update means which release?

| Change | Plan component(s) | Version and production action |
| --- | --- | --- |
| Acore/compiler or CLI behavior | `cli`; possibly `backend-worker` or `contract-test-runner` if their declared inputs changed | Bump `AxiomCore/cli/Cargo.toml` for a CLI release. The safe local CLI adapter only stages an archive; production CLI publication is not yet connected to this control plane. |
| Embedded runtime, extension ABI, or renderer | `runtime-apple`, affected `ui-host-*`, and SDK consumers shown by the plan | A host has its own `0.x.y` release version; update its reviewed runtime pin and validate all targets before the existing host publisher. That publisher still rebuilds all three targets. |
| Only web/Android/iOS host implementation | Corresponding `ui-host-web`, `ui-host-android`, or `ui-host-ios` for build planning | Host publication is currently one signed manifest/version across targets, so the legacy publisher still does a full host rebuild. |
| `atmx-web`, React wrapper, or `atmx-cli` | `sdk-atmx-web`, `sdk-atmx-react`, or `sdk-atmx-cli` | Bump only the changed `package.json`; if React's pinned `atmx-web` range changes, publish web first, then React. npm publication is CI-only/pending migration here. |
| Flutter generator vs runtime plugin | `sdk-flutter-generator` vs `sdk-flutter` | Bump the affected `pubspec.yaml`. The plugin's native/wasm pins must refer to verified framework/runtime bytes. Legacy `publish_sdk.sh flutter` updates and publishes both, so do not use it for a one-package release. |
| Swift package or Apple framework | `sdk-swift`, `runtime-apple` | Publish and verify the exact XCFramework archive before updating `Package.swift` URL/checksum. The legacy Apple publisher mutates source and uses `--clobber`; it is not the selective path. |
| FastAPI or Go extractor | `extractor-fastapi` or `extractor-go` | FastAPI uses its `pyproject.toml` version; Go currently uses its immutable release tag. Legacy extractor publisher mutates local build paths and uploads with `--clobber`. |
| Management API / release worker / mock runner / test runner | `backend-api`, `backend-worker`, `mock-runner`, `contract-test-runner` | No shared SemVer bump. Record immutable image digest and Cloud Run revision/job; use only the matching owner deploy recipe below. |
| Dashboard origin / edge proxy / docs | `dashboard-origin`, `dashboard-proxy`, `docs` | Identify by image/deployment digest and Git SHA. A proxy origin URL change needs `--force dashboard-proxy` even without source edits. Local frontend/docs builds are not yet SSD-routed by this control plane. |
| Customer Acore contract or application | Not a platform-train component | Release a new immutable contract version or target `.axiomapp` using the application workflow; do not bump CLI/host merely because app source changed. |

The backend has targeted remote-build/deploy recipes. After reviewing the plan,
tests, migrations, and required Infisical/GCP configuration, run **only** the
matching command from `/Users/yashmakan/AxiomCore/axiom-backend`:

```sh
just deploy-update
just deploy-release-worker-update
just deploy-mock-runner-update
just deploy-contract-test-runner-update
```

Those commands are alternatives, **not a sequence to run every time**. Use
`AXIOM_GCP_SYNC_SECRETS=true just deploy-update` only when the production API
secret mapping needs refreshing. They build on Cloud Build rather than placing
large image layers on the local disk. The `axiom-frontend` origin/proxy and
docs have owner deploy recipes, but those currently generate local output in
their source worktrees. Use CI or migrate them to an SSD-backed build workspace
before local production deployment; do not claim the new control plane makes
their existing recipes SSD-safe.

For clarity, these are the current owner publication recipes, **not** extra
steps in every train:

| Product | Owner command | Local safety boundary |
| --- | --- | --- |
| UI Host (new host version) | `cd /Users/yashmakan/AxiomCore/axiom-ui-host && just release-update NEW_VERSION` | Publishes one signed manifest and rebuilds web, Android, and iOS. Its legacy build/cache paths are not automatically redirected by `release-mount`; run only after an SSD-safe publisher migration or in a reviewed CI job. `release-dry-run` also builds all targets. |
| Dashboard origin | `cd /Users/yashmakan/AxiomCore/axiom-frontend && just deploy-update` | Rebuilds assets in the source checkout before the remote Cloud Build; CI or SSD-backed workspace only for now. |
| Dashboard proxy | `cd /Users/yashmakan/AxiomCore/axiom-frontend && just deploy-remote-update` | Also runs `build-dashboard` locally; CI or SSD-backed workspace only for now. Deploy origin first if its URL changed. |
| Documentation site | `cd /Users/yashmakan/AxiomCore/AxiomCore/docs && just deploy` | Builds local Next/Pages output in the checkout; CI or SSD-backed workspace only for now. |
| CLI, npm/pub SDKs, Apple framework, extractors | No safe selective publish command in the new control plane yet | Do not substitute `release/justfile`'s `release-all`, `release-atmx`, or combined Apple/Flutter publishers for a single-component change. Migrate each owning publisher to consume verified receipts first. |

For a customer contract, this platform train is not the publisher: from the
application's backend directory, use
`axiom build axiom.acore --release --version NEW_CONTRACT_VERSION` after the
contract's own diff, tests, and Cloud signing checks. A changed `.axiomapp`
similarly follows its application
release workflow, not a platform host version bump.

## Version and changelog ledger

- Component versions are independent. The train ID groups evidence; it does
  not force a common number. The planner blocks a selected versioned component
  whose version still equals the published baseline. Make the reviewed version
  bump and release-note fragment in the owning repo, commit both, then create
  the strict plan. The planner reads versions; it does not edit them.
- CLI: `AxiomCore/cli/Cargo.toml`; npm: each package's `package.json`;
  Flutter: each `pubspec.yaml`; FastAPI: `pyproject.toml`. Host: signed host
  manifest and immutable `vX.Y.Z` release. Services: image digest/revision.
  Contracts: Cloud project/version and consumer lock. Docs: Git SHA/deployment.
- The CLI's minimum host protocol in `cli/src/commands/ui.rs` is a compatibility
  floor, **not** the CLI or host release version. Change it only with a reviewed
  delivery/protocol requirement and test that older hosts fail clearly.
- Before planning, add a committed JSON fragment under the owning repository's
  `release-notes/unreleased/`, following `change-fragment.example.json`. The
  `component` must match the catalog ID; `breaking` requires `migration`.
- `just release-notes-save <plan> <ssd-path>` creates the cross-repo train
  rollup and fails if a selected component lacks its committed fragment. After
  actual publication, append the matching entries to each owner's changelog
  (`CHANGELOG.md` if present; create it on that owner's first release)
  and **move**, do not delete, those fragments into an owner-managed released
  archive. That append/archive step is not yet automated; leaving fragments in
  `unreleased/` would repeat them in the next train.
- Retain the plan, notes, receipts, signatures, exact published URLs/digests,
  and rollback references. Do not mark a staged manifest `published` manually;
  the signed publisher/baseline bootstrap is a separate migration milestone.
