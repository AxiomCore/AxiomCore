# Documentation deployment runbook

This runbook covers the public documentation application at
`https://docs.axiomcore.dev`. It defines a provider-neutral promotion contract;
credentials, DNS changes, production aliases, and rollback operations require
an authorized deployment owner.

Use `LAUNCH_CHECKLIST.md` as the acceptance record for preview and production
cutover. URL changes additionally follow `MIGRATION.md`.

For the static Cloudflare Pages implementation, use the build and deployment
procedure in `CLOUDFLARE_PAGES.md`. The static health and version documents are
build-time release identity, not live server probes.

## Release inputs

A release candidate consists of:

- one repository revision;
- `docs/package.json` and `docs/pnpm-lock.yaml`;
- public MDX, application code, and documentation scripts from that revision;
- the successful `pnpm check` result; and
- `docs/artifacts/docs-release-manifest.json` produced by that check.

Do not rebuild from a moving branch after approval. Promote the reviewed build
or rebuild the exact revision with the same Node major and locked dependencies.

## Environment contract

| Setting | Production requirement |
| --- | --- |
| Node.js | 22.x for the current CI/deployment baseline; package minimum remains explicit in `package.json` |
| Package manager | Corepack and the pinned pnpm release from `packageManager` |
| Canonical origin | `https://docs.axiomcore.dev` |
| `VERCEL_GIT_COMMIT_SHA` or `GITHUB_SHA` | Optional public source revision for health/version output; only a hexadecimal revision is accepted |
| `VERCEL` | When set by that provider, enables the implemented analytics component |
| Secrets | None required to build or serve public documentation |

Do not expose control-plane credentials, deploy tokens, or private API endpoints
to the documentation runtime. Provider credentials belong to deployment
automation, not the application environment.

## Build and preview

```bash
cd docs
corepack enable
pnpm install --frozen-lockfile
CI=1 NEXT_TELEMETRY_DISABLED=1 pnpm check
```

The command produces `artifacts/docs-release-manifest.json`. Publish the build
to an isolated preview and verify:

```bash
curl --fail --show-error https://PREVIEW_HOST/api/health
curl --fail --show-error https://PREVIEW_HOST/version.json
curl --fail --show-error https://PREVIEW_HOST/.well-known/security.txt
curl --fail --show-error --head https://PREVIEW_HOST/reference/versioning-and-deprecation
```

Also inspect the home page, one long reference table, search, a processed MDX
route, a social image, a 404, and a 390 CSS-pixel layout in light and dark
appearance. Confirm there are no console or failed-network errors.

## Promotion gate

Promote only when:

1. the preview source revision matches the reviewed manifest;
2. canonical, Open Graph, sitemap, robots, and machine-reader URLs use the
   production origin;
3. security headers and content types match the automated audit;
4. the evidence ledger date matches the support matrix and readiness record;
5. no Coming soon feature has runnable instructions; and
6. the previous healthy deployment remains selectable for rollback.

The validation workflow does not deploy. Add a provider-specific deployment job
only after repository environment protection, reviewer ownership, and scoped
credentials are configured.

## Post-promotion checks

Verify from outside the deployment account:

- `/api/health` returns HTTP 200 and `status: "ok"`;
- `/version.json` reports the intended source revision when supplied;
- `/`, `/llms.txt`, `/sitemap.xml`, and one Open Graph image return HTTP 200;
- an unknown path returns the custom HTTP 404 page;
- `Content-Security-Policy`, HSTS, referrer, permissions, frame, and content-type
  protections are present; and
- search and navigation reach the newly published pages.

Monitor availability, 5xx rate, latency, failed asset loads, and build/runtime
errors. Analytics usage and retention must follow the deployment privacy review.

## Rollback

Rollback selects the previous known-good immutable deployment. Do not repair a
failed production build in place or rebuild an unpinned branch.

1. record the failing revision, manifest, first error, and affected routes;
2. move the production alias to the previous healthy deployment;
3. verify health, version, security headers, home, search, and a reference page;
4. open a corrective change against the failed revision; and
5. produce a new preview and manifest before promoting again.

If only DNS or certificate service is failing, changing application content is
not a valid fix. Escalate to the owning provider/DNS operator and preserve the
last healthy application deployment.

## Security and privacy incident

For accidental publication of a credential or private contract, treat removal
from the current page as containment, not remediation. Roll back or disable the
affected deployment, rotate/revoke the exposed material, purge provider caches
where supported, inspect repository history and artifacts, and follow the
private process in the repository `SECURITY.md`.
