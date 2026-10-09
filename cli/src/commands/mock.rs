//! Explicit full-server or labeled contract-only simulation, never real fallback.
use super::server_command_contract::{MockInput, PreparationInput, ResolvedInput};
use anyhow::{bail, ensure, Context, Result};
use axiom_lib::{
    backend::{ServerPlan, ServingMode},
    contract::AxiomFile,
};
use axiom_server::{
    development::ValidatedMockFixtures, host::ActionHost, invocation::InvocationCancellation,
    simulation::MockIdentities,
};
use std::{net::SocketAddr, sync::Arc};

pub async fn handle(input: MockInput) -> Result<()> {
    handle_resolved(input.resolve(&std::env::current_dir()?)?).await
}
enum Prepared {
    Server {
        host: Arc<ActionHost>,
        _artifacts: Option<tempfile::TempDir>,
    },
    Contract(axiom_mock::models::AxiomConfig),
}
pub async fn handle_resolved(resolved: ResolvedInput) -> Result<()> {
    let listen = SocketAddr::new(resolved.host, resolved.port);
    let debug = resolved.debug;
    let cancellation = InvocationCancellation::default();
    let token = cancellation.clone();
    let signal = tokio::spawn(async move {
        axiom_server::development::shutdown_signal().await;
        token.cancel();
    });
    let result=async {
        if resolved.watch { return super::mock_development::supervise(resolved,cancellation).await; }
        let prepared=if matches!(resolved.input,PreparationInput::Cloud) {
            resolved.mode(false)?;
            let json=super::legacy_mock::cloud_configuration().await?;
            Prepared::Contract(cloud_contract_config(json.as_bytes())?)
        } else {
            tokio::task::spawn_blocking(move ||prepare(resolved)).await.context("mock preparation task failed")??
        };
        match prepared {
            Prepared::Server{host,_artifacts}=>{
                let _artifacts=_artifacts;
                axiom_server::development::real_session(host,listen,debug).run(cancellation).await?;
            },
            Prepared::Contract(config)=>{
                // Non-loopback contract mocks protect data/docs/reset with an
                // isolated key. Loopback is the explicit legacy development profile.
                let owned=tempfile::Builder::new().prefix("axiom-contract-mock-").tempdir()?;
                let access_token=(!listen.ip().is_loopback()).then(||format!("{}{}",uuid::Uuid::new_v4(),uuid::Uuid::new_v4()));
                let credentials=owned.path().join("credentials.json");
                if let Some(token)=&access_token {
                    use std::io::Write;
                    let mut file=std::fs::OpenOptions::new();file.write(true).create_new(true);
                    #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;file.mode(0o600);}
                    file.open(&credentials)?.write_all(serde_json::to_string(&serde_json::json!({"headers":{"X-Axiom-Sandbox-Key":token}}))?.as_bytes())?;
                }
                if cancellation.is_cancelled(){return Ok(());}
                let listener=tokio::net::TcpListener::bind(listen).await.context("contract mock listener unavailable; check host and port")?;
                let address=listener.local_addr()?;
                println!("{}",serde_json::json!({"ready":true,"mode":"contract-mock","profile":"contract","address":address,"port":address.port(),"debug":debug,"privateGuards":"unavailable","credentialsFile":access_token.as_ref().map(|_|credentials)}));
                // New debug mode is redacted; legacy engine payload logging stays off.
                axiom_mock::server::serve_config_on_listener_with_shutdown(config,listener,false,access_token,async move {
                    while !cancellation.is_cancelled(){tokio::time::sleep(std::time::Duration::from_millis(25)).await;}
                }).await.map_err(|_|anyhow::anyhow!("contract mock transport failed"))?;
            }
        }
        Ok(())
    }.await;
    signal.abort();
    result
}
fn prepare(resolved: ResolvedInput) -> Result<Prepared> {
    let mut temporary = None;
    let mut config = None;
    let (contract_path, plan_path) = match &resolved.input {
        PreparationInput::Source { path, variant } => {
            let variant = variant.as_deref().unwrap_or("default");
            let evaluated = axiom_extractor::evaluate_acore_config_with_json(
                path.to_str().context("source path must be UTF-8")?,
                Some(variant),
            )?;
            let directory = tempfile::Builder::new()
                .prefix("axiom-mock-build-")
                .tempdir()?;
            let artifact = directory.path().join("axiom.axiom");
            axiom_build::core::build::build_evaluated_config(
                evaluated.config,
                variant,
                None,
                &artifact,
            )?;
            config = Some(
                serde_json::from_str::<axiom_mock::models::AxiomConfig>(&evaluated.json)
                    .context("invalid contract mock scenario schema")?,
            );
            let plan = artifact.with_extension("server.json");
            let plan = plan.is_file().then_some(plan);
            temporary = Some(directory);
            (artifact, plan)
        }
        PreparationInput::Artifact { contract, plan } => (contract.clone(), plan.clone()),
        PreparationInput::Cloud => bail!("cloud input must use explicit cloud retrieval"),
    };
    let mode = resolved.mode(plan_path.is_some())?;
    if mode == ServingMode::ServerMock {
        let contract: AxiomFile = axiom_server::serve::read_json(&contract_path, 16_777_216)?;
        let plan: ServerPlan = axiom_server::serve::read_json(
            plan_path
                .as_deref()
                .context("full mock requires its private plan")?,
            16_777_216,
        )?;
        let bytes = axiom_server::serve::read(
            resolved
                .fixtures
                .as_deref()
                .context("full mock requires private fixtures")?,
            axiom_lib::backend::MAX_FIXTURE_BYTES as u64,
        )?;
        let fixtures = ValidatedMockFixtures::from_json(&bytes, &contract, &plan)?;
        let identities = match resolved.identities {
            Some(path) => MockIdentities::from_json(&axiom_server::serve::read(&path, 1_048_576)?)?,
            None => MockIdentities::default(),
        };
        let host = Arc::new(ActionHost::simulated(contract, plan, fixtures, identities)?);
        Ok(Prepared::Server {
            host,
            _artifacts: temporary,
        })
    } else {
        let config = match config {
            Some(config) => config,
            None => {
                let contract: AxiomFile =
                    axiom_server::serve::read_json(&contract_path, 16_777_216)?;
                serde_json::from_value(serde_json::to_value(contract.ir)?)?
            }
        };
        axiom_mock::validation::validate_mock_config(&config).map_err(|e| anyhow::anyhow!(e))?;
        Ok(Prepared::Contract(config))
    }
}

