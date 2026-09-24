# From candidate gate to production release

The current platform control plane **cannot yet publish all 20 components**.
`release-plan` is a source decision; `release-gate` proves a locally staged
candidate is internally consistent. Neither proves that a remote registry or
deployment has those bytes. Do not mark a staged manifest `published` or feed
it back as a baseline. This document is the implementation contract for the
remaining production path.

## Required state machine

```text
intent -> prepared source PRs -> committed source -> strict plan
       -> selected builds + verified reuse receipts -> staged candidate
       -> approved publisher jobs -> remote digest/health verification
       -> signed published manifest -> owner changelogs/archive
```

Every transition records the train ID, catalog digest, intent digest, exact
source SHAs, per-component version, artifact SHA-256, publisher identity,
destination, and evidence URI. A failed target leaves the train staged or
partially published; it never creates a successful baseline. Retrying must be
idempotent by immutable version/digest and must not use `--clobber`.

Production publishing should run in protected GitHub Actions environments.
The local control plane may prepare, test, build supported targets on the SSD,
and inspect evidence; it must not implicitly publish when a developer runs
`release-plan`, `release-build`, or `release-gate`.

## Milestone 1 — trusted baseline and release identity

- Provision a separate platform-release signing key in Infisical for protected
  CI. Commit only its public verification key and rotation policy. Never print
  or export the private key to a developer shell or the external SSD.
- Add `release baseline import` to reconstruct the current baseline from live
  immutable GitHub, npm, pub.dev, Artifact Registry/Cloud Run, and Cloudflare
  records. Verify remote bytes/digests before signing it. No synthetic
  `status: published` JSON is accepted.
- Make `read_baseline` verify the signature and trusted source automatically.
  An absent/invalid baseline may still permit a first-build preview but not a
  production publish or a `REUSE` decision.

Acceptance: tampered baseline, missing registry artifact, wrong source SHA,
rotated key, and duplicate component fail closed in tests.

## Milestone 2 — selective builders and receipts

- Give each catalog component an owning CI build adapter. Every job checks out
  exact plan SHAs, runs target tests, emits an immutable artifact or image
  digest, and writes the same receipt schema as local builders. Do not silently
  call broad `release-all` or `deploy-prod` recipes.
- Locally, use only `/Volumes/AxiomReleaseBuild/axiom-release`; CI uses runner
  temporary storage. Source edits happen in reviewed commits, not inside a
  build job. Build jobs never receive publisher credentials.
- Reuse an unchanged component only by fetching the exact baseline artifact
  and verifying its checksum/signature. The host release must account for web,
  Android, and iOS even when one target changed.

Acceptance: a docs-only change schedules no CLI/native build; an ABI change
schedules affected consumers; a dirty/missing checkout or mismatched renderer
lock stops before building.

## Milestone 3 — publisher adapters and remote verification

| Destination | Components | Required publication proof |
| --- | --- | --- |
| GitHub immutable assets | `cli`, `runtime-apple`, `ui-host-*`, `extractor-fastapi`, `extractor-go` | Tag absent before create; exact staged bytes uploaded once; download and hash match; host manifest signature validates; Homebrew checksum updated only after CLI asset verification. |
| npm | `sdk-atmx-web`, `sdk-atmx-react`, `sdk-atmx-cli` | Publish one package/version at a time from its verified tarball; registry integrity equals staged bytes; React's lock and dependency range resolve to the already-published web version. |
| pub.dev and Swift | `sdk-flutter-generator`, `sdk-flutter`, `sdk-swift` | Published package/tag resolves to the reviewed commit; Flutter plugin's independent Apple runtime pin and Swift's XCFramework checksum match a verified framework asset. Publish dependency first. |
| GCP | `backend-api`, `backend-worker`, `mock-runner`, `contract-test-runner`, `dashboard-origin` | Cloud Build source SHA and immutable image digest match the candidate; Cloud Run service/job revision points to that digest; readiness/health and migration gates pass. |
| Cloudflare Pages | `dashboard-proxy`, `docs` | Deployment ID, production URL, source SHA, build digest, and smoke checks are recorded. A changed dashboard origin URL forces a proxy rollout even with unchanged proxy source. |

Each adapter must be independently callable for its component, run only after
the candidate gate passes, and refuse an existing version with different bytes.
The current host publisher rebuilds all three targets and the old Apple,
Flutter, ATMX, and CLI scripts mutate source or overwrite assets; they are not
valid adapters without migration. Remote deployment is a separate approval
step from artifact upload where a channel or service can be promoted later.

Acceptance: simulate a publish failure after the first component, verify no
signed successful baseline appears, then retry without changing or replacing
the already-published immutable bytes.

## Milestone 4 — changelogs and finalization

- Fragments are created by `release-prepare-apply`, reviewed and committed in
  the owning repository. The control plane generates a whole-train rollup and
  owner-specific changelog entries from exactly the selected fragments.
- Only after publisher verification, a finalizer writes the dated owner
  `CHANGELOG.md` entry (creating the file on first release if absent), archives
  the fragments by component/version/train, and commits a signed published
  manifest with remote references. It refuses to re-finalize the same train.
- Keep independent component versions. The train ID is correlation, not a
  shared SemVer bump; services and sites are identified by immutable digest,
  revision, or deployment ID. App contracts retain their own Cloud version.

Acceptance: one component's patch release does not bump siblings; a breaking
fragment without migration guidance or a missing owner fragment blocks; a
partial deployment cannot archive notes as a completed train.

## Cutover rule

Do not replace the existing owner commands until their adapter has an end-to-end
CI rehearsal with a disposable tag/project/package scope, a documented
rollback, and remote byte verification. Remove blanket `release-all` from the
operator path only after every destination above has passed that gate.
