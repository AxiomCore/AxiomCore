//! `axiom auth email|smtp|templates`: authentication email delivery (the Dashboard's
//! Authentication > Methods > Email section).
//!
//! The SMTP password comes only from stdin or a masked prompt, never argv, and is never
//! printed: `smtp get` shows only whether it is set. Template text is plain text with the
//! documented `{{variables}}`; it is checked here with the backend's rules before sending.
use super::advanced::{confirm_yes, read_secret_input};
use super::{looks_like_email, print, sanitize, target, text, usage, AuthArgs, Target};
use anyhow::{Context, Result};
use clap::builder::PossibleValuesParser;
use clap::Subcommand;
use reqwest::Method;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// Template kinds (auth-admin-client.js `TEMPLATE_KINDS`, backend `TemplateKinds`).
pub const TEMPLATE_KINDS: [&str; 8] = ["email_code", "magic_link", "signup", "verify_email", "password_reset", "email_change_confirm", "email_change_notice", "invitation"];

/// Request body bound of `PUT templates/{kind}` and its preview (the proxy and Dashboard limit).
pub const TEMPLATE_BODY_LIMIT: usize = 64 << 10;

/// SMTP ports the backend accepts.
const SMTP_PORTS: [&str; 3] = ["465", "587", "2525"];

