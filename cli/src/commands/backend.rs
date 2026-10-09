//! Shared local backend compilation and semantic-baseline handling.
use anyhow::{bail, Context, Result};
use axiom_extractor::EvaluatedAcoreConfig;
use axiom_lib::action::{Action, StepStatus};
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn baseline_path(source: &Path, variant: &str) -> PathBuf {
    let mut path = source.as_os_str().to_os_string();
    if variant != "default" {
        // Variant names are configuration keys, not filesystem paths.
        let encoded: String = variant
            .bytes()
            .map(|byte| match byte {
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' => (byte as char).to_string(),
                _ => format!("%{byte:02X}"),
            })
            .collect();
        path.push(format!(".{encoded}"));
    }
    path.push(".lockfile");
    PathBuf::from(path)
}

pub fn write_baseline(source: &Path, variant: &str, json: &str) -> Result<()> {
    // Reject invalid baselines before touching the previous successful snapshot.
    serde_json::from_str::<serde_json::Value>(json).context("invalid JSON build baseline")?;
    let path = baseline_path(source, variant);
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("create baseline beside {}", path.display()))?;
    temporary.write_all(json.as_bytes())?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(&path)
        .with_context(|| format!("write JSON baseline {}", path.display()))?;
    Ok(())
}

pub fn build_evaluated(
    source: &Path,
    evaluated: EvaluatedAcoreConfig,
    variant: &str,
    tx: Option<tokio::sync::mpsc::UnboundedSender<Action>>,
    output: &Path,
    compatibility_baseline: Option<&Path>,
) -> Result<String> {
    let artifact = axiom_build::core::build::build_evaluated_config_with_baseline(
        evaluated.config,
        variant,
        tx,
        output,
        compatibility_baseline,
    )?;
    write_baseline(source, variant, &evaluated.json)?;
    Ok(artifact)
}

pub async fn build_to(
    source: &Path,
    variant: &str,
    tx: Option<tokio::sync::mpsc::UnboundedSender<Action>>,
    compatibility_baseline: Option<&Path>,
    output: &Path,
) -> Result<String> {
    super::contract_build::validate_output(output)?;
    dotenvy::dotenv().ok();
    let emit = |status, message| {
        if let Some(tx) = &tx {
            let _ = tx.send(Action::UpdateBuildStep {
                index: 0,
                status,
                message,
            });
        }
    };
    emit(
        StepStatus::Processing,
        "Evaluating Acore configuration...".into(),
    );
    let path = source.to_str().context("Acore path is not valid UTF-8")?;
    let evaluated = match axiom_extractor::evaluate_acore_config_with_json(path, Some(variant)) {
        Ok(evaluated) => evaluated,
        Err(error) => {
            emit(StepStatus::Failed, error.to_string());
            return Err(error);
        }
    };
    emit(StepStatus::Success, "Acore evaluation complete.".into());
    // Compile the contract and private companion plan before touching any
    // successful project outputs. Individual files are replaced atomically.
    let staging = tempfile::tempdir()?;
    let staged = staging.path().join("backend.axiom");
    axiom_build::core::build::build_evaluated_config_with_baseline(
        evaluated.config,
        variant,
        tx,
        &staged,
        compatibility_baseline,
    )?;
    let plan = output.with_extension("server.json");
    if plan.is_symlink() {
        bail!(
            "Private server plan output cannot be a symlink: {}",
            plan.display()
        );
    }
    let staged_plan = staged.with_extension("server.json");
    if staged_plan.is_file() {
        super::contract_build::atomic_write(&plan, &std::fs::read(staged_plan)?)?;
    } else if plan.is_file() {
        let generated = std::fs::File::open(&plan)
            .ok()
            .filter(|f| f.metadata().is_ok_and(|m| m.len() <= 16_777_216))
            .and_then(|f| serde_json::from_reader::<_, axiom_lib::backend::ServerPlan>(f).ok())
            .is_some_and(|plan| plan.source_managed);
        if generated {
            std::fs::remove_file(&plan)?;
        }
    }
    super::contract_build::write_artifact(output, &std::fs::read(staged)?)?;
    write_baseline(source, variant, &evaluated.json)?;
    Ok(output.to_str().context("output path is not UTF-8")?.into())
}

