//! Private-alpha access for cloud commands (W7 J1).
//!
//! Cloud accounts exist only by invitation, so a signed-in Cloud account is
//! enough: the machine registers with the account's device-flow session and
//! no referral code is asked for. A referral code stays the way in for people
//! without an account (the waitlist path). Without a terminal the CLI never
//! prompts; it says what to do instead.
use crate::access_config::AccessConfig;
use anyhow::Result;
use console::style;
use dialoguer::{theme::ColorfulTheme, Input};
use std::io::IsTerminal;

/// What the gate does for a machine that has no access record yet.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    /// `axiom login` runs first; the machine registers with the new session.
    LoginFirst,
    /// A saved Cloud session exists: register with it.
    Account,
    /// Register with the code from `AXIOM_REFERRAL_CODE`.
    EnvironmentCode(String),
    /// Ask for a code at the terminal.
    Prompt,
    /// No terminal and nothing to register with: explain and stop.
    Explain,
}

pub fn plan(
    is_login: bool,
    has_session: bool,
    environment_code: Option<String>,
    interactive: bool,
) -> Step {
    if is_login {
        return Step::LoginFirst;
    }
    if has_session {
        return Step::Account;
    }
    match environment_code.filter(|code| !code.trim().is_empty()) {
        Some(code) => Step::EnvironmentCode(code.trim().to_string()),
        None if interactive => Step::Prompt,
        None => Step::Explain,
    }
}

/// The message for a machine with no terminal and no way to register.
pub fn no_access_message() -> String {
    [
        "This command needs Axiom Cloud access, and this machine is not registered yet.",
        "  Invited or existing Axiom Cloud account: run `axiom login` once (it opens the device sign-in).",
        "  Referral code: set AXIOM_REFERRAL_CODE and run the command again.",
        "  No account yet: join the waitlist with `axiom join <EMAIL>`.",
    ]
    .join("\n")
}

/// Outcome of a registration with a Cloud session.
#[derive(Debug, PartialEq, Eq)]
pub enum AccountRegistration {
    Registered,
    /// The session is expired or not a CLI sign-in: sign in again.
    SessionRejected,
    /// The control plane refused for another reason (for example a backend
    /// that predates account registration); the text is safe to show.
    Refused(String),
}

/// POST /api/v1/cli/register with the session as Bearer and no code.
pub async fn register_account(
    base_url: &str,
    access_token: &str,
    machine_id: &str,
) -> Result<AccountRegistration> {
    let user_agent = format!(
        "axiom-cli/{} ({})",
        env!("CARGO_PKG_VERSION"),
        os_info::get()
    );
    let response = reqwest::Client::new()
        .post(format!(
            "{}/api/v1/cli/register",
            base_url.trim_end_matches('/')
        ))
        .bearer_auth(access_token)
        .json(&serde_json::json!({ "machine_id": machine_id, "user_agent": user_agent }))
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await?;
    let status = response.status();
    if status.is_success() {
        return Ok(AccountRegistration::Registered);
    }
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Ok(AccountRegistration::SessionRejected);
    }
    let text: String = response
        .text()
        .await
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect();
    Ok(AccountRegistration::Refused(format!(
        "{status}: {}",
        text.trim()
    )))
}

fn saved_access_token() -> Option<String> {
    crate::auth_store::load_auth_data_if_present()
        .ok()
        .flatten()
        .map(|data| data.access_token)
        .filter(|token| !token.trim().is_empty())
}

/// Register this machine with the saved Cloud session. `Ok(None)` means it
/// could not (the reason is printed); the caller decides what comes next.
pub async fn register_with_session() -> Result<Option<AccessConfig>> {
    let Some(token) = saved_access_token() else {
        return Ok(None);
    };
    let config = AccessConfig::save("").await?;
    let base_url = axiom_cloud::cloud_base_url()?;
    let outcome = register_account(&base_url, &token, &config.machine_id).await;
    match outcome {
        Ok(AccountRegistration::Registered) => Ok(Some(config)),
        Ok(AccountRegistration::SessionRejected) => {
            let _ = AccessConfig::wipe().await;
            eprintln!(
                "Your Axiom Cloud session has expired or is not a CLI sign-in. Run `axiom login` again."
            );
            Ok(None)
        }
        Ok(AccountRegistration::Refused(reason)) => {
            let _ = AccessConfig::wipe().await;
            eprintln!("Axiom Cloud did not register this machine for your account ({reason}).");
            Ok(None)
        }
        Err(error) => {
            let _ = AccessConfig::wipe().await;
            eprintln!("Could not reach Axiom Cloud to register this machine: {error}");
            Ok(None)
        }
    }
}

/// Register with a referral code (the waitlist path); exits on refusal as
/// the gate always has.
async fn register_with_code(code: &str) -> Result<AccessConfig> {
    let config = AccessConfig::save(code).await?;
    println!("{}", style("Verifying code...").dim());
    match axiom_cloud::CliApi::register(&config.referral_code, &config.machine_id).await {
        Ok(()) => {
            println!(
                "✅ {}",
                style("Access Granted. Welcome to Axiom.").green().bold()
            );
            println!();
            Ok(config)
        }
        Err(error) => {
            // Registration failed, wipe local config so they try again next time
            let _ = AccessConfig::wipe().await;
            println!(
                "\n❌ {}",
                style(format!("Authorization Failed: {}", error)).red()
            );
            std::process::exit(1);
        }
    }
}

