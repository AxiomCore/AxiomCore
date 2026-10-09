//! Database authoring, inspection, isolated replay and explicit signed target operations.
use anyhow::{bail, Context, Result};
use axiom_lib::{
    database::{self, DatabaseContractIR},
    package::*,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

pub(super) fn source(path: &Path) -> Result<(PathBuf, acore::database::Compilation)> {
    let (root, input) =
        acore::database::project::load(path, &HashMap::new()).map_err(anyhow::Error::msg)?;
    let compiled = acore::database::compile(&input).map_err(|errors| {
        anyhow::anyhow!(serde_json::to_string_pretty(&errors)
            .unwrap_or_else(|_| "database compilation failed".into()))
    })?;
    Ok((root, compiled))
}
pub(super) fn contract(path: &Path) -> Result<DatabaseContractIR> {
    if path.extension().is_some_and(|e| e == "axiom") {
        let DecodedAxiomPackage::Package(package) = decode_axiom_package(&std::fs::read(path)?)?
        else {
            bail!("ADB128: expected a typed database-v2 schema");
        };
        database::contract_from_package(&package)
    } else {
        Ok(source(path)?.1.contract)
    }
}
pub fn check(path: &Path) -> Result<()> {
    let (_, compiled) = source(path)?;
    println!("{}", serde_json::to_string_pretty(&compiled.evidence()?)?);
    Ok(())
}
pub fn build(path: &Path, out: Option<&Path>) -> Result<()> {
    println!("{}", build_artifact(path, out)?);
    Ok(())
}
pub(super) fn build_artifact(path: &Path, out: Option<&Path>) -> Result<serde_json::Value> {
    let (_, compiled) = source(path)?;
    let encoded = axiom_build::core::database::prepare(&compiled.contract, compiled.identity)?;
    let path = out.map(Path::to_path_buf).unwrap_or_else(|| {
        path.parent().unwrap_or(Path::new(".")).join("database.axiom")
    });
    if path.extension().is_none_or(|e| e != "axiom") {
        bail!("ADB160: schema output must use .axiom");
    }
    if path.is_symlink() {
        bail!("ADB160: schema output cannot be a symlink");
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    use std::io::Write;
    temp.write_all(&encoded.bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(&path)
        .map_err(|e| anyhow::anyhow!("write schema artifact: {}", e.error))?;
    Ok(serde_json::json!({"format":"axiom-database-build/v1","profile":database::PROFILE,"artifact":path,"sha256":encoded.sha256,"effects":"offline-only","migrationAuthority":false}))
}
pub fn inspect(path: &Path) -> Result<()> {
    if path.extension().is_some_and(|e| e == "axiom") {
        let DecodedAxiomPackage::Package(package) = decode_axiom_package(&std::fs::read(path)?)?
        else {
            bail!("ADB128: expected database package");
        };
        if let PackageContents::DatabaseSchema(payload) = &package.contents {
            if payload.schema == DATABASE_SCHEMA_PACKAGE_SCHEMA {
                println!(
                    "{}",
                    serde_json::json!({"schema":payload.schema,"status":"legacy-data-only","exportNames":payload.exports.keys().collect::<Vec<_>>(),"migrationAuthority":false})
                );
                return Ok(());
            }
        }
    }
    let c = contract(path)?;
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"profile":c.profile,"engine":c.engine,"database":c.database,"namespace":c.namespace,"contractSha256":c.sha256()?,"schemas":c.schemas,"sqlite":c.sqlite,"exports":c.exports()?,"liveState":"not-observed","backendImpact":"unknown-no-runtime-mappings","migrationAuthority":false})
        )?
    );
    Ok(())
}
pub fn diff(before: &Path, after: &Path) -> Result<()> {
    let report = database::change::diff(&contract(before)?, &contract(after)?)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
pub fn format(path: &Path, write: bool) -> Result<()> {
    let source = std::fs::read_to_string(path).context("read database source")?;
    let formatted = acore::database::format::format(&source)?;
    if write {
        std::fs::write(path, formatted)?;
    } else {
        print!("{formatted}");
    }
    Ok(())
}
pub fn catalog() -> Result<()> {
    let declarations:Vec<_>=acore::database::catalog::SIGNATURES.iter().chain(acore::database::catalog::POSTGRES_SIGNATURES).map(|s|serde_json::json!({"kind":s.kind,"named":s.named,"required":s.required,"optional":s.optional,"children":s.children})).collect();
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"profile":database::PROFILE,"scope":"offline-authoring-only","declarations":declarations,"types":acore::database::catalog::TYPES,"evolutionDeclarations":acore::database::catalog::EVOLUTION_SIGNATURES.iter().map(|s|serde_json::json!({"kind":s.kind,"named":s.named,"required":s.required,"optional":s.optional,"children":s.children})).collect::<Vec<_>>(),"evolutionCoordination":["maintenanceFenced"],"columnOptions":acore::database::catalog::COLUMN_OPTIONS,"members":acore::database::catalog::MEMBERS,"expressions":acore::database::catalog::EXPRESSIONS,"sqlite":{"declarations":acore::database::catalog::SQLITE_SIGNATURES.iter().map(|s|serde_json::json!({"kind":s.kind,"named":s.named,"required":s.required,"optional":s.optional,"children":s.children})).collect::<Vec<_>>(),"types":acore::database::catalog::SQLITE_TYPES,"columnOptions":acore::database::catalog::SQLITE_OPTIONS,"expressions":acore::database::catalog::SQLITE_EXPRESSIONS}})
        )?
    );
    Ok(())
}