pub fn validate(file: &Path, variant: Option<&str>) -> Result<()> {
    let variant = variant.unwrap_or("default");
    (|| -> Result<()> {
        let path = file.to_str().context("Acore path is not valid UTF-8")?;
        let config = axiom_extractor::evaluate_acore_config(path, Some(variant))?;
        // Use the real compiler, including domain/security and code generation
        // validation, without changing the project's artifact or diff baseline.
        let directory = tempfile::tempdir()?;
        let artifact = directory.path().join("checked.axiom");
        axiom_build::core::build::build_evaluated_config(config, variant, None, &artifact)?;
        Ok(())
    })()
}

pub fn explain(
    file: Option<&Path>,
    variant: Option<&str>,
    schema: bool,
    provenance: bool,
) -> Result<()> {
    if schema {
        println!(
            "{}",
            serde_json::to_string_pretty(&acore::backend::catalog::schema())?
        );
        return Ok(());
    }
    let file = file.context("pass an Acore file or use --schema")?;
    if provenance {
        let file = file.canonicalize().context("resolve backend source")?;
        let source = std::fs::read_to_string(&file)?;
        let inspection = acore::backend::inspection::compile(
            &format!("file://{}", file.display()),
            &source,
            &Default::default(),
            true,
            variant,
        )?;
        println!(
            "{}",
            serde_json::to_string_pretty(&inspection.explain(variant))?
        );
        return Ok(());
    }
    let path = file.to_str().context("Acore path is not valid UTF-8")?;
    println!("{}", axiom_extractor::evaluate_acore_json(path, variant)?);
    Ok(())
}

/// Migrate into new sibling files only after every variant has been compiled and
/// compared. Virtual source files preserve the original import base during the
/// check, so failures never leave a partially migrated source behind.
/// Preserve comments and unrelated manifest settings when selecting backend syntax.
pub(crate) fn backend_manifest(source: &str) -> Result<String> {
    let mut doc = source.parse::<toml_edit::DocumentMut>()?;
    if let Some(kind) = doc.get("type").and_then(toml_edit::Item::as_str) {
        if kind != "backend" {
            bail!("AxiomDeps.toml selects type = {kind:?}; choose a backend project directory");
        }
    }
    // Normalize only the obsolete discriminator; do not rewrite dependency tables.
    doc.remove("format");
    doc["type"] = toml_edit::value("backend");
    let text = doc.to_string();
    axiom_lib::deps_manifest::parse_axiom_deps_v2(&text)?;
    Ok(text)
}

