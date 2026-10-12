//! Database developer commands; no operator JSON or native shell required.
use anyhow::{bail, Context, Result};
use axiom_database::developer::{self as db, Connection, Release, Session};
use axiom_lib::{
    database::{
        version::{suggest, Version},
        DatabaseContractIR,
    },
    package::*,
};
use clap::Args;
use std::{
    collections::{BTreeMap, HashMap},
    io::{IsTerminal, Write},
    path::{Path, PathBuf},
};

#[derive(Args, Clone, Default)]
pub struct ConnectionArgs {
    /// PostgreSQL or SQLite URL; defaults to DATABASE_URL when no endpoint is specified
    #[arg(long)]
    pub database_url: Option<String>,
    /// PostgreSQL host (HTTP host for backend run)
    #[arg(long)]
    pub host: Option<String>,
    /// PostgreSQL port, default 5432 (HTTP port for backend/frontend run)
    #[arg(short, long)]
    pub port: Option<u16>,
    #[arg(long)]
    pub database: Option<String>,
    #[arg(long)]
    pub username: Option<String>,
    /// Prefer PGPASSWORD or the masked interactive prompt
    #[arg(long)]
    pub password: Option<String>,
    #[arg(long, conflicts_with = "database_url")]
    pub database_file: Option<PathBuf>,
    #[arg(long, value_parser=["disable","prefer","require","verify-full"])]
    pub ssl_mode: Option<String>,
    #[arg(long)]
    pub tls_ca: Option<PathBuf>,
}
impl std::fmt::Debug for ConnectionArgs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DatabaseConnectionArguments([REDACTED])")
    }
}
#[derive(Args, Clone, Debug, Default)]
pub struct Policy {
    #[arg(long)]
    pub auto_version: bool,
    #[arg(long)]
    pub non_interactive: bool,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub allow_destructive: bool,
    #[arg(long)]
    pub allow_data_dependent: bool,
    #[arg(long)]
    pub allow_security_changes: bool,
    /// Enroll an existing scope only when it exactly matches the validated source
    #[arg(long)]
    pub adopt: bool,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub export: Option<PathBuf>,
}
#[derive(Args, Clone, Debug)]
pub struct MigrateArgs {
    #[arg(value_name = "CONTRACT")]
    pub source: PathBuf,
    #[command(flatten)]
    pub connection: ConnectionArgs,
    #[command(flatten)]
    pub policy: Policy,
    #[arg(long)]
    pub frozen: bool,
    #[arg(long)]
    pub to: Option<String>,
}
#[derive(Args, Clone, Debug)]
pub struct RollbackArgs {
    #[arg(value_name = "CONTRACT")]
    pub source: PathBuf,
    #[command(flatten)]
    pub connection: ConnectionArgs,
    #[command(flatten)]
    pub policy: Policy,
    #[arg(long, conflicts_with = "to")]
    pub steps: Option<usize>,
    #[arg(long)]
    pub to: Option<String>,
}
#[derive(Args, Clone, Debug)]
pub struct ReadArgs {
    #[arg(value_name = "CONTRACT")]
    pub source: PathBuf,
    #[command(flatten)]
    pub connection: ConnectionArgs,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub watch: bool,
    #[arg(long)]
    pub check: bool,
}
struct Loaded {
    root: PathBuf,
    compiled: acore::database::Compilation,
    input: acore::database::ProjectInput,
    immutable: bool,
    release: Option<Release>,
    bundle: Vec<Release>,
}
fn owning_root(path: &Path) -> PathBuf {
    let directory = path.parent().unwrap_or(Path::new("."));
    directory
        .ancestors()
        .find(|p| p.join("AxiomDeps.toml").is_file())
        .unwrap_or(directory)
        .to_path_buf()
}
fn load(path: &Path) -> Result<Loaded> {
    let source = path
        .canonicalize()
        .with_context(|| format!("read contract {}", path.display()))?;
    if path.extension().is_some_and(|e| e == "acore") {
        let (root, input) =
            acore::database::project::load(&source, &HashMap::new()).map_err(anyhow::Error::msg)?;
        let compiled = acore::database::compile(&input)
            .map_err(|e| anyhow::anyhow!(serde_json::to_string_pretty(&e).unwrap_or_default()))?;
        return Ok(Loaded {
            root,
            compiled,
            input,
            immutable: false,
            release: None,
            bundle: vec![],
        });
    }
    let root = owning_root(&source);
    if let Ok(bundle) = db::read_bundle(&source) {
        let release = bundle.releases.last().unwrap().clone();
        let input = input_from_release(&release)?;
        let compiled = acore::database::compile(&input)
            .map_err(|e| anyhow::anyhow!(serde_json::to_string_pretty(&e).unwrap_or_default()))?;
        return Ok(Loaded {
            root,
            compiled,
            input,
            immutable: true,
            release: Some(release),
            bundle: bundle.releases,
        });
    }
    if let Ok(release) = db::read_release(&source) {
        let input = input_from_release(&release)?;
        let compiled = acore::database::compile(&input)
            .map_err(|e| anyhow::anyhow!(serde_json::to_string_pretty(&e).unwrap_or_default()))?;
        return Ok(Loaded {
            root,
            compiled,
            input,
            immutable: true,
            release: Some(release),
            bundle: vec![],
        });
    }
    let DecodedAxiomPackage::Package(package) = decode_axiom_package(&std::fs::read(&source)?)?
    else {
        bail!("ADB1101: expected a database schema or migration release");
    };
    let contract = axiom_lib::database::contract_from_package(&package)?;
    let capsule = source.with_extension("sources.axiom");
    if capsule.is_file() {
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&capsule)?)?;
        if value["format"] != "axiom-database-source/v1"
            || value["sha256"] != axiom_database::digest(&std::fs::read(&source)?)
        {
            bail!("ADB1101: source capsule does not match the database artifact; rebuild it");
        }
        let input: acore::database::ProjectInput = serde_json::from_value(value["input"].clone())?;
        let compiled = acore::database::compile(&input)
            .map_err(|e| anyhow::anyhow!(serde_json::to_string_pretty(&e).unwrap_or_default()))?;
        if compiled.contract != contract || compiled.identity != package.package {
            bail!("ADB1101: artifact identity and source snapshot differ");
        }
        return Ok(Loaded {
            root,
            compiled,
            input,
            immutable: true,
            release: None,
            bundle: vec![],
        });
    }
    bail!("ADB1101: this schema-only artifact has no source capsule; rebuild with the current CLI or run the Acore source");
}
fn load_existing(path: &Path) -> Result<(Loaded, Option<String>)> {
    match load(path) {
        Ok(loaded) => Ok((loaded, None)),
        Err(error) => {
            let root = owning_root(path);
            let versions = db::archive_versions(&root)?;
            let Some(release) = versions.last() else {
                return Err(error);
            };
            let input = input_from_release(release)?;
            let compiled = acore::database::compile(&input)
                .map_err(|_| anyhow::anyhow!("ADB1101: archived source does not compile"))?;
            let diagnostic = format!(
                "Working source is invalid; inspecting the last archived definition. {error}"
            );
            Ok((
                Loaded {
                    root,
                    compiled,
                    input,
                    immutable: true,
                    release: Some(release.clone()),
                    bundle: vec![],
                },
                Some(diagnostic),
            ))
        }
    }
}
fn input_from_release(release: &Release) -> Result<acore::database::ProjectInput> {
    Ok(acore::database::ProjectInput {
        entry: release.revision.entry.clone(),
        files: release.revision.sources.clone(),
        packages: release.revision.packages.clone(),
    })
}
pub fn interactive(policy: &Policy) -> bool {
    !policy.non_interactive
        && !policy.json
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && !std::env::var("CI").is_ok_and(|v| !v.is_empty() && v != "false" && v != "0")
}
pub fn resolve(args: &ConnectionArgs, engine: &str, root: &Path) -> Result<Connection> {
    let endpoint_fields = args.host.is_some()
        || args.port.is_some()
        || args.database.is_some()
        || args.username.is_some();
    let url = args.database_url.clone().or_else(|| {
        if !endpoint_fields && args.database_file.is_none() {
            std::env::var("DATABASE_URL").ok()
        } else {
            None
        }
    });
    if engine == "sqlite" {
        if endpoint_fields
            || args.password.is_some()
            || args.ssl_mode.is_some()
            || args.tls_ca.is_some()
        {
            bail!("ADB1107: SQLite takes --database-file or a SQLite URL, not PostgreSQL connection fields");
        }
        let file = if let Some(file) = &args.database_file {
            file.clone()
        } else if let Some(url) = url {
            let Some(value) = url.strip_prefix("sqlite:") else {
                bail!(
                    "ADB1107: SQLite contract requires a SQLite connection, not a PostgreSQL URL"
                );
            };
            if value.trim_start_matches('/') == ":memory:" {
                bail!("ADB1107: this persistent migration profile requires a SQLite file");
            }
            let uri = if value.starts_with("//") {
                reqwest::Url::parse(&format!("file:{value}"))?
            } else if value.starts_with('/') {
                reqwest::Url::parse(&format!("file://{value}"))?
            } else {
                reqwest::Url::from_directory_path(std::env::current_dir()?)
                    .map_err(|_| anyhow::anyhow!("ADB1107: SQLite working directory unavailable"))?
                    .join(value)?
            };
            if uri.query().is_some() || uri.fragment().is_some() {
                bail!("ADB1107: this persistent profile accepts a SQLite file URI without query options");
            }
            uri.to_file_path()
                .map_err(|_| anyhow::anyhow!("ADB1107: SQLite URI must identify a local persistent file"))?
        } else {
            root.join(".axiom/database/data/database.sqlite")
        };
        return Ok(Connection::Sqlite { file });
    }
    if args.database_file.is_some() {
        bail!("ADB1107: PostgreSQL contracts cannot use --database-file");
    }
    let mut url = if let Some(url) = url {
        let parsed = reqwest::Url::parse(&url).context("ADB1107: invalid PostgreSQL URL")?;
        if !["postgres", "postgresql"].contains(&parsed.scheme()) {
            bail!("ADB1107: PostgreSQL contract requires postgres:// or postgresql://");
        }
        if args.database_url.is_some() && endpoint_fields {
            bail!("ADB1107: provide a URL or individual endpoint fields, not both");
        }
        parsed
    } else {
        let database = args
            .database
            .clone()
            .or_else(|| std::env::var("PGDATABASE").ok())
            .context("ADB1107: supply --database-url or --database and connection fields")?;
        let mut url = reqwest::Url::parse("postgresql://localhost")?;
        url.set_host(Some(args.host.as_deref().unwrap_or("localhost")))
            .map_err(|_| anyhow::anyhow!("ADB1107: invalid host"))?;
        url.set_port(Some(args.port.unwrap_or(5432)))
            .map_err(|_| anyhow::anyhow!("ADB1107: invalid port"))?;
        url.set_path(&database);
        let user = args
            .username
            .clone()
            .or_else(|| std::env::var("PGUSER").ok())
            .or_else(|| std::env::var("USER").ok())
            .context("ADB1107: supply --username")?;
        url.set_username(&user)
            .map_err(|_| anyhow::anyhow!("ADB1107: invalid username"))?;
        url
    };
    if let Some(password) = args.password.clone().or_else(|| {
        if url.password().is_none() { std::env::var("PGPASSWORD").ok() } else { None }
    })
    {
        url.set_password(Some(&password))
            .map_err(|_| anyhow::anyhow!("ADB1107: invalid password input"))?;
    }
    if let Some(mode) = &args.ssl_mode {
        url.query_pairs_mut().append_pair("sslmode", mode);
    }
    Ok(Connection::Postgres {
        url: url.to_string(),
        ca: args.tls_ca.clone(),
    })
}
async fn open(connection: &mut Connection, writable: bool, policy: &Policy) -> Result<Session> {
    match db::connect(connection, writable).await {
        Ok(session) => Ok(session),
        Err(error) if error.code == "ADB1110" && interactive(policy) => {
            let password = dialoguer::Password::new()
                .with_prompt("PostgreSQL password")
                .interact()?;
            if let Connection::Postgres { url, .. } = connection {
                let mut parsed = reqwest::Url::parse(url)?;
                parsed
                    .set_password(Some(&password))
                    .map_err(|_| anyhow::anyhow!("invalid password input"))?;
                *url = parsed.to_string();
            }
            Ok(db::connect(connection, writable).await?)
        }
        Err(error) => Err(error.into()),
    }
}
fn show_preview(version: &str, preview: &axiom_lib::database::migration::Preview) {
    println!(
        "Migration {version}{}",
        if preview.destructive {
            " · BREAKING / possible data loss"
        } else {
            ""
        }
    );
    for change in &preview.changes {
        println!(
            "  {} {}{}",
            change.operation,
            change.path,
            if change.changed_properties.is_empty() {
                String::new()
            } else {
                format!(" ({})", change.changed_properties.join(", "))
            }
        );
    }
    if preview.changes.is_empty() {
        println!("  Metadata only; no schema changes.");
    }
}
fn approve(policy: &Policy, preview: &axiom_lib::database::migration::Preview) -> Result<()> {
    for (needed,allowed,flag,message) in [(preview.destructive,policy.allow_destructive,"--allow-destructive","This change may delete data or remove invariants; lost values are not restored by a schema rollback."),(preview.data_dependent,policy.allow_data_dependent,"--allow-data-dependent","This change depends on populated data or rebuilds existing invariants."),(preview.security,policy.allow_security_changes,"--allow-security-changes","This change alters database permissions or security policy.")] {
        if !needed||allowed {continue;}
        if !interactive(policy) { bail!("ADB1111: {message} Authorize this exact migration with {flag} in noninteractive mode."); }
        if !dialoguer::Confirm::new().with_prompt(format!("{message} Apply?" )).default(false).interact()? { bail!("Migration cancelled; target and source were not changed."); }
    }
    Ok(())
}
fn updated_input(
    input: &acore::database::ProjectInput,
    version: &str,
) -> Result<acore::database::ProjectInput> {
    let mut input = input.clone();
    let source = input
        .files
        .get_mut(&input.entry)
        .context("ADB1100: source metadata is required for version changes")?;
    *source =
        acore::database::identity::replace_version(source, version).map_err(anyhow::Error::msg)?;
    if let Some(manifest) = input.files.get_mut("AxiomDeps.toml") {
        let mut document = manifest.parse::<toml_edit::DocumentMut>()?;
        document.remove("application");
        *manifest = document.to_string();
    }
    Ok(input)
}
fn save_input(
    root: &Path,
    before: &acore::database::ProjectInput,
    after: &acore::database::ProjectInput,
) -> Result<()> {
    let edits = after
        .files
        .iter()
        .filter(|(name, value)| before.files.get(*name) != Some(*value))
        .collect::<Vec<_>>();
    for (name, _) in &edits {
        if std::fs::read_to_string(root.join(name))? != before.files[*name] {
            bail!(
                "ADB1104: source changed while preparing the migration; no version edit was saved"
            );
        }
    }
    let directory = root.join(".axiom/database");
    std::fs::create_dir_all(&directory)?;
    let journal = directory.join("source-edit.json");
    private_write(
        &journal,
        &serde_json::to_vec(
            &serde_json::json!({"format":"axiom-source-edit/v1","before":before.files,"after":after.files}),
        )?,
    )?;
    for (name, value) in edits {
        let path = root.join(name);
        let mut staged = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
        staged.write_all(value.as_bytes())?;
        staged.as_file().sync_all()?;
        staged.persist(path)?;
    }
    std::fs::remove_file(journal)?;
    Ok(())
}
fn strip_migration(input: &mut acore::database::ProjectInput) -> Result<()> {
    let original = input
        .files
        .get(&input.entry)
        .context("database source unavailable")?
        .clone();
    let mut source = original.clone();
    let ds = acore::database::syntax::parse(&original)?;
    for d in ds.into_iter().rev().filter(|d| d.kind == "migration") {
        if let Some(acore::database::syntax::Node {
            value: acore::database::syntax::Value::String(target),
            ..
        }) = d
            .args
            .iter()
            .find(|a| a.name.as_deref() == Some("source"))
            .map(|a| &a.value)
        {
            if let Ok(path) = acore::database::project::relative_path(&input.entry, target) {
                input.files.remove(&path);
            }
        }
        source.replace_range(d.span, "");
    }
    input.files.insert(input.entry.clone(), source);
    Ok(())
}
fn recover_source_edit(root: &Path) -> Result<()> {
    let journal = root.join(".axiom/database/source-edit.json");
    if !journal.exists() {
        return Ok(());
    }
    let data = std::fs::read(&journal)?;
    if data.len() > 16_777_216 {
        bail!("ADB1104: source recovery journal exceeds bounds");
    }
    let value: serde_json::Value = serde_json::from_slice(&data)?;
    if value["format"] != "axiom-source-edit/v1" {
        bail!("ADB1104: unsupported source recovery journal");
    }
    let before: BTreeMap<String, String> = serde_json::from_value(value["before"].clone())?;
    let after: BTreeMap<String, String> = serde_json::from_value(value["after"].clone())?;
    let edits = after
        .iter()
        .filter(|(name, text)| before.get(*name) != Some(*text))
        .collect::<Vec<_>>();
    for (name, text) in &edits {
        if !acore::database::project::validate_path(name) {
            bail!("ADB1104: invalid recovery path");
        }
        let current = std::fs::read_to_string(root.join(name))?;
        if current != **text && before.get(*name) != Some(&current) {
            bail!("ADB1104: source changed after an interrupted version edit; preserve your edits and resolve the private source journal before retrying");
        }
    }
    for (name, text) in edits {
        let path = root.join(name);
        let mut staged = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
        staged.write_all(text.as_bytes())?;
        staged.as_file().sync_all()?;
        staged.persist(path)?;
    }
    std::fs::remove_file(journal)?;
    Ok(())
}
fn known(root: &Path, history: &[Release]) -> Result<Vec<Release>> {
    let mut all = db::archive_versions(root)?;
    for release in history {
        if all
            .iter()
            .any(|r| r.revision.identity == release.revision.identity && r.sha256 != release.sha256)
        {
            bail!("ADB1103: local and target catalogs bind the same version to different content");
        }
        if !all.iter().any(|r| r.sha256 == release.sha256) {
            db::archive(root, release)?;
            all.push(release.clone());
        }
    }
    Ok(all)
}
fn prefix(head: &Release, catalog: &[Release], applied: Option<&str>) -> Result<Vec<Release>> {
    let mut pending = vec![];
    let mut current = head.clone();
    let mut visited = std::collections::BTreeSet::new();
    loop {
        if Some(current.sha256.as_str()) == applied {
            break;
        }
        if !visited.insert(current.sha256.clone()) {
            bail!("ADB1101: cyclic migration catalog");
        }
        pending.push(current.clone());
        if current.revision.parent.as_deref() == applied {
            break;
        }
        if let Some(parent) = &current.revision.parent {
            current=catalog.iter().find(|r|&r.sha256==parent).cloned().context("ADB1104: complete migration prefix is missing; use an exported bundle or restore the committed catalog")?;
        } else {
            bail!("ADB1104: requested migration is not a forward descendant; use rollback to restore a previous state");
        }
    }
    pending.reverse();
    Ok(pending)
}
fn published_parent<'a>(
    catalog: &'a [Release],
    applied: Option<&'a Release>,
    project: &str,
) -> Result<Option<&'a Release>> {
    let tips = catalog
        .iter()
        .filter(|r| {
            r.revision.identity.name == project
                && !catalog
                    .iter()
                    .any(|child| child.revision.parent.as_deref() == Some(&r.sha256))
        })
        .collect::<Vec<_>>();
    if tips.is_empty() {
        return Ok(applied);
    }
    if tips.len() > 1 {
        bail!("ADB1104: migration catalog has divergent branches; select and reconcile the intended parent before publishing");
    }
    prefix(tips[0], catalog, applied.map(|r| r.sha256.as_str()))?;
    Ok(Some(tips[0]))
}

