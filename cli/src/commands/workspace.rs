//! `axiom workspace` (P4): the dashboard's workspace settings from the terminal: the workspace
//! itself, billing and usage, members, invitations (send, resend, cancel, accept), your
//! notification preferences, workspace-ID availability and first-run onboarding.
//!
//! Every `/workspace...` route is scoped by the optional `workspace_slug` query parameter,
//! set with the global `--workspace <slug>`; without it the server picks your primary
//! workspace (the one where you have the highest role, oldest first). Owner-only and
//! reader-denied checks are made by the server. Invitation tokens are secrets: they are read
//! from standard input, a file or a hidden prompt, never from argv, and never printed.
use super::cloud_cli::{self, text};
use super::resources_recovery::Api;
use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Subcommand};
use reqwest::Method;
use serde_json::{json, Value};
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

#[derive(Args, Debug)]
pub struct WorkspaceArgs {
    /// Workspace slug to act on (default: your primary workspace)
    #[arg(long, global = true, value_name = "SLUG")]
    pub workspace: Option<String>,
    /// Machine-readable output
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub action: WorkspaceAction,
}

#[derive(Subcommand, Debug)]
// Parsed once per run; boxing the onboarding flags would only complicate the clap derive.
#[allow(clippy::large_enum_variant)]
pub enum WorkspaceAction {
    /// The workspace: name, slug, your role, plan and team size
    Show,
    /// Rename the workspace (Owner)
    Update {
        /// New display name (2 to 120 characters)
        #[arg(long)]
        name: String,
    },
    /// Plan, trial and usage of the workspace
    Billing,
    /// Workspace members and their roles
    Members {
        #[command(subcommand)]
        action: MembersAction,
    },
    /// Invitations to join the workspace
    Invitations {
        #[command(subcommand)]
        action: InvitationsAction,
    },
    /// Your notification preferences for the workspace
    Notifications {
        #[command(subcommand)]
        action: NotificationsAction,
    },
    /// Check whether a workspace ID (slug) is valid and free
    Availability {
        /// The workspace ID to check, for example `acme-labs`
        slug: String,
    },
    /// First-run onboarding: your profile and your first workspace
    Onboarding {
        #[command(subcommand)]
        action: OnboardingAction,
    },
}

