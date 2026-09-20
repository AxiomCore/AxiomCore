# Documentation launch checklist

Use this checklist for the first production cutover and every material
documentation-site migration. Local implementation completion does not grant
authority to change DNS, aliases, credentials, or production deployments.

## Repository acceptance

- [x] Public terminology, availability, and evidence policies are automated.
- [x] Navigation covers every public MDX page and internal page routes resolve.
- [x] Legacy URL migration is explicit and tested without redirect chains.
- [x] Locked installation, type generation, production build, and route audit
  are available through `pnpm check`.
- [x] The production audit covers every public page, per-page social image,
  processed MDX representation, sitemap/LLM inventory, and legacy redirect.
- [x] Security headers, custom failures, health, version, security contact, and
  release-manifest integrity are tested.
- [x] Preview, promotion, monitoring, incident, rollback, and maintenance
  procedures are documented.

## Authorized preview

- [ ] Build the exact reviewed revision using Node 22 and the committed pnpm
  lockfile.
- [ ] Retain `artifacts/docs-release-manifest.json` with the preview evidence.
- [ ] Confirm `/version.json` reports the intended revision.
- [ ] Review the home page, search, a long reference page, a guide, a legacy
  redirect, a 404, and machine-readable output on the preview host.
- [ ] Check light and dark appearance at desktop and 390 CSS-pixel widths.
- [ ] Confirm there are no browser console errors or failed first-party assets.
- [ ] Obtain the repository/environment approvals required by the deployment
  provider.

## Production cutover

- [ ] Preserve the previous healthy immutable deployment for rollback.
- [ ] Promote the approved preview without rebuilding a moving branch.
- [ ] Run the external post-promotion checks in `DEPLOYMENT.md`.
- [ ] Confirm certificate, DNS, canonical URLs, sitemap, robots, health, version,
  security headers, search, analytics policy, and the legacy redirect.
- [ ] Record the promoted revision, release manifest, operator, approval, time,
  and previous rollback target in the deployment system.

## Acceptance rule

The repository portion is complete when `pnpm check`, browser acceptance, and
the release-readiness record agree. Production launch is complete only after an
authorized operator checks every preview and production item above. If any
production item fails, follow the rollback procedure before attempting a new
release.
