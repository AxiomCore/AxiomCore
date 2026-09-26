use anyhow::{anyhow, Result};
use axiom_extractor::evaluate_acore_config;
use std::path::PathBuf;

/// Run the same deterministic security analyzer used by `axiom build` without
/// writing an artifact. This is intentionally useful in local review and CI.
pub async fn handle_check(file: PathBuf, json: bool, fail_on_warning: bool) -> Result<()> {
    let config = evaluate_acore_config(&file.to_string_lossy(), None)?;
    config.validate_security()?;
    let report = match config.compile_security_manifest()? {
        Some(manifest) => manifest,
        None => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "enabled": false,
                        "message": "Security Mode is not declared for this contract."
                    })
                );
            } else {
                println!(
                    "ℹ️  Security Mode is not enabled. Add a `security {{ ... }}` block to opt in."
                );
            }
            return Ok(());
        }
    };
    let errors = report
        .findings
        .iter()
        .filter(|finding| finding.severity == "error")
        .count();
    let warnings = report
        .findings
        .iter()
        .filter(|finding| finding.severity == "warning")
        .count();
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "Security Mode {} · {} error(s), {} warning(s) · {} of {} endpoints covered ({} direct, {} default-policy)",
            report.mode,
            errors,
            warnings,
            report.coverage.covered_endpoint_count,
            report.coverage.endpoint_count,
            report.coverage.explicit_endpoint_count,
            report.coverage.default_policy_endpoint_count
        );
        for finding in &report.findings {
            println!(
                "{} {} [{}] {}\n  → {}",
                finding.rule,
                finding.severity.to_uppercase(),
                finding.target,
                finding.message,
                finding.remediation
            );
        }
    }
    // Audit is deliberately non-blocking: it gives a team a complete,
    // reviewable baseline before they opt into release gates. Strict matches
    // `axiom build` and fails on error findings. CI may additionally elect to
    // fail audit warnings with `--fail-on-warning`.
    if (report.mode == "strict" && errors > 0) || (fail_on_warning && warnings > 0) {
        return Err(anyhow!(
            "security check found {} error(s) and {} warning(s)",
            errors,
            warnings
        ));
    }
    Ok(())
}
