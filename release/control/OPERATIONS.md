# AxiomCore release operator guide

For the guided local workflow, start `just release web` and use the loopback
dashboard. It previews the next train, exact managed source edits, source
pushes, and build/publish order before accepting a typed production
confirmation. It stops before expensive builds when a required repository is
dirty, divergent, or lacks an upstream. SSD and Cloud Build candidate builders
run sequentially and produce the same pinned receipts. The
terminal commands below remain available for inspection and recovery.

Run every operator command from `/Users/yashmakan/AxiomCore/AxiomCore`:

```sh
just release
```

That is the dashboard. It scans each catalog component's build inputs against
the local upstream branches, shows the active and queued components, candidate
versions, SSD evidence location, and the next safe action. Local upstream is
**not** an authenticated published baseline, so this inventory is a draft
selection, not proof that a component needs production publication.

## Current train

The checked-in [intent.json](./intent.json) is train `2026.09.24.2`. Its
foundation wave contains the Apple runtime, three UI Hosts, `atmx-web`, the
backend worker, mock runner, contract-test runner, and docs. The affected
consumers are queued: `atmx-react` (after web npm bytes), Flutter and Swift
(after the Apple framework bytes/checksum), and the CLI (after its generated
Flutter SDK pins are settled). The new landing site and a dashboard-origin
audit are also queued because the landing toolchain changed the shared pnpm
lockfile. The CLI's `0.147.0` manifest/lock edit and its
release-note fragment were already present in the worktree when this wave was
organized; they are preserved, not published. Train `2026.09.24.1` already has
CLI-only preparation evidence on the SSD, so the expanded foundation wave uses
a fresh train ID rather than overwriting it. The intent's top-level `summary`
describes the release-control-plane theme; each component's `summary` becomes
its own changelog fragment when that component is activated and prepared.

Candidate versions for **all 21 components** live in
[versions.json](./versions.json). Cargo, npm, and pubspec manifests remain
package-manager-facing mirrors. The ledger is not a published-version record:
the latter must come from a signed, remotely verified baseline. Services,
jobs, and sites use immutable image/deployment digests rather than SemVer.

## One interface, three routine actions

1. Run `just release` to see affected components and the next action. Start a
   new cycle with `just release new`. Use arrows and Space to select components;
   UI Host web/Android/iOS stay together, and declared dependencies are added
   before confirmation. The wizard shows the current source version, the next
   candidate version, and the last locally recorded remotely verified release
   (or **unknown**—not proof that no release exists). It asks for each
   component's change type and next SemVer where applicable. For each component
   and for the overall release summary, choose a version-aware template or
   write a custom summary from an empty prompt; breaking changes also require
   migration guidance. It allocates the next unused train ID, manages the wave
   name internally, and checkpoints selections and each answer on the release
   SSD immediately.
   Re-run `just release new` after interruption to resume at the next unanswered
   field. `just release new restart` archives the unfinished draft (after
   confirmation) before starting a different selection. Nothing is applied to
   source files until a final `yes`. The previous intent and version ledger
   are backed up on the release SSD; unfinished unselected changes stay queued.
   Candidate versions sharing one GitHub Releases destination are validated at
   each version prompt; a collision suggests the next patch version rather
   than discarding the answers already entered.
   The intent is still reviewable source of truth, but routine releases no
   longer require hand-editing it.

   To activate a queued component in an existing prepared cycle, run
   `just release update`. The picker starts with the current unfinished active
   components selected; add or remove components with Space, or press `c` to
   clear the selection before choosing just one component. Already-published
   components start deselected. Existing type, summary, migration guidance and
   candidate version for retained active components are carried forward. The
   command allocates a successor train ID internally rather than changing
   preparation or publication evidence in place. It stops if the current train
   has an interrupted or staged-but-unpublished candidate: finish or inspect
   that candidate before changing the active selection. Update answers are
   checkpointed separately from `new`; rerun `just release update` to resume.
   After confirmation, continue with `just release prepare`.
2. If a SemVer candidate needs correcting outside the wizard, run
   `just release version COMPONENT X.Y.Z`. Use `ui-host` as `COMPONENT` to set
   web, Android, and iOS together. This edits only the ledger. Review its diff.
