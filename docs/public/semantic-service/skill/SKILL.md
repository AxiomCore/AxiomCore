---
name: acore-semantic
description: Work on an Acore (.acore) project through the acore-semantic MCP server instead of grepping and rewriting source files. Use when a task reads or changes an Acore UI project (pages, components, states, actions, elements) and the acore-semantic tools are connected. Covers attaching a workspace, checking capabilities, finding nodes, building bounded context, discovering legal edits, proposing, checking and applying a change, and reacting to each error code.
compatibility: Needs the acore-semantic MCP server (acore-semantic mcp) speaking axiom-agent-semantic/v1. Local projects only; no network or account.
---

# Working on Acore projects through acore-semantic

The service compiles the project and answers from compiler facts. It finds nodes, explains their relationships, tells you which edits are legal, applies them as a private candidate, compiles that candidate, and writes the files only when you apply. Prefer it to grep and hand edits: its edits are minimal source splices with exact preconditions, and every result says what it covered and what it left out.

This file is workflow guidance. It is not live state and not a language manual. The server is the authority on what exists.

## Rule zero: ask the server what it serves

1. Call `capabilities` (or read the manifest that `workspace.open` returns) before anything else.
2. Use only operations whose `status` is `implemented`, or `partial` within the subset its `notes` describe. Use only edit kinds listed under `edit_kinds` as `implemented`.
3. Never assume an operation exists because this skill mentions it. A `planned` operation answers `unsupported_operation` (`not_implemented`).
4. Read `limits.defaults` and `limits.maximums` from the manifest; do not guess them.

The MCP tool list is generated from the same schemas. A client may show a tool under a slightly different name (for example with `_` instead of `.`); match it by its operation name and follow its input schema exactly.

## Request shape

Every call carries the protocol `axiom-agent-semantic/v1` and, besides `capabilities` and `workspace.open`, the `workspace` handle. Reads name a `snapshot` (an id or `latest`; the response names the snapshot it used, so keep that id and reuse it). Edits name a `base_snapshot`, which must be a real id, never `latest`. `change.propose` and `change.apply` require a `request_id`, a fresh unique string per intended change. Arguments go in `params`; byte and count bounds go in `limits` (`max_bytes`, `max_nodes`, `max_depth`, `max_items`).

Node ids are opaque and valid within one snapshot. Never build or edit an id by hand, never parse it, and never carry it to another snapshot without `node.relocate` or an `id_map`.

## Workflow

1. **Attach.** `workspace.open` with `roots: [{path: "<absolute project dir>"}]`. Check each project's `status`: `ready` is usable; `needs_preparation` lists `preparation` steps (for example a missing UI lock) that the user must run, because the service never installs anything; `unsupported` means stop and tell the user.
2. **Orient.** `project.overview` lists modules, entry points, contracts and diagnostics. If the project does not compile, start from these diagnostics.
3. **Find.** `node.find` with a `query` (name, id or literal text), an optional `kind` and a `limit`. Each node comes with a `version`. For relationships use `graph.query` (`mode`: `dependencies`, `dependents`, `references` or `paths`) or `impact.analyze`. Use `node.get` for properties and edges of known ids.
4. **Build context.** `context.build` with `focus` (node ids), a `purpose` (`understand`, `change_input`, `change_behavior`, `diagnose`) and `limits.max_bytes`. The purpose `change_schema` needs a backend profile and answers `unsupported_operation` here. Every item says why it is included and whether it is `required`. Start small; expand with `expand` refs from the response rather than raising limits blindly.
5. **Discover legal edits.** `node.edits` on the nodes you will change. Each option has an `option_id`, a `fingerprint`, the `kind`, the `property` and what it `accepts` (and for `bind_property` the allowed `states`). Anything not offered is not legal now; `include_unavailable: true` lists the edit kinds that are not offered for the node (the service does not say why). For a multi-step change, `edit.plan` dry-runs the batch and returns owners, files and `required_checks` without creating a candidate.
6. **Propose.** `change.propose` with `base_snapshot`, `request_id` and ordered `operations`. Give each operation its exact precondition (`old_value` or `old_text`, copied from `node.get` or `node.edits`), the `expected_node_version`, and `option: {option_id, fingerprint}` when you took it from `node.edits`. The batch is all-or-nothing. The result is a `candidate` (`id`, `version`), per-operation outcomes, the composed `id_map`, file digests, diagnostics and `checks`. Nothing is on disk yet.
7. **Inspect and check.** `change.inspect` the candidate (`include`: `semantic_diff`, `source_diff`, `affected`, `checks`) and confirm the semantic diff is exactly what you meant; an entry with `expected: false` is a surprise to investigate. `check.run` with the candidate and the checks you need (compile always runs on propose). A failed check is a result with `ok: false`, not an error: fix by proposing again, extending the candidate with `candidate` and `expected_candidate_version`.
8. **Apply.** `change.apply` with `candidate`, `expected_candidate_version`, a new `request_id` and `require_checks` (at least `compile`). Read the `receipt`: `outcome` is `applied`, `not_applied` or `unknown`. After `applied`, use `result_snapshot` and the receipt's `id_map` for further work. On `unknown`, repeat the same `change.apply` with the same `request_id`; the service reconciles from its journal. If `change.apply` fails with `resource_limit` and reason `timeout`, the change may be on disk even though you have no receipt (`details.outcome` is `unknown`, or `not_applied` when this attempt wrote nothing; an earlier attempt with the same `request_id` may still have): repeat the identical `change.apply` with the same `request_id` once. It returns the receipt or applies the candidate exactly once. The server exits after a timeout, so reconnect it first if its tools are gone. Never re-propose, use a new `request_id`, or edit the files by hand to "retry" an apply.
9. **Clean up.** `change.discard` a candidate you abandon. Discard never undoes an applied change; undo is a new compensating change.

