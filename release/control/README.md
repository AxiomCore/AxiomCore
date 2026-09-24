# AxiomCore platform release control

This is the operator entry point for **AxiomCore's own tools and services**. It
does not replace the customer-contract release control plane in
`axiom-backend/docs/release-control-plane.md`. It never publishes, tags, deploys,
or mutates a source repository by itself.

For commands by change type, use the [release operator runbook](./OPERATIONS.md).
The remaining production publisher and signed-baseline cutover is specified in
[publisher migration](./PUBLISHER_MIGRATION.md).
The new `release/control/flow.py` is an explicit exception to the read-only
default: `just release-prepare-apply` edits only the version files named in its
preview and creates owned release-note fragments. It requires a clean target
file, saves originals on the external build volume, and never commits or
publishes. `release/control/gate.py` checks the intent, committed source,
notes, staged manifest, and verified receipts before a publisher handoff.

From the `AxiomCore` repository root, run `just release-test`,
`just release-versions`, and `just release-prepare`. The checked-in
`intent.json` names the active train and changed components; `versions.json`
owns candidate versions for all 20 catalog components. The package-native
Cargo/npm/pubspec versions are read back as mirrors, and published versions
remain unknown until an authenticated baseline is imported. For the current
CLI-only draft train, `just release-train-plan` writes a scoped first-build
plan to the mounted SSD and `just release-train-status` shows its evidence
folder. The catalog in `catalog.toml` names each deliverable's
committed source inputs, dependency edges, owner, and publication destination.
`ctl.py` hashes the selected Git tree objects—not an entire repository HEAD—so
a docs-only commit does not rebuild a CLI binary. A dependency fingerprint is
part of its consumer's fingerprint. Missing or dirty inputs block building.
The justfile defaults to Python 3.12 (`AXIOM_RELEASE_PYTHON` can select any
Python 3.11+ interpreter); the macOS system Python 3.9 is too old for
`tomllib`.

## What gets built?

| Change | Deliverables | Not implied |
| --- | --- | --- |
| Acore/compiler, shared CLI crates | CLI and affected compiler tests | UI Hosts if their embedded ABI/renderer inputs are unchanged |
| Runtime/extension ABI | Affected hosts, Apple framework, dependent SDKs and compatibility tests | Management API and docs |
| UI Host web, Android, or iOS implementation | Affected target host archive/APK/app | Every Acore application bundle |
| Acore application UI/extension | Target `.axiomapp` and extension artifacts for that application | New UI Host or platform runtime |
| Application backend contract | New immutable, Cloud-signed `.axiom`; consumers with changed locks | AxiomCore platform release |
| SDK source or pinned distribution | Changed package plus dependent package if its pin changes; Flutter generator and runtime plugin are independent components | Unrelated SDKs |
| Go/Python extractor or `atmx-cli` source | That extractor or npm CLI package and its platform archives | The other extractor, browser SDK, and main CLI unless their own inputs change |
| Management API, worker, mock/test runner | The affected Cloud Run service/job image | Other services merely because they share a repository |
| Dashboard or docs | Corresponding origin/proxy or docs deployment | CLI and native artifacts |

The host Android APK is a signed **development host**, not an end-user app APK.
The iOS simulator host, standalone `AxiomRuntime.xcframework`, browser WASM
inside the web host, and target-specific `.axiomapp` bundles are different
deliverables. App-store packaging is not part of this control plane.
The `axiom-contracts` repository and customer application contracts are
released per contract, not as one platform-train component; their own signed
artifacts and locks determine whether an application consumer needs a rebuild.

## Release cycle

1. Commit and review each changed source repository. The planner previews
   dirty declared inputs, and `build` refuses them; unrelated dirty sibling
   checkouts do not block a scoped plan. Pin every source SHA and
   toolchain/engine lock used by the candidate.
2. Authenticate the last published manifest through its owning release
   channel, then compare against it:
   `python3 release/control/ctl.py plan --baseline <manifest> --out <plan>`.
   Without a baseline, every component is a first-build candidate. Use
   `--only cli` (or another component ID) for a scoped plan; dependencies are
   included automatically. CI should add `--strict` (or use
   `just release-plan-against-strict <manifest> <plan>`) so dirty declared
   inputs fail the planning job. When only external build-time configuration
   or a required rebuild changed, use `--force <component>`; force propagates
   to its in-scope dependents. The dashboard proxy is one example: its generated
   worker embeds `AXIOM_DASHBOARD_ORIGIN`, which Git source hashes cannot see.
3. Review `BUILD` and `REUSE` lines. `ctl.py` checks the baseline's format and
   published status but does not cryptographically authenticate its origin.
   A matching fingerprint is a *reuse candidate*, not proof of the remote
   bytes. Fetch the immutable artifact and verify its digest/signature before
   promotion. Missing receipts force a build.
