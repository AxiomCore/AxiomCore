# AxiomCore examples

These examples are reviewable fixtures for the implemented AxiomCore and Acore
surfaces. They demonstrate a bounded capability; they are not production
application templates and do not contain production credentials.

## Recommended starting points

| Example | Demonstrates | Validation |
| --- | --- | --- |
| `domain-commerce` | Self-contained domain entities, relationships, invariants, projections, and an audience-visible projection change | `axiom domain validate`, `axiom diff` |
| `domain-support` | A compact domain model and audience-specific projection | `axiom domain validate` |
| `domain-inference-fastapi` | FastAPI/Pydantic model extraction followed by explicit domain promotion | `axiom domain validate`, `axiom build` |
| `security-mode-v1` | Audit-mode security baseline plus a deliberately rejected strict fixture | `axiom security check` |
| `simple-backend` | FastAPI and Go services with generated-client application fixtures | Validate each backend and selected frontend separately |
| `offline-first-feed` | Cache/offline-oriented service and web consumer fixture | Contract build plus application integration checks |
| `realtime-chat` | Go service and web/React/Flutter stream consumers | Contract build plus target-specific stream lifecycle checks |

The `rpc`, `stream`, and `observability-and-auth` directories are lower-level
runtime fixtures. Use them when changing the owning runtime or SDK; they are
not the first onboarding path.

## Run a self-contained contract

```bash
cd examples/domain-commerce
axiom domain validate commerce-v1.acore
axiom build commerce-v1.acore
axiom diff commerce-v1.acore commerce-v2.acore --format semantic
```

The second contract intentionally removes an audience-visible projected field.
The current semantic output identifies the changed projection path; reviewers
must treat that removal as a consumer-breaking change.

## Run the FastAPI inference fixture

Create a Python environment containing FastAPI and Pydantic, and install the
FastAPI extractor expected by your CLI:

```bash
python -m venv .venv
source .venv/bin/activate
python -m pip install fastapi pydantic
axiom install axiom-fastapi --module

cd examples/domain-inference-fastapi
axiom domain validate axiom.acore
axiom build axiom.acore
```

The extractor imports `main.py`. Keep module initialization declaration-only
and review the resulting artifact before using it as release evidence.

## Example safety rules

- Relative paths only; never commit a developer home-directory path.
- Synthetic data only.
- Placeholder values must be visibly non-production.
- Never commit tokens, private keys, telemetry DSNs, cookies, or sandbox keys.
- Do not present an internal acceptance fixture as a supported product path.
- Pin dependency intent and commit locks when the example demonstrates
  consumption rather than authoring.

See the public [example catalog](../docs/content/docs/guides/example-catalog.mdx)
for capability status and the associated documentation.
