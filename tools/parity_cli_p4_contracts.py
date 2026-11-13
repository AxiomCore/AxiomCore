"""P4 contracts caller anchors (see parity_cli_p4.py)."""

_SRC = 'cli/src/commands/contract_releases.rs'
_BASE = '/api/v1/projects/{project_slug}/contracts'

CALLERS = [
    {'command': 'axiom contract releases list', 'method': 'GET', 'path': _BASE, 'source': _SRC, 'evidence': 'cloud_cli::call(api, Method::GET, &["projects", slug, "contracts"], &[], None).await', 'command_evidence': 'Action::List'},
    {'command': 'axiom contract releases show', 'method': 'GET', 'path': _BASE + '/{version}', 'source': _SRC, 'evidence': 'cloud_cli::call(api, Method::GET, &["projects", slug, "contracts", version], &[], None).await', 'command_evidence': 'Action::Show'},
    {'command': 'axiom contract releases manifest', 'method': 'GET', 'path': _BASE + '/{version}/manifest', 'source': _SRC, 'evidence': 'cloud_cli::call(api, Method::GET, &["projects", &slug, "contracts", version, "manifest"], &[], None).await', 'command_evidence': 'Action::Manifest'},
    {'command': 'axiom contract releases verification', 'method': 'GET', 'path': _BASE + '/{version}/verification', 'source': _SRC, 'evidence': 'cloud_cli::call(api, Method::GET, &["projects", &slug, "contracts", version, "verification"], &[], None).await', 'command_evidence': 'Action::Verification'},
    {'command': 'axiom contract releases diff', 'method': 'GET', 'path': _BASE + '/{version}/diff', 'source': _SRC, 'evidence': 'cloud_cli::call(api, Method::GET, &["projects", &slug, "contracts", version, "diff"], &query, None).await', 'command_evidence': 'Action::Diff'},
    {'command': 'axiom contract releases impact', 'method': 'GET', 'path': _BASE + '/{version}/impact', 'source': _SRC, 'evidence': 'cloud_cli::call(api, Method::GET, &["projects", &slug, "contracts", version, "impact"], &query, None).await', 'command_evidence': 'Action::Impact'},
    {'command': 'axiom contract releases semantic-view', 'method': 'GET', 'path': _BASE + '/{version}/semantic-view', 'source': _SRC, 'evidence': 'cloud_cli::call(api, Method::GET, &["projects", &slug, "contracts", version, "semantic-view"], &[], None).await', 'command_evidence': 'Action::SemanticView'},
    {'command': 'axiom contract releases semantic-items', 'method': 'GET', 'path': _BASE + '/{version}/semantic-items', 'source': _SRC, 'evidence': 'cloud_cli::call(api, Method::GET, &["projects", &slug, "contracts", version, "semantic-items"], &query, None).await', 'command_evidence': 'Action::SemanticItems'},
    {'command': 'axiom contract releases backfill', 'method': 'POST', 'path': _BASE + '/{version}/semantic-backfill', 'source': _SRC, 'evidence': 'cloud_cli::call(api, Method::POST, &["projects", &slug, "contracts", version, "semantic-backfill"], &[], Some(&json!({}))).await', 'command_evidence': 'Action::Backfill'},
    {'command': 'axiom contract releases endpoints', 'method': 'GET', 'path': _BASE + '/{version}/endpoints', 'source': _SRC, 'evidence': 'cloud_cli::call(api, Method::GET, &["projects", &slug, "contracts", version, "endpoints"], &[], None).await', 'command_evidence': 'Action::Endpoints'},
    {'command': 'axiom contract releases entities', 'method': 'GET', 'path': _BASE + '/{version}/entities', 'source': _SRC, 'evidence': 'cloud_cli::call(api, Method::GET, &["projects", &slug, "contracts", version, "entities"], &[], None).await', 'command_evidence': 'Action::Entities'},
    {'command': 'axiom contract releases import', 'method': 'POST', 'path': _BASE + '/import', 'source': _SRC, 'evidence': 'cloud_cli::call(api, Method::POST, &["projects", &slug, "contracts", "import"], &[], Some(&body)).await', 'command_evidence': 'Action::Import'},
    {'command': 'axiom contract releases download', 'method': 'GET', 'path': _BASE + '/{version}/pull', 'source': _SRC, 'evidence': 'client.pull_contract_content(slug, &resolved).await', 'command_evidence': 'Action::Download'},
]
