# Worked examples

Optional reading. Both examples use an example shopping app: the member price button on the Home page, `element:Home/view/0/0/3/1/0`, whose `label` is `"Apply member price"` and whose `on_press` triggers `action:Home.apply_member_price`, which writes `state:Home.cart.total_label`.

Requests are shown as full envelopes. Over MCP you call the tool for the operation with the same fields; the tool's input schema is authoritative. Snapshot and workspace ids are whatever the server returns. Versions and fingerprints are shortened here with `...`; always copy them whole from the response.

Before either example: `capabilities` (or `workspace.open`'s manifest) must list every operation used below as `implemented` (or `partial` covering this use) and `bind_property` and `set_property` under `edit_kinds`. If not, stop and use what is served.

## 1. Bind the button label to a state (`bind_property`)

Task: "Make the member button show the cart total label."

**Attach.**

```json
{"protocol": "axiom-agent-semantic/v1", "operation": "workspace.open", "snapshot": "latest",
 "params": {"roots": [{"path": "/path/to/shopping-app"}], "language_profile": "acore-ui", "target": "web"}}
```

The result has `workspace.id` `ws-shop-1`, the project with `status` `ready`, and the envelope names the snapshot, say `snap-0007`.

**Find the button.**

```json
{"protocol": "axiom-agent-semantic/v1", "operation": "node.find", "workspace": "ws-shop-1", "snapshot": "snap-0007",
 "params": {"query": "Apply member price", "kind": "element", "limit": 10}}
```

One match: `element:Home/view/0/0/3/1/0`, with its `version`.

**Discover legal edits.**

```json
{"protocol": "axiom-agent-semantic/v1", "operation": "node.edits", "workspace": "ws-shop-1", "snapshot": "snap-0007",
 "params": {"nodes": ["element:Home/view/0/0/3/1/0"], "include_unavailable": true}}
```

Among the options: `{"option_id": "o4", "fingerprint": "251e6f87...", "kind": "bind_property", "property": "label", "accepts": "String", "states": ["state:Home.cart.discount_label", "state:Home.cart.pricing_status", "state:Home.cart.total_label"]}`. The target state is in `states`, so the edit is legal. `unavailable` shows `set_reference` on `on_press` as `unsupported_operation`: do not try to retarget the handler.

**Propose.**

```json
{"protocol": "axiom-agent-semantic/v1", "operation": "change.propose", "workspace": "ws-shop-1",
 "base_snapshot": "snap-0007", "request_id": "bind-label-1",
 "params": {"operations": [{"kind": "bind_property", "node": "element:Home/view/0/0/3/1/0",
   "expected_node_version": "4755202c...", "property": "label", "old_value": "Apply member price",
   "target": "state:Home.cart.total_label", "option": {"option_id": "o4", "fingerprint": "251e6f87..."}}]}}
```

The result: `candidate` `{"id": "cand-2", "version": 1, "state": "open"}`, `id_map` `[]` (binding changes no ids), one modified file `pages/home.acore` with before and after digests, and a `compile` check `passed`. Nothing is on disk yet.

**Inspect, then apply.**

```json
{"protocol": "axiom-agent-semantic/v1", "operation": "change.inspect", "workspace": "ws-shop-1",
 "params": {"candidate": "cand-2", "include": ["semantic_diff", "source_diff"]}}
```

The semantic diff should show only `label` on the button changing from the literal to the state, with `expected: true`. Then:

```json
{"protocol": "axiom-agent-semantic/v1", "operation": "change.apply", "workspace": "ws-shop-1", "request_id": "apply-bind-label-1",
 "params": {"candidate": "cand-2", "expected_candidate_version": 1, "require_checks": ["compile"]}}
```

`receipt.outcome` `applied` and a `result_snapshot` (say `snap-0008`). Tell the user: "Bound the member button's label to `cart.total_label` in pages/home.acore; compile check passed."

**If it fails.**
- `precondition_failed` with `reason` `old_value_mismatch` (`details` carries the current value): the label changed. Re-read with `node.get`, confirm the task still makes sense, propose again with the current `old_value` and a new `request_id`.
- `stale_snapshot`: the file changed after `snap-0007`. Follow `next_actions`: `changes.since` with `since` `snap-0007` and the held ids (the member button) in `scope.nodes`, map them with its `nodes` (or `node.relocate`); if `history` is not `known`, re-find the button with `node.find`. Then propose on the new head.
- `precondition_failed` with `external_write` at apply: someone edited `pages/home.acore`. `changes.since`, `change.discard` the candidate, start over.

## 2. Read context first, then edit (`context.build`, `set_property`)

Task: "The member button's wording is confusing; rename it to 'Apply member pricing' if nothing depends on the text."

**Build bounded context around the button.**

```json
{"protocol": "axiom-agent-semantic/v1", "operation": "context.build", "workspace": "ws-shop-1", "snapshot": "snap-0007",
 "limits": {"max_bytes": 2048, "max_nodes": 3, "max_depth": 2},
 "params": {"focus": ["element:Home/view/0/0/3/1/0"], "purpose": "change_input", "include": ["signature", "callers", "diagnostics"]}}
```

The result's `items`:
- the button itself (`required: true`, reason `focus`), with its signature `Button(class: String, label: String, on_press: Action, accessibility_label: String)` and its source;
- `action:Home.apply_member_price` (`required: true`, reason `caller`: "Button.on_press: triggered action");
- `state:Home.cart.total_label` (`required: false`, reason `consumer`: "written by apply_member_price").

`edges`: the button `triggers` the action, the action `writes` the state, both on the `resolved` layer. Check `coverage` and `truncation`: if `truncated` is true, the omitted parts carry expansion refs. Nothing reads the label text, so the rename is local.

If the server answers `insufficient_context` instead (for example with `focus` `page:Home` and `max_bytes` 1024), do not raise the limit to the maximum: retry with `include: ["signature"]` as `next_actions` suggests, then pass one ref from `details.expand` in `expand`.

**Discover and propose.** `node.edits` on the button (as in example 1) lists `{"option_id": "o2", "kind": "set_property", "property": "label", "accepts": "String", ...}`. Propose:

```json
{"protocol": "axiom-agent-semantic/v1", "operation": "change.propose", "workspace": "ws-shop-1",
 "base_snapshot": "snap-0007", "request_id": "rename-label-1",
 "params": {"operations": [{"kind": "set_property", "node": "element:Home/view/0/0/3/1/0",
   "expected_node_version": "4755202c...", "property": "label",
   "old_value": "Apply member price", "new_value": "Apply member pricing",
   "option": {"option_id": "o2", "fingerprint": "eba80e54..."}}]}}
```

**Check and apply.** `change.inspect` with `include: ["semantic_diff", "source_diff", "checks"]` shows one property change, `label` `"Apply member price"` to `"Apply member pricing"`, and a one-line source hunk in `pages/home.acore`. If you want an explicit check run as well:

```json
{"protocol": "axiom-agent-semantic/v1", "operation": "check.run", "workspace": "ws-shop-1",
 "params": {"candidate": "cand-1", "checks": ["compile"]}}
```

`ok: true`. Then `change.apply` with `candidate` `cand-1`, `expected_candidate_version` 1, `require_checks` `["compile"]` and a new `request_id`.

**What not to do.** Do not open `pages/home.acore` and replace the string with a text edit: the service checks the old value, preserves every other byte, compiles the result and reports the exact change, and a hand edit invalidates the snapshot and any open candidate.
