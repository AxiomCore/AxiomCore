use acore::evaluator::Evaluator;
use acore::security::SecurityManager;
use acore::value::Value;
use anyhow::{anyhow, Result};
use axiom_test_runner::models::TestConfig;
use axiom_test_runner::runner::{EndpointInfo, Runner};
use console::{style, Emoji};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

static CHECK_MARK: Emoji<'_, '_> = Emoji("✔", "v");
static CROSS_MARK: Emoji<'_, '_> = Emoji("✘", "x");

pub async fn handle_test(file: Option<PathBuf>, tag: Option<String>) -> Result<()> {
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

    let mut evaluator = Evaluator::new(SecurityManager::allow_all());
    evaluator.is_axiom_project = true;

    let abs_path = file_path.canonicalize()?;
    let module_uri = format!("file://{}", abs_path.display());

    let val = evaluator
        .evaluate_module(&module_uri)
        .map_err(|e| anyhow!("Evaluation failed: {}", e))?;

    // Resolve all properties including inherited ones (via 'amends')
    let mut all_props = HashMap::new();
    acore::render::collect_all_properties(&val, &mut all_props);

    // 1. Extract Endpoints
    let mut endpoints = HashMap::new();
    if let Some(ep_thunk) = all_props.get("endpoints") {
        if let Ok(eps_val) = evaluator.evaluate_thunk_in_context(ep_thunk, Some(val.clone())) {
            // FIX: Deep-merge all entries from parent schemas (inherited via amends)
            let mut ep_entries = HashMap::new();
            acore::render::collect_all_entries(&eps_val, &mut ep_entries);

            for (k_str, v_thunk) in ep_entries {
                if let Ok(ep_obj) = evaluator.evaluate_thunk_in_context(&v_thunk, Some(val.clone()))
                {
                    let mut ep_props = HashMap::new();
                    acore::render::collect_all_properties(&ep_obj, &mut ep_props);

                    let path = if let Some(p_thunk) = ep_props.get("path") {
                        if let Ok(acore::value::Value::String(p)) =
                            evaluator.evaluate_thunk_in_context(p_thunk, Some(ep_obj.clone()))
                        {
                            p
                        } else {
                            "".to_string()
                        }
                    } else {
                        "".to_string()
                    };

                    let method = if let Some(m_thunk) = ep_props.get("method") {
                        if let Ok(acore::value::Value::String(m)) =
                            evaluator.evaluate_thunk_in_context(m_thunk, Some(ep_obj.clone()))
                        {
                            m
                        } else {
                            "GET".to_string()
                        }
                    } else {
                        "GET".to_string()
                    };

                    endpoints.insert(k_str, EndpointInfo { path, method });
                }
            }
        }
    }

    // 2. Extract TestConfig
    let tc_thunk = all_props.get("testConfig").ok_or_else(|| anyhow!("No 'testConfig' property found in the Acore file. Define 'testConfig = new TestConfig {{ ... }}' to run tests."))?;
    let tc_val = evaluator
        .evaluate_thunk_in_context(tc_thunk, Some(val.clone()))
        .map_err(|e| anyhow!("Failed to evaluate testConfig: {}", e))?;
    // 3. Render only the TestConfig to JSON
    let json_payload =
        acore::render::render_value(&mut evaluator, &tc_val, acore::render::OutputFormat::Json)
            .map_err(|e| anyhow!("Failed to render test config to JSON: {}", e))?;

    let mut test_config: TestConfig = serde_json::from_str(&json_payload).map_err(|e| {
        anyhow!(
            "Failed to parse test configuration: {}\nPayload: {}",
            e,
            json_payload
        )
    })?;

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
