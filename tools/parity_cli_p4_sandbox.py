"""P4 sandbox caller anchors (see parity_cli_p4.py)."""

_SRC = 'cli/src/commands/sandbox_environments.rs'
_BASE = '/api/v1/projects/{project}/environments'

CALLERS = [
    {'command': 'axiom project environments list', 'method': 'GET', 'path': _BASE, 'source': _SRC, 'evidence': 'cloud_cli::call(api, Method::GET, &["projects", slug, "environments"], &[], None).await', 'command_evidence': 'Action::List =>'},
    {'command': 'axiom project environments create', 'method': 'POST', 'path': _BASE, 'source': _SRC, 'evidence': 'cloud_cli::call(api, Method::POST, &["projects", slug, "environments"], &[], Some(&body)).await?', 'command_evidence': 'Action::Create {'},
    {'command': 'axiom project environments events', 'method': 'GET', 'path': _BASE + '/{environment_id}/events', 'source': _SRC, 'evidence': 'cloud_cli::call(api, Method::GET, &["projects", slug, "environments", &id, "events"], &query, None).await?', 'command_evidence': 'Action::Events {'},
    {'command': 'axiom project environments connection', 'method': 'POST', 'path': _BASE + '/{environment_id}/connection', 'source': _SRC, 'evidence': 'cloud_cli::call(api, Method::POST, &["projects", slug, "environments", &id, "connection"], &[], Some(&json!({}))).await?', 'command_evidence': 'Action::Connection {'},
    {'command': 'axiom project environments reset', 'method': 'POST', 'path': _BASE + '/{environment_id}/reset', 'source': _SRC, 'evidence': 'cloud_cli::call(api, Method::POST, &["projects", slug, "environments", &id, "reset"], &[], Some(&json!({}))).await?', 'command_evidence': 'Action::Reset {'},
    {'command': 'axiom project environments delete', 'method': 'DELETE', 'path': _BASE + '/{environment_id}', 'source': _SRC, 'evidence': 'cloud_cli::call(api, Method::DELETE, &["projects", slug, "environments", &id], &[], None).await?', 'command_evidence': 'Action::Delete {'},
]
