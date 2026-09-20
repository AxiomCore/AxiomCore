# Jev planning in Axiom Inspector

Axiom Inspector can optionally use Jev through Vercel AI Gateway to translate a
developer's natural-language question into the existing bounded
`axiom-inspector-question/v1` grammar.

Jev is a planner, not an evidence source:

```text
developer question
  -> deterministic candidate retrieval (local)
  -> Jev Choice evaluation (remote, explicit opt-in)
  -> confidence and allowlist gate (local)
  -> QuestionRequest validation (local)
  -> EvidenceIndex query (local)
  -> deterministic facts and trace
```

The compiler, evidence graph, source, node attributes, graph edges, extension
authority, secrets, and runtime values are not uploaded. The remote request
contains only:

- the question text;
- up to 48 deterministic candidate semantic IDs, kinds, and labels;
- six fixed intent choices;
- instructions that restrict selection to those choices.

## Authentication with Infisical

The CLI reads `AI_GATEWAY_API_KEY` from its process environment. It does not
read `.env` files and never logs the value.

For the current Infisical project, run commands through the development slug:

```bash
infisical run --env=dev -- \
  laxiom inspect ask \
  "What code can mess with the cart total?" . \
  --planner jev \
  --allow-remote
```

The value stored in Infisical must be a Vercel **AI Gateway API key**, created
from the AI Gateway area or with:

```bash
vercel ai-gateway api-keys create --name axiom-inspector-local
```

Do not paste the key into source, command arguments, logs, or support output.

## Dashboard

Remote planning is disabled by default. Enable it for one loopback Inspector
session:

```bash
infisical run --env=dev -- \
  laxiom inspect serve . --allow-remote-planner
```

Ask Axiom then offers two modes:

- **Local deterministic** uses the existing phrase planner and makes no remote request.
- **Jev · remote** sends the bounded planning payload and displays the resulting
  probabilities, acceptance/abstention state, and receipt path.

The dashboard cannot enable Jev unless the server was explicitly started with
`--allow-remote-planner`.

## Confidence and abstention

Defaults:

- intent probability: `0.65`;
- selector probability: `0.55`.

Override them only for evaluation:

```bash
laxiom inspect ask "Who touches checkout total?" . \
  --planner jev --allow-remote \
  --jev-intent-threshold 0.75 \
  --jev-selector-threshold 0.70
```

Below threshold, with an unsupported intent, or without an allowlisted target,
Jev abstains. Inspector may use its local phrase planner when that parser can
handle the original question; otherwise it returns an unsupported answer. Jev
can never invent a selector or bypass `QuestionRequest::validate`.

Exact semantic names such as `Home.total_label` are resolved locally before the
selector confidence gate. This is deliberately deterministic: Jev classifies
the natural-language intent, while ordinary identifier equality stays in code.
Fuzzy targets still require Jev to select one allowlisted candidate above the
selector threshold. Receipts distinguish `deterministic-exact`,
`deterministic-exact+jev`, and `jev` selector sources and retain Jev's evaluated
selector separately for audit.

## Audit receipts

Each completed Jev decision is stored locally at:

```text
.axiom/inspector/planner-receipts/jev-<timestamp>-<question-hash>.json
```

Receipts contain the graph revision, question SHA-256 (not the question),
candidate set, thresholds, probabilities, resolved model, selected typed
request, token usage, and Gateway routing metadata. They are written with
owner-only permissions on Unix. Canonical Inspector answers remain separate
and deterministic.

## Failure behavior

- No `--allow-remote`: the CLI rejects Jev before any network request.
- Missing key: the CLI explains that Infisical injection or an exported key is required.
- Gateway failure: the command returns an error and does not claim a deterministic answer.
- Jev abstention: the decision is audited and the bounded local fallback is attempted.
- Invalid Jev output: local deserialization and request validation reject it.

The current pilot does not use Jev for parsing, compilation, semantic diff,
permission enforcement, sandbox grants, authentication, impact calculation, or
fact generation.
