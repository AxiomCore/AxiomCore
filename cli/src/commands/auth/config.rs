//! Config promotion (examples-matrix 7, E12): `axiom auth config export`, `diff` and `push`
//! with the `axiom-auth-config/v1` format. A secret is only ever a reference to a Secrets name
//! in the target environment (`{"ref":"<NAME>"}`); no value, hash or sealed blob is exported.
//! `diff` classifies every item `widening`, `narrowing`, `credential` or `neutral`; `push`
//! refuses unless every widening and credential item is approved in the approvals file. Exit
//! codes: 0 clean, 6 unapproved widening or credential item, 8 stale approvals, 2 usage.
use super::{policy, print, target, text, usage, AuthArgs, Exit, Target};
use anyhow::{Context, Result};
use axiom_lib::auth_impact::{self as ai, Policy};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub const CONFIG_FORMAT: &str = "axiom-auth-config/v1";
pub const APPROVALS_FORMAT: &str = "axiom-auth-config-approvals/v1";
const MAX_CONFIG: usize = 256 * 1024;

/// Non-secret settings that promote between environments. Audiences, rate limits and the
/// stream version are environment-specific and never exported.
pub const SETTINGS: &[&str] = &[
    "enabled", "signup_policy", "access_ttl_seconds", "session_absolute_seconds", "session_idle_seconds", "allowed_origins", "enabled_methods",
    "require_verified_email", "password_min_length", "redirect_urls", "logout_urls", "default_redirect_url", "email_sender", "email_from_name",
    "app_name", "invitation_ttl_seconds", "captcha", "turnstile_site_key", "mfa_policy", "mfa_factors", "step_up_seconds", "webauthn_rp_id",
    "webauthn_rp_name", "webauthn_origins", "hosted_sessions",
];
/// List settings compared element by element; an added element widens.
const WIDENING_LISTS: &[&str] = &["enabled_methods", "redirect_urls", "allowed_origins", "logout_urls", "webauthn_origins", "mfa_factors"];
/// Lifetimes: raising one widens.
const LIFETIMES: &[&str] = &["access_ttl_seconds", "session_absolute_seconds", "session_idle_seconds", "invitation_ttl_seconds", "step_up_seconds"];
const NEUTRAL_SETTINGS: &[&str] = &["app_name", "email_from_name", "webauthn_rp_name", "email_sender", "turnstile_site_key"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub format: String,
    pub source: Value,
    pub settings: Map<String, Value>,
    pub providers: Vec<Value>,
    pub trusted_issuers: Vec<Value>,
    pub policy: Value,
    pub outbound: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Item {
    pub id: String,
    pub path: String,
    pub op: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
    pub impact: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Approvals {
    pub format: String,
    pub base_revision: i64,
    pub approved: Vec<ai::Approval>,
}

/// `AUTH_PROVIDER_GOOGLE_CLIENT_SECRET` and the like: uppercase, other characters `_`.
pub fn secret_name(parts: &[&str]) -> String {
    parts.join("_").chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' }).collect()
}
fn reference(name: String) -> Value {
    json!({"ref": name})
}

/// Builds the export of an environment from its admin views. Every secret becomes a reference.
pub fn export(source: Value, settings: &Value, providers: &Value, issuers: &Value, policy: &Policy, webhooks: &Value, hooks: &Value, sms: Option<&Value>) -> Config {
    let settings = SETTINGS.iter().filter_map(|k| settings.get(*k).map(|v| (k.to_string(), v.clone()))).collect::<Map<_, _>>();
    let providers = providers["providers"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            let slug = p["slug"].as_str().or(p["kind"].as_str()).unwrap_or("provider").to_string();
            let mut out = json!({"slug": slug, "kind": p["kind"], "client_id": p["client_id"], "scopes": p["scopes"]});
            for key in ["issuer", "microsoft_tenant", "allowed_tenants", "apple_team_id", "apple_key_id", "enabled"] {
                if let Some(v) = p.get(key).filter(|v| !v.is_null()) {
                    out[key] = v.clone();
                }
            }
            let secret = if p["kind"] == "apple" { "private_key" } else { "client_secret" };
            out["secret"] = reference(secret_name(&["auth", "provider", &slug, secret]));
            out
        })
        .collect();
    let trusted_issuers = issuers["trusted_issuers"]
        .as_array()
        .or(issuers["issuers"].as_array())
        .into_iter()
        .flatten()
        .map(|i| json!({"issuer": i["issuer"], "audiences": i["audiences"], "jwks_url": i["jwks_uri"], "preset": i["preset"], "authorized_parties": i["authorized_parties"], "forbidden_claims": i["forbidden_claims"]}))
        .collect();
    let webhooks = webhooks["webhooks"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(n, w)| json!({"url": w["url"], "event_types": w["event_types"], "enabled": w["enabled"], "secret": reference(secret_name(&["auth", "webhook", &(n + 1).to_string(), "secret"]))}))
        .collect::<Vec<_>>();
    let hooks = hooks["hooks"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|h| {
            let kind = h["kind"].as_str().unwrap_or("hook");
            json!({"kind": kind, "url": h["url"], "timeout_ms": h["timeout_ms"], "failure_policy": h["failure_policy"], "grantable_roles": h["grantable_roles"], "include_email": h["include_email"], "enabled": h["enabled"], "secret": reference(secret_name(&["auth", "hook", kind, "secret"]))})
        })
        .collect::<Vec<_>>();
    let mut outbound = json!({"webhooks": webhooks, "hooks": hooks});
    if let Some(sms) = sms {
        let s = sms.get("sms").unwrap_or(sms);
        let mut out = Map::new();
        for key in ["account_sid", "credential_kind", "api_key_sid", "messaging_service_sid", "from_number", "country_allowlist", "monthly_cap", "daily_cap", "hourly_cap", "enabled"] {
            if let Some(v) = s.get(key).filter(|v| !v.is_null()) {
                out.insert(key.into(), v.clone());
            }
        }
        out.insert("secret".into(), reference(secret_name(&["auth", "sms", "twilio", "secret"])));
        outbound["sms"] = Value::Object(out);
    }
    Config { format: CONFIG_FORMAT.into(), source, settings, providers, trusted_issuers, policy: serde_json::from_slice(&policy.canonical()).unwrap_or(Value::Null), outbound }
}

/// Strict parse: the format, at most 256 KiB, and no secret value anywhere (every `secret`
/// is a `{"ref"}` naming a Secrets entry).
pub fn parse(bytes: &[u8]) -> Result<Config> {
    if bytes.len() > MAX_CONFIG {
        return Err(usage("auth config files are at most 256 KiB"));
    }
    let config: Config = serde_json::from_slice(bytes).map_err(|_| usage("not an axiom-auth-config/v1 file"))?;
    if config.format != CONFIG_FORMAT {
        return Err(usage("not an axiom-auth-config/v1 file"));
    }
    fn refs_only(value: &Value) -> bool {
        match value {
            Value::Object(map) => map.iter().all(|(k, v)| {
                let lower = k.to_ascii_lowercase();
                let secretish = lower == "secret" || lower.ends_with("_secret") || lower.contains("password") || lower == "private_key" || lower.ends_with("token");
                if secretish {
                    v.as_object().is_some_and(|r| r.len() == 1 && r.get("ref").and_then(Value::as_str).is_some_and(|n| !n.is_empty() && n.len() <= 128 && n.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')))
                } else {
                    refs_only(v)
                }
            }),
            Value::Array(items) => items.iter().all(refs_only),
            _ => true,
        }
    }
    let whole = serde_json::to_value(&config)?;
    if !refs_only(&whole) {
        return Err(usage("a secret field holds a value; secrets are only {\"ref\":\"<SECRET_NAME>\"} references"));
    }
    if let Some(key) = config.settings.keys().find(|k| !SETTINGS.contains(&k.as_str())) {
        return Err(usage(format!("settings.{key} is not a promotable setting")));
    }
    Ok(config)
}

fn rank_mfa(policy: &str) -> i32 {
    match policy {
        "off" | "disabled" => 0,
        "optional" => 1,
        "required" => 2,
        _ => 1,
    }
}

fn item(id: String, path: &str, op: &str, before: Option<&Value>, after: Option<&Value>, impact: &str) -> Item {
    Item { id, path: path.into(), op: op.into(), before: before.cloned(), after: after.cloned(), impact: impact.into() }
}

fn list(value: Option<&Value>) -> BTreeSet<String> {
    value.and_then(Value::as_array).into_iter().flatten().map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).collect()
}

fn settings_items(from: &Map<String, Value>, to: &Map<String, Value>, items: &mut Vec<Item>) {
    for key in SETTINGS {
        let (before, after) = (to.get(*key), from.get(*key));
        if after.is_none() || before == after {
            continue;
        }
        let path = format!("settings.{key}");
        if WIDENING_LISTS.contains(key) {
            let (b, a) = (list(before), list(after));
            for v in a.difference(&b) {
                items.push(item(format!("{path}:+:{v}"), &path, "add", None, Some(&json!(v)), "widening"));
            }
            for v in b.difference(&a) {
                items.push(item(format!("{path}:-:{v}"), &path, "remove", Some(&json!(v)), None, "narrowing"));
            }
            continue;
        }
        let impact = if NEUTRAL_SETTINGS.contains(key) {
            "neutral"
        } else if LIFETIMES.contains(key) {
            match (before.and_then(Value::as_i64), after.and_then(Value::as_i64)) {
                (Some(b), Some(a)) if a < b => "narrowing",
                _ => "widening",
            }
        } else {
            match *key {
                "captcha" => {
                    if after.and_then(Value::as_str) == Some("off") {
                        "widening"
                    } else {
                        "narrowing"
                    }
                }
                "mfa_policy" => {
                    if rank_mfa(after.and_then(Value::as_str).unwrap_or("")) < rank_mfa(before.and_then(Value::as_str).unwrap_or("")) {
                        "widening"
                    } else {
                        "narrowing"
                    }
                }
                "hosted_sessions" | "enabled" => {
                    if after == Some(&Value::Bool(true)) {
                        "widening"
                    } else {
                        "narrowing"
                    }
                }
                "require_verified_email" => {
                    if after == Some(&Value::Bool(false)) {
                        "widening"
                    } else {
                        "narrowing"
                    }
                }
                "password_min_length" => match (before.and_then(Value::as_i64), after.and_then(Value::as_i64)) {
                    (Some(b), Some(a)) if a > b => "narrowing",
                    _ => "widening",
                },
                // signup_policy, webauthn_rp_id, default_redirect_url: treated as the riskier side.
                _ => "widening",
            }
        };
        items.push(item(path.clone(), &path, "change", before, after, impact));
    }
}

/// Entries of a keyed list by `key`.
fn keyed<'a>(items: &'a [Value], key: &str) -> BTreeMap<String, &'a Value> {
    items.iter().filter_map(|v| v.get(key).and_then(Value::as_str).map(|k| (k.to_string(), v))).collect()
}

