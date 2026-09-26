# Contributing to AxiomCore

AxiomCore is a pre-stable contract platform spread across independently owned
repositories. Keep a change inside the owning boundary and do not infer that a
passing test in this repository validates every runtime, SDK, target host, or
Cloud service.

## Before opening a change

- Read `README.md`, `SECURITY.md`, and the affected documentation section.
- Do not use a public issue or pull request for a security vulnerability.
- Preserve unrelated work in a dirty checkout.
- State the capability, target, current behavior, expected behavior, and
  smallest reproducible input.
- Add implementation tests before changing a public claim.

## Documentation

The public site lives in `docs/content/docs`. Keep capability status accurate in
the public support matrix. Maintainers keep editorial evidence and deployment
records in the private `AxiomCore/axiom-internal-docs` repository.

Run the complete gate:

```bash
cd docs
corepack enable
pnpm install --frozen-lockfile
pnpm check
```

The gate checks page metadata, navigation coverage, links, forbidden internal
terminology, unsafe example values and paths, MDX/TypeScript, the production
build, and public route responses. If a page changes layout, also inspect it at
a narrow viewport and in light and dark appearance.

## Examples

Examples are bounded fixtures, not production templates. Use synthetic data,
relative paths, and development-only placeholder values. Never commit access
tokens, signing keys, DSNs, cookies, sandbox credentials, production URLs, or
developer home-directory paths.

Run the exact example commands listed in the [examples repository](https://github.com/AxiomCore/examples). A deliberately
unsafe or incompatible fixture must document its expected non-zero result.

## Public claims

Attach status to a precise surface: Available, Alpha, Experimental, or Coming
soon. Do not describe Acore as mandatory for existing Python, Go, Flutter,
React, web, or Swift code. Do not present Axiom Studio, Acode, or Axiom
Marketplace as installable products until their status and evidence change.

When changing a command or API, update its reference, affected workflow,
troubleshooting guidance, examples, evidence row, and machine-readable output
in the same review. The documentation release gate also generates a manifest
that binds the source and verification inputs to the audited public outputs.
