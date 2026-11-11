# AxiomCore documentation

This directory contains the public AxiomCore documentation application. It is
built with Next.js and Fumadocs from MDX content in `content/docs`.

Read [the public documentation guardrails](AGENTS.md) before changing content,
examples, navigation, diagrams, screenshots, or documentation tooling.

## Local development

Requirements:

- the Node.js version supported by the current Next.js release;
- Corepack or pnpm; and
- Corepack/pnpm and the committed `pnpm-lock.yaml`.

```bash
corepack enable
pnpm install --frozen-lockfile
pnpm dev
```

Open `http://localhost:3000`.

## Visual foundation

The docs use the approved index-v17 AxiomCore theme: black surfaces, orange
accents, square controls, PP Supply Sans headings, Neue Haas body text, and
Commit Mono code and labels. The appearance is dark-only, including search.

`lib/brand` is a committed snapshot of `axiom-frontend/lib/src/styles/{theme,fonts}.css`
and its font assets. Fumadocs tokens are mapped to those shared tokens in
`app/global.css`; the docs logo uses the landing page's SVG geometry.
Keep the snapshot in this repository so a standalone Cloudflare release needs
neither the frontend checkout nor a third-party font service. Refresh it after
changing the shared design system:

```bash
node scripts/sync-brand.mjs
# Or supply the absolute path to another axiom-frontend/lib/src checkout.
```

## Required validation

```bash
pnpm check
```

`check` enforces content policy and navigation, compiles MDX and TypeScript,
builds the production application, writes a reproducible release manifest, and
starts the result briefly to audit public HTML, processed MDX, Open Graph,
sitemap, robots, LLM-reader, health, version, security, and failure routes.

Use focused checks while editing:

```bash
pnpm lint:content
pnpm types:check
pnpm build
pnpm release:manifest
pnpm test:routes
```

Before merging a content change, also confirm:

1. every new public capability has an accurate status in the support matrix;
2. its implementation and target boundary have been reviewed;
3. internal Markdown links resolve;
4. runnable commands exist in the current CLI or owning repository;
5. linked examples in `AxiomCore/examples` contain no private paths, credentials, or internal milestone names; and
6. the affected page renders in the shared dark theme at desktop and narrow
   widths, including code blocks, keyboard navigation, and search.

## Content structure

| Path | Purpose |
| --- | --- |
| `content/docs/introduction` | Product definition, landscape, audience, and support |
| `content/docs/getting-started` | Installation and first local paths |
| `content/docs/guides` | Existing-stack integration and agent workflows |
| `content/docs/core-concepts` | Artifacts, pipeline, extraction, runtime, and trust |
| `content/docs/acore-language` | Acore language manual and schema-backed concepts |
| `content/docs/backend` | Backend authoring, execution, adapters, and target limits |
| `content/docs/frontend` | Existing clients and Acore UI |
| `content/docs/database` | Schemas, persistence, migrations, and fleet delivery |
| `content/docs/examples` | Runnable task/exam apps and advanced patterns |
| `content/docs/testing` | Contracts, mocks, scenarios, and CI |
| `content/docs/delivery` | Review, releases, security, and readiness |
| `content/docs/workflows` | Contract, package, UI, review, test, mock, and release lifecycles |
| `content/docs/tooling` | CLI and local developer tools |
| `content/docs/cloud` | Implemented connected control-plane capabilities |
| `content/docs/reference` | Exact schemas, errors, security rules, diagnostics, and glossary |

Navigation order lives in each directory's `meta.json`. All public pages need
`title`, `description`, and `pageType` frontmatter. Add `testedWith` to
verified runnable procedures.

## Editorial rules

- Use **AxiomCore** for the platform and **Acore** for the language.
- Do not imply that Acore is required for existing backend or frontend code.
- Attach availability to a precise capability and target.
- Mark Axiom Studio, Acode, and Axiom Marketplace as **Coming soon**.
- Never publish internal workstream or numbered milestone names.
- Describe extraction, runtime, trust, and Cloud boundaries as implemented;
  avoid universal or “zero” claims.
- Do not publish runnable instructions for a coming-soon product.

Maintainer-only editorial, deployment, migration, and launch procedures live in
the private `AxiomCore/axiom-internal-docs` repository. The public documentation
workflow validates a release candidate but does not hold production deployment
authority.

## Machine-readable documentation

- `/agents.json` provides a typed page index, tested versions, canonical reader
  URLs, interpretation rules, and redirects.
- `/llms.txt` lists every page with an absolute canonical URL and description.
- `/llms-full.txt` contains processed text for the full public documentation.
- In the server build, `/docs/<path>.mdx` rewrites to the MDX-reader route.
  In the Cloudflare static build, the same URLs are materialized as build-time
  MDX assets from the authored documentation.
- `/docs/index.mdx` is the plain Markdown home page. All readers preserve
  status, tables, code, links, and diagram source while removing presentation JSX.
- `/sitemap.xml` and `/robots.txt` are generated by the application.
- `/api/health` reports a minimal, non-cached service health document.
- `/version.json` reports the public documentation release identity.
- `/.well-known/security.txt` publishes the private vulnerability-reporting
  channel and security-policy location.

These outputs contain public documentation only. Internal evidence and planning
files are maintained outside this repository.

## Ask AI