pub fn migrate(source: &Path, output: &Path) -> Result<()> {
    let source = source.canonicalize().context("resolve migration input")?;
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()
        .context("resolve migration output directory")?;
    if source.parent() != Some(parent.as_path()) {
        bail!(
            "Migration output must be beside the original source so relative imports retain their meaning"
        );
    }
    let output = parent.join(
        output
            .file_name()
            .context("migration output must have a filename")?,
    );
    if output.extension().and_then(|extension| extension.to_str()) != Some("acore") {
        bail!("Migration output must use the .acore extension");
    }
    let sidecar = PathBuf::from(format!("{}.identities.json", output.display()));
    if output.exists() || sidecar.exists() {
        bail!(
            "Migration output or its identity sidecar already exists; choose a new output filename"
        );
    }
    let old_source = std::fs::read_to_string(&source)?;
    let source_uri = format!("file://{}", source.display());
    if acore::backend::is_backend_source(&old_source)
        || (acore::backend::is_backend_module(&source_uri, &old_source, &Default::default())
            && acore::backend::parse(&old_source).is_ok())
    {
        bail!("{} already uses the backend profile", source.display());
    }
    let migrated = acore::backend::migrate::migrate(&old_source)?;
    let manifest_path = parent.join("AxiomDeps.toml");
    let old_manifest = acore::backend::audience::manifest(&source_uri, &Default::default())
        .map(|(_, text)| text)
        .unwrap_or_default();
    let manifest = backend_manifest(&old_manifest)?;
    let output_uri = format!("file://{}", output.display());
    let full = migration_json(&source_uri, None, None, None)?.full;
    let mut identities = serde_json::Map::new();
    if let Some(endpoints) = full.get("endpoints").and_then(serde_json::Value::as_object) {
        for (key, endpoint) in endpoints {
            let id = endpoint
                .get("id")
                .and_then(serde_json::Value::as_u64)
                .filter(|id| *id > 0 && *id <= u32::MAX as u64)
                .with_context(|| format!("Endpoint '{key}' has no valid compiled ID"))?;
            let name = endpoint
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(key);
            identities.insert(
                key.clone(),
                serde_json::json!({"key":key,"name":name,"id":id}),
            );
        }
    }
    let identities = serde_json::to_string_pretty(&serde_json::json!({
        "format":"acore-backend-identities/v1", "endpoints":identities,
    }))?;
    let mut variants = full
        .get("variants")
        .and_then(serde_json::Value::as_object)
        .map(|variants| variants.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    if variants.is_empty() {
        variants.push("default".into());
    }
    let temporary = tempfile::tempdir()?;
    for (index, variant) in variants.iter().enumerate() {
        let old = migration_json(&source_uri, None, None, Some(variant))?;
        let new = migration_json(
            &output_uri,
            Some(&migrated),
            Some(&identities),
            Some(variant),
        )?;
        if old.selected != new.selected {
            let patch = acore_diff::diff(
                &acore_diff::value_utils::from_json(&old.selected),
                &acore_diff::value_utils::from_json(&new.selected),
            );
            bail!(
                "Migration changed the expanded contract for variant '{variant}':\n{}",
                acore_diff::renderers::render_atom(&patch)?
            );
        }
        let config =
            serde_json::from_value(new.full).context("decode migrated backend configuration")?;
        axiom_build::core::build::build_evaluated_config(
            config,
            variant,
            None,
            &temporary.path().join(format!("variant-{index}.axiom")),
        )?;
    }
    let mut source_file = tempfile::NamedTempFile::new_in(&parent)?;
    source_file.write_all(migrated.as_bytes())?;
    source_file.as_file().sync_all()?;
    let mut identity_file = tempfile::NamedTempFile::new_in(&parent)?;
    identity_file.write_all(identities.as_bytes())?;
    identity_file.write_all(b"\n")?;
    identity_file.as_file().sync_all()?;
    identity_file
        .persist_noclobber(&sidecar)
        .context("write migration identity sidecar")?;
    if let Err(error) = source_file.persist_noclobber(&output) {
        // This sidecar was created by this invocation and the source was never
        // installed. Remove only our newly created sidecar before reporting.
        let _ = std::fs::remove_file(&sidecar);
        return Err(error).context("write migrated source");
    }
    if let Err(error) = super::release::write_atomic(&manifest_path, manifest.as_bytes()) {
        let _ = std::fs::remove_file(&output);
        let _ = std::fs::remove_file(&sidecar);
        return Err(error).context("write backend manifest");
    }
    println!(
        "Migrated {} to {} ({} variant(s) verified).\nCompiler identities: {}",
        source.display(),
        output.display(),
        variants.len(),
        sidecar.display()
    );
    Ok(())
}

struct MigrationView {
    full: serde_json::Value,
    selected: serde_json::Value,
}

fn migration_json(
    uri: &str,
    virtual_source: Option<&str>,
    identities: Option<&str>,
    variant: Option<&str>,
) -> Result<MigrationView> {
    let mut evaluator =
        acore::evaluator::Evaluator::new(acore::security::SecurityManager::allow_all());
    evaluator.is_axiom_project = true;
    evaluator.active_variant = variant.map(str::to_string);
    if virtual_source.is_none() {
        // The migration input is explicitly legacy, even when other files in
        // this project have already migrated to the manifest-selected dialect.
        if let Some((manifest_uri, text)) =
            acore::backend::audience::manifest(uri, &Default::default())
        {
            let mut manifest = text.parse::<toml_edit::DocumentMut>()?;
            manifest.remove("type");
            manifest["format"] = toml_edit::value("axiom-deps/v2");
            evaluator
                .virtual_sources
                .insert(manifest_uri, manifest.to_string());
        }
    }
    if let Some(source) = virtual_source {
        evaluator.virtual_sources.insert(uri.into(), source.into());
        let manifest_uri = acore::backend::audience::manifest_candidates(uri)
            .into_iter()
            .next()
            .context("migration manifest URI")?;
        let old = acore::backend::audience::manifest(uri, &evaluator.virtual_sources)
            .map(|(_, text)| text)
            .unwrap_or_default();
        evaluator
            .virtual_sources
            .insert(manifest_uri, backend_manifest(&old)?);
    }
    if let Some(identities) = identities {
        evaluator
            .virtual_sources
            .insert(format!("{uri}.identities.json"), identities.into());
    }
    let value = evaluator.evaluate_module(uri)?;
    let (value, converters, _) = acore::render::extract_output_config(&mut evaluator, &value)?;
    let mut full = acore::render::materialize(&mut evaluator, &value, None, &converters)?;
    if evaluator.backend_profile {
        acore::backend::validate::validate(&full)?;
    }
    let mut selected = full.clone();
    acore::render::apply_variant_filtering(&mut selected, variant.unwrap_or("default"));
    acore::render::prune_empty_fields(&mut full);
    acore::render::prune_empty_fields(&mut selected);
    Ok(MigrationView {
        full: serde_json::from_str(&acore::render::JsonRenderer::new().render(&full))?,
        selected: serde_json::from_str(&acore::render::JsonRenderer::new().render(&selected))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evaluated(json: &str) -> EvaluatedAcoreConfig {
        EvaluatedAcoreConfig {
            config: serde_json::from_str(json).unwrap(),
            json: json.into(),
        }
    }

    const CONFIG: &str = r#"{
        "project":{"id":"backend-cli-test","version":"1.0.0"},
        "variants":{"default":{},"mobile":{}},
        "models":{},"endpoints":{},"extraSourceMetadata":{"retained":true}
    }"#;

    #[test]
    fn backend_default_and_variant_baselines_are_independent_json_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("contract.acore");
        std::fs::write(&source, "source is deliberately not JSON").unwrap();
        for variant in ["default", "mobile"] {
            build_evaluated(
                &source,
                evaluated(CONFIG),
                variant,
                None,
                &dir.path().join(format!("{variant}.axiom")),
                None,
            )
            .unwrap();
            let baseline: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(baseline_path(&source, variant)).unwrap(),
            )
            .unwrap();
            assert_eq!(baseline["extraSourceMetadata"]["retained"], true);
        }
        assert_ne!(
            baseline_path(&source, "default"),
            baseline_path(&source, "mobile")
        );
        assert_eq!(
            baseline_path(Path::new("contract.acore"), "default"),
            Path::new("contract.acore.lockfile")
        );
    }

    #[test]
    fn backend_failed_build_preserves_previous_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("contract.acore");
        write_baseline(&source, "default", CONFIG).unwrap();
        let previous = std::fs::read(baseline_path(&source, "default")).unwrap();
        let invalid = r#"{"variants":{"default":{}}}"#;
        assert!(build_evaluated(
            &source,
            evaluated(invalid),
            "default",
            None,
            &dir.path().join("failed.axiom"),
            None
        )
        .is_err());
        assert_eq!(
            std::fs::read(baseline_path(&source, "default")).unwrap(),
            previous
        );
        assert!(write_baseline(&source, "default", "not JSON").is_err());
        assert_eq!(
            std::fs::read(baseline_path(&source, "default")).unwrap(),
            previous
        );
    }

    #[test]
    fn backend_variant_names_cannot_escape_baseline_directory() {
        let source = Path::new("/tmp/contract.acore");
        let baseline = baseline_path(source, "../../other/mobile");
        assert_eq!(baseline.parent(), source.parent());
        assert!(baseline
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("%2F"));
        assert_ne!(baseline_path(source, "a/b"), baseline_path(source, "a%2Fb"));
    }

    #[test]
    fn backend_variant_build_validates_full_security_and_domain_before_filtering() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("variant.acore");
        std::fs::write(
            &source,
            r#"profile backend 1
project(id: "variant-security", version: "1.0.0")
variant default(include: ["*"])
variant mobile(include: ["health"])
model Project { id: String title: String }
entity project from Models.Project
projection card(entity: Entities.project, fields: ["id", "title"])
endpoint health(method: GET, path: "/health") { response(type: Void) }
endpoint projects(method: GET, path: "/projects") {
  response(type: List<Entities.project>, projection: Projections.card)
}
security(mode: "audit", endpoints: {
  health: EndpointGuard(public: true),
  projects: EndpointGuard(public: true)
})
"#,
        )
        .unwrap();
        let evaluated = axiom_extractor::evaluate_acore_config_with_json(
            source.to_str().unwrap(),
            Some("mobile"),
        )
        .unwrap();
        assert_eq!(evaluated.config.endpoints.len(), 2);
        evaluated.config.validate_domain().unwrap();
        evaluated.config.validate_security().unwrap();
        let selected: serde_json::Value = serde_json::from_str(&evaluated.json).unwrap();
        assert_eq!(selected["endpoints"].as_object().unwrap().len(), 1);
        assert!(selected["endpoints"].get("health").is_some());
        let artifact = directory.path().join("mobile.axiom");
        build_evaluated(&source, evaluated, "mobile", None, &artifact, None).unwrap();
        let artifact =
            axiom_lib::unpackager::unpack_axiom_bytes(&std::fs::read(artifact).unwrap()).unwrap();
        assert_eq!(artifact.endpoints.len(), 1);
        assert_eq!(artifact.endpoints[0].name, "health");
    }
}