pub async fn migrate(args: &MigrateArgs, shell: bool, execute: Option<&str>) -> Result<()> {
    let mut loaded = load(&args.source)?;
    let mut connection = resolve(
        &args.connection,
        &loaded.compiled.contract.engine,
        &loaded.root,
    )?;
    if args.policy.dry_run {
        if let Connection::Sqlite { file } = &connection {
            if !file.exists() {
                let mut catalog = db::archive_versions(&loaded.root)?;
                catalog.extend(loaded.bundle.clone());
                let digest = db::content_digest(
                    &loaded.compiled.contract,
                    &loaded.compiled.definitions,
                    loaded.compiled.migration_source.as_deref(),
                )?;
                if let Some(release) = loaded.release.as_ref().or_else(|| {
                    catalog.iter().find(|r| {
                        r.revision.identity == loaded.compiled.identity
                            && r.revision.semantic_digest().ok().as_deref() == Some(&digest)
                    })
                }) {
                    if args.policy.json {
                        println!(
                            "{}",
                            serde_json::json!({"format":"axiom-database-migration-preview/v1","version":release.revision.identity.version,"preview":release.revision.preview,"targetCreated":false})
                        );
                    } else {
                        show_preview(
                            &release.revision.identity.version,
                            &release.revision.preview,
                        );
                        println!("Dry run; the missing SQLite target was not created.");
                    }
                    return Ok(());
                }
                if args.frozen {
                    bail!("ADB1103: frozen mode requires a prepared immutable migration release");
                }
                let parent = published_parent(&catalog, None, &loaded.compiled.identity.name)?;
                let preview = axiom_lib::database::migration::preview(
                    parent.map(|p| &p.revision.desired),
                    &loaded.compiled.contract,
                )?;
                if parent.is_some_and(|p| {
                    loaded.compiled.identity.version.parse::<Version>().ok()
                        <= p.revision.identity.version.parse::<Version>().ok()
                }) {
                    let version = suggest(
                        &loaded.compiled.identity.version,
                        catalog.iter().map(|r| r.revision.identity.version.as_str()),
                        preview.destructive,
                    )?;
                    loaded.input = updated_input(&loaded.input, &version)?;
                    loaded.compiled = acore::database::compile(&loaded.input).map_err(|e| {
                        anyhow::anyhow!(serde_json::to_string(&e).unwrap_or_default())
                    })?;
                }
                let revision = db::revision(
                    &loaded.compiled,
                    loaded.input.clone(),
                    parent,
                    "migration",
                    None,
                )?;
                if args.policy.json {
                    println!(
                        "{}",
                        serde_json::json!({"format":"axiom-database-migration-preview/v1","version":revision.identity.version,"preview":revision.preview,"targetCreated":false})
                    );
                } else {
                    show_preview(&revision.identity.version, &revision.preview);
                    println!("Dry run; the missing SQLite target was not created.");
                }
                return Ok(());
            }
        }
    }
    let session = open(&mut connection, !args.policy.dry_run, &args.policy).await?;
    if !args.policy.json {
        println!("Connected to {}", session.label().await?);
    }
    let _lock = db::CatalogLock::acquire(&loaded.root)?;
    if !args.policy.dry_run && args.source.extension().is_some_and(|e| e == "acore") {
        axiom_lib::project_state::import_legacy(&args.source)?;
    }
    recover_source_edit(&loaded.root)?;
    loaded = load(&args.source)?;
    let status = session.status_compilation(&loaded.compiled).await?;
    if status.drift {
        bail!("ADB1105: database drift detected; inspect status before applying another migration");
    }
    let mut catalog = known(&loaded.root, &status.releases)?;
    for release in &loaded.bundle {
        if !catalog.iter().any(|r| r.sha256 == release.sha256) {
            catalog.push(release.clone());
        }
    }
    if catalog
        .iter()
        .any(|r| r.revision.identity.name != loaded.compiled.identity.name)
    {
        bail!("ADB1101: this hidden catalog belongs to another contract ID; keep the ID stable or use a separate database project");
    }
    let original = loaded.input.clone();
    let mut selected = loaded.release.clone();
    if let Some(version) = &args.to {
        selected = Some(
            catalog
                .iter()
                .find(|r| r.revision.identity.version == *version)
                .cloned()
                .with_context(|| {
                    format!("ADB1102: version {version} is not in the migration catalog")
                })?,
        );
    }
    let desired_hash = db::content_digest(
        &loaded.compiled.contract,
        &loaded.compiled.definitions,
        loaded.compiled.migration_source.as_deref(),
    )?;
    if selected.is_none() {
        if let Some(existing) = catalog.iter().find(|r| {
            r.revision.identity.version == loaded.compiled.identity.version
                && r.revision.semantic_digest().ok().as_deref() == Some(&desired_hash)
        }) {
            if prefix(
                existing,
                &catalog,
                status.releases.last().map(|r| r.sha256.as_str()),
            )
            .is_ok()
            {
                selected = Some(existing.clone());
            }
        }
    }
    if selected.is_none() {
        if args.frozen {
            bail!("ADB1103: frozen mode requires an existing immutable migration release; prepare it from source first");
        }
        if !loaded.immutable {
            acore::database::identity::project(&loaded.input.files[&loaded.input.entry])
                .map_err(anyhow::Error::msg)?;
        }
        let collision = catalog.iter().any(|r| {
            loaded.compiled.identity.version.parse::<Version>().unwrap()
                <= r.revision.identity.version.parse::<Version>().unwrap()
        });
        if collision {
            let version = suggest(
                &loaded.compiled.identity.version,
                catalog.iter().map(|r| r.revision.identity.version.as_str()),
                status.preview.destructive,
            )?;
            if args.policy.dry_run {
                if args.policy.json {
                    println!(
                        "{}",
                        serde_json::json!({"suggestedVersion":version,"preview":status.preview})
                    );
                } else {
                    println!(
                        "Contract version {} was already used; suggested version: {version}",
                        loaded.compiled.identity.version
                    );
                    show_preview(&version, &status.preview);
                }
                return Ok(());
            }
            if loaded.immutable {
                bail!("ADB1103: version already used; prepare version {version} from Acore source, then rebuild the immutable artifact");
            }
            let version = if args.policy.auto_version {
                version
            } else if interactive(&args.policy) {
                dialoguer::Input::<String>::new()
                    .with_prompt(format!(
                        "Contract version {} was already used. Use next version",
                        loaded.compiled.identity.version
                    ))
                    .default(version)
                    .interact_text()?
            } else {
                bail!("ADB1103: version {} was already used. Suggested version: {version}. Update project(version: …) or use --auto-version.",loaded.compiled.identity.version);
            };
            loaded.input = updated_input(&loaded.input, &version)?;
            loaded.compiled = acore::database::compile(&loaded.input).map_err(|e| {
                anyhow::anyhow!(serde_json::to_string_pretty(&e).unwrap_or_default())
            })?;
            if catalog
                .iter()
                .any(|r| r.revision.identity.version == version)
            {
                bail!("ADB1103: selected version {version} is already published");
            }
        }
        let parent = published_parent(
            &catalog,
            status.releases.last(),
            &loaded.compiled.identity.name,
        )?;
        let revision = db::revision(
            &loaded.compiled,
            loaded.input.clone(),
            parent,
            if args.policy.adopt {
                "adoption"
            } else {
                "migration"
            },
            None,
        )?;
        if args.policy.dry_run {
            if args.policy.json {
                println!("{}", serde_json::to_string(&revision.preview)?);
            } else {
                show_preview(&revision.identity.version, &revision.preview);
            }
            return Ok(());
        }
        let signed = db::sign(revision)?;
        if parent.is_some_and(|p| p.public_key != signed.public_key) {
            bail!("ADB1109: signing provider differs from the published history; restore its key before preparing another version");
        }
        selected = Some(signed);
    }
    let release = selected.unwrap();
    if let Ok(trusted) = std::env::var("AXIOM_DATABASE_TRUSTED_KEY") {
        if trusted != release.public_key {
            bail!("ADB1109: release signer differs from the configured trusted key");
        }
    }
    if status
        .releases
        .last()
        .is_some_and(|r| r.sha256 == release.sha256)
    {
        if args.policy.json {
            println!(
                "{}",
                serde_json::json!({"format":"axiom-database-migration-result/v1","version":release.revision.identity.version,"status":"current","applied":false})
            );
        } else {
            println!(
                "Database is current · contract {}",
                release.revision.identity.version
            );
        }
    } else {
        if !args.policy.json {
            show_preview(
                &release.revision.identity.version,
                &release.revision.preview,
            );
            println!("Validating migration internally…");
        }
        if args.policy.dry_run {
            return Ok(());
        }
        let pending = prefix(
            &release,
            &catalog,
            status.releases.last().map(|r| r.sha256.as_str()),
        )?;
        for candidate in &pending {
            approve(&args.policy, &candidate.revision.preview)?;
        }
        for (index, candidate) in pending.iter().enumerate() {
            let fingerprint = match session.validate(&candidate.revision).await {
                Ok(value) => value,
                Err(error) => {
                    let _ = session
                        .record_attempt(
                            &candidate.revision.identity.name,
                            &candidate.revision.identity.version,
                            "validation-failed",
                            &error.code,
                        )
                        .await;
                    private_write(
                        &loaded.root.join(".axiom/database/last-attempt.json"),
                        &serde_json::to_vec(
                            &serde_json::json!({"format":"axiom-database-migration-result/v1","version":candidate.revision.identity.version,"outcome":"validation-failed","code":error.code}),
                        )?,
                    )?;
                    return Err(error.into());
                }
            };
            let path = db::archive(&loaded.root, candidate)?;
            if index + 1 == pending.len()
                && loaded.input.files != original.files
                && !loaded.immutable
            {
                save_input(&loaded.root, &original, &loaded.input)?;
            }
            let target = session
                .apply(candidate, args.policy.adopt, &fingerprint)
                .await?;
            if args.policy.json {
                println!(
                    "{}",
                    serde_json::json!({"format":"axiom-database-migration-result/v1","version":candidate.revision.identity.version,"status":"applied","head":target.head,"release":path})
                );
            } else {
                println!(
                    "Applied migration {} · database verified",
                    candidate.revision.identity.version
                );
            }
        }
    }
    if let Some(path) = &args.policy.export {
        let (_, history) = session.history(&loaded.compiled.identity.name).await?;
        db::write_bundle(path, history)?;
    }
    // Publication is complete. A long-running SQL session must not prevent
    // migration preparation in another terminal.
    drop(_lock);
    if let Some(sql) = execute {
        execute_sql_mode(&session, sql, args.policy.json).await?;
    }
    if shell && !args.policy.json && !args.policy.dry_run {
        console(&session).await?;
    }
    Ok(())
}