3. Run `just release prepare`. It applies the active wave's version mirrors
   and owned `release-notes/unreleased/TRAIN-COMPONENT.json` fragments, backing
   up originals on the release SSD. In an interactive terminal it opens the
   worktree review screen. Use ↑/↓ and Enter to choose a repository, Space to
   select files, `d` to inspect a diff, `m` to enter a commit message, and `c`
   to confirm a commit of only those files. Return to the list and repeat;
   clean repositories disappear. `just release review` reopens the same screen
   without preparing again. Nothing is staged or committed by default.

   Once every declared repository is clean, the screen asks whether to run a
   safe candidate preflight. If it was already clean, preflight runs directly.
   Preflight verifies preparation evidence and the committed source plan, then
   identifies SSD and Cloud Build candidates and any prerequisite gaps. It does not build, push,
   publish, or deploy. From a noninteractive shell, `prepare` prints a short
   dirty-repository summary and asks you to run `just release review` in a
   terminal. After preflight, run each owner repo's tests and use
   `just release COMPONENT` for an individual component, or use the dashboard
   for an ordered multi-component run.

`prepare` mounts the **existing** SSD-backed APFS image if needed. It does not
create or format a disk. The normal path is derived automatically from the
train ID: `/Volumes/AxiomReleaseBuild/axiom-release/trains/TRAIN/`. No command
argument or shell-wide `AXIOM_RELEASE_BUILD_ROOT` export is needed. An
advanced override must still pass the external-volume safety checks.

The candidate command plans exact committed source SHAs, checks committed
notes, builds supported targets into the SSD, verifies receipts, stages a
candidate, and records the gate and notes under
`trains/TRAIN/components/COMPONENT/`. `just release ui-host` builds the three
host targets together; `just release cli` works only when CLI is in the active
wave. A local build shows a rotating progress indicator and elapsed time.
Press `l` to attach to the live build terminal and inspect logs; press
`Ctrl-]` to return to progress. `Ctrl-C` stops the build and reports the
full log path on the release SSD.
For the Android release APK, the control plane checks access to the four
signing variables before starting the host builds. It invokes the Android
build through `infisical run --env=prod` when they are not already all present
in the environment, matching the UI Host's own release recipe. Secret values
are not printed or copied to release evidence.

An interrupted candidate can be retried only with the exact same intent and
source plan. Verified receipts from completed targets are reused; attempt logs
are retained. A staged candidate or a nonempty artifact directory without a
receipt is never overwritten automatically. Dirty inputs, dependencies outside
the active wave, and missing builder prerequisites also stop candidate creation.
`just release test` runs the control-plane tests; `just release help` prints the
command summary.

The current `.2` preparation evidence was recorded before the landing and
dashboard-origin queue entries were added. The control plane authenticates
that historical intent in Git and permits only unchanged existing entries plus
new queued components. It does not rewrite `.2` evidence or let active changes
slip in. The staged `.2` Host can be published from its saved candidate with
`just release publish ui-host 2026.09.24.2` after required source commits are
pushed. To build landing, begin a fresh train ID, move `landing` from `queued`
to `changes`, review the dashboard-origin lockfile impact, then prepare and
build the new train.

## Releasing one specific component later

For a CLI fix after SDK pins are settled, make a fresh intent with `cli` in
`changes` and an accurate one-line summary, run
`just release version cli 0.147.1` (choose the actual next verified version),
then `just release prepare`. Review and commit the Cargo manifest, lockfile,
and generated note. Run `just release cli` after its source tests.
That command produces a verified local archive; CLI publication remains gated
until the full platform asset matrix and Homebrew update can be verified.

For a UI Host change, select all three host targets in the intent and set one
shared candidate with `just release version ui-host X.Y.Z`. Build the group
with `just release ui-host` after source commits. The Android APK is
a development host, not an end-user application APK. For `backend-api`, select
the component and add its note, but do not invent a SemVer: its verified Cloud
Run image digest and revision become the release identity after its CI adapter
is implemented.

The `summary` in each intent change becomes the owned changelog fragment.
Candidate notes are rendered from committed fragments. **Only after actual
remote publication and verification** should a finalizer append those entries
to each owner's `CHANGELOG.md` and archive the fragments. That finalizer is
not implemented yet; do not move or delete unreleased fragments manually to
make a candidate look complete.

## Production boundary

