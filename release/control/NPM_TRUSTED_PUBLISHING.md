# Unattended npm publishing

The release dashboard publishes these public npm packages from their owning
GitHub repositories. It builds and gates each candidate as before, transfers
the **same** staged `.tgz` to a GitHub prerelease handoff, verifies the downloaded
bytes, dispatches the repository's GitHub-hosted workflow, and checks the
published npm `dist.integrity` before recording `remote-verified`.

| Package | GitHub repository | Trusted workflow filename |
| --- | --- | --- |
| `atmx-web` | `AxiomCore/atmx` | `npm-trusted-publish.yml` |
| `atmx-react` | `AxiomCore/atmx-react` | `npm-trusted-publish.yml` |
| `atmx-cli` | `AxiomCore/AxiomCore` | `npm-trusted-publish.yml` |

## One-time bootstrap

1. Review and merge each repository's workflow and `package.json` repository
   URL onto its **default branch**. GitHub will not accept a
   `workflow_dispatch` for a workflow absent from the default branch. Keep
   the source branch being released pushed as well.
2. As an npm package maintainer, create a **Trusted Publishing → GitHub
   Actions** entry in each package's npm settings. Use the exact organization,
   repository, and workflow filename in the table. Allow direct `npm publish`
   (not only `npm stage publish`). Do not set an environment name unless the
   workflow is changed to use that same GitHub environment.
3. Ensure GitHub Actions is enabled on all three repositories. The release
   operator needs GitHub authorization to create a prerelease and dispatch
   a workflow; this is **GitHub** authorization, not npm authentication.

All three packages already exist on npm (each was at `0.146.0` when this
integration was prepared), so their maintainers can configure trust without
bootstrapping a new package through a token. Pre-OIDC staged tarballs without
the matching `repository.url` must be rebuilt in a new candidate; the workflow
will reject them rather than repack or silently change their bytes.

npm does not provide a way for an unauthenticated process to create a package's
trust grant. That one-time package-owner grant is unavoidable; after it is in
place, no browser, OTP, npm login, or npm publishing token is needed during
release runs. Do not add `NPM_TOKEN`, `NODE_AUTH_TOKEN`, or npm publish secrets
to these workflows. Remove/rotate old npm publish tokens after confirming the
first OIDC release.

## Day-to-day

Use the normal release dashboard or `just release publish sdk-atmx-web`,
`sdk-atmx-react`, or `sdk-atmx-cli` after the candidate is staged. The browser
run waits for each workflow and registry verification before moving to its
dependent package. `atmx-react` still waits for the exact published
`atmx-web` version and its committed dependency lockfile. `atmx-web` still
publishes and verifies its immutable R2 browser assets using the separate
Cloudflare credentials; npm OIDC does not grant R2 access.

The GitHub prerelease is a transport handoff, not a stable product release.
The tarball is visible there shortly before npm publication according to
the repository's visibility: `atmx` and `atmx-react` are currently private,
while `AxiomCore` is public. This is an intentional trade-off so the
GitHub-hosted runner can read the exact bytes with only `contents: read` and
no extra storage credential. The handoff is
created only after the operator confirms publication, never during prepare
or build. If npm rejects the publish, the prerelease remains as evidence and
is not silently deleted or replaced.
Its tag, source commit, asset SHA-256, and Action run are saved under the
component publication evidence on the external release SSD. A retry reuses
the same handoff and workflow run; it will not silently dispatch a second
publish. If a workflow fails, inspect its Action URL, fix the cause, and
explicitly rerun the failed GitHub workflow or prepare a new candidate.

The workflow runs on a GitHub-hosted runner with Node 24, npm 11, and
`id-token: write`. It checks the prerelease, source commit, package name,
version, repository URL, and tarball SHA-256 before invoking direct
`npm publish`. It does **not** rebuild the package. The release controller
then checks npm's SHA-512 integrity against its original staged tarball.
Trusted publishing works for private source repositories, but npm does not
generate a provenance attestation for them; the published npm tarball still
receives the OIDC authorization and the controller's integrity verification.

Sources: [npm trusted publishing](https://docs.npmjs.com/trusted-publishers/),
[GitHub workflow dispatch](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/manually-run-a-workflow).
