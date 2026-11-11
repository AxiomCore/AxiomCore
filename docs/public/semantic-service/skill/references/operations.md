# Operations of axiom-agent-semantic/v1

Optional reading. One entry per operation. The authoritative definitions are the JSON schemas listed at https://docs.axiomcore.dev/semantic-service/schemas/index.json: `operations/<operation>.request.json` (params) and `operations/<operation>.result.json` (result, and the error codes in `x-errors`); the protocol reference, https://docs.axiomcore.dev/semantic-service/protocol, explains them. Field names are snake_case.

**Which of these are served right now is in the `capabilities` manifest, not here.** A `planned` operation answers `unsupported_operation` (`not_implemented`).

Envelope fields, in short: `workspace` (all but `capabilities` and `workspace.open`), `snapshot` (reads: an id or `latest`), `base_snapshot` (`change.propose`, `edit.plan`: an id), `request_id` (required on `change.propose` and `change.apply`), `limits` (`max_bytes`, `max_nodes`, `max_depth`, `max_items`), `params`.

Besides the codes listed per entry, operations can return the common codes `invalid_arguments`, `not_found`, `internal`, `resource_limit`, `permission_denied` and `unsupported_operation` (`capabilities` and `workspace.open` only a subset; each result schema's `x-errors` has the exact set). "Read codes" below means `snapshot_expired` and `stale_snapshot`.

## Discovery and attach

### `capabilities`
- Purpose: the manifest of what this server serves.
- Params: `profiles` (optional).
- Result: `manifest` (`protocol`, `service`, `operations` with `status` `implemented`, `partial` or `planned` and `notes`, `edit_kinds`, `profiles` with node and edge kinds, `limits` with `defaults` and `maximums`, `checks`, `publication`, `error_codes`) and `manifest_fingerprint`.
- Errors: `unsupported_operation` (`protocol_unsupported` for a protocol string the server does not speak), `internal`.

### `workspace.open`
- Purpose: attach local project roots and get a `workspace` handle.
- Params: `roots` (`[{path, entry?}]`, absolute paths; `entry` defaults to `main.acore`), `language_profile`, `target` (optional).
- Result: `workspace` (`id`, `roots`, `source_authority`), `projects` (each with `status` `ready`, `needs_preparation` with `preparation` steps, or `unsupported`), `capabilities` (the manifest).
- Errors: `permission_denied` (`path_escape`, `root_not_allowed`), `not_found`, `invalid_arguments`, `resource_limit`.

## Reading

### `project.overview`
- Purpose: modules, entry points, imported contracts and diagnostics, paginated.
- Params: `projects`, `include` (`modules`, `entry_points`, `contracts`, `diagnostics`), `cursor`.
- Result: `projects` (with `semantic_state`), `modules`, `entry_points`, `contracts`, `diagnostics`.
- Errors: read codes.

### `node.find`
- Purpose: find candidate nodes.
- Params: `query`, `limit` (required), `kind`, `owner`, `path`.
- Result: `nodes` (`id`, `kind`, `name`, `path`, `version`) and `matches` with `reasons`.
- Errors: `compile_failed`, read codes.

### `node.get`
- Purpose: facts about known node ids, batched.
- Params: `nodes` (1 to 50), `projection` (`summary`, `standard`, `full`), `include` (`properties`, `edges`, `incoming`, `body`, `provenance`, `edits`), `limit`.
- Result: `nodes` with properties, edges, optional body, provenance and edit summary.
- Errors: `compile_failed`, read codes.

### `graph.query`
- Purpose: typed relationships: what a node depends on, what depends on it, where it is referenced, or paths between nodes.
- Params: `from`, `mode` (`dependencies`, `dependents`, `references`, `paths`), `to` (with `paths`), `edge_kinds`, `layers`, `transitive`.
- Result: `traversed` (the relations actually followed), `nodes`, `edges` (with `layer` and `depth`), `paths`, `boundaries`.
- Errors: `compile_failed`, read codes.

### `context.build`
- Purpose: task-shaped, bounded context with inclusion reasons.
- Params: `focus` (node ids), `purpose` (`understand`, `change_input`, `change_behavior`, `diagnose`; `change_schema` needs a backend profile and answers `unsupported_operation`), `include` (`signature`, `implementation`, `input_constraints`, `callers`, `consumers`, `authorization`, `diagnostics`, `tests`), `expand` (refs from an earlier response on the same snapshot). Set `limits.max_bytes`.
- Result: `items` (node, `depth`, `required`, `reasons`, `signature`, optional `source`) and `edges`.
- Errors: `insufficient_context` (with `details.required_bytes` and `details.expand`), `compile_failed`, read codes.

### `impact.analyze`
- Purpose: what a change to some nodes may affect.
- Params: `targets`, `direction` (`dependents`, `dependencies`, `both`), `edge_kinds`.
- Result: `impacted` (node, `depth`, `from`, `edge_kind`, `layer`). `tests` is not available from this service (no test associations are indexed), so it is omitted; do not read its absence as "no tests".
- Errors: `compile_failed`, read codes.

### `changes.since`
- Purpose: what changed between an older snapshot and this one.
- Params: `since`, `scope` (`nodes`, `paths`, `candidate`), `cursor`.
- Result: `since`, `history` (`known`, `unknown`, `expired`), `files`, `nodes` (relocations), `relevant`.
- Errors: read codes.

### `node.relocate`
- Purpose: map node ids from an older snapshot to this one.
- Params: `from_snapshot`, `nodes` (1 to 200).
- Result: `relocations`, one per id in order: `old_id`, `new_id`, `status`, `relation` (`proven` only with `basis` `id_map`; otherwise `inferred`), `basis`.
- Errors: `compile_failed`, read codes.

## Editing

### `node.edits`
- Purpose: the edits that are legal on given nodes right now.
- Params: `nodes` (1 to 20), `kinds`, `include_unavailable`.
- Result: per node `node_version`, `options` (`option_id`, `fingerprint`, `kind`, `property`, `accepts`, and per kind `states`, `targets`, `ops`, `text`) and, if asked, `unavailable` (kind, `code`, `reason`).
- Errors: `compile_failed`, read codes.

### `edit.plan`
- Purpose: dry run of a batch of edit operations; no candidate is created.
- Envelope: `base_snapshot`; no `request_id`.
- Params: `operations` (same shapes as `change.propose`).
- Result: `outcome`, `reason`, `owners`, per-operation plan, `files`, `required_checks`.
- Errors: `precondition_failed`, `stale_snapshot`, `ambiguous_owner`, `read_only_origin`, `compile_failed`, `snapshot_expired`.

### `change.propose`
- Purpose: apply ordered edit operations to a private candidate and compile it.
- Envelope: `base_snapshot`, `request_id`.
- Params: `operations` (1 to 50), `candidate` and `expected_candidate_version` (to extend an open candidate), `checks`.
- Edit kinds: `set_property` (`node`, `property`, `old_value`, `new_value`), `bind_property` (`node`, `property`, `old_value`, `target` state), `list_element` with `op` `remove` (`node`, `old_text`) or `insert` (`anchor`, `position`, `old_text`, `element`), `declaration` (`create`, `update`, `remove`), `set_reference`, `import` (`add`, `remove`). Each may carry `expected_node_version`, `option` (`option_id`, `fingerprint`) and `handle`. Check `edit_kinds` in the manifest for which are served.
- Result: `candidate` (`id`, `version`, `state`, `expires_at`), `replayed`, per-operation outcomes, the composed `id_map`, `files` (digests only), `diagnostics`, `checks`, `evidence`, `impact`.
- Errors: `precondition_failed`, `stale_snapshot`, `ambiguous_owner`, `read_only_origin`, `compile_failed`, `conflict`, `snapshot_expired`.

### `change.inspect`
- Purpose: review a candidate before applying it.
- Params: `candidate`, `include` (`semantic_diff`, `source_diff`, `content`, `affected`, `checks`).
- Result: `candidate`, `files`, `semantic_diff` (with `expected` per entry), `source_diff`, `affected`, `risks`, `checks`.
- Errors: `snapshot_expired`.

### `check.run`
- Purpose: run checks on a candidate, or on a snapshot plus overlay files.
- Envelope: `snapshot` exactly when `params.candidate` is absent.
- Params: `checks` (for example `compile`), and either `candidate` or `files` (`[]` checks the snapshot as is).
- Result: `ok` and one entry per check with `status` `passed`, `failed`, `not_run` or `unavailable`. A failing check is a result, not an error.
- Errors: `stale_snapshot`, `snapshot_expired`.

### `change.apply`
- Purpose: publish a candidate to the local files.
- Envelope: `request_id`.
- Params: `candidate`, `expected_candidate_version`, `preconditions` (`[{path, sha256}]`; by default every written file must still have its base digest), `require_checks`.
- Result: `replayed` and `receipt` (`receipt_id`, `outcome` `applied`, `not_applied` or `unknown`, `result_snapshot`, `files`, `id_map`, `journal`).
- Errors: `precondition_failed` (`external_write`, `candidate_version_mismatch`), `conflict` (`recovery_required`, `request_id_reused`), `compile_failed`, `snapshot_expired`.

### `change.discard`
- Purpose: release an unapplied candidate. Never undoes an applied change.
- Params: `candidate`, `expected_candidate_version`.
- Result: `candidate` and `released`.
- Errors: `precondition_failed` (`candidate_applied`).
