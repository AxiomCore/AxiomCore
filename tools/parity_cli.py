"""Reviewed CLI caller map for the dashboard parity audit.

This is deliberately conservative: indirect CloudClient calls need a reviewed
mapping. Entries count only while the production call and dispatch anchors
remain in cli/src. Changes to a wrapper require review of its route mapping.
This is endpoint coverage, not a claim that every payload or output is exposed.
Tests and comments cannot establish coverage. Keep notes for partial workflows.
"""
from pathlib import Path
import re

CALLERS = [
    {'command': 'axiom database convert submit', 'method': 'POST', 'path': '/api/v1/projects/{project}/resources/{id}/conversions', 'source': 'cli/src/commands/database_convert.rs', 'evidence': 'client.resource_request(Method::POST, &["projects", project, "resources", resource, "conversions"], Some(&body), Some(&key)).await?', 'command_evidence': 'Action::Submit'},
    {'command': 'axiom database convert submit --wait', 'method': 'GET', 'path': '/api/v1/projects/{project}/resources/{id}/conversions/{id}', 'source': 'cli/src/commands/database_convert.rs', 'evidence': 'client.resource_request(Method::GET, &["projects", project, "resources", resource, "conversions", &id], None, None).await?', 'command_evidence': 'Action::Submit'},
    {'command': 'axiom project resources receipt verify', 'method': 'GET', 'path': '/api/v1/resources/receipt-keys', 'source': 'cli/src/commands/resources_placement.rs', 'evidence': 'api.get(&["resources", "receipt-keys"]).await?', 'command_evidence': 'ReceiptAction::Verify'},
    {'command': 'axiom auth policy release', 'method': 'GET', 'path': '/api/v1/projects/{project}/auth/policy/releases', 'source': 'cli/src/commands/auth/policy.rs', 'evidence': 'target.client.project_auth_request(Method::GET, &target.slug, "auth/policy/releases", None, None).await', 'command_evidence': 'async fn release(', 'note': 'Supporting lookup only; no standalone list/show command.'},
    {'command': 'axiom auth invite', 'method': 'GET', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/roles', 'source': 'cli/src/commands/auth.rs', 'evidence': 'target.call(Method::GET, "roles", &[], None).await?', 'command_evidence': 'Action::Invite', 'note': 'Supporting lookup only; no standalone list/show command.'},
    {'command': 'axiom release', 'method': 'GET', 'path': '/api/v1/projects/{project}/contracts', 'source': 'cli/src/commands/release.rs', 'evidence': 'client.list_contracts(project_slug).await', 'command_evidence': 'pub async fn handle_release(', 'note': 'Supporting lookup only; no standalone list/show command.'},
    {'command': 'axiom release', 'method': 'GET', 'path': '/api/v1/projects/{project}/contracts/{version}', 'source': 'cli/src/commands/release.rs', 'evidence': '.get_contract_metadata(project_slug, version)', 'command_evidence': 'pub async fn handle_release(', 'note': 'Supporting lookup only; no standalone list/show command.'},
    {'command': 'axiom project signers add', 'method': 'GET', 'path': '/api/v1/projects/{project}', 'source': 'cli/src/commands/signers.rs', 'evidence': 'client.project_id(project).await?', 'command_evidence': 'Action::Add', 'note': 'Supporting lookup only; no standalone list/show command.'},
    {'command': 'axiom auth policy release', 'method': 'GET', 'path': '/api/v1/projects/{project}', 'source': 'cli/src/commands/auth/policy.rs', 'evidence': 'target.client.project_id(&target.slug).await', 'command_evidence': 'async fn release(', 'note': 'Supporting lookup only; no standalone list/show command.'},
    {'command': 'axiom database convert submit', 'method': 'GET', 'path': '/api/v1/projects/{project}', 'source': 'cli/src/commands/database_convert.rs', 'evidence': 'h.client.project_id(h.project).await?', 'command_evidence': 'Action::Submit', 'note': 'Supporting lookup only; no standalone list/show command.'},

    {'command': 'axiom auth status', 'method': 'GET', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/overview', 'source': 'cli/src/commands/auth.rs', 'evidence': 'let mut overview = target.call(Method::GET, "overview", &[], None).await?;', 'command_evidence': 'Action::Status'},
    {'command': 'axiom auth settings get', 'method': 'GET', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/settings', 'source': 'cli/src/commands/auth.rs', 'evidence': 'let mut settings = target.call(Method::GET, "settings", &[], None).await?;', 'command_evidence': 'SettingsAction::Get'},
    {'command': 'axiom auth settings set', 'method': 'PUT', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/settings', 'source': 'cli/src/commands/auth.rs', 'evidence': 'let mut settings = target.call(Method::PUT, "settings", &[], Some(&body)).await?;', 'command_evidence': 'SettingsAction::Set'},
    {'command': 'axiom auth invite', 'method': 'POST', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/invitations', 'source': 'cli/src/commands/auth.rs', 'evidence': 'let mut invitation = target.call(Method::POST, "invitations", &[], Some(&body)).await?;', 'command_evidence': 'Action::Invite'},
    {'command': 'axiom auth providers list', 'method': 'GET', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/providers', 'source': 'cli/src/commands/auth.rs', 'evidence': 'let mut listed = target.call(Method::GET, "providers", &[], None).await?;', 'command_evidence': 'ProvidersAction::List'},
    {'command': 'axiom auth providers set', 'method': 'POST', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/providers', 'source': 'cli/src/commands/auth.rs', 'evidence': 'let result = target.call(Method::POST, "providers", &[], Some(&body)).await;', 'command_evidence': 'ProvidersAction::Set'},
    {'command': 'axiom auth providers set', 'method': 'PATCH', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/providers/{id}', 'source': 'cli/src/commands/auth.rs', 'evidence': 'let result = target.call(Method::PATCH, &format!("providers/{id}"), &[], Some(&body)).await;', 'command_evidence': 'ProvidersAction::Set'},
    {'command': 'axiom auth providers test', 'method': 'POST', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/providers/{id}/test', 'source': 'cli/src/commands/auth.rs', 'evidence': 'let mut result = target.call(Method::POST, &format!("providers/{id}/test"), &[], Some(&json!({}))).await?;', 'command_evidence': 'ProvidersAction::Test'},
    {'command': 'axiom auth users list', 'method': 'GET', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/users', 'source': 'cli/src/commands/auth.rs', 'evidence': 'target.call(Method::GET, "users", &query, None).await?', 'command_evidence': 'UsersAction::List'},
    {'command': 'axiom auth users list --search', 'method': 'POST', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/users/search', 'source': 'cli/src/commands/auth.rs', 'evidence': 'target.call(Method::POST, "users/search", &[], Some(&body)).await?', 'command_evidence': 'UsersAction::List'},
    {'command': 'axiom auth users show', 'method': 'GET', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/users/{id}', 'source': 'cli/src/commands/auth.rs', 'evidence': 'let mut user = target.call(Method::GET, &format!("users/{}", checked_id(user_id)?), &[], None).await?;', 'command_evidence': 'UsersAction::Show'},
    {'command': 'axiom auth users ban', 'method': 'POST', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/users/{id}/ban', 'source': 'cli/src/commands/auth.rs', 'evidence': 'let mut user = target.call(Method::POST, &format!("users/{}/ban", checked_id(user_id)?), &[], Some(&json!({"reason": reason}))).await?;', 'command_evidence': 'UsersAction::Ban'},
    {'command': 'axiom auth users unban', 'method': 'POST', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/users/{id}/unban', 'source': 'cli/src/commands/auth.rs', 'evidence': 'let mut user = target.call(Method::POST, &format!("users/{}/unban", checked_id(user_id)?), &[], Some(&json!({}))).await?;', 'command_evidence': 'UsersAction::Unban'},
    {'command': 'axiom auth users revoke', 'method': 'POST', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/users/{id}/sessions/revoke', 'source': 'cli/src/commands/auth.rs', 'evidence': 'let ended = target.call(Method::POST, &format!("users/{user_id}/sessions/revoke"), &[], Some(&body)).await?;', 'command_evidence': 'UsersAction::Revoke'},
    {'command': 'axiom auth jwks --check', 'method': 'GET', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/keys', 'source': 'cli/src/commands/auth.rs', 'evidence': 'if let Ok(mut keys) = target.call(Method::GET, "keys", &[], None).await {', 'command_evidence': 'Action::Jwks'},
    {'command': 'axiom auth oauth-apps list', 'method': 'GET', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/oauth-clients', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'let mut listed = target.call(Method::GET, "oauth-clients", &[], None).await?;', 'command_evidence': 'OauthAppsAction::List'},
    {'command': 'axiom auth oauth-apps show', 'method': 'GET', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/oauth-clients/{id}', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'let mut client = target.call(Method::GET, &format!("oauth-clients/{}", checked_id(id)?), &[], None).await?;', 'command_evidence': 'OauthAppsAction::Show'},
    {'command': 'axiom auth oauth-apps create', 'method': 'POST', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/oauth-clients', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'let mut created = target.keyed(Method::POST, "oauth-clients", &body, &key).await?;', 'command_evidence': 'OauthAppsAction::Create'},
    {'command': 'axiom auth oauth-apps update', 'method': 'PATCH', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/oauth-clients/{id}', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'let mut saved = target.call(Method::PATCH, &format!("oauth-clients/{id}"), &[], Some(&body)).await?;', 'command_evidence': 'OauthAppsAction::Update'},
    {'command': 'axiom auth oauth-apps rotate-secret', 'method': 'POST', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/oauth-clients/{id}/secret', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'let mut rotated = target.call(Method::POST, &format!("oauth-clients/{}/secret", checked_id(id)?), &[], Some(&json!({"previous_valid_seconds": seconds}))).await?;', 'command_evidence': 'OauthAppsAction::RotateSecret'},
    {'command': 'axiom auth oauth-apps disable', 'method': 'PATCH', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/oauth-clients/{id}', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'let mut saved = target.call(Method::PATCH, &format!("oauth-clients/{id}"), &[], Some(&json!({"enabled": false, "revision": revision}))).await?;', 'command_evidence': 'OauthAppsAction::Disable'},
    {'command': 'axiom auth oauth-apps delete', 'method': 'DELETE', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/oauth-clients/{id}', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'target.call(Method::DELETE, &format!("oauth-clients/{id}"), &[], None).await?;', 'command_evidence': 'OauthAppsAction::Delete'},
    {'command': 'axiom auth trusted-issuers list', 'method': 'GET', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/trusted-issuers', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'let mut listed = target.call(Method::GET, "trusted-issuers", &[], None).await?;', 'command_evidence': 'TrustedIssuersAction::List'},
    {'command': 'axiom auth trusted-issuers add', 'method': 'POST', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/trusted-issuers', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'let mut saved = target.call(Method::POST, "trusted-issuers", &[], Some(&body)).await?;', 'command_evidence': 'TrustedIssuersAction::Add'},
    {'command': 'axiom auth trusted-issuers test', 'method': 'POST', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/trusted-issuers/{id}/test', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'let result = target.call(Method::POST, &format!("trusted-issuers/{}/test", checked_id(id)?), &[], Some(&body)).await;', 'command_evidence': 'TrustedIssuersAction::Test'},
    {'command': 'axiom auth mfa status', 'method': 'GET', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/mfa', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'let mut status = target.call(Method::GET, "mfa", &[], None).await?;', 'command_evidence': 'async fn mfa('},
    {'command': 'axiom auth users factors', 'method': 'GET', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/users/{id}/factors', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'let mut factors = target.call(Method::GET, &format!("users/{}/factors", checked_id(user_id)?), &[], None).await?;', 'command_evidence': 'async fn user_factors('},
    {'command': 'axiom auth users reset-factors', 'method': 'POST', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/users/{id}/factors/reset', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'let result = target.call(Method::POST, &format!("users/{user_id}/factors/reset"), &[], Some(&json!({"reason": reason}))).await?;', 'command_evidence': 'async fn reset_factors('},
    {'command': 'axiom auth users consents', 'method': 'GET', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/users/{id}/consents', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'let mut consents = target.call(Method::GET, &format!("users/{}/consents", checked_id(user_id)?), &[], None).await?;', 'command_evidence': 'async fn user_consents('},
    {'command': 'axiom auth users revoke-consent', 'method': 'DELETE', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/users/{id}/consents/{id}', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'target.call(Method::DELETE, &format!("users/{user_id}/consents/{consent_id}"), &[], None).await?;', 'command_evidence': 'async fn revoke_consent('},
    {'command': 'axiom auth webhooks list', 'method': 'GET', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/webhooks', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'let mut listed = target.call(Method::GET, "webhooks", &[], None).await?;', 'command_evidence': 'WebhooksAction::List'},
    {'command': 'axiom auth webhooks create', 'method': 'POST', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/webhooks', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'let mut created = target.keyed(Method::POST, "webhooks", &json!({"url": url, "event_types": events, "enabled": enable}), &key).await?;', 'command_evidence': 'WebhooksAction::Create'},
    {'command': 'axiom auth webhooks update', 'method': 'PATCH', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/webhooks/{id}', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'let mut saved = target.call(Method::PATCH, &format!("webhooks/{id}"), &[], Some(&body)).await?;', 'command_evidence': 'WebhooksAction::Update'},
    {'command': 'axiom auth webhooks rotate-secret', 'method': 'POST', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/webhooks/{id}/secret', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'let mut rotated = target.call(Method::POST, &format!("webhooks/{}/secret", checked_id(id)?), &[], Some(&json!({}))).await?;', 'command_evidence': 'WebhooksAction::RotateSecret'},
    {'command': 'axiom auth webhooks test', 'method': 'POST', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/webhooks/{id}/test', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'let result = target.call(Method::POST, &format!("webhooks/{}/test", checked_id(id)?), &[], Some(&json!({}))).await?;', 'command_evidence': 'WebhooksAction::Test'},
    {'command': 'axiom auth webhooks deliveries', 'method': 'GET', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/webhooks/{id}/deliveries', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'let mut listed = target.call(Method::GET, &format!("webhooks/{}/deliveries", checked_id(id)?), &query, None).await?;', 'command_evidence': 'WebhooksAction::Deliveries'},
    {'command': 'axiom auth webhooks replay', 'method': 'POST', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/webhooks/{id}/deliveries/{id}/replay', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'let result = target.call(Method::POST, &format!("webhooks/{}/deliveries/{}/replay", checked_id(id)?, checked_id(delivery_id)?), &[], Some(&json!({}))).await?;', 'command_evidence': 'WebhooksAction::Replay'},
    {'command': 'axiom auth webhooks delete', 'method': 'DELETE', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/webhooks/{id}', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'target.call(Method::DELETE, &format!("webhooks/{id}"), &[], None).await?;', 'command_evidence': 'WebhooksAction::Delete'},
    {'command': 'axiom auth hooks get', 'method': 'GET', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/hooks', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'let mut listed = target.call(Method::GET, "hooks", &[], None).await?;', 'command_evidence': 'HooksAction::Get'},
    {'command': 'axiom auth hooks set', 'method': 'PUT', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/hooks/{kind}', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'let mut saved = target.call(Method::PUT, &format!("hooks/{kind}"), &[], Some(&body)).await?;', 'command_evidence': 'HooksAction::Set'},
    {'command': 'axiom auth hooks test', 'method': 'POST', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/hooks/{kind}/test', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'let result = target.call(Method::POST, &format!("hooks/{kind}/test"), &[], Some(&json!({"sample": sample}))).await?;', 'command_evidence': 'HooksAction::Test'},
    {'command': 'axiom auth hooks rotate-secret', 'method': 'POST', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/hooks/{kind}/secret', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'let mut rotated = target.call(Method::POST, &format!("hooks/{kind}/secret"), &[], Some(&json!({}))).await?;', 'command_evidence': 'HooksAction::RotateSecret'},
    {'command': 'axiom auth hooks remove', 'method': 'DELETE', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/hooks/{kind}', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'target.call(Method::DELETE, &format!("hooks/{kind}"), &[], None).await?;', 'command_evidence': 'HooksAction::Remove'},
    {'command': 'axiom auth sms set', 'method': 'PUT', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/sms', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'let result = target.call(Method::PUT, "sms", &[], Some(&body)).await;', 'command_evidence': 'SmsAction::Set'},
    {'command': 'axiom auth sms get', 'method': 'GET', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/sms', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'let mut settings = target.call(Method::GET, "sms", &[], None).await?;', 'command_evidence': 'SmsAction::Get'},
    {'command': 'axiom auth sms test', 'method': 'POST', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/sms/test', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'let result = target.call(Method::POST, "sms/test", &[], Some(&json!({"to": to}))).await?;', 'command_evidence': 'SmsAction::Test'},
    {'command': 'axiom auth sms remove', 'method': 'DELETE', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/sms', 'source': 'cli/src/commands/auth/advanced.rs', 'evidence': 'target.call(Method::DELETE, "sms", &[], None).await?;', 'command_evidence': 'SmsAction::Remove'},
    {'command': 'axiom auth policy pull', 'method': 'GET', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/policy', 'source': 'cli/src/commands/auth/policy.rs', 'evidence': 'let view = target.call(Method::GET, "policy", &[], None).await?;', 'command_evidence': 'PolicyAction::Pull'},
    {'command': 'axiom auth policy plan', 'method': 'POST', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/policy/plan', 'source': 'cli/src/commands/auth/policy.rs', 'evidence': 'let plan = target.call(Method::POST, "policy/plan", &[], Some(&json!({"release": release}))).await?;', 'command_evidence': 'PolicyAction::Plan'},
    {'command': 'axiom auth policy apply', 'method': 'POST', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/policy/apply', 'source': 'cli/src/commands/auth/policy.rs', 'evidence': 'let applied = target.call(Method::POST, "policy/apply", &[], Some(&body)).await?;', 'command_evidence': 'PolicyAction::Apply'},
    {'command': 'axiom auth config promote', 'method': 'DELETE', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/providers/{id}', 'source': 'cli/src/commands/auth/config.rs', 'evidence': 'target.call(Method::DELETE, &format!("providers/{}", p["id"].as_str().unwrap_or_default()), &[], None).await?;', 'note': 'Bulk promotion only; requires a source environment and diff acceptance, not a direct per-object administration command.', 'command_evidence': 'pub async fn run(args: &AuthArgs, command: &ConfigCommand)'},
    {'command': 'axiom auth config promote', 'method': 'DELETE', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/trusted-issuers/{id}', 'source': 'cli/src/commands/auth/config.rs', 'evidence': 'target.call(Method::DELETE, &format!("trusted-issuers/{}", x["id"].as_str().unwrap_or_default()), &[], None).await?;', 'note': 'Bulk promotion only; requires a source environment and diff acceptance, not a direct per-object administration command.', 'command_evidence': 'pub async fn run(args: &AuthArgs, command: &ConfigCommand)'},
    {'command': 'axiom auth config promote', 'method': 'PATCH', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/trusted-issuers/{id}', 'source': 'cli/src/commands/auth/config.rs', 'evidence': 'target.call(Method::PATCH, &format!("trusted-issuers/{}", x["id"].as_str().unwrap_or_default()), &[], Some(&body)).await?;', 'note': 'Bulk promotion only; requires a source environment and diff acceptance, not a direct per-object administration command.', 'command_evidence': 'pub async fn run(args: &AuthArgs, command: &ConfigCommand)'},
    {'command': 'axiom branch list', 'method': 'GET', 'path': '/api/v1/projects/{project}/branches', 'source': 'cli/src/commands/branch.rs', 'evidence': 'let listed = api.get_query(&["projects", p, "branches"], &query).await?;', 'command_evidence': 'BranchAction::List'},
    {'command': 'axiom branch create', 'method': 'POST', 'path': '/api/v1/projects/{project}/branches', 'source': 'cli/src/commands/branch.rs', 'evidence': 'let mut created = api.step_up_action(&Method::POST, &["projects", p, "branches"], &body, &key).await?;', 'command_evidence': 'BranchAction::Create'},
    {'command': 'axiom branch show', 'method': 'GET', 'path': '/api/v1/projects/{project}/branches/{id}', 'source': 'cli/src/commands/branch.rs', 'evidence': 'let branch = api.get(&["projects", p, "branches", id]).await?;', 'command_evidence': 'BranchAction::Show'},
    {'command': 'axiom branch extend', 'method': 'POST', 'path': '/api/v1/projects/{project}/branches/{id}/extend', 'source': 'cli/src/commands/branch.rs', 'evidence': 'let branch = api.write(&Method::POST, &["projects", p, "branches", id, "extend"], &json!({"expires_at": at}), Some(&key)).await?;', 'command_evidence': 'BranchAction::Extend'},
    {'command': 'axiom branch delete', 'method': 'POST', 'path': '/api/v1/projects/{project}/branches/{id}/delete', 'source': 'cli/src/commands/branch.rs', 'evidence': 'let deleted = api.write(&Method::POST, &["projects", p, "branches", id, "delete"], &json!({"confirm": phrase}), Some(&key)).await?;', 'command_evidence': 'BranchAction::Delete'},
    {'command': 'axiom branch policy get', 'method': 'GET', 'path': '/api/v1/projects/{project}/branches/policy', 'source': 'cli/src/commands/branch.rs', 'evidence': 'let policy = api.get_query(&["projects", p, "branches", "policy"], &[("environment", environment.clone())]).await?;', 'command_evidence': 'PolicyAction::Get'},
    {'command': 'axiom branch policy set', 'method': 'PUT', 'path': '/api/v1/projects/{project}/branches/policy', 'source': 'cli/src/commands/branch.rs', 'evidence': 'let saved = api.step_up_action_query(&Method::PUT, &["projects", p, "branches", "policy"], &[("environment", environment.clone())], &body, &key).await?;', 'command_evidence': 'PolicyAction::Set'},
    {'command': 'axiom branch settings get', 'method': 'GET', 'path': '/api/v1/projects/{project}/branches/settings', 'source': 'cli/src/commands/branch.rs', 'evidence': 'let settings = api.get_query(&["projects", p, "branches", "settings"], &[("environment", environment.clone())]).await?;', 'command_evidence': 'SettingsAction::Get'},
    {'command': 'axiom branch settings set', 'method': 'PUT', 'path': '/api/v1/projects/{project}/branches/settings', 'source': 'cli/src/commands/branch.rs', 'evidence': 'let saved = api.session_action_query(&Method::PUT, &["projects", p, "branches", "settings"], &[("environment", environment.clone())], &body, &key).await?;', 'command_evidence': 'SettingsAction::Set'},
    {'command': 'axiom branch sources', 'method': 'GET', 'path': '/api/v1/projects/{project}/branches/sources', 'source': 'cli/src/commands/branch.rs', 'evidence': 'let sources = api.get_query(&["projects", p, "branches", "sources"], &[("environment", environment.clone())]).await?;', 'command_evidence': 'BranchAction::Sources'},
    {'command': 'axiom branch review create', 'method': 'POST', 'path': '/api/v1/projects/{project}/branches/{id}/reviews', 'source': 'cli/src/commands/branch.rs', 'evidence': 'let review = api.step_up_action(&Method::POST, &["projects", p, "branches", branch, "reviews"], &body, &key).await?;', 'command_evidence': 'ReviewAction::Create'},
    {'command': 'axiom branch review list', 'method': 'GET', 'path': '/api/v1/projects/{project}/branches/reviews', 'source': 'cli/src/commands/branch.rs', 'evidence': 'let reviews = api.get_query(&["projects", p, "branches", "reviews"], &[("environment", environment.clone())]).await?;', 'command_evidence': 'ReviewAction::List'},
    {'command': 'axiom branch review revoke', 'method': 'POST', 'path': '/api/v1/projects/{project}/branches/reviews/{id}/revoke', 'source': 'cli/src/commands/branch.rs', 'evidence': 'let revoked = api.write(&Method::POST, &["projects", p, "branches", "reviews", review, "revoke"], &json!({}), Some(&key)).await?;', 'command_evidence': 'ReviewAction::Revoke'},
    {'command': 'axiom branch mail list', 'method': 'GET', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/mail', 'source': 'cli/src/commands/branch.rs', 'evidence': 'let mail = admin(&client, p, &environment, Method::GET, &rest, &query, None, None).await?;', 'command_evidence': 'MailAction::List'},
    {'command': 'axiom branch mail show', 'method': 'GET', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/mail/{id}', 'source': 'cli/src/commands/branch.rs', 'evidence': 'let mail = admin(&client, p, &environment, Method::GET, &rest, &query, None, None).await?;', 'command_evidence': 'MailAction::Show'},
    {'command': 'axiom branch test-users', 'method': 'POST', 'path': '/api/v1/projects/{project}/auth/environments/{environment}/test-users', 'source': 'cli/src/commands/branch.rs', 'evidence': 'let created = admin(&client, p, &environment, Method::POST, "test-users", &[], Some(&body), Some(&key)).await?;', 'command_evidence': 'BranchAction::TestUsers'},
    {'command': 'axiom project resources', 'method': 'GET', 'path': '/api/v1/projects/{project}/resources', 'source': 'cli/src/commands/resources.rs', 'evidence': '.resource_request(Method::GET, &base, None, None)', 'command_evidence': 'let base = ["projects", project, "resources"];'},
    {'command': 'axiom project resources checkout', 'method': 'POST', 'path': '/api/v1/projects/{project}/resources/checkout', 'source': 'cli/src/commands/resources.rs', 'evidence': '.resource_request(\n                    Method::POST,\n                    &["projects", project, "resources", "checkout"],\n                    Some(&json!({})),\n                    Some(&key),\n                )\n                .await', 'command_evidence': 'Action::Checkout'},
    {'command': 'axiom project resources billing-portal', 'method': 'POST', 'path': '/api/v1/projects/{project}/resources/billing-portal', 'source': 'cli/src/commands/resources.rs', 'evidence': '.resource_request(\n                    Method::POST,\n                    &["projects", project, "resources", "billing-portal"],\n                    Some(&json!({})),\n                    None,\n                )\n                .await', 'command_evidence': 'Action::BillingPortal'},
    {'command': 'axiom project resources configure', 'method': 'POST', 'path': '/api/v1/projects/{project}/resources/configuration', 'source': 'cli/src/commands/resources.rs', 'evidence': '.resource_request(\n                    Method::POST,\n                    &["projects", project, "resources", "configuration"],\n                    Some(&serde_json::to_value(selection)?),\n                    None,\n                )\n                .await', 'command_evidence': 'Action::Configure'},
    {'command': 'axiom project resources catalog', 'method': 'GET', 'path': '/api/v1/resource-catalog', 'source': 'cli/src/commands/resources.rs', 'evidence': '.resource_request(Method::GET, &["resource-catalog"], None, None)', 'command_evidence': 'Action::Catalog'},
    {'command': 'axiom project resources request --action provision', 'method': 'POST', 'path': '/api/v1/projects/{project}/resources/{resource}/provision', 'source': 'cli/src/commands/resources.rs', 'evidence': '.resource_request(\n                    Method::POST,\n                    &["projects", project, "resources", resource, action],\n                    Some(&json!({"generation":generation,"confirm":confirm})),\n                    Some(&key),\n                )\n                .await', 'command_evidence': 'Action::Request'},
    {'command': 'axiom project resources request --action retry', 'method': 'POST', 'path': '/api/v1/projects/{project}/resources/{resource}/retry', 'source': 'cli/src/commands/resources.rs', 'evidence': '.resource_request(\n                    Method::POST,\n                    &["projects", project, "resources", resource, action],\n                    Some(&json!({"generation":generation,"confirm":confirm})),\n                    Some(&key),\n                )\n                .await', 'command_evidence': 'Action::Request'},
    {'command': 'axiom project resources request --action suspend', 'method': 'POST', 'path': '/api/v1/projects/{project}/resources/{resource}/suspend', 'source': 'cli/src/commands/resources.rs', 'evidence': '.resource_request(\n                    Method::POST,\n                    &["projects", project, "resources", resource, action],\n                    Some(&json!({"generation":generation,"confirm":confirm})),\n                    Some(&key),\n                )\n                .await', 'command_evidence': 'Action::Request'},
    {'command': 'axiom project resources request --action resume', 'method': 'POST', 'path': '/api/v1/projects/{project}/resources/{resource}/resume', 'source': 'cli/src/commands/resources.rs', 'evidence': '.resource_request(\n                    Method::POST,\n                    &["projects", project, "resources", resource, action],\n                    Some(&json!({"generation":generation,"confirm":confirm})),\n                    Some(&key),\n                )\n                .await', 'command_evidence': 'Action::Request'},
    {'command': 'axiom project resources request --action delete', 'method': 'POST', 'path': '/api/v1/projects/{project}/resources/{resource}/delete', 'source': 'cli/src/commands/resources.rs', 'evidence': '.resource_request(\n                    Method::POST,\n                    &["projects", project, "resources", resource, action],\n                    Some(&json!({"generation":generation,"confirm":confirm})),\n                    Some(&key),\n                )\n                .await', 'command_evidence': 'Action::Request'},
    {'command': 'axiom project resources request --action rotate', 'method': 'POST', 'path': '/api/v1/projects/{project}/resources/{resource}/rotate', 'source': 'cli/src/commands/resources.rs', 'evidence': '.resource_request(\n                    Method::POST,\n                    &["projects", project, "resources", resource, action],\n                    Some(&json!({"generation":generation,"confirm":confirm})),\n                    Some(&key),\n                )\n                .await', 'command_evidence': 'Action::Request'},
    {'command': 'axiom project resources request --action restore', 'method': 'POST', 'path': '/api/v1/projects/{project}/resources/{resource}/restore', 'source': 'cli/src/commands/resources_recovery.rs', 'evidence': '.post_action(\n            &[\n                "projects",\n                project,\n                "resources",\n                input.resource,\n                input.action,\n            ],\n            &body,\n            &key,\n        )\n        .await', 'command_evidence': 'pub async fn run_action('},
    {'command': 'axiom project resources request --action archive', 'method': 'POST', 'path': '/api/v1/projects/{project}/resources/{resource}/archive', 'source': 'cli/src/commands/resources_recovery.rs', 'evidence': '.post_action(\n            &[\n                "projects",\n                project,\n                "resources",\n                input.resource,\n                input.action,\n            ],\n            &body,\n            &key,\n        )\n        .await', 'command_evidence': 'pub async fn run_action('},
    {'command': 'axiom project resources request --action unarchive', 'method': 'POST', 'path': '/api/v1/projects/{project}/resources/{resource}/unarchive', 'source': 'cli/src/commands/resources_recovery.rs', 'evidence': '.post_action(\n            &[\n                "projects",\n                project,\n                "resources",\n                input.resource,\n                input.action,\n            ],\n            &body,\n            &key,\n        )\n        .await', 'command_evidence': 'pub async fn run_action('},
    {'command': 'axiom project resources request --action retire_previous', 'method': 'POST', 'path': '/api/v1/projects/{project}/resources/{resource}/retire_previous', 'source': 'cli/src/commands/resources_recovery.rs', 'evidence': '.post_action(\n            &[\n                "projects",\n                project,\n                "resources",\n                input.resource,\n                input.action,\n            ],\n            &body,\n            &key,\n        )\n        .await', 'command_evidence': 'pub async fn run_action('},
    {'command': 'axiom project resources recovery', 'method': 'GET', 'path': '/api/v1/projects/{project}/resources/{resource}/recovery', 'source': 'cli/src/commands/resources_recovery.rs', 'evidence': 'api.get(&["projects", project, "resources", resource, "recovery"])', 'command_evidence': 'pub async fn handle_view('},
    {'command': 'axiom project resources connect', 'method': 'PUT', 'path': '/api/v1/projects/{project}/resources/{resource}/connection', 'source': 'cli/src/commands/resources_placement.rs', 'evidence': 'let result = api.step_up_action(&Method::PUT, &["projects", project, "resources", &id, "connection"], &body, &key).await;', 'command_evidence': 'pub async fn handle_connect('},
    {'command': 'axiom project resources connection', 'method': 'GET', 'path': '/api/v1/projects/{project}/resources/{resource}/connection', 'source': 'cli/src/commands/resources_placement.rs', 'evidence': 'let view = api.get(&["projects", project, "resources", &resource_id(&resource)?, "connection"]).await?;', 'command_evidence': 'pub async fn handle_connection('},
    {'command': 'axiom project resources connection check', 'method': 'POST', 'path': '/api/v1/projects/{project}/resources/{resource}/connection/check', 'source': 'cli/src/commands/resources_placement.rs', 'evidence': 'let value = api.write(&Method::POST, &["projects", project, "resources", &resource_id(&resource)?, "connection", "check"], &json!({}), Some(&key)).await?;', 'command_evidence': 'pub async fn handle_connection('},
    {'command': 'axiom project resources disconnect', 'method': 'POST', 'path': '/api/v1/projects/{project}/resources/{resource}/detach', 'source': 'cli/src/commands/resources_placement.rs', 'evidence': 'let value = api.step_up_action(&Method::POST, &["projects", project, "resources", &id, "detach"], &body, &key).await?;', 'command_evidence': 'pub async fn handle_disconnect('},
    {'command': 'axiom project resources placement', 'method': 'GET', 'path': '/api/v1/projects/{project}/resources/{resource}/placement', 'source': 'cli/src/commands/resources_placement.rs', 'evidence': 'let view = api.get(&["projects", project, "resources", &resource_id(&resource)?, "placement"]).await?;', 'command_evidence': 'pub async fn handle_placement('},
    {'command': 'axiom project resources relocation-consent', 'method': 'POST', 'path': '/api/v1/projects/{project}/resources/{resource}/relocation-consents', 'source': 'cli/src/commands/resources_placement.rs', 'evidence': 'let value = api.step_up_action(&Method::POST, &["projects", project, "resources", &resource_id(&resource)?, "relocation-consents"], &body, &key).await?;', 'command_evidence': 'pub async fn handle_consent('},
    {'command': 'axiom project resources request --action convert', 'method': 'POST', 'path': '/api/v1/projects/{project}/resources/{resource}/convert', 'source': 'cli/src/commands/resources_placement.rs', 'evidence': 'let value = api.step_up_action(&Method::POST, &["projects", project, "resources", resource, "convert"], &body, &key).await?;', 'command_evidence': 'pub async fn handle_convert('},
    {'command': 'axiom secrets create-environment', 'method': 'POST', 'path': '/api/v1/projects/{project}/configuration-environments', 'source': 'cli/src/commands/secrets.rs', 'evidence': 'metadata(project,&["configuration-environments"],reqwest::Method::POST,Some(serde_json::json!({"slug":slug,"kind":kind,"protected":protected,"expires_at":expires_at}))).await', 'command_evidence': 'Action::CreateEnvironment'},
    {'command': 'axiom secrets define', 'method': 'POST', 'path': '/api/v1/projects/{project}/secrets', 'source': 'cli/src/commands/secrets.rs', 'evidence': 'metadata(\n                project,\n                &["secrets"],\n                reqwest::Method::POST,\n                Some(serde_json::json!({"name":name,"purpose":purpose,"description":description})),\n            )\n            .await', 'command_evidence': 'Action::Define'},
    {'command': 'axiom secrets bind', 'method': 'PUT', 'path': '/api/v1/projects/{project}/configuration-environments/{environment}/secret-bindings', 'source': 'cli/src/commands/secrets.rs', 'evidence': 'metadata(project, &parts, method, Some(body)).await', 'command_evidence': 'Action::Bind', 'extra_evidence': ['Action::Bind { .. } => ("secret-bindings", reqwest::Method::PUT)']},
    {'command': 'axiom secrets register-workload', 'method': 'POST', 'path': '/api/v1/projects/{project}/configuration-environments/{environment}/secret-workloads', 'source': 'cli/src/commands/secrets.rs', 'evidence': 'metadata(project, &parts, method, Some(body)).await', 'command_evidence': 'Action::RegisterWorkload', 'extra_evidence': ['Action::RegisterWorkload { .. } => ("secret-workloads", reqwest::Method::POST)']},
    {'command': 'axiom secrets grant', 'method': 'POST', 'path': '/api/v1/projects/{project}/configuration-environments/{environment}/secret-grants', 'source': 'cli/src/commands/secrets.rs', 'evidence': 'metadata(project, &parts, method, Some(body)).await', 'command_evidence': 'Action::Grant', 'extra_evidence': ['Action::Grant { .. } => ("secret-grants", reqwest::Method::POST)']},
    {'command': 'axiom secrets update-environment', 'method': 'PATCH', 'path': '/api/v1/projects/{project}/configuration-environments/{environment}/', 'source': 'cli/src/commands/secrets.rs', 'evidence': 'metadata(project, &parts, method, Some(body)).await', 'command_evidence': 'Action::UpdateEnvironment', 'extra_evidence': ['_ => ("", reqwest::Method::PATCH)']},
    {'command': 'axiom secrets workloads', 'method': 'GET', 'path': '/api/v1/projects/{project}/configuration-environments/{environment}/secret-workloads', 'source': 'cli/src/commands/secrets.rs', 'evidence': 'metadata(\n                project,\n                &["configuration-environments", environment, part],\n                reqwest::Method::GET,\n                None,\n            )\n            .await', 'command_evidence': 'Action::Workloads', 'extra_evidence': ['"secret-workloads"']},
    {'command': 'axiom secrets grants', 'method': 'GET', 'path': '/api/v1/projects/{project}/configuration-environments/{environment}/secret-grants', 'source': 'cli/src/commands/secrets.rs', 'evidence': 'metadata(\n                project,\n                &["configuration-environments", environment, part],\n                reqwest::Method::GET,\n                None,\n            )\n            .await', 'command_evidence': 'Action::Grants', 'extra_evidence': ['"secret-grants"']},
    {'command': 'axiom secrets disable-workload', 'method': 'POST', 'path': '/api/v1/projects/{project}/configuration-environments/{environment}/secret-workloads/{id}/disable', 'source': 'cli/src/commands/secrets.rs', 'evidence': 'metadata(\n                project,\n                &[\n                    "configuration-environments",\n                    environment,\n                    part,\n                    id,\n                    operation,\n                ],\n                reqwest::Method::POST,\n                None,\n            )\n            .await', 'command_evidence': 'Action::DisableWorkload', 'extra_evidence': ['("secret-workloads", "disable")']},
    {'command': 'axiom secrets revoke-grant', 'method': 'POST', 'path': '/api/v1/projects/{project}/configuration-environments/{environment}/secret-grants/{id}/revoke', 'source': 'cli/src/commands/secrets.rs', 'evidence': 'metadata(\n                project,\n                &[\n                    "configuration-environments",\n                    environment,\n                    part,\n                    id,\n                    operation,\n                ],\n                reqwest::Method::POST,\n                None,\n            )\n            .await', 'command_evidence': 'Action::RevokeGrant', 'extra_evidence': ['("secret-grants", "revoke")']},
    {'command': 'axiom secrets versions', 'method': 'GET', 'path': '/api/v1/projects/{project}/configuration-environments/{environment}/secrets/{secret}/versions', 'source': 'cli/src/commands/secrets.rs', 'evidence': 'metadata(\n                project,\n                &[\n                    "configuration-environments",\n                    environment,\n                    "secrets",\n                    secret,\n                    "versions",\n                ],\n                reqwest::Method::GET,\n                None,\n            )\n            .await', 'command_evidence': 'Action::Versions'},
    {'command': 'axiom secrets destroy', 'method': 'POST', 'path': '/api/v1/projects/{project}/configuration-environments/{environment}/secrets/{secret}/versions/{version}/destroy', 'source': 'cli/src/commands/secrets.rs', 'evidence': 'metadata(\n                project,\n                &[\n                    "configuration-environments",\n                    environment,\n                    "secrets",\n                    secret,\n                    "versions",\n                    version,\n                    "destroy",\n                ],\n                reqwest::Method::POST,\n                Some(serde_json::json!({"expected_revision":revision})),\n            )\n            .await', 'command_evidence': 'Action::Destroy'},
    {'command': 'axiom secrets set', 'method': 'POST', 'path': '/api/v1/projects/{project}/configuration-environments/{environment}/secrets/{secret}/versions', 'source': 'cli/src/commands/secrets.rs', 'evidence': '.write_secret_version(\n                    project,\n                    environment,\n                    secret,\n                    &material,\n                    &uuid::Uuid::new_v4().to_string(),\n                )\n                .await', 'command_evidence': 'Action::Set'},
    {'command': 'axiom secrets list', 'method': 'GET', 'path': '/api/v1/projects/{project}/secrets', 'source': 'cli/src/commands/secrets.rs', 'evidence': '.secret_metadata(project, &[part], reqwest::Method::GET, None)\n                        .await', 'command_evidence': 'Action::List'},
    {'command': 'axiom secrets environments', 'method': 'GET', 'path': '/api/v1/projects/{project}/configuration-environments', 'source': 'cli/src/commands/secrets.rs', 'evidence': '.secret_metadata(project, &[part], reqwest::Method::GET, None)\n                        .await', 'command_evidence': 'Action::Environments'},
    {'command': 'axiom secrets audit', 'method': 'GET', 'path': '/api/v1/projects/{project}/configuration-environments/{environment}/secret-audit', 'source': 'cli/src/commands/secrets.rs', 'evidence': '.secret_audit_page(project, environment, *before)\n                        .await', 'command_evidence': 'Action::Audit'},
    {'command': 'axiom secrets activate', 'method': 'POST', 'path': '/api/v1/projects/{project}/configuration-environments/{environment}/secrets/{secret}/versions/{version}/activate', 'source': 'cli/src/commands/secrets.rs', 'evidence': '.secret_metadata(\n                            project,\n                            &[\n                                "configuration-environments",\n                                environment,\n                                "secrets",\n                                secret,\n                                "versions",\n                                version,\n                                operation\n                            ],\n                            reqwest::Method::POST,\n                            Some(serde_json::json!({"expected_revision":revision}))\n                        )\n                        .await', 'command_evidence': 'Action::Activate', 'extra_evidence': ['"activate"']},
    {'command': 'axiom secrets rewrap', 'method': 'POST', 'path': '/api/v1/projects/{project}/configuration-environments/{environment}/secrets/{secret}/versions/{version}/rewrap', 'source': 'cli/src/commands/secrets.rs', 'evidence': '.secret_metadata(\n                            project,\n                            &[\n                                "configuration-environments",\n                                environment,\n                                "secrets",\n                                secret,\n                                "versions",\n                                version,\n                                operation\n                            ],\n                            reqwest::Method::POST,\n                            Some(serde_json::json!({"expected_revision":revision}))\n                        )\n                        .await', 'command_evidence': 'Action::Rewrap', 'extra_evidence': ['"rewrap"']},
    {'command': 'axiom secrets revoke', 'method': 'POST', 'path': '/api/v1/projects/{project}/configuration-environments/{environment}/secrets/{secret}/versions/{version}/disable', 'source': 'cli/src/commands/secrets.rs', 'evidence': '.secret_metadata(\n                            project,\n                            &[\n                                "configuration-environments",\n                                environment,\n                                "secrets",\n                                secret,\n                                "versions",\n                                version,\n                                operation\n                            ],\n                            reqwest::Method::POST,\n                            Some(serde_json::json!({"expected_revision":revision}))\n                        )\n                        .await', 'command_evidence': 'Action::Revoke', 'extra_evidence': ['"disable"']},
    {'command': 'axiom project list', 'method': 'GET', 'path': '/api/v1/projects', 'source': 'cli/src/commands/project.rs', 'evidence': 'let projects = client.list_projects().await?;', 'command_evidence': 'pub async fn handle_project_list('},
    {'command': 'axiom project create', 'method': 'POST', 'path': '/api/v1/projects', 'source': 'cli/src/commands/project.rs', 'evidence': '.create_project_with_resources(&name, &slug, &description, &selection)', 'command_evidence': 'pub async fn handle_project_create('},
    {'command': 'axiom project rotate-key', 'method': 'POST', 'path': '/api/v1/projects/{project}/keys/rotate', 'source': 'cli/src/commands/project.rs', 'evidence': 'let key = client.rotate_project_key(&project_slug).await?;', 'command_evidence': 'pub async fn handle_project_rotate_key('},
    {'command': 'axiom project signers list', 'method': 'GET', 'path': '/api/v1/projects/{project}/migrations/signers', 'source': 'cli/src/commands/signers.rs', 'evidence': 'let signers = client.migration_signers(project).await?;', 'command_evidence': 'Action::List'},
    {'command': 'axiom project signers add', 'method': 'POST', 'path': '/api/v1/projects/{project}/migrations/signers', 'source': 'cli/src/commands/signers.rs', 'evidence': '.add_migration_signer(', 'command_evidence': 'Action::Add'},
    {'command': 'axiom project signers revoke', 'method': 'POST', 'path': '/api/v1/projects/{project}/migrations/signers/{id}/revoke', 'source': 'cli/src/commands/signers.rs', 'evidence': '.revoke_migration_signer(project, public_key, &uuid::Uuid::new_v4().to_string())', 'command_evidence': 'Action::Revoke'},
    {'command': 'axiom database history --project', 'method': 'GET', 'path': '/api/v1/projects/{project}/migrations/operations', 'source': 'cli/src/commands/database_remote.rs', 'evidence': '.migration_operations(h.project, Some(h.environment))', 'command_evidence': 'pub async fn history('},
    {'command': 'axiom database history --project', 'method': 'GET', 'path': '/api/v1/projects/{project}/migrations/releases', 'source': 'cli/src/commands/database_remote.rs', 'evidence': 'let releases = h.client.migration_releases(h.project, Some(&stream)).await?;', 'command_evidence': 'pub async fn history('},
    {'command': 'axiom database pull --project', 'method': 'GET', 'path': '/api/v1/projects/{project}/migrations/releases/{sha}', 'source': 'cli/src/commands/database_remote.rs', 'evidence': 'let bytes = h.client.migration_release(h.project, sha256).await?;', 'command_evidence': 'pub async fn pull('},
    {'command': 'axiom database migrate --project', 'method': 'POST', 'path': '/api/v1/projects/{project}/migrations/operations', 'source': 'cli/src/commands/database_remote.rs', 'evidence': '.request_migration_operation(h.project, h.environment, mode, release, &idempotency_key())', 'command_evidence': 'pub async fn migrate('},
    {'command': 'axiom database rollback --project', 'method': 'POST', 'path': '/api/v1/projects/{project}/migrations/operations', 'source': 'cli/src/commands/database_remote.rs', 'evidence': '.request_migration_operation(h.project, h.environment, mode, release, &idempotency_key())', 'command_evidence': 'pub async fn rollback('},
    {'command': 'axiom database status --project', 'method': 'POST', 'path': '/api/v1/projects/{project}/migrations/operations', 'source': 'cli/src/commands/database_remote.rs', 'evidence': '.request_migration_operation(h.project, h.environment, mode, release, &idempotency_key())', 'command_evidence': 'pub async fn status('},
    {'command': 'axiom release', 'method': 'POST', 'path': '/api/v1/projects/{project}/contracts', 'source': 'cli/src/commands/release.rs', 'evidence': '.upload_contract(&project_slug, &version, &artifact_path)', 'command_evidence': 'pub async fn handle_release('},
    {'command': 'axiom release', 'method': 'POST', 'path': '/api/v1/contracts/{id}/sign', 'source': 'cli/src/commands/release.rs', 'evidence': 'client.sign_contract(&existing.id).await', 'command_evidence': 'pub async fn handle_release('},
    {'command': 'axiom pull', 'method': 'GET', 'path': '/contracts/{organization}/{project}/{version}', 'source': 'cli/src/commands/pull.rs', 'evidence': 'Some(client) => client.pull_contract_reference(reference).await', 'command_evidence': 'pub async fn handle_pull(', 'note': 'Public contract download route differs from dashboard /api/v1/projects/.../contracts/{version}/pull; not same-API parity.'},
    {'command': 'axiom pull', 'method': 'GET', 'path': '/contracts/{organization}/{project}', 'source': 'cli/src/commands/pull.rs', 'evidence': 'Some(client) => client.pull_contract_reference(reference).await', 'command_evidence': 'pub async fn handle_pull(', 'note': 'Latest public contract download route differs from dashboard /api/v1/projects/.../contracts/latest.axiom; not same-API parity.'},
    {'command': 'axiom deploy mock-server', 'method': 'POST', 'path': '/api/v1/projects/{project}/mocks', 'source': 'cli/src/commands/deploy.rs', 'evidence': '.upload_mock_config(&project_slug, &json_payload)', 'command_evidence': 'pub async fn handle_deploy_mock_server('},
    {'command': 'axiom mock --cloud', 'method': 'GET', 'path': '/api/v1/projects/{project}/mocks', 'source': 'cli/src/commands/legacy_mock.rs', 'evidence': 'client.get_mock_config(&project_slug, etag.as_deref()).await', 'command_evidence': 'pub async fn cloud_configuration('},
]

# Reviewed exemptions: (METHOD, path) -> {'category', 'reason'}: why the CLI deliberately has
# no command. parity-check.validate_exemptions allows only listed categories and routes.
EXEMPT = {}

# P4 cli-workspace (W4-L7 part 2): workspace, onboarding, sandbox environments and hosted
# contract releases, plus the account-authentication exemptions.
from parity_cli_p4 import CALLERS as _P4_CALLERS, EXEMPT as _P4_EXEMPT  # noqa: E402
CALLERS += _P4_CALLERS
EXEMPT.update(_P4_EXEMPT)

# Reviewed intel request arms. Keep method, route, and dispatch together.
CALLERS += [{'command': 'axiom intel activity list',
  'method': 'GET',
  'path': '/api/v1/activity',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::Activity {\n'
              '                action: ActivityAction::List,\n'
              '            } => (\n'
              '                "GET",\n'
              '                "/api/v1/activity".to_string(),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel activity stream',
  'method': 'GET',
  'path': '/api/v1/activity/stream',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::Activity {\n'
              '                action: ActivityAction::Stream { follow },\n'
              '            } => (\n'
              '                "GET",\n'
              '                "/api/v1/activity/stream".to_string(),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel me get',
  'method': 'GET',
  'path': '/api/v1/me',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::Me {\n'
              '                action: MeAction::Get,\n'
              '            } => ("GET", "/api/v1/me".to_string(), None, false, false, false),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel project get',
  'method': 'GET',
  'path': '/api/v1/projects/{project}',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::Project {\n'
              '                action: ProjectAction::Get,\n'
              '            } => (\n'
              '                "GET",\n'
              '                "/api/v1/projects/{project}".to_string(),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel project overview',
  'method': 'GET',
  'path': '/api/v1/projects/{project}/overview',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::Project {\n'
              '                action: ProjectAction::Overview,\n'
              '            } => (\n'
              '                "GET",\n'
              '                "/api/v1/projects/{project}/overview".to_string(),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel alerts list',
  'method': 'GET',
  'path': '/api/v1/projects/{project}/alerts',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::Alerts {\n'
              '                action: AlertsAction::List,\n'
              '            } => (\n'
              '                "GET",\n'
              '                "/api/v1/projects/{project}/alerts".to_string(),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel alert-policies list',
  'method': 'GET',
  'path': '/api/v1/projects/{project}/alert-policies',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::AlertPolicies {\n'
              '                action: AlertPoliciesAction::List,\n'
              '            } => (\n'
              '                "GET",\n'
              '                "/api/v1/projects/{project}/alert-policies".to_string(),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel alert-policies update',
  'method': 'PUT',
  'path': '/api/v1/projects/{project}/alert-policies/{id}',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::AlertPolicies {\n'
              '                action:\n'
              '                    AlertPoliciesAction::Update {\n'
              '                        policy_kind,\n'
              '                        fields,\n'
              '                    },\n'
              '            } => (\n'
              '                "PUT",\n'
              '                "/api/v1/projects/{project}/alert-policies/{id}"\n'
              '                    .to_string()\n'
              '                    .replace("{id}", &segment(policy_kind)?),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel logs list',
  'method': 'GET',
  'path': '/api/v1/projects/{project}/logs',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::Logs {\n'
              '                action: LogsAction::List { follow, .. },\n'
              '            } => (\n'
              '                "GET",\n'
              '                "/api/v1/projects/{project}/logs".to_string(),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel observability get',
  'method': 'GET',
  'path': '/api/v1/projects/{project}/observability',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::Observability {\n'
              '                action: ObservabilityAction::Get,\n'
              '            } => (\n'
              '                "GET",\n'
              '                "/api/v1/projects/{project}/observability".to_string(),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel observability settings get',
  'method': 'GET',
  'path': '/api/v1/projects/{project}/observability/settings',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::Observability {\n'
              '                action:\n'
              '                    ObservabilityAction::Settings {\n'
              '                        action: ObservabilitySettingsAction::Get,\n'
              '                    },\n'
              '            } => (\n'
              '                "GET",\n'
              '                "/api/v1/projects/{project}/observability/settings".to_string(),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel observability settings update',
  'method': 'PUT',
  'path': '/api/v1/projects/{project}/observability/settings',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::Observability {\n'
              '                action:\n'
              '                    ObservabilityAction::Settings {\n'
              '                        action: ObservabilitySettingsAction::Update { body },\n'
              '                    },\n'
              '            } => (\n'
              '                "PUT",\n'
              '                "/api/v1/projects/{project}/observability/settings".to_string(),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel observability sources list',
  'method': 'GET',
  'path': '/api/v1/projects/{project}/observability/otlp/sources',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::Observability {\n'
              '                action:\n'
              '                    ObservabilityAction::Sources {\n'
              '                        action: ObservabilitySourcesAction::List,\n'
              '                    },\n'
              '            } => (\n'
              '                "GET",\n'
              '                '
              '"/api/v1/projects/{project}/observability/otlp/sources".to_string(),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel observability sources create',
  'method': 'POST',
  'path': '/api/v1/projects/{project}/observability/otlp/sources',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::Observability {\n'
              '                action:\n'
              '                    ObservabilityAction::Sources {\n'
              '                        action: ObservabilitySourcesAction::Create { body },\n'
              '                    },\n'
              '            } => (\n'
              '                "POST",\n'
              '                '
              '"/api/v1/projects/{project}/observability/otlp/sources".to_string(),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel observability sources revoke',
  'method': 'POST',
  'path': '/api/v1/projects/{project}/observability/otlp/sources/{id}/revoke',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::Observability {\n'
              '                action:\n'
              '                    ObservabilityAction::Sources {\n'
              '                        action: ObservabilitySourcesAction::Revoke { source_id },\n'
              '                    },\n'
              '            } => (\n'
              '                "POST",\n'
              '                '
              '"/api/v1/projects/{project}/observability/otlp/sources/{id}/revoke"\n'
              '                    .to_string()\n'
              '                    .replace("{id}", &segment(source_id)?),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel pipeline-runs list',
  'method': 'GET',
  'path': '/api/v1/projects/{project}/pipeline-runs',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::PipelineRuns {\n'
              '                action: PipelineRunsAction::List,\n'
              '            } => (\n'
              '                "GET",\n'
              '                "/api/v1/projects/{project}/pipeline-runs".to_string(),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel pipeline-runs steps',
  'method': 'GET',
  'path': '/api/v1/projects/{project}/pipeline-runs/{id}/steps',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::PipelineRuns {\n'
              '                action: PipelineRunsAction::Steps { run_id },\n'
              '            } => (\n'
              '                "GET",\n'
              '                "/api/v1/projects/{project}/pipeline-runs/{id}/steps"\n'
              '                    .to_string()\n'
              '                    .replace("{id}", &segment(run_id)?),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel pipeline-runs attempts',
  'method': 'GET',
  'path': '/api/v1/projects/{project}/pipeline-runs/{id}/attempts',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::PipelineRuns {\n'
              '                action: PipelineRunsAction::Attempts { run_id },\n'
              '            } => (\n'
              '                "GET",\n'
              '                "/api/v1/projects/{project}/pipeline-runs/{id}/attempts"\n'
              '                    .to_string()\n'
              '                    .replace("{id}", &segment(run_id)?),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel pull-requests list',
  'method': 'GET',
  'path': '/api/v1/projects/{project}/pull-requests',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::PullRequests {\n'
              '                action: PullRequestsAction::List,\n'
              '            } => (\n'
              '                "GET",\n'
              '                "/api/v1/projects/{project}/pull-requests".to_string(),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel pull-requests get',
  'method': 'GET',
  'path': '/api/v1/projects/{project}/pull-requests/{id}',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::PullRequests {\n'
              '                action: PullRequestsAction::Get { number },\n'
              '            } => (\n'
              '                "GET",\n'
              '                "/api/v1/projects/{project}/pull-requests/{id}"\n'
              '                    .to_string()\n'
              '                    .replace("{id}", &segment(number)?),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel pull-requests create',
  'method': 'POST',
  'path': '/api/v1/projects/{project}/pull-requests',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::PullRequests {\n'
              '                action: PullRequestsAction::Create { body },\n'
              '            } => (\n'
              '                "POST",\n'
              '                "/api/v1/projects/{project}/pull-requests".to_string(),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel pull-requests update',
  'method': 'PATCH',
  'path': '/api/v1/projects/{project}/pull-requests/{id}',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::PullRequests {\n'
              '                action: PullRequestsAction::Update { number, body },\n'
              '            } => (\n'
              '                "PATCH",\n'
              '                "/api/v1/projects/{project}/pull-requests/{id}"\n'
              '                    .to_string()\n'
              '                    .replace("{id}", &segment(number)?),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel pull-requests changes',
  'method': 'GET',
  'path': '/api/v1/projects/{project}/pull-requests/{id}/changes',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::PullRequests {\n'
              '                action: PullRequestsAction::Changes { number },\n'
              '            } => (\n'
              '                "GET",\n'
              '                "/api/v1/projects/{project}/pull-requests/{id}/changes"\n'
              '                    .to_string()\n'
              '                    .replace("{id}", &segment(number)?),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel pull-requests comments',
  'method': 'GET',
  'path': '/api/v1/projects/{project}/pull-requests/{id}/comments',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::PullRequests {\n'
              '                action: PullRequestsAction::Comments { number },\n'
              '            } => (\n'
              '                "GET",\n'
              '                "/api/v1/projects/{project}/pull-requests/{id}/comments"\n'
              '                    .to_string()\n'
              '                    .replace("{id}", &segment(number)?),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel pull-requests comment',
  'method': 'POST',
  'path': '/api/v1/projects/{project}/pull-requests/{id}/comments',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::PullRequests {\n'
              '                action: PullRequestsAction::Comment { number, body },\n'
              '            } => (\n'
              '                "POST",\n'
              '                "/api/v1/projects/{project}/pull-requests/{id}/comments"\n'
              '                    .to_string()\n'
              '                    .replace("{id}", &segment(number)?),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel release-channel latest',
  'method': 'GET',
  'path': '/api/v1/projects/{project}/release-channels/latest',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::ReleaseChannel {\n'
              '                action: ReleaseChannelAction::Latest,\n'
              '            } => (\n'
              '                "GET",\n'
              '                "/api/v1/projects/{project}/release-channels/latest".to_string(),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel security audit',
  'method': 'GET',
  'path': '/api/v1/projects/{project}/security-audit',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::Security {\n'
              '                action: SecurityAction::Audit,\n'
              '            } => (\n'
              '                "GET",\n'
              '                "/api/v1/projects/{project}/security-audit".to_string(),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel security posture',
  'method': 'GET',
  'path': '/api/v1/projects/{project}/security-posture',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::Security {\n'
              '                action: SecurityAction::Posture,\n'
              '            } => (\n'
              '                "GET",\n'
              '                "/api/v1/projects/{project}/security-posture".to_string(),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel speed-insights get',
  'method': 'GET',
  'path': '/api/v1/projects/{project}/speed-insights',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::SpeedInsights {\n'
              '                action: SpeedInsightsAction::Get,\n'
              '            } => (\n'
              '                "GET",\n'
              '                "/api/v1/projects/{project}/speed-insights".to_string(),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel test-runs list',
  'method': 'GET',
  'path': '/api/v1/projects/{project}/test-runs',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::TestRuns {\n'
              '                action: TestRunsAction::List,\n'
              '            } => (\n'
              '                "GET",\n'
              '                "/api/v1/projects/{project}/test-runs".to_string(),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel test-runs results',
  'method': 'GET',
  'path': '/api/v1/projects/{project}/test-runs/{id}/results',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::TestRuns {\n'
              '                action: TestRunsAction::Results { test_run_id },\n'
              '            } => (\n'
              '                "GET",\n'
              '                "/api/v1/projects/{project}/test-runs/{id}/results"\n'
              '                    .to_string()\n'
              '                    .replace("{id}", &segment(test_run_id)?),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel test-runs queue',
  'method': 'POST',
  'path': '/api/v1/projects/{project}/test-runs',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::TestRuns {\n'
              '                action: TestRunsAction::Queue { version },\n'
              '            } => (\n'
              '                "POST",\n'
              '                "/api/v1/projects/{project}/test-runs".to_string(),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel traces list',
  'method': 'GET',
  'path': '/api/v1/projects/{project}/traces',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::Traces {\n'
              '                action: TracesAction::List,\n'
              '            } => (\n'
              '                "GET",\n'
              '                "/api/v1/projects/{project}/traces".to_string(),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel traces get',
  'method': 'GET',
  'path': '/api/v1/projects/{project}/traces/{id}',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::Traces {\n'
              '                action: TracesAction::Get { trace_id },\n'
              '            } => (\n'
              '                "GET",\n'
              '                "/api/v1/projects/{project}/traces/{id}"\n'
              '                    .to_string()\n'
              '                    .replace("{id}", &segment(trace_id)?),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel traffic list',
  'method': 'GET',
  'path': '/api/v1/projects/{project}/traffic',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::Traffic {\n'
              '                action: TrafficAction::List,\n'
              '            } => (\n'
              '                "GET",\n'
              '                "/api/v1/projects/{project}/traffic".to_string(),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel settings update',
  'method': 'PATCH',
  'path': '/api/v1/projects/{project}/settings',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::Settings {\n'
              '                action: SettingsAction::Update { fields },\n'
              '            } => (\n'
              '                "PATCH",\n'
              '                "/api/v1/projects/{project}/settings".to_string(),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}},
 {'command': 'axiom intel cli-installed confirm',
  'method': 'POST',
  'path': '/api/v1/dashboard/setup/cli-installed',
  'source': 'cli/src/commands/intel_routes.rs',
  'evidence': 'IntelCommand::CliInstalled {\n'
              '                action: CliInstalledAction::Confirm,\n'
              '            } => (\n'
              '                "POST",\n'
              '                "/api/v1/dashboard/setup/cli-installed".to_string(),',
  'command_evidence': 'pub fn request(&self) -> Result<IntelRequest>',
  'cross_source_evidence': {'cli/src/commands/intel.rs': ['let mut request = args.command.request()?;',
                                                          '.dashboard_request(',
                                                          'request.body.as_ref()',
                                                          'request.method.as_bytes()'],
                            'cli/src/main.rs': ['Commands::Intel(args) => '
                                                'commands::intel::handle(args).await']}}]

def production(text: str) -> str:
    # Command tests live in trailing cfg(test) modules. Drop them before matching.
    text = re.split(r"#\[cfg\(test\)\]\s*mod\s+", text, maxsplit=1)[0]
    return re.sub(r"/\*.*?\*/|(?m:^\s*//[^\n]*)", "", text, flags=re.S)


def scan_cli(core: Path) -> list[dict]:
    """Return current callers whose reviewed production anchors still exist.

    Missing/stale anchors remove coverage rather than silently trusting old
    inventory. Main command dispatch must also exist. A new transport pattern
    needs a reviewed entry in CALLERS; the dashboard scan will expose its gap.
    """
    core = Path(core)
    main_path = core / "cli/src/main.rs"
    main = production(main_path.read_text()) if main_path.is_file() else ""
    result = []
    sources = {}

    def read_source(relative):
        if relative not in sources:
            path = core / relative
            raw = path.read_text() if path.is_file() else ""
            sources[relative] = (raw, production(raw))
        return sources[relative]

    for caller in CALLERS:
        path = core / caller["source"]
        if not path.is_file():
            continue
        raw, source = read_source(caller["source"])
        family = caller["command"].split()[1].capitalize()
        if "Commands::" + family not in main:
            continue
        if caller["evidence"] not in source:
            continue
        if caller.get("command_evidence", "") not in source or any(e not in source for e in caller.get("extra_evidence", [])):
            continue
        cross = caller.get("cross_source_evidence", {})
        if any(not (core / file).is_file() or any(anchor not in read_source(file)[1] for anchor in anchors) for file, anchors in cross.items()):
            continue
        row = {k: v for k, v in caller.items() if k not in ("evidence", "command_evidence", "extra_evidence", "cross_source_evidence")}
        row["line"] = raw[:raw.index(caller["evidence"])].count("\n") + 1
        result.append(row)
    return result
