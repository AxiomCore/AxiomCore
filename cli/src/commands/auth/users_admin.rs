//! `axiom auth users ...` (delete, sessions, export, send-verification, remove-factor) and
//! `axiom auth events`: user lifecycle administration and the audit trail.
//!
//! A user export carries personal data. Like the dashboard (which saves it as a
//! downloaded file), it is written only to a new `--output` file created with mode 0600
//! (an existing file is never replaced), or printed to stdout when `--json` is given.
use super::advanced::confirm_yes;
use super::{checked_id, print, sanitize, target, text, usage, AuthArgs, Exit, Target};
use anyhow::{Context, Result};
use clap::Subcommand;
use reqwest::Method;
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

/// The event types the audit trail can be filtered by (the backend refuses any other).
pub const EVENT_TYPES: [&str; 25] = [
    "user_created",
    "sign_in",
    "exchange",
    "password_changed",
    "session_reuse_detected",
    "sessions_revoked",
    "user_banned",
    "user_unbanned",
    "role_changed",
    "permission_changed",
    "assignment_granted",
    "assignment_revoked",
    "organization_changed",
    "membership_changed",
    "email_verified",
    "email_changed",
    "password_reset",
    "identity_linked",
    "identity_unlinked",
    "invitation_created",
    "invitation_revoked",
    "invitation_accepted",
    "user_deleted",
    "user_exported",
    "authorization_code_replayed",
];