use axiom_database::inspection::{
    bindings::{Deployment, NativeProvider},
    normalize::{self, Candidate},
    postgres::{self, Request},
};
use database::inspection::Report;

fn bounded_read(path: &Path, limit: u64, private: bool) -> Result<Vec<u8>> {
    use std::io::Read;
    let meta = std::fs::symlink_metadata(path)
        .map_err(|_| anyhow::anyhow!("ADB200: input file is unavailable"))?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > limit {
        bail!("ADB200: input must be a regular bounded file");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if private && meta.permissions().mode() & 0o077 != 0 {
            bail!("ADB200: private deployment requires owner-only permissions");
        }
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| anyhow::anyhow!("ADB200: input file is unavailable"))?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("ADB200: input could not be read"))?;
    if bytes.len() as u64 > limit {
        bail!("ADB200: input exceeds budget");
    }
    Ok(bytes)
}
fn binding(path: &Path) -> Result<(Request, NativeProvider)> {
    let deployment = Deployment::decode(&bounded_read(path, 65_536, true)?)?;
    let root = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()
        .map_err(|_| anyhow::anyhow!("ADB200: binding directory unavailable"))?;
    Ok((Request::new(deployment), NativeProvider { root }))
}
pub fn binding_check(path: &Path) -> Result<()> {
    let (request, _) = binding(path)?;
    request.deployment.validate()?;
    println!(
        "{}",
        serde_json::json!({"format":"axiom-database-binding-check/v1","namespace":request.deployment.namespace,"database":request.deployment.database,"postgresMajor":request.deployment.postgres_major,"schemas":request.deployment.schemas,"providersResolved":false,"connectionObserved":false,"migrationAuthority":false})
    );
    Ok(())
}
pub async fn inspect_live(path: &Path, desired: Option<&Path>, out: Option<&Path>) -> Result<()> {
    let (mut request, provider) = binding(path)?;
    request.desired_contract = desired.map(contract).transpose()?;
    request.validate()?;
    if let Some(path) = out {
        ensure_new_output(path)?;
    }
    let (report, candidate) = postgres::inspect(&request, &provider).await?;
    emit(report, candidate, out)
}
pub async fn verify_isolation(path: &Path, desired: Option<&Path>) -> Result<()> {
    let (mut request, provider) = binding(path)?;
    request.desired_contract = desired.map(contract).transpose()?;
    println!(
        "{}",
        serde_json::to_string_pretty(&postgres::verify_isolation(&request, &provider).await?)?
    );
    Ok(())
}
pub fn import(
    path: &Path,
    namespace: &str,
    name: &str,
    schemas: &[String],
    major: u16,
    kind: &str,
    out: Option<&Path>,
) -> Result<()> {
    if let Some(path) = out {
        ensure_new_output(path)?;
    }
    let bytes = bounded_read(path, axiom_database::inspection::MAX_INPUT as u64, false)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| anyhow::anyhow!("ADB230: static import must be UTF-8"))?;
    let catalog = match kind {
        "sql" => axiom_database::inspection::imports::sql(text)?,
        "hcl" => axiom_database::inspection::imports::hcl(text)?,
        _ => bail!("ADB230: unknown static import format"),
    };
    let (report, candidate) = normalize::prepare(
        catalog,
        namespace,
        name,
        major,
        schemas,
        kind,
        &axiom_database::digest(&bytes),
    )?;
    emit(report, candidate, out)
}
fn ensure_new_output(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty()
        || path.file_name().is_none()
        || std::fs::symlink_metadata(path).is_ok()
    {
        bail!("ADB228: adoption output must be a new project directory");
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if !parent.is_dir() || parent.is_symlink() {
        bail!("ADB228: adoption parent must be an existing real directory");
    }
    Ok(())
}
fn emit(report: Report, candidate: Option<Candidate>, out: Option<&Path>) -> Result<()> {
    report.validate()?;
    if let Some(path) = out {
        let Some(candidate) = candidate else {
            println!("{}", serde_json::to_string_pretty(&report)?);
            bail!("ADB228: incomplete coverage prevents adoption output; no project was created");
        };
        ensure_new_output(path)?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let staged = tempfile::Builder::new()
            .prefix(".axiom-adoption-")
            .tempdir_in(parent)?;
        fn write(path: &Path, bytes: &[u8]) -> Result<()> {
            use std::io::Write;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(path)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            Ok(())
        }
        write(
            &staged.path().join("AxiomDeps.toml"),
            candidate.manifest.as_bytes(),
        )?;
        write(
            &staged.path().join("schema.acore"),
            candidate.source.as_bytes(),
        )?;
        write(&staged.path().join(".gitignore"), b".axiom/\n")?;
        let mut directory = staged.path().to_path_buf();
        for part in [".axiom", "database", "v1"] {
            directory.push(part);
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(&directory)?;
        }
        for part in ["adoption", "contracts"] {
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(directory.join(part))?;
        }
        write(
            &directory.join("adoption/observation.json"),
            &serde_json::to_vec(&report)?,
        )?;
        write(
            &directory.join("adoption/graph.json"),
            &serde_json::to_vec_pretty(&report.graph())?,
        )?;
        write(
            &directory.join(format!(
                "contracts/{}.axiom",
                axiom_database::digest(&candidate.artifact)
            )),
            &candidate.artifact,
        )?;
        // Reserve the destination atomically. If a writer won the race, preserve
        // their directory. Prepared children are moved only into our reservation.
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(path)
            .map_err(|_| anyhow::anyhow!("ADB228: adoption destination could not be reserved"))?;
        let result = (|| -> Result<()> {
            for entry in std::fs::read_dir(staged.path())? {
                let entry = entry?;
                std::fs::rename(entry.path(), path.join(entry.file_name()))?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            let _ = std::fs::remove_dir_all(path);
            return Err(error);
        }
    }
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
pub fn graph(path: &Path, id: Option<&str>) -> Result<()> {
    if path.extension().is_some_and(|e| e == "json") {
        let report = Report::decode(&bounded_read(path, 4_194_304, false)?)?;
        if id.is_some_and(|id| !report.objects.iter().any(|o| o.id == id)) {
            bail!("ADB229: catalog object is absent from this evidence");
        }
        let value = id
            .map(|id| report.explain(id))
            .unwrap_or_else(|| report.graph());
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        let c = contract(path)?;
        let edges = database::dependency_edges(&c)?;
        let objects = serde_json::to_value(&c)?;
        fn find<'a>(v: &'a serde_json::Value, id: &str) -> Option<&'a serde_json::Value> {
            match v {
                serde_json::Value::Object(map) => {
                    if map.get("id").and_then(serde_json::Value::as_str) == Some(id) {
                        return Some(v);
                    }
                    map.values().find_map(|v| find(v, id))
                }
                serde_json::Value::Array(a) => a.iter().find_map(|v| find(v, id)),
                _ => None,
            }
        }
        let value = if let Some(id) = id {
            let object = find(&objects, id)
                .ok_or_else(|| anyhow::anyhow!("ADB229: object is absent from this contract"))?;
            serde_json::json!({"format":"axiom-database-contract-explain/v1","object":object,"dependencies":edges.iter().filter(|e|e.consumer==id||e.target==id).collect::<Vec<_>>(),"backendImpact":"unknown-no-runtime-mappings","migrationAuthority":false})
        } else {
            {
                let mut value = serde_json::json!({"format":"axiom-database-contract-graph/v1","schemas":c.schemas,"edges":edges,"liveState":"not-observed","backendImpact":"unknown-no-runtime-mappings","migrationAuthority":false});
                if let Some(postgres) = c.postgres {
                    value["postgres"] = serde_json::to_value(postgres)?;
                }
                value
            }
        };
        println!("{}", serde_json::to_string_pretty(&value)?);
    }
    Ok(())
}

pub async fn plan(
    path: &Path,
    deployment: &Path,
    dev_profile: &Path,
    baseline: Option<&str>,
    parent: Option<&str>,
    tests: Option<&Path>,
    evolution: Option<&Path>,
    checkpoint: Option<&Path>,
) -> Result<()> {
    use axiom_database::planning::{
        self, history, tests::Descriptor, Baseline, Development, Inputs,
    };
    let (root, compiled) = source(path)?;
    if checkpoint.is_some() && history::head(&root)?.is_some() {
        bail!("ADB518: checkpoint fresh installation requires a new local private-history root; existing projects retain their accepted prefix");
    }
    let (target, provider) = binding(deployment)?;
    let development = Development::decode(&bounded_read(dev_profile, 65_536, true)?)?;
    let dev_provider = NativeProvider {
        root: dev_profile
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .canonicalize()
            .map_err(|_| anyhow::anyhow!("ADB305: dev provider directory unavailable"))?,
    };
    let selected_parent = if baseline.is_some() {
        None
    } else {
        parent.map(str::to_owned).or(history::head(&root)?)
    };
    let (baseline, history) = match (baseline, selected_parent.as_deref()) {
        (Some("empty"), None) => (Baseline::Empty, history::VerifiedHistory::empty()),
        (Some("adoption"), None) => (Baseline::Adoption, history::VerifiedHistory::empty()),
        (None, Some(head)) => (Baseline::History, history::load(&root, head)?),
        _ => bail!(
            "ADB300: first plan requires --baseline empty or --baseline adoption; later plans use the local candidate head or --parent"
        ),
    };
    let tests = tests
        .map(|p| -> Result<Descriptor> {
            Ok(Descriptor::decode(&bounded_read(p, 1_048_576, false)?)?)
        })
        .transpose()?
        .unwrap_or_default();
    let mut inputs = Inputs::from_compilation(&compiled)?;
    let mut intent = evolution
        .map(|p| -> Result<_> {
            let bytes = bounded_read(p, 262144, false)?;
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| anyhow::anyhow!("ADB501: evolution source must be UTF8"))?;
            let previous = if baseline == Baseline::Adoption {
                Some(compiled.contract.clone())
            } else {
                history.state().contract
            };
            let mut intent =
                acore::database::evolution::compile(text, previous.as_ref(), &compiled.contract)?;
            planning::evolution::bind(&mut intent, &history)?;
            Ok(intent)
        })
        .transpose()?;
    if let (Some(path), Some(intent)) = (evolution, intent.as_ref()) {
        let absolute = path.canonicalize()?;
        let relative = absolute
            .strip_prefix(&root)
            .map_err(|_| {
                anyhow::anyhow!(
                    "ADB501: evolution source must belong to the selected project snapshot"
                )
            })?
            .to_string_lossy()
            .replace('\\', "/");
        if !acore::database::project::validate_path(&relative)
            || path.extension().is_none_or(|e| e != "acore")
        {
            bail!("ADB501: evolution requires a project .acore source");
        }
        if inputs
            .source_hashes
            .insert(relative, intent.source_sha256.clone())
            .is_some()
        {
            bail!("ADB501: evolution input must be separate from schema inputs");
        }
    }
    let fresh_install = checkpoint
        .map(planning::checkpoint::Archive::load)
        .transpose()?;
    if let Some(archive) = &fresh_install {
        if intent.is_some() || baseline != Baseline::Empty || history.head().is_some() {
            bail!(
                "ADB518: --checkpoint requires --baseline empty and supplies its own exact seed intent"
            );
        }
        intent = Some(archive.prepare(&compiled.contract, &mut inputs)?);
    }
    let mut request = planning::Request::new(
        target.deployment,
        development,
        compiled.contract,
        inputs,
        baseline,
        history,
        tests,
    );
    request.evolution = intent;
    request.fresh_install = fresh_install;
    let candidate = planning::validate(&request, &provider, &dev_provider).await?;
    history::store(&root, &candidate)?;
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"format":"axiom-database-candidate/v1","sha256":candidate.sha256,"report":candidate.report,"history":"private-local-candidate","migrationAuthority":false,"acceptedHead":null})
        )?
    );
    Ok(())
}
pub fn history_export(path: &Path, head: Option<&str>, out: &Path) -> Result<()> {
    use axiom_database::planning::history;
    let (root, _) = source(path)?;
    let head = head
        .map(str::to_owned)
        .or(history::head(&root)?)
        .ok_or_else(|| anyhow::anyhow!("ADB333: no local candidate history"))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&history::export(&root, &head, out)?)?
    );
    Ok(())
}
pub fn history_verify(bundle: &Path) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&axiom_database::planning::history::verify_export(bundle)?)?
    );
    Ok(())
}