4. In CI, run selected jobs with the plan's pinned SHAs. Locally, only the
   `cli`, `ui-host-web`, `ui-host-android`, and `ui-host-ios` adapters are
   available; other components remain with their owning CI/deploy scripts.
   `build` produces a checksum receipt, never an upload. Run
   `just release-verify <receipt>` before handing it to a publisher.
   Host adapters also require the local Lynx renderer checkout to match its
   lock exactly and have no uncommitted files.
   CI-only builders can register their output with `ctl.py receipt --plan
   <plan> --component <id> --artifact <file> --out <receipt>`; local artifacts
   and receipts must be under the external build root.
5. Use `ctl.py stage --plan <plan> --train-id <id> --receipt <receipt> ...
   --out <manifest>` to require and verify exactly one receipt per planned
   component. Known host targets require their exact archive/APK name, so a
   receipt for the wrong target cannot complete a release candidate. The
   resulting manifest says `staged-not-published`; it cannot be used as a
   published baseline or mistaken for an activated release.
6. Publish immutable assets through the owning repository, npm/pub registry,
   Artifact Registry, or Cloudflare. Record the actual URL/digest and verify
   it before changing a channel, Homebrew formula, Swift checksum, Cloud Run
   revision, or `latest` pointer. A failed/partial target must never be called
   a successful release.
7. Retain the plan, verified receipts, signatures, SBOMs, compatibility test
   results, and final cross-repository bill of materials for the release train.
   A train ID coordinates independent component versions; it does not force
   every package to share a version or publish again.

`just release-notes <plan>` requires a committed fragment for every selected
component. Place a JSON file under the owning repository's
`release-notes/unreleased/` directory, using `change-fragment.example.json` as
the shape. The release train notes group fragments by component and owner.
Publication should append the same fragments to that repository's changelog
and then archive the fragments; this is **not yet automated** by `ctl.py`.

## Local SSD rule

The local default is `/Volumes/AxiomReleaseBuild/axiom-release`, inside the
`/Volumes/ExternalSSD/AxiomReleaseBuild.sparsebundle` APFS image. The image is
already provisioned on the ExFAT SSD; it does not reformat that disk and grows
only as build data is written. No shell-wide `AXIOM_RELEASE_BUILD_ROOT` export
is needed. After reconnecting the SSD or rebooting, run `just release-mount`
and `just release-preflight` from the `AxiomCore` repository. The mount recipe
only attaches the existing image; it never creates or overwrites one. The
preflight checks the APFS mount and its exact SSD-backed image identity.

An explicitly set `AXIOM_RELEASE_BUILD_ROOT` remains an advanced override; it
must point to an existing, writable external APFS volume. The local gate
refuses ExFAT and internal storage before a build starts.
The local adapters route Cargo targets/cache, Gradle cache, npm/pnpm, pip,
Habitat and XDG caches, host staging/output, temporary build files, and release
archives beneath this root. GitHub Actions CI may use its runner temporary
directory; a local shell setting `CI=true` does not bypass the SSD gate. A
missing local SSD never falls back to the nearly full internal disk. The
pre-existing Android SDK, Xcode, Rust toolchain, and pinned renderer source
remain where the developer installed them; they are inputs, not new release
outputs.

If the image is missing or the mount point belongs to something else, stop and
inspect the disk; do not recreate the image or bypass a failed preflight.

Signing secrets are **not build artifacts**. The Android adapter sets a
separate protected signing-temporary location on the internal system volume;
the host script removes its decoded keystore on exit. Do not put signing keys,
Infisical exports, or `.env` files on the portable SSD.

Some older recipes (`release/justfile`, `release/scripts/`, and the combined
`deploy-prod` recipes) still publish several products or write into repository
`target`/`dist` directories. They are compatibility paths, **not SSD-safe
local release entry points**. They must be migrated and verified one by one
before being retired; do not call `release-all` as a substitute for this plan.

## Compatibility and version rules

- CLI, host, runtime distribution, SDKs, services, and customer contracts have
  independent versions. Bump a component only when its published input or
  compatibility contract changes. When a versioned component is selected but
  still has the version recorded in the published baseline, the planner blocks
  the release instead of overwriting that version's artifact.
- A host release records its embedded runtime commit, renderer commit, target
  assets, digests, and signature. The CLI's minimum host protocol and the host
  manifest must agree before promotion.
- A runtime/extension ABI change requires guest compatibility tests and all
  affected host targets. A UI-only edit does not require a runtime version bump.
- A Swift package checksum or Flutter/native framework pin changes only after
  the exact framework archive is published and verified. React's `atmx-web`
  pin changes only after that package's exact version is available.
- Cloud Run deployments select immutable image digests. A mutable `managed` or
  `latest` tag is a channel pointer, not artifact identity.

## Current migration boundary

This implementation provides a deterministic planner, reviewed version/note
preparation with backups, SSD-gated local builders for four components, artifact
receipts, release-note validation, a cross-component candidate gate, and tests.
It deliberately does **not** automate production publication or claim that
existing CI is already selective. A passing gate says only that the candidate
is ready for an owning publisher; it does not create a trusted baseline.
Next, adapt each owning CI workflow to consume the plan, publish/reuse assets
by verified digest, and produce a signed published release manifest. Until
then, use the existing per-repository publishers only after manual verification
and review.
