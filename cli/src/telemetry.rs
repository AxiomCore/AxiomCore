use crate::access_config::AccessConfig;
use axiom_cloud::CliApi;
use std::time::Duration;
use tokio::time::timeout;

pub struct Telemetry;

impl Telemetry {
    pub async fn track(
        config: &AccessConfig,
        command: &str,
        duration: Duration,
        success: bool,
        error: Option<&str>,
    ) {
        let os_info = os_info::get();

        if telemetry_disabled() {
            return;
        }

        let payload = serde_json::json!({
            "machine_id": config.machine_id,
            "command": command,
            // `args` is retained for wire compatibility with the alpha
            // endpoint, but it now holds a small schema version only. Never
            // send file paths, URLs, credentials, query parameters, or user
            // supplied contract/project names as adoption telemetry.
            "args": "{\"schema_version\":1}",
            "duration": duration.as_millis() as u64,
            "success": success,
            "error": error.map(error_category).unwrap_or(""),
            "os": os_info.os_type().to_string(),
            "version": env!("CARGO_PKG_VERSION")
        });

        // Spawn a background task, but we must await it with a timeout in main
        // otherwise the process might exit before the network request finishes.
        // For CLI tools, it's common to block for a few hundred ms at exit.

        let result = timeout(Duration::from_millis(1500), CliApi::send_telemetry(payload)).await;

        match result {
            Ok(Err(e)) => {
                if e.to_string().contains("ACCESS_REVOKED") {
                    let _ = AccessConfig::wipe().await;
                    eprintln!("\n\n❌ \x1b[1;31mYOUR ACCESS HAS BEEN REVOKED BY THE ADMINISTRATOR.\x1b[0m");
                    eprintln!("Referral code: {}", config.referral_code);
                    eprintln!("If you believe this is an error, contact support.\n");
                    std::process::exit(1);
                }
            }
            _ => {} // Ignore timeouts or success
        }
    }
}

fn telemetry_disabled() -> bool {
    matches!(
        std::env::var("AXIOM_TELEMETRY")
            .ok()
            .as_deref()
            .map(str::trim),
        Some("0" | "false" | "FALSE" | "off" | "OFF")
    )
}

fn error_category(error: &str) -> &'static str {
    let normalized = error.to_ascii_lowercase();
    if normalized.contains("not logged in")
        || normalized.contains("authentication")
        || normalized.contains("access")
    {
        "authentication"
    } else if normalized.contains("network")
        || normalized.contains("connection")
        || normalized.contains("timeout")
    {
        "network"
    } else if normalized.contains("not found")
        || normalized.contains("missing")
        || normalized.contains("invalid")
    {
        "input"
    } else if normalized.contains("build")
        || normalized.contains("compile")
        || normalized.contains("artifact")
    {
        "build"
    } else {
        "other"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telemetry_never_uses_the_raw_error_as_an_attribute() {
        assert_eq!(
            error_category("missing axiom.acore at /private/project"),
            "input"
        );
        assert_eq!(
            error_category("connection timeout to a collector"),
            "network"
        );
    }
}
