# Documentation maintenance handbook

This handbook defines how the public documentation remains accurate after its
initial release. It assigns responsibilities to roles rather than individuals
so repository permissions and team membership can change without weakening the
review contract.

## Responsibilities

| Role | Responsibility |
| --- | --- |
| Capability owner | Confirms commands, schemas, availability, limitations, and implementation evidence for the capability being changed. |
| Documentation reviewer | Reviews structure, terminology, links, examples, accessibility, and consistency with the evidence ledger. |
| Release operator | Produces the locked build, reviews the release manifest, promotes an approved artifact, monitors it, and can roll it back. |
| Security contact | Receives private vulnerability reports and keeps `security.txt` plus the repository security policy current. |

One person may perform multiple roles, but capability evidence and release
approval should receive an independent review whenever repository protections
allow it.

## Change-triggered maintenance

Update documentation in the same change whenever any of these surfaces change:

- Acore grammar, standard modules, schemas, or evaluator behavior;
- CLI commands, arguments, defaults, exit behavior, or environment variables;
- package kinds, artifact formats, trust policy, runtime behavior, or targets;
- public Cloud routes, access requirements, or release semantics;
- client integration APIs, generated output, supported frameworks, or hosts;
- examples, availability labels, support boundaries, or product names; or
- public routes, canonical URLs, metadata, analytics, or deployment behavior.

The owning change must update affected concept, workflow, reference,
troubleshooting, example, evidence, and machine-readable surfaces together.

## Review cadence

| Cadence | Required review |
| --- | --- |
| Every change | Run `pnpm check`; review changed public pages in light and dark appearance at desktop and narrow widths. |
| Monthly | Confirm the security contact and expiry, canonical origin, health/version output, analytics boundary, and rollback access. |
| At least every 120 days | Revalidate the complete evidence ledger, support matrix, availability labels, current examples, and release-readiness record. |
| Before promotion | Complete `LAUNCH_CHECKLIST.md` against an immutable preview and its release manifest. |

The content policy rejects evidence dates more than one calendar day ahead of
UTC and reviews older than 120 days. The one-day tolerance supports reviewers
east of UTC; updating the date without rechecking the ledger is not maintenance.

## Triage expectations

- Treat unsafe instructions, exposed secrets, incorrect security boundaries,
  and broken release guidance as urgent.
- Treat incorrect API or command reference, broken navigation, missing
  migration redirects, and false availability as release-blocking.
- Treat wording, discoverability, and non-blocking example improvements through
  the normal documentation review path.
- Use the private repository security channel for vulnerabilities; public
  documentation issues must not contain credentials or private contracts.

## Route lifecycle

Public URL changes follow `MIGRATION.md`. New destinations must ship with their
legacy redirects, and redirect removal requires recorded evidence. Sitemap,
canonical metadata, LLM-reader output, and navigation always use the current
destination rather than a legacy source.

## Maintenance evidence

The authoritative records are:

- `CONTENT_EVIDENCE.md` for implementation-backed capability claims;
- `RELEASE_READINESS.md` for the most recent complete local acceptance result;
- `artifacts/docs-release-manifest.json` for generated build identity; and
- the protected deployment provider for preview, promotion, and rollback
  history.