fn cloud_contract_config(bytes: &[u8]) -> Result<axiom_mock::models::AxiomConfig> {
    let value: serde_json::Value = axiom_server::serve::parse_json(bytes, 4_194_304)?;
    let config = value.get("configuration").unwrap_or(&value);
    for candidate in [&value, config] {
        ensure!(
            candidate
                .get("server")
                .is_none_or(serde_json::Value::is_null)
                && candidate.get("format").and_then(serde_json::Value::as_str)
                    != Some(axiom_lib::backend::MOCK_FIXTURE_FORMAT),
            "cloud contract profile cannot consume private server plans or fixtures"
        );
    }
    ensure!(
        value.get("configuration").is_none() || value.get("endpoints").is_none(),
        "ambiguous raw/wrapped cloud mock configuration"
    );
    let config =
        serde_json::from_value(config.clone()).context("invalid cloud contract mock fields")?;
    axiom_mock::validation::validate_mock_config(&config)
        .map_err(|_| anyhow::anyhow!("invalid cloud contract mock scenarios"))?;
    Ok(config)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cloud_raw_wrapped_and_ambiguous_private_configs_have_explicit_profiles() {
        let raw = serde_json::json!({"endpoints":{"hello":{"method":"GET","path":"/hello","mock":{"responses":[{"data":{"type":"static","body":"hello"}}]}}}});
        for value in [
            raw.clone(),
            serde_json::json!({"configuration":raw.clone()}),
        ] {
            assert_eq!(
                cloud_contract_config(&serde_json::to_vec(&value).unwrap())
                    .unwrap()
                    .endpoints
                    .len(),
                1
            );
        }
        for value in [
            serde_json::json!({"server":{}}),
            serde_json::json!({"format":axiom_lib::backend::MOCK_FIXTURE_FORMAT}),
            serde_json::json!({"configuration":raw,"endpoints":{}}),
        ] {
            assert!(cloud_contract_config(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        assert!(cloud_contract_config(br#"{"endpoints":{},"endpoints":{}}"#).is_err());
    }
}
