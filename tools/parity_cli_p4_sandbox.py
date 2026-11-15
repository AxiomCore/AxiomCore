"""P4 sandbox caller anchors (see parity_cli_p4.py).

The sandbox API lives under `/sandboxes` (OD-1 rename); the path placeholder stays
`{environment_id}` as in the contract. `/environments/{environment_id}` is reserved
for configuration environments (deployments; today it answers AXSANDBOX_MOVED), which
these commands never call.
"""

_SRC = 'cli/src/commands/sandbox_environments.rs'
_BASE = '/api/v1/projects/{project}/sandboxes'

CALLERS = [
    {'command': 'axiom project sandboxes list', 'method': 'GET', 'path': _BASE, 'source': _SRC, 'evidence': 'call(api, Method::GET, &["projects", slug, "sandboxes"], &[], None).await', 'command_evidence': 'Action::List =>'},
    {'command': 'axiom project sandboxes create', 'method': 'POST', 'path': _BASE, 'source': _SRC, 'evidence': 'call(api, Method::POST, &["projects", slug, "sandboxes"], &[], Some(&body)).await?', 'command_evidence': 'Action::Create {'},
    {'command': 'axiom project sandboxes show', 'method': 'GET', 'path': _BASE + '/{environment_id}', 'source': _SRC, 'evidence': 'call(api, Method::GET, &["projects", slug, "sandboxes", id], &[], None).await', 'command_evidence': 'Action::Show {'},
    {'command': 'axiom project sandboxes events', 'method': 'GET', 'path': _BASE + '/{environment_id}/events', 'source': _SRC, 'evidence': 'call(api, Method::GET, &["projects", slug, "sandboxes", &id, "events"], &query, None).await?', 'command_evidence': 'Action::Events {'},
    {'command': 'axiom project sandboxes connection', 'method': 'POST', 'path': _BASE + '/{environment_id}/connection', 'source': _SRC, 'evidence': 'call(api, Method::POST, &["projects", slug, "sandboxes", &id, "connection"], &[], None).await?', 'command_evidence': 'Action::Connection {'},
    {'command': 'axiom project sandboxes reset', 'method': 'POST', 'path': _BASE + '/{environment_id}/reset', 'source': _SRC, 'evidence': 'call(api, Method::POST, &["projects", slug, "sandboxes", &id, "reset"], &[], None).await?', 'command_evidence': 'Action::Reset {'},
    {'command': 'axiom project sandboxes delete', 'method': 'DELETE', 'path': _BASE + '/{environment_id}', 'source': _SRC, 'evidence': 'call(api, Method::DELETE, &["projects", slug, "sandboxes", &id], &[], None).await?', 'command_evidence': 'Action::Delete {'},
]