pub async fn rollback(args: &RollbackArgs) -> Result<()> {
    let (loaded, diagnostic) = load_existing(&args.source)?;
    if let Some(diagnostic) = diagnostic {
        eprintln!("{diagnostic}");
    }
    let mut connection = resolve(
        &args.connection,
        &loaded.compiled.contract.engine,
        &loaded.root,
    )?;
    let session = open(&mut connection, !args.policy.dry_run, &args.policy).await?;
    let _lock = db::CatalogLock::acquire(&loaded.root)?;
    let status = session.status_compilation(&loaded.compiled).await?;
    if status.drift {
        bail!("ADB1105: rollback refused because the database has drift");
    }
    let history = &status.releases;
    let current = history
        .last()
        .context("ADB1102: database has no applied migrations")?;
    let restored = if args.to.as_deref() == Some("empty") {
        None
    } else if let Some(version) = &args.to {
        Some(
            history
                .iter()
                .find(|r| r.revision.identity.version == *version)
                .context("ADB1102: requested rollback version was not applied")?,
        )
    } else {
        let steps = args.steps.unwrap_or(1);
        if steps == 0 || steps > history.len() {
            bail!("ADB1102: rollback steps must be between one and the number of applied versions");
        }
        if steps == history.len() {
            None
        } else {
            Some(&history[history.len() - 1 - steps])
        }
    };
    let requested = restored.map_or("empty", |r| r.revision.identity.version.as_str());
    if restored.is_some_and(|r| r.sha256 == current.sha256)
        || (current.revision.kind == "rollback"
            && current.revision.restores.as_deref() == Some(requested))
    {
        if args.policy.json {
            println!("{}", serde_json::json!({"format":"axiom-database-migration-result/v1",
                "version":current.revision.identity.version,"status":"current",
                "applied":false,"restores":requested}));
        } else {
            println!("Database is already at the requested schema state.");
        }
        return Ok(());
    }
    let restored_index = restored.and_then(|r| history.iter().position(|h| h.sha256 == r.sha256));
    for release in &history[restored_index.map_or(0, |i| i + 1)..] {
        if release.revision.evolution.as_ref().is_some_and(|e| {
            e.backfills.iter().any(|b| {
                release.revision.base.as_ref().is_some_and(|base| {
                    base.schemas
                        .iter()
                        .flat_map(|s| &s.tables)
                        .flat_map(|t| &t.columns)
                        .any(|c| c.id == b.column_id)
                })
            })
        }) {
            bail!("ADB1113: rollback crosses a data overwrite whose previous values were not archived; define an explicit compensating migration or restore a backup");
        }
    }
    let catalog = known(&loaded.root, history)?;
    let restored_version = restored.map_or("empty", |r| r.revision.identity.version.as_str());
    let mut input = if let Some(restored) = restored {
        let mut i = input_from_release(restored)?;
        strip_migration(&mut i)?;
        i
    } else {
        let mut input = input_from_release(&history[0])?;
        let schema = &history[0].revision.desired;
        let schemas = schema
            .schemas
            .iter()
            .map(|s| {
                if schema.engine == "sqlite" {
                    format!("schema {} {{}}", s.name)
                } else {
                    format!(
                        "schema {}(sqlName: {}) {{}}",
                        s.name,
                        serde_json::to_string(&s.sql_name).unwrap()
                    )
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        input
            .files
            .retain(|name, _| !name.ends_with(".acore") || *name == input.entry);
        let manage = schema
            .schemas
            .iter()
            .map(|s| format!("Schemas.{}", s.name))
            .collect::<Vec<_>>()
            .join(", ");
        input.files.insert(input.entry.clone(),format!("project(id: {}, version: {})\nmodule rollback.baseline\ndatabase {}(engine: DatabaseEngine.{}) {{\nmanage(schemas: [{manage}], mode: ManagementMode.ownedObjects)\n{schemas}\n}}\n",serde_json::to_string(&current.revision.identity.name)?,serde_json::to_string(&current.revision.identity.version)?,schema.database,schema.engine));
        input
    };
    let baseline = acore::database::compile(&input)
        .map_err(|e| anyhow::anyhow!(serde_json::to_string_pretty(&e).unwrap_or_default()))?;
    let preview = axiom_lib::database::migration::preview(
        Some(&current.revision.desired),
        &baseline.contract,
    )?;
    let suggested = suggest(
        &current.revision.identity.version,
        catalog.iter().map(|r| r.revision.identity.version.as_str()),
        preview.destructive,
    )?;
    let version = if args.policy.auto_version || args.policy.dry_run {
        suggested
    } else if interactive(&args.policy) {
        dialoguer::Input::<String>::new()
            .with_prompt("New rollback contract version")
            .default(suggested)
            .interact_text()?
    } else {
        bail!("ADB1103: rollback publishes a new version; use --auto-version in CI (suggested {suggested})");
    };
    if catalog
        .iter()
        .any(|r| r.revision.identity.version == version)
    {
        bail!("ADB1103: rollback version {version} is already used");
    }
    input = updated_input(&input, &version)?;
    let compiled = acore::database::compile(&input)
        .map_err(|e| anyhow::anyhow!(serde_json::to_string_pretty(&e).unwrap_or_default()))?;
    let revision = db::revision(
        &compiled,
        input.clone(),
        Some(current),
        "rollback",
        Some(restored_version.to_string()),
    )?;
    if !args.policy.json {
        show_preview(&version, &revision.preview);
        println!("Restores schema state of {restored_version}; working source is preserved.");
    }
    if args.policy.dry_run {
        if args.policy.json {
            println!("{}", serde_json::to_string(&revision.preview)?);
        }
        return Ok(());
    }
    let fingerprint = session.validate(&revision).await?;
    approve(&args.policy, &revision.preview)?;
    let release = db::sign(revision)?;
    if release.public_key != current.public_key {
        bail!("ADB1109: restore the enrolled signing provider before preparing a rollback");
    }
    db::archive(&loaded.root, &release)?;
    let target = session.apply(&release, false, &fingerprint).await?;
    if args.policy.json {
        println!(
            "{}",
            serde_json::json!({"format":"axiom-database-migration-result/v1","version":version,"restores":release.revision.restores,"status":"applied","head":target.head})
        );
    } else {
        println!("Applied rollback {version}; previous migration history is preserved.");
    }
    if let Some(path) = &args.policy.export {
        let (_, history) = session.history(&loaded.compiled.identity.name).await?;
        db::write_bundle(path, history)?;
    }
    Ok(())
}
pub async fn read(args: &ReadArgs, history: bool) -> Result<()> {
    loop {
        let (loaded, diagnostic) = load_existing(&args.source)?;
        let mut connection = resolve(
            &args.connection,
            &loaded.compiled.contract.engine,
            &loaded.root,
        )?;
        let session = open(
            &mut connection,
            false,
            &Policy {
                non_interactive: !std::io::stdin().is_terminal(),
                json: args.json,
                ..Default::default()
            },
        )
        .await?;
        let mut status = session.status_compilation(&loaded.compiled).await?;
        if let Some(diagnostic) = diagnostic {
            eprintln!("{diagnostic}");
            status.pending = true;
            status.working_version = "invalid".into();
        }
        if history {
            let _ = known(&loaded.root, &status.releases);
        }
        if args.json {
            println!("{}", serde_json::to_string(&status)?);
        } else {
            println!("Contract {} · {}", status.project, status.engine);
            println!(
                "Applied: {} · Working: {} · Drift: {}",
                status
                    .applied_version
                    .as_deref()
                    .unwrap_or("not initialized"),
                status.working_version,
                if status.drift { "detected" } else { "none" }
            );
            if history {
                for release in &status.releases {
                    println!(
                        "{}  {}  {}{}",
                        release.revision.identity.version,
                        release.revision.kind,
                        release
                            .revision
                            .preview
                            .changes
                            .iter()
                            .map(|c| format!("{} {}", c.operation, c.path))
                            .collect::<Vec<_>>()
                            .join("; "),
                        release
                            .revision
                            .restores
                            .as_ref()
                            .map_or(String::new(), |v| format!(" · restores {v}"))
                    );
                }
            } else if status.pending {
                show_preview(&status.working_version, &status.preview);
            } else {
                println!("Database matches the contract.");
            }
        }
        if history && !args.json {
            for attempt in &status.attempts {
                if attempt.outcome != "applied" {
                    println!(
                        "{}  {}  {}",
                        attempt.version, attempt.outcome, attempt.detail
                    );
                }
            }
        }
        if args.check && (status.drift || status.pending) {
            bail!("ADB1105: database is not current");
        }
        if !args.watch {
            return Ok(());
        }
        tokio::select! {_=tokio::time::sleep(std::time::Duration::from_secs(2))=>{},_=tokio::signal::ctrl_c()=>return Ok(())}
    }
}
pub fn diff(source: &Path, from: &str, to: &str, json: bool) -> Result<()> {
    let loaded = if from == "working" || to == "working" {
        load(source)?
    } else {
        load_existing(source)?.0
    };
    let versions = db::archive_versions(&loaded.root)?;
    let get = |version: &str| -> Result<(DatabaseContractIR, Option<String>)> {
        if version == "working" {
            Ok((loaded.compiled.contract.clone(), loaded.compiled.migration_source.clone()))
        } else {
            versions
                .iter()
                .find(|r| r.revision.identity.version == version)
                .map(|r| (r.revision.desired.clone(), r.revision.evolution.as_ref().map(|e| e.source.clone())))
                .with_context(|| format!("ADB1102: version {version} is not in the local catalog"))
        }
    };
    let before = get(from)?;
    let after = get(to)?;
    let preview = db::comparison(&before.0, &after.0, before.1.as_deref(), after.1.as_deref())?;
    if json {
        println!("{}", serde_json::to_string(&preview)?);
    } else {
        println!("{from} → {to}");
        show_preview(to, &preview);
    }
    Ok(())
}
pub async fn run(
    source: &Path,
    connection: &ConnectionArgs,
    policy: &Policy,
    frozen: bool,
    no_shell: bool,
    execute: Option<&str>,
) -> Result<()> {
    migrate(
        &MigrateArgs {
            source: source.into(),
            connection: connection.clone(),
            policy: policy.clone(),
            frozen,
            to: None,
        },
        !no_shell,
        execute,
    )
    .await
}
pub fn has_storage(source: &Path) -> bool {
    fn visit(source: &Path, seen: &mut std::collections::BTreeSet<PathBuf>) -> bool {
        if seen.len() > 64 || !seen.insert(source.to_path_buf()) {
            return false;
        }
        let Some(ds) = std::fs::read_to_string(source)
            .ok()
            .and_then(|s| acore::backend::syntax::parse(&s).ok())
        else {
            return false;
        };
        ds.iter().any(|d| {
            d.kind == "storage" && d.args.iter().any(|a| a.name.as_deref() == Some("resource"))
        }) || ds
            .iter()
            .filter(|d| matches!(d.kind.as_str(), "amends" | "import"))
            .any(|d| match d.name.as_ref().map(|n| &n.value) {
                Some(acore::backend::syntax::Value::String(path)) if path.ends_with(".acore") => {
                    visit(&source.parent().unwrap_or(Path::new(".")).join(path), seen)
                }
                _ => false,
            })
    }
    visit(source, &mut Default::default())
}
pub async fn run_backend(
    source: &Path,
    args: &ConnectionArgs,
    policy: &Policy,
    variant: Option<&str>,
    host: &str,
    port: u16,
    debug: bool,
) -> Result<()> {
    use axiom_lib::backend::*;
    let source = source.canonicalize()?;
    let root = owning_root(&source);
    let output = root.join(".axiom/contracts/backend.axiom");
    super::backend::build_to(&source, variant.unwrap_or("default"), None, None, &output).await?;
    let plan_path = output.with_extension("server.json");
    let mut plan: ServerPlan = serde_json::from_slice(&std::fs::read(&plan_path)?)?;
    let mut endpoint_args = args.clone();
    endpoint_args.host = None;
    endpoint_args.port = None;
    let mut bindings = BTreeMap::new();
    let mut grants = std::collections::BTreeSet::new();
    for (name, storage) in &plan.storage {
        let Some(resource) = &storage.resource else {
            continue;
        };
        let schema_path = source.parent().unwrap().join(&storage.schema);
        migrate(
            &MigrateArgs {
                source: schema_path.clone(),
                connection: endpoint_args.clone(),
                policy: policy.clone(),
                frozen: false,
                to: None,
            },
            false,
            None,
        )
        .await?;
        let loaded = load(&schema_path)?;
        let operator = resolve(
            &endpoint_args,
            &loaded.compiled.contract.engine,
            &loaded.root,
        )?;
        let mut application = operator.clone();
        if let Connection::Postgres { url, .. } = &mut application {
            let role=axiom_lib::database::postgres::objects(&loaded.compiled.contract).into_iter().find_map(|object|{
                if let axiom_lib::database::postgres::Definition::Role{managed:true,login:true,credential_ref:Some(slot),..}=&object.definition{Some((object.sql_name.clone(),slot.clone()))}else{None}
            }).context("database-backed PostgreSQL requires a declared managed application login role with credentialRef")?;
            let password =
                db::application_password(&loaded.compiled.identity.name, &role.0, &role.1)?;
            let mut parsed = reqwest::Url::parse(url)?;
            parsed
                .set_username(&role.0)
                .map_err(|_| anyhow::anyhow!("invalid application role"))?;
            parsed
                .set_password(Some(&password))
                .map_err(|_| anyhow::anyhow!("invalid credential"))?;
            *url = parsed.to_string();
        }
        let runtime = axiom_lib::database::runtime::RuntimePlan {
            format: axiom_lib::database::runtime::FORMAT.into(),
            profile: axiom_lib::database::runtime::DEVELOPMENT_PROFILE.into(),
            schema: loaded.compiled.contract.clone(),
            source_origins: Default::default(),
            tenant_claim: storage.tenant_claim.clone(),
            tenant_column: storage.tenant_column.clone(),
            operations: storage.operations.clone(),
        };
        plan.persistence.insert(resource.clone(), runtime);
        let binding_path = root.join(format!(".axiom/database/{name}.binding.json"));
        let binding = axiom_server::persistence_development::Binding {
            format: "axiom-database-runtime-binding/v2".into(),
            identity: loaded.compiled.identity,
            operator,
            application,
        };
        private_write(&binding_path, &serde_json::to_vec(&binding)?)?;
        let variable = format!("AXIOM_DATABASE_BINDING_{}", name.to_ascii_uppercase());
        std::env::set_var(&variable, &binding_path);
        let interface = &plan.resources[resource];
        bindings.insert(
            resource.clone(),
            ProviderBinding {
                provider: "database".into(),
                interface_sha256: interface.interface_hash()?,
                configuration: BTreeMap::from([("binding".into(), variable)]),
                configuration_sha256: None,
                secrets: Default::default(),
            },
        );
        for operation in interface.operations.keys() {
            grants.insert(format!("{resource}.{operation}"));
        }
    }
    if bindings.len() != plan.resources.len() {
        bail!("automatic database development only binds declared storage resources; other providers need explicit deployment configuration");
    }
    plan.required_capabilities
        .insert(Capability::DatabasePersistence);
    private_write(&plan_path, &serde_json::to_vec(&plan)?)?;
    let token_path = root.join(".axiom/database/development.key");
    let token = if token_path.exists() {
        std::fs::read_to_string(&token_path)?
    } else {
        let token = format!("axiom_dev_{}", uuid::Uuid::new_v4().simple());
        private_write(&token_path, token.as_bytes())?;
        token
    };
    let registry = root.join(".axiom/database/identities.json");
    let principal = Principal {
        subject: "local-developer".into(),
        tenant: Some("demo".into()),
        roles: Default::default(),
        permissions: Default::default(),
        claims: Default::default(),
    };
    let record = CredentialRecord {
        principal,
        expires_unix_seconds: 4_102_444_800,
        csrf_sha256: None,
    };
    private_write(
        &registry,
        &serde_json::to_vec(&BTreeMap::from([(
            axiom_database::digest(token.as_bytes()),
            record,
        )]))?,
    )?;
    std::env::set_var("AXIOM_DATABASE_DEV_IDENTITIES", &registry);
    let deployment = Deployment {
        format: DEPLOYMENT_FORMAT.into(),
        plan_sha256: plan.sha256()?,
        capabilities: plan.required_capabilities.clone(),
        bindings,
        grants,
        authentication: Some(AuthenticationProvider::ApiKey {
            header: "apikey".into(),
            registry_file_env: "AXIOM_DATABASE_DEV_IDENTITIES".into(),
        }),
        coordination: None,
    };
    let deployment_path = root.join(".axiom/database/deployment.json");
    private_write(&deployment_path, &serde_json::to_vec(&deployment)?)?;
    println!("Development API key: {token}");
    println!("Backend: http://{host}:{port}");
    let server = super::serve::load_host(&output, &plan_path, &deployment_path, None, None)?;
    let cancellation = axiom_server::invocation::InvocationCancellation::default();
    let token = cancellation.clone();
    tokio::spawn(async move {
        axiom_server::development::shutdown_signal().await;
        token.cancel();
    });
    axiom_server::development::real_session(server, format!("{host}:{port}").parse()?, debug)
        .run(cancellation)
        .await?;
    Ok(())
}
fn private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("private output needs a directory")?;
    std::fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    }
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}
async fn execute_sql(session: &Session, sql: &str) -> Result<()> {
    execute_sql_mode(session, sql, false).await
}
async fn execute_sql_mode(session: &Session, sql: &str, json: bool) -> Result<()> {
    let (mut complete, pending) = statements(sql);
    if !pending.trim().is_empty() {
        complete.push(pending);
    }
    for statement in complete {
        let query = session.query(&statement);
        tokio::pin!(query);
        let result = tokio::select! {
            result=&mut query=>result?,
            _=tokio::signal::ctrl_c()=>{
                session.cancel().await?;
                let _=query.await;
                bail!("Query cancelled.");
            }
        };
        if json {
            println!(
                "{}",
                serde_json::json!({"format":"axiom-database-sql-result/v1","columns":result.columns,"rows":result.rows,"affected":result.affected})
            );
            continue;
        }
        if !result.columns.is_empty() {
            println!("{}", result.columns.join(" | "));
        }
        for row in &result.rows {
            println!(
                "{}",
                row.iter()
                    .map(|v| v.as_deref().unwrap_or("NULL"))
                    .collect::<Vec<_>>()
                    .join(" | ")
            );
        }
        if result.columns.is_empty() {
            println!("OK ({} rows affected)", result.affected);
        } else {
            println!("({} rows)", result.rows.len());
        }
    }
    Ok(())
}
async fn console(session: &Session) -> Result<()> {
    if !std::io::stdin().is_terminal() {
        let mut input = String::new();
        use std::io::Read;
        std::io::stdin().read_to_string(&mut input)?;
        return execute_sql(session, &input).await;
    }
    println!("SQL console · \\dt tables · \\d TABLE columns · \\conninfo · \\q exit");
    let mut buffer = String::new();
    let mut input_history = vec![];
    loop {
        let history = input_history.clone();
        let prompt = if buffer.is_empty() {
            "axiom> "
        } else {
            "...> "
        };
        let line =
            match tokio::task::spawn_blocking(move || terminal_line(prompt, &history)).await?? {
                TerminalLine::Line(line) => {
                    if !line.trim().is_empty() {
                        input_history.push(line.clone());
                        if input_history.len() > 100 {
                            input_history.remove(0);
                        }
                    }
                    line
                }
                TerminalLine::Cancel => {
                    buffer.clear();
                    continue;
                }
                TerminalLine::Eof => return Ok(()),
            };
        let line = line.trim_end();
        if buffer.is_empty() && line.starts_with('\\') {
            if line == "\\q" {
                return Ok(());
            }
            if line == "\\conninfo" {
                println!("{}", session.label().await?);
                continue;
            }
            if line == "\\dt" {
                let sql=match session {Session::Postgres{..}=>"SELECT schemaname,tablename FROM pg_tables WHERE schemaname NOT IN ('pg_catalog','information_schema','_axiom_migrations') ORDER BY 1,2",#[cfg(unix)]Session::Sqlite{..}=>"SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE '__axiom_%' ORDER BY name"};
                execute_sql(session, &format!("{sql};")).await?;
                continue;
            }
            if let Some(name) = line.strip_prefix("\\d ") {
                let quoted = format!("'{}'", name.replace('\'', "''"));
                let sql = match session {
                    Session::Postgres { .. } => describe_postgres(name),
                    #[cfg(unix)]
                    Session::Sqlite { .. } => format!("PRAGMA table_info({quoted});"),
                };
                execute_sql(session, &sql).await?;
                continue;
            }
            eprintln!("Unknown command; use \\dt, \\d TABLE, \\conninfo or \\q.");
            continue;
        }
        buffer.push_str(line);
        buffer.push('\n');
        let (complete, pending) = statements(&buffer);
        for statement in complete {
            if let Err(error) = execute_sql(session, &format!("{statement};")).await {
                eprintln!("{error}");
            }
        }
        buffer = pending;
    }
}
fn describe_postgres(name: &str) -> String {
    let (schema, table) = name
        .split_once('.')
        .map_or((None, name), |(s, t)| (Some(s), t));
    let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
    format!("SELECT column_name,data_type,is_nullable,column_default FROM information_schema.columns WHERE table_schema={} AND table_name={} ORDER BY ordinal_position;",schema.map_or("current_schema()".into(),quote),quote(table))
}
enum TerminalLine {
    Line(String),
    Cancel,
    Eof,
}
fn terminal_line(prompt: &str, history: &[String]) -> Result<TerminalLine> {
    use crossterm::{
        cursor,
        event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
        execute, terminal,
    };
    terminal::enable_raw_mode()?;
    struct Raw;
    impl Drop for Raw {
        fn drop(&mut self) {
            let _ = crossterm::terminal::disable_raw_mode();
        }
    }
    let _raw = Raw;
    let mut out = std::io::stdout();
    let mut line = String::new();
    let mut index = history.len();
    print!("{prompt}");
    out.flush()?;
    loop {
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => {
                print!("^C\r\n");
                out.flush()?;
                return Ok(TerminalLine::Cancel);
            }
            (KeyCode::Char('d'), m) if m.contains(KeyModifiers::CONTROL) && line.is_empty() => {
                print!("\r\n");
                out.flush()?;
                return Ok(TerminalLine::Eof);
            }
            (KeyCode::Enter, _) => {
                print!("\r\n");
                out.flush()?;
                return Ok(TerminalLine::Line(line));
            }
            (KeyCode::Backspace, _) => {
                line.pop();
            }
            (KeyCode::Up, _) => {
                index = index.saturating_sub(1);
                line = history.get(index).cloned().unwrap_or_default();
            }
            (KeyCode::Down, _) => {
                index = (index + 1).min(history.len());
                line = history.get(index).cloned().unwrap_or_default();
            }
            (KeyCode::Char(c), m) if !m.contains(KeyModifiers::CONTROL) => line.push(c),
            _ => continue,
        }
        execute!(
            out,
            cursor::MoveToColumn(0),
            terminal::Clear(terminal::ClearType::CurrentLine)
        )?;
        print!("{prompt}{line}");
        out.flush()?;
    }
}
/// Split SQL at unquoted semicolons, retaining incomplete multiline input.
fn statements(source: &str) -> (Vec<String>, String) {
    let bytes = source.as_bytes();
    let mut start = 0;
    let mut i = 0;
    let mut quote = 0u8;
    let mut escaped = false;
    let mut dollar: Option<String> = None;
    let mut comment = 0usize;
    let mut line_comment = false;
    let mut statements = vec![];
    while i < bytes.len() {
        if line_comment {
            if bytes[i] == b'\n' {
                line_comment = false;
            }
            i += source[i..].chars().next().unwrap().len_utf8();
            continue;
        }
        if comment > 0 {
            if source[i..].starts_with("/*") {
                comment += 1;
                i += 2;
            } else if source[i..].starts_with("*/") {
                comment -= 1;
                i += 2;
            } else {
                i += source[i..].chars().next().unwrap().len_utf8();
            }
            continue;
        }
        if let Some(tag) = &dollar {
            if source[i..].starts_with(tag.as_str()) {
                i += tag.len();
                dollar = None;
            } else {
                i += source[i..].chars().next().unwrap().len_utf8();
            }
            continue;
        }
        if quote != 0 {
            if escaped && bytes[i] == b'\\' {
                i += 1;
                if i < bytes.len() {
                    i += source[i..].chars().next().unwrap().len_utf8();
                }
                continue;
            }
            if bytes[i] == quote {
                if i + 1 < bytes.len() && bytes[i + 1] == quote {
                    i += 2;
                    continue;
                }
                quote = 0;
                escaped = false;
            }
            i += source[i..].chars().next().unwrap().len_utf8();
            continue;
        }
        if source[i..].starts_with("--") {
            line_comment = true;
            i += 2;
            continue;
        }
        if source[i..].starts_with("/*") {
            comment = 1;
            i += 2;
            continue;
        }
        if bytes[i] == b'\'' || bytes[i] == b'"' || bytes[i] == 0x60 || bytes[i] == b'[' {
            quote = if bytes[i] == b'[' { b']' } else { bytes[i] };
            escaped = quote == b'\''
                && i > 0
                && matches!(bytes[i - 1], b'e' | b'E')
                && (i < 2 || !(bytes[i - 2].is_ascii_alphanumeric() || bytes[i - 2] == b'_'));
            i += 1;
            continue;
        }
        if bytes[i] == b'$' {
            if let Some(end) = source[i + 1..].find('$') {
                let tag = &source[i..i + end + 2];
                if tag[1..tag.len() - 1]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
                {
                    dollar = Some(tag.to_string());
                    i += tag.len();
                    continue;
                }
            }
        }
        if bytes[i] == b';' {
            let statement = source[start..i].trim();
            if !statement.is_empty() {
                statements.push(statement.into());
            }
            start = i + 1;
        }
        i += source[i..].chars().next().unwrap().len_utf8();
    }
    (statements, source[start..].trim().to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sql_boundaries_do_not_split_literals_or_procedures() {
        let (complete, pending) =
            statements("SELECT 'a;😀'; /* ; */ SELECT $$begin;end$$; SELECT 3");
        assert_eq!(complete.len(), 2);
        assert_eq!(pending, "SELECT 3");
    }
    #[test]
    fn connection_fields_are_engine_specific() {
        let root = Path::new("/tmp");
        let fields = ConnectionArgs {
            database: Some("tasks".into()),
            username: Some("owner".into()),
            ..Default::default()
        };
        assert!(matches!(
            resolve(&fields, "postgres", root).unwrap(),
            Connection::Postgres { .. }
        ));
        assert!(resolve(&fields, "sqlite", root).is_err());
    }
    #[test]
    fn sqlite_urls_decode_paths_and_reject_ephemeral_or_remote_targets() {
        let root = Path::new("/tmp");
        let args = |url: &str| ConnectionArgs {
            database_url: Some(url.into()),
            ..Default::default()
        };
        let Connection::Sqlite { file } =
            resolve(&args("sqlite:///tmp/task%20list.sqlite"), "sqlite", root).unwrap()
        else { panic!("expected SQLite file"); };
        assert_eq!(file, Path::new("/tmp/task list.sqlite"));
        for url in ["sqlite::memory:", "sqlite:///:memory:", "sqlite://remote/tasks.sqlite",
                    "sqlite:tasks.sqlite?mode=memory"] {
            assert!(resolve(&args(url), "sqlite", root).is_err(), "{url}");
        }
    }
}