fn entry_items(path: &str, key: &str, from: &[Value], to: &[Value], items: &mut Vec<Item>) {
    let (a, b) = (keyed(from, key), keyed(to, key));
    for (name, after) in &a {
        match b.get(name) {
            None => items.push(item(format!("{path}:+:{name}"), path, "add", None, Some(*after), "widening")),
            Some(before) => {
                let (before, after) = ((*before).clone(), (*after).clone());
                if before == after {
                    continue;
                }
                let mut rest_b = before.clone();
                let mut rest_a = after.clone();
                let ref_changed = before.get("secret") != after.get("secret");
                for v in [&mut rest_b, &mut rest_a] {
                    if let Some(m) = v.as_object_mut() {
                        m.remove("secret");
                    }
                }
                if ref_changed {
                    items.push(item(format!("{path}:credential:{name}"), path, "change", before.get("secret"), after.get("secret"), "credential"));
                }
                if rest_a != rest_b {
                    // Removing protective fields or adding parties is not decidable per field here: treat as widening.
                    let narrowing = path == "outbound.webhooks" && rest_a.get("enabled") == Some(&Value::Bool(false)) && rest_b.get("enabled") == Some(&Value::Bool(true));
                    items.push(item(format!("{path}:~:{name}"), path, "change", Some(&rest_b), Some(&rest_a), if narrowing { "narrowing" } else { "widening" }));
                }
            }
        }
    }
    for (name, before) in &b {
        if !a.contains_key(name) {
            items.push(item(format!("{path}:-:{name}"), path, "remove", Some(*before), None, "narrowing"));
        }
    }
}