pub fn history_import(bundle: &Path, path: &Path) -> Result<()> {
    let (root, compiled) = source(path)?;
    let inputs = axiom_database::planning::Inputs::from_compilation(&compiled)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&axiom_database::planning::history::import(
            &root,
            bundle,
            &inputs.package_sha256
        )?)?
    );
    Ok(())
}
pub async fn history_import_atlas(
    bundle: &Path,
    directory: &Path,
    path: &Path,
    revision_schema: &str,
    revision_table: &str,
    legacy_stopped: bool,
) -> Result<()> {
    let target = axiom_database::execution::Target::decode(&bounded_read(path, 65_536, true)?)?;
    let provider = axiom_database::inspection::bindings::NativeProvider {
        root: path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .into(),
    };
    println!(
        "{}",
        serde_json::to_string_pretty(
            &axiom_database::execution::interop::prepare(
                bundle,
                directory,
                &target,
                &provider,
                revision_schema,
                revision_table,
                legacy_stopped
            )
            .await?
        )?
    );
    Ok(())
}
pub fn release_keygen(out: &Path) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&axiom_database::execution::authority::keygen(out)?)?
    );
    Ok(())
}
pub fn release_sign(
    bundle: &Path,
    key: &Path,
    destructive: bool,
    data_dependent: bool,
) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&axiom_database::execution::authority::sign(
            bundle,
            key,
            destructive,
            data_dependent
        )?)?
    );
    Ok(())
}
pub fn release_verify(bundle: &Path, key: &str) -> Result<()> {
    let bundle = axiom_database::execution::Bundle::load(bundle, key)?;
    println!(
        "{}",
        serde_json::json!({"format":"axiom-database-release-verification/v1","head":bundle.head(),"signature":"ed25519-verified","effects":"offline-only"})
    );
    Ok(())
}
pub fn target_binding_check(path: &Path) -> Result<()> {
    let binding = axiom_database::execution::Target::decode(&bounded_read(path, 65_536, true)?)?;
    println!(
        "{}",
        serde_json::json!({"format":"axiom-database-target-check/v1","profile":binding.profile,"providerResolution":false,"mutationAuthority":false})
    );
    Ok(())
}
pub async fn target_operation(
    bundle: &Path,
    path: &Path,
    action: axiom_database::execution::Action,
) -> Result<()> {
    let target = axiom_database::execution::Target::decode(&bounded_read(path, 65_536, true)?)?;
    let bundle = axiom_database::execution::Bundle::load(bundle, &target.trusted_key)?;
    let provider = axiom_database::inspection::bindings::NativeProvider {
        root: path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .into(),
    };
    let report = axiom_database::execution::execute(&target, &bundle, &provider, action).await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

pub fn history_checkpoint(path: &Path, head: Option<&str>, out: &Path) -> Result<()> {
    let (root, _) = source(path)?;
    let head = head
        .map(str::to_owned)
        .or(axiom_database::planning::history::head(&root)?)
        .ok_or_else(|| anyhow::anyhow!("ADB518: no candidate history"))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&axiom_database::planning::checkpoint::export(
            &root, &head, out
        )?)?
    );
    Ok(())
}
pub fn release_repair_index(
    bundle: &Path,
    deployment: &Path,
    index_id: &str,
    index_oid: &str,
    strategy: &str,
    key_file: &Path,
    out: &Path,
) -> Result<()> {
    use axiom_database::execution::{repair, Bundle, Target};
    let target = Target::decode(&bounded_read(deployment, 65536, true)?)?;
    let bundle = Bundle::load(bundle, &target.trusted_key)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&repair::sign(
            &bundle, &target, index_id, index_oid, strategy, key_file, out
        )?)?
    );
    Ok(())
}
pub async fn target_reconcile(
    bundle: &Path,
    deployment: &Path,
    repair: Option<&Path>,
) -> Result<()> {
    if let Some(repair) = repair {
        use axiom_database::execution::{self, repair::Permit, Bundle, Target};
        let target = Target::decode(&bounded_read(deployment, 65536, true)?)?;
        let bundle = Bundle::load(bundle, &target.trusted_key)?;
        let permit = Permit::decode(&bounded_read(repair, 16384, true)?, &target.trusted_key)?;
        let provider = NativeProvider {
            root: deployment
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."))
                .canonicalize()?,
        };
        println!(
            "{}",
            serde_json::to_string_pretty(
                &execution::execute_repair(&target, &bundle, &provider, &permit).await?
            )?
        );
        Ok(())
    } else {
        target_operation(
            bundle,
            deployment,
            axiom_database::execution::Action::Reconcile,
        )
        .await
    }
}

