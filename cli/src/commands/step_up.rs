//! `axiom login --step-up` (recovery/v1, Authorization): a short device-flow
//! token with scope `resources:recovery`, no `secrets:development`, a
//! 15-minute lifetime and no refresh token. Restore, archive and
//! retire_previous run with it. The token is kept in its own private file
//! next to the CLI session (same profile directory, mode 0600) so the
//! ordinary session is never replaced; it is never printed.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const STEP_UP_SCOPE: &str = "resources:recovery";
/// The contract's step-up lifetime. A longer `expires_in` is clamped.
pub const MAX_LIFETIME_SECONDS: u64 = 15 * 60;
/// A token this close to expiry is not used: the action could outlive it.
const EXPIRY_MARGIN_SECONDS: u64 = 30;
const STEP_UP_FILE: &str = "step-up.json";

#[derive(Serialize, Deserialize)]
struct StoredStepUp {
    access_token: String,
    expires_at: u64,
    scope: String,
}

impl std::fmt::Debug for StoredStepUp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StoredStepUp([redacted])")
    }
}

/// Where the step-up token lives. Production uses the active CLI profile;
/// tests use a temporary directory.
#[derive(Clone, Debug)]
pub struct StepUpStore {
    path: PathBuf,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl StepUpStore {
    pub fn for_profile() -> Result<Self> {
        let mut path = crate::auth_store::get_config_dir()?;
        path.push(STEP_UP_FILE);
        Ok(Self { path })
    }

    /// Forgets the profile's step-up token. Every new sign-in calls it, so a
    /// step-up token never outlives an account switch (W6-F L-20).
    pub fn forget_for_profile() {
        if let Ok(store) = Self::for_profile() {
            store.clear();
        }
    }

    #[cfg(test)]
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The stored token while it has more than the margin left; otherwise
    /// `None` (and an expired file is removed).
    pub fn load(&self) -> Option<String> {
        let content = std::fs::read(&self.path).ok()?;
        let stored: StoredStepUp = match serde_json::from_slice(&content) {
            Ok(stored) => stored,
            Err(_) => {
                self.clear();
                return None;
            }
        };
        if stored.scope != STEP_UP_SCOPE
            || stored.access_token.trim().is_empty()
            || stored.expires_at <= now().saturating_add(EXPIRY_MARGIN_SECONDS)
        {
            self.clear();
            return None;
        }
        Some(stored.access_token)
    }

    pub fn save(&self, access_token: &str, expires_in: u64) -> Result<()> {
        let stored = StoredStepUp {
            access_token: access_token.to_string(),
            expires_at: now().saturating_add(expires_in.min(MAX_LIFETIME_SECONDS)),
            scope: STEP_UP_SCOPE.to_string(),
        };
        crate::auth_store::write_private_file(&self.path, &serde_json::to_vec(&stored)?)
    }

    pub fn clear(&self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[derive(Deserialize)]
struct DeviceStart {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    interval: Option<u64>,
    #[serde(default)]
    expires_in: Option<u64>,
}

#[derive(Deserialize)]
struct StepUpToken {
    access_token: String,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
}

#[derive(Deserialize)]
struct PollError {
    error: String,
}

fn http() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .build()?)
}

/// Run the step-up device flow against `base_url` and store the token.
/// Instructions go to stderr so `--json` output stays parseable.
/// `poll_floor` is the minimum poll interval (tests use zero).
pub async fn step_up(base_url: &str, store: &StepUpStore, poll_floor: Duration) -> Result<()> {
    let client = http()?;
    let base = base_url.trim_end_matches('/');
    let response = client
        .post(format!("{base}/auth/device/start"))
        .json(&serde_json::json!({ "scope": STEP_UP_SCOPE }))
        .send()
        .await
        .context("Could not contact AxiomCore authentication. Check your connection and retry")?;
    if !response.status().is_success() {
        bail!(
            "Could not start the step-up sign-in (HTTP {}). Retry in a minute.",
            response.status().as_u16()
        );
    }
    let start: DeviceStart = response
        .json()
        .await
        .context("AxiomCore authentication returned an invalid device-login response")?;

    eprintln!("This action needs a recent sign-in (step-up, valid 15 minutes).");
    eprintln!(
        "Open {} in a browser where you signed in to AxiomCore within the last 15 minutes",
        start.verification_uri
    );
    eprintln!(
        "(sign in again there if it was longer ago) and enter code: {}",
        start.user_code
    );

    let mut interval = Duration::from_secs(start.interval.unwrap_or(5)).max(poll_floor);
    let deadline = tokio::time::Instant::now()
        + Duration::from_secs(start.expires_in.unwrap_or(600).clamp(60, 900));
    loop {
        if tokio::time::Instant::now() >= deadline {
            bail!("The step-up code expired. Run `axiom login --step-up` again.");
        }
        tokio::time::sleep(interval).await;
        let response = client
            .post(format!("{base}/auth/device/poll"))
            .json(&serde_json::json!({ "device_code": start.device_code }))
            .send()
            .await
            .context("Could not contact AxiomCore authentication while waiting for approval")?;
        let status = response.status();
        let body = response
            .bytes()
            .await
            .context("Could not read the authentication response")?;
        if let Ok(error) = serde_json::from_slice::<PollError>(&body) {
            match error.error.as_str() {
                "authorization_pending" => continue,
                "slow_down" => {
                    interval += Duration::from_secs(5);
                    continue;
                }
                "expired_token" => {
                    bail!("The step-up code expired. Run `axiom login --step-up` again.")
                }
                "access_denied" => bail!("The step-up sign-in was declined."),
                // The server's own words are not echoed; only known codes are.
                _ => bail!(
                    "AxiomCore refused the step-up sign-in. Sign in again in the browser \
                     (within 15 minutes) and retry `axiom login --step-up`."
                ),
            }
        }
        if !status.is_success() {
            bail!(
                "AxiomCore refused the step-up sign-in (HTTP {}). Sign in again in the browser \
                 and retry `axiom login --step-up`.",
                status.as_u16()
            );
        }
        let token: StepUpToken = serde_json::from_slice(&body)
            .context("AxiomCore authentication returned an invalid token response")?;
        // A refresh token, or another scope, means the server issued an
        // ordinary session: never store it as a step-up token.
        if token
            .refresh_token
            .as_deref()
            .is_some_and(|value| !value.is_empty())
            || token
                .scope
                .as_deref()
                .is_some_and(|scope| scope != STEP_UP_SCOPE)
            || token.access_token.trim().is_empty()
        {
            bail!(
                "AxiomCore did not return a step-up token (scope {STEP_UP_SCOPE}, no refresh). \
                 Nothing was stored."
            );
        }
        store.save(
            &token.access_token,
            token.expires_in.unwrap_or(MAX_LIFETIME_SECONDS),
        )?;
        // W6-F L-17: the server's expires_in is the true lifetime (it ends
        // 15 minutes after the browser sign-in that approved it).
        let minutes = token
            .expires_in
            .unwrap_or(MAX_LIFETIME_SECONDS)
            .min(MAX_LIFETIME_SECONDS)
            .div_ceil(60);
        eprintln!(
            "Step-up sign-in complete. It is valid for about {minutes} minutes and is not refreshed."
        );
        return Ok(());
    }
}

/// `axiom login --step-up`.
pub async fn handle_login_step_up() -> Result<()> {
    let base = axiom_cloud::cloud_base_url()?;
    step_up(&base, &StepUpStore::for_profile()?, Duration::from_secs(1)).await
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    #[test]
    fn login_accepts_step_up() {
        let cli = crate::Cli::try_parse_from(["axiom", "login", "--step-up"]).unwrap();
        assert!(matches!(
            *cli.command,
            crate::Commands::Login { step_up: true }
        ));
        let cli = crate::Cli::try_parse_from(["axiom", "login"]).unwrap();
        assert!(matches!(
            *cli.command,
            crate::Commands::Login { step_up: false }
        ));
    }
}
