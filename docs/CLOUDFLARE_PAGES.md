# Deploy documentation to Cloudflare Pages

This documentation application can be deployed as a static site to Cloudflare
Pages. The deployment has no Node.js server and no Pages Functions: pages,
social images, machine-reader routes, search indexes, health, and version data
are generated during the build and uploaded as static assets.

The production origin is `https://docs.axiomcore.dev`. Cloudflare Pages applies
the generated `_headers` and `_redirects` files at its edge, preserving the
security policy and the documented legacy URL redirect without Next.js server
features.

## One-command deployments

After the one-time configuration below, these commands are all run from the
`docs/` directory:

```bash
just deploy-initial                  # create the Pages project and first production upload
just deploy-preview feature-docs     # upload a non-production preview
just deploy                          # update the production deployment
```

Each deployment installs locked dependencies, runs the full server release
gate, builds and verifies the static export, then uploads `out/`. The scripts
pass the current Git revision and dirty-worktree status to Cloudflare as
deployment metadata and to the build-time health/version documents.

`deploy-initial` is intentionally only for a new Pages project. It fails if
the project already exists. `deploy-preview` requires a branch name that does
not match `CLOUDFLARE_PAGES_PRODUCTION_BRANCH` (which defaults to `main`).

## One-time setup

### 1. Install local tools and sign in to Infisical

```bash
cd docs
corepack enable
pnpm install --frozen-lockfile
brew install just
brew install infisical/get-cli/infisical
infisical login
```

The committed `.infisical.json` links this directory to the existing
AxiomCore Infisical project. It contains no secret values. If your Infisical
project changes, rerun `infisical init` from `docs/` and commit only
the resulting non-secret project configuration.

Wrangler is a pinned development dependency, so use `pnpm exec wrangler` rather
than a globally installed copy.

### 2. Configure the two Infisical secrets

In the AxiomCore Infisical project, add these secrets to the environments that
will deploy documentation:

| Secret | Required value |
| --- | --- |
| `CLOUDFLARE_ACCOUNT_ID` | Cloudflare account ID |
| `CLOUDFLARE_API_TOKEN` | API token with **Account → Cloudflare Pages → Edit** |

No local `.env` file is read or generated. The Pages project and branch are
non-secret defaults in the deployment script:

| Setting | Default |
| --- | --- |
| Pages project | `axiomcore-docs` |
| Production branch | `main` |

Override either default only when needed by exporting
`CLOUDFLARE_PAGES_PROJECT` or `CLOUDFLARE_PAGES_PRODUCTION_BRANCH`
in the job environment.

### 3. Create and deploy the Pages project

```bash
just deploy-initial
```

This command runs under Infisical, creates the Direct Upload Pages project, and
publishes its first production deployment. Run it only once: it fails if the
project already exists. A Direct Upload project cannot later be converted to
Git integration; use these commands or a CI workflow for subsequent deploys.

### 4. Attach `docs.axiomcore.dev`

In Cloudflare Dashboard, open **Workers & Pages** → the Pages project →
**Custom domains** → **Set up a domain**, then enter `docs.axiomcore.dev`.

If `axiomcore.dev` is an active Cloudflare zone in the same account, Cloudflare
can create the required DNS record. Otherwise create the CNAME requested by the
dashboard, typically:

| Type | Name | Target |
| --- | --- | --- |
| CNAME | `docs` | `axiomcore-docs.pages.dev` |

Wait until the dashboard shows the hostname as active before treating the custom
domain as a production target.

### 5. Use the deployment commands

The Just recipes use `infisical run` to inject the two credentials into
the deploy script. They use this AxiomCore project's Infisical environment slug
`prod` for initial/production uploads and `staging` for previews by default.

```bash
just deploy-preview feature-docs
just deploy
```

If the project’s environment slugs change, override them without changing
source code:

```bash
INFISICAL_PRODUCTION_ENV=prod just deploy
INFISICAL_PREVIEW_ENV=dev just deploy-preview feature-docs
```

For CI, authenticate the Infisical CLI with a machine identity or service token
through `INFISICAL_TOKEN`, then invoke the same Just command. Keep the
Infisical authentication credential in the CI secret store; Cloudflare
credentials remain in Infisical and are injected only into the deployment
process.

## Test before upload

Build the exact static assets and validate their Cloudflare configuration:

```bash
just static-check
```

Run the actual Pages static server locally:

```bash
just preview 8788
```

Open `http://127.0.0.1:8788`, verify search, navigation, a reference page, the
legacy `/cloud/deplying-mocks` URL, the 404 page, and both light and dark
appearance. Stop the local server with `Ctrl-C`.

## Verify a deployed site

Replace `SITE` with the preview URL or `https://docs.axiomcore.dev`:

```bash
SITE=https://docs.axiomcore.dev
curl --fail --show-error --head "$SITE/"
curl --fail --show-error "$SITE/api/health"
curl --fail --show-error "$SITE/version.json"
curl --fail --show-error "$SITE/.well-known/security.txt"
curl --fail --show-error "$SITE/sitemap.xml" >/dev/null
curl --fail --show-error "$SITE/llms.txt" >/dev/null
curl --fail --show-error --location "$SITE/cloud/deplying-mocks" >/dev/null
```

Inspect the first response for `Content-Security-Policy`,
`Strict-Transport-Security`, `X-Content-Type-Options`, and `X-Frame-Options`.
The static `/api/health` and `/version.json` files describe the revision at
build time; they do not execute a runtime health probe.

## Operational notes

- The static search index downloads to the browser and is queried locally. It
  is suitable for the current documentation corpus; reassess its payload size
  if the corpus grows substantially.
- `_headers` and `_redirects` are generated into `out/` from the shared
  deployment policy and redirect inventory. Do not hand-edit generated files.
- A production upload is immutable. Use the Cloudflare Pages deployment history
  to roll back to the previous healthy deployment, then fix forward through a
  new reviewed build.
- The full promotion and incident procedure remains in `DEPLOYMENT.md`; the
  final acceptance list is in `LAUNCH_CHECKLIST.md`.