pub fn evolution_check(path: &Path, previous: Option<&Path>, evolution: &Path) -> Result<()> {
    use axiom_database::planning::{self, Inputs, State};
    let (_, current) = source(path)?;
    let previous = previous.map(source).transpose()?.map(|(_, c)| c);
    let bytes = bounded_read(evolution, 262144, false)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| anyhow::anyhow!("ADB501: evolution must be UTF8"))?;
    let intent = acore::database::evolution::compile(
        text,
        previous.as_ref().map(|c| &c.contract),
        &current.contract,
    )?;
    let before = previous
        .as_ref()
        .map(|c| State::new(c.contract.clone()))
        .transpose()?
        .unwrap_or_else(State::empty);
    let inputs = Inputs::from_compilation(&current)?;
    let previous_inputs = previous
        .as_ref()
        .map(Inputs::from_compilation)
        .transpose()?;
    let program = planning::evolution::transition(
        &before,
        &State::new(current.contract)?,
        previous_inputs.as_ref(),
        &inputs,
        &intent,
    )?;
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"format":"axiom-database-evolution-check/v1","fromSha256":intent.from_sha256,"toSha256":intent.to_sha256,"sourceSha256":intent.source_sha256,"coordination":intent.coordination,"operations":program.operations,"referenceOwners":intent.reference_data.iter().map(|r|serde_json::json!({"owner":r.owner,"tableId":r.table_id,"rows":r.rows.len(),"maxRows":r.max_rows,"maxChanges":r.max_changes})).collect::<Vec<_>>(),"referenceOwnership":"requires-verified-private-history","effects":"offline-only","migrationAuthority":false,"backendImpact":"unknown-no-runtime-mappings"})
        )?
    );
    Ok(())
}
