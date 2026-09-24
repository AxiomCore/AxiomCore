# AxiomCore release operator guide

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
backend worker, mock runner, contract-test runner, and docs. Four affected
consumers are queued: `atmx-react` (after web npm bytes), Flutter and Swift
(after the Apple framework bytes/checksum), and the CLI (after its generated
Flutter SDK pins are settled). The CLI's `0.147.0` manifest/lock edit and its
release-note fragment were already present in the worktree when this wave was
organized; they are preserved, not published. Train `2026.09.24.1` already has
CLI-only preparation evidence on the SSD, so the expanded foundation wave uses
a fresh train ID rather than overwriting it. The intent's top-level `summary`
describes the release-control-plane theme; each component's `summary` becomes
its own changelog fragment when that component is activated and prepared.

Candidate versions for **all 20 components** live in
[versions.json](./versions.json). Cargo, npm, and pubspec manifests remain
package-manager-facing mirrors. The ledger is not a published-version record:
the latter must come from a signed, remotely verified baseline. Services,
jobs, and sites use immutable image/deployment digests rather than SemVer.

## One interface, three routine actions

1. Run `just release` to see affected components and the next action. Review
   [intent.json](./intent.json); its `changes` are this wave, and `queued` are
   accounted-for follow-on releases. Edit the one-line `summary` and `type`
   there when describing a change. Use a fresh train ID for each new wave.
2. If a SemVer candidate needs changing, run
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
   identifies local builders and CI-only gaps. It does not build, push,
   publish, or deploy. From a noninteractive shell, `prepare` prints a short
   dirty-repository summary and asks you to run `just release review` in a
   terminal. After preflight, run each owner repo's tests and use
   `just release COMPONENT` for a component with a supported local builder
   (`cli` or `ui-host`).

`prepare` mounts the **existing** SSD-backed APFS image if needed. It does not
create or format a disk. The normal path is derived automatically from the
train ID: `/Volumes/AxiomReleaseBuild/axiom-release/trains/TRAIN/`. No command
argument or shell-wide `AXIOM_RELEASE_BUILD_ROOT` export is needed. An
advanced override must still pass the external-volume safety checks.

The candidate command is intentionally immutable: it plans exact committed
source SHAs, checks committed notes, builds supported targets into the SSD,
verifies receipts, stages a candidate, and records the gate and notes under
`trains/TRAIN/components/COMPONENT/`. If an input is dirty, an artifact folder
already exists, a dependency is outside the active wave, or a component needs
a CI-only builder, it stops before claiming success. `just release test` runs
the control-plane tests; `just release help` prints the command summary.

## Releasing one specific component later

For a CLI fix after SDK pins are settled, make a fresh intent with `cli` in
`changes` and an accurate one-line summary, run
`just release version cli 0.147.1` (choose the actual next verified version),
then `just release prepare`. Review and commit the Cargo manifest, lockfile,
and generated note. Run `just release cli` after its source tests.
That command produces a verified local archive; it does **not** publish it.

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

`just release publish COMPONENT` currently **fails closed**. It will not call
the old broad `release-all`, source-mutating publishers, or overwrite GitHub
assets. Production release still needs a trusted signed baseline, selective CI
builders for the CI-only components, publisher adapters that consume exact
receipts, remote digest/health verification, partial-release retries, and
changelog finalization. See [PUBLISHER_MIGRATION.md](./PUBLISHER_MIGRATION.md).
Until those adapters exist, no single safe command can honestly deploy all
components. Never rename a staged candidate to `published` or treat a local
installed host version as the published baseline.
