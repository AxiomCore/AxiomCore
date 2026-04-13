use crate::access_config::AccessConfig;
use axiom_cloud::CliApi;
use std::time::Duration;
use tokio::time::timeout;

pub struct Telemetry;

impl Telemetry {
    pub async fn track(
        config: &AccessConfig,
        command: &str,
        args: Vec<String>,
        duration: Duration,
        success: bool,
        error_msg: Option<String>,
    ) {
        let os_info = os_info::get();

        let payload = serde_json::json!({
            "machine_id": config.machine_id,
            "command": command,
            "args": serde_json::to_string(&args).unwrap_or_default(),
            "duration": duration.as_millis() as u64,
            "success": success,
            "error": error_msg.unwrap_or_default(),
            "os": os_info.to_string(),
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