#[derive(Subcommand, Debug)]
pub enum EmailAction {
    /// This month's platform email usage, today's sending shares and suppression counts
    Usage,
    /// Remove an address from this environment's suppression list so it receives email again
    Unsuppress {
        /// The suppressed email address
        email: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum SmtpAction {
    /// Show the custom SMTP settings (the password is never shown, only whether it is set)
    Get {
        /// Show the user name and from address unmasked
        #[arg(long)]
        reveal_email: bool,
    },
    /// Create or change the custom SMTP settings (Owner). Flags left out keep their current
    /// values; a new configuration needs every field and a password. Until an Owner's test
    /// passes, messages are sent with the platform sender
    Set {
        /// SMTP host name or public IP address
        #[arg(long)]
        host: Option<String>,
        /// SMTP port: 465, 587 or 2525 (default for a new configuration: 587)
        #[arg(long, value_parser = SMTP_PORTS)]
        port: Option<String>,
        /// User name for signing in to the SMTP server
        #[arg(long)]
        username: Option<String>,
        /// Sender address of authentication email
        #[arg(long)]
        from_address: Option<String>,
        /// Sender display name
        #[arg(long, conflicts_with = "clear_from_name")]
        from_name: Option<String>,
        /// Remove the sender display name
        #[arg(long)]
        clear_from_name: bool,
        /// Read the SMTP password from stdin
        #[arg(long, conflicts_with = "password_prompt")]
        password_stdin: bool,
        /// Ask for the SMTP password with a masked prompt
        #[arg(long)]
        password_prompt: bool,
    },
    /// Delete the custom SMTP settings (Owner). Refused while the environment's email sender is SMTP
    Remove {
        /// Confirm without a prompt
        #[arg(long)]
        yes: bool,
    },
    /// Send one test message through the custom SMTP server (5 per hour)
    Test {
        /// Address that receives the test message
        #[arg(long)]
        to: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum TemplatesAction {
    /// List every template kind and whether it is customized
    List,
    /// Show a template's subject, intro and outro, and Axiom's default text
    Show {
        /// Template kind
        #[arg(value_parser = PossibleValuesParser::new(TEMPLATE_KINDS))]
        kind: String,
    },
    /// Customize a template. Fields left out keep their current text; an empty value uses the default text
    Set {
        /// Template kind
        #[arg(value_parser = PossibleValuesParser::new(TEMPLATE_KINDS))]
        kind: String,
        /// Subject line (one line, at most 600 bytes)
        #[arg(long)]
        subject: Option<String>,
        /// File with the text before the code or link (plain text, at most 2,000 bytes)
        #[arg(long)]
        intro_file: Option<PathBuf>,
        /// File with the text after the code or link (plain text, at most 2,000 bytes)
        #[arg(long)]
        outro_file: Option<PathBuf>,
    },
    /// Remove the customization so messages use Axiom's default text again
    Reset {
        /// Template kind
        #[arg(value_parser = PossibleValuesParser::new(TEMPLATE_KINDS))]
        kind: String,
        /// Confirm without a prompt
        #[arg(long)]
        yes: bool,
    },
    /// Render a template with sample values; sends nothing. Without text flags the saved template is rendered
    Preview {
        /// Template kind
        #[arg(value_parser = PossibleValuesParser::new(TEMPLATE_KINDS))]
        kind: String,
        /// Subject line to try
        #[arg(long)]
        subject: Option<String>,
        /// File with intro text to try
        #[arg(long)]
        intro_file: Option<PathBuf>,
        /// File with outro text to try
        #[arg(long)]
        outro_file: Option<PathBuf>,
        /// Print the HTML part instead of the plain-text part
        #[arg(long)]
        html: bool,
    },
}

// ---------------------------------------------------------------------------
// Email usage and suppressions
// ---------------------------------------------------------------------------

pub async fn email(args: &AuthArgs, action: &EmailAction) -> Result<()> {
    if let EmailAction::Unsuppress { email } = action {
        valid_email(email, "the address")?;
    }
    let target = target(args).await?;
    email_with(args, &target, action).await
}

fn valid_email(value: &str, what: &str) -> Result<()> {
    if value.len() > 320 || !looks_like_email(value) {
        return Err(usage(format!("{what} is not an email address")));
    }
    Ok(())
}

pub(super) async fn email_with(args: &AuthArgs, target: &Target, action: &EmailAction) -> Result<()> {
    match action {
        EmailAction::Usage => {
            let mut usage_view = target.call(Method::GET, "email/usage", &[], None).await?;
            sanitize(&mut usage_view, false);
            print(args, &usage_view, |u| {
                let mut lines = vec![
                    format!("Since {}: {} of {} platform emails sent ({} reserved for recovery email).", text(u, "period_start"), text(u, "sent"), text(u, "budget"), text(u, "recovery_reserve")),
                    format!("Sent with custom SMTP: {}.", text(u, "smtp_sent")),
                ];
                if let Some(daily) = u["daily"].as_object().filter(|d| !d.is_empty()) {
                    let shares = daily.iter().map(|(name, c)| format!("{name} {}/{}", text(c, "used"), text(c, "limit"))).collect::<Vec<_>>();
                    lines.push(format!("Today: {}.", shares.join(", ")));
                }
                let s = &u["suppressions"];
                lines.push(format!("Suppressed addresses: {} in this environment; {} permanent bounces from this environment across Axiom.", text(s, "environment"), text(s, "global_from_environment")));
                lines
            })
        }
        EmailAction::Unsuppress { email } => {
            valid_email(email, "the address")?;
            let removed = target.call(Method::POST, "email/suppressions/remove", &[], Some(&json!({"email": email}))).await?;
            let report = json!({"removed": removed["removed"] == true});
            print(args, &report, |r| {
                vec![if r["removed"] == true {
                    "The address can receive email again.".into()
                } else {
                    "No suppression was found for this address in this environment. Permanent bounces across Axiom are removed only by Axiom support.".into()
                }]
            })
        }
    }
}

// ---------------------------------------------------------------------------
// SMTP
// ---------------------------------------------------------------------------

pub async fn smtp(args: &AuthArgs, action: &SmtpAction) -> Result<()> {
    let password = match action {
        SmtpAction::Set { password_stdin, password_prompt, .. } if *password_stdin || *password_prompt => Some(read_secret_input(*password_stdin, "SMTP password", "--password-stdin", 1024)?),
        SmtpAction::Test { to } => {
            valid_email(to, "--to")?;
            None
        }
        _ => None,
    };
    let target = target(args).await?;
    smtp_with(args, &target, action, password).await
}

fn url_like(value: &str) -> bool {
    // Backend `URLLike`: variables masked with an alphanumeric, then three patterns.
    let masked = regex::Regex::new(r"\{\{[a-z_]+\}\}").expect("static regex").replace_all(value, "x");
    [r"(?i)([a-z][a-z0-9+.-]*:)?//", r"(?i)\bwww\.", r"(?i)[a-z0-9-]+(\.[a-z0-9-]+)*\.[a-z]{2,63}\b"].iter().any(|p| regex::Regex::new(p).expect("static regex").is_match(&masked))
}

fn control(value: &str) -> bool {
    value.chars().any(|c| c.is_control() && !matches!(c, '\t' | '\r' | '\n'))
}

/// The display-name rule of the backend (`ValidDisplayName`).
fn name_problem(label: &str, value: &str) -> Option<String> {
    if value.len() > 64 {
        Some(format!("{label} is at most 64 bytes"))
    } else if value.contains(['"', '<', '>', '@', '\r', '\n']) || control(value) {
        Some(format!("{label} must not contain \" < > @, line breaks or control characters"))
    } else if url_like(value) {
        Some(format!("{label} must not look like a web address or domain name"))
    } else {
        None
    }
}

/// A current SMTP field the caller may see (Readers get `***`).
fn current_field(current: &Value, key: &str) -> Option<String> {
    current["configured"].as_bool().filter(|c| *c)?;
    current[key].as_str().filter(|s| !s.is_empty() && *s != "***").map(str::to_string)
}

/// `PUT smtp` without the password: flags over the current settings, plus the current
/// revision (0 creates). Checked with the Dashboard's and backend's rules.
pub fn smtp_body(current: &Value, host: Option<&str>, port: Option<&str>, username: Option<&str>, from_address: Option<&str>, from_name: Option<&str>, clear_from_name: bool) -> Result<Value> {
    let host = host.map(|h| h.trim().to_string()).or_else(|| current_field(current, "host")).ok_or_else(|| usage("pass --host"))?;
    if host.is_empty() || host.len() > 253 || !host.bytes().all(|b| b.is_ascii_alphanumeric() || b".:[]-".contains(&b)) {
        return Err(usage("--host is an SMTP host name or public IP address"));
    }
    let port: u16 = match port {
        Some(p) => p.parse().map_err(|_| usage("--port is 465, 587 or 2525"))?,
        None => current["port"].as_u64().filter(|_| current["configured"] == true).map_or(587, |p| p as u16),
    };
    if !SMTP_PORTS.contains(&port.to_string().as_str()) {
        return Err(usage("--port is 465, 587 or 2525"));
    }
    let username = username.map(|u| u.trim().to_string()).or_else(|| current_field(current, "username")).ok_or_else(|| usage("pass --username"))?;
    if username.is_empty() || username.len() > 256 || username.contains(['\r', '\n']) || control(&username) {
        return Err(usage("--username is 1 to 256 bytes on one line"));
    }
    let from_address = from_address.map(|f| f.trim().to_string()).or_else(|| current_field(current, "from_address")).ok_or_else(|| usage("pass --from-address"))?;
    valid_email(&from_address, "--from-address")?;
    let from_name = if clear_from_name {
        None
    } else {
        match from_name {
            Some(name) => Some(name.trim().to_string()).filter(|n| !n.is_empty()),
            None => current["from_name"].as_str().map(str::to_string),
        }
    };
    if let Some(problem) = from_name.as_deref().and_then(|n| name_problem("--from-name", n)) {
        return Err(usage(problem));
    }
    let revision = if current["configured"] == true { current["revision"].as_i64().unwrap_or(0) } else { 0 };
    Ok(json!({"host": host, "port": port, "username": username, "from_address": from_address, "from_name": from_name, "revision": revision}))
}

fn smtp_result(result: &str) -> &str {
    match result {
        "ok" => "delivered to the test address",
        "tls_failed" => "TLS failed",
        "auth_failed" => "sign-in to the SMTP server failed",
        "refused_address" => "the server refused the address",
        "unreachable" => "server unreachable",
        "untested" => "not tested",
        other => other,
    }
}

fn smtp_lines(s: &Value) -> Vec<String> {
    if s["configured"] != true {
        return vec!["No custom SMTP settings; email is sent with the platform sender.".into()];
    }
    let from = match s["from_name"].as_str() {
        Some(name) => format!("{name} <{}>", text(s, "from_address")),
        None => text(s, "from_address"),
    };
    vec![
        format!("{}:{} as {}, password {} (revision {})", text(s, "host"), text(s, "port"), text(s, "username"), if s["password_set"] == true { "set" } else { "not set" }, text(s, "revision")),
        format!("From {from}"),
        format!("Last test: {}{}", smtp_result(s["last_test"]["result"].as_str().unwrap_or("untested")), s["last_test"]["at"].as_str().map(|at| format!(" at {at}")).unwrap_or_default()),
    ]
}

pub(super) async fn smtp_with(args: &AuthArgs, target: &Target, action: &SmtpAction, password: Option<Zeroizing<String>>) -> Result<()> {
    match action {
        SmtpAction::Get { reveal_email } => {
            let mut view = target.call(Method::GET, "smtp", &[], None).await?;
            sanitize(&mut view, *reveal_email);
            print(args, &view, smtp_lines)
        }
        SmtpAction::Set { host, port, username, from_address, from_name, clear_from_name, .. } => {
            let current = target.call(Method::GET, "smtp", &[], None).await?;
            let mut body = smtp_body(&current, host.as_deref(), port.as_deref(), username.as_deref(), from_address.as_deref(), from_name.as_deref(), *clear_from_name)?;
            match &password {
                Some(secret) => body["password"] = json!(secret.as_str()),
                None if current["password_set"] != true => return Err(usage("a new SMTP configuration needs the password: pass --password-stdin or --password-prompt")),
                None => {}
            }
            let result = target.call(Method::PUT, "smtp", &[], Some(&body)).await;
            if let Some(map) = body.as_object_mut() {
                map.remove("password");
            }
            drop(password);
            let mut saved = result?;
            sanitize(&mut saved, false);
            print(args, &saved, |s| {
                let mut lines = smtp_lines(s);
                lines.insert(0, "SMTP settings saved.".into());
                if s["last_test"]["result"] != "ok" {
                    lines.push("Until an Owner's test passes, messages are sent with the platform sender: run `axiom auth smtp test --to <address>`.".into());
                }
                lines
            })
        }
        SmtpAction::Remove { yes } => {
            confirm_yes(*yes, "Remove the custom SMTP settings? The host, user name and password are deleted.")?;
            target.call(Method::DELETE, "smtp", &[], None).await?;
            print(args, &json!({"deleted": true}), |_| vec!["Custom SMTP is removed.".into()])
        }
        SmtpAction::Test { to } => {
            valid_email(to, "--to")?;
            let result = target.call(Method::POST, "smtp/test", &[], Some(&json!({"to": to}))).await?;
            print(args, &result, |r| vec![format!("SMTP test: {}.", smtp_result(r["result"].as_str().unwrap_or("-")))])
        }
    }
}

// ---------------------------------------------------------------------------
// Templates
// ---------------------------------------------------------------------------

pub async fn templates(args: &AuthArgs, action: &TemplatesAction) -> Result<()> {
    let target = target(args).await?;
    templates_with(args, &target, action).await
}

fn template_kind(kind: &str) -> Result<&str> {
    if TEMPLATE_KINDS.contains(&kind) {
        Ok(kind)
    } else {
        Err(usage(format!("`{kind}` is not a template kind; use one of {}", TEMPLATE_KINDS.join(", "))))
    }
}

fn template_variables(kind: &str) -> Vec<&'static str> {
    let mut variables = vec!["{{app_name}}", "{{expires_minutes}}", "{{recipient_email}}"];
    if kind == "email_change_notice" {
        variables.push("{{new_email}}");
    }
    variables
}

/// The backend's `ValidateTemplate` rules for one field: byte bound, plain text, known
/// variables only, no URL- or domain-like text.
pub fn template_problem(kind: &str, field: &str, value: &str, max: usize) -> Option<String> {
    if value.len() > max {
        return Some(format!("{field} is at most {max} bytes"));
    }
    if control(value) || field == "the subject" && value.contains(['\r', '\n']) {
        return Some(format!("{field} must be plain text{}", if field == "the subject" { " on one line" } else { "" }));
    }
    let allowed = template_variables(kind);
    let mut masked = value.to_string();
    for found in regex::Regex::new(r"\{\{[a-z_]+\}\}").expect("static regex").find_iter(value) {
        if !allowed.contains(&found.as_str()) {
            return Some(format!("{field} uses {}, which is not available; use {}", found.as_str(), allowed.join(", ")));
        }
        masked = masked.replacen(found.as_str(), "", 1);
    }
    if masked.contains("{{") || masked.contains("}}") {
        return Some(format!("{field} has an unclosed or unknown {{{{variable}}}}; use {}", allowed.join(", ")));
    }
    if url_like(value) {
        return Some(format!("{field} must not contain links, web addresses or domain names; the message adds its own sign-in link"));
    }
    None
}

/// Template text from a file: UTF-8, at most the template body limit; trailing line
/// breaks are dropped and an empty file means the default text.
pub fn read_template_file(path: &Path) -> Result<Option<String>> {
    let meta = std::fs::metadata(path).with_context(|| format!("read {}", path.display()))?;
    if !meta.is_file() || meta.len() > TEMPLATE_BODY_LIMIT as u64 {
        return Err(usage(format!("{} must be a file of at most 64 KiB", path.display())));
    }
    let raw = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let text = String::from_utf8(raw).map_err(|_| usage(format!("{} is not UTF-8", path.display())))?;
    Ok(Some(text.trim_end_matches(['\r', '\n']).to_string()).filter(|t| !t.is_empty()))
}

/// `{subject, intro, outro}`: the given fields over the current template (fields left
/// out keep the current text; an empty value is null, meaning the default text).
pub fn template_body(kind: &str, current: &Value, subject: Option<&str>, intro_file: Option<&Path>, outro_file: Option<&Path>) -> Result<Value> {
    let keep = |key: &str| current[key].as_str().filter(|s| !s.is_empty()).map(str::to_string);
    let subject = match subject {
        Some(s) => Some(s.to_string()).filter(|s| !s.is_empty()),
        None => keep("subject"),
    };
    let intro = match intro_file {
        Some(path) => read_template_file(path)?,
        None => keep("intro"),
    };
    let outro = match outro_file {
        Some(path) => read_template_file(path)?,
        None => keep("outro"),
    };
    for (field, value, max) in [("the subject", &subject, 600), ("the intro", &intro, 2000), ("the outro", &outro, 2000)] {
        if let Some(problem) = value.as_deref().and_then(|v| template_problem(kind, field, v, max)) {
            return Err(usage(problem));
        }
    }
    let body = json!({"subject": subject, "intro": intro, "outro": outro});
    if serde_json::to_vec(&body)?.len() > TEMPLATE_BODY_LIMIT {
        return Err(usage("a template is at most 64 KiB"));
    }
    Ok(body)
}

async fn template_view(target: &Target, kind: &str) -> Result<Value> {
    target.call(Method::GET, &format!("templates/{kind}"), &[], None).await
}

fn template_lines(t: &Value) -> Vec<String> {
    let field = |key: &str| match t[key].as_str() {
        Some(value) if !value.is_empty() => value.to_string(),
        _ => format!("(default) {}", t["defaults"][key].as_str().unwrap_or("-")),
    };
    vec![
        format!("{}: {} (revision {})", text(t, "kind"), if t["overridden"] == true { "customized" } else { "default" }, text(t, "revision")),
        format!("Subject: {}", field("subject")),
        format!("Intro: {}", field("intro")),
        format!("Outro: {}", field("outro")),
    ]
}

pub(super) async fn templates_with(args: &AuthArgs, target: &Target, action: &TemplatesAction) -> Result<()> {
    match action {
        TemplatesAction::List => {
            let mut listed = target.call(Method::GET, "templates", &[], None).await?;
            sanitize(&mut listed, true);
            print(args, &listed, |l| {
                l["templates"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|t| format!("{}  {}  revision {}  {}", text(t, "kind"), if t["overridden"] == true { "customized" } else { "default" }, text(t, "revision"), t["subject"].as_str().unwrap_or("")))
                    .collect()
            })
        }
        TemplatesAction::Show { kind } => {
            let mut view = template_view(target, template_kind(kind)?).await?;
            sanitize(&mut view, true);
            print(args, &view, template_lines)
        }
        TemplatesAction::Set { kind, subject, intro_file, outro_file } => {
            let kind = template_kind(kind)?;
            if subject.is_none() && intro_file.is_none() && outro_file.is_none() {
                return Err(usage("nothing to change: pass --subject, --intro-file or --outro-file"));
            }
            let current = template_view(target, kind).await?;
            let mut body = template_body(kind, &current, subject.as_deref(), intro_file.as_deref(), outro_file.as_deref())?;
            if body["subject"].is_null() && body["intro"].is_null() && body["outro"].is_null() {
                return Err(usage(format!("every field is empty; run `axiom auth templates reset {kind}` to use the default text")));
            }
            // The revision of the current override (0 creates one).
            body["revision"] = json!(current["revision"].as_i64().unwrap_or(0));
            let mut saved = target.call(Method::PUT, &format!("templates/{kind}"), &[], Some(&body)).await?;
            sanitize(&mut saved, true);
            print(args, &saved, |t| {
                let mut lines = template_lines(t);
                lines.insert(0, "Template saved.".into());
                lines
            })
        }
        TemplatesAction::Reset { kind, yes } => {
            let kind = template_kind(kind)?;
            confirm_yes(*yes, &format!("Reset the {kind} template? Messages use Axiom's default text again."))?;
            target.call(Method::DELETE, &format!("templates/{kind}"), &[], None).await?;
            print(args, &json!({"deleted": true, "kind": kind}), |_| vec![format!("The {kind} template is reset to the default text.")])
        }
        TemplatesAction::Preview { kind, subject, intro_file, outro_file, html } => {
            let kind = template_kind(kind)?;
            let body = if subject.is_none() && intro_file.is_none() && outro_file.is_none() {
                json!({})
            } else {
                let current = template_view(target, kind).await?;
                template_body(kind, &current, subject.as_deref(), intro_file.as_deref(), outro_file.as_deref())?
            };
            let rendered = target.call(Method::POST, &format!("templates/{kind}/preview"), &[], Some(&body)).await?;
            print(args, &rendered, |r| vec![format!("Subject: {}", text(r, "subject")), String::new(), text(r, if *html { "html" } else { "text" })])
        }
    }
}

#[cfg(test)]
mod tests;
