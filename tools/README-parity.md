# Dashboard parity checks

Run from AxiomCore with a sibling frontend checkout:

```sh
python3 -m unittest discover -s tools/tests -p 'test_parity*.py'
python3 tools/parity-check.py --frontend ../axiom-frontend --strict
```

Strict mode returns 0 when all dashboard actions map to CLI commands or reviewed
exemptions, 1 for coverage gaps, and 2 for extraction or exemption errors.

The source scanner accepts `nativeFetch(input, init)` only inside the single
reviewed `window.fetch = function fetch(input, init)` interceptor body in
`api-client.js`. Nested callbacks belong to that body; quoted strings and comments
do not affect its boundary. Unquoted slash syntax (regex literals or division)
in that body fails closed and requires scanner review. The startup session request has its own path and
options checks. New `window.fetch.bind(...)` aliases and replacement interceptors
must be reviewed and modeled before the audit can pass. Only the existing
`nativeFetch` binding is accepted.

Archive and recovery `this.send(...)` actions must be literal selectors, apart
from the existing `s.action` step-up retry and recovery `c.action` confirmation
forwarding. Unknown variable selectors fail extraction instead of disappearing
from coverage. When changing these flows, update their finite selector resolver
and mutation tests together.

The owner-browser-only exemption is valid only when every CLI mapping carries a
note explaining the `BrowserOnly` refusal. A generic partial-coverage note cannot
justify this exemption. Unknown categories, routes, or dashboard callers remain
errors. This audit verifies source correspondence; it does not establish runtime
permissions or payload equivalence.
