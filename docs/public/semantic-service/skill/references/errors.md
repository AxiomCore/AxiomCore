# Errors and retry semantics

Optional reading. The error object is defined by https://docs.axiomcore.dev/semantic-service/schemas/error.json: `code`, `reason` (optional refinement), `message` (for people; never parse it), `operation_index` (which operation of a batch failed), `target` (node, property, path, candidate, snapshot or request id), `diagnostics`, `retryable`, `next_actions` and `details`.

## `retryable`

| Value | Meaning | What you do |
| --- | --- | --- |
| `none` | The same request fails again. | Change something: arguments, scope, limits or approach. Never loop. |
| `same_request` | Transient. | Retry the identical request once or twice, with a short pause. |
| `after_refresh` | Your view is stale. | Re-read (new snapshot, node versions, options, current values), rebuild the request, send it with a new `request_id`. |

`retryable` in the error is authoritative; the defaults below are only what to expect. `next_actions` lists follow-up operations, often with ready `params`: prefer them.

## Codes

| Code | Default `retryable` | `reason` values | Route |
| --- | --- | --- | --- |
| `stale_snapshot` | `after_refresh` | `base_not_head` | `changes.since` from your snapshot, `node.relocate` held ids, re-read, re-propose on the head. |
| `precondition_failed` | `after_refresh` (`candidate_applied`: `none`) | `old_value_mismatch`, `old_text_mismatch`, `node_version_mismatch`, `option_stale`, `external_write`, `candidate_version_mismatch`, `candidate_closed`, `candidate_applied`, `incomplete_graph` | Re-read the value or option (`details` may carry the current value), rebuild. For `candidate_version_mismatch` use the version the candidate now has. For `external_write`, run `changes.since`, discard, start over. |
| `insufficient_context` | `none` | | Narrow `include` or `focus`, then expand selected refs from `details.expand`; `details.required_bytes` says what everything would cost. |
| `compile_failed` | `none` | | Read `diagnostics`; fix the operation at `operation_index`. If the base does not compile, repair it first. |
| `unsupported_operation` | `none` | `protocol_unsupported`, `unknown_operation`, `not_implemented`, `edit_not_offered` | Check `capabilities` and `node.edits`. If nothing serves the change, a plain file edit is allowed after telling the user (SKILL.md, "Plain file edits"). |
| `ambiguous_owner` | `none` | `duplicate_property`, `unverified_span`, `shadowed_name`, `name_clash`, `multiple_owners` | Pick from `details.owners` and target it explicitly, or ask the user. |
| `conflict` | `none` (`concurrent_candidate`: `after_refresh`) | `request_id_reused`, `recovery_required`, `concurrent_candidate` | New `request_id`; for `recovery_required` stop and show the user the paths in `details`. |
| `resource_limit` | `none` (`queue_full`: `same_request`) | `request_too_large`, `timeout`, `content_too_large`, `nesting_limit`, `bracket_limit`, `brackets_total_limit`, `file_too_large`, `sources_too_large`, `too_many_modules`, `import_depth_limit`, `queue_full`, `budget_exhausted` | Make the request smaller or page it. Project-size limits (`file_too_large`, `sources_too_large`, `too_many_modules`, `import_depth_limit`) cannot be fixed by the agent: report them. |
| `snapshot_expired` | `after_refresh` | | The snapshot or candidate was evicted. Read again on `latest`; re-propose if a candidate was lost. |
| `read_only_origin` | `none` | `generated`, `inherited`, `dependency` | Edit the owner that produces the value, or tell the user it is not editable here. |
| `not_found` | `none` | `candidate_expired` | Re-find the node or re-propose; never guess an id. |
| `invalid_arguments` | `none` | `schema`, `limit_out_of_range`, `wrong_value_type`, `no_op`, `incompatible_state`, `sha256_mismatch`, `unknown_file`, `duplicate_path` | Fix the call against the tool's input schema. |
| `permission_denied` | `none` | `path_escape`, `root_not_allowed`, `not_owner`, `policy` | Do not work around it; tell the user. |
| `internal` | `same_request`, once | | Retry once; if it repeats, report it with the request. |

## Idempotency

- `request_id` is scoped to the workspace and caller and bound to the request's input. Repeating the same request with the same `request_id` returns the recorded result with `replayed: true`; that is safe and is how you recover from a lost response.
- The same `request_id` with different input is `conflict` (`request_id_reused`). Every new intent needs a new `request_id`.

## Apply outcomes

`change.apply` can succeed as a response and still not publish. Read `receipt.outcome`:

- `applied`: done; continue from `result_snapshot`.
- `not_applied`: nothing was written. Re-read on `latest` before deciding whether to propose again.
- `unknown`: publication may be partial (the journal is `pending_reconcile`). Send the same `change.apply` with the same `request_id`. The service returns the recorded receipt if the files match the candidate, retries if they match the base, and answers `conflict` (`recovery_required`) without writing if they match neither.