#[derive(Subcommand, Debug)]
pub enum MembersAction {
    /// Members, owner first
    List,
    /// Change a member's workspace role (Owner; the owner's role cannot change)
    Update {
        /// Member user id or email
        member: String,
        /// New workspace role
        #[arg(long, value_parser = ["maintainer", "reader"])]
        role: String,
    },
    /// Remove a member and their roles on every project of the workspace (Owner)
    Remove {
        /// Member user id or email
        member: String,
        /// Confirm without a prompt
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum InvitationsAction {
    /// Invitations of the workspace, newest first (never their tokens)
    List,
    /// Invite someone by email (Owner or Maintainer); the link is emailed, never shown
    Create {
        /// Email address to invite
        #[arg(long)]
        email: String,
        /// Workspace role on joining
        #[arg(long, value_parser = ["maintainer", "reader"])]
        role: String,
    },
    /// Send a pending invitation again with a new link (Owner or Maintainer)
    Resend {
        /// Invitation id or invited email
        invitation: String,
    },
    /// Cancel a pending invitation and revoke its link (Owner or Maintainer)
    Cancel {
        /// Invitation id or invited email
        invitation: String,
        /// Confirm without a prompt
        #[arg(long)]
        yes: bool,
    },
    /// Accept an invitation sent to your account email. Without --token-stdin or --token-file,
    /// asks for the token or the full invitation link without echoing it
    Accept {
        /// Read the token or the invitation link from standard input
        #[arg(long, conflicts_with = "token_file")]
        token_stdin: bool,
        /// Read the token or the invitation link from this file
        #[arg(long, value_name = "PATH")]
        token_file: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
pub enum NotificationsAction {
    /// Your notification preferences in the workspace
    Show,
    /// Change some preferences; the others keep their current value (read first, then saved whole)
    Update {
        /// Email when a contract release is deployed
        #[arg(long, action = clap::ArgAction::Set, value_name = "BOOL")]
        contract_deployed: Option<bool>,
        /// Email when a pipeline fails
        #[arg(long, action = clap::ArgAction::Set, value_name = "BOOL")]
        pipeline_failed: Option<bool>,
        /// Email about invitations you sent (sent, accepted, cancelled)
        #[arg(long, action = clap::ArgAction::Set, value_name = "BOOL")]
        invitation_activity: Option<bool>,
        /// Product update emails
        #[arg(long, action = clap::ArgAction::Set, value_name = "BOOL")]
        product_updates: Option<bool>,
    },
    /// Queue a test notification to your account email (at most one per 10 minutes)
    Test,
}

#[derive(Subcommand, Debug)]
// Parsed once per run; boxing the onboarding flags would only complicate the clap derive.
#[allow(clippy::large_enum_variant)]
pub enum OnboardingAction {
    /// Whether you have completed onboarding, and the workspace it created
    Status,
    /// Complete onboarding: save your profile and create your first workspace
    Complete(CompleteArgs),
}

#[derive(Args, Debug)]
pub struct CompleteArgs {
    /// Your full name
    #[arg(long)]
    pub full_name: String,
    /// Your job title
    #[arg(long)]
    pub job_title: Option<String>,
    /// Your IANA timezone, for example `Europe/Berlin`
    #[arg(long)]
    pub timezone: String,
    /// Receive product update emails
    #[arg(long)]
    pub product_updates: bool,
    /// What you do: developer, engineering-manager, product-manager, designer, founder-executive, operations
    #[arg(long)]
    pub role: Option<String>,
    /// How you heard of AxiomCore: github, search, twitter, community, content, friend, other
    #[arg(long)]
    pub discovery_source: Option<String>,
    /// Details when --discovery-source is `other`
    #[arg(long)]
    pub discovery_source_detail: Option<String>,
    /// Workspace display name
    #[arg(long)]
    pub workspace_name: String,
    /// Workspace ID: 3 to 63 lowercase letters, digits or hyphens
    #[arg(long)]
    pub workspace_slug: String,
    /// Team size
    #[arg(long, default_value = "just-me", value_parser = ["just-me", "2-10", "11-50", "51-200", "201-500", "501-1000", "1001-5000", "5000-plus"])]
    pub team_size: String,
    /// What you want to use AxiomCore for (enforcement, diffing, sandbox, telemetry, sdk); repeatable
    #[arg(long = "interest", value_name = "INTEREST")]
    pub interests: Vec<String>,
    /// Invite a teammate: `email` or `email:role` (role maintainer, the default, or reader); repeatable
    #[arg(long = "invite", value_name = "EMAIL[:ROLE]")]
    pub invites: Vec<String>,
    /// Plan
    #[arg(long, default_value = "free", value_parser = ["free", "cloud"])]
    pub plan: String,
}

const NOTIFICATION_KEYS: [(&str, &str); 4] = [
    ("contract_deployed", "Contract deployed"),
    ("pipeline_failed", "Pipeline failed"),
    ("invitation_activity", "Invitation activity"),
    ("product_updates", "Product updates"),
];
const MAX_TOKEN_INPUT: u64 = 8 * 1024;

/// The `workspace_slug` query of every `/workspace...` route; empty without `--workspace`.
pub fn scope(workspace: Option<&str>) -> Vec<(&'static str, String)> {
    workspace.map(str::trim).filter(|s| !s.is_empty()).map(|s| vec![("workspace_slug", s.to_lowercase())]).unwrap_or_default()
}

pub fn update_body(name: &str) -> Result<Value> {
    let name = name.trim();
    let length = name.chars().count();
    if !(2..=120).contains(&length) {
        bail!("the workspace name must be 2 to 120 characters");
    }
    Ok(json!({"name": name}))
}

pub fn invitation_body(email: &str, role: &str) -> Result<Value> {
    let email = email.trim().to_lowercase();
    if !email_shaped(&email) {
        bail!("`{}` is not an email address", cloud_cli::clean(&email));
    }
    Ok(json!({"email": email, "role": role}))
}

fn email_shaped(email: &str) -> bool {
    match email.split_once('@') {
        Some((local, domain)) => !local.is_empty() && !domain.contains('@') && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.') && !email.chars().any(char::is_whitespace),
        None => false,
    }
}

/// The token of an invitation: the bare token, or the `token` parameter of a pasted
/// `https://<dashboard>/dashboard/invitations/accept?token=…` link.
pub fn invitation_token(input: &str) -> Result<Zeroizing<String>> {
    let input = input.trim();
    let token = match input.rfind("token=") {
        Some(at) => input[at + "token=".len()..].split(['&', '#']).next().unwrap_or(""),
        None if input.contains("://") || input.contains('?') => bail!("this link has no invitation token; paste the whole link from the invitation email"),
        None => input,
    };
    if token.len() < 16 || token.len() > 256 || !token.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        bail!("that is not an invitation token or link; paste the link from the invitation email");
    }
    Ok(Zeroizing::new(token.to_string()))
}

fn read_token(stdin: bool, file: Option<&Path>) -> Result<Zeroizing<String>> {
    let mut raw = Zeroizing::new(String::new());
    if stdin {
        std::io::stdin().take(MAX_TOKEN_INPUT).read_to_string(&mut raw).context("read the invitation token from standard input")?;
    } else if let Some(path) = file {
        std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?.take(MAX_TOKEN_INPUT).read_to_string(&mut raw).with_context(|| format!("read {}", path.display()))?;
    } else if std::io::stdin().is_terminal() {
        raw = Zeroizing::new(dialoguer::Password::new().with_prompt("Invitation link or token").interact()?);
    } else {
        bail!("no invitation token: pass --token-stdin or --token-file <path> (tokens are never taken as arguments)");
    }
    invitation_token(&raw)
}

/// The full preferences document for `PUT /workspace/notifications`: the current values with
/// the requested changes applied. Every field must be known, so nothing is reset by omission.
pub fn notification_body(current: &Value, changes: [Option<bool>; 4]) -> Result<Value> {
    if changes.iter().all(Option::is_none) {
        bail!("nothing to change: pass at least one of --contract-deployed, --pipeline-failed, --invitation-activity, --product-updates (true or false)");
    }
    let mut body = serde_json::Map::new();
    for ((key, _), change) in NOTIFICATION_KEYS.iter().zip(changes) {
        let value = match change {
            Some(v) => v,
            None => current.get(*key).and_then(Value::as_bool).ok_or_else(|| anyhow!("the server did not return the current `{key}` preference; nothing was changed"))?,
        };
        body.insert((*key).to_string(), Value::Bool(value));
    }
    Ok(Value::Object(body))
}

fn slug_shaped(slug: &str) -> bool {
    let bytes = slug.as_bytes();
    (3..=63).contains(&bytes.len())
        && bytes.iter().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        && bytes[0] != b'-'
        && bytes[bytes.len() - 1] != b'-'
}

/// Checks the onboarding flags as the server does and builds `POST /onboarding/complete`.
pub fn onboarding_body(a: &CompleteArgs) -> Result<Value> {
    let full_name = a.full_name.trim();
    let workspace_name = a.workspace_name.trim();
    let slug = a.workspace_slug.trim().to_lowercase();
    let timezone = a.timezone.trim();
    if full_name.len() < 2 {
        bail!("--full-name must be at least 2 characters");
    }
    if timezone.is_empty() {
        bail!("--timezone is required, for example Europe/Berlin");
    }
    if workspace_name.len() < 2 {
        bail!("--workspace-name must be at least 2 characters");
    }
    if !slug_shaped(&slug) {
        bail!("--workspace-slug must be 3 to 63 lowercase letters, digits or hyphens, starting and ending with a letter or digit");
    }
    let optional = |v: &Option<String>| v.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let source = optional(&a.discovery_source);
    let detail = optional(&a.discovery_source_detail);
    if source.as_deref() == Some("other") && detail.as_deref().is_none_or(|d| d.len() < 2) {
        bail!("--discovery-source other needs --discovery-source-detail (at least 2 characters)");
    }
    let mut invitees = Vec::new();
    for invite in &a.invites {
        let (email, role) = match invite.split_once(':') {
            Some((email, role)) => (email, role.trim().to_lowercase()),
            None => (invite.as_str(), "maintainer".to_string()),
        };
        let body = invitation_body(email, &role)?;
        if role != "maintainer" && role != "reader" {
            bail!("--invite {}: the role is maintainer or reader", cloud_cli::clean(invite));
        }
        if invitees.iter().any(|i: &Value| i["email"] == body["email"]) {
            bail!("--invite {} is given twice", cloud_cli::clean(email.trim()));
        }
        invitees.push(body);
    }
    let interests: Vec<String> = a.interests.iter().map(|i| i.trim().to_string()).filter(|i| !i.is_empty()).collect();
    let mut body = json!({
        "full_name": full_name,
        "timezone": timezone,
        "product_updates": a.product_updates,
        "workspace_name": workspace_name,
        "workspace_slug": slug,
        "team_size": a.team_size,
        "interests": interests,
        "invitees": invitees,
        "selected_plan": a.plan,
    });
    if let Some(job) = optional(&a.job_title) {
        body["job_title"] = json!(job);
    }
    if let Some(role) = optional(&a.role) {
        body["role"] = json!(role);
    }
    if let Some(source) = source {
        if source == "other" {
            body["discovery_source_detail"] = json!(detail);
        }
        body["discovery_source"] = json!(source);
    }
    Ok(body)
}

fn id_of(item: &Value) -> Result<String> {
    item.get("id").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string).ok_or_else(|| anyhow!("the server listed an entry without an id"))
}

/// Removes any invitation secret a reply might carry before it is shown.
pub fn redact(mut value: Value) -> Value {
    fn strip(v: &mut Value) {
        match v {
            Value::Object(map) => {
                for key in ["token", "invite_token", "accept_url", "acceptURL", "invite_link"] {
                    map.remove(key);
                }
                map.values_mut().for_each(strip);
            }
            Value::Array(list) => list.iter_mut().for_each(strip),
            _ => {}
        }
    }
    strip(&mut value);
    value
}

pub fn workspace_lines(v: &Value) -> Vec<String> {
    vec![
        format!("Workspace {} ({})", text(v, "/name"), text(v, "/slug")),
        format!("Id: {}", text(v, "/id")),
        format!("Your role: {}", text(v, "/role")),
        format!("Plan: {} ({}), trial days {}", text(v, "/plan"), text(v, "/plan_status"), text(v, "/trial_days")),
        format!("Team size: {}", text(v, "/team_size")),
    ]
}

pub fn billing_lines(v: &Value) -> Vec<String> {
    vec![
        format!("Plan: {} ({}), trial days {}", text(v, "/plan"), text(v, "/status"), text(v, "/trial_days")),
        format!("Billing provider: {}", text(v, "/billing_provider")),
        format!("Projects: {}", text(v, "/usage/projects")),
        format!("Releases in the last 30 days: {}", text(v, "/usage/releases_last_30_days")),
        format!("Running environments: {}", text(v, "/usage/running_environments")),
    ]
}

pub fn member_lines(v: &Value) -> Vec<String> {
    let lines: Vec<String> = cloud_cli::items(v, &["members", "items"])
        .into_iter()
        .map(|m| format!("{}  {}  {}  {}  joined {}", text(m, "/id"), text(m, "/email"), text(m, "/name"), text(m, "/role"), text(m, "/joined_at")))
        .collect();
    if lines.is_empty() {
        vec!["No members.".into()]
    } else {
        lines
    }
}

pub fn invitation_lines(v: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    for i in cloud_cli::items(v, &["invitations", "items"]) {
        lines.push(format!("{}  {}  {}  {}  expires {}  resent {}", text(i, "/id"), text(i, "/email"), text(i, "/role"), text(i, "/status"), text(i, "/expires_at"), text(i, "/resend_count")));
        if i.get("last_error").and_then(Value::as_str).is_some_and(|e| !e.is_empty()) {
            lines.push(format!("  last delivery error: {}", text(i, "/last_error")));
        }
    }
    if lines.is_empty() {
        vec!["No invitations.".into()]
    } else {
        lines
    }
}

/// The sentence after an invitation is sent or resent; says plainly when no email went out.
pub fn invitation_sentence(v: &Value, verb: &str) -> String {
    let (email, role) = (text(v, "/email"), text(v, "/role"));
    match (v.get("status").and_then(Value::as_str), v.get("delivery").and_then(Value::as_str)) {
        (Some("sent"), _) => format!("{verb} {email} as {role}; the invitation email was sent and its link expires {}.", text(v, "/expires_at")),
        (_, Some("delivery_unavailable")) => format!("Saved the invitation for {email} as {role}, but workspace email delivery is not configured, so no email was sent."),
        _ => format!("Saved the invitation for {email} as {role}; the email is queued and will be retried (see `axiom workspace invitations list`)."),
    }
}

pub fn notification_lines(v: &Value) -> Vec<String> {
    NOTIFICATION_KEYS
        .iter()
        .map(|(key, label)| {
            let state = match v.get(*key).and_then(Value::as_bool) {
                Some(true) => "on",
                Some(false) => "off",
                None => "-",
            };
            format!("{label}: {state}")
        })
        .collect()
}

pub fn availability_line(v: &Value) -> String {
    let slug = text(v, "/slug");
    match (v.get("valid").and_then(Value::as_bool), v.get("available").and_then(Value::as_bool)) {
        (Some(false), _) => format!("`{slug}` is not a valid workspace ID: use 3 to 63 lowercase letters, digits or hyphens, starting and ending with a letter or digit."),
        (_, Some(true)) => format!("`{slug}` is available."),
        _ => format!("`{slug}` is already in use."),
    }
}

pub fn onboarding_lines(v: &Value) -> Vec<String> {
    if v.get("completed").and_then(Value::as_bool) == Some(false) {
        return vec!["Onboarding is not complete. Run `axiom workspace onboarding complete` to create your workspace.".into()];
    }
    vec![
        format!("Workspace {} ({}), id {}", text(v, "/workspace_name"), text(v, "/workspace_slug"), text(v, "/workspace_id")),
        format!("Plan: {} ({}), trial days {}", text(v, "/plan"), text(v, "/status"), text(v, "/trial_days")),
        format!("Invitations queued: {}", text(v, "/invitations_queued")),
        format!("Created: {}", text(v, "/created_at")),
    ]
}

pub async fn handle(args: &WorkspaceArgs) -> Result<()> {
    let api = Api::from_profile()?;
    run(&api, args).await
}

async fn list_members(api: &Api, scope: &[(&str, String)]) -> Result<Value> {
    cloud_cli::call(api, Method::GET, &["workspace", "members"], scope, None).await
}

async fn list_invitations(api: &Api, scope: &[(&str, String)]) -> Result<Value> {
    cloud_cli::call(api, Method::GET, &["workspace", "invitations"], scope, None).await
}

/// A member by user id or email, from the member list.
async fn member(api: &Api, scope: &[(&str, String)], wanted: &str) -> Result<Value> {
    let listed = list_members(api, scope).await?;
    let found = cloud_cli::pick(&cloud_cli::items(&listed, &["members", "items"]), wanted, &["email"], "member")?.clone();
    if found.get("role").and_then(Value::as_str) == Some("owner") {
        bail!("{} is the workspace owner; the owner's role cannot be changed or removed here", text(&found, "/email"));
    }
    Ok(found)
}

/// An invitation by id or invited email, from the invitation list.
async fn invitation(api: &Api, scope: &[(&str, String)], wanted: &str) -> Result<Value> {
    let listed = list_invitations(api, scope).await?;
    Ok(cloud_cli::pick(&cloud_cli::items(&listed, &["invitations", "items"]), wanted, &["email"], "invitation")?.clone())
}

pub async fn run(api: &Api, args: &WorkspaceArgs) -> Result<()> {
    let scope = scope(args.workspace.as_deref());
    let json_out = args.json;
    match &args.action {
        WorkspaceAction::Show => {
            let value = cloud_cli::call(api, Method::GET, &["workspace"], &scope, None).await?;
            cloud_cli::print(json_out, &value, workspace_lines)
        }
        WorkspaceAction::Update { name } => {
            let body = update_body(name)?;
            let value = cloud_cli::call(api, Method::PATCH, &["workspace"], &scope, Some(&body)).await?;
            cloud_cli::print(json_out, &value, |v| vec![format!("Renamed workspace {} to {}.", text(v, "/slug"), text(v, "/name"))])
        }
        WorkspaceAction::Billing => {
            let value = cloud_cli::call(api, Method::GET, &["workspace", "billing"], &scope, None).await?;
            cloud_cli::print(json_out, &value, billing_lines)
        }
        WorkspaceAction::Members { action } => match action {
            MembersAction::List => {
                let value = list_members(api, &scope).await?;
                cloud_cli::print(json_out, &value, member_lines)
            }
            MembersAction::Update { member: wanted, role } => {
                let found = member(api, &scope, wanted).await?;
                let id = id_of(&found)?;
                let body = json!({"role": role});
                let value = cloud_cli::call(api, Method::PATCH, &["workspace", "members", &id], &scope, Some(&body)).await?;
                cloud_cli::print(json_out, &value, |v| vec![format!("{} is now a {} of the workspace.", text(v, "/email"), text(v, "/role"))])
            }
            MembersAction::Remove { member: wanted, yes } => {
                let found = member(api, &scope, wanted).await?;
                let id = id_of(&found)?;
                let email = text(&found, "/email");
                cloud_cli::confirm(*yes, &format!("Remove {email} from the workspace? Their roles on every project of the workspace are removed too, and projects they own pass to the workspace owner."))?;
                cloud_cli::call(api, Method::DELETE, &["workspace", "members", &id], &scope, None).await?;
                cloud_cli::print(json_out, &json!({"removed": true, "member_id": id, "email": email}), |_| {
                    vec![format!("Removed {email} from the workspace, with their roles on its projects. Inviting them again restores workspace membership only; project roles must be granted again.")]
                })
            }
        },
        WorkspaceAction::Invitations { action } => match action {
            InvitationsAction::List => {
                let value = redact(list_invitations(api, &scope).await?);
                cloud_cli::print(json_out, &value, invitation_lines)
            }
            InvitationsAction::Create { email, role } => {
                let body = invitation_body(email, role)?;
                let value = redact(cloud_cli::call(api, Method::POST, &["workspace", "invitations"], &scope, Some(&body)).await?);
                cloud_cli::print(json_out, &value, |v| vec![invitation_sentence(v, "Invited")])
            }
            InvitationsAction::Resend { invitation: wanted } => {
                let id = id_of(&invitation(api, &scope, wanted).await?)?;
                let value = redact(cloud_cli::call(api, Method::POST, &["workspace", "invitations", &id, "resend"], &scope, None).await?);
                cloud_cli::print(json_out, &value, |v| vec![invitation_sentence(v, "Re-invited"), "Earlier links for this invitation no longer work.".into()])
            }
            InvitationsAction::Cancel { invitation: wanted, yes } => {
                let found = invitation(api, &scope, wanted).await?;
                let id = id_of(&found)?;
                let email = text(&found, "/email");
                cloud_cli::confirm(*yes, &format!("Cancel the invitation for {email}? Its link stops working at once."))?;
                cloud_cli::call(api, Method::DELETE, &["workspace", "invitations", &id], &scope, None).await?;
                cloud_cli::print(json_out, &json!({"cancelled": true, "invitation_id": id, "email": email}), |_| vec![format!("Cancelled the invitation for {email}; its link no longer works.")])
            }
            InvitationsAction::Accept { token_stdin, token_file } => {
                let token = read_token(*token_stdin, token_file.as_deref())?;
                let body = json!({"token": token.as_str()});
                let value = redact(cloud_cli::call(api, Method::POST, &["workspace", "invitations", "accept"], &[], Some(&body)).await?);
                cloud_cli::print(json_out, &value, |v| vec![format!("Joined workspace {}. Pass --workspace {} to work in it.", text(v, "/workspace_slug"), text(v, "/workspace_slug"))])
            }
        },
        WorkspaceAction::Notifications { action } => match action {
            NotificationsAction::Show => {
                let value = cloud_cli::call(api, Method::GET, &["workspace", "notifications"], &scope, None).await?;
                cloud_cli::print(json_out, &value, notification_lines)
            }
            NotificationsAction::Update { contract_deployed, pipeline_failed, invitation_activity, product_updates } => {
                let changes = [*contract_deployed, *pipeline_failed, *invitation_activity, *product_updates];
                if changes.iter().all(Option::is_none) {
                    notification_body(&Value::Null, changes)?;
                }
                let current = cloud_cli::call(api, Method::GET, &["workspace", "notifications"], &scope, None).await?;
                let body = notification_body(&current, changes)?;
                let value = cloud_cli::call(api, Method::PUT, &["workspace", "notifications"], &scope, Some(&body)).await?;
                cloud_cli::print(json_out, &value, |v| std::iter::once("Saved your notification preferences:".to_string()).chain(notification_lines(v).into_iter().map(|l| format!("  {l}"))).collect())
            }
            NotificationsAction::Test => {
                let value = cloud_cli::call(api, Method::POST, &["workspace", "notifications", "test"], &scope, None).await?;
                cloud_cli::print(json_out, &value, |_| vec!["Queued a test notification to your account email. You can send another in 10 minutes.".into()])
            }
        },
        WorkspaceAction::Availability { slug } => {
            let query = [("slug", slug.trim().to_lowercase())];
            let value = cloud_cli::call(api, Method::GET, &["onboarding", "workspace-availability"], &query, None).await?;
            cloud_cli::print(json_out, &value, |v| vec![availability_line(v)])
        }
        WorkspaceAction::Onboarding { action } => match action {
            OnboardingAction::Status => {
                let value = cloud_cli::call(api, Method::GET, &["onboarding"], &[], None).await?;
                cloud_cli::print(json_out, &value, onboarding_lines)
            }
            OnboardingAction::Complete(complete) => {
                let body = onboarding_body(complete)?;
                let value = cloud_cli::call(api, Method::POST, &["onboarding", "complete"], &[], Some(&body)).await?;
                let requested = body["workspace_slug"].as_str().unwrap_or_default().to_string();
                cloud_cli::print(json_out, &value, |v| {
                    if v.get("workspace_slug").and_then(Value::as_str) != Some(requested.as_str()) {
                        return vec![format!("Onboarding was already complete; nothing was changed. Your workspace is {} ({}).", text(v, "/workspace_name"), text(v, "/workspace_slug"))];
                    }
                    vec![format!(
                        "Created workspace {} ({}) on the {} plan; {} invitation(s) queued.",
                        text(v, "/workspace_name"),
                        text(v, "/workspace_slug"),
                        text(v, "/plan"),
                        text(v, "/invitations_queued")
                    )]
                })
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cloud_cli::test_support::{harness, Harness};
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        args: WorkspaceArgs,
    }

    const MEMBER: &str = "11111111-1111-4111-8111-111111111111";
    const OWNER: &str = "22222222-2222-4222-8222-222222222222";
    const INVITE: &str = "33333333-3333-4333-8333-333333333333";
    const TOKEN: &str = "SECRET_INVITE_TOKEN_CANARY_abcdefghijklmn";

    fn args(argv: &[&str]) -> WorkspaceArgs {
        Cli::try_parse_from(std::iter::once("workspace").chain(argv.iter().copied())).unwrap().args
    }

    async fn exec(h: &Harness, argv: &[&str]) -> Result<()> {
        run(&h.api, &args(argv)).await
    }

    fn members(h: &Harness) {
        h.mock.script(
            "GET /api/v1/workspace/members",
            200,
            json!([
                {"id": OWNER, "email": "owner@acme.dev", "name": "Olive", "role": "owner", "joined_at": "2026-01-01T00:00:00Z"},
                {"id": MEMBER, "email": "dev@acme.dev", "name": "Dee", "role": "maintainer", "joined_at": "2026-02-01T00:00:00Z"}
            ]),
        );
    }

    fn invitations(h: &Harness) {
        h.mock.script("GET /api/v1/workspace/invitations", 200, json!([{"id": INVITE, "email": "new@acme.dev", "role": "reader", "status": "sent", "created_at": "2026-11-01T00:00:00Z", "resend_count": 0}]));
    }

    #[tokio::test]
    async fn show_without_workspace_sends_no_scope_and_with_it_sends_the_slug() {
        let h = harness().await;
        h.mock.script("GET /api/v1/workspace", 200, json!({"id": "w", "name": "Acme", "slug": "acme", "role": "owner", "plan": "free", "plan_status": "active", "trial_days": 0, "team_size": "2-10"}));
        exec(&h, &["show"]).await.unwrap();
        exec(&h, &["--workspace", "Acme", "show", "--json"]).await.unwrap();
        let seen = h.mock.seen("GET /api/v1/workspace");
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].query, None);
        assert_eq!(seen[1].query.as_deref(), Some("workspace_slug=acme"));
        assert_eq!(seen[0].bearer.as_deref(), Some("access"));
    }

    #[tokio::test]
    async fn update_renames_with_a_trimmed_name() {
        let h = harness().await;
        h.mock.script("PATCH /api/v1/workspace", 200, json!({"id": "w", "name": "Acme Labs", "slug": "acme"}));
        exec(&h, &["update", "--name", "  Acme Labs ", "--workspace", "acme"]).await.unwrap();
        let seen = h.mock.seen("PATCH /api/v1/workspace");
        assert_eq!(seen[0].body, json!({"name": "Acme Labs"}));
        assert_eq!(seen[0].query.as_deref(), Some("workspace_slug=acme"));
        assert!(exec(&h, &["update", "--name", "A"]).await.is_err());
        assert_eq!(h.mock.seen("PATCH /api/v1/workspace").len(), 1, "invalid names are refused locally");
    }

    #[tokio::test]
    async fn billing_reads_plan_and_usage() {
        let h = harness().await;
        h.mock.script("GET /api/v1/workspace/billing", 200, json!({"plan": "free", "status": "active", "trial_days": 0, "billing_provider": "none", "usage": {"projects": 2, "releases_last_30_days": 5, "running_environments": 1}}));
        exec(&h, &["billing", "--workspace", "acme"]).await.unwrap();
        assert_eq!(h.mock.seen("GET /api/v1/workspace/billing")[0].query.as_deref(), Some("workspace_slug=acme"));
        let lines = billing_lines(&json!({"plan": "free", "status": "active", "trial_days": 0, "billing_provider": "none", "usage": {"projects": 2, "releases_last_30_days": 5, "running_environments": 1}}));
        assert!(lines.contains(&"Projects: 2".to_string()));
    }

    #[tokio::test]
    async fn members_list_and_update_resolve_emails_to_ids() {
        let h = harness().await;
        members(&h);
        exec(&h, &["members", "list"]).await.unwrap();
        h.mock.script(&format!("PATCH /api/v1/workspace/members/{MEMBER}"), 200, json!({"id": MEMBER, "email": "dev@acme.dev", "role": "reader"}));
        exec(&h, &["--workspace", "acme", "members", "update", "DEV@acme.dev", "--role", "reader"]).await.unwrap();
        let seen = h.mock.seen(&format!("PATCH /api/v1/workspace/members/{MEMBER}"));
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].body, json!({"role": "reader"}));
        assert_eq!(seen[0].query.as_deref(), Some("workspace_slug=acme"));
        exec(&h, &["members", "update", MEMBER, "--role", "maintainer"]).await.unwrap();
        assert_eq!(h.mock.seen(&format!("PATCH /api/v1/workspace/members/{MEMBER}"))[1].body, json!({"role": "maintainer"}));
        let e = exec(&h, &["members", "update", "owner@acme.dev", "--role", "reader"]).await.unwrap_err().to_string();
        assert!(e.contains("workspace owner"), "{e}");
        let e = exec(&h, &["members", "update", "ghost@acme.dev", "--role", "reader"]).await.unwrap_err().to_string();
        assert!(e.contains("no member `ghost@acme.dev`"), "{e}");
        assert!(Cli::try_parse_from(["workspace", "members", "update", MEMBER, "--role", "owner"]).is_err());
    }

