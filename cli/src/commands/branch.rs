//! `axiom branch` (branches-v1 B8): list, create, show, extend and delete branches; the
//! sanitization policy and branch settings of a parent environment; sanitized sources;
//! review records; captured mail and test users of a branch environment.
//!
//! Owner actions that need a recent sign-in (sanitized branches, the policy, reviews) run the
//! step-up device flow with the existing scope `resources:recovery` on
//! `AXRESOURCE_REAUTH_REQUIRED` and retry with the same Idempotency-Key. Test-user passwords
//! are written only to a new 0600 file, never to standard output. Server text is never echoed.
use super::resources_recovery::Api;
use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Subcommand};
use reqwest::Method;
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Args, Debug)]
pub struct BranchArgs {
    /// Cloud project slug
    #[arg(long, global = true)]
    pub project: Option<String>,
    /// Machine-readable output
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub action: BranchAction,
}

#[derive(Subcommand, Debug)]
pub enum BranchAction {
    /// Live branches and the last 10 ended
    List {
        /// Only branches of this parent environment
        #[arg(long)]
        environment: Option<String>,
    },
    /// Create a branch of a parent environment (Owner; sanitized branches need a recent sign-in)
    Create {
        name: String,
        /// The parent environment, for example `production`
        #[arg(long)]
        parent: String,
        /// `schema` (no application rows) or `sanitized` (rows through the sanitization policy)
        #[arg(long, value_parser = ["schema", "sanitized"], default_value = "schema")]
        mode: String,
        /// Copy the parent as it was at this time (RFC 3339)
        #[arg(long, conflicts_with_all = ["backup", "source"])]
        at: Option<String>,
        /// Copy from a daily backup
        #[arg(long, conflicts_with = "source")]
        backup: Option<String>,
        /// Reuse a sealed sanitized source (`axiom branch sources`)
        #[arg(long)]
        source: Option<String>,
        /// Lifetime in hours (the server caps it)
        #[arg(long, default_value_t = 24)]
        ttl_hours: u32,
        /// Create the branch without authentication
        #[arg(long)]
        no_auth: bool,
        /// Roles and permissions: copied from the source's policy, or empty
        #[arg(long, value_parser = ["copy", "none"], default_value = "copy")]
        auth_policy: String,
        /// Extra redirect URL of the branch environment; repeatable
        #[arg(long = "redirect-url")]
        redirect_urls: Vec<String>,
        /// Extra allowed origin of the branch environment; repeatable
        #[arg(long = "allowed-origin")]
        allowed_origins: Vec<String>,
        /// Typed confirmation: "branch <parent> as <name>"
        #[arg(long)]
        confirm: Option<String>,
        /// Poll until the branch is ready (20 minutes)
        #[arg(long)]
        wait: bool,
        #[arg(long)]
        operation_id: Option<String>,
    },
    Show {
        id: String,
    },
    /// Move the branch's expiry (Owner)
    Extend {
        id: String,
        /// New expiry (RFC 3339)
        #[arg(long, conflicts_with = "hours")]
        expires_at: Option<String>,
        /// New expiry this many hours from now
        #[arg(long)]
        hours: Option<u32>,
    },
    /// Delete a branch and its database (Owner)
    Delete {
        id: String,
        /// Typed confirmation: "delete branch <name>"
        #[arg(long)]
        confirm: Option<String>,
    },
    /// The sanitization policy of a parent environment
    Policy {
        #[command(subcommand)]
        action: PolicyAction,
    },
    /// Branch settings of a parent environment (require_review)
    Settings {
        #[command(subcommand)]
        action: SettingsAction,
    },
    /// Sanitized sources built for a parent environment (counts and checks, never values)
    Sources {
        #[arg(long)]
        environment: String,
    },
    /// Review records that let a parent apply a release tested on a branch
    Review {
        #[command(subcommand)]
        action: ReviewAction,
    },
    /// Mail and SMS captured by a branch environment
    Mail {
        #[command(subcommand)]
        action: MailAction,
    },
    /// Create password test users on a branch; passwords go only to a new 0600 file
    TestUsers {
        id: String,
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=10))]
        count: u32,
        /// Role name; repeatable
        #[arg(long = "role")]
        roles: Vec<String>,
        /// Organization slug (created if absent)
        #[arg(long)]
        organization: Option<String>,
        /// New file for the emails and passwords (mode 0600)
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum PolicyAction {
    Get {
        #[arg(long)]
        environment: String,
    },
    /// Replace the policy from an axiom-branch-sanitization/v1 file (Owner, recent sign-in)
    Set {
        #[arg(long)]
        environment: String,
        #[arg(long)]
        file: PathBuf,
    },
}

