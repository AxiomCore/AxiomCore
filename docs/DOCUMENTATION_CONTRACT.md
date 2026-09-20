# AxiomCore documentation contract

This file is the editorial and information-architecture contract for public
AxiomCore documentation. It is maintained with the docs application but is not
published as a documentation page.

## Product vocabulary

- **AxiomCore** is the contract platform and ecosystem. It includes the Acore
  language, CLI, contract artifacts, build and runtime libraries, UI and app
  tooling, test and mock tooling, and cloud services.
- **Acore** is the declarative, contract-oriented language used to author
  `.acore` source. It has its own lexer, parser, evaluator, standard library,
  formatter/editor services, and language server. Never describe it as a thin
  alias for another language.
- **Axiom CLI** is the `axiom` command-line interface.
- **Axiom Inspector** is the implemented alpha CLI and secure local dashboard
  over canonical application evidence. Its facts and relationships are
  deterministic. Optional Jev planning interprets a question but never becomes
  the source of application facts.
- **Axiom package** is a canonical, versioned `.axiom` envelope. Its kind is
  service, theme, component library, database schema, extension, or application
  policy.
- **Axiom application** is a target-specific or multi-target `.axiomapp`
  artifact produced from Acore UI source and resolved dependencies.
- **Axiom Cloud Dashboard** is the implemented web control plane for accounts,
  projects, releases, and related cloud workflows. Describe only shipped views.
- **Axiom Studio**, **Acode**, and **Axiom Marketplace** are product directions
  that are not yet usable products. Every mention must carry a Coming soon label.

## Availability labels

| Label | Editorial meaning |
| --- | --- |
| Available | Implemented, documented, and usable on the stated path. This label does not imply a stable major release or universal production certification. |
| Alpha | Implemented and testable, but APIs or workflows can still change. |
| Experimental | A bounded implementation exists; parity, compatibility, or distribution is incomplete. |
| Coming soon | Planned or in development, with no supported user workflow today. Never include installation or usage instructions. |

Status is attached to a capability, not inherited from the whole company or
product. Each support statement must name its boundary (target, framework,
command, host, or artifact) and be traceable to the evidence ledger.

## Writing rules

1. Lead with the user outcome, then explain contracts and implementation.
2. Use **AxiomCore** and **Acore** exactly. Use `axiom` for CLI commands,
   `.acore` for source, `.axiom` for packages/contracts, and `.axiomapp` for
   application artifacts.
3. Do not expose internal workstream names, milestone codenames, or numbered
   implementation phases in public copy.
4. Avoid absolutes such as “zero bugs,” “completely eliminates,” “identical on
   every platform,” and “production ready” unless a reproducible test proves the
   exact claim.
5. Separate implementation from direction. Planned products always use the
   `coming-soon` status component.
6. Do not position Acore as mandatory. Existing Flutter, web, Swift, Python,
   Go, and contract-description workflows can adopt the implemented AxiomCore
   boundary incrementally where the support matrix says they can.
7. Code examples must state their language and must not contain unpublished
   commands presented as runnable.
8. Tables require headers; diagrams require a text explanation; status must
   never be communicated by color alone.

## Page templates

### Concept page

Outcome → definition → where it fits → boundaries → related reference.

### How-to page

Goal → prerequisites → numbered procedure → expected result → troubleshooting.

### Reference page

Surface and version → exact syntax/API → parameters and defaults → errors →
verified examples.

### Product direction page

Coming soon badge → intended problem → planned relationship to shipped
capabilities → explicit statement that it cannot currently be installed or used.

## Review gate

Before merging a public claim, confirm a source path and test/command in
`CONTENT_EVIDENCE.md`. If evidence is missing, downgrade the status or describe
the item as a direction rather than an implementation.

Run the executable gate from `docs/`:

```bash
pnpm check
```

The content policy validates frontmatter, unique metadata, navigation coverage,
internal routes, public terminology, example paths and placeholder safety, and
review-date consistency. The remaining stages compile types, build all public
representations, generate content-addressed release evidence, and audit the
production routes, metadata, semantics, failure handling, operational endpoints,
and response security policy. CI runs the same command; deployment is a
separate authorized operation governed by `DEPLOYMENT.md`.

The evidence ledger and release-readiness record must share a valid ISO review
date. The executable policy rejects dates more than one calendar day ahead of
UTC and reviews older than 120 days. Public URL changes must follow
`MIGRATION.md` and retain a tested direct redirect to the current canonical page.