`just release publish` opens a staged-candidate selector (arrows, Space,
Enter, then `p` to confirm). `just release publish COMPONENT [TRAIN]` is the
non-interactive form. Both consume staged receipts and check pinned source SHAs
on tracked remotes. The destination adapters cover GitHub releases, npm/R2,
pub.dev, Swift tags, Artifact Registry/Cloud Run, and Cloudflare Pages. They
do not call source-mutating legacy deploy scripts. The landing Pages
project is `axiom-landing`; it is created on the first publish, and the
deployment is verified at its immutable URL and `axiom-landing.pages.dev`.
That does **not** move `axiomcore.dev` from its current origin. Coordinate DNS
and the existing `/join` route separately after reviewing the Pages URL; the
control plane never changes a custom domain automatically.

Targets without a complete gated candidate still **fail closed**. Selective
builders now exist for the former CI-only targets; missing toolchains, cloud
credentials or an unprovisioned destination still stop the run. No live
production publish was run during the builder implementation; registry
credentials and disposable rehearsal are
still required. Production also needs a trusted signed baseline, partial-release
retries, and changelog finalization. See
[PUBLISHER_MIGRATION.md](./PUBLISHER_MIGRATION.md). Never rename a staged
candidate to `published` or treat a local installed host version as the
published baseline.

## Exact receipt contract for CI-owned targets

`just release capabilities` labels these targets `selective`. `just release
COMPONENT` stages committed source under the external release root, builds its
immutable artifact and writes the standard receipt. GCP targets use Cloud
Build with a source-head substitution and candidate-only image tag; the
publisher independently verifies the build ID and registry digest before
rolling a service/job or moving a managed tag. A receipt from an unrelated
plan or changed bytes is rejected.

| Target | Exact artifact expected by its publisher |
| --- | --- |
| `runtime-apple` | `AxiomRuntime.xcframework.zip` |
| `sdk-atmx-web` | one npm `.tgz` plus `atmx.umd.js`, `atmx.es.js`, `axiom_runtime.wasm` matching its package |
| `sdk-atmx-react`, `sdk-atmx-cli` | one npm `.tgz` with the staged package name and version |
| `sdk-flutter-generator`, `sdk-flutter` | `<pub-package>-<version>.tar.gz` containing the exact publishable package tree |
| `sdk-swift` | reviewed `Package.swift` with the published Apple runtime URL/checksum |
| `extractor-fastapi`, `extractor-go` | one or more platform binaries prefixed `axiom-fastapi-` or `axiom-go-extractor-` |
| `backend-api`, `backend-worker`, `mock-runner`, `contract-test-runner`, `dashboard-origin` | `image-ref.json` containing `image` as an Artifact Registry `@sha256:` URI, `sourceHeads` equal to the plan, and the Cloud Build `buildId` |

The selective builder runner requires the relevant toolchain on its machine:
Xcode, `cbindgen` and `wasm-pack` for Apple/WASM; Node/npm, pnpm, FVM (`fvm dart`/`fvm flutter`),
Swift, Go and Python/Poetry/PyInstaller for their packages; and authenticated
`gcloud` plus an explicit `AXIOM_GCP_REGION` for image candidates. A selected
target fails before writing a receipt if a tool, remote build result, or
expected output is missing. Container candidates create only immutable
`candidate-*` tags; unlike the old deployment recipes, a build does not
advance a stable runner tag or a Cloud Run revision. The release dashboard
keeps all local source and artifact work on the external APFS build volume.

Publishing `backend-api` or `backend-worker` is fail-closed for the release
worker: the publisher pauses and verifies the Scheduler job and Cloud Tasks
queue before and after rollout, writes `AXIOM_RELEASE_WORKER_ENABLED=false`,
and sets the worker Job's maximum retries to zero. It never executes the Job
or resumes a trigger. A paused queue may retain tasks; inspect its backlog
and resolve the worker's database quota failure before any separately
authorized activation.