/// Flattened into `axiom auth users`.
#[derive(Subcommand, Debug)]
pub enum UsersAdminAction {
    /// Delete a user: they are signed out everywhere and their authentication records are removed.
    /// Your application's own tables are not changed. Run it again if it reports that records remain
    Delete {
        /// The user's ID (UUID)
        user_id: String,
        /// Delete without asking for confirmation
        #[arg(long)]
        yes: bool,
    },
    /// List a user's sessions (active and ended)
    Sessions {
        /// The user's ID (UUID)
        user_id: String,
    },
    /// Export everything Axiom stores about a user (personal data) to a new file
    Export {
        /// The user's ID (UUID)
        user_id: String,
        /// Write the export to this new file (created with mode 0600; an existing file is never replaced).
        /// Without it, the export is printed only when --json is given
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Send the user an email with a link to verify their address
    SendVerification {
        /// The user's ID (UUID)
        user_id: String,
    },
    /// Remove one factor (authenticator app or passkey) of a user; their sessions continue
    RemoveFactor {
        /// The user's ID (UUID)
        user_id: String,
        /// The factor's ID (UUID), as shown by `axiom auth users factors`
        factor_id: String,
        /// Remove without asking for confirmation
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum EventsAction {
    /// List audit trail events, newest first
    List {
        /// Only events of this type
        #[arg(long = "type", value_parser = EVENT_TYPES)]
        event_type: Option<String>,
        /// Only events about this user (UUID)
        #[arg(long = "user")]
        user_id: Option<String>,
        /// Events per page, 1 to 100 (default 50)
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: Option<u32>,
        /// Continue from the cursor printed by the previous page
        #[arg(long)]
        cursor: Option<String>,
    },
}

pub async fn users(args: &AuthArgs, action: &UsersAdminAction) -> Result<()> {
    let target = target(args).await?;
    users_with(args, &target, action).await
}

pub async fn events(args: &AuthArgs, action: &EventsAction) -> Result<()> {
    let target = target(args).await?;
    events_with(args, &target, action).await
}

fn session_line(s: &Value) -> String {
    let end = match s["ended_at"] {
        Value::Null => format!("idle expiry {}", text(s, "idle_expires_at")),
        _ => format!("ended {} ({})", text(s, "ended_at"), text(s, "end_reason")),
    };
    format!("{}  {}  {}  created {}  {}", text(s, "id"), text(s, "status"), text(s, "auth_method"), text(s, "created_at"), end)
}

fn event_line(e: &Value) -> String {
    let detail = match &e["detail"] {
        Value::Object(map) if !map.is_empty() => map.iter().map(|(k, v)| format!("{k}: {}", v.as_str().map_or_else(|| v.to_string(), str::to_string))).collect::<Vec<_>>().join(", "),
        _ => String::new(),
    };
    format!("{}  {}  user {}  actor {}  {}", text(e, "occurred_at"), text(e, "type"), text(e, "user_id"), text(e, "actor"), detail).trim_end().to_string()
}

/// Writes `value` (pretty JSON) to a new file with mode 0600; never replaces a file.
fn write_new_private_file(path: &Path, value: &Value) -> Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).with_context(|| format!("create {}", path.display()))?;
    file.write_all(serde_json::to_string_pretty(value)?.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

fn valid_cursor(cursor: &str) -> bool {
    (1..=512).contains(&cursor.len()) && cursor.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

pub(super) async fn users_with(args: &AuthArgs, target: &Target, action: &UsersAdminAction) -> Result<()> {
    match action {
        UsersAdminAction::Delete { user_id, yes } => {
            let user_id = checked_id(user_id)?;
            confirm_yes(*yes, &format!("Delete user {user_id}? They are signed out everywhere and their authentication records are removed."))?;
            let result = target.call(Method::DELETE, &format!("users/{user_id}"), &[], None).await?;
            print(args, &result, |r| {
                if r["purge"].as_str() == Some("requires_v2") {
                    vec![format!("User {user_id} is banned and signed out. Full removal needs this environment's upgrade to finish.")]
                } else if r["purged"] == false {
                    vec![format!("Deletion of {user_id} is pending: some records remain. Run the same command again to continue.")]
                } else {
                    vec![format!("User {user_id} is deleted. Delete or anonymize rows your application keyed by this user ID yourself.")]
                }
            })
        }
        UsersAdminAction::Sessions { user_id } => {
            let mut sessions = target.call(Method::GET, &format!("users/{}/sessions", checked_id(user_id)?), &[], None).await?;
            sanitize(&mut sessions, false);
            print(args, &sessions, |s| {
                let lines = s["sessions"].as_array().into_iter().flatten().map(session_line).collect::<Vec<_>>();
                if lines.is_empty() {
                    vec!["No sessions.".into()]
                } else {
                    lines
                }
            })
        }
        UsersAdminAction::Export { user_id, output } => {
            let user_id = checked_id(user_id)?;
            // Refuse before the request: the export is recorded in the audit trail.
            match output {
                Some(path) if path.exists() => return Err(usage(format!("{} exists; --output never replaces a file", path.display()))),
                None if !args.json => return Err(usage("the export contains personal data: pass --output <file> (a new file, mode 0600) or --json to print it")),
                _ => {}
            }
            let mut document = target.call(Method::POST, &format!("users/{user_id}/export"), &[], Some(&json!({}))).await?;
            // Never secrets; addresses stay (the export is the user's data).
            sanitize(&mut document, true);
            match output {
                Some(path) => {
                    write_new_private_file(path, &document)?;
                    let summary = json!({"user_id": user_id, "output": path.display().to_string(), "events_truncated": document["events_truncated"]});
                    print(args, &summary, |_| {
                        let mut lines = vec![format!("The export of {user_id} was written to {} (mode 0600). It contains personal data; store it accordingly.", path.display())];
                        if document["events_truncated"] == true {
                            lines.push("The oldest events were left out to keep the export under 1 MiB.".into());
                        }
                        lines
                    })
                }
                None => print(args, &document, |_| vec![]),
            }
        }
        UsersAdminAction::SendVerification { user_id } => {
            let user_id = checked_id(user_id)?;
            let sent = match target.call(Method::POST, &format!("users/{user_id}/verification"), &[], Some(&json!({}))).await {
                Err(error) if error.downcast_ref::<Exit>().is_some_and(|e| e.code == 4) => {
                    return Err(Exit { code: 4, message: "AXAUTH_CONFLICT: the address is already verified, or the user cannot receive a verification email (not an active email and password user, or the address is suppressed)".into() }.into());
                }
                other => other?,
            };
            print(args, &sent, |_| vec![format!("A verification email was sent to user {user_id}.")])
        }
        UsersAdminAction::RemoveFactor { user_id, factor_id, yes } => {
            let (user_id, factor_id) = (checked_id(user_id)?, checked_id(factor_id)?);
            confirm_yes(*yes, &format!("Remove factor {factor_id} of user {user_id}? They can no longer use it."))?;
            let removed = target.call(Method::DELETE, &format!("users/{user_id}/factors/{factor_id}"), &[], None).await?;
            print(args, &removed, |_| vec![format!("Factor {factor_id} removed. The user's sessions continue.")])
        }
    }
}

pub(super) async fn events_with(args: &AuthArgs, target: &Target, action: &EventsAction) -> Result<()> {
    match action {
        EventsAction::List { event_type, user_id, limit, cursor } => {
            let mut query = vec![];
            if let Some(limit) = limit {
                query.push(("limit", limit.to_string()));
            }
            if let Some(cursor) = cursor {
                if !valid_cursor(cursor) {
                    return Err(usage("--cursor is not a cursor printed by `axiom auth events list`"));
                }
                query.push(("cursor", cursor.clone()));
            }
            if let Some(user_id) = user_id {
                query.push(("user_id", checked_id(user_id)?.to_string()));
            }
            if let Some(event_type) = event_type {
                query.push(("type", event_type.clone()));
            }
            let mut page = target.call(Method::GET, "events", &query, None).await?;
            sanitize(&mut page, false);
            print(args, &page, |p| {
                let mut lines = p["events"].as_array().into_iter().flatten().map(event_line).collect::<Vec<_>>();
                if lines.is_empty() {
                    lines.push("No events.".into());
                }
                if let Some(next) = p["next_cursor"].as_str() {
                    lines.push(format!("More: --cursor {next}"));
                }
                lines
            })
        }
    }
}

#[cfg(test)]
mod tests;
