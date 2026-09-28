//! Consume browser-built development applications through the maintained local hosts.
use anyhow::{bail, Context, Result};
use axiom_lib::{playground_application::Application, ui_contract::*};
use std::{collections::BTreeMap, path::Path};

fn prepare(
    application: &Application,
    root: &Path,
    base_url: &str,
    target: &str,
) -> Result<(std::path::PathBuf, std::path::PathBuf)> {
    let source = std::str::from_utf8(&application.files["source/frontend.acore"])?;
    let original_lock: UiContractLock =
        serde_json::from_slice(&application.files["contracts/lock.json"])?;
    if original_lock.contracts.len() != 1 || !original_lock.contracts.contains_key("tasks") {
        bail!("Playground export requires its local tasks contract");
    }
    // Match browser restrictions before involving any filesystem-backed compiler path.
    let compiled =
        axiom_ui::compile_ui_source_with_lock(source, super::parse_target(target)?, &original_lock);
    if !compiled.is_valid() {
        bail!(
            "Playground source does not compile with this CLI: {:?}",
            compiled.diagnostics
        );
    }
    let ir = compiled.ir.context("Missing UI model")?;
    if !ir.assets.is_empty() || !ir.extension_imports.is_empty() {
        bail!("External assets and extensions are unavailable in portable playground exports");
    }
    let config: axiom_lib::config::AxiomConfig =
        serde_json::from_slice(&application.files["backend/config.json"])?;
    let artifact = root.join("backend.axiom");
    axiom_build::core::build::build_evaluated_config(config, "default", None, &artifact)?;
    let manifest = UiApplicationManifest {
        format: UI_MANIFEST_FORMAT.into(),
        contracts: BTreeMap::from([(
            "tasks".into(),
            UiContractReference {
                artifact: "backend.axiom".into(),
                signature: None,
                public_key: None,
                base_url: base_url.into(),
                audience: "mobile".into(),
                operations: original_lock.contracts["tasks"]
                    .operations
                    .iter()
                    .map(|o| o.name.clone())
                    .collect(),
            },
        )]),
    };
    let manifest_path = root.join("AxiomContracts.toml");
    std::fs::write(&manifest_path, toml::to_string_pretty(&manifest)?)?;
    let lock = resolve_manifest(&manifest_path)?;
    let lock_path = root.join("axiom.ui.lock.json");
    write_lock(&lock_path, &lock)?;
    let source_path = root.join("main.acore");
    std::fs::write(&source_path, source)?;
    Ok((source_path, lock_path))
}

pub(super) async fn run(application: &Application, target: &str, launch: bool) -> Result<()> {
    let temp = tempfile::tempdir().context("Create isolated application session")?;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let base_url = format!("http://{}", listener.local_addr()?);
    let (source, lock) = prepare(application, temp.path(), &base_url, target)?;
    if target != "web" && !launch {
        super::super::ui::handle_run(source, lock, target.into(), true, false).await?;
        println!("Verified portable {target} application. Pass --launch to open its local emulator/simulator and mock backend.");
        return Ok(());
    }
    let config: axiom_mock::models::AxiomConfig =
        serde_json::from_slice(&application.files["backend/config.json"])?;
    println!("Running portable development application with a local mock backend at {base_url}.");
    // Both futures are owned by this session; errors or Ctrl-C close the mock listener too.
    tokio::select! {
        result = axiom_mock::server::serve_config_on_listener(config, listener, false, None) => result.map_err(|error| anyhow::anyhow!("Mock backend stopped: {error}")),
        result = super::super::ui::handle_run(source, lock, target.into(), false, launch) => result,
    }
}