    #[tokio::test]
    async fn members_remove_needs_yes_without_a_terminal() {
        let h = harness().await;
        members(&h);
        h.mock.script(&format!("DELETE /api/v1/workspace/members/{MEMBER}"), 204, Value::Null);
        if !std::io::stdin().is_terminal() {
            let e = exec(&h, &["members", "remove", "dev@acme.dev"]).await.unwrap_err().to_string();
            assert!(e.contains("--yes") && e.contains("project"), "{e}");
            assert!(h.mock.seen(&format!("DELETE /api/v1/workspace/members/{MEMBER}")).is_empty());
        }
        exec(&h, &["members", "remove", "dev@acme.dev", "--yes", "--workspace", "acme"]).await.unwrap();
        let seen = h.mock.seen(&format!("DELETE /api/v1/workspace/members/{MEMBER}"));
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].query.as_deref(), Some("workspace_slug=acme"));
    }

    #[tokio::test]
    async fn invitations_list_and_create() {
        let h = harness().await;
        invitations(&h);
        exec(&h, &["invitations", "list", "--workspace", "acme"]).await.unwrap();
        assert_eq!(h.mock.seen("GET /api/v1/workspace/invitations")[0].query.as_deref(), Some("workspace_slug=acme"));
        h.mock.script("POST /api/v1/workspace/invitations", 201, json!({"id": INVITE, "email": "new@acme.dev", "role": "reader", "status": "sent", "expires_at": "2026-11-20T00:00:00Z"}));
        exec(&h, &["invitations", "create", "--email", " New@Acme.dev ", "--role", "reader"]).await.unwrap();
        let seen = h.mock.seen("POST /api/v1/workspace/invitations");
        assert_eq!(seen[0].body, json!({"email": "new@acme.dev", "role": "reader"}));
        assert_eq!(seen[0].query, None);
        assert!(exec(&h, &["invitations", "create", "--email", "not-an-email", "--role", "reader"]).await.is_err());
        assert_eq!(h.mock.seen("POST /api/v1/workspace/invitations").len(), 1);
    }

    #[tokio::test]
    async fn invitations_resend_and_cancel_resolve_emails() {
        let h = harness().await;
        invitations(&h);
        h.mock.script(&format!("POST /api/v1/workspace/invitations/{INVITE}/resend"), 202, json!({"id": INVITE, "email": "new@acme.dev", "role": "reader", "status": "queued", "delivery": "queued"}));
        exec(&h, &["invitations", "resend", "new@acme.dev", "--workspace", "acme"]).await.unwrap();
        let seen = h.mock.seen(&format!("POST /api/v1/workspace/invitations/{INVITE}/resend"));
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].query.as_deref(), Some("workspace_slug=acme"));

        h.mock.script(&format!("DELETE /api/v1/workspace/invitations/{INVITE}"), 204, Value::Null);
        if !std::io::stdin().is_terminal() {
            assert!(exec(&h, &["invitations", "cancel", INVITE]).await.unwrap_err().to_string().contains("--yes"));
            assert!(h.mock.seen(&format!("DELETE /api/v1/workspace/invitations/{INVITE}")).is_empty());
        }
        exec(&h, &["invitations", "cancel", INVITE, "--yes"]).await.unwrap();
        assert_eq!(h.mock.seen(&format!("DELETE /api/v1/workspace/invitations/{INVITE}")).len(), 1);
        assert!(exec(&h, &["invitations", "cancel", "other@acme.dev", "--yes"]).await.unwrap_err().to_string().contains("no invitation"));
    }

    #[tokio::test]
    async fn accept_reads_the_token_from_a_file_and_never_from_argv() {
        let h = harness().await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invite.txt");
        std::fs::write(&path, format!("https://app.axiomcore.dev/dashboard/invitations/accept?token={TOKEN}\n")).unwrap();
        h.mock.script("POST /api/v1/workspace/invitations/accept", 200, json!({"organization_id": "o", "workspace_slug": "acme", "status": "accepted"}));
        exec(&h, &["invitations", "accept", "--token-file", path.to_str().unwrap(), "--workspace", "ignored"]).await.unwrap();
        let seen = h.mock.seen("POST /api/v1/workspace/invitations/accept");
        assert_eq!(seen[0].body, json!({"token": TOKEN}));
        assert_eq!(seen[0].query, None, "accept is not workspace-scoped");
        assert!(Cli::try_parse_from(["workspace", "invitations", "accept", TOKEN]).is_err(), "no positional token");
        assert!(Cli::try_parse_from(["workspace", "invitations", "accept", "--token", TOKEN]).is_err());
        if !std::io::stdin().is_terminal() {
            assert!(exec(&h, &["invitations", "accept"]).await.unwrap_err().to_string().contains("--token-stdin"));
        }
    }

    #[test]
    fn invitation_links_and_tokens_parse() {
        assert_eq!(invitation_token(TOKEN).unwrap().as_str(), TOKEN);
        assert_eq!(invitation_token(&format!("  https://app.example/dashboard/invitations/accept?token={TOKEN}#x ")).unwrap().as_str(), TOKEN);
        assert_eq!(invitation_token(&format!("app.example/dashboard/invitations/accept?a=1&token={TOKEN}&b=2")).unwrap().as_str(), TOKEN);
        assert!(invitation_token("https://app.example/dashboard/invitations/accept").unwrap_err().to_string().contains("no invitation token"));
        assert!(invitation_token("short").is_err());
        assert!(invitation_token("has spaces in it and is long").is_err());
    }

    #[test]
    fn secrets_are_not_rendered() {
        let reply = json!([{"id": INVITE, "email": "new@acme.dev", "role": "reader", "status": "sent", "token": TOKEN, "accept_url": format!("https://x/accept?token={TOKEN}")}]);
        let shown = redact(reply);
        assert!(!shown.to_string().contains("CANARY"));
        assert!(!invitation_lines(&shown).join("\n").contains("CANARY"));
        let accepted = redact(json!({"workspace_slug": "acme", "status": "accepted", "token": TOKEN}));
        assert!(!accepted.to_string().contains("CANARY"));
    }

    #[test]
    fn invitation_sentences_say_when_no_email_was_sent() {
        assert!(invitation_sentence(&json!({"email": "a@b.co", "role": "reader", "status": "queued", "delivery": "delivery_unavailable"}), "Invited").contains("no email was sent"));
        assert!(invitation_sentence(&json!({"email": "a@b.co", "role": "reader", "status": "queued", "delivery": "queued"}), "Invited").contains("queued"));
        assert!(invitation_sentence(&json!({"email": "a@b.co", "role": "reader", "status": "sent", "expires_at": "t"}), "Invited").starts_with("Invited a@b.co as reader"));
    }

    #[tokio::test]
    async fn notifications_show_update_merges_and_test() {
        let h = harness().await;
        h.mock.script("GET /api/v1/workspace/notifications", 200, json!({"contract_deployed": true, "pipeline_failed": true, "invitation_activity": false, "product_updates": true}));
        exec(&h, &["notifications", "show", "--workspace", "acme"]).await.unwrap();
        assert_eq!(h.mock.seen("GET /api/v1/workspace/notifications")[0].query.as_deref(), Some("workspace_slug=acme"));

        h.mock.script("PUT /api/v1/workspace/notifications", 200, json!({"contract_deployed": true, "pipeline_failed": false, "invitation_activity": false, "product_updates": true}));
        exec(&h, &["--workspace", "acme", "notifications", "update", "--pipeline-failed", "false"]).await.unwrap();
        let seen = h.mock.seen("PUT /api/v1/workspace/notifications");
        assert_eq!(seen[0].body, json!({"contract_deployed": true, "pipeline_failed": false, "invitation_activity": false, "product_updates": true}));
        assert_eq!(seen[0].query.as_deref(), Some("workspace_slug=acme"));

        let reads = h.mock.seen("GET /api/v1/workspace/notifications").len();
        assert!(exec(&h, &["notifications", "update"]).await.unwrap_err().to_string().contains("nothing to change"));
        assert_eq!(h.mock.seen("GET /api/v1/workspace/notifications").len(), reads, "no request without a change");

        h.mock.script("POST /api/v1/workspace/notifications/test", 202, json!({"status": "queued"}));
        exec(&h, &["notifications", "test"]).await.unwrap();
        let seen = h.mock.seen("POST /api/v1/workspace/notifications/test");
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].query, None);
    }

    #[test]
    fn notification_merge_refuses_unknown_current_values() {
        let partial = json!({"contract_deployed": true});
        assert!(notification_body(&partial, [None, Some(true), None, None]).is_err());
        assert_eq!(
            notification_body(&partial, [None, Some(true), Some(false), Some(false)]).unwrap(),
            json!({"contract_deployed": true, "pipeline_failed": true, "invitation_activity": false, "product_updates": false})
        );
    }

    #[tokio::test]
    async fn availability_checks_a_lowercased_slug() {
        let h = harness().await;
        h.mock.script("GET /api/v1/onboarding/workspace-availability", 200, json!({"slug": "acme-labs", "valid": true, "available": true}));
        exec(&h, &["availability", " Acme-Labs ", "--workspace", "ignored"]).await.unwrap();
        assert_eq!(h.mock.seen("GET /api/v1/onboarding/workspace-availability")[0].query.as_deref(), Some("slug=acme-labs"));
        assert!(availability_line(&json!({"slug": "a", "valid": false, "available": false})).contains("not a valid"));
        assert!(availability_line(&json!({"slug": "acme", "valid": true, "available": false})).contains("already in use"));
    }

    #[tokio::test]
    async fn onboarding_status_and_complete() {
        let h = harness().await;
        h.mock.script("GET /api/v1/onboarding", 200, json!({"completed": false}));
        exec(&h, &["onboarding", "status"]).await.unwrap();
        assert_eq!(h.mock.seen("GET /api/v1/onboarding")[0].query, None);
        assert!(onboarding_lines(&json!({"completed": false}))[0].contains("not complete"));

        h.mock.script("POST /api/v1/onboarding/complete", 201, json!({"workspace_id": "w", "workspace_name": "Acme Labs", "workspace_slug": "acme-labs", "plan": "free", "status": "active", "trial_days": 0, "invitations_queued": 1, "created_at": "t"}));
        exec(
            &h,
            &[
                "onboarding", "complete", "--full-name", "Ada Lovelace", "--timezone", "Europe/London", "--workspace-name", "Acme Labs", "--workspace-slug", "Acme-Labs", "--team-size", "2-10",
                "--interest", "diffing", "--interest", "sdk", "--invite", "Bob@Acme.dev:reader", "--invite", "cy@acme.dev", "--role", "developer", "--discovery-source", "other",
                "--discovery-source-detail", "a talk", "--job-title", "CTO", "--product-updates",
            ],
        )
        .await
        .unwrap();
        let seen = h.mock.seen("POST /api/v1/onboarding/complete");
        assert_eq!(
            seen[0].body,
            json!({
                "full_name": "Ada Lovelace", "job_title": "CTO", "timezone": "Europe/London", "product_updates": true, "role": "developer",
                "discovery_source": "other", "discovery_source_detail": "a talk", "workspace_name": "Acme Labs", "workspace_slug": "acme-labs",
                "team_size": "2-10", "interests": ["diffing", "sdk"],
                "invitees": [{"email": "bob@acme.dev", "role": "reader"}, {"email": "cy@acme.dev", "role": "maintainer"}],
                "selected_plan": "free"
            })
        );
        assert_eq!(seen[0].query, None);
    }

    #[test]
    fn onboarding_flags_are_checked_like_the_server() {
        let base = ["x", "onboarding", "complete", "--full-name", "Ada", "--timezone", "UTC", "--workspace-name", "Acme"];
        let parse = |extra: &[&str]| {
            let argv: Vec<&str> = base.iter().chain(extra.iter()).copied().collect();
            match Cli::try_parse_from(argv).unwrap().args.action {
                WorkspaceAction::Onboarding { action: OnboardingAction::Complete(c) } => onboarding_body(&c),
                _ => unreachable!(),
            }
        };
        let minimal = parse(&["--workspace-slug", "acme"]).unwrap();
        assert_eq!(
            minimal,
            json!({"full_name": "Ada", "timezone": "UTC", "product_updates": false, "workspace_name": "Acme", "workspace_slug": "acme", "team_size": "just-me", "interests": [], "invitees": [], "selected_plan": "free"})
        );
        assert!(parse(&["--workspace-slug", "ac"]).is_err());
        assert!(parse(&["--workspace-slug=-acme"]).is_err());
        assert!(parse(&["--workspace-slug", "acme", "--discovery-source", "other"]).is_err());
        assert!(parse(&["--workspace-slug", "acme", "--invite", "a@b.co:owner"]).is_err());
        assert!(parse(&["--workspace-slug", "acme", "--invite", "a@b.co", "--invite", "A@B.co:reader"]).is_err());
        assert!(parse(&["--workspace-slug", "acme", "--invite", "nope"]).is_err());
        assert_eq!(parse(&["--workspace-slug", "acme", "--discovery-source", "github", "--discovery-source-detail", "x"]).unwrap().get("discovery_source_detail"), None);
    }
}