/// The items that turn `to` into `from`.
pub fn diff(from: &Config, to: &Config) -> Result<Vec<Item>> {
    let mut items = vec![];
    settings_items(&from.settings, &to.settings, &mut items);
    entry_items("providers", "slug", &from.providers, &to.providers, &mut items);
    entry_items("trusted_issuers", "issuer", &from.trusted_issuers, &to.trusted_issuers, &mut items);
    let parse_policy = |v: &Value| -> Result<Policy> {
        let policy: Policy = serde_json::from_value(v.clone()).map_err(|_| usage("the policy is not an axiom-auth-policy/v1 document"))?;
        policy.validate().map_err(|e| usage(format!("{e:#}")))?;
        Ok(policy)
    };
    let (pa, pb) = (parse_policy(&from.policy)?, parse_policy(&to.policy)?);
    for i in ai::diff(&pb, &pa) {
        let impact = match i.direction.as_str() {
            "widening" => "widening",
            "narrowing" => "narrowing",
            _ if i.kind == "description" => "neutral",
            _ => "neutral",
        };
        items.push(item(format!("policy.{}", i.id), "policy", if i.id.contains(":+:") { "add" } else if i.id.contains(":-:") { "remove" } else { "change" }, None, None, impact));
    }
    let arr = |v: &Value, k: &str| v[k].as_array().cloned().unwrap_or_default();
    entry_items("outbound.webhooks", "url", &arr(&from.outbound, "webhooks"), &arr(&to.outbound, "webhooks"), &mut items);
    entry_items("outbound.hooks", "kind", &arr(&from.outbound, "hooks"), &arr(&to.outbound, "hooks"), &mut items);
    let one = |v: &Value| v.get("sms").filter(|s| s.is_object()).map(|s| vec![{
        let mut s = s.clone();
        s["name"] = json!("twilio");
        s
    }]).unwrap_or_default();
    entry_items("outbound.sms", "name", &one(&from.outbound), &one(&to.outbound), &mut items);
    Ok(items)
}

