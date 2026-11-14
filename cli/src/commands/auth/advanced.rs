//! `axiom auth` additions of auth-advanced/v1 17.3: OAuth applications, the OIDC issuer check,
//! Microsoft and Apple providers, trusted-issuer presets and their diagnostic test, MFA status,
//! user factors and consents, event webhooks, claim hooks and SMS settings.
//!
//! Secrets come only from a masked prompt, `--*-stdin` or `--*-file`. Secrets the API returns
//! once are written only to `--output-secret <file>` (created with mode 0600, never replaced)
//! or shown once on a terminal with a warning; never in `--json` output, logs or errors.
use super::{admin, checked_id, print, sanitize, target, text, usage, valid_issuer, AuthArgs, Exit, Target};
use anyhow::{Context, Result};
use axiom_cloud::auth_admin::public_issuer_get;
use clap::Subcommand;
use reqwest::Method;
use serde_json::{json, Value};
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

pub const HOOK_KINDS: [&str; 2] = ["before_user_create", "token_claims"];

#[derive(Subcommand, Debug)]
pub enum OauthAppsAction {
    List,
    Show {
        id: String,
    },
    /// Register an OAuth application (Owner). A confidential client's secret is returned once
    Create {
        #[arg(long)]
        name: String,
        #[arg(long = "type", value_parser = ["public", "confidential"])]
        client_type: String,
        /// Exact redirect URI; repeatable
        #[arg(long = "redirect-uri", required = true)]
        redirect_uris: Vec<String>,
        #[arg(long = "post-logout-redirect-uri")]
        post_logout_redirect_uris: Vec<String>,
        /// Browser origin allowed to call the token endpoint (public clients); repeatable
        #[arg(long = "web-origin")]
        web_origins: Vec<String>,
        /// Allowed scope; repeatable (default `openid`)
        #[arg(long = "scope")]
        scopes: Vec<String>,
        #[arg(long = "audience")]
        audiences: Vec<String>,
        /// Permission ceiling; repeatable
        #[arg(long = "permission")]
        permissions: Vec<String>,
        /// Role ceiling; repeatable
        #[arg(long = "role")]
        roles: Vec<String>,
        /// Skip the consent screen (your own application)
        #[arg(long)]
        first_party: bool,
        /// Allow the client to call the introspection endpoint
        #[arg(long)]
        can_introspect: bool,
        /// Confidential clients: how the client authenticates at the token endpoint
        #[arg(long, value_parser = ["client_secret_basic", "client_secret_post"])]
        auth_method: Option<String>,
        /// Write the one-time client secret to this new file (mode 0600)
        #[arg(long)]
        output_secret: Option<PathBuf>,
        /// Reuse after a network failure (UUID); a replay answers the client ID without a secret
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    Update {
        id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long = "redirect-uri")]
        redirect_uris: Vec<String>,
        #[arg(long = "post-logout-redirect-uri")]
        post_logout_redirect_uris: Vec<String>,
        #[arg(long = "web-origin")]
        web_origins: Vec<String>,
        #[arg(long = "scope")]
        scopes: Vec<String>,
        #[arg(long = "audience")]
        audiences: Vec<String>,
        #[arg(long = "permission")]
        permissions: Vec<String>,
        #[arg(long = "role")]
        roles: Vec<String>,
        #[arg(long, conflicts_with = "disable")]
        enable: bool,
        #[arg(long)]
        disable: bool,
    },
    /// Issue a new client secret; the previous one stays valid for --previous-valid
    RotateSecret {
        id: String,
        /// How long the previous secret keeps working: 0 to 7d, for example 24h, 90m, 0
        #[arg(long, default_value = "24h")]
        previous_valid: String,
        #[arg(long)]
        output_secret: Option<PathBuf>,
    },
    Disable {
        id: String,
    },
    /// Delete a disabled client; its grant sessions end at their next refresh
    Delete {
        id: String,
        /// Typed confirmation: "delete client <id>"
        #[arg(long)]
        confirm: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum OidcAction {
    /// Validate the issuer's public discovery document and JWKS (no sign-in needed)
    Check {
        /// Issuer URL (default: the environment's issuer)
        #[arg(long)]
        issuer: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum TrustedIssuersAction {
    List,
    /// Trust tokens of an external issuer, optionally from a preset (supabase, auth0, clerk)
    Add {
        #[arg(long, value_parser = ["custom", "supabase", "auth0", "clerk"], default_value = "custom")]
        preset: String,
        #[arg(long)]
        issuer: String,
        /// Accepted audience; repeatable
        #[arg(long = "audience", required = true)]
        audiences: Vec<String>,
        #[arg(long)]
        jwks_uri: Option<String>,
        /// Exact origin accepted in `azp`; repeatable (Clerk needs at least one)
        #[arg(long = "authorized-party")]
        authorized_parties: Vec<String>,
        #[arg(long)]
        enable: bool,
    },
    /// Run the diagnostic test with a real token read from a file; creates nothing
    Test {
        id: String,
        #[arg(long)]
        token_file: PathBuf,
    },
}

/// Second factors an environment can offer (the dashboard's FACTOR_CHOICES).
pub const FACTOR_CHOICES: [&str; 3] = ["totp", "passkey", "sms"];

#[derive(Subcommand, Debug)]
pub enum MfaAction {
    /// Policy, enabled factors and how many users have factors (counts only)
    Status,
    /// Change the policy, factors, reauthentication window or passkey relying party (Owner).
    /// Flags you omit keep their current value; turning MFA on from off asks for confirmation
    Set {
        /// off; optional (users may add a factor); required (users without a factor add one
        /// before they get a session)
        #[arg(long, value_parser = ["off", "optional", "required"])]
        policy: Option<String>,
        /// A factor users may add: totp, passkey or sms; repeatable, replaces the list
        #[arg(long = "factor", value_parser = FACTOR_CHOICES)]
        factors: Vec<String>,
        /// How recent a sign-in must be for security actions, 2 to 60 minutes
        #[arg(long)]
        step_up_minutes: Option<u32>,
        /// Passkey relying party ID: your app's registrable domain (for example example.com) or
        /// localhost, never an axiomcore.dev name; "" clears it
        #[arg(long)]
        rp_id: Option<String>,
        /// Passkey relying party name shown by browsers, at most 64 bytes; "" clears it
        #[arg(long)]
        rp_name: Option<String>,
        /// Exact passkey origin (https://host[:port] or http://localhost[:port]) that is also an
        /// allowed origin of the settings; repeatable (at most 8), replaces the list
        #[arg(long = "origin", conflicts_with = "clear_origins")]
        origins: Vec<String>,
        /// Remove every passkey origin
        #[arg(long)]
        clear_origins: bool,
        /// Confirm turning multi-factor on without a prompt (required without a terminal)
        #[arg(long)]
        yes: bool,
    },
}

/// The flags of `mfa set`; `None` keeps the current value.
#[derive(Debug, Default)]
pub struct MfaChange<'a> {
    pub policy: Option<&'a str>,
    pub factors: Option<&'a [String]>,
    pub step_up_minutes: Option<u32>,
    pub rp_id: Option<&'a str>,
    pub rp_name: Option<&'a str>,
    pub origins: Option<&'a [String]>,
}

#[derive(Subcommand, Debug)]
pub enum WebhooksAction {
    List,
    /// Add an endpoint (Owner); its signing secret is returned once
    Create {
        #[arg(long)]
        url: String,
        /// Event type; repeatable, for example user.created
        #[arg(long = "event", required = true)]
        events: Vec<String>,
        #[arg(long)]
        enable: bool,
        #[arg(long)]
        output_secret: Option<PathBuf>,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    Update {
        id: String,
        #[arg(long)]
        url: Option<String>,
        #[arg(long = "event")]
        events: Vec<String>,
        #[arg(long, conflicts_with = "disable")]
        enable: bool,
        #[arg(long)]
        disable: bool,
    },
    RotateSecret {
        id: String,
        #[arg(long)]
        output_secret: Option<PathBuf>,
    },
    /// Send a ping event (10 per hour)
    Test {
        id: String,
    },
    Deliveries {
        id: String,
        #[arg(long, value_parser = ["pending", "delivered", "failed", "dead"])]
        state: Option<String>,
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: Option<u32>,
        #[arg(long)]
        cursor: Option<String>,
    },
    Replay {
        id: String,
        delivery_id: String,
    },
    Delete {
        id: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum HooksAction {
    Get {
        #[arg(value_parser = HOOK_KINDS)]
        kind: Option<String>,
    },
    /// Configure a claim hook (Owner); the signing secret is generated at the first save and returned once
    Set {
        #[arg(value_parser = HOOK_KINDS)]
        kind: String,
        #[arg(long)]
        url: String,
        /// What happens when the hook fails or times out
        #[arg(long, value_parser = ["deny", "continue"])]
        policy: String,
        #[arg(long, value_parser = clap::value_parser!(u32).range(100..=10000))]
        timeout_ms: Option<u32>,
        /// Role the hook may grant; repeatable
        #[arg(long = "grantable-role")]
        grantable_roles: Vec<String>,
        #[arg(long)]
        include_email: bool,
        #[arg(long, conflicts_with = "disable")]
        enable: bool,
        #[arg(long)]
        disable: bool,
        #[arg(long)]
        output_secret: Option<PathBuf>,
    },
    /// Call the hook with a synthetic user (never real data)
    Test {
        #[arg(value_parser = HOOK_KINDS)]
        kind: String,
        #[arg(long, value_parser = ["new_user", "sign_in"], default_value = "new_user")]
        sample: String,
    },
    RotateSecret {
        #[arg(value_parser = HOOK_KINDS)]
        kind: String,
        #[arg(long)]
        output_secret: Option<PathBuf>,
    },
    Remove {
        #[arg(value_parser = HOOK_KINDS)]
        kind: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum SmsAction {
    Get,
    /// Configure the Twilio account (Owner, Pro). The Auth Token or API key secret comes from stdin
    Set {
        #[arg(long)]
        account_sid: String,
        /// Read the Twilio Auth Token from stdin
        #[arg(long, conflicts_with_all = ["api_key_sid", "api_key_secret_stdin"])]
        auth_token_stdin: bool,
        /// Twilio API key SID (with --api-key-secret-stdin)
        #[arg(long, requires = "api_key_secret_stdin")]
        api_key_sid: Option<String>,
        /// Read the Twilio API key secret from stdin
        #[arg(long, requires = "api_key_sid")]
        api_key_secret_stdin: bool,
        #[arg(long, conflicts_with = "from_number")]
        messaging_service: Option<String>,
        #[arg(long)]
        from_number: Option<String>,
        /// Allowed destination country (ISO 3166 alpha-2); repeatable
        #[arg(long = "country", required = true)]
        countries: Vec<String>,
        #[arg(long)]
        monthly_cap: u64,
        #[arg(long)]
        daily_cap: Option<u64>,
        #[arg(long)]
        hourly_cap: Option<u64>,
        #[arg(long, conflicts_with = "disable")]
        enable: bool,
        #[arg(long)]
        disable: bool,
    },
    /// Send a test message to an allowlisted number (5 per day)
    Test {
        #[arg(long)]
        to: String,
    },
    Remove {
        #[arg(long)]
        yes: bool,
    },
}

// ---------------------------------------------------------------------------
// Secret input and one-time output
// ---------------------------------------------------------------------------

/// A secret from stdin (bounded) or a masked prompt; never argv.
pub fn read_secret_input(stdin: bool, prompt: &str, flag: &str, limit: usize) -> Result<Zeroizing<String>> {
    let secret = if stdin {
        let mut bytes = Zeroizing::new(Vec::new());
        std::io::stdin().take(limit as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > limit {
            return Err(usage(format!("{prompt} is at most {limit} bytes")));
        }
        Zeroizing::new(String::from_utf8(bytes.to_vec()).map_err(|_| usage(format!("{prompt} is not UTF-8")))?)
    } else {
        if !std::io::stdin().is_terminal() {
            return Err(usage(format!("no terminal for the masked prompt; pass {flag}")));
        }
        Zeroizing::new(dialoguer::Password::new().with_prompt(prompt).interact()?)
    };
    let trimmed = Zeroizing::new(secret.trim_end_matches(['\r', '\n']).to_string());
    if trimmed.is_empty() {
        return Err(usage(format!("{prompt} is empty")));
    }
    Ok(trimmed)
}

/// A file given with `--*-file`, bounded; its content is secret and never printed.
pub fn read_secret_file(path: &Path, what: &str, limit: u64) -> Result<Zeroizing<String>> {
    let meta = std::fs::metadata(path).with_context(|| format!("read {}", path.display()))?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > limit {
        return Err(usage(format!("{what} must be a file of 1 to {limit} bytes")));
    }
    let bytes = Zeroizing::new(std::fs::read(path).with_context(|| format!("read {}", path.display()))?);
    let text = std::str::from_utf8(&bytes).map_err(|_| usage(format!("{what} is not UTF-8")))?;
    Ok(Zeroizing::new(text.trim_end_matches(['\r', '\n']).to_string()))
}

/// Where a one-time secret goes, decided before the request so it is never lost.
pub enum SecretSink {
    File(PathBuf),
    Terminal,
}

pub fn secret_sink(output: Option<&Path>, json: bool) -> Result<SecretSink> {
    match output {
        Some(path) => {
            if path.exists() {
                return Err(usage(format!("{} exists; --output-secret never replaces a file", path.display())));
            }
            Ok(SecretSink::File(path.to_path_buf()))
        }
        None if !json && std::io::stdout().is_terminal() => Ok(SecretSink::Terminal),
        None => Err(usage("the secret is returned once: pass --output-secret <file> (it is never part of --json output)")),
    }
}

/// Writes `secret` once: a new 0600 file, or the terminal with a warning.
pub fn deliver_secret(sink: &SecretSink, secret: &str, what: &str) -> Result<()> {
    match sink {
        SecretSink::File(path) => {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(path).with_context(|| format!("create {}", path.display()))?;
            file.write_all(secret.as_bytes())?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            eprintln!("The {what} was written to {} (mode 0600). It is shown only once.", path.display());
        }
        SecretSink::Terminal => {
            eprintln!("Warning: the {what} is shown only once. Store it in a secret manager now; it is not saved anywhere.");
            println!("{secret}");
        }
    }
    Ok(())
}

/// Removes and returns the one-time secret at `key` of a response.
fn take_secret(value: &mut Value, keys: &[&str]) -> Option<Zeroizing<String>> {
    for key in keys {
        if let Some(Value::String(secret)) = value.as_object_mut().and_then(|m| m.remove(*key)) {
            return Some(Zeroizing::new(secret));
        }
    }
    None
}

/// `24h`, `90m`, `30s`, `7d` or plain seconds, 0 to 604,800.
pub fn duration_seconds(value: &str) -> Result<u64> {
    let (number, unit) = match value.char_indices().find(|(_, c)| !c.is_ascii_digit()) {
        Some((i, _)) => value.split_at(i),
        None => (value, "s"),
    };
    let multiplier = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        _ => return Err(usage("durations look like 24h, 90m, 30s or 7d")),
    };
    let seconds = number.parse::<u64>().map_err(|_| usage("durations look like 24h, 90m, 30s or 7d"))?.saturating_mul(multiplier);
    if seconds > 604_800 {
        return Err(usage("the previous secret stays valid at most 7 days"));
    }
    Ok(seconds)
}

/// Ask before a destructive change: `--yes` skips the question; without a terminal and
/// without `--yes` it refuses before anything is sent. Under `axiom auth` the refusal exits 2;
/// `axiom secrets` and `axiom project resources revoke` share it and exit 1.
pub(crate) fn confirm_yes(yes: bool, prompt: &str) -> Result<()> {
    if yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        return Err(usage("pass --yes to confirm without a prompt"));
    }
    if !dialoguer::Confirm::new().with_prompt(prompt).default(false).interact()? {
        return Err(usage("cancelled"));
    }
    Ok(())
}

fn idempotency(key: Option<&str>) -> Result<String> {
    let key = key.map(str::to_string).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    uuid::Uuid::parse_str(&key).map_err(|_| usage("--idempotency-key must be a UUID"))?;
    Ok(key)
}

impl Target {
    async fn keyed(&self, method: Method, rest: &str, body: &Value, key: &str) -> Result<Value> {
        self.client.auth_admin_request_keyed(method, &self.slug, &self.environment, rest, Some(body), key).await.map_err(admin)
    }
}

// ---------------------------------------------------------------------------
// OAuth applications
// ---------------------------------------------------------------------------

fn oauth_line(c: &Value) -> String {
    format!(
        "{}  {}  {}  {}  {}{}",
        text(c, "id"),
        text(c, "client_id"),
        text(c, "name"),
        text(c, "type"),
        if c["enabled"] == true { "enabled" } else { "disabled" },
        if c["first_party"] == true { "  first-party" } else { "" }
    )
}

#[allow(clippy::too_many_arguments)]
pub fn oauth_create_body(
    name: &str,
    client_type: &str,
    redirect_uris: &[String],
    post_logout: &[String],
    web_origins: &[String],
    scopes: &[String],
    audiences: &[String],
    permissions: &[String],
    roles: &[String],
    first_party: bool,
    can_introspect: bool,
    auth_method: Option<&str>,
) -> Result<Value> {
    if name.trim().is_empty() || name.len() > 128 {
        return Err(usage("--name is 1 to 128 characters"));
    }
    let method = match (client_type, auth_method) {
        ("public", None) => "none",
        ("public", Some(_)) => return Err(usage("public clients have no secret; omit --auth-method")),
        (_, Some(m)) => m,
        (_, None) => "client_secret_basic",
    };
    let scopes = if scopes.is_empty() { vec!["openid".to_string()] } else { scopes.to_vec() };
    let mut body = json!({
        "name": name,
        "type": client_type,
        "token_endpoint_auth_method": method,
        "redirect_uris": redirect_uris,
        "allowed_scopes": scopes,
        "first_party": first_party,
        "can_introspect": can_introspect,
    });
    for (key, list) in [("post_logout_redirect_uris", post_logout), ("web_origins", web_origins), ("audiences", audiences), ("permission_ceiling", permissions), ("role_ceiling", roles)] {
        if !list.is_empty() {
            body[key] = json!(list);
        }
    }
    Ok(body)
}

pub async fn oauth_apps(args: &AuthArgs, action: &OauthAppsAction) -> Result<()> {
    // Local checks first: no secret may be lost, no request sent for a mistake.
    let sink = match action {
        OauthAppsAction::Create { client_type, output_secret, .. } if client_type == "confidential" => Some(secret_sink(output_secret.as_deref(), args.json)?),
        OauthAppsAction::RotateSecret { output_secret, previous_valid, .. } => {
            duration_seconds(previous_valid)?;
            Some(secret_sink(output_secret.as_deref(), args.json)?)
        }
        _ => None,
    };
    let target = target(args).await?;
    oauth_apps_with(args, &target, action, sink.as_ref()).await
}

pub(super) async fn oauth_apps_with(args: &AuthArgs, target: &Target, action: &OauthAppsAction, sink: Option<&SecretSink>) -> Result<()> {
    match action {
        OauthAppsAction::List => {
            let mut listed = target.call(Method::GET, "oauth-clients", &[], None).await?;
            sanitize(&mut listed, true);
            print(args, &listed, |l| {
                let lines = l["clients"].as_array().into_iter().flatten().map(oauth_line).collect::<Vec<_>>();
                if lines.is_empty() {
                    vec!["No OAuth applications.".into()]
                } else {
                    lines
                }
            })
        }
        OauthAppsAction::Show { id } => {
            let mut client = target.call(Method::GET, &format!("oauth-clients/{}", checked_id(id)?), &[], None).await?;
            sanitize(&mut client, true);
            print(args, &client, |c| {
                let c = c.get("client").unwrap_or(c);
                vec![oauth_line(c), format!("redirect URIs: {}", c["redirect_uris"]), format!("scopes: {}", c["allowed_scopes"]), format!("secret: {}", if c["secret_set"] == true { "set" } else { "none" })]
            })
        }
        OauthAppsAction::Create { name, client_type, redirect_uris, post_logout_redirect_uris, web_origins, scopes, audiences, permissions, roles, first_party, can_introspect, auth_method, idempotency_key, .. } => {
            let body = oauth_create_body(name, client_type, redirect_uris, post_logout_redirect_uris, web_origins, scopes, audiences, permissions, roles, *first_party, *can_introspect, auth_method.as_deref())?;
            let key = idempotency(idempotency_key.as_deref())?;
            eprintln!("Idempotency key: {key}");
            let mut created = target.keyed(Method::POST, "oauth-clients", &body, &key).await?;
            let secret = take_secret(&mut created, &["client_secret"]);
            if let (Some(secret), Some(sink)) = (&secret, sink) {
                deliver_secret(sink, secret, "client secret")?;
            }
            sanitize(&mut created, true);
            print(args, &created, |c| vec![format!("Created {}", oauth_line(c.get("client").unwrap_or(c)))])
        }
        OauthAppsAction::Update { id, name, redirect_uris, post_logout_redirect_uris, web_origins, scopes, audiences, permissions, roles, enable, disable } => {
            let id = checked_id(id)?;
            let current = target.call(Method::GET, &format!("oauth-clients/{id}"), &[], None).await?;
            let current = current.get("client").unwrap_or(&current);
            let mut body = json!({"revision": current["revision"]});
            if let Some(name) = name {
                body["name"] = json!(name);
            }
            for (key, list) in [("redirect_uris", redirect_uris), ("post_logout_redirect_uris", post_logout_redirect_uris), ("web_origins", web_origins), ("allowed_scopes", scopes), ("audiences", audiences), ("permission_ceiling", permissions), ("role_ceiling", roles)] {
                if !list.is_empty() {
                    body[key] = json!(list);
                }
            }
            if *enable || *disable {
                body["enabled"] = json!(*enable);
            }
            if body.as_object().is_some_and(|m| m.len() == 1) {
                return Err(usage("nothing to change"));
            }
            let mut saved = target.call(Method::PATCH, &format!("oauth-clients/{id}"), &[], Some(&body)).await?;
            sanitize(&mut saved, true);
            print(args, &saved, |c| vec![format!("Saved {}", oauth_line(c.get("client").unwrap_or(c)))])
        }
        OauthAppsAction::RotateSecret { id, previous_valid, .. } => {
            let seconds = duration_seconds(previous_valid)?;
            let mut rotated = target.call(Method::POST, &format!("oauth-clients/{}/secret", checked_id(id)?), &[], Some(&json!({"previous_valid_seconds": seconds}))).await?;
            let secret = take_secret(&mut rotated, &["client_secret"]);
            match (&secret, sink) {
                (Some(secret), Some(sink)) => deliver_secret(sink, secret, "client secret")?,
                _ => return Err(Exit { code: 5, message: "AXAUTH_UNAVAILABLE: the response carried no secret".into() }.into()),
            }
            sanitize(&mut rotated, true);
            print(args, &rotated, |r| vec![format!("Secret rotated; the previous secret works until {}.", text(r, "previous_expires_at"))])
        }
        OauthAppsAction::Disable { id } => {
            let id = checked_id(id)?;
            let current = target.call(Method::GET, &format!("oauth-clients/{id}"), &[], None).await?;
            let revision = current.get("client").unwrap_or(&current)["revision"].clone();
            let mut saved = target.call(Method::PATCH, &format!("oauth-clients/{id}"), &[], Some(&json!({"enabled": false, "revision": revision}))).await?;
            sanitize(&mut saved, true);
            print(args, &saved, |_| vec![format!("Disabled {id}; new authorizations are refused.")])
        }
        OauthAppsAction::Delete { id, confirm } => {
            let id = checked_id(id)?;
            let phrase = format!("delete client {id}");
            if confirm.as_deref() != Some(phrase.as_str()) {
                return Err(usage(format!("deleting a client ends its grant sessions at their next refresh; repeat with --confirm \"{phrase}\" (disable it first)")));
            }
            target.call(Method::DELETE, &format!("oauth-clients/{id}"), &[], None).await?;
            print(args, &json!({"deleted": id}), |_| vec![format!("Deleted {id}.")])
        }
    }
}

// ---------------------------------------------------------------------------
// OIDC issuer check
// ---------------------------------------------------------------------------

fn b64url_len(value: &str) -> Option<usize> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(value.trim_end_matches('=')).ok().map(|b| b.len())
}

/// Checks a discovery document, its JWKS and the epoch (auth-advanced/v1 11.4, 11.9). Each
/// entry is `{check, ok}`; failures name the check, never server text.
pub fn oidc_report(issuer: &str, discovery: &Value, jwks: &Value, epoch: Option<&Value>) -> Value {
    let mut checks = vec![];
    let mut check = |name: &str, ok: bool| checks.push(json!({"check": name, "ok": ok}));
    check("issuer_matches", discovery["issuer"].as_str() == Some(issuer));
    let under = |key: &str| discovery[key].as_str().is_some_and(|u| u.starts_with(&format!("{issuer}/")));
    for key in ["authorization_endpoint", "token_endpoint", "jwks_uri", "userinfo_endpoint"] {
        check(&format!("{key}_under_issuer"), under(key));
    }
    let has = |key: &str, value: &str| discovery[key].as_array().is_some_and(|a| a.iter().any(|v| v.as_str() == Some(value)));
    check("response_type_code", has("response_types_supported", "code"));
    check("pkce_s256", has("code_challenge_methods_supported", "S256"));
    check("id_token_rs256", has("id_token_signing_alg_values_supported", "RS256"));
    check("no_implicit_grant", !has("response_types_supported", "token") && !has("response_types_supported", "id_token"));
    let keys = jwks["keys"].as_array().cloned().unwrap_or_default();
    let mut kids = std::collections::BTreeSet::new();
    let unique = keys.iter().all(|k| k["kid"].as_str().is_some_and(|kid| kids.insert(kid.to_string())));
    check("kids_unique", !keys.is_empty() && unique);
    let private = ["d", "p", "q", "dp", "dq", "qi", "k"];
    check("public_keys_only", keys.iter().all(|k| private.iter().all(|p| k.get(*p).is_none())));
    let typed = keys.iter().all(|k| match (k["kty"].as_str(), k["alg"].as_str()) {
        (Some("OKP"), Some("EdDSA")) => k["crv"].as_str() == Some("Ed25519") && k["x"].as_str().and_then(b64url_len) == Some(32),
        (Some("RSA"), Some("RS256")) => k["n"].as_str().and_then(b64url_len).is_some_and(|n| n >= 256) && k["e"].as_str().is_some(),
        _ => false,
    });
    check("key_types", typed);
    check("rs256_key_present", keys.iter().any(|k| k["alg"].as_str() == Some("RS256")));
    let jwks_rev = jwks["axiom_keys_rev"].as_i64();
    let epoch_rev = epoch.and_then(|e| e["keys_rev"].as_i64());
    check("keys_rev", matches!((jwks_rev, epoch_rev), (Some(j), Some(e)) if j >= e));
    let ok = checks.iter().all(|c| c["ok"] == true);
    json!({"issuer": issuer, "ok": ok, "checks": checks, "kids": kids.into_iter().collect::<Vec<_>>()})
}

pub async fn oidc(args: &AuthArgs, action: &OidcAction) -> Result<()> {
    let OidcAction::Check { issuer } = action;
    let issuer = match issuer {
        Some(issuer) => issuer.trim_end_matches('/').to_string(),
        None => {
            let target = target(args).await?;
            let overview = target.call(Method::GET, "overview", &[], None).await?;
            overview["issuer"].as_str().map(str::to_string).ok_or_else(|| Exit { code: 5, message: "AXAUTH_UNAVAILABLE: the environment reports no issuer".into() })?
        }
    };
    if !valid_issuer(&issuer) {
        return Err(usage("issuers are https://auth.axiomcore.dev/<project_id>/<environment_id>"));
    }
    let discovery = public_issuer_get(&issuer, "/.well-known/openid-configuration").await.map_err(|e| match e.status() {
        Some(404) => Exit { code: 5, message: "AXAUTH_NOT_FOUND: the issuer serves no OpenID discovery; OAuth applications are not served for this environment".into() }.into(),
        _ => admin(e),
    })?;
    let jwks = public_issuer_get(&issuer, "/.well-known/jwks.json").await.map_err(admin)?;
    let epoch = public_issuer_get(&issuer, "/epoch").await.ok();
    let report = oidc_report(&issuer, &discovery, &jwks, epoch.as_ref());
    print(args, &report, |r| {
        let mut lines = vec![format!("Issuer: {}", text(r, "issuer"))];
        for c in r["checks"].as_array().into_iter().flatten() {
            lines.push(format!("  {} {}", if c["ok"] == true { "ok  " } else { "FAIL" }, text(c, "check")));
        }
        lines
    })?;
    if report["ok"] != true {
        return Err(Exit { code: 5, message: "AXAUTH_OIDC_CHECK_FAILED: see the failed checks above".into() }.into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Microsoft and Apple provider fields
// ---------------------------------------------------------------------------

fn guid(value: &str) -> bool {
    value.len() == 36 && value.bytes().enumerate().all(|(i, b)| if [8, 13, 18, 23].contains(&i) { b == b'-' } else { b.is_ascii_hexdigit() })
}

/// The kind-specific fields of `providers set microsoft|apple`.
pub fn provider_extras(kind: &str, tenant: Option<&str>, allowed_tenants: &[String], team_id: Option<&str>, key_id: Option<&str>, has_private_key: bool, creating: bool) -> Result<Value> {
    let mut extra = json!({});
    let apple_flags = team_id.is_some() || key_id.is_some() || has_private_key;
    let microsoft_flags = tenant.is_some() || !allowed_tenants.is_empty();
    match kind {
        "microsoft" => {
            if apple_flags {
                return Err(usage("--team-id, --key-id and --private-key-file apply to apple only"));
            }
            match tenant {
                Some(t) if ["common", "organizations", "consumers"].contains(&t) || guid(t) => extra["microsoft_tenant"] = json!(t),
                Some(_) => return Err(usage("--tenant is common, organizations, consumers or a tenant GUID")),
                None if creating => return Err(usage("--tenant is required for a new Microsoft provider")),
                None => {}
            }
            if allowed_tenants.iter().any(|t| !guid(t)) {
                return Err(usage("--allowed-tenant takes tenant GUIDs"));
            }
            if !allowed_tenants.is_empty() {
                extra["allowed_tenants"] = json!(allowed_tenants);
            }
        }
        "apple" => {
            if microsoft_flags {
                return Err(usage("--tenant and --allowed-tenant apply to microsoft only"));
            }
            let ten = |v: &str| v.len() == 10 && v.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
            for (flag, value) in [("--team-id", team_id), ("--key-id", key_id)] {
                match value {
                    Some(v) if ten(v) => extra[if flag == "--team-id" { "apple_team_id" } else { "apple_key_id" }] = json!(v),
                    Some(_) => return Err(usage(format!("{flag} is 10 uppercase letters or digits"))),
                    None if creating => return Err(usage(format!("{flag} is required for a new Apple provider"))),
                    None => {}
                }
            }
            if creating && !has_private_key {
                return Err(usage("--private-key-file <AuthKey.p8> is required for a new Apple provider"));
            }
        }
        _ if apple_flags || microsoft_flags => return Err(usage("--tenant, --allowed-tenant, --team-id, --key-id and --private-key-file apply to microsoft or apple")),
        _ => {}
    }
    Ok(extra)
}

/// The Apple `.p8` private key (PEM, at most 8 KiB).
pub fn apple_private_key(path: &Path) -> Result<Zeroizing<String>> {
    let pem = read_secret_file(path, "--private-key-file", 8192)?;
    if !pem.contains("-----BEGIN PRIVATE KEY-----") || !pem.contains("-----END PRIVATE KEY-----") {
        return Err(usage("--private-key-file must be the PEM AuthKey .p8 file Apple issued"));
    }
    Ok(pem)
}

// ---------------------------------------------------------------------------
// Trusted issuers, MFA, user factors and consents
// ---------------------------------------------------------------------------

pub async fn trusted_issuers(args: &AuthArgs, action: &TrustedIssuersAction) -> Result<()> {
    let token = match action {
        TrustedIssuersAction::Test { token_file, .. } => Some(read_secret_file(token_file, "--token-file", 16 * 1024)?),
        _ => None,
    };
    let target = target(args).await?;
    match action {
        TrustedIssuersAction::List => {
            let mut listed = target.call(Method::GET, "trusted-issuers", &[], None).await?;
            sanitize(&mut listed, true);
            print(args, &listed, |l| {
                let mut lines = l["trusted_issuers"].as_array().or(l["issuers"].as_array()).into_iter().flatten().map(|i| format!("{}  {}  preset {}  {}", text(i, "id"), text(i, "issuer"), text(i, "preset"), if i["enabled"] == true { "enabled" } else { "disabled" })).collect::<Vec<_>>();
                if lines.is_empty() {
                    lines.push("No trusted issuers.".into());
                }
                lines
            })
        }
        TrustedIssuersAction::Add { preset, issuer, audiences, jwks_uri, authorized_parties, enable } => {
            if !issuer.starts_with("https://") {
                return Err(usage("--issuer must be an https URL"));
            }
            if preset == "clerk" && authorized_parties.is_empty() {
                return Err(usage("the clerk preset needs at least one --authorized-party (your frontend origin)"));
            }
            let mut body = json!({"preset": preset, "issuer": issuer, "audiences": audiences, "enabled": enable});
            if let Some(uri) = jwks_uri {
                body["jwks_uri"] = json!(uri);
            }
            if !authorized_parties.is_empty() {
                body["authorized_parties"] = json!(authorized_parties);
            }
            let mut saved = target.call(Method::POST, "trusted-issuers", &[], Some(&body)).await?;
            sanitize(&mut saved, true);
            print(args, &saved, |i| vec![format!("Trusted issuer {} saved (preset {}). Run `axiom auth trusted-issuers test {} --token-file <file>` with a real token before enabling.", text(i, "id"), text(i, "preset"), text(i, "id"))])
        }
        TrustedIssuersAction::Test { id, .. } => {
            let token = token.expect("read above");
            let mut body = json!({"token": token.as_str()});
            let result = target.call(Method::POST, &format!("trusted-issuers/{}/test", checked_id(id)?), &[], Some(&body)).await;
            if let Some(map) = body.as_object_mut() {
                map.remove("token");
            }
            let mut result = result?;
            sanitize(&mut result, true);
            print(args, &result, |r| {
                let mut lines = vec![format!("Result: {}", text(r, "result"))];
                for s in r["steps"].as_array().into_iter().flatten() {
                    let claims = s["claims"].as_array().map(|c| c.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default();
                    lines.push(format!("  {:<16} {}{}", text(s, "step"), text(s, "decision"), if claims.is_empty() { String::new() } else { format!(" (claims: {claims})") }));
                }
                lines
            })
        }
    }
}

pub async fn mfa(args: &AuthArgs, action: &MfaAction) -> Result<()> {
    match action {
        MfaAction::Status => {
            let target = target(args).await?;
            print_mfa_status(args, &target).await
        }
        MfaAction::Set { policy, factors, step_up_minutes, rp_id, rp_name, origins, clear_origins, yes } => {
            let change = MfaChange {
                policy: policy.as_deref(),
                factors: (!factors.is_empty()).then_some(factors.as_slice()),
                step_up_minutes: *step_up_minutes,
                rp_id: rp_id.as_deref(),
                rp_name: rp_name.as_deref(),
                origins: if *clear_origins { Some(&[][..]) } else { (!origins.is_empty()).then_some(origins.as_slice()) },
            };
            mfa_set(args, &change, *yes).await
        }
    }
}

async fn print_mfa_status(args: &AuthArgs, target: &Target) -> Result<()> {
    let mut status = target.call(Method::GET, "mfa", &[], None).await?;
    sanitize(&mut status, false);
    print(args, &status, |s| {
        vec![
            format!("Policy: {}", text(s, "policy")),
            format!("Factors: {}", s["factors"]),
            format!("Users with factors: {}{}", text(s, "users_with_factors"), if s["users_count_capped"] == true { "+" } else { "" }),
        ]
    })
}

/// The current factor types: settings, else `GET mfa`, else the dialog's default.
fn current_factors(settings: &Value, mfa: Option<&Value>) -> Vec<String> {
    let types = |list: &Value| list.as_array().map(|a| a.iter().filter_map(|f| f.as_str().or(f["type"].as_str())).map(str::to_string).collect::<Vec<_>>());
    types(&settings["mfa_factors"])
        .or_else(|| mfa.and_then(|m| types(&m["factors"])))
        .unwrap_or_else(|| vec!["totp".into(), "passkey".into()])
        .into_iter()
        .filter(|f| FACTOR_CHOICES.contains(&f.as_str()))
        .collect()
}

/// The stored policy: settings, else `GET mfa`, else off.
pub fn current_policy(settings: &Value, mfa: Option<&Value>) -> String {
    settings["mfa_policy"].as_str().or_else(|| mfa.and_then(|m| m["policy"].as_str())).unwrap_or("off").to_string()
}

/// A relying party ID the dashboard accepts: a host name (or localhost), never axiomcore.dev.
fn rp_id_valid(id: &str) -> bool {
    static HOST: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let host = HOST.get_or_init(|| regex::Regex::new(r"^([a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}$").expect("host pattern"));
    let shaped = id == "localhost" || (id.len() <= 253 && host.is_match(id));
    shaped && id != "axiomcore.dev" && !id.ends_with(".axiomcore.dev")
}

fn exact_origin(origin: &str) -> bool {
    static ORIGIN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    ORIGIN.get_or_init(|| regex::Regex::new(r"^(https://[a-z0-9.-]+(:[0-9]{1,5})?|http://localhost(:[0-9]{1,5})?)$").expect("origin pattern")).is_match(origin)
}

/// The origin's host is the RP ID or a subdomain of it (the server's WebAuthn rule).
fn origin_under(origin: &str, rp_id: &str) -> bool {
    reqwest::Url::parse(origin).ok().and_then(|u| u.host_str().map(str::to_string)).is_some_and(|h| h == rp_id || h.ends_with(&format!(".{rp_id}")))
}

/// The dashboard's `mfaInput` with omitted flags kept: the six multi-factor settings, or the
/// problems found (each a sentence).
pub fn mfa_input(settings: &Value, mfa: Option<&Value>, change: &MfaChange) -> std::result::Result<Value, Vec<String>> {
    let mut problems = vec![];
    let policy = change.policy.map(str::to_string).unwrap_or_else(|| current_policy(settings, mfa));
    if !["off", "optional", "required"].contains(&policy.as_str()) {
        problems.push("Choose a policy: off, optional or required.".to_string());
    }
    let mut factors: Vec<String> = vec![];
    for f in change.factors.map(<[String]>::to_vec).unwrap_or_else(|| current_factors(settings, mfa)) {
        if !factors.contains(&f) {
            factors.push(f);
        }
    }
    if factors.is_empty() {
        problems.push("Allow at least one factor (--factor totp, passkey or sms).".into());
    }
    let step_up_seconds = match change.step_up_minutes {
        Some(minutes) if !(2..=60).contains(&minutes) => {
            problems.push("The reauthentication window is 2 to 60 minutes.".into());
            0
        }
        Some(minutes) => i64::from(minutes) * 60,
        None => settings["step_up_seconds"].as_i64().unwrap_or(600),
    };
    let rp_id = change.rp_id.map(str::to_string).or_else(|| settings["webauthn_rp_id"].as_str().map(str::to_string)).map(|v| v.trim().to_ascii_lowercase()).filter(|v| !v.is_empty());
    if rp_id.as_deref().is_some_and(|id| !rp_id_valid(id)) {
        problems.push("The relying party ID is your app's registrable domain (for example example.com) or localhost, never an axiomcore.dev name.".into());
    }
    let rp_name = change.rp_name.map(str::to_string).or_else(|| settings["webauthn_rp_name"].as_str().map(str::to_string)).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    if rp_name.as_ref().is_some_and(|n| n.len() > 64) {
        problems.push("The relying party name is at most 64 bytes.".into());
    }
    let origins: Vec<String> = match change.origins {
        Some(list) => list.iter().map(|o| o.trim().to_string()).filter(|o| !o.is_empty()).collect(),
        None => settings["webauthn_origins"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect(),
    };
    if origins.len() > 8 {
        problems.push("List at most 8 passkey origins.".into());
    }
    let allowed = settings["allowed_origins"].as_array().cloned().unwrap_or_default();
    for origin in &origins {
        if !exact_origin(origin) {
            problems.push(format!("{origin} is not an exact origin (https://host[:port] or http://localhost[:port])."));
        } else if !allowed.iter().any(|a| a.as_str() == Some(origin.as_str())) {
            problems.push(format!("{origin} is not an allowed origin of the settings (allowed_origins). Add it there first."));
        } else if let Some(id) = rp_id.as_deref().filter(|id| rp_id_valid(id)) {
            if !origin_under(origin, id) {
                problems.push(format!("{origin} is not {id} or a subdomain of it (the relying party ID)."));
            }
        }
    }
    if !origins.is_empty() && rp_id.is_none() {
        problems.push("Passkey origins need a relying party ID (--rp-id).".into());
    }
    if factors.iter().any(|f| f == "passkey") && rp_id.is_none() && policy != "off" {
        problems.push("Passkeys as a second factor need a relying party ID (--rp-id).".into());
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    Ok(json!({
        "mfa_policy": policy,
        "mfa_factors": factors,
        "step_up_seconds": step_up_seconds,
        "webauthn_rp_id": rp_id,
        "webauthn_rp_name": rp_name,
        "webauthn_origins": origins,
    }))
}

/// The dashboard's `enrollableFrom`: TOTP (the platform switch is assumed on), a passkey with
/// a relying party ID, SMS with enabled Twilio settings.
pub fn enrollable(factors: &[String], rp_id: Option<&str>, sms_ready: bool) -> Vec<String> {
    factors.iter().filter(|f| match f.as_str() {
        "totp" => true,
        "passkey" => rp_id.is_some_and(|id| !id.is_empty()),
        "sms" => sms_ready,
        _ => false,
    }).cloned().collect()
}

const MFA_SDK_WARNING: &str = "Before you continue, upgrade your app's Axiom sign-in SDK (@axiomcore/auth-web 0.2.0 or later). Older SDKs cannot complete a second step, so users with a factor cannot sign in through them.";

/// The dashboard's wording of a refused multi-factor settings write.
pub fn mfa_refusal(error: axiom_cloud::auth_admin::AuthAdminError) -> anyhow::Error {
    let code = super::exit_code(&error);
    let message = if error.status() == Some(400) && ["", "AXAUTH_INVALID_REQUEST"].contains(&error.code()) {
        format!("{}: the authentication service does not accept these multi-factor settings yet, or a value is out of bounds. Nothing was changed.", error.code())
    } else if error.code() == "AXAUTH_FEATURE_DISABLED" {
        "AXAUTH_FEATURE_DISABLED: multi-factor is not available for this environment yet, or required was chosen while no factor can be added. Nothing was changed.".to_string()
    } else {
        return admin(error);
    };
    Exit { code, message }.into()
}

async fn mfa_set(args: &AuthArgs, change: &MfaChange<'_>, yes: bool) -> Result<()> {
    let target = target(args).await?;
    let settings = target.call(Method::GET, "settings", &[], None).await?;
    // Settings without the multi-factor fields: the policy and factors come from GET mfa.
    let mfa = if settings.get("mfa_policy").is_none() { target.call(Method::GET, "mfa", &[], None).await.ok() } else { None };
    let input = mfa_input(&settings, mfa.as_ref(), change).map_err(|problems| usage(problems.join("\n")))?;
    let policy = input["mfa_policy"].as_str().unwrap_or("off");
    if policy == "required" {
        let factors = input["mfa_factors"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>();
        let rp_id = input["webauthn_rp_id"].as_str();
        let mut can = enrollable(&factors, rp_id, false);
        if can.is_empty() && factors.iter().any(|f| f == "sms") {
            // Read the SMS settings as the dashboard does: enabled with a stored secret.
            let sms_ready = target.call(Method::GET, "sms", &[], None).await.ok().is_some_and(|v| {
                let sms = v.get("sms").unwrap_or(&v);
                sms["enabled"] == true && sms["secret_set"] == true
            });
            can = enrollable(&factors, rp_id, sms_ready);
        }
        if can.is_empty() {
            return Err(usage("Required needs at least one factor that can be added now: TOTP, a passkey with a relying party ID, or SMS with enabled Twilio settings."));
        }
    }
    if current_policy(&settings, mfa.as_ref()) == "off" && policy != "off" {
        let effect = if policy == "required" { "Users without a factor must add one at their next sign-in." } else { "Users may add a factor; once they do, they need it at every sign-in." };
        eprintln!("Turning multi-factor {policy} in {}. {MFA_SDK_WARNING} {effect}", args.environment);
        if !yes {
            if !std::io::stdin().is_terminal() {
                return Err(usage("pass --yes to turn multi-factor on without a prompt, after you upgrade your app's Axiom sign-in SDK"));
            }
            if !dialoguer::Confirm::new().with_prompt(format!("Turn multi-factor {policy}?")).default(false).interact()? {
                return Err(usage("cancelled"));
            }
        }
    }
    let body = super::settings_put_body(&settings, &input);
    target.call_raw(Method::PUT, "settings", &[], Some(&body)).await.map_err(mfa_refusal)?;
    if !args.json {
        println!("Multi-factor settings saved.");
    }
    // The settings are saved; like the dashboard, a failed re-read does not turn it into a failure.
    if let Err(error) = print_mfa_status(args, &target).await {
        eprintln!("axiom auth: the multi-factor settings are saved, but their status could not be read ({error:#}); run `axiom auth mfa status`");
        if args.json {
            println!("{}", json!({"saved": true, "status": null}));
        }
    }
    Ok(())
}

/// `users factors`, `reset-factors`, `consents`, `revoke-consent`.
pub async fn user_factors(args: &AuthArgs, user_id: &str) -> Result<()> {
    let target = target(args).await?;
    let mut factors = target.call(Method::GET, &format!("users/{}/factors", checked_id(user_id)?), &[], None).await?;
    sanitize(&mut factors, false);
    print(args, &factors, |f| {
        let mut lines = f["factors"].as_array().into_iter().flatten().map(|x| format!("{}  {}  {}  {}  last used {}", text(x, "id"), text(x, "type"), text(x, "label"), text(x, "status"), text(x, "last_used_on"))).collect::<Vec<_>>();
        if lines.is_empty() {
            lines.push("No factors.".into());
        }
        lines.push(format!("Recovery codes remaining: {}", text(f, "recovery_codes_remaining")));
        lines
    })
}

pub async fn reset_factors(args: &AuthArgs, user_id: &str, reason: &str, yes: bool) -> Result<()> {
    let user_id = checked_id(user_id)?;
    if reason.trim().is_empty() || reason.len() > 256 {
        return Err(usage("--reason is 1 to 256 characters"));
    }
    confirm_yes(yes, &format!("Remove every factor and recovery code of user {user_id}?"))?;
    let target = target(args).await?;
    let result = target.call(Method::POST, &format!("users/{user_id}/factors/reset"), &[], Some(&json!({"reason": reason}))).await?;
    print(args, &result, |_| vec![format!("Factors of {user_id} reset; the user enrolls again at the next sign-in that needs a factor.")])
}

pub async fn user_consents(args: &AuthArgs, user_id: &str) -> Result<()> {
    let target = target(args).await?;
    let mut consents = target.call(Method::GET, &format!("users/{}/consents", checked_id(user_id)?), &[], None).await?;
    sanitize(&mut consents, false);
    print(args, &consents, |c| {
        let mut lines = c["consents"].as_array().into_iter().flatten().map(|x| format!("{}  {}  {}  scopes {}", text(x, "id"), text(x, "client_id"), text(x, "name"), x["scopes"])).collect::<Vec<_>>();
        if lines.is_empty() {
            lines.push("No consents.".into());
        }
        lines
    })
}

pub async fn revoke_consent(args: &AuthArgs, user_id: &str, consent_id: &str, yes: bool) -> Result<()> {
    let (user_id, consent_id) = (checked_id(user_id)?, checked_id(consent_id)?);
    confirm_yes(yes, &format!("Revoke consent {consent_id} of user {user_id}? The application's sessions end."))?;
    let target = target(args).await?;
    target.call(Method::DELETE, &format!("users/{user_id}/consents/{consent_id}"), &[], None).await?;
    print(args, &json!({"revoked": consent_id}), |_| vec![format!("Consent {consent_id} revoked.")])
}

// ---------------------------------------------------------------------------
// Webhooks, hooks, SMS
// ---------------------------------------------------------------------------

fn https_url(url: &str) -> Result<()> {
    match reqwest::Url::parse(url) {
        Ok(u) if u.scheme() == "https" && u.host_str().is_some() && u.username().is_empty() && u.password().is_none() => Ok(()),
        _ => Err(usage("URLs are https://host/path without credentials")),
    }
}

fn webhook_line(w: &Value) -> String {
    format!("{}  {}  {}  {}  failures {}", text(w, "id"), text(w, "url"), if w["enabled"] == true { "enabled" } else { "disabled" }, w["event_types"], text(w, "consecutive_failures"))
}

async fn webhook_revision(target: &Target, id: &str) -> Result<Value> {
    let listed = target.call(Method::GET, "webhooks", &[], None).await?;
    listed["webhooks"].as_array().into_iter().flatten().find(|w| w["id"].as_str() == Some(id)).map(|w| w["revision"].clone()).ok_or_else(|| usage(format!("no webhook {id}")))
}

pub async fn webhooks(args: &AuthArgs, action: &WebhooksAction) -> Result<()> {
    let sink = match action {
        WebhooksAction::Create { output_secret, url, .. } => {
            https_url(url)?;
            Some(secret_sink(output_secret.as_deref(), args.json)?)
        }
        WebhooksAction::RotateSecret { output_secret, .. } => Some(secret_sink(output_secret.as_deref(), args.json)?),
        _ => None,
    };
    let target = target(args).await?;
    webhooks_with(args, &target, action, sink.as_ref()).await
}

pub(super) async fn webhooks_with(args: &AuthArgs, target: &Target, action: &WebhooksAction, sink: Option<&SecretSink>) -> Result<()> {
    match action {
        WebhooksAction::List => {
            let mut listed = target.call(Method::GET, "webhooks", &[], None).await?;
            sanitize(&mut listed, true);
            print(args, &listed, |l| {
                let lines = l["webhooks"].as_array().into_iter().flatten().map(webhook_line).collect::<Vec<_>>();
                if lines.is_empty() {
                    vec!["No webhooks.".into()]
                } else {
                    lines
                }
            })
        }
        WebhooksAction::Create { url, events, enable, idempotency_key, .. } => {
            let key = idempotency(idempotency_key.as_deref())?;
            let mut created = target.keyed(Method::POST, "webhooks", &json!({"url": url, "event_types": events, "enabled": enable}), &key).await?;
            if let (Some(secret), Some(sink)) = (take_secret(&mut created, &["secret"]), sink) {
                deliver_secret(sink, &secret, "webhook signing secret")?;
            }
            sanitize(&mut created, true);
            print(args, &created, |w| vec![format!("Created {}", webhook_line(w.get("webhook").unwrap_or(w)))])
        }
        WebhooksAction::Update { id, url, events, enable, disable } => {
            let id = checked_id(id)?;
            let mut body = json!({"revision": webhook_revision(target, id).await?});
            if let Some(url) = url {
                https_url(url)?;
                body["url"] = json!(url);
            }
            if !events.is_empty() {
                body["event_types"] = json!(events);
            }
            if *enable || *disable {
                body["enabled"] = json!(*enable);
            }
            if body.as_object().is_some_and(|m| m.len() == 1) {
                return Err(usage("nothing to change"));
            }
            let mut saved = target.call(Method::PATCH, &format!("webhooks/{id}"), &[], Some(&body)).await?;
            sanitize(&mut saved, true);
            print(args, &saved, |w| vec![format!("Saved {}", webhook_line(w.get("webhook").unwrap_or(w)))])
        }
        WebhooksAction::RotateSecret { id, .. } => {
            let mut rotated = target.call(Method::POST, &format!("webhooks/{}/secret", checked_id(id)?), &[], Some(&json!({}))).await?;
            match (take_secret(&mut rotated, &["secret"]), sink) {
                (Some(secret), Some(sink)) => deliver_secret(sink, &secret, "webhook signing secret")?,
                _ => return Err(Exit { code: 5, message: "AXAUTH_UNAVAILABLE: the response carried no secret".into() }.into()),
            }
            sanitize(&mut rotated, true);
            print(args, &rotated, |_| vec!["Signing secret rotated.".into()])
        }
        WebhooksAction::Test { id } => {
            let result = target.call(Method::POST, &format!("webhooks/{}/test", checked_id(id)?), &[], Some(&json!({}))).await?;
            print(args, &result, |r| vec![format!("Ping: {}", text(r, "result"))])
        }
        WebhooksAction::Deliveries { id, state, limit, cursor } => {
            let mut query = vec![];
            if let Some(state) = state {
                query.push(("state", state.clone()));
            }
            if let Some(limit) = limit {
                query.push(("limit", limit.to_string()));
            }
            if let Some(cursor) = cursor {
                query.push(("cursor", cursor.clone()));
            }
            let mut listed = target.call(Method::GET, &format!("webhooks/{}/deliveries", checked_id(id)?), &query, None).await?;
            sanitize(&mut listed, false);
            print(args, &listed, |l| {
                let mut lines = l["deliveries"].as_array().into_iter().flatten().map(|d| format!("{}  {}  {}  attempts {}  last {}", text(d, "event_id"), text(d, "event_type"), text(d, "state"), text(d, "attempts"), text(d, "last_status"))).collect::<Vec<_>>();
                if let Some(next) = l["next_cursor"].as_str() {
                    lines.push(format!("More: --cursor {next}"));
                }
                lines
            })
        }
        WebhooksAction::Replay { id, delivery_id } => {
            let result = target.call(Method::POST, &format!("webhooks/{}/deliveries/{}/replay", checked_id(id)?, checked_id(delivery_id)?), &[], Some(&json!({}))).await?;
            print(args, &result, |_| vec!["Replay queued.".into()])
        }
        WebhooksAction::Delete { id, yes } => {
            let id = checked_id(id)?;
            confirm_yes(*yes, &format!("Delete webhook {id}? Pending deliveries are dropped."))?;
            target.call(Method::DELETE, &format!("webhooks/{id}"), &[], None).await?;
            print(args, &json!({"deleted": id}), |_| vec![format!("Deleted {id}.")])
        }
    }
}

fn hook_line(h: &Value) -> String {
    format!("{}  {}  policy {}  timeout {} ms  {}", text(h, "kind"), text(h, "url"), text(h, "failure_policy"), text(h, "timeout_ms"), if h["enabled"] == true { "enabled" } else { "disabled" })
}

pub async fn hooks(args: &AuthArgs, action: &HooksAction) -> Result<()> {
    let sink = match action {
        HooksAction::Set { output_secret, url, .. } => {
            https_url(url)?;
            // The secret is returned only at the first save; a terminal or a file must be ready.
            match secret_sink(output_secret.as_deref(), args.json) {
                Ok(sink) => Some(sink),
                Err(_) if output_secret.is_none() => None,
                Err(error) => return Err(error),
            }
        }
        HooksAction::RotateSecret { output_secret, .. } => Some(secret_sink(output_secret.as_deref(), args.json)?),
        _ => None,
    };
    let target = target(args).await?;
    match action {
        HooksAction::Get { kind } => {
            let mut listed = target.call(Method::GET, "hooks", &[], None).await?;
            sanitize(&mut listed, true);
            let shown = match kind {
                Some(kind) => listed["hooks"].as_array().into_iter().flatten().find(|h| h["kind"].as_str() == Some(kind)).cloned().unwrap_or(Value::Null),
                None => listed,
            };
            print(args, &shown, |l| {
                let items = match l.get("hooks") {
                    Some(Value::Array(items)) => items.clone(),
                    _ if l.is_object() => vec![l.clone()],
                    _ => vec![],
                };
                let lines = items.iter().map(hook_line).collect::<Vec<_>>();
                if lines.is_empty() {
                    vec!["No hooks.".into()]
                } else {
                    lines
                }
            })
        }
        HooksAction::Set { kind, url, policy, timeout_ms, grantable_roles, include_email, enable, disable, .. } => {
            let listed = target.call(Method::GET, "hooks", &[], None).await?;
            let existing = listed["hooks"].as_array().into_iter().flatten().find(|h| h["kind"].as_str() == Some(kind)).cloned();
            if existing.is_none() && sink.is_none() {
                return Err(usage("the first save returns the hook's signing secret once: pass --output-secret <file>"));
            }
            let mut body = json!({"url": url, "failure_policy": policy, "include_email": include_email});
            if let Some(timeout) = timeout_ms {
                body["timeout_ms"] = json!(timeout);
            }
            if !grantable_roles.is_empty() {
                body["grantable_roles"] = json!(grantable_roles);
            }
            if *enable || *disable {
                body["enabled"] = json!(*enable);
            }
            if let Some(existing) = &existing {
                body["revision"] = existing["revision"].clone();
            }
            let mut saved = target.call(Method::PUT, &format!("hooks/{kind}"), &[], Some(&body)).await?;
            if let (Some(secret), Some(sink)) = (take_secret(&mut saved, &["secret"]), &sink) {
                deliver_secret(sink, &secret, "hook signing secret")?;
            }
            sanitize(&mut saved, true);
            print(args, &saved, |h| vec![format!("Saved {}", hook_line(h.get("hook").unwrap_or(h)))])
        }
        HooksAction::Test { kind, sample } => {
            let result = target.call(Method::POST, &format!("hooks/{kind}/test"), &[], Some(&json!({"sample": sample}))).await?;
            print(args, &result, |r| vec![format!("Result {} in {} ms; validation {}", text(r, "result"), text(r, "latency_ms"), text(r, "validation"))])
        }
        HooksAction::RotateSecret { kind, .. } => {
            let mut rotated = target.call(Method::POST, &format!("hooks/{kind}/secret"), &[], Some(&json!({}))).await?;
            match (take_secret(&mut rotated, &["secret"]), &sink) {
                (Some(secret), Some(sink)) => deliver_secret(sink, &secret, "hook signing secret")?,
                _ => return Err(Exit { code: 5, message: "AXAUTH_UNAVAILABLE: the response carried no secret".into() }.into()),
            }
            print(args, &json!({"rotated": kind}), |_| vec!["Signing secret rotated.".into()])
        }
        HooksAction::Remove { kind, yes } => {
            confirm_yes(*yes, &format!("Remove the {kind} hook?"))?;
            target.call(Method::DELETE, &format!("hooks/{kind}"), &[], None).await?;
            print(args, &json!({"removed": kind}), |_| vec![format!("Removed the {kind} hook.")])
        }
    }
}

/// `PUT sms` without the secret; the secret, when given, is added by the caller.
#[allow(clippy::too_many_arguments)]
pub fn sms_body(account_sid: &str, api_key_sid: Option<&str>, messaging_service: Option<&str>, from_number: Option<&str>, countries: &[String], monthly_cap: u64, daily_cap: Option<u64>, hourly_cap: Option<u64>) -> Result<Value> {
    let sid = |v: &str, prefix: &str| v.len() == 34 && v.starts_with(prefix) && v[2..].bytes().all(|b| b.is_ascii_hexdigit());
    if !sid(account_sid, "AC") {
        return Err(usage("--account-sid is AC followed by 32 hex digits"));
    }
    if api_key_sid.is_some_and(|k| !sid(k, "SK")) {
        return Err(usage("--api-key-sid is SK followed by 32 hex digits"));
    }
    match (messaging_service, from_number) {
        (Some(m), None) if sid(m, "MG") => {}
        (Some(_), None) => return Err(usage("--messaging-service is MG followed by 32 hex digits")),
        (None, Some(n)) if n.starts_with('+') && (8..=16).contains(&n.len()) && n[1..].bytes().all(|b| b.is_ascii_digit()) => {}
        (None, Some(_)) => return Err(usage("--from-number is an E.164 number, for example +15551234567")),
        _ => return Err(usage("pass --messaging-service or --from-number")),
    }
    if countries.iter().any(|c| c.len() != 2 || !c.bytes().all(|b| b.is_ascii_uppercase())) {
        return Err(usage("--country takes ISO 3166 alpha-2 codes such as US"));
    }
    let mut body = json!({
        "account_sid": account_sid,
        "credential_kind": if api_key_sid.is_some() { "api_key" } else { "auth_token" },
        "country_allowlist": countries,
        "monthly_cap": monthly_cap,
    });
    if let Some(k) = api_key_sid {
        body["api_key_sid"] = json!(k);
    }
    if let Some(m) = messaging_service {
        body["messaging_service_sid"] = json!(m);
    }
    if let Some(n) = from_number {
        body["from_number"] = json!(n);
    }
    if let Some(d) = daily_cap {
        body["daily_cap"] = json!(d);
    }
    if let Some(h) = hourly_cap {
        body["hourly_cap"] = json!(h);
    }
    Ok(body)
}

pub async fn sms(args: &AuthArgs, action: &SmsAction) -> Result<()> {
    match action {
        SmsAction::Set { account_sid, auth_token_stdin, api_key_sid, api_key_secret_stdin, messaging_service, from_number, countries, monthly_cap, daily_cap, hourly_cap, enable, disable } => {
            let mut body = sms_body(account_sid, api_key_sid.as_deref(), messaging_service.as_deref(), from_number.as_deref(), countries, *monthly_cap, *daily_cap, *hourly_cap)?;
            let secret = if *auth_token_stdin || *api_key_secret_stdin { Some(read_secret_input(true, "Twilio secret", "--auth-token-stdin or --api-key-secret-stdin", 1024)?) } else { None };
            let target = target(args).await?;
            let current = target.call(Method::GET, "sms", &[], None).await.ok();
            if let Some(current) = &current {
                body["revision"] = current.get("sms").unwrap_or(current)["revision"].clone();
            }
            let kind_changes = current.as_ref().map_or(true, |c| c.get("sms").unwrap_or(c)["credential_kind"] != body["credential_kind"]);
            if kind_changes && secret.is_none() {
                return Err(usage("the Twilio secret is required for a new configuration or a credential change: pass --auth-token-stdin or --api-key-secret-stdin"));
            }
            if *enable || *disable {
                body["enabled"] = json!(*enable);
            }
            if let Some(secret) = &secret {
                body["secret"] = json!(secret.as_str());
            }
            let result = target.call(Method::PUT, "sms", &[], Some(&body)).await;
            if let Some(map) = body.as_object_mut() {
                map.remove("secret");
            }
            let mut saved = result?;
            sanitize(&mut saved, false);
            print(args, &saved, |s| {
                let s = s.get("sms").unwrap_or(s);
                vec![format!("SMS saved: secret {}, status callbacks {}.", if s["secret_set"] == true { "set" } else { "not set" }, text(s, "status_callbacks"))]
            })
        }
        SmsAction::Get => {
            let target = target(args).await?;
            let mut settings = target.call(Method::GET, "sms", &[], None).await?;
            sanitize(&mut settings, false);
            print(args, &settings, |s| {
                let s = s.get("sms").unwrap_or(s);
                vec![
                    format!("Account {} ({}), {}", text(s, "account_sid"), text(s, "credential_kind"), if s["enabled"] == true { "enabled" } else { "disabled" }),
                    format!("Countries {}; caps monthly {} daily {} hourly {}", s["country_allowlist"], text(s, "monthly_cap"), text(s, "daily_cap"), text(s, "hourly_cap")),
                ]
            })
        }
        SmsAction::Test { to } => {
            if !to.starts_with('+') || !(8..=16).contains(&to.len()) || !to[1..].bytes().all(|b| b.is_ascii_digit()) {
                return Err(usage("--to is an E.164 number in the country allowlist"));
            }
            let target = target(args).await?;
            let result = target.call(Method::POST, "sms/test", &[], Some(&json!({"to": to}))).await?;
            print(args, &result, |r| vec![format!("Test message: {}", text(r, "result"))])
        }
        SmsAction::Remove { yes } => {
            confirm_yes(*yes, "Remove the SMS configuration? SMS factors stop working.")?;
            let target = target(args).await?;
            target.call(Method::DELETE, "sms", &[], None).await?;
            print(args, &json!({"removed": true}), |_| vec!["SMS configuration removed.".into()])
        }
    }
}

#[cfg(test)]
mod tests;
