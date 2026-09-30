use anyhow::{anyhow, Context, Result};
use axiom_test_runner::models::TestConfig;
use axiom_test_runner::runner::{EndpointInfo, Runner};
use console::{style, Emoji};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

static CHECK_MARK: Emoji<'_, '_> = Emoji("✔", "v");
static CROSS_MARK: Emoji<'_, '_> = Emoji("✘", "x");

fn load_test_configuration(
    source: &Path,
    variant: Option<&str>,
) -> Result<(HashMap<String, EndpointInfo>, TestConfig)> {
    let mut evaluator =
        acore::evaluator::Evaluator::new(acore::security::SecurityManager::allow_all());
    evaluator.is_axiom_project = true;
    evaluator.active_variant = variant.map(str::to_string);
    let uri = format!("file://{}", source.canonicalize()?.display());
    let value = evaluator.evaluate_module(&uri)?;
    // An explicitly empty testConfig is a valid zero-suite run. Keep it in
    // this internal JSON view instead of pruning the entire empty object.
    evaluator.is_axiom_project = false;
    let json =
        acore::render::render_value(&mut evaluator, &value, acore::render::OutputFormat::Json)?;
    let config: serde_json::Value = serde_json::from_str(&json)?;
    let mut endpoints = HashMap::new();
    if let Some(entries) = config
        .get("endpoints")
        .and_then(serde_json::Value::as_object)
    {
        for (name, endpoint) in entries {
            let path = endpoint
                .get("path")
                .and_then(serde_json::Value::as_str)
                .with_context(|| format!("Endpoint '{name}' has no string path"))?;
            let method = endpoint
                .get("method")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("GET");
            endpoints.insert(
                name.clone(),
                EndpointInfo {
                    path: path.into(),
                    method: method.into(),
                },
            );
        }
    }
    let test_config = config.get("testConfig").filter(|value| !value.is_null())
        .context("No test configuration found. In a backend profile, declare testConfig(suites: [TestSuite(name: \"smoke\", baseUrl: \"http://localhost:8080\", checks: [...])]).")?;
    let test_config = serde_json::from_value(test_config.clone())
        .context("Failed to parse test configuration")?;
    Ok((endpoints, test_config))
}

pub async fn handle_test(
    file: Option<PathBuf>,
    tag: Option<String>,
    variant: Option<String>,
) -> Result<()> {
    let file_path = file.unwrap_or_else(|| PathBuf::from("axiom.acore"));

    if !file_path.exists() {
        return Err(anyhow!(
            "Test file not found at {}. Pass a valid path or run in an Axiom project directory.",
            file_path.display()
        ));
    }

    println!(
        "{}",
        style(format!("🧪 Running Tests from {}", file_path.display()))
            .cyan()
            .bold()
    );

    let (endpoints, mut test_config) = load_test_configuration(&file_path, variant.as_deref())?;

    // Apply Tag Filtering
    if let Some(target_tag) = tag {
        test_config.suites.retain(|s| s.tags.contains(&target_tag));
        if test_config.suites.is_empty() {
            println!(
                "{}",
                style(format!("No suites found matching tag '{}'", target_tag)).yellow()
            );
            return Ok(());
        }
    }

    // 4. Execute the Runner
    let mut runner = Runner::new(endpoints);
    let start_time = Instant::now();
    let result = runner.run(test_config).await?;
    let total_time = start_time.elapsed();

    // 5. Console Reporter
    println!("\n{}", style("Test Results:").bold().underlined());

    for suite in &result.suites {
        let suite_color = if suite.passed {
            console::Style::new().green()
        } else {
            console::Style::new().red()
        };
        println!(
            "\n{} {}",
            style("Suite:").bold(),
            suite_color.apply_to(&suite.name)
        );

        for check in &suite.checks {
            if check.passed {
                println!(
                    "  [{}] {} {}",
                    style(CHECK_MARK).green(),
                    check.label,
                    style(format!("({}ms)", check.duration_ms)).dim()
                );
            } else {
                println!(
                    "  [{}] {} {}",
                    style(CROSS_MARK).red(),
                    check.label,
                    style(format!("({}ms)", check.duration_ms)).dim()
                );

                if let Some(err) = &check.error_message {
                    println!("      ↳ {}", style(err).red().dim());
                }

                for failure in &check.assertion_failures {
                    println!("      ↳ {}", style(failure).red());
                }
            }
        }
    }

    println!("\n{}", style("─".repeat(40)).dim());
    let summary_color = if result.total_failed == 0 {
        console::Style::new().green()
    } else {
        console::Style::new().red()
    };

    println!(
        "{}",
        summary_color.apply_to(format!(
            "Tests: {} passed, {} failed",
            result.total_passed, result.total_failed
        ))
    );
    println!("Time:  {:.2}s", total_time.as_secs_f64());

    if result.total_failed > 0 {
        std::process::exit(1);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_test_endpoint_lookup_observes_variant_filtering() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("tests.acore");
        std::fs::write(
            &source,
            r#"
            variants { ["mobile"] { include = Listing { "getProject" } } }
            endpoints {
                ["getProject"] = EndpointDef { name = "getProject" method = "GET" path = "/projects/{id}" }
                ["deleteProject"] = EndpointDef { name = "deleteProject" method = "DELETE" path = "/projects/{id}" }
            }
            testConfig = TestConfig { suites = Listing {} }
        "#,
        )
        .unwrap();
        let (endpoints, tests) = load_test_configuration(&source, Some("mobile")).unwrap();
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints["getProject"].path, "/projects/{id}");
        assert_eq!(endpoints["getProject"].method, "GET");
        assert!(tests.suites.is_empty());
    }
}
