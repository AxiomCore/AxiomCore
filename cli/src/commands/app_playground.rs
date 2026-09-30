//! Consume browser-built development applications through the maintained local hosts.
use anyhow::{Context, Result, bail};
use axiom_lib::{playground_application::Application, ui_contract::*};
use std::path::Path;

fn prepare(
    application: &Application,
    root: &Path,
    base_url: &str,
    target: &str,
) -> Result<(std::path::PathBuf, std::path::PathBuf)> {
    let source = application
        .files
        .get("source/frontend/frontend.acore")
        .or_else(|| application.files.get("source/frontend.acore"))
        .context("Missing frontend source in playground application")?;
    let source = std::str::from_utf8(source)?;
    let original_lock: UiContractLock =
        serde_json::from_slice(&application.files["contracts/lock.json"])?;
    if original_lock.contracts.values().any(|contract| {
        contract.artifact != Path::new("backend.axiom")
            || contract.signature.is_some()
            || contract.public_key.is_some()
    }) {
        bail!("Playground exports require the local unsigned backend.axiom contract");
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
        contracts: original_lock
            .contracts
            .iter()
            .map(|(alias, contract)| {
                (
                    alias.clone(),
                    UiContractReference {
                        artifact: "backend.axiom".into(),
                        signature: None,
                        public_key: None,
                        base_url: base_url.into(),
                        audience: contract.audience.clone(),
                        operations: contract.operations.iter().map(|o| o.name.clone()).collect(),
                    },
                )
            })
            .collect(),
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
        println!(
            "Verified portable {target} application. Pass --launch to open its local emulator/simulator and mock backend."
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn exported_alias_audience_and_operation_selection_survive_native_preparation() {
        let source = r#"module typed.ui
use contract Work from "work" audience "web"
app Example { route "/" => Home }
page Home {
  query items = Work.list()
  view { if items.pending { Text("Loading") } else if items.error { Text("Failed") } else { Text("Tasks") } }
}
"#;
        let contract = json!({
            "project":{"id":"typed","version":"1.0.0"},
            "variants":{"default":{"include":["*"]}},
            "models":{"Task":{"name":"Task","fields":{"id":{"typeRef":{"kind":"string"},"isOptional":false}}}},
            "domain":{"entities":{"task":{"model":"Task","key":["id"]}},"projections":{"card":{"entity":"task","fields":["id"],"audience":"web"}}},
            "endpoints":{"list":{"id":1,"name":"list","method":"GET","path":"/tasks","responseType":{"kind":"list","itemType":{"kind":"named","value":"Task"}},"responseProjection":"card","errors":["unavailable"],"cacheIdentity":"tasks:list"}}
        });
        let lock = json!({"format":UI_LOCK_FORMAT,"contracts":{"work":{
            "artifact":"backend.axiom","artifactSha256":"a".repeat(64),"projectId":"typed","projectVersion":"1.0.0","minRuntimeVersion":1,
            "baseUrl":"http://localhost:8080","audience":"web","operations":[{
                "name":"list","endpointId":1,"method":"GET","path":"/tasks","kind":"query","responseProjection":"card",
                "requestFields":[],"responseFields":["id"],"audiences":["web"],"idempotent":true,"cacheIdentity":"tasks:list","invalidates":[],"errors":["unavailable"],"sensitiveFields":[]
            }]
        }}});
        let application = Application {
            manifest: axiom_lib::playground_application::Manifest {
                format: axiom_lib::playground_application::FORMAT.into(),
                application_id: "typed.ui".into(),
                mode: "development".into(),
                graph_revision: "test".into(),
                targets: vec!["web".into()],
                compiler: "test".into(),
            },
            files: BTreeMap::from([
                ("source/frontend.acore".into(), source.as_bytes().to_vec()),
                (
                    "contracts/lock.json".into(),
                    serde_json::to_vec(&lock).unwrap(),
                ),
                (
                    "backend/config.json".into(),
                    serde_json::to_vec(&contract).unwrap(),
                ),
            ]),
        };
        // Both the current split-project layout and existing downloads run.
        for path in ["source/frontend.acore", "source/frontend/frontend.acore"] {
            let mut application = application.clone();
            let source = application.files.remove("source/frontend.acore").unwrap();
            application.files.insert(path.into(), source);
            let root = tempfile::tempdir().unwrap();
            let (_, path) =
                prepare(&application, root.path(), "http://127.0.0.1:9876", "web").unwrap();
            let lock: UiContractLock =
                serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            assert_eq!(lock.contracts.len(), 1);
            assert_eq!(lock.contracts["work"].audience, "web");
            assert_eq!(lock.contracts["work"].base_url, "http://127.0.0.1:9876");
            assert_eq!(lock.contracts["work"].operations[0].name, "list");
        }
    }
}