The docs have an Ask AI chat (a floating **Ask AI** button; Ctrl or Cmd + / opens
it). It uses the Fumadocs AI chat components (`components/ai/chat/`), mounted by
`components/ai/ask-ai.tsx` in `app/layout.tsx`.

- **Server.** Production is a static export, so `functions/api/chat.ts` (a
  Cloudflare Pages Function) answers `POST /api/chat`; `app/api/chat/route.ts`
  does the same for `next dev` and `next start`, and the static build leaves it
  out. Both call `lib/ask-ai/chat.ts`.
- **Configuration.** `OPENROUTER_API_KEY` (secret) and `DOCS_AI_MODEL` (an
  OpenRouter model ID; defaults to `minimax/minimax-m3.1-flash:free`) are set on
  the Pages project for each environment, from Infisical, with the
  abuse-protection settings below. Change the model by
  changing `DOCS_AI_MODEL`; no code change or redeploy of content is needed, but a
  new deployment picks up a changed variable.
- **Retrieval.** `pnpm static:build` writes `out/ask-ai/docs-index.json`, a
  precomputed search index over the same reader Markdown served at
  `/docs/<page>.mdx`. Each question is searched on the server first and the best
  page or sections go to the model, so most answers take one model request (at
  most two: the model may open a page with `read_page`, then must answer). This
  matters for free models, which have a daily request cap per OpenRouter account.
- **Abuse protection.** `lib/ask-ai/guard.ts` holds the checks; the Pages
  Function refuses every question (503) unless all of them are configured:
  - Edge (axiomcore.dev zone): a WAF custom rule blocks `/api/chat` requests that
    are not `POST` from `https://docs.axiomcore.dev`, and a rate-limiting rule
    blocks an IP for 10 seconds after 5 requests in 10 seconds. Neither request
    reaches the function.
  - Human check: Cloudflare Turnstile widget "AxiomCore docs Ask AI" (managed,
    invisible unless Cloudflare asks for a click). Its public site key is in
    `components/ai/human-check.ts`; `TURNSTILE_SECRET_KEY` verifies each
    single-use token, including its action and hostname.
  - Question limits in D1 (binding `ASK_AI_DB`, database
    `axiomcore-docs-ask-ai`): 4 a minute, 20 an hour and 40 a day per visitor (an
    IPv4 address or IPv6 /64), plus a site-wide daily total
    (`ASK_AI_DAILY_LIMIT`, default 300). Visitors are stored only as an HMAC of
    their network keyed by `ASK_AI_HASH_SECRET`, and rows are deleted after their
    window ends.
  - Input: docs origins only, JSON only, a 64 KB body read with a cap, size
    limits on questions and history, and a rebuilt history: only user and
    assistant text and the open page's path reach the model, never client
    `system` messages or tool results.
  `TURNSTILE_SECRET_KEY`, `TURNSTILE_SITE_KEY`, `ASK_AI_HASH_SECRET` and
  `ASK_AI_D1_DATABASE_ID` are in Infisical with the other variables.
- **Local test.** Run `pnpm static:build`, then `wrangler pages dev out --d1
  ASK_AI_DB` from a directory that has `out` and `functions` (symlinks are fine)
  and a `.dev.vars` file with `OPENROUTER_API_KEY` and Cloudflare's test secret
  `TURNSTILE_SECRET_KEY=1x0000000000000000000000000000000AA` (the chat uses the
  matching test site key on localhost). Keep `.dev.vars` out of the repository.

## Publication and maintenance

The repository workflow in `.github/workflows/docs.yml` runs `pnpm check` for
documentation and documentation-policy changes. It does not publish
the site; deployment remains a separately authorized environment operation.
The workflow retains `artifacts/docs-release-manifest.json` as release evidence.

Public lifecycle guidance is maintained under:

- `/reference/versioning-and-deprecation`; and
- `/reference/support-and-feedback` (support channels and how to contribute).

Update the support matrix only after reviewing the matching implementation.
The public content gate requires its review date to be refreshed at least every
120 days. Maintainers record detailed evidence and readiness privately.

## Concept illustrations

`ConceptDiagram` is a reusable MDX component with four reviewed presets:
`contract` (documentation home), `pipeline` (contract pipeline), `inspector`
(Inspector overview), and `evidence` (the four truth layers). Select a node to
read its explanation and follow the relevant guide. The illustration labels
its conceptual scope and does not pretend to run a compiler or capture events.

Motion advances only while the diagram is visible, pauses on manual selection
or keyboard interaction, and is disabled for reduced-motion preferences.
Keep code examples and reference tables as text; use illustrations to explain
relationships and boundaries, not to replace exact commands or API definitions.

## Diagrams and example downloads

Captioned `mermaid` fences render locally to safe SVG at build time. Supported
flowcharts, sequence diagrams, and ER diagrams retain selectable source and
text explanations. External fonts, resources, HTML, directives, and click
handlers are rejected. `pnpm test:content` tests reader semantics and diagram
safety; production/static audits compare every reader with its authored source.

Runnable source lives in `../examples/documentation`. After changing a sample:

```bash
pnpm examples:package
pnpm examples:check
```

The deterministic archives contain source, manifests, dependency locks, and
READMEs. They reject generated SDKs, caches, virtual environments, deployment
files, and binaries. Commit refreshed ZIPs with their source. Run `pnpm check`
and `pnpm static:check` before completing changes to the publishing pipeline.