The local dashboard proxy build needs `AXIOM_DASHBOARD_ORIGIN` set to the
deployed HTTPS Cloud Run origin. It stages `_worker.js` with a source-bound
release marker, then publishes to the existing `axiom-dashboard` Pages
project. Wrangler's project listing supplies the actual `pages.dev` domain;
it need not equal `<project-name>.pages.dev`. A saved proxy upload intent is
checked against listed deployments and the served marker before retrying, so
an interrupted publish does not silently upload a second deployment. The
proxy verifier sends the same named User-Agent as the other Pages verifier;
Cloudflare can reject Python's default `urllib` User-Agent with HTTP 403 even
when the exact deployment is publicly available. A bounded retry handles
brief propagation delays without treating a persistent 403 as success. The
proxy is a separate wave after its origin when the origin URL
changes. This selector does not provision missing Cloud Run services/jobs,
run backend migrations, or reconfigure production secrets; use the owning
initial-provisioning workflow before the first image-only update.
Each GCP selective builder must set Cloud Build substitution
`_AXIOM_SOURCE_HEADS_SHA256` to SHA-256 of canonical plan `repositories` and
record the successful build ID. The publisher checks that Cloud Build's
reported image digest and Artifact Registry both match the staged descriptor.
Set `AXIOM_GCP_REGION` explicitly for image publishers (and optionally
`AXIOM_GCP_PROJECT_ID`, default `axiomcore`). The three npm packages publish
through their owning GitHub repositories' `npm-trusted-publish.yml` workflows
using npm Trusted Publishing/OIDC. The release controller puts the exact
receipt-verified `.tgz` in a GitHub prerelease handoff, verifies the
downloaded bytes, dispatches the GitHub-hosted workflow, waits for success,
then waits briefly for npm metadata to converge and verifies both
`dist.integrity` and the downloaded registry tarball against the staged
archive. A persistent mismatch blocks publication. Neither a local
`npm login` nor `NPM_TOKEN`/`NODE_AUTH_TOKEN` is used. See
[`NPM_TRUSTED_PUBLISHING.md`](./NPM_TRUSTED_PUBLISHING.md) for the one-time npm
trust grants and default-branch requirement. For pub.dev, configure a dedicated
Google service account as an automated publisher in the Admin tab of both
`axiom_flutter_generator` and `axiom_flutter`, grant the release machine
permission to impersonate it, and set `AXIOM_PUB_SERVICE_ACCOUNT` to its email
in the AxiomCore Infisical `prod` project. The publisher obtains a short-lived
identity token with audience `https://pub.dev`, and Dart stores only the
`PUB_TOKEN` environment-variable reference in its component-scoped SSD cache,
not the token value. A directly supplied `PUB_TOKEN` remains a fallback for an
already-issued short-lived token; do not store an expiring token in Infisical.
The `atmx-web` publisher uses
the `atmx`-bucket-scoped `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` from
the existing Infisical `prod` project, or a complete explicit environment
with `CLOUDFLARE_ACCOUNT_ID`. It never reads the local ignored `.env` file.
The AWS child receives only its S3 pair, without printing it or saving it on
the release SSD. Generic Cloudflare public or
private bucket keys are not assumed to have access to `atmx`. Publication
checks existing versioned objects and verifies both R2 and public-domain
bytes; it never mutates the `latest` alias.
The dashboard reads npm and pub.dev published stable versions when suggesting
candidate versions, and the release preview refuses an already occupied
version. If an older saved run is blocked because its npm version has different
published bytes, the live-run panel offers an explicit successor-train replan.
Review the new version and changelog note, type the displayed `REPLAN` phrase,
click **Step 1 — Save successor version**, then type `RESUME <train>` and click
**Step 2 — Resume saved run**. Resume is withheld until the replan has been
saved; it cannot retry the known occupied npm version. The original staged
archive and run backup remain on the release
SSD; the occupied version is never relabeled as published. Remaining primary
components publish before the new candidate is prepared and built in the
internal follow-up train.

The same explicit recovery applies when an existing pub.dev version has
different package files. The dashboard checks the downloaded pub.dev archive,
proposes an unused patch version, and keeps the old staged receipt untouched.
The Flutter package builder now excludes hidden and `.gitignore`-ignored files
before sealing a receipt, matching Dart's published file set. For the staged
`2026.09.25.4` generator collision, recovery also moves the Flutter SDK's old
over-inclusive archive to the successor train for a rebuild; its still-free
version is preserved. Restart the dashboard process to load updated server
code, but do not advance the tracked remote tips of the current train's pinned
source repositories before its remaining publications complete. Review and
commit local control-plane changes when the run reaches its successor phase;
that phase will pause safely if they remain uncommitted.
After verifying the CLI GitHub archive, the CLI publisher updates only
`Formula/axiom.rb` in the clean `AxiomCore/homebrew-tap` checkout, pushes its
single release commit, and reads the remote formula back. It refuses to
overwrite unrelated tap commits or publish a CLI archive without the
`axiom-macos-arm64.tar.gz` asset required by that formula.
