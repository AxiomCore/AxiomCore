//! `axiom auth`: project authentication administration (auth-surfaces/v1 W4.5).
//!
//! Output is metadata-safe by default: addresses are masked unless
//! `--reveal-email`, and no secret or token field is ever printed or written.
//! Secret values come only from stdin or a masked prompt, never argv. Exit
//! codes: 0 ok, 2 usage, 3 forbidden, 4 conflict, 5 unavailable.
use anyhow::{Context, Result};
use axiom_cloud::auth_admin::{public_issuer_get, AuthAdminError};
use axiom_cloud::CloudClient;
use axiom_lib::backend::authentication::{axiom_auth_issuer, LOOPBACK_ISSUERS};
use axiom_lib::deps_manifest::{parse_axiom_deps_v2, AuthDeclaration};
use clap::{Args, Subcommand};
use reqwest::Method;
use serde_json::{json, Map, Value};
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

mod advanced;
mod config;
mod email;
mod emulate;
mod orgs;
mod policy;
mod rbac;
mod users_admin;

#[derive(Args, Debug)]
pub struct AuthArgs {
    /// Cloud project slug, name or id (default: the project linked to this directory)
    #[arg(long, global = true)]
    pub project: Option<String>,
    /// Organization of a `--project` outside your current organization. The project list
    /// covers only the current organization, so with `--org` the `--project` value must
    /// be the project's exact slug
    #[arg(long, global = true, value_name = "ORGANIZATION_SLUG")]
    pub org: Option<String>,
    /// Environment name or UUID
    #[arg(long, global = true, default_value = "development")]
    pub environment: String,
    /// Stable machine-readable output
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Subcommand, Debug)]
pub enum Action {
    /// Show whether authentication is enabled and ready, and the issuer epoch
    Status,
    /// Read or replace the environment's authentication settings
    Settings {
        #[command(subcommand)]
        action: SettingsAction,
    },
    /// Configure Google, GitHub and OIDC sign-in providers
    Providers {
        #[command(subcommand)]
        action: ProvidersAction,
    },
    /// List, inspect, ban, unban and revoke users
    Users {
        #[command(subcommand)]
        action: UsersAction,
    },
    /// Invite an address (sends one invitation email)
    Invite {
        email: String,
        /// Role name; repeatable
        #[arg(long = "role")]
        roles: Vec<String>,
        #[arg(long)]
        organization: Option<String>,
    },
    /// Read the public JWKS and epoch; --check compares their key revisions
    Jwks {
        /// Issuer URL (default: the environment's issuer)
        #[arg(long)]
        issuer: Option<String>,
        #[arg(long)]
        check: bool,
    },
    /// Write the environment's issuer configuration into a frontend manifest
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// OAuth applications that sign users in with this environment (auth-advanced/v1 11)
    OauthApps {
        #[command(subcommand)]
        action: advanced::OauthAppsAction,
    },
    /// The environment's OpenID Connect issuer
    Oidc {
        #[command(subcommand)]
        action: advanced::OidcAction,
    },
    /// External token issuers whose tokens are exchanged for sessions
    TrustedIssuers {
        #[command(subcommand)]
        action: advanced::TrustedIssuersAction,
    },
    /// Auth policy releases: pull, diff, impact check, release, plan and apply (branches-v1 CDB4)
    Policy {
        #[command(subcommand)]
        action: policy::PolicyAction,
    },
    /// Multi-factor authentication
    Mfa {
        #[command(subcommand)]
        action: advanced::MfaAction,
    },
    /// Signed event webhooks (Owner)
    Webhooks {
        #[command(subcommand)]
        action: advanced::WebhooksAction,
    },
    /// Claim hooks: before_user_create and token_claims (Owner)
    Hooks {
        #[command(subcommand)]
        action: advanced::HooksAction,
    },
    /// Twilio SMS for phone verification and the SMS factor (Owner, Pro)
    Sms {
        #[command(subcommand)]
        action: advanced::SmsAction,
    },
    /// Roles and the permissions each role grants
    Roles {
        #[command(subcommand)]
        action: rbac::RolesAction,
    },
    /// Permission keys that roles grant
    Permissions {
        #[command(subcommand)]
        action: rbac::PermissionsAction,
    },
    /// Role assignments of users (environment-wide or in an organization)
    Assignments {
        #[command(subcommand)]
        action: rbac::AssignmentsAction,
    },
    /// Organizations and their members
    Organizations {
        #[command(subcommand)]
        action: orgs::OrganizationsAction,
    },
    /// Pending, accepted and revoked invitations
    Invitations {
        #[command(subcommand)]
        action: orgs::InvitationsAction,
    },
    /// The authentication audit trail
    Events {
        #[command(subcommand)]
        action: users_admin::EventsAction,
    },
    /// Email delivery usage and suppressed addresses
    Email {
        #[command(subcommand)]
        action: email::EmailAction,
    },
    /// Custom SMTP server for authentication email (Owner)
    Smtp {
        #[command(subcommand)]
        action: email::SmtpAction,
    },
    /// Authentication email templates
    Templates {
        #[command(subcommand)]
        action: email::TemplatesAction,
    },
    /// Local auth emulation: a loopback issuer on a disposable SQLite store with the hosted
    /// auth stream, a mail viewer and a mock OIDC issuer (no network, no hosted database)
    Emulate(emulate::EmulateArgs),
    /// Compare a frontend manifest, the environment settings and a deployment
    Check {
        #[arg(long, default_value = "AxiomDeps.toml")]
        manifest: PathBuf,
        /// Backend deployment JSON whose axiomAuth provider is compared
        #[arg(long)]
        deployment: Option<PathBuf>,
        /// Backend .acore source: guarded endpoints without an auth(...) clause are reported
        #[arg(long)]
        backend: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
pub enum SettingsAction {
    Get {
        /// Write the settings JSON to this file (default stdout)
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Set {
        /// Settings JSON including `revision`; `turnstile_secret` may only come from here
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum ProvidersAction {
    List,
    /// Create or update `google`, `github`, `microsoft`, `apple` or `oidc-<slug>`
    Set {
        provider: String,
        #[arg(long)]
        client_id: Option<String>,
        /// OIDC issuer (oidc-<slug> only)
        #[arg(long)]
        issuer: Option<String>,
        /// Comma-separated scopes
        #[arg(long)]
        scopes: Option<String>,
        #[arg(long, conflicts_with = "disable")]
        enable: bool,
        #[arg(long)]
        disable: bool,
        /// Read the client secret from stdin (otherwise a masked prompt when one is needed)
        #[arg(long)]
        client_secret_stdin: bool,
        /// Microsoft: common, organizations, consumers or a tenant GUID
        #[arg(long)]
        tenant: Option<String>,
        /// Microsoft: a tenant GUID allowed to sign in (with common or organizations); repeatable
        #[arg(long = "allowed-tenant")]
        allowed_tenants: Vec<String>,
        /// Apple: the Team ID
        #[arg(long)]
        team_id: Option<String>,
        /// Apple: the Key ID of the Sign in with Apple key
        #[arg(long)]
        key_id: Option<String>,
        /// Apple: the AuthKey .p8 file (read from the file, never an argument)
        #[arg(long)]
        private_key_file: Option<PathBuf>,
    },
    /// Check the provider's credentials without signing anyone in
    Test { provider: String },
    /// Delete a disabled provider's configuration and client secret; users keep their accounts
    Remove {
        /// `google`, `github`, `microsoft`, `apple`, `oidc-<slug>` or the provider's UUID
        provider: String,
        /// Delete without asking for confirmation
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum UsersAction {
    List {
        /// An email prefix, or a user UUID
        #[arg(long)]
        search: Option<String>,
        #[arg(long, value_parser = ["active", "banned", "deleted", "unverified"])]
        status: Option<String>,
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: Option<u32>,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long)]
        reveal_email: bool,
    },
    Show {
        user_id: String,
        #[arg(long)]
        reveal_email: bool,
    },
    Ban {
        user_id: String,
        #[arg(long)]
        reason: String,
    },
    Unban { user_id: String },
    /// A user's factors (types and labels only)
    Factors { user_id: String },
    /// Remove every factor and recovery code of a user (account recovery)
    ResetFactors {
        user_id: String,
        #[arg(long)]
        reason: String,
        #[arg(long)]
        yes: bool,
    },
    /// The OAuth applications a user consented to
    Consents { user_id: String },
    /// Revoke one consent; the application's sessions end
    RevokeConsent {
        user_id: String,
        consent_id: String,
        #[arg(long)]
        yes: bool,
    },
    /// End the user's sessions (one with --session)
    Revoke {
        user_id: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    #[command(flatten)]
    Admin(users_admin::UsersAdminAction),
}

#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Export, diff and push axiom-auth-config/v1 (local-to-Cloud promotion)
    #[command(flatten)]
    Promote(config::ConfigCommand),
    Pull {
        #[arg(long, default_value = "AxiomDeps.toml")]
        manifest: PathBuf,
        /// Update the manifest (otherwise print the block)
        #[arg(long)]
        write: bool,
    },
}

/// An `axiom auth` failure with its exit code; the message names a code only.
#[derive(Debug)]
pub struct Exit {
    pub code: i32,
    pub message: String,
}

impl std::fmt::Display for Exit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Exit {}

fn usage(message: impl Into<String>) -> anyhow::Error {
    Exit { code: 2, message: message.into() }.into()
}

pub fn exit_code(error: &AuthAdminError) -> i32 {
    match error {
        AuthAdminError::Invalid { .. } => 2,
        AuthAdminError::Forbidden { .. } | AuthAdminError::Unauthenticated => 3,
        AuthAdminError::Conflict { .. } => 4,
        AuthAdminError::Unavailable { .. } => 5,
    }
}

fn admin(error: AuthAdminError) -> anyhow::Error {
    Exit { code: exit_code(&error), message: error.to_string() }.into()
}

/// Run `axiom auth`; failures exit with the documented code.
pub async fn handle(args: &AuthArgs) -> Result<()> {
    match run(args).await {
        Ok(()) => Ok(()),
        Err(error) => {
            let (code, message) = match error.downcast_ref::<Exit>() {
                Some(exit) => (exit.code, exit.message.clone()),
                None => (5, format!("{error:#}")),
            };
            eprintln!("axiom auth: {message}");
            std::process::exit(code)
        }
    }
}

struct Target {
    client: CloudClient,
    slug: String,
    environment: String,
}

fn uuid_shaped(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
}

async fn target(args: &AuthArgs) -> Result<Target> {
    let mut target = project_target(args).await?;
    target.environment = resolve_environment(&target.client, &target.slug, &args.environment).await?;
    Ok(target)
}

/// The signed-in client and project slug only, for project-wide routes such as policy
/// releases (`environment` is left empty and never sent).
async fn project_target(args: &AuthArgs) -> Result<Target> {
    let client = crate::auth_store::authenticated_cloud_client()
        .map_err(|_| Exit { code: 3, message: "AXAUTH_UNAUTHENTICATED: run `axiom login` first".into() })?;
    let unavailable = || Exit { code: 5, message: "AXAUTH_UNAVAILABLE: could not list projects".into() };
    let slug = match (&args.project, &args.org) {
        (None, Some(_)) => return Err(usage("--org needs --project <project-slug>")),
        (Some(given), Some(org)) => project_in_org(&client, given, org).await?,
        // Slugs, ids and display names all go through the project list, so a lowercase
        // name is never mistaken for a slug and an ambiguous name always exits 2.
        (Some(given), None) => pick_project(&client.list_projects().await.map_err(|_| unavailable())?, given)?,
        (None, None) => {
            let directory = std::env::current_dir()?;
            let linked = crate::auth_store::get_project_id(&directory)?
                .ok_or_else(|| usage("no project: pass --project <slug> or link this directory with `axiom project link`"))?;
            let projects = client.list_projects().await.map_err(|_| unavailable())?;
            projects
                .into_iter()
                .find(|p| p.id == linked || p.slug == linked)
                .map(|p| p.slug)
                .ok_or_else(|| usage("the linked project is not available; pass --project <slug>"))?
        }
    };
    Ok(Target { client, slug, environment: String::new() })
}

/// `--project` given as a slug, an id or a display name. A value is a slug or id only if a
/// listed project has it; otherwise it is a display name (case-insensitive), which must
/// name exactly one project. Unknown and ambiguous values are usage errors (exit 2).
pub fn pick_project(projects: &[axiom_cloud::project::Project], given: &str) -> Result<String> {
    if let Some(p) = projects.iter().find(|p| p.slug == given).or_else(|| projects.iter().find(|p| p.id.eq_ignore_ascii_case(given))) {
        return Ok(p.slug.clone());
    }
    let named: Vec<_> = projects.iter().filter(|p| p.name.eq_ignore_ascii_case(given.trim())).collect();
    match named.as_slice() {
        [p] => Ok(p.slug.clone()),
        [] => Err(usage(format!(
            "no project with the slug, id or name `{given}` in your current organization (see `axiom project list`); for a project in another organization pass --org <organization-slug> with the project's exact slug"
        ))),
        _ => Err(usage(format!("more than one project is named `{given}`; pass its slug"))),
    }
}

/// `--project <slug> --org <organization>`: a project outside the current organization.
/// The Cloud API lists projects of the current organization only and has no list per
/// organization, so the value must be the project's exact slug (slugs are unique across
/// Axiom); `GET /api/v1/projects/<slug>` confirms it exists and that you may use it.
/// Names and ids are not resolved here. The API does not say which organization a
/// project belongs to, so `--org` cannot be checked against it.
async fn project_in_org(client: &CloudClient, given: &str, org: &str) -> Result<String> {
    if !org_slug_shaped(org) {
        return Err(usage(format!("--org `{org}` is not an organization slug")));
    }
    if !project_slug_shaped(given) {
        return Err(usage(format!("with --org, --project must be the project's exact slug; `{given}` is not a slug")));
    }
    match client.project_auth_request(Method::GET, given, "", None, None).await {
        Ok(project) if project["slug"].as_str() == Some(given) => Ok(given.to_string()),
        Ok(_) | Err(AuthAdminError::Forbidden { .. } | AuthAdminError::Invalid { .. }) => Err(usage(format!(
            "no project with the slug `{given}` that you can use (looked it up for organization `{org}`; with --org, --project must be the project's exact slug)"
        ))),
        Err(error) => Err(admin(error)),
    }
}

/// The backend's slug patterns: projects `^[a-z0-9]+(?:-[a-z0-9]+)*$`, organizations
/// (workspaces) `^[a-z0-9][a-z0-9-]{1,61}[a-z0-9]$`.
fn project_slug_shaped(value: &str) -> bool {
    value.len() <= 255 && !value.is_empty() && value.split('-').all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
}

fn org_slug_shaped(value: &str) -> bool {
    let bytes = value.as_bytes();
    (3..=63).contains(&bytes.len())
        && bytes.iter().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        && bytes[0] != b'-'
        && bytes[bytes.len() - 1] != b'-'
}

/// A name resolves through `configuration-environments`; a UUID passes through.
pub async fn resolve_environment(client: &CloudClient, slug: &str, name: &str) -> Result<String> {
    if uuid_shaped(name) {
        return Ok(name.to_string());
    }
    let environments = client.configuration_environments(slug).await.map_err(admin)?;
    pick_environment(&environments, name)
}

pub fn pick_environment(environments: &[Value], name: &str) -> Result<String> {
    environments
        .iter()
        .find(|e| e["slug"].as_str() == Some(name))
        .and_then(|e| e["id"].as_str())
        .filter(|id| uuid_shaped(id))
        .map(str::to_string)
        .ok_or_else(|| usage(format!("unknown environment `{name}`; pass a configured environment name or UUID")))
}

impl Target {
    async fn call(&self, method: Method, rest: &str, query: &[(&str, String)], body: Option<&Value>) -> Result<Value> {
        self.client
            .auth_admin_request(method, &self.slug, &self.environment, rest, query, body)
            .await
            .map_err(admin)
    }
}

/// Keys whose values are never printed or written: secrets, credentials,
/// hashes, sealed material, matched as whole names or `_`-suffixes so settings
/// such as `password_min_length` remain. Booleans such as `client_secret_set`
/// remain too.
const DENIED: [&str; 11] = ["secret", "password", "token", "phc", "hash", "ciphertext", "wrapped_key", "nonce", "seed", "pending_phc", "credential"];

fn denied(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    DENIED.iter().any(|d| key == *d || key.ends_with(&format!("_{d}")) || key.starts_with(&format!("{d}_hash")))
}

/// `j***@e***.com`.
pub fn mask_email(value: &str) -> String {
    let Some((local, domain)) = value.split_once('@') else { return value.to_string() };
    let first = |s: &str| s.chars().next().map(String::from).unwrap_or_default();
    let tld = domain.rsplit_once('.').map(|(_, t)| t).unwrap_or("");
    format!("{}***@{}***.{}", first(local), first(domain), tld)
}

fn looks_like_email(value: &str) -> bool {
    value.split_once('@').is_some_and(|(local, domain)| {
        !local.is_empty() && domain.contains('.') && !value.contains(char::is_whitespace) && !value.contains('/')
    })
}

/// Remove secret fields and, unless `reveal`, mask every address.
pub fn sanitize(value: &mut Value, reveal: bool) {
    match value {
        Value::Object(map) => {
            map.retain(|key, value| value.is_boolean() || !denied(key));
            for value in map.values_mut() {
                sanitize(value, reveal);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|v| sanitize(v, reveal)),
        Value::String(text) if !reveal && looks_like_email(text) => *text = mask_email(text),
        _ => {}
    }
}

fn print(args: &AuthArgs, value: &Value, human: impl FnOnce(&Value) -> Vec<String>) -> Result<()> {
    if args.json {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        for line in human(value) {
            println!("{line}");
        }
    }
    Ok(())
}

fn text(value: &Value, key: &str) -> String {
    match &value[key] {
        Value::Null => "-".into(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Refusal of a confirmation that cannot be asked: there is no terminal and no `--yes`.
pub(crate) const NEEDS_YES: &str = "pass --yes to confirm without a prompt";

/// `Some(yes)` for a command that asks before it changes or removes something
/// (`yes` is its `--yes`); `None` for every other command.
fn confirmation(action: &Action) -> Option<bool> {
    use advanced::{HooksAction, SmsAction, TrustedIssuersAction, WebhooksAction};
    use email::{EmailAction, SmtpAction, TemplatesAction};
    use orgs::{InvitationsAction, OrganizationsAction};
    use rbac::{AssignmentsAction, PermissionsAction, RolesAction};
    use users_admin::UsersAdminAction;
    let yes = match action {
        Action::Providers { action: ProvidersAction::Remove { yes, .. } }
        | Action::Users { action: UsersAction::ResetFactors { yes, .. } | UsersAction::RevokeConsent { yes, .. } | UsersAction::Revoke { yes, .. } }
        | Action::Users { action: UsersAction::Admin(UsersAdminAction::Delete { yes, .. } | UsersAdminAction::RemoveFactor { yes, .. }) }
        | Action::Organizations { action: OrganizationsAction::RemoveMember { yes, .. } }
        | Action::Invitations { action: InvitationsAction::Revoke { yes, .. } }
        | Action::Roles { action: RolesAction::Delete { yes, .. } }
        | Action::Permissions { action: PermissionsAction::Delete { yes, .. } }
        | Action::Assignments { action: AssignmentsAction::Remove { yes, .. } }
        | Action::Email { action: EmailAction::Unsuppress { yes, .. } }
        | Action::Smtp { action: SmtpAction::Remove { yes } }
        | Action::Templates { action: TemplatesAction::Reset { yes, .. } }
        | Action::TrustedIssuers { action: TrustedIssuersAction::Remove { yes, .. } }
        | Action::Webhooks { action: WebhooksAction::Delete { yes, .. } }
        | Action::Hooks { action: HooksAction::Remove { yes, .. } }
        | Action::Sms { action: SmsAction::Remove { yes, .. } } => yes,
        // Suspending asks; reactivating does not.
        Action::Organizations { action: OrganizationsAction::Update { status, yes, .. } | OrganizationsAction::AddMember { status, yes, .. } } if status == "suspended" => yes,
        _ => return None,
    };
    Some(*yes)
}

/// Before signing in, resolving the project or sending anything: a confirmation that
/// cannot be asked is refused (exit 2), as is an OAuth client delete without its
/// typed `--confirm` phrase. The terminal prompt itself comes later, next to the change.
fn preflight(action: &Action) -> Result<()> {
    if confirmation(action) == Some(false) && !std::io::stdin().is_terminal() {
        return Err(usage(NEEDS_YES));
    }
    if let Action::OauthApps { action: advanced::OauthAppsAction::Delete { id, confirm } } = action {
        advanced::delete_client_phrase(id, confirm.as_deref())?;
    }
    Ok(())
}

async fn run(args: &AuthArgs) -> Result<()> {
    preflight(&args.action)?;
    match &args.action {
        Action::Jwks { issuer, check } => jwks(args, issuer.as_deref(), *check).await,
        Action::Check { manifest, deployment, backend } => check(args, manifest, deployment.as_deref(), backend.as_deref()).await,
        Action::Config { action: ConfigAction::Pull { manifest, write } } => config_pull(args, manifest, *write).await,
        Action::Config { action: ConfigAction::Promote(command) } => config::run(args, command).await,
        Action::Policy { action } => policy::run(args, action).await,
        Action::Emulate(emulate) => emulate::run(args, emulate).await,
        Action::Status => {
            let target = target(args).await?;
            let mut overview = target.call(Method::GET, "overview", &[], None).await?;
            sanitize(&mut overview, false);
            let epoch = match overview["issuer"].as_str() {
                Some(issuer) if valid_issuer(issuer) => public_issuer_get(issuer, "/epoch").await.ok(),
                _ => None,
            };
            let report = json!({
                "enabled": overview["enabled"],
                "auth_status": overview["auth_status"],
                "stream_version": overview["stream_version"],
                "upgrade_pending": overview["upgrade_pending"],
                "issuer": overview["issuer"],
                "users_count": overview["users_count"],
                "viewer": overview["viewer"],
                "epoch": epoch,
            });
            print(args, &report, |r| {
                vec![
                    format!("Authentication: {} ({})", if r["enabled"] == true { "enabled" } else { "disabled" }, text(r, "auth_status")),
                    format!("Stream version: {}{}", text(r, "stream_version"), if r["upgrade_pending"] == true { " (upgrade pending)" } else { "" }),
                    format!("Issuer: {}", text(r, "issuer")),
                    format!("Users: {}", text(r, "users_count")),
                    format!("Epoch: {}", r["epoch"]["epoch"].as_i64().map_or("unavailable".to_string(), |e| format!("{e} (keys_rev {})", r["epoch"]["keys_rev"]))),
                ]
            })
        }
        Action::Settings { action: SettingsAction::Get { output } } => {
            let target = target(args).await?;
            let mut settings = target.call(Method::GET, "settings", &[], None).await?;
            sanitize(&mut settings, true);
            let rendered = format!("{}\n", serde_json::to_string_pretty(&settings)?);
            match output {
                Some(path) => {
                    std::fs::write(path, rendered).with_context(|| format!("write {}", path.display()))?;
                    if !args.json {
                        println!("Settings written to {} (revision {}).", path.display(), text(&settings, "revision"));
                    }
                    Ok(())
                }
                None => {
                    print!("{rendered}");
                    Ok(())
                }
            }
        }
        Action::Settings { action: SettingsAction::Set { file, dry_run } } => {
            let body = settings_body(file)?;
            if *dry_run {
                let mut shown = body.clone();
                if shown.get("turnstile_secret").is_some_and(|v| !v.is_null()) {
                    shown["turnstile_secret"] = json!("(set from file)");
                }
                println!("{}", serde_json::to_string_pretty(&json!({"dry_run": true, "request": shown}))?);
                return Ok(());
            }
            let target = target(args).await?;
            let mut settings = target.call(Method::PUT, "settings", &[], Some(&body)).await?;
            sanitize(&mut settings, true);
            print(args, &settings, |s| vec![format!("Settings saved (revision {}).", text(s, "revision"))])
        }
        Action::Providers { action } => providers(args, action).await,
        Action::Users { action: UsersAction::Factors { user_id } } => advanced::user_factors(args, user_id).await,
        Action::Users { action: UsersAction::ResetFactors { user_id, reason, yes } } => advanced::reset_factors(args, user_id, reason, *yes).await,
        Action::Users { action: UsersAction::Consents { user_id } } => advanced::user_consents(args, user_id).await,
        Action::Users { action: UsersAction::RevokeConsent { user_id, consent_id, yes } } => advanced::revoke_consent(args, user_id, consent_id, *yes).await,
        Action::Users { action: UsersAction::Admin(action) } => users_admin::users(args, action).await,
        Action::Users { action } => users(args, action).await,
        Action::Roles { action } => rbac::roles(args, action).await,
        Action::Permissions { action } => rbac::permissions(args, action).await,
        Action::Assignments { action } => rbac::assignments(args, action).await,
        Action::Organizations { action } => orgs::organizations(args, action).await,
        Action::Invitations { action } => orgs::invitations(args, action).await,
        Action::Events { action } => users_admin::events(args, action).await,
        Action::Email { action } => email::email(args, action).await,
        Action::Smtp { action } => email::smtp(args, action).await,
        Action::Templates { action } => email::templates(args, action).await,
        Action::OauthApps { action } => advanced::oauth_apps(args, action).await,
        Action::Oidc { action } => advanced::oidc(args, action).await,
        Action::TrustedIssuers { action } => advanced::trusted_issuers(args, action).await,
        Action::Mfa { action } => advanced::mfa(args, action).await,
        Action::Webhooks { action } => advanced::webhooks(args, action).await,
        Action::Hooks { action } => advanced::hooks(args, action).await,
        Action::Sms { action } => advanced::sms(args, action).await,
        Action::Invite { email, roles, organization } => {
            let target = target(args).await?;
            let role_ids = if roles.is_empty() {
                vec![]
            } else {
                let listed = target.call(Method::GET, "roles", &[], None).await?;
                let items = listed.get("roles").cloned().unwrap_or(listed);
                roles
                    .iter()
                    .map(|name| {
                        items
                            .as_array()
                            .into_iter()
                            .flatten()
                            .find(|r| r["name"].as_str() == Some(name))
                            .and_then(|r| r["id"].as_str())
                            .map(str::to_string)
                            .ok_or_else(|| usage(format!("unknown role `{name}`")))
                    })
                    .collect::<Result<Vec<_>>>()?
            };
            let mut body = json!({"email": email});
            if !role_ids.is_empty() {
                body["role_ids"] = json!(role_ids);
            }
            if let Some(organization) = organization {
                body["organization_id"] = json!(organization);
            }
            let mut invitation = target.call(Method::POST, "invitations", &[], Some(&body)).await?;
            sanitize(&mut invitation, false);
            print(args, &invitation, |i| vec![format!("Invitation {} sent to {} ({}), expires {}.", text(i, "id"), text(i, "email"), text(i, "delivery_state"), text(i, "expires_at"))])
        }
    }
}

/// Fields `GET settings` returns that `PUT settings` does not accept.
const READ_ONLY_SETTINGS: [&str; 4] = ["updated_at", "turnstile_secret_set", "stream_version", "upgrade_pending"];

/// Settings from a file: an object with `revision`; provider and SMTP secret
/// fields are refused (they have their own commands and write-only routes).
pub fn settings_body(file: &Path) -> Result<Value> {
    let raw = std::fs::read(file).with_context(|| format!("read {}", file.display()))?;
    if raw.len() > 32 * 1024 {
        return Err(usage("settings files are at most 32 KiB"));
    }
    let body: Value = serde_json::from_slice(&raw).map_err(|_| usage("the settings file is not JSON"))?;
    let Value::Object(map) = &body else { return Err(usage("the settings file is a JSON object")) };
    if !map.get("revision").is_some_and(Value::is_u64) {
        return Err(usage("the settings file needs the current `revision` (from `axiom auth settings get`)"));
    }
    // Only secret fields are refused: provider client secrets, the SMTP password
    // and configuration, providers. `password_min_length` and the `smtp-env-*`
    // rate limits are ordinary settings.
    fn refused(map: &Map<String, Value>) -> Option<String> {
        for (key, value) in map {
            let lower = key.to_ascii_lowercase();
            let secret = lower != "turnstile_secret" && (lower.ends_with("_secret") || lower == "secret");
            if secret || lower == "password" || lower.ends_with("_password") || lower == "smtp" || lower == "providers" {
                return Some(key.clone());
            }
            if let Value::Object(inner) = value {
                if let Some(found) = refused(inner) {
                    return Some(found);
                }
            }
        }
        None
    }
    if let Some(key) = refused(map) {
        return Err(usage(format!("`{key}` is not a settings field; provider and SMTP secrets have their own commands")));
    }
    // Read-only fields of the GET view; PUT decodes strictly.
    let mut body = body;
    if let Value::Object(map) = &mut body {
        for key in READ_ONLY_SETTINGS {
            map.remove(key);
        }
    }
    Ok(body)
}

fn valid_issuer(issuer: &str) -> bool {
    axiom_auth_issuer(issuer, LOOPBACK_ISSUERS).is_some()
}

/// Parse `google`, `github`, `microsoft`, `apple` or `oidc-<slug>`.
pub fn provider_selector(value: &str) -> Result<(&'static str, Option<String>)> {
    match value {
        "google" => Ok(("google", None)),
        "github" => Ok(("github", None)),
        "microsoft" => Ok(("microsoft", None)),
        "apple" => Ok(("apple", None)),
        other => match other.strip_prefix("oidc-") {
            // The provider slug is the whole `oidc-<slug>` (auth-surfaces/v1 `auth_identity_providers.slug`);
            // stripping the prefix made create send a slug the server refuses and broke matching.
            Some(slug) if !slug.is_empty() && slug.len() <= 31 && slug.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') => {
                Ok(("oidc", Some(other.to_string())))
            }
            _ => Err(usage("providers are google, github, microsoft, apple or oidc-<slug>")),
        },
    }
}

fn find_provider<'a>(providers: &'a Value, kind: &str, slug: Option<&str>) -> Option<&'a Value> {
    providers["providers"].as_array()?.iter().find(|p| {
        p["kind"].as_str() == Some(kind) && (slug.is_none() || p["slug"].as_str() == slug)
    })
}

/// The client secret from stdin (bounded) or a masked prompt; never argv.
fn read_secret(stdin: bool) -> Result<Zeroizing<String>> {
    let secret = if stdin {
        let mut bytes = Zeroizing::new(Vec::new());
        std::io::stdin().take(4097).read_to_end(&mut bytes)?;
        if bytes.len() > 4096 {
            return Err(usage("client secrets are at most 4096 bytes"));
        }
        Zeroizing::new(String::from_utf8(bytes.to_vec()).map_err(|_| usage("the client secret is not UTF-8"))?)
    } else {
        if !std::io::stdin().is_terminal() {
            return Err(usage("no terminal for the masked prompt; pass --client-secret-stdin"));
        }
        Zeroizing::new(dialoguer::Password::new().with_prompt("Client secret").interact()?)
    };
    let trimmed = Zeroizing::new(secret.trim_end_matches(['\r', '\n']).to_string());
    if trimmed.is_empty() {
        return Err(usage("the client secret is empty"));
    }
    Ok(trimmed)
}

async fn providers(args: &AuthArgs, action: &ProvidersAction) -> Result<()> {
    if let ProvidersAction::Remove { provider, .. } = action {
        if !uuid_shaped(provider) {
            provider_selector(provider)?;
        }
    }
    let target = target(args).await?;
    providers_with(args, &target, action).await
}

async fn providers_with(args: &AuthArgs, target: &Target, action: &ProvidersAction) -> Result<()> {
    match action {
        ProvidersAction::List => {
            let mut listed = target.call(Method::GET, "providers", &[], None).await?;
            sanitize(&mut listed, true);
            print(args, &listed, |l| {
                let mut lines = vec![];
                for p in l["providers"].as_array().into_iter().flatten() {
                    let name = match p["kind"].as_str() {
                        Some("oidc") => text(p, "slug"),
                        _ => text(p, "kind"),
                    };
                    lines.push(format!(
                        "{name}  {}  client {}  secret {}  test {}",
                        if p["enabled"] == true { "enabled" } else { "disabled" },
                        text(p, "client_id"),
                        if p["client_secret_set"] == true { "configured" } else { "not configured" },
                        p["last_test"]["result"].as_str().unwrap_or("untested"),
                    ));
                }
                if lines.is_empty() {
                    lines.push("No providers configured.".into());
                }
                lines
            })
        }
        ProvidersAction::Set { provider, client_id, issuer, scopes, enable, disable, client_secret_stdin, tenant, allowed_tenants, team_id, key_id, private_key_file } => {
            let (kind, slug) = provider_selector(provider)?;
            if issuer.is_some() && kind != "oidc" {
                return Err(usage("--issuer applies to oidc-<slug> providers only"));
            }
            if kind == "apple" && *client_secret_stdin {
                return Err(usage("Apple takes no client secret; pass --private-key-file <AuthKey.p8>"));
            }
            let private_key = private_key_file.as_deref().map(advanced::apple_private_key).transpose()?;
            let scopes = scopes.as_ref().map(|s| s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect::<Vec<_>>());
            let listed = target.call(Method::GET, "providers", &[], None).await?;
            let existing = find_provider(&listed, kind, slug.as_deref());
            let extras = advanced::provider_extras(kind, tenant.as_deref(), allowed_tenants, team_id.as_deref(), key_id.as_deref(), private_key.is_some(), existing.is_none())?;
            let mut saved = match existing {
                None => {
                    let client_id = client_id.clone().ok_or_else(|| usage("--client-id is required for a new provider"))?;
                    if kind == "oidc" && issuer.is_none() {
                        return Err(usage("--issuer is required for a new OIDC provider"));
                    }
                    let mut body = json!({"kind": kind, "client_id": client_id, "enabled": *enable});
                    if kind == "apple" {
                        body["private_key"] = json!(private_key.as_ref().map(|k| k.as_str()));
                    } else {
                        let secret = read_secret(*client_secret_stdin)?;
                        body["client_secret"] = json!(secret.as_str());
                    }
                    for (key, value) in extras.as_object().into_iter().flatten() {
                        body[key] = value.clone();
                    }
                    if let Some(slug) = &slug {
                        body["slug"] = json!(slug);
                    }
                    if let Some(issuer) = issuer {
                        body["issuer"] = json!(issuer);
                    }
                    if let Some(scopes) = &scopes {
                        body["scopes"] = json!(scopes);
                    }
                    let result = target.call(Method::POST, "providers", &[], Some(&body)).await;
                    if let Value::Object(map) = &mut body {
                        map.remove("client_secret");
                        map.remove("private_key");
                    }
                    result?
                }
                Some(existing) => {
                    let id = existing["id"].as_str().ok_or_else(|| Exit { code: 5, message: "AXAUTH_UNAVAILABLE: provider without id".into() })?;
                    let mut body = json!({"revision": existing["revision"]});
                    if let Some(client_id) = client_id {
                        body["client_id"] = json!(client_id);
                    }
                    if let Some(scopes) = &scopes {
                        body["scopes"] = json!(scopes);
                    }
                    if *enable || *disable {
                        body["enabled"] = json!(*enable);
                    }
                    if issuer.is_some() {
                        return Err(usage("an OIDC provider's issuer never changes; delete and create it again"));
                    }
                    let secret = if *client_secret_stdin { Some(read_secret(true)?) } else { None };
                    if let Some(secret) = &secret {
                        body["client_secret"] = json!(secret.as_str());
                    }
                    if let Some(key) = &private_key {
                        body["private_key"] = json!(key.as_str());
                    }
                    for (key, value) in extras.as_object().into_iter().flatten() {
                        body[key] = value.clone();
                    }
                    if body.as_object().is_some_and(|m| m.len() == 1) {
                        return Err(usage("nothing to change: pass --client-id, --scopes, --enable, --disable or --client-secret-stdin"));
                    }
                    let result = target.call(Method::PATCH, &format!("providers/{id}"), &[], Some(&body)).await;
                    if let Value::Object(map) = &mut body {
                        map.remove("client_secret");
                        map.remove("private_key");
                    }
                    result?
                }
            };
            sanitize(&mut saved, true);
            print(args, &saved, |p| {
                let mut lines = vec![format!("Provider {provider} saved ({}).", if p["enabled"] == true { "enabled" } else { "disabled" })];
                if p["warning"].as_str() == Some("pairwise_subjects") {
                    lines.push("Warning: providers that issue a different subject per client ID split existing accounts.".into());
                }
                if p["warning"].as_str() == Some("any_tenant") {
                    lines.push("Warning: any Microsoft tenant can sign in; restrict it with --allowed-tenant.".into());
                }
                if p["available"] == false {
                    lines.push("Note: this provider kind is not served on this platform yet; it stays unavailable to users.".into());
                }
                lines
            })
        }
        ProvidersAction::Test { provider } => {
            let (kind, slug) = provider_selector(provider)?;
            let listed = target.call(Method::GET, "providers", &[], None).await?;
            let id = find_provider(&listed, kind, slug.as_deref())
                .and_then(|p| p["id"].as_str())
                .ok_or_else(|| usage(format!("provider {provider} is not configured")))?
                .to_string();
            let mut result = target.call(Method::POST, &format!("providers/{id}/test"), &[], Some(&json!({}))).await?;
            sanitize(&mut result, true);
            print(args, &result, |r| {
                vec![match r["result"].as_str() {
                    Some("ok") => format!("{provider}: ok"),
                    Some("untested") => format!("{provider}: configured, untested"),
                    Some(other) => format!("{provider}: {other}"),
                    None => format!("{provider}: unknown result"),
                }]
            })
        }
        ProvidersAction::Remove { provider, yes } => {
            let listed = target.call(Method::GET, "providers", &[], None).await?;
            let found = if uuid_shaped(provider) {
                listed["providers"].as_array().into_iter().flatten().find(|p| p["id"].as_str() == Some(provider.as_str()))
            } else {
                let (kind, slug) = provider_selector(provider)?;
                find_provider(&listed, kind, slug.as_deref())
            };
            let found = found.ok_or_else(|| usage(format!("provider {provider} is not configured")))?;
            if found["enabled"] == true {
                return Err(Exit { code: 4, message: format!("AXAUTH_CONFLICT: provider {provider} is enabled; disable it first with `axiom auth providers set {provider} --disable`") }.into());
            }
            let id = checked_id(found["id"].as_str().unwrap_or_default())?.to_string();
            advanced::confirm_yes(*yes, &format!("Delete provider {provider}? Its configuration and client secret are removed; users who signed in with it keep their accounts."))?;
            target.call(Method::DELETE, &format!("providers/{id}"), &[], None).await?;
            print(args, &json!({"deleted": id, "provider": provider}), |_| vec![format!("Provider {provider} deleted.")])
        }
    }
}

fn user_line(u: &Value) -> String {
    format!(
        "{}  {}  {}  {}  sessions {}",
        text(u, "id"),
        text(u, "email"),
        text(u, "status"),
        text(u, "kind"),
        text(u, "active_sessions")
    )
}

async fn users(args: &AuthArgs, action: &UsersAction) -> Result<()> {
    let target = target(args).await?;
    match action {
        UsersAction::List { search, status, limit, cursor, reveal_email } => {
            let mut listed = match search {
                Some(id) if uuid_shaped(id) => {
                    let mut query = vec![("user_id", id.clone())];
                    if let Some(status) = status {
                        query.push(("status", status.clone()));
                    }
                    if let Some(limit) = limit {
                        query.push(("limit", limit.to_string()));
                    }
                    if let Some(cursor) = cursor {
                        query.push(("cursor", cursor.clone()));
                    }
                    target.call(Method::GET, "users", &query, None).await?
                }
                Some(prefix) => {
                    let mut body = json!({"email_prefix": prefix});
                    if let Some(status) = status {
                        body["status"] = json!(status);
                    }
                    if let Some(limit) = limit {
                        body["limit"] = json!(limit);
                    }
                    if let Some(cursor) = cursor {
                        body["cursor"] = json!(cursor);
                    }
                    target.call(Method::POST, "users/search", &[], Some(&body)).await?
                }
                None => {
                    let mut query = vec![];
                    if let Some(status) = status {
                        query.push(("status", status.clone()));
                    }
                    if let Some(limit) = limit {
                        query.push(("limit", limit.to_string()));
                    }
                    if let Some(cursor) = cursor {
                        query.push(("cursor", cursor.clone()));
                    }
                    target.call(Method::GET, "users", &query, None).await?
                }
            };
            sanitize(&mut listed, *reveal_email);
            print(args, &listed, |l| {
                let mut lines = l["users"].as_array().into_iter().flatten().map(user_line).collect::<Vec<_>>();
                if lines.is_empty() {
                    lines.push("No users.".into());
                }
                if let Some(next) = l["next_cursor"].as_str() {
                    lines.push(format!("More: --cursor {next}"));
                }
                lines
            })
        }
        UsersAction::Show { user_id, reveal_email } => {
            let mut user = target.call(Method::GET, &format!("users/{}", checked_id(user_id)?), &[], None).await?;
            sanitize(&mut user, *reveal_email);
            print(args, &user, |u| {
                let mut lines = vec![user_line(u)];
                for identity in u["identities"].as_array().into_iter().flatten() {
                    lines.push(format!("  identity {} {}", text(identity, "provider"), text(identity, "email")));
                }
                lines
            })
        }
        UsersAction::Ban { user_id, reason } => {
            if reason.trim().is_empty() {
                return Err(usage("--reason cannot be empty"));
            }
            let mut user = target.call(Method::POST, &format!("users/{}/ban", checked_id(user_id)?), &[], Some(&json!({"reason": reason}))).await?;
            sanitize(&mut user, false);
            print(args, &user, |u| vec![format!("Banned {}; sessions end at their next refresh.", text(u, "id"))])
        }
        UsersAction::Unban { user_id } => {
            let mut user = target.call(Method::POST, &format!("users/{}/unban", checked_id(user_id)?), &[], Some(&json!({}))).await?;
            sanitize(&mut user, false);
            print(args, &user, |u| vec![format!("Unbanned {}.", text(u, "id"))])
        }
        UsersAction::Factors { .. } | UsersAction::ResetFactors { .. } | UsersAction::Consents { .. } | UsersAction::RevokeConsent { .. } | UsersAction::Admin(..) => unreachable!("dispatched in run"),
        UsersAction::Revoke { user_id, session, yes } => {
            let user_id = checked_id(user_id)?;
            if !yes {
                if !std::io::stdin().is_terminal() {
                    return Err(usage(NEEDS_YES));
                }
                let what = session.as_ref().map_or("every session".to_string(), |s| format!("session {s}"));
                if !dialoguer::Confirm::new().with_prompt(format!("End {what} of user {user_id}?")).default(false).interact()? {
                    return Err(usage("cancelled"));
                }
            }
            let body = match session {
                Some(sid) => json!({"session_id": checked_id(sid)?}),
                None => json!({}),
            };
            let ended = target.call(Method::POST, &format!("users/{user_id}/sessions/revoke"), &[], Some(&body)).await?;
            print(args, &ended, |_| vec!["Sessions ended.".to_string()])
        }
    }
}

fn checked_id(value: &str) -> Result<&str> {
    if uuid_shaped(value) {
        Ok(value)
    } else {
        Err(usage(format!("`{value}` is not a lowercase UUID")))
    }
}

/// `--check`: the JWKS must carry an `axiom_keys_rev` at least the epoch's
/// `keys_rev` (auth-core "Epoch").
pub fn jwks_report(jwks: &Value, epoch: &Value) -> Value {
    let keys = jwks["keys"].as_array().cloned().unwrap_or_default();
    let jwks_rev = jwks["axiom_keys_rev"].as_i64();
    let epoch_rev = epoch["keys_rev"].as_i64();
    let consistent = matches!((jwks_rev, epoch_rev), (Some(j), Some(e)) if j >= e);
    json!({
        "kids": keys.iter().filter_map(|k| k["kid"].as_str()).collect::<Vec<_>>(),
        "axiom_keys_rev": jwks_rev,
        "epoch": epoch["epoch"],
        "keys_rev": epoch_rev,
        "emergency": epoch["emergency"],
        "consistent": consistent,
    })
}

async fn jwks(args: &AuthArgs, issuer: Option<&str>, check: bool) -> Result<()> {
    let (issuer, target) = match issuer {
        Some(issuer) => (issuer.to_string(), None),
        None => {
            let target = target(args).await?;
            let overview = target.call(Method::GET, "overview", &[], None).await?;
            let issuer = overview["issuer"].as_str().map(str::to_string).ok_or_else(|| Exit { code: 5, message: "AXAUTH_UNAVAILABLE: the environment reports no issuer".into() })?;
            (issuer, Some(target))
        }
    };
    if !valid_issuer(&issuer) {
        return Err(usage("issuers are https://auth.axiomcore.dev/<project_id>/<environment_id>"));
    }
    let jwks = public_issuer_get(&issuer, "/.well-known/jwks.json").await.map_err(admin)?;
    let epoch = public_issuer_get(&issuer, "/epoch").await.map_err(admin)?;
    let mut report = jwks_report(&jwks, &epoch);
    report["issuer"] = json!(issuer);
    if let Some(target) = &target {
        if let Ok(mut keys) = target.call(Method::GET, "keys", &[], None).await {
            sanitize(&mut keys, true);
            report["keys"] = keys.get("keys").cloned().unwrap_or(keys);
        }
    }
    print(args, &report, |r| {
        let mut lines = vec![
            format!("Issuer: {}", text(r, "issuer")),
            format!("Keys: {}", r["kids"].as_array().map(|k| k.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default()),
            format!("JWKS axiom_keys_rev {} / epoch {} keys_rev {}", text(r, "axiom_keys_rev"), text(r, "epoch"), text(r, "keys_rev")),
        ];
        if check {
            lines.push(if r["consistent"] == true { "Check: consistent".into() } else { "Check: the JWKS is behind the epoch".into() });
        }
        lines
    })?;
    if check && report["consistent"] != true {
        return Err(Exit { code: 5, message: "AXAUTH_KEYS_STALE: the published JWKS is behind the epoch's keys_rev".into() }.into());
    }
    Ok(())
}

fn read_manifest(path: &Path) -> Result<(String, Option<AuthDeclaration>)> {
    let source = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let manifest = parse_axiom_deps_v2(&source).map_err(|e| usage(format!("{}: {e:#}", path.display())))?;
    Ok((source, manifest.auth))
}

/// The `[auth.environments.<env>]` values implied by the environment settings.
pub fn environment_block(overview: &Value, settings: &Value, callback_path: &str) -> Result<Map<String, Value>> {
    let issuer = overview["issuer"].as_str().ok_or_else(|| Exit { code: 5, message: "AXAUTH_UNAVAILABLE: the environment reports no issuer".into() })?;
    let allowed = settings["allowed_origins"].as_array().cloned().unwrap_or_default();
    let candidates = settings["redirect_urls"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|u| reqwest::Url::parse(u).ok())
        .filter(|u| u.path() == callback_path && u.query().is_none())
        .map(|u| u.origin().ascii_serialization())
        .collect::<Vec<_>>();
    let origin = candidates
        .iter()
        .find(|o| allowed.iter().any(|a| a.as_str() == Some(o.as_str())))
        .or(candidates.first())
        .cloned()
        .ok_or_else(|| usage(format!("no redirect URL of this environment ends with {callback_path}; add <origin>{callback_path} to redirect_urls")))?;
    let mut block = Map::new();
    block.insert("issuer".into(), json!(issuer));
    if let Some(audience) = settings["audiences"].as_array().and_then(|a| a.first()).and_then(Value::as_str) {
        if audience != format!("{issuer}/api") {
            block.insert("audience".into(), json!(audience));
        }
    }
    block.insert("redirectOrigin".into(), json!(origin));
    if let Some(logout) = settings["logout_urls"].as_array().and_then(|a| a.first()).and_then(Value::as_str) {
        block.insert("logoutUri".into(), json!(logout));
    }
    // auth-advanced/v1 17.3: the passkey relying party and the OAuth discovery URL.
    if passkeys_enabled(settings) {
        if let Some(rp_id) = settings["webauthn_rp_id"].as_str() {
            block.insert("webauthnRpId".into(), json!(rp_id));
        }
    }
    if overview["oauth_served"] == true {
        block.insert("oauthDiscovery".into(), json!(format!("{issuer}/.well-known/openid-configuration")));
    }
    Ok(block)
}

fn passkeys_enabled(settings: &Value) -> bool {
    let has = |key: &str| settings[key].as_array().is_some_and(|a| a.iter().any(|m| m.as_str() == Some("passkey")));
    has("enabled_methods") || has("mfa_factors")
}

/// The manifest with `[auth.environments.<name>]` replaced (comments kept).
pub fn write_environment(source: &str, name: &str, block: &Map<String, Value>) -> Result<String> {
    let mut document = source.parse::<toml_edit::DocumentMut>().map_err(|_| usage("the manifest is not valid TOML"))?;
    if document.get("auth").is_none() {
        let mut auth = toml_edit::Table::new();
        auth.insert("provider", toml_edit::value("axiom"));
        auth.insert("callbackPath", toml_edit::value("/auth/callback"));
        document.insert("auth", toml_edit::Item::Table(auth));
    }
    let auth = document["auth"].as_table_mut().ok_or_else(|| usage("`auth` must be a table"))?;
    if auth.get("environments").is_none() {
        let mut environments = toml_edit::Table::new();
        environments.set_implicit(true);
        auth.insert("environments", toml_edit::Item::Table(environments));
    }
    let environments = auth["environments"].as_table_mut().ok_or_else(|| usage("`auth.environments` must be a table"))?;
    let mut table = toml_edit::Table::new();
    for (key, value) in block {
        if let Some(text) = value.as_str() {
            table.insert(key, toml_edit::value(text));
        }
    }
    environments.insert(name, toml_edit::Item::Table(table));
    let rendered = document.to_string();
    parse_axiom_deps_v2(&rendered).map_err(|e| usage(format!("the updated manifest would be invalid: {e:#}")))?;
    Ok(rendered)
}

async fn config_pull(args: &AuthArgs, manifest: &Path, write: bool) -> Result<()> {
    let (source, auth) = read_manifest(manifest)?;
    let callback = auth.as_ref().map_or("/auth/callback".to_string(), |a| a.callback_path.clone());
    let target = target(args).await?;
    let mut overview = target.call(Method::GET, "overview", &[], None).await?;
    let settings = target.call(Method::GET, "settings", &[], None).await?;
    // OAuth is served when the issuer publishes OpenID discovery.
    if let Some(issuer) = overview["issuer"].as_str().filter(|i| valid_issuer(i)).map(str::to_string) {
        overview["oauth_served"] = json!(public_issuer_get(&issuer, "/.well-known/openid-configuration").await.is_ok());
    }
    let name = &args.environment;
    let block = environment_block(&overview, &settings, &callback)?;
    if write {
        let updated = write_environment(&source, name, &block)?;
        let temporary = manifest.with_extension("toml.axiom-tmp");
        std::fs::write(&temporary, &updated)?;
        std::fs::rename(&temporary, manifest)?;
        if !args.json {
            println!("Updated [auth.environments.{name}] in {}.", manifest.display());
        }
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&json!({"environment": name, "block": block, "written": write}))?);
    } else if !write {
        println!("[auth.environments.{name}]");
        for (key, value) in &block {
            println!("{key} = {value}");
        }
    }
    Ok(())
}

/// Read-only comparison of a manifest environment, its settings and a deployment.
pub fn compare(
    auth: &AuthDeclaration,
    environment: &str,
    overview: &Value,
    settings: &Value,
    deployment: Option<&Value>,
) -> Vec<String> {
    let mut warnings = vec![];
    let Some(selected) = auth.environments.get(environment) else {
        return vec![format!("the manifest has no [auth.environments.{environment}]")];
    };
    if overview["issuer"].as_str().is_some_and(|i| i != selected.issuer) {
        warnings.push("the manifest issuer differs from the environment's issuer".into());
    }
    let audience = selected.effective_audience();
    if !settings["audiences"].as_array().is_some_and(|a| a.iter().any(|v| v.as_str() == Some(audience.as_str()))) {
        warnings.push(format!("audience {audience} is not one of the environment's audiences"));
    }
    let redirect = auth.redirect_uri(environment).unwrap_or_default();
    if !settings["redirect_urls"].as_array().is_some_and(|a| a.iter().any(|v| v.as_str() == Some(redirect.as_str()))) {
        warnings.push(format!("{redirect} is not in the environment's redirect_urls"));
    }
    if !settings["allowed_origins"].as_array().is_some_and(|a| a.iter().any(|v| v.as_str() == Some(selected.redirect_origin.as_str()))) {
        warnings.push(format!("{} is not in the environment's allowed_origins", selected.redirect_origin));
    }
    if let Some(logout) = &selected.logout_uri {
        if !settings["logout_urls"].as_array().is_some_and(|a| a.iter().any(|v| v.as_str() == Some(logout.as_str()))) {
            warnings.push(format!("{logout} is not in the environment's logout_urls"));
        }
    }
    if passkeys_enabled(settings) && !settings["webauthn_origins"].as_array().is_some_and(|a| a.iter().any(|v| v.as_str() == Some(selected.redirect_origin.as_str()))) {
        warnings.push(format!("passkeys are enabled but {} is not in the environment's webauthn_origins", selected.redirect_origin));
    }
    if let Some(deployment) = deployment {
        let provider = &deployment["authentication"];
        if provider["kind"].as_str() != Some("axiomAuth") {
            warnings.push("the deployment does not use the axiomAuth provider".into());
        } else {
            if provider["issuer"].as_str() != Some(selected.issuer.as_str()) {
                warnings.push("the deployment's axiomAuth issuer differs from the manifest issuer".into());
            }
            if provider["audience"].as_str() != Some(audience.as_str()) {
                warnings.push("the deployment's axiomAuth audience differs from the manifest audience".into());
            }
        }
    }
    warnings
}

/// Endpoints whose handled action has `require(guard:)` but no `auth(...)`
/// clause: the compiler derives none, so the web client never attaches the
/// managed bearer and every call answers 401.
pub fn guarded_without_auth(contract: &Value) -> Vec<String> {
    let actions = &contract["server"]["actions"];
    let mut found = contract["server"]["endpointPolicies"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(endpoint, policy)| {
            let action = policy["handle"]["action"].as_str()?;
            let guarded = actions[action]["settings"]["guards"].as_array().is_some_and(|g| !g.is_empty())
                || policy["guards"].as_array().is_some_and(|g| !g.is_empty());
            let auth = &contract["endpoints"][endpoint]["auth"];
            (guarded && auth.is_null()).then(|| endpoint.clone())
        })
        .collect::<Vec<_>>();
    found.sort();
    found
}

fn backend_contract(path: &Path) -> Result<Value> {
    let source = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let absolute = std::fs::canonicalize(path)?;
    let uri = format!("file://{}", absolute.display());
    let mut open = std::collections::HashMap::new();
    if let Some(deps) = absolute.parent().map(|p| p.join("AxiomDeps.toml")).filter(|p| p.is_file()) {
        open.insert(format!("file://{}", deps.display()), std::fs::read_to_string(&deps)?);
    }
    let inspection = acore::backend::inspection::compile(&uri, &source, &open, false, None)
        .map_err(|error| usage(format!("{}: {error}", path.display())))?;
    Ok(inspection.contract)
}

async fn check(args: &AuthArgs, manifest: &Path, deployment: Option<&Path>, backend: Option<&Path>) -> Result<()> {
    let (_, auth) = read_manifest(manifest)?;
    let auth = auth.ok_or_else(|| usage(format!("{} has no [auth] block", manifest.display())))?;
    let deployment = deployment
        .map(|path| -> Result<Value> {
            let raw = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
            serde_json::from_slice(&raw).map_err(|_| usage("the deployment is not JSON"))
        })
        .transpose()?;
    let target = target(args).await?;
    let overview = target.call(Method::GET, "overview", &[], None).await?;
    let settings = target.call(Method::GET, "settings", &[], None).await?;
    let mut warnings = compare(&auth, &args.environment, &overview, &settings, deployment.as_ref());
    if let Some(backend) = backend {
        for endpoint in guarded_without_auth(&backend_contract(backend)?) {
            warnings.push(format!("guarded endpoint `{endpoint}` has no auth(...) clause; the web client will not attach the access token"));
        }
    }
    let report = json!({"environment": args.environment, "warnings": warnings, "consistent": warnings.is_empty()});
    print(args, &report, |r| {
        let warnings = r["warnings"].as_array().cloned().unwrap_or_default();
        if warnings.is_empty() {
            vec!["Manifest, settings and deployment agree.".into()]
        } else {
            warnings.iter().filter_map(Value::as_str).map(|w| format!("warning: {w}")).collect()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[derive(clap::Parser)]
    struct Harness {
        #[command(flatten)]
        auth: AuthArgs,
    }

    fn every_argument(command: &clap::Command, out: &mut Vec<String>) {
        for argument in command.get_arguments() {
            if let Some(long) = argument.get_long() {
                out.push(long.to_string());
            }
        }
        for sub in command.get_subcommands() {
            every_argument(sub, out);
        }
    }

    #[test]
    fn no_secret_value_is_ever_an_argument() {
        let command = Harness::command();
        command.clone().debug_assert();
        let mut arguments = vec![];
        every_argument(&command, &mut arguments);
        assert!(arguments.contains(&"client-secret-stdin".to_string()));
        // Secret input is stdin or a file; one-time secrets go to a file; the rest name
        // identifiers, never values (auth-advanced/v1 17.3).
        let allowed = ["client-secret-stdin", "auth-token-stdin", "api-key-secret-stdin", "private-key-file", "token-file", "output-secret", "key-id", "api-key-sid", "idempotency-key", "signing-key-file", "password-stdin", "password-prompt"];
        for argument in &arguments {
            let secretish = ["secret", "password", "token", "key"].iter().any(|word| argument.contains(word));
            assert!(!secretish || allowed.contains(&argument.as_str()), "secret-bearing flag --{argument}");
        }
        for refused in [
            &["t", "oauth-apps", "rotate-secret", "x", "--secret", "s"][..],
            &["t", "sms", "set", "--auth-token", "t"],
            &["t", "trusted-issuers", "test", "x", "--token", "t"],
            &["t", "providers", "set", "apple", "--private-key", "k"],
            &["t", "smtp", "set", "--password", "p"],
        ] {
            assert!(Harness::try_parse_from(refused).is_err(), "{refused:?}");
        }
        use clap::Parser;
        assert!(Harness::try_parse_from(["t", "providers", "set", "google", "--client-secret", "x"]).is_err());
        assert!(Harness::try_parse_from(["t", "users", "list", "--limit", "0"]).is_err());
        assert!(Harness::try_parse_from(["t", "users", "list", "--limit", "101"]).is_err());
        assert!(Harness::try_parse_from(["t", "users", "list", "--status", "gone"]).is_err());
        assert!(Harness::try_parse_from(["t", "--environment", "production", "users", "list", "--limit", "100"]).is_ok());
    }

    #[test]
    fn output_is_masked_and_never_carries_secrets() {
        assert_eq!(mask_email("jane@example.com"), "j***@e***.com");
        assert_eq!(mask_email("a.b@mail.example.co.uk"), "a***@m***.uk");
        let mut value = json!({
            "users": [{"id": "u", "email": "jane@example.com", "has_password": true}],
            "client_secret": "SYNTHETIC_SECRET_CANARY", "client_secret_set": true,
            "refresh_token": "axr1_x", "password_hash": "$argon2id$", "nested": {"turnstile_secret": "x", "pending_phc": "y"},
            "callback_url": "https://auth.axiomcore.dev/p/e/v1/providers/google/callback"
        });
        sanitize(&mut value, false);
        let text = value.to_string();
        for canary in ["SYNTHETIC_SECRET_CANARY", "axr1_", "argon2id", "jane@example.com", "pending_phc", "turnstile_secret"] {
            assert!(!text.contains(canary), "{canary} in {text}");
        }
        assert_eq!(value["users"][0]["email"], "j***@e***.com");
        assert_eq!(value["client_secret_set"], true);
        assert_eq!(value["users"][0]["has_password"], true);
        assert!(value["callback_url"].as_str().unwrap().starts_with("https://"));
        let mut revealed = json!({"email": "jane@example.com"});
        sanitize(&mut revealed, true);
        assert_eq!(revealed["email"], "jane@example.com");
    }

    #[test]
    fn settings_files_need_a_revision_and_refuse_provider_and_smtp_secrets() {
        let directory = tempfile::tempdir().unwrap();
        let write = |body: &str| {
            let path = directory.path().join("settings.json");
            std::fs::write(&path, body).unwrap();
            path
        };
        let ok = settings_body(&write(r#"{"revision":3,"enabled":true,"turnstile_secret":"t"}"#)).unwrap();
        assert_eq!(ok["revision"], 3);
        for refused in [
            r#"{"enabled":true}"#,
            r#"{"revision":3,"client_secret":"x"}"#,
            r#"{"revision":3,"smtp":{"password":"x"}}"#,
            r#"{"revision":3,"smtp_password":"x"}"#,
            r#"{"revision":3,"providers":[]}"#,
            r#"{"revision":3,"nested":{"client_secret":"x"}}"#,
            "not json",
        ] {
            let error = settings_body(&write(refused)).unwrap_err();
            assert_eq!(error.downcast_ref::<Exit>().unwrap().code, 2, "{refused}");
        }
    }

    #[test]
    fn settings_round_trip_keeps_password_min_length_and_drops_read_only_fields() {
        let mut view = json!({"revision": 4, "enabled": true, "password_min_length": 12, "access_ttl_seconds": 900,
            "rate_limits": {"smtp-env-hour": 10}, "turnstile_secret_set": true, "stream_version": 2, "upgrade_pending": false, "updated_at": "2026-10-27T00:00:00Z"});
        sanitize(&mut view, true);
        assert_eq!(view["password_min_length"], 12);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        std::fs::write(&path, view.to_string()).unwrap();
        let body = settings_body(&path).unwrap();
        assert_eq!(body["password_min_length"], 12);
        assert_eq!(body["rate_limits"]["smtp-env-hour"], 10);
        for absent in READ_ONLY_SETTINGS {
            assert!(body.get(absent).is_none(), "{absent}");
        }
    }

    #[test]
    fn check_reports_guarded_endpoints_without_an_auth_clause() {
        let contract = json!({
            "endpoints": {"me": {"auth": null}, "secured": {"auth": {"required": true}}, "open": {"auth": null}},
            "server": {"actions": {"whoami": {"settings": {"guards": ["signedIn"]}}, "health": {"settings": {"guards": []}}},
                "endpointPolicies": {"me": {"handle": {"action": "whoami"}}, "secured": {"handle": {"action": "whoami"}}, "open": {"handle": {"action": "health"}}}}
        });
        assert_eq!(guarded_without_auth(&contract), vec!["me".to_string()]);
    }

    #[test]
    fn exit_codes_follow_the_contract() {
        let status = |code: u16| exit_code(&axiom_cloud::auth_admin::classify(code, b"{}"));
        assert_eq!(status(403), 3);
        assert_eq!(status(409), 4);
        assert_eq!(status(503), 5);
        assert_eq!(status(400), 2);
        assert_eq!(exit_code(&AuthAdminError::Unauthenticated), 3);
    }

    #[test]
    fn projects_resolve_by_slug_id_or_unique_name() {
        let project = |id: &str, name: &str, slug: &str| axiom_cloud::project::Project {
            id: id.into(),
            name: name.into(),
            slug: slug.into(),
            visibility: "private".into(),
            description: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
        };
        let id = "6a0c1f8e-3b2d-4c5e-9f70-1a2b3c4d5e6f";
        let projects = [project(id, "Web Shop", "shop"), project("p2", "Twin", "twin-a"), project("p3", "twin", "twin-b"), project("p4", "storefront", "shop-eu")];
        assert_eq!(pick_project(&projects, "shop").unwrap(), "shop");
        assert_eq!(pick_project(&projects, id).unwrap(), "shop");
        // A lowercase display name that differs from its slug is a name, not a slug.
        assert_eq!(pick_project(&projects, "storefront").unwrap(), "shop-eu");
        assert_eq!(pick_project(&projects, "twin").unwrap_err().downcast_ref::<Exit>().unwrap().code, 2);
        assert_eq!(pick_project(&projects, "no-such-slug").unwrap_err().downcast_ref::<Exit>().unwrap().code, 2);
        assert_eq!(pick_project(&projects, "web shop").unwrap(), "shop");
        let ambiguous = pick_project(&projects, "Twin").unwrap_err();
        assert_eq!(ambiguous.downcast_ref::<Exit>().unwrap().code, 2);
        assert!(ambiguous.to_string().contains("more than one"));
        assert_eq!(pick_project(&projects, "Nope").unwrap_err().downcast_ref::<Exit>().unwrap().code, 2);
    }

    #[test]
    fn environments_resolve_by_name_and_uuid() {
        let listed = vec![
            json!({"id": "0b5e2c1a-9f3d-4c7e-8a21-5d6f7e8a9b0c", "slug": "development", "kind": "development"}),
            json!({"id": "1c2d3e4f-5a6b-4c7d-8e9f-0a1b2c3d4e5f", "slug": "production", "kind": "production"}),
        ];
        assert_eq!(pick_environment(&listed, "production").unwrap(), "1c2d3e4f-5a6b-4c7d-8e9f-0a1b2c3d4e5f");
        assert_eq!(pick_environment(&listed, "staging").unwrap_err().downcast_ref::<Exit>().unwrap().code, 2);
        assert!(uuid_shaped("0b5e2c1a-9f3d-4c7e-8a21-5d6f7e8a9b0c"));
        assert!(!uuid_shaped("0B5E2C1A-9F3D-4C7E-8A21-5D6F7E8A9B0C"));
        assert_eq!(provider_selector("oidc-okta").unwrap(), ("oidc", Some("oidc-okta".to_string())));
        assert!(provider_selector("oidc-").is_err());
        assert!(provider_selector("twitter").is_err());
    }

    #[test]
    fn jwks_check_compares_key_revisions() {
        let epoch = json!({"epoch": 3, "keys_rev": 5, "emergency": false});
        let current = json!({"keys": [{"kid": "k1"}, {"kid": "k2"}], "axiom_keys_rev": 5});
        let stale = json!({"keys": [{"kid": "k1"}], "axiom_keys_rev": 4});
        assert_eq!(jwks_report(&current, &epoch)["consistent"], true);
        assert_eq!(jwks_report(&current, &epoch)["kids"], json!(["k1", "k2"]));
        assert_eq!(jwks_report(&stale, &epoch)["consistent"], false);
        assert_eq!(jwks_report(&json!({"keys": []}), &epoch)["consistent"], false);
    }

    const ISSUER: &str = "https://auth.axiomcore.dev/0b5e2c1a-9f3d-4c7e-8a21-5d6f7e8a9b0c/1c2d3e4f-5a6b-4c7d-8e9f-0a1b2c3d4e5f";

    #[test]
    fn config_pull_writes_a_valid_v3_environment_and_check_compares_it() {
        let overview = json!({"issuer": ISSUER});
        let settings = json!({
            "audiences": [format!("{ISSUER}/api")],
            "allowed_origins": ["https://shop.example.com"],
            "redirect_urls": ["http://127.0.0.1:5173/auth/callback", "https://shop.example.com/auth/callback", "https://shop.example.com/other"],
            "logout_urls": ["https://shop.example.com/signed-out"],
        });
        let block = environment_block(&overview, &settings, "/auth/callback").unwrap();
        assert_eq!(block["redirectOrigin"], "https://shop.example.com");
        assert!(block.get("audience").is_none());
        let source = "# frontend\ntype = \"frontend\"\ntargets = [\"web\"]\n";
        let written = write_environment(source, "production", &block).unwrap();
        assert!(written.starts_with("# frontend\n"));
        let manifest = parse_axiom_deps_v2(&written).unwrap();
        let auth = manifest.auth.unwrap();
        assert_eq!(auth.environments["production"].issuer, ISSUER);
        assert!(compare(&auth, "production", &overview, &settings, None).is_empty());
        let deployment = json!({"authentication": {"kind": "axiomAuth", "issuer": ISSUER, "audience": "other"}});
        let warnings = compare(&auth, "production", &overview, &settings, Some(&deployment));
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        // Passkeys: the RP ID is written and an origin outside webauthn_origins is reported.
        let mut passkeys = settings.clone();
        passkeys["enabled_methods"] = json!(["password", "passkey"]);
        passkeys["webauthn_rp_id"] = json!("shop.example.com");
        passkeys["webauthn_origins"] = json!(["https://other.example.com"]);
        let served = json!({"issuer": ISSUER, "oauth_served": true});
        let block = environment_block(&served, &passkeys, "/auth/callback").unwrap();
        assert_eq!(block["webauthnRpId"], "shop.example.com");
        assert_eq!(block["oauthDiscovery"], format!("{ISSUER}/.well-known/openid-configuration"));
        let written = write_environment(source, "production", &block).unwrap();
        let auth = parse_axiom_deps_v2(&written).unwrap().auth.unwrap();
        assert_eq!(auth.environments["production"].webauthn_rp_id.as_deref(), Some("shop.example.com"));
        assert!(compare(&auth, "production", &served, &passkeys, None).iter().any(|w| w.contains("webauthn_origins")));
        let mut narrow = settings.clone();
        narrow["redirect_urls"] = json!([]);
        assert!(environment_block(&overview, &narrow, "/auth/callback").is_err());
        assert_eq!(compare(&auth, "production", &overview, &narrow, None).len(), 1);
    }
}