Tell the user what you applied: the files from the receipt and the semantic diff in one or two sentences.

## Honest reading

- `coverage.completeness` other than `complete` means gaps; an empty answer proves absence only when `coverage.exhaustive` is true.
- `truncation.truncated` with a `continuation` means there is another page on the same snapshot; omissions carry expansion refs.
- Edges have a `layer`; inferred relations are labelled. Do not present them as proven. Test associations are not served in this profile; a request for them comes back as a coverage gap.

## When an error comes back

Errors carry `code`, `reason`, `retryable` (`none`, `same_request`, `after_refresh`) and `next_actions`. Decide from those fields, never from `message`. Follow `next_actions` when present. Details: `references/errors.md`.

| Code | What to do |
| --- | --- |
| `stale_snapshot` | Your base is behind the head and a touched file changed. Call `changes.since` with your old snapshot and the ids you hold in `scope.nodes` (unscoped, it lists only changed nodes, never added ones); map them with the `nodes` it returns (or `node.relocate`). If `history` is not `known`, re-find the nodes with `node.find`. Re-read the values, then propose again on the new snapshot with a new `request_id`. |
| `precondition_failed` | A value, node version, option fingerprint, candidate version or on-disk file changed (`reason` says which; `details` may hold the current value). Re-read with `node.get` or `node.edits`, rebuild the operation, propose again. On `external_write` at apply, someone edited the file: run `changes.since`, discard the candidate, start over from the new snapshot. |
| `insufficient_context` | The required context exceeds your `max_bytes`. Retry with fewer `include` kinds or a narrower `focus`, then pass selected refs from `details.expand`. Raise `max_bytes` only up to the manifest maximum and only if the task needs it. |
| `compile_failed` | Read `diagnostics` (and `operation_index`). Fix the arguments, for example the inserted element source, and propose again. If the base itself does not compile, use `project.overview` diagnostics and repair with supported edits first. |
| `unsupported_operation` | The operation, edit kind or option is not served (`not_implemented`, `edit_not_offered`, `unknown_operation`). Do not retry. Re-check `capabilities` and `node.edits` for a supported route. If there is none, see "Plain file edits" below. |
| `ambiguous_owner` | No single authored owner. Choose from `details.owners` if given and target that node explicitly; otherwise ask the user. Never pick by text position. |
| `conflict` | `request_id_reused`: you reused a key with different input; use a new `request_id`. `recovery_required`: a pending apply left files matching neither side; stop and show the user the paths. `concurrent_candidate`: refresh and propose again. |
| `resource_limit` | A service bound was hit (`reason` names it). Retry unchanged only when `retryable` is `same_request`: `queue_full`, and a `change.apply` that hit `timeout` (repeat it with the same `request_id`; see step 8). Otherwise narrow the request (smaller focus, fewer nodes, a page cursor), or report the limit; the same input will fail again. |

Others: `snapshot_expired` (open or re-read on `latest` and redo the reads), `not_found` (re-find the node, do not guess), `read_only_origin` (the value is generated, inherited or from a dependency; edit its source owner or tell the user), `invalid_arguments` (fix the call against the tool schema), `permission_denied` (outside the attached roots or refused by policy; do not work around it), `internal` (retry once unchanged, then report).

## Plain file edits

Do not fall back to grep and hand edits because an edit failed; every error above has a semantic route. A plain file edit is legitimate only when the change is genuinely not served: the manifest lists the needed operation or edit kind as `planned` or absent, or `node.edits` with `include_unavailable` reports it unavailable for this node. Then:

1. Tell the user which operation is missing and that you are editing the file directly.
2. Keep the edit minimal and in the owning file.
3. Afterwards re-read on `latest` and run `check.run` with `files: []` on the new snapshot to compile it. Old node ids and candidates are now stale.

Non-Acore files (docs, configs, other languages) are outside this service; edit them normally.

## References (optional)

- `references/operations.md`: one entry per operation (purpose, key params, result, errors).
- `references/errors.md`: retry semantics and reasons per code.
- `references/examples.md`: a `bind_property` change and a context-then-edit flow on an example shopping app.
- Schemas: https://docs.axiomcore.dev/semantic-service/schemas/index.json lists every schema file.