pub struct Gate {
    pub exit: i32,
    pub unapproved: Vec<String>,
    pub stale: bool,
}

/// Every widening and credential item must be approved for `base_revision`.
pub fn gate(items: &[Item], approvals: Option<&Approvals>, base_revision: i64) -> Gate {
    let stale = approvals.is_some_and(|a| a.base_revision != base_revision);
    let approved = approvals.filter(|_| !stale).map(|a| a.approved.iter().map(|x| x.id.as_str()).collect::<BTreeSet<_>>()).unwrap_or_default();
    let unapproved = items.iter().filter(|i| i.impact == "widening" || i.impact == "credential").filter(|i| !approved.contains(i.id.as_str())).map(|i| i.id.clone()).collect::<Vec<_>>();
    let exit = if stale {
        8
    } else if unapproved.is_empty() {
        0
    } else {
        6
    };
    Gate { exit, unapproved, stale }
}

pub fn parse_approvals(bytes: &[u8]) -> Result<Approvals> {
    let approvals: Approvals = serde_json::from_slice(bytes).map_err(|_| usage("not an axiom-auth-config-approvals/v1 file"))?;
    if approvals.format != APPROVALS_FORMAT || approvals.approved.iter().any(|a| a.id.is_empty() || a.reason.trim().is_empty()) {
        return Err(usage("not an axiom-auth-config-approvals/v1 file (every approval needs an id and a reason)"));
    }
    Ok(approvals)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[derive(clap::Subcommand, Debug)]
pub enum ConfigCommand {
    /// Write the environment's auth configuration as axiom-auth-config/v1 (secrets as references)
    Export {
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Show what pushing a configuration would change in an environment, with its security impact
    Diff {
        /// An axiom-auth-config/v1 file (`local` needs local auth emulation)
        #[arg(long)]
        from: String,
        /// `env:<environment>`
        #[arg(long)]
        to: String,
    },
    /// Apply a configuration to an environment; widening and credential items need approval
    Push {
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        #[arg(long, default_value = "auth/config.approvals.json")]
        approvals: PathBuf,
        /// A policy release already uploaded (`axiom auth policy release`) whose policy equals the file's
        #[arg(long)]
        policy_release: Option<String>,
    },
}

fn scoped(args: &AuthArgs, environment: &str) -> AuthArgs {
    AuthArgs { project: args.project.clone(), environment: environment.to_string(), json: args.json, action: super::Action::Status }
}

fn env_of(to: &str) -> Result<&str> {
    to.strip_prefix("env:").filter(|e| !e.is_empty()).ok_or_else(|| usage("--to is env:<environment>"))
}

async fn environment_config(args: &AuthArgs, target: &Target) -> Result<(Config, Value)> {
    let settings = target.call(Method::GET, "settings", &[], None).await?;
    let overview = target.call(Method::GET, "overview", &[], None).await.unwrap_or(Value::Null);
    let providers = target.call(Method::GET, "providers", &[], None).await?;
    let issuers = target.call(Method::GET, "trusted-issuers", &[], None).await.unwrap_or(json!({}));
    let (policy, _) = policy::environment_policy(target).await.unwrap_or((Policy::empty(), Value::Null));
    let webhooks = target.call(Method::GET, "webhooks", &[], None).await.unwrap_or(json!({}));
    let hooks = target.call(Method::GET, "hooks", &[], None).await.unwrap_or(json!({}));
    let sms = target.call(Method::GET, "sms", &[], None).await.ok();
    let source = json!({"kind": "environment", "project": target.slug, "environment": args.environment, "revision": settings["revision"]});
    let config = export(source, &settings, &providers, &issuers, &policy, &webhooks, &hooks, sms.as_ref());
    Ok((config, json!({"issuer": overview["issuer"], "revision": settings["revision"]})))
}

fn read_from(from: &str) -> Result<Config> {
    if from == "local" {
        return Err(usage("local auth emulation is not available in this CLI build; export the local configuration to a file"));
    }
    let bytes = std::fs::read(from).with_context(|| format!("read {from}"))?;
    parse(&bytes)
}

fn render(header: &Value, items: &[Item]) -> Vec<String> {
    let mut lines = vec![format!(
        "Environment {} of {} (issuer {}, runtime {}), base revision {}",
        text(header, "environment"),
        text(header, "project"),
        text(&header["runtime_target"], "issuer"),
        text(&header["runtime_target"], "runtime"),
        text(header, "base_revision")
    )];
    if items.is_empty() {
        lines.push("No changes.".into());
    }
    for i in items {
        lines.push(format!("  {:<10} {:<6} {}", i.impact, i.op, i.id));
    }
    lines
}

pub async fn run(args: &AuthArgs, command: &ConfigCommand) -> Result<()> {
    match command {
        ConfigCommand::Export { output } => {
            let target = target(args).await?;
            let (config, _) = environment_config(args, &target).await?;
            let mut bytes = serde_json::to_vec_pretty(&serde_json::to_value(&config)?)?;
            bytes.push(b'\n');
            match output {
                Some(path) => {
                    std::fs::write(path, &bytes)?;
                    print(args, &json!({"written": path, "providers": config.providers.len()}), |_| vec![format!("Auth configuration written to {} (secrets as references only).", path.display())])
                }
                None => {
                    print!("{}", String::from_utf8_lossy(&bytes));
                    Ok(())
                }
            }
        }
        ConfigCommand::Diff { from, to } => {
            let from = read_from(from)?;
            let environment = env_of(to)?;
            let scoped = scoped(args, environment);
            let target = target(&scoped).await?;
            let (current, meta) = environment_config(&scoped, &target).await?;
            let items = diff(&from, &current)?;
            let header = json!({"environment": environment, "project": target.slug, "runtime_target": {"issuer": meta["issuer"], "runtime": "axiomAuth"}, "base_revision": meta["revision"]});
            print(args, &json!({"header": header, "items": items}), |_| render(&header, &items))
        }
        ConfigCommand::Push { from, to, approvals, policy_release } => {
            let from = read_from(from)?;
            let environment = env_of(to)?;
            let scoped = scoped(args, environment);
            let target = target(&scoped).await?;
            let (current, meta) = environment_config(&scoped, &target).await?;
            let items = diff(&from, &current)?;
            let base_revision = meta["revision"].as_i64().unwrap_or_default();
            let approvals = if approvals.exists() { Some(parse_approvals(&std::fs::read(approvals)?)?) } else { None };
            let gate = gate(&items, approvals.as_ref(), base_revision);
            if gate.exit != 0 {
                let header = json!({"environment": environment, "project": target.slug, "runtime_target": {"issuer": meta["issuer"], "runtime": "axiomAuth"}, "base_revision": base_revision});
                print(args, &json!({"header": header, "items": items, "unapproved": gate.unapproved, "stale": gate.stale}), |_| render(&header, &items))?;
                let message = if gate.stale { "AXAUTH_CONFIG_STALE_APPROVALS: approvals were made for another base revision" } else { "AXAUTH_CONFIG_UNAPPROVED: approve every widening and credential item in the approvals file" };
                return Err(Exit { code: gate.exit, message: message.into() }.into());
            }
            push(args, &target, &from, &items, base_revision, policy_release.as_deref()).await
        }
    }
}

/// Applies the items in the fixed order: settings, providers, trusted issuers, policy, outbound.
/// Credential items and new outbound endpoints need their secret, which only Secrets holds:
/// they are refused with `missing_secret` guidance rather than applied without one.
async fn push(args: &AuthArgs, target: &Target, from: &Config, items: &[Item], base_revision: i64, policy_release: Option<&str>) -> Result<()> {
    if let Some(i) = items.iter().find(|i| i.impact == "credential" || (i.op == "add" && (i.path == "providers" || i.path.starts_with("outbound")))) {
        return Err(usage(format!(
            "{} needs a secret from Secrets, which the auth admin API does not resolve by reference yet; apply it with the matching `axiom auth` command (secret through stdin) and push again",
            i.id
        )));
    }
    let mut applied = vec![];
    // 1. Settings: only the changed fields, with the base revision.
    let changed = items.iter().filter(|i| i.path.starts_with("settings.")).map(|i| i.path.trim_start_matches("settings.").to_string()).collect::<BTreeSet<_>>();
    if !changed.is_empty() {
        let mut body = json!({"revision": base_revision});
        for key in &changed {
            body[key.as_str()] = from.settings.get(key).cloned().unwrap_or(Value::Null);
        }
        target.call(Method::PUT, "settings", &[], Some(&body)).await?;
        applied.push(format!("settings ({} field(s))", changed.len()));
    }
    // 2. Providers: non-secret fields of existing providers.
    let providers = target.call(Method::GET, "providers", &[], None).await?;
    for i in items.iter().filter(|i| i.path == "providers") {
        let slug = i.id.rsplit(':').next().unwrap_or_default();
        let existing = providers["providers"].as_array().into_iter().flatten().find(|p| p["slug"].as_str().or(p["kind"].as_str()) == Some(slug));
        match (i.op.as_str(), existing) {
            ("remove", Some(p)) => {
                target.call(Method::DELETE, &format!("providers/{}", p["id"].as_str().unwrap_or_default()), &[], None).await?;
            }
            ("change", Some(p)) => {
                let wanted = keyed(&from.providers, "slug")[slug];
                let mut body = json!({"revision": p["revision"]});
                for key in ["client_id", "scopes", "enabled", "microsoft_tenant", "allowed_tenants", "apple_team_id", "apple_key_id"] {
                    if let Some(v) = wanted.get(key).filter(|v| !v.is_null() && p.get(key) != Some(*v)) {
                        body[key] = v.clone();
                    }
                }
                target.call(Method::PATCH, &format!("providers/{}", p["id"].as_str().unwrap_or_default()), &[], Some(&body)).await?;
            }
            _ => continue,
        }
        applied.push(i.id.clone());
    }
    // 3. Trusted issuers.
    let issuers = target.call(Method::GET, "trusted-issuers", &[], None).await.unwrap_or(json!({}));
    let listed = issuers["trusted_issuers"].as_array().or(issuers["issuers"].as_array()).cloned().unwrap_or_default();
    for i in items.iter().filter(|i| i.path == "trusted_issuers") {
        let name = i.id.splitn(3, ':').nth(2).unwrap_or_default();
        let existing = listed.iter().find(|x| x["issuer"].as_str() == Some(name));
        let wanted = keyed(&from.trusted_issuers, "issuer").get(name).map(|v| (*v).clone());
        match (i.op.as_str(), existing, wanted) {
            ("add", None, Some(w)) => {
                let mut body = json!({"issuer": w["issuer"], "audiences": w["audiences"], "preset": w["preset"], "enabled": false});
                for (key, field) in [("jwks_uri", "jwks_url"), ("authorized_parties", "authorized_parties"), ("forbidden_claims", "forbidden_claims")] {
                    if !w[field].is_null() {
                        body[key] = w[field].clone();
                    }
                }
                target.call(Method::POST, "trusted-issuers", &[], Some(&body)).await?;
            }
            ("remove", Some(x), _) => {
                target.call(Method::DELETE, &format!("trusted-issuers/{}", x["id"].as_str().unwrap_or_default()), &[], None).await?;
            }
            ("change", Some(x), Some(w)) => {
                let mut body = json!({"revision": x["revision"], "audiences": w["audiences"]});
                for (key, field) in [("authorized_parties", "authorized_parties"), ("forbidden_claims", "forbidden_claims")] {
                    if !w[field].is_null() {
                        body[key] = w[field].clone();
                    }
                }
                target.call(Method::PATCH, &format!("trusted-issuers/{}", x["id"].as_str().unwrap_or_default()), &[], Some(&body)).await?;
            }
            _ => continue,
        }
        applied.push(i.id.clone());
    }
    // 4. Policy: through a signed release and policy/apply.
    if items.iter().any(|i| i.path == "policy") {
        let release = policy_release.ok_or_else(|| usage("the policy changes: upload it with `axiom auth policy release` and pass --policy-release <sha256>"))?;
        let plan = target.call(Method::POST, "policy/plan", &[], Some(&json!({"release": release}))).await?;
        let wanted: Policy = serde_json::from_value(from.policy.clone()).map_err(|_| usage("the policy is not an axiom-auth-policy/v1 document"))?;
        if plan["policy_after_sha256"].as_str() != Some(wanted.sha256().as_str()) {
            return Err(usage("--policy-release carries another policy than the file"));
        }
        // The config approvals already cover widening items; narrowing items are acknowledged as listed.
        let body = json!({"release": release, "base_sha256": plan["base_sha256"], "acknowledged": {"widening": plan["widening"], "narrowing": plan["narrowing"]}});
        target.call(Method::POST, "policy/apply", &[], Some(&body)).await?;
        applied.push("policy".into());
    }
    // 5. Outbound: changes to existing endpoints only (new ones need a secret).
    for i in items.iter().filter(|i| i.path == "outbound.hooks" && i.op != "add") {
        let kind = i.id.rsplit(':').next().unwrap_or_default();
        if i.op == "remove" {
            target.call(Method::DELETE, &format!("hooks/{kind}"), &[], None).await?;
        } else if let Some(w) = keyed(from.outbound["hooks"].as_array().map(Vec::as_slice).unwrap_or(&[]), "kind").get(kind) {
            let hooks = target.call(Method::GET, "hooks", &[], None).await?;
            let revision = hooks["hooks"].as_array().into_iter().flatten().find(|h| h["kind"].as_str() == Some(kind)).map(|h| h["revision"].clone());
            let body = json!({"url": w["url"], "timeout_ms": w["timeout_ms"], "failure_policy": w["failure_policy"], "grantable_roles": w["grantable_roles"], "include_email": w["include_email"], "enabled": w["enabled"], "revision": revision});
            target.call(Method::PUT, &format!("hooks/{kind}"), &[], Some(&body)).await?;
        }
        applied.push(i.id.clone());
    }
    let skipped = items.iter().filter(|i| (i.path == "outbound.webhooks" || i.path == "outbound.sms") && !applied.contains(&i.id)).map(|i| i.id.clone()).collect::<Vec<_>>();
    print(args, &json!({"applied": applied, "skipped": skipped}), |r| {
        let mut lines = vec![format!("Applied {} item group(s).", r["applied"].as_array().map_or(0, Vec::len))];
        for s in r["skipped"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            lines.push(format!("Not applied (use `axiom auth webhooks` or `axiom auth sms`): {s}"));
        }
        lines
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(settings: Value, providers: Value, policy: Value) -> Config {
        Config { format: CONFIG_FORMAT.into(), source: json!({"kind": "local"}), settings: settings.as_object().unwrap().clone(), providers: providers.as_array().unwrap().clone(), trusted_issuers: vec![], policy, outbound: json!({"webhooks": [], "hooks": []}) }
    }
    fn empty_policy() -> Value {
        json!({"format": "axiom-auth-policy/v1", "permissions": [], "roles": []})
    }

    #[test]
    fn exports_carry_references_only() {
        let settings = json!({"revision": 4, "enabled_methods": ["password"], "audiences": ["x"], "turnstile_secret_set": true, "rate_limits": {}});
        let providers = json!({"providers": [{"id": "p", "kind": "google", "client_id": "cid", "scopes": ["openid"], "client_secret_set": true}]});
        let sms = json!({"sms": {"account_sid": "AC1", "credential_kind": "auth_token", "secret_set": true}});
        let config = export(json!({"kind": "environment"}), &settings, &providers, &json!({}), &Policy::empty(), &json!({"webhooks": [{"url": "https://h.example", "event_types": [], "enabled": true}]}), &json!({}), Some(&sms));
        let text = serde_json::to_string(&config).unwrap();
        assert!(!text.contains("audiences") && !text.contains("rate_limits") && !text.contains("secret_set"), "{text}");
        assert_eq!(config.providers[0]["secret"], json!({"ref": "AUTH_PROVIDER_GOOGLE_CLIENT_SECRET"}));
        assert_eq!(config.outbound["sms"]["secret"], json!({"ref": "AUTH_SMS_TWILIO_SECRET"}));
        assert_eq!(parse(text.as_bytes()).unwrap(), config);
        // A value where a reference belongs is refused.
        let mut leaked = serde_json::to_value(&config).unwrap();
        leaked["providers"][0]["secret"] = json!("SYNTHETIC_SECRET_CANARY");
        assert!(parse(leaked.to_string().as_bytes()).is_err());
        leaked["providers"][0]["secret"] = json!({"ref": "ok_lower"});
        assert!(parse(leaked.to_string().as_bytes()).is_err());
    }

    #[test]
    fn diff_classifies_security_impact() {
        let base = config(json!({"redirect_urls": ["https://a.example/cb"], "captcha": "turnstile", "mfa_policy": "required", "access_ttl_seconds": 900, "app_name": "A"}), json!([{"slug": "google", "kind": "google", "client_id": "1", "secret": {"ref": "G"}}]), empty_policy());
        let next = config(
            json!({"redirect_urls": ["https://b.example/cb"], "captcha": "off", "mfa_policy": "optional", "access_ttl_seconds": 600, "app_name": "B"}),
            json!([{"slug": "google", "kind": "google", "client_id": "1", "secret": {"ref": "G2"}}, {"slug": "github", "kind": "github", "client_id": "2", "secret": {"ref": "H"}}]),
            json!({"format": "axiom-auth-policy/v1", "permissions": [{"name": "p"}], "roles": [{"name": "r", "permissions": ["p"]}]}),
        );
        let items = diff(&next, &base).unwrap();
        let find = |id: &str| items.iter().find(|i| i.id == id).map(|i| i.impact.as_str()).unwrap_or_else(|| panic!("{id} in {items:?}"));
        assert_eq!(find("settings.redirect_urls:+:https://b.example/cb"), "widening");
        assert_eq!(find("settings.redirect_urls:-:https://a.example/cb"), "narrowing");
        assert_eq!(find("settings.captcha"), "widening");
        assert_eq!(find("settings.mfa_policy"), "widening");
        assert_eq!(find("settings.access_ttl_seconds"), "narrowing");
        assert_eq!(find("settings.app_name"), "neutral");
        assert_eq!(find("providers:credential:google"), "credential");
        assert_eq!(find("providers:+:github"), "widening");
        assert_eq!(find("policy.grant:+:r:p"), "widening");
        let g = gate(&items, None, 4);
        assert_eq!(g.exit, 6);
        let approvals = Approvals { format: APPROVALS_FORMAT.into(), base_revision: 4, approved: g.unapproved.iter().map(|id| ai::Approval { id: id.clone(), reason: "reviewed".into() }).collect() };
        assert_eq!(gate(&items, Some(&approvals), 4).exit, 0);
        assert_eq!(gate(&items, Some(&approvals), 5).exit, 8);
        assert!(diff(&base, &base).unwrap().is_empty());
    }
}