/// The gate for a machine without an access record. `Ok(None)` lets
/// `axiom login` run first ([`after_login`] registers it).
pub async fn admit(is_login: bool) -> Result<Option<AccessConfig>> {
    let interactive = std::env::var_os("CI").is_none() && std::io::stdin().is_terminal();
    let step = plan(
        is_login,
        saved_access_token().is_some(),
        std::env::var("AXIOM_REFERRAL_CODE").ok(),
        interactive,
    );
    let step = match step {
        Step::LoginFirst => return Ok(None),
        Step::Account => match register_with_session().await? {
            Some(config) => return Ok(Some(config)),
            // The session did not work: fall back to a code.
            None => plan(
                false,
                false,
                std::env::var("AXIOM_REFERRAL_CODE").ok(),
                interactive,
            ),
        },
        other => other,
    };
    match step {
        Step::EnvironmentCode(code) => {
            println!("Using referral code from environment variable.");
            register_with_code(&code).await.map(Some)
        }
        Step::Prompt => {
            println!(
                "{}",
                style("🔒 Axiom CLI is currently in Private Alpha.")
                    .bold()
                    .yellow()
            );
            println!(
                "Invited or existing Axiom Cloud account: press Ctrl-C and run {} instead.",
                style("axiom login").cyan()
            );
            println!(
                "Otherwise enter a referral code. If you don't have one, run: {}\n",
                style("axiom join <EMAIL>").cyan()
            );
            let code: String = Input::with_theme(&ColorfulTheme::default())
                .with_prompt("Enter Referral Code")
                .interact_text()?;
            register_with_code(code.trim()).await.map(Some)
        }
        _ => {
            eprintln!("{}", no_access_message());
            std::process::exit(1);
        }
    }
}

/// After a successful `axiom login` on an unregistered machine: register it
/// with the new session. A refusal is reported but does not undo the login.
pub async fn after_login() -> Option<AccessConfig> {
    match register_with_session().await {
        Ok(Some(config)) => Some(config),
        Ok(None) => None,
        Err(error) => {
            eprintln!("Could not register this machine: {error}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{http::HeaderMap, http::StatusCode, routing::post, Json, Router};

    #[test]
    fn a_cloud_account_needs_no_code() {
        assert_eq!(plan(false, true, None, false), Step::Account);
        assert_eq!(plan(false, true, None, true), Step::Account);
        assert_eq!(plan(true, false, None, false), Step::LoginFirst);
    }

    #[test]
    fn without_an_account_a_code_is_still_required() {
        assert_eq!(
            plan(false, false, Some(" CODE ".into()), false),
            Step::EnvironmentCode("CODE".into())
        );
        assert_eq!(plan(false, false, None, true), Step::Prompt);
        assert_eq!(plan(false, false, Some("  ".into()), false), Step::Explain);
        assert_eq!(plan(false, false, None, false), Step::Explain);
    }

    #[test]
    fn the_non_terminal_message_names_every_next_step() {
        let message = no_access_message();
        for needle in ["axiom login", "AXIOM_REFERRAL_CODE", "axiom join <EMAIL>"] {
            assert!(message.contains(needle), "{needle} missing");
        }
        assert!(!message.contains("not a terminal"));
        assert!(
            !message.contains('\u{2014}'),
            "no em dashes in user-facing text"
        );
    }

    async fn serve(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{address}")
    }

    #[tokio::test]
    async fn registration_sends_the_session_and_no_code() {
        let router = Router::new().route(
            "/api/v1/cli/register",
            post(
                |headers: HeaderMap, Json(body): Json<serde_json::Value>| async move {
                    let bearer = headers
                        .get("authorization")
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_string();
                    if bearer != "Bearer good-session" {
                        return (
                            StatusCode::UNAUTHORIZED,
                            "Axiom Cloud session is invalid or expired. Run: axiom login"
                                .to_string(),
                        );
                    }
                    if body.get("code").is_some() || body["machine_id"] != "machine-1" {
                        return (StatusCode::BAD_REQUEST, "unexpected body".to_string());
                    }
                    (
                        StatusCode::OK,
                        r#"{"status":"registered","access":"account"}"#.to_string(),
                    )
                },
            ),
        );
        let base = serve(router).await;
        assert_eq!(
            register_account(&base, "good-session", "machine-1")
                .await
                .unwrap(),
            AccountRegistration::Registered
        );
        assert_eq!(
            register_account(&base, "stale-session", "machine-1")
                .await
                .unwrap(),
            AccountRegistration::SessionRejected
        );
    }

    #[tokio::test]
    async fn an_older_backend_refusal_is_reported_not_hidden() {
        let router = Router::new().route(
            "/api/v1/cli/register",
            post(|| async { (StatusCode::BAD_REQUEST, "missing code or machine_id\n") }),
        );
        let base = serve(router).await;
        assert_eq!(
            register_account(&base, "good-session", "machine-1")
                .await
                .unwrap(),
            AccountRegistration::Refused("400 Bad Request: missing code or machine_id".into())
        );
    }
}