#[derive(Subcommand, Debug)]
pub enum SettingsAction {
    Get {
        #[arg(long)]
        environment: String,
    },
    Set {
        #[arg(long)]
        environment: String,
        /// Require a branch review before the parent applies a release or policy
        #[arg(long, action = clap::ArgAction::Set)]
        require_review: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum ReviewAction {
    /// Approve a release applied on the branch for its parent (Owner, recent sign-in)
    Create {
        branch: String,
        #[arg(long, value_parser = ["migration_release", "auth_policy_release"])]
        kind: String,
        #[arg(long)]
        sha256: String,
        /// Typed confirmation: "approve <first 12 hex> for <parent environment>"
        #[arg(long)]
        confirm: Option<String>,
    },
    List {
        #[arg(long)]
        environment: String,
    },
    Revoke {
        review: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum MailAction {
    List {
        branch: String,
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: Option<u32>,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// One captured message (its links and codes are the point of capturing it)
    Show {
        branch: String,
        message: String,
    },
}

pub const POLICY_FORMAT: &str = "axiom-branch-sanitization/v1";
const MAX_POLICY: u64 = 64 * 1024;
const READY_WAIT: Duration = Duration::from_secs(20 * 60);

fn clean(value: &str) -> String {
    value.chars().filter(|c| !c.is_control()).take(128).collect()
}
fn text(value: &Value, pointer: &str) -> String {
    match value.pointer(pointer) {
        Some(Value::String(s)) if !s.is_empty() => clean(s),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => "-".into(),
    }
}
fn uuid(value: &str, flag: &str) -> Result<()> {
    uuid::Uuid::parse_str(value).map(|_| ()).map_err(|_| anyhow!("{flag} must be a UUID"))
}
fn key(operation_id: Option<&str>) -> Result<String> {
    let key = operation_id.map(str::to_string).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    uuid(&key, "--operation-id")?;
    Ok(key)
}
fn environment_name(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 64 || !value.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_') {
        bail!("environment and branch names are lowercase letters, digits, - and _ (at most 64)");
    }
    Ok(())
}

/// Validates the flags and builds `POST /branches`. Nothing here contacts the server.
#[allow(clippy::too_many_arguments)]
pub fn create_body(name: &str, parent: &str, mode: &str, at: Option<&str>, backup: Option<&str>, source: Option<&str>, ttl_hours: u32, auth: bool, auth_policy: &str, redirect_urls: &[String], allowed_origins: &[String], confirm: Option<&str>) -> Result<Value> {
    environment_name(name)?;
    environment_name(parent)?;
    if ttl_hours == 0 {
        bail!("--ttl-hours must be positive");
    }
    let source = match (at, backup, source) {
        (Some(at), None, None) => {
            let parsed = chrono::DateTime::parse_from_rfc3339(at).map_err(|_| anyhow!("--at must be an RFC 3339 time"))?;
            if parsed.with_timezone(&chrono::Utc) > chrono::Utc::now() {
                bail!("--at must be in the past");
            }
            json!({"kind": "point_in_time", "at": at})
        }
        (None, Some(backup), None) => {
            if backup.is_empty() || backup.len() > 128 || !backup.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
                bail!("--backup must be a backup id from `axiom project resources recovery`");
            }
            json!({"kind": "backup", "backup_id": backup})
        }
        (None, None, Some(id)) => {
            uuid(id, "--source")?;
            if mode != "sanitized" {
                bail!("--source reuses a sanitized source: use --mode sanitized");
            }
            json!({"kind": "sanitized_source", "id": id})
        }
        _ => json!({"kind": "current"}),
    };
    let phrase = format!("branch {parent} as {name}");
    match confirm {
        Some(c) if c == phrase => {}
        Some(_) => bail!("The confirmation does not match. Use --confirm \"{phrase}\"."),
        None => bail!(
            "A {mode} branch copies {parent}'s schema{} into a new database that expires after {ttl_hours} hours. \
             Users, sessions and secrets are never copied; mail is captured, never delivered.\n\nTo go ahead, repeat the command with --confirm \"{phrase}\".",
            if mode == "sanitized" { " and rows through the sanitization policy" } else { "" }
        ),
    }
    let mut auth_body = json!({"enabled": auth, "policy": auth_policy});
    if !redirect_urls.is_empty() {
        auth_body["redirect_urls"] = json!(redirect_urls);
    }
    if !allowed_origins.is_empty() {
        auth_body["allowed_origins"] = json!(allowed_origins);
    }
    Ok(json!({"name": name, "parent_environment": parent, "mode": mode, "source": source, "ttl_hours": ttl_hours, "auth": auth_body, "confirm": phrase}))
}

/// A sanitization policy file: format, size and strict JSON; the server validates the rest.
pub fn read_policy(path: &Path) -> Result<Value> {
    let meta = std::fs::metadata(path).with_context(|| format!("read {}", path.display()))?;
    if meta.len() > MAX_POLICY {
        bail!("sanitization policies are at most 64 KiB");
    }
    let document: Value = serde_json::from_slice(&std::fs::read(path)?).with_context(|| format!("{} is not JSON", path.display()))?;
    if document["format"].as_str() != Some(POLICY_FORMAT) || !document["tables"].is_array() {
        bail!("{} is not an {POLICY_FORMAT} document", path.display());
    }
    if document["tables"].as_array().map_or(0, Vec::len) > 256 {
        bail!("a sanitization policy names at most 256 tables");
    }
    Ok(document)
}

pub fn review_body(kind: &str, sha256: &str, parent: &str, confirm: Option<&str>) -> Result<Value> {
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        bail!("--sha256 is a lowercase hex SHA-256");
    }
    let phrase = format!("approve {} for {parent}", &sha256[..12]);
    match confirm {
        Some(c) if c == phrase => Ok(json!({"subject": {"kind": kind, "sha256": sha256}, "confirm": phrase})),
        _ => bail!("A review lets {parent} apply this release (valid 30 days). Repeat with --confirm \"{phrase}\"."),
    }
}

pub fn branch_line(b: &Value) -> String {
    format!(
        "{}  {}  {} of {}  {}  expires {}  database {}",
        text(b, "/id"),
        text(b, "/name"),
        text(b, "/mode"),
        text(b, "/parent_environment"),
        text(b, "/status"),
        text(b, "/expires_at"),
        text(b, "/database/status")
    )
}

/// Writes the test users' emails and passwords to a new 0600 file and returns the shown view
/// (ids and emails only).
pub fn store_test_users(path: &Path, mut response: Value) -> Result<Value> {
    let users = response["users"].as_array().cloned().unwrap_or_default();
    let mut lines = String::new();
    for user in &users {
        if let (Some(email), Some(password)) = (user["email"].as_str(), user["password"].as_str()) {
            lines.push_str(&format!("{email}\t{password}\n"));
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).with_context(|| format!("create {} (it must not exist)", path.display()))?;
    file.write_all(lines.as_bytes())?;
    file.sync_all()?;
    lines.replace_range(.., &"\0".repeat(lines.len()));
    if let Some(users) = response["users"].as_array_mut() {
        for user in users {
            if let Some(map) = user.as_object_mut() {
                map.remove("password");
            }
        }
    }
    Ok(response)
}

fn print(json_out: bool, value: &Value, human: impl FnOnce(&Value) -> Vec<String>) -> Result<()> {
    if json_out {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        for line in human(value) {
            println!("{line}");
        }
    }
    Ok(())
}

async fn admin(client: &axiom_cloud::CloudClient, project: &str, environment: &str, method: Method, rest: &str, query: &[(&str, String)], body: Option<&Value>, idempotency_key: Option<&str>) -> Result<Value> {
    let result = match idempotency_key {
        Some(key) => client.auth_admin_request_keyed(method, project, environment, rest, body, key).await,
        None => client.auth_admin_request(method, project, environment, rest, query, body).await,
    };
    result.map_err(|e| anyhow!("{e}"))
}

pub async fn handle(args: &BranchArgs) -> Result<()> {
    let project = args.project.clone().ok_or_else(|| anyhow!("pass --project <slug>"))?;
    let api = Api::from_profile()?;
    run(&api, &project, args).await
}

pub async fn run(api: &Api, project: &str, args: &BranchArgs) -> Result<()> {
    let p = project;
    match &args.action {
        BranchAction::List { environment } => {
            let query = environment.iter().map(|e| ("environment", e.clone())).collect::<Vec<_>>();
            let listed = api.get_query(&["projects", p, "branches"], &query).await?;
            print(args.json, &listed, |l| {
                let lines = l["branches"].as_array().into_iter().flatten().map(branch_line).collect::<Vec<_>>();
                if lines.is_empty() {
                    vec!["No branches.".into()]
                } else {
                    lines
                }
            })
        }
        BranchAction::Create { name, parent, mode, at, backup, source, ttl_hours, no_auth, auth_policy, redirect_urls, allowed_origins, confirm, wait, operation_id } => {
            let body = create_body(name, parent, mode, at.as_deref(), backup.as_deref(), source.as_deref(), *ttl_hours, !no_auth, auth_policy, redirect_urls, allowed_origins, confirm.as_deref())?;
            let key = key(operation_id.as_deref())?;
            eprintln!("Idempotency key: {key}");
            let mut created = api.step_up_action(&Method::POST, &["projects", p, "branches"], &body, &key).await?;
            if *wait {
                let id = created.pointer("/branch/id").and_then(Value::as_str).context("AXRESOURCE_INVALID_RESPONSE: no branch id")?.to_string();
                let deadline = tokio::time::Instant::now() + READY_WAIT;
                loop {
                    let current = api.get(&["projects", p, "branches", &id]).await?;
                    match current.pointer("/branch/status").and_then(Value::as_str) {
                        Some("ready") => {
                            created = current;
                            break;
                        }
                        Some("failed") | Some("deleted") | Some("expired") => {
                            let code = current.pointer("/branch/code").and_then(Value::as_str).unwrap_or_default();
                            match axiom_cloud::resources::code_message(code) {
                                Some(message) => bail!("The branch did not become ready. {code}: {message}"),
                                None => bail!("The branch did not become ready. Run `axiom branch show {id}` for its code."),
                            }
                        }
                        _ if tokio::time::Instant::now() >= deadline => bail!("The branch is still being built. Run `axiom branch show {id}`."),
                        _ => tokio::time::sleep(api.wait_interval).await,
                    }
                }
            }
            print(args.json, &created, |c| vec![format!("Branch {}", branch_line(&c["branch"]))])
        }
        BranchAction::Show { id } => {
            uuid(id, "the branch id")?;
            let branch = api.get(&["projects", p, "branches", id]).await?;
            print(args.json, &branch, |b| {
                let b = &b["branch"];
                vec![
                    branch_line(b),
                    format!("Source {} at {} (head {})", text(b, "/source/kind"), text(b, "/source/taken_at"), text(b, "/source/head")),
                    format!("Auth {} ({}), issuer {}", text(b, "/auth/enabled"), text(b, "/auth/status"), text(b, "/auth/issuer")),
                    format!("Environment {} ({}), max expiry {}", text(b, "/environment"), text(b, "/environment_id"), text(b, "/max_expires_at")),
                ]
            })
        }
        BranchAction::Extend { id, expires_at, hours } => {
            uuid(id, "the branch id")?;
            let at = match (expires_at, hours) {
                (Some(at), None) => {
                    chrono::DateTime::parse_from_rfc3339(at).map_err(|_| anyhow!("--expires-at must be an RFC 3339 time"))?;
                    at.clone()
                }
                (None, Some(h)) => (chrono::Utc::now() + chrono::Duration::hours(i64::from(*h))).to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                _ => bail!("pass --expires-at <RFC 3339> or --hours <n>"),
            };
            let key = key(None)?;
            let branch = api.write(&Method::POST, &["projects", p, "branches", id, "extend"], &json!({"expires_at": at}), Some(&key)).await?;
            print(args.json, &branch, |b| vec![format!("Branch now expires {}.", text(b, "/branch/expires_at"))])
        }
        BranchAction::Delete { id, confirm } => {
            uuid(id, "the branch id")?;
            let branch = api.get(&["projects", p, "branches", id]).await?;
            let name = branch.pointer("/branch/name").and_then(Value::as_str).unwrap_or_default().to_string();
            let phrase = format!("delete branch {name}");
            if confirm.as_deref() != Some(phrase.as_str()) {
                bail!("Deleting the branch deletes its database and auth environment; reviews stay. Repeat with --confirm \"{}\".", clean(&phrase));
            }
            let key = key(None)?;
            let deleted = api.write(&Method::POST, &["projects", p, "branches", id, "delete"], &json!({"confirm": phrase}), Some(&key)).await?;
            print(args.json, &deleted, |_| vec![format!("Branch {} is being deleted.", clean(&name))])
        }
        BranchAction::Policy { action: PolicyAction::Get { environment } } => {
            let policy = api.get_query(&["projects", p, "branches", "policy"], &[("environment", environment.clone())]).await?;
            print(args.json, &policy, |v| vec![format!("Policy revision {} sha256 {} updated {}", text(v, "/revision"), text(v, "/sha256"), text(v, "/updated_at")), serde_json::to_string_pretty(&v["policy"]).unwrap_or_default()])
        }
        BranchAction::Policy { action: PolicyAction::Set { environment, file } } => {
            let document = read_policy(file)?;
            let current = api.get_query(&["projects", p, "branches", "policy"], &[("environment", environment.clone())]).await?;
            let body = json!({"document": document, "revision": current["revision"].as_i64().unwrap_or(0)});
            let key = key(None)?;
            let saved = api.step_up_action_query(&Method::PUT, &["projects", p, "branches", "policy"], &[("environment", environment.clone())], &body, &key).await?;
            print(args.json, &saved, |v| vec![format!("Policy saved (revision {}, sha256 {}).", text(v, "/revision"), text(v, "/sha256"))])
        }
        BranchAction::Settings { action: SettingsAction::Get { environment } } => {
            let settings = api.get_query(&["projects", p, "branches", "settings"], &[("environment", environment.clone())]).await?;
            print(args.json, &settings, |s| vec![format!("require_review {} (revision {})", text(s, "/require_review"), text(s, "/revision"))])
        }
        BranchAction::Settings { action: SettingsAction::Set { environment, require_review } } => {
            let current = api.get_query(&["projects", p, "branches", "settings"], &[("environment", environment.clone())]).await?;
            let body = json!({"require_review": require_review, "revision": current["revision"].as_i64().unwrap_or(0)});
            let key = key(None)?;
            let saved = api.session_action_query(&Method::PUT, &["projects", p, "branches", "settings"], &[("environment", environment.clone())], &body, &key).await?;
            print(args.json, &saved, |s| vec![format!("require_review {} (revision {})", text(s, "/require_review"), text(s, "/revision"))])
        }
        BranchAction::Sources { environment } => {
            let sources = api.get_query(&["projects", p, "branches", "sources"], &[("environment", environment.clone())]).await?;
            print(args.json, &sources, |s| {
                let mut lines = s["sources"].as_array().into_iter().flatten().map(|x| format!("{}  {}  {}  {} bytes  expires {}  {}", text(x, "/id"), text(x, "/mode"), text(x, "/state"), text(x, "/size_bytes"), text(x, "/expires_at"), text(x, "/code"))).collect::<Vec<_>>();
                if lines.is_empty() {
                    lines.push("No sources.".into());
                }
                lines
            })
        }
        BranchAction::Review { action: ReviewAction::Create { branch, kind, sha256, confirm } } => {
            uuid(branch, "the branch id")?;
            let current = api.get(&["projects", p, "branches", branch]).await?;
            let parent = current.pointer("/branch/parent_environment").and_then(Value::as_str).context("AXRESOURCE_INVALID_RESPONSE: branch without parent")?.to_string();
            let body = review_body(kind, sha256, &parent, confirm.as_deref())?;
            let key = key(None)?;
            let review = api.step_up_action(&Method::POST, &["projects", p, "branches", branch, "reviews"], &body, &key).await?;
            print(args.json, &review, |r| vec![format!("Review {} recorded; {} may apply {} until {}.", text(r, "/review/id"), clean(&parent), &sha256[..12], text(r, "/review/expires_at"))])
        }
        BranchAction::Review { action: ReviewAction::List { environment } } => {
            let reviews = api.get_query(&["projects", p, "branches", "reviews"], &[("environment", environment.clone())]).await?;
            print(args.json, &reviews, |r| {
                let mut lines = r["reviews"].as_array().into_iter().flatten().map(|x| format!("{}  {} {}  branch {}  expires {}{}", text(x, "/id"), text(x, "/subject/kind"), text(x, "/subject/sha256"), text(x, "/branch_id"), text(x, "/expires_at"), if x["revoked_at"].is_string() { "  revoked" } else { "" })).collect::<Vec<_>>();
                if lines.is_empty() {
                    lines.push("No reviews.".into());
                }
                lines
            })
        }
        BranchAction::Review { action: ReviewAction::Revoke { review } } => {
            uuid(review, "the review id")?;
            let key = key(None)?;
            let revoked = api.write(&Method::POST, &["projects", p, "branches", "reviews", review, "revoke"], &json!({}), Some(&key)).await?;
            print(args.json, &revoked, |_| vec!["Review revoked.".into()])
        }
        BranchAction::Mail { action } => {
            let (branch, rest, query) = match action {
                MailAction::List { branch, limit, cursor } => {
                    let mut query = vec![];
                    if let Some(limit) = limit {
                        query.push(("limit", limit.to_string()));
                    }
                    if let Some(cursor) = cursor {
                        query.push(("cursor", cursor.clone()));
                    }
                    (branch, "mail".to_string(), query)
                }
                MailAction::Show { branch, message } => {
                    uuid(message, "the message id")?;
                    (branch, format!("mail/{message}"), vec![])
                }
            };
            let environment = branch_environment(api, p, branch).await?;
            let client = crate::auth_store::authenticated_cloud_client()?;
            let mail = admin(&client, p, &environment, Method::GET, &rest, &query, None, None).await?;
            print(args.json, &mail, |m| match action {
                MailAction::List { .. } => {
                    let mut lines = m["mail"].as_array().or(m["messages"].as_array()).into_iter().flatten().map(|x| format!("{}  {}  {}  {}  {}", text(x, "/id"), text(x, "/created_at"), text(x, "/channel"), text(x, "/template"), text(x, "/to_address"))).collect::<Vec<_>>();
                    if let Some(next) = m["next_cursor"].as_str() {
                        lines.push(format!("More: --cursor {next}"));
                    }
                    lines
                }
                MailAction::Show { .. } => {
                    let x = m.get("message").unwrap_or(m);
                    vec![format!("{} to {}: {}", text(x, "/template"), text(x, "/to_address"), text(x, "/subject")), x["body_text"].as_str().map(|b| b.chars().filter(|c| *c == '\n' || !c.is_control()).collect()).unwrap_or_default()]
                }
            })
        }
        BranchAction::TestUsers { id, count, roles, organization, output, idempotency_key } => {
            if output.exists() {
                bail!("{} exists; test-user passwords are written only to a new file", output.display());
            }
            let environment = branch_environment(api, p, id).await?;
            let key = key(idempotency_key.as_deref())?;
            let mut body = json!({"count": count, "roles": roles});
            if let Some(org) = organization {
                body["organization"] = json!(org);
            }
            let client = crate::auth_store::authenticated_cloud_client()?;
            let created = admin(&client, p, &environment, Method::POST, "test-users", &[], Some(&body), Some(&key)).await?;
            let shown = store_test_users(output, created)?;
            print(args.json, &shown, |s| vec![format!("{} test user(s) created; emails and passwords are in {} (mode 0600).", s["users"].as_array().map_or(0, Vec::len), output.display())])
        }
    }
}

/// The auth environment of a branch.
async fn branch_environment(api: &Api, project: &str, branch: &str) -> Result<String> {
    uuid(branch, "the branch id")?;
    let current = api.get(&["projects", project, "branches", branch]).await?;
    current.pointer("/branch/environment_id").and_then(Value::as_str).map(str::to_string).context("AXRESOURCE_INVALID_RESPONSE: branch without environment")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cloud_mock::Mock;
    use crate::commands::step_up::StepUpStore;

    const BRANCH: &str = "3e4f5a6b-7c8d-4e9f-8a0b-1c2d3e4f5a6b";

    #[test]
    fn create_needs_the_typed_phrase_and_valid_sources() {
        let error = create_body("feature-x", "production", "schema", None, None, None, 24, true, "copy", &[], &[], None).unwrap_err().to_string();
        assert!(error.contains("--confirm \"branch production as feature-x\""), "{error}");
        let body = create_body("feature-x", "production", "schema", None, None, None, 24, true, "copy", &[], &["http://localhost:5173".into()], Some("branch production as feature-x")).unwrap();
        assert_eq!(body["source"], json!({"kind": "current"}));
        assert_eq!(body["auth"], json!({"enabled": true, "policy": "copy", "allowed_origins": ["http://localhost:5173"]}));
        assert!(create_body("Feature X", "production", "schema", None, None, None, 24, true, "copy", &[], &[], Some("x")).is_err());
        assert!(create_body("f", "production", "schema", None, None, Some(BRANCH), 24, true, "copy", &[], &[], Some("branch production as f")).is_err(), "sources need sanitized");
        assert_eq!(create_body("f", "production", "sanitized", None, None, Some(BRANCH), 24, true, "copy", &[], &[], Some("branch production as f")).unwrap()["source"]["kind"], "sanitized_source");
        assert_eq!(create_body("f", "production", "schema", Some("2026-10-30T08:00:00Z"), None, None, 24, false, "none", &[], &[], Some("branch production as f")).unwrap()["source"]["kind"], "point_in_time");
        assert!(create_body("f", "production", "schema", Some("2999-01-01T00:00:00Z"), None, None, 24, true, "copy", &[], &[], Some("branch production as f")).is_err());
    }

    #[test]
    fn reviews_and_policies_are_checked_locally() {
        let sha = "ab".repeat(32);
        assert!(review_body("migration_release", &sha, "production", None).unwrap_err().to_string().contains("approve abababababab for production"));
        assert_eq!(review_body("migration_release", &sha, "production", Some("approve abababababab for production")).unwrap()["subject"]["sha256"], sha);
        assert!(review_body("migration_release", "ABC", "production", Some("x")).is_err());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("policy.json");
        std::fs::write(&path, r#"{"format":"axiom-branch-sanitization/v1","tables":[{"schema":"app","table":"t","rows":"empty"}]}"#).unwrap();
        assert!(read_policy(&path).is_ok());
        std::fs::write(&path, r#"{"format":"other","tables":[]}"#).unwrap();
        assert!(read_policy(&path).is_err());
    }

    #[test]
    fn test_user_passwords_go_only_to_a_new_private_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.tsv");
        let response = json!({"users": [{"id": BRANCH, "email": "test-1-abcd1234@br.branch.invalid", "password": "SYNTHETIC_PASSWORD_CANARY"}]});
        let shown = store_test_users(&path, response.clone()).unwrap();
        assert!(!shown.to_string().contains("CANARY"));
        assert!(std::fs::read_to_string(&path).unwrap().contains("SYNTHETIC_PASSWORD_CANARY"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert!(store_test_users(&path, response).is_err(), "never replaces a file");
    }

    #[tokio::test]
    async fn create_and_policy_writes_use_step_up_routes_with_keys() {
        let mock = Mock::default();
        let base = mock.start().await;
        let dir = tempfile::tempdir().unwrap();
        let api = Api::new(&base, "a".into(), "r".into(), false, StepUpStore::at(dir.path().join("s.json"))).unwrap().fast();
        mock.script("POST /api/v1/projects/demo/branches", 202, json!({"branch": {"id": BRANCH, "status": "building"}}));
        mock.script(&format!("GET /api/v1/projects/demo/branches/{BRANCH}"), 200, json!({"branch": {"id": BRANCH, "status": "building"}}));
        mock.script(&format!("GET /api/v1/projects/demo/branches/{BRANCH}"), 200, json!({"branch": {"id": BRANCH, "status": "ready", "name": "f"}}));
        let args = BranchArgs {
            project: Some("demo".into()),
            json: true,
            action: BranchAction::Create {
                name: "f".into(),
                parent: "production".into(),
                mode: "schema".into(),
                at: None,
                backup: None,
                source: None,
                ttl_hours: 2,
                no_auth: false,
                auth_policy: "copy".into(),
                redirect_urls: vec![],
                allowed_origins: vec![],
                confirm: Some("branch production as f".into()),
                wait: true,
                operation_id: None,
            },
        };
        run(&api, "demo", &args).await.unwrap();
        let seen = mock.seen("POST /api/v1/projects/demo/branches");
        assert!(uuid::Uuid::parse_str(seen[0].key.as_deref().unwrap()).is_ok());
        assert_eq!(seen[0].body["ttl_hours"], 2);
        mock.script("GET /api/v1/projects/demo/branches/policy", 200, json!({"revision": 3}));
        mock.script("PUT /api/v1/projects/demo/branches/policy", 403, json!({"code": "AXRESOURCE_DENIED"}));
        let path = dir.path().join("p.json");
        std::fs::write(&path, r#"{"format":"axiom-branch-sanitization/v1","tables":[]}"#).unwrap();
        let set = BranchArgs { project: Some("demo".into()), json: true, action: BranchAction::Policy { action: PolicyAction::Set { environment: "production".into(), file: path } } };
        let error = run(&api, "demo", &set).await.unwrap_err().to_string();
        assert!(error.starts_with("AXRESOURCE_DENIED"), "{error}");
        let put = mock.seen("PUT /api/v1/projects/demo/branches/policy");
        assert_eq!(put[0].query.as_deref(), Some("environment=production"));
        assert_eq!(put[0].body["revision"], 3);
    }
}
