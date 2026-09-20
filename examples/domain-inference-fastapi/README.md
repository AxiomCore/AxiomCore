# FastAPI domain inference fixture

This fixture demonstrates a bounded extraction and promotion workflow:

- `main.py` declares `Customer` and `Order` Pydantic models;
- the FastAPI extractor discovers models, field validation, routes, and a
  relationship candidate;
- `axiom.acore` promotes the two application concepts with `extend`; and
- domain validation produces compiler-owned semantic identities and a
  canonical manifest hash.

FastAPI extraction imports and executes the selected module's top-level
initialization. Run this fixture in a Python environment with FastAPI and
Pydantic installed.

## Validate

From the repository root:

```bash
python -m venv .venv
source .venv/bin/activate
python -m pip install fastapi pydantic
axiom install axiom-fastapi --module

cd examples/domain-inference-fastapi
axiom domain validate axiom.acore
axiom build axiom.acore --variant default
```

Expected result:

- domain validation reports a canonical manifest hash;
- the build writes `axiom.axiom`; and
- the artifact's domain manifest contains the promoted entities, inferred
  relationship evidence, and declared invariant/projection.

This is a local fixture. It does not require a Cloud release or a local Cloud
endpoint override.
