# Documentation release readiness

Last verified: **2026-09-21**

This internal record covers the public documentation application in
`content/docs`, its checked-in example entry points, and its discovery output.
It does not promote an alpha or experimental product capability to Available;
public maturity remains governed by `CONTENT_EVIDENCE.md` and the support
matrix.

## Completed gates

- 93 MDX pages have unique titles and descriptions.
- All checked internal page routes resolve.
- Public content and catalogued examples contain no numbered internal
  milestone names, developer home-directory paths, or credential-shaped demo
  values found by the repository scan.
- `pnpm install --frozen-lockfile` succeeds against the committed pnpm lock.
- `pnpm check` passes the content policy, type, build, and HTTP smoke stages.
- `pnpm build` completes 289 static-generation tasks and records 289
  prerendered routes, including page HTML, processed MDX, Open Graph images,
  `llms.txt`, `llms-full.txt`, `sitemap.xml`, and `robots.txt`.
- The release gate generates a content-addressed manifest covering source and
  verification inputs, route counts, application route patterns, public output
  hashes, runtime versions, source revision, and dirty-worktree state.
- The production server audit verifies canonical metadata, landmark headings,
  table semantics, the custom 404, health and version JSON, security contact,
  cache policy, and the complete response-header baseline. It also confirms
  that the framework identity header is absent.
- The repository documentation workflow runs the same release gate for public
  docs, example, and policy changes without holding deployment authority.
- The complete-corpus audit checks all 93 HTML pages, 93 Open Graph images, 92
  per-page processed MDX routes, exact sitemap/LLM-reader inventory parity, and
  every legacy redirect against the production server.
- The Axiom Inspector documentation track covers canonical evidence, facts and
  relationships, frontend/backend inspection, contracts and executable closure,
  security and authority, runtime audits, semantic change and impact, the local
  dashboard, agent-oriented CLI output, and the bounded optional Jev planner.
- The historical `/cloud/deplying-mocks` URL permanently redirects directly to
  the canonical `/cloud/deploying-mocks` page and is excluded from discovery
  output.
- Browser acceptance follows that legacy URL to the canonical Cloud mocks page,
  renders one `h1` without horizontal overflow, returns the page from search,
  and applies the expected dark background and foreground. The previously
  recorded 390 CSS-pixel checks remain applicable because this stage changes no
  layout or styling code.
- All six adoption-guide routes and their machine-reader/social-card variants
  return HTTP 200 from the production server.
- The FastAPI adoption page renders with semantic headings, labelled controls,
  code regions, status text, and no browser console errors in the collaborative
  browser smoke test.
- The production-readiness page renders in a 390 CSS-pixel document without
  horizontal overflow; its system-driven dark appearance applies the expected
  dark background and light foreground.
- The versioning page renders with semantic table/headings and no browser
  console errors. The support page renders at 390 CSS pixels without horizontal
  overflow and applies the expected dark appearance.
- The documentation-site privacy page hydrates under the production Content
  Security Policy without console or network errors, has one `h1`, has no
  horizontal overflow, and applies the expected dark background and foreground.
  The `frame-ancestors 'none'` policy blocks framing as intended.
- Domain examples validate and build with Axiom CLI 0.141.0.
- The security baseline passes; the deliberately unsafe strict fixture fails
  with `AXSEC-001` as expected.
- Repository-relative Flutter fixture dependencies resolve with FVM. The RPC
  and stream generated clients retain non-fatal analyzer lint findings; the
  observability/auth fixture analyzes cleanly.

## Known boundaries

- AxiomCore remains an alpha ecosystem. A successful documentation build is
  not product stability or production certification.
- Connected Cloud paths require an account, authorization, and a configured
  control plane; they were not exercised by the local documentation gate.
- Native client release builds, physical devices, signing, and app-store
  publication require their own target validation.
- Axiom Studio, Acode, and Axiom Marketplace remain Coming soon and have no
  runnable public instructions.
- The current semantic diff identifies a removed domain projection path but
  does not classify that removal as breaking; a human reviewer must apply the
  consumer-compatibility decision.
- Historical versioned documentation and a general support/LTS SLA are not
  published today; the site describes the current reviewed implementation.
- No production environment was changed during this review. Preview promotion,
  monitoring, incident response, and rollback require the separately authorized
  deployment procedure in `DEPLOYMENT.md` and the unchecked operator items in
  `LAUNCH_CHECKLIST.md`.

## Re-run when

Repeat the full gate when command syntax, Acore grammar, package kinds, target
support, trust policy, product availability, documentation routing, or a
catalogued example changes. Update the evidence date only after implementation
and public claims are checked together. A full evidence review is required at
least every 120 days even when individual claims appear unchanged.
