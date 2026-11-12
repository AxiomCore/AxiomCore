//! `axiom database convert draft|plan|submit` (`placement/v1` 7.2, 7.3 and 10.3): engine
//! conversion of a hosted libSQL database to PostgreSQL.
//!
//! `draft` writes a PostgreSQL contract proposal from the hosted libSQL head. `plan`
//! builds and signs the PostgreSQL baseline release (a new stream with the same name),
//! computes the compatibility report locally and signs the transition receipt; nothing is
//! sent. `submit` uploads the baseline and the receipt; the server recomputes the report
//! and answers `validated` or `refused`. The conversion itself is the resource action
//! `convert` (`axiom project resources request --action convert`).
use super::database_dev as dev;
use super::database_remote::{self as remote, Hosted};
use anyhow::{bail, Context, Result};
use axiom_cloud::migrations::WaitPolicy;
use axiom_database::conversion::{self as conv, draft, Binding, Encoding, Mapping, Receipt, SignedReceipt};
use axiom_database::developer::{self as db, Release};
use clap::Subcommand;
use reqwest::Method;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
};

const BASELINE: &str = "baseline.release.json";
const REPORT: &str = "report.json";
const RECEIPT: &str = "receipt.json";
const BINDING: &str = "binding.json";

#[derive(Subcommand, Debug)]
pub enum Action {
    /// Write a PostgreSQL contract proposal from the hosted libSQL head
    ///
    /// Uses the exact storage pairs only (INTEGER to Int64, REAL to Float64, TEXT to Text,
    /// BLOB to Bytea). Views, triggers and anything without an automatic form are listed
    /// for you to rewrite. Review and edit the proposal before `convert plan`.
    Draft {
        /// AxiomCore Cloud project slug
        #[arg(long)]
        project: String,
        /// Hosted environment slug of the libSQL database
        #[arg(long)]
        environment: String,
        /// Application stream (contract ID) of the hosted head
        #[arg(long)]
        stream: String,
        /// PostgreSQL schema for every table
        #[arg(long, default_value = draft::DEFAULT_SCHEMA)]
        schema: String,
        /// New directory for `schema.acore` and `AxiomDeps.toml`
        #[arg(long)]
        out: PathBuf,
    },
    /// Sign the PostgreSQL baseline and the transition receipt; print the report
    ///
    /// Reads the hosted libSQL head, builds the baseline release from CONTRACT (a
    /// PostgreSQL contract with the same contract ID), computes the compatibility report
    /// and, when nothing is refused, signs the receipt with your enrolled key. Writes the
    /// baseline, report and receipt to --out. Nothing is uploaded.
    Plan {
        #[arg(value_name = "CONTRACT")]
        source: PathBuf,
        #[arg(long)]
        project: String,
        #[arg(long)]
        environment: String,
        /// INTEGER to timestamp encoding, `table.column=unix_ms` or `=unix_s`; repeatable
        #[arg(long = "encoding", value_name = "TABLE.COLUMN=ENCODING")]
        encodings: Vec<String>,
        /// Read zone-less TEXT as UTC into a timestamptz column, `table.column`; repeatable
        #[arg(long = "assume-utc", value_name = "TABLE.COLUMN")]
        assume_utc: Vec<String>,
        /// Accept the semantic warnings of the report (SQLite NOCASE or RTRIM collations)
        #[arg(long)]
        acknowledge_semantics: bool,
        /// Directory for the plan files
        #[arg(long, default_value = ".axiom/conversion")]
        out: PathBuf,
        /// Print the report as JSON
        #[arg(long)]
        json: bool,
    },
    /// Upload the baseline and the receipt; the server recomputes the report
    Submit {
        #[arg(long)]
        project: String,
        /// Directory written by `convert plan`
        #[arg(long, default_value = ".axiom/conversion")]
        dir: PathBuf,
        /// Wait until the server has checked the conversion (five minutes)
        #[arg(long)]
        wait: bool,
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(action: &Action) -> Result<()> {
    match action {
        Action::Draft { project, environment, stream, schema, out } => {
            let (project, environment) = remote::target(Some((project, environment)))?;
            let client = crate::auth_store::authenticated_cloud_client()?;
            let h = Hosted { client: &client, project, environment, wait: WaitPolicy::default() };
            draft_with(&h, stream, schema, out, &mut std::io::stdout()).await
        }
        Action::Plan { source, project, environment, encodings, assume_utc, acknowledge_semantics, out, json } => {
            let mapping = mapping(encodings, assume_utc, *acknowledge_semantics)?;
            let (project, environment) = remote::target(Some((project, environment)))?;
            let client = crate::auth_store::authenticated_cloud_client()?;
            let h = Hosted { client: &client, project, environment, wait: WaitPolicy::default() };
            plan_with(&h, source, mapping, out, *json, &mut std::io::stdout()).await
        }
        Action::Submit { project, dir, wait, json } => submit(project, dir, *wait, *json).await,
    }
}

fn split(value: &str) -> Result<(String, String)> {
    let (table, column) = value.split_once('.').filter(|(t, c)| !t.is_empty() && !c.is_empty()).with_context(|| format!("`{value}` is not table.column"))?;
    Ok((table.into(), column.into()))
}
/// The receipt mapping from the command line flags.
pub(crate) fn mapping(encodings: &[String], assume_utc: &[String], acknowledge: bool) -> Result<Mapping> {
    let mut m = Mapping { acknowledge_semantics: acknowledge, ..Default::default() };
    for e in encodings {
        let (column, encoding) = e.split_once('=').with_context(|| format!("--encoding `{e}` needs table.column=unix_ms or =unix_s"))?;
        let encoding = match encoding {
            "unix_ms" => Encoding::UnixMs,
            "unix_s" => Encoding::UnixS,
            _ => bail!("--encoding `{e}`: the encoding is unix_ms or unix_s"),
        };
        let (table, column) = split(column)?;
        if m.encodings.entry(table).or_default().insert(column, encoding).is_some() {
            bail!("--encoding `{e}` is given twice");
        }
    }
    for value in assume_utc {
        let (table, column) = split(value)?;
        m.assume_utc.entry(table).or_default().insert(column);
    }
    m.validate().map_err(remote::engine_error)?;
    Ok(m)
}

/// The libSQL head of `stream`, downloaded and verified.
async fn head(h: &Hosted<'_>, stream: &str) -> Result<Release> {
    let target = remote::hosted_head(h, stream).await?.with_context(|| format!("ADB1102: stream `{stream}` has no applied release in {}", h.environment))?;
    let release = remote::fetch_release(h, &target.head).await?;
    if release.revision.desired.engine != "sqlite" {
        bail!("ADB1150: the hosted database of {} is not libSQL; there is nothing to convert", h.environment);
    }
    Ok(release)
}

pub(crate) async fn draft_with(h: &Hosted<'_>, stream: &str, schema: &str, out: &Path, w: &mut dyn Write) -> Result<()> {
    if out.exists() && std::fs::read_dir(out).map(|mut d| d.next().is_some()).unwrap_or(true) {
        bail!("{} exists and is not empty; choose a new directory", out.display());
    }
    let head = head(h, stream).await?;
    let d = draft::draft(&head.revision.desired, schema).map_err(remote::engine_error)?;
    write_draft(&d, out)?;
    writeln!(w, "Wrote a PostgreSQL contract proposal for {stream} {} to {}.", head.revision.identity.version, out.display())?;
    if d.notes.is_empty() {
        writeln!(w, "Every table uses exact storage pairs; review it, then run `axiom database convert plan`.")?;
    } else {
        writeln!(w, "Review these before `axiom database convert plan`:")?;
        for note in &d.notes {
            writeln!(w, "  - {note}")?;
        }
    }
    Ok(())
}
pub(crate) fn write_draft(d: &draft::Draft, out: &Path) -> Result<()> {
    std::fs::create_dir_all(out)?;
    std::fs::write(out.join("schema.acore"), &d.source)?;
    std::fs::write(out.join("AxiomDeps.toml"), &d.manifest)?;
    Ok(())
}

/// The baseline release of the PostgreSQL contract at `source`, signed with the local key.
pub(crate) fn baseline(source: &Path) -> Result<Release> {
    let loaded = dev::load(source)?;
    if loaded.compiled.contract.engine != "postgres" {
        bail!("ADB1150: the baseline contract must declare engine postgres");
    }
    let revision = db::revision(&loaded.compiled, loaded.input, None, "migration", None).map_err(remote::engine_error)?;
    db::sign(revision).map_err(remote::engine_error)
}

/// The resource of `environment` and the project UUID the receipt binds.
async fn binding(h: &Hosted<'_>) -> Result<Binding> {
    let project_id = h.client.project_id(h.project).await?;
    let summary = h.client.resource_request(Method::GET, &["projects", h.project, "resources"], None, None).await?;
    let resources: Vec<&Value> = summary["resources"].as_array().map(|r| r.iter().filter(|r| r["environment"].as_str() == Some(h.environment)).collect()).unwrap_or_default();
    let [resource] = resources.as_slice() else {
        bail!("AXRESOURCE_NOT_FOUND: {} has no single hosted database in {}", h.project, h.environment);
    };
    if resource["engine"].as_str() != Some("sqlite") {
        bail!("ADB1150: the hosted database of {} is not libSQL; there is nothing to convert", h.environment);
    }
    let text = |k: &str| resource[k].as_str().map(str::to_string).with_context(|| format!("AXRESOURCE_INVALID_RESPONSE: resource has no {k}"));
    Ok(Binding { project_id, environment_id: text("environment_id")?, resource_id: text("id")?, resource_generation: resource["generation"].as_i64().context("AXRESOURCE_INVALID_RESPONSE: resource has no generation")? })
}

pub(crate) fn print_report(report: &conv::Report, w: &mut dyn Write) -> Result<()> {
    let c = &report.counts;
    writeln!(w, "Compatibility report for {}: {} tables, {} columns ({} exact, {} checked per value), {} added, {} warnings, {} refused.", report.stream, c.tables, c.columns, c.exact, c.checked, c.added, c.warnings, c.refused)?;
    let name = |f: &conv::Finding| f.column.as_ref().map_or_else(|| f.table.clone(), |c| format!("{}.{c}", f.table));
    for f in &report.warnings {
        writeln!(w, "  warning {}: {}", name(f), f.rule)?;
    }
    for f in &report.refused {
        writeln!(w, "  refused {}: {}", name(f), f.rule)?;
    }
    Ok(())
}

pub(crate) async fn plan_with(h: &Hosted<'_>, source: &Path, mapping: Mapping, out: &Path, json: bool, w: &mut dyn Write) -> Result<()> {
    let baseline = baseline(source)?;
    let stream = baseline.revision.identity.name.clone();
    let head = head(h, &stream).await?;
    conv::check_baseline(&head, &baseline).map_err(remote::engine_error)?;
    remote::require_enrolled(h, [baseline.public_key.as_str()]).await?;
    let report = conv::report(&head, &baseline, &mapping).map_err(remote::engine_error)?;
    if json {
        writeln!(w, "{}", serde_json::to_string(&report)?)?;
    } else {
        print_report(&report, w)?;
    }
    if !report.compatible() {
        bail!("AXRESOURCE_CONVERSION_INCOMPATIBLE: the report refuses {} item(s); change the PostgreSQL contract or the mapping and plan again. Nothing was signed.", report.refused.len());
    }
    let binding = binding(h).await?;
    let created_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let receipt = Receipt::new(&binding, &report, created_at).map_err(remote::engine_error)?;
    let signed = receipt.sign(|m| db::sign_message_with_local_key(&stream, m)).map_err(remote::engine_error)?;
    if signed.public_key != baseline.public_key {
        bail!("ADB1109: the receipt and the baseline must be signed by the same enrolled key");
    }
    write_plan(out, &baseline, &report, &signed, &binding)?;
    if !json {
        writeln!(w, "Signed the baseline {} and the transition receipt for libSQL head {} (generation {}). Plan written to {}.", baseline.revision.identity.version, head.revision.identity.version, binding.resource_generation, out.display())?;
        writeln!(w, "Next: `axiom database convert submit --project {}`.", h.project)?;
    }
    Ok(())
}
pub(crate) fn write_plan(out: &Path, baseline: &Release, report: &conv::Report, receipt: &SignedReceipt, binding: &Binding) -> Result<()> {
    std::fs::create_dir_all(out)?;
    std::fs::write(out.join(BASELINE), axiom_database::canonical_bytes(baseline).map_err(remote::engine_error)?)?;
    std::fs::write(out.join(REPORT), report.canonical().map_err(remote::engine_error)?)?;
    std::fs::write(out.join(RECEIPT), serde_json::to_vec(receipt)?)?;
    std::fs::write(out.join(BINDING), serde_json::to_vec(binding)?)?;
    Ok(())
}

/// The plan files, checked against each other before anything is sent.
pub(crate) struct Plan {
    pub baseline: Release,
    pub report: Value,
    pub receipt: SignedReceipt,
    pub binding: Binding,
}
pub(crate) fn read_plan(dir: &Path) -> Result<Plan> {
    let read = |name: &str| std::fs::read(dir.join(name)).with_context(|| format!("read {}; run `axiom database convert plan` first", dir.join(name).display()));
    let baseline: Release = serde_json::from_slice(&read(BASELINE)?).context("ADB1101: the baseline release does not parse")?;
    let report_bytes = read(REPORT)?;
    let report: conv::Report = serde_json::from_slice(&report_bytes).context("the report does not parse")?;
    let receipt: SignedReceipt = serde_json::from_slice(&read(RECEIPT)?).context("the receipt does not parse")?;
    let binding: Binding = serde_json::from_slice(&read(BINDING)?).context("the binding does not parse")?;
    let body: Receipt = serde_json::from_value(receipt.receipt.clone()).context("the receipt does not parse")?;
    if body.report_sha256 != report.sha256().map_err(remote::engine_error)? || body.to.baseline_release_sha256 != baseline.sha256 || body.resource_id != binding.resource_id {
        bail!("AXRESOURCE_RECEIPT_INVALID: the plan files in {} do not belong together; plan again", dir.display());
    }
    Ok(Plan { baseline, report: serde_json::from_slice(&report_bytes)?, receipt, binding })
}

async fn submit(project: &str, dir: &Path, wait: bool, json: bool) -> Result<()> {
    let plan = read_plan(dir)?;
    // Any environment slug works for the upload; it is project-scoped.
    let client = crate::auth_store::authenticated_cloud_client()?;
    let h = Hosted { client: &client, project, environment: "development", wait: WaitPolicy::default() };
    remote::upload(&h, &plan.baseline).await?;
    let body = json!({"receipt": plan.receipt, "report": plan.report, "baseline_release_sha256": plan.baseline.sha256});
    let key = remote::idempotency_key();
    let resource = plan.binding.resource_id.as_str();
    let mut value = client.resource_request(Method::POST, &["projects", project, "resources", resource, "conversions"], Some(&body), Some(&key)).await?;
    let id = value["conversion"]["id"].as_str().context("AXRESOURCE_INVALID_RESPONSE: no conversion id")?.to_string();
    if wait {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
        loop {
            let current = client.resource_request(Method::GET, &["projects", project, "resources", resource, "conversions", &id], None, None).await?;
            let status = current["conversion"]["status"].as_str().or_else(|| current["status"].as_str()).unwrap_or("checking").to_string();
            value = current;
            if status != "checking" || tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    let conversion = if value["conversion"].is_object() { &value["conversion"] } else { &value };
    let status = conversion["status"].as_str().unwrap_or("checking");
    match status {
        "validated" => println!("Conversion {id} is validated. Start it with `axiom project resources request --project {project} --resource {resource} --action convert --conversion {id} --generation {} --confirm \"convert <database> to postgresql\"`.", plan.binding.resource_generation),
        "refused" => bail!("{}: conversion {id} was refused by the server check.", conversion["code"].as_str().unwrap_or("AXRESOURCE_CONVERSION_INCOMPATIBLE")),
        _ => println!("Conversion {id} is {status}. Check it with `axiom database convert submit --wait` or the dashboard."),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping_flags_build_the_receipt_mapping() {
        let m = mapping(&["users.created=unix_ms".into(), "events.at=unix_s".into()], &["logs.when".into()], true).unwrap();
        assert_eq!(serde_json::to_value(&m).unwrap(), json!({"encodings": {"events": {"at": "unix_s"}, "users": {"created": "unix_ms"}}, "assume_utc": {"logs": ["when"]}, "acknowledge_semantics": true}));
        assert!(mapping(&["users.created=unix_ns".into()], &[], false).is_err());
        assert!(mapping(&["created=unix_ms".into()], &[], false).is_err());
        assert!(mapping(&["users.created=unix_ms".into(), "users.created=unix_s".into()], &[], false).is_err());
        assert!(mapping(&[], &[".when".into()], false).is_err());
    }

    const SQLITE: &str = "table users(strict: true) { id: Sqlite.Integer(autoIncrement: true) email: Sqlite.Text created: Sqlite.Integer primaryKey(columns: [.id]) }";
    fn sqlite_head() -> Release {
        let input = acore::database::ProjectInput {
            entry: "schema.acore".into(),
            files: BTreeMap::from([
                ("AxiomDeps.toml".into(), "type = \"database\"\ntargets = [\"server\"]\n".into()),
                ("schema.acore".into(), format!("profile database 1\nproject(id: \"cli.convert\", version: \"1.0.0\")\nmodule cli.convert\ndatabase main(engine: DatabaseEngine.sqlite) {{ manage(schemas: [Schemas.main], mode: ManagementMode.ownedObjects) schema main {{ {SQLITE} }} }}")),
            ]),
            packages: Default::default(),
        };
        let compiled = acore::database::compile(&input).unwrap();
        db::sign_with_key(db::revision(&compiled, input, None, "migration", None).unwrap(), [7; 32]).unwrap()
    }

    #[test]
    fn draft_plan_files_round_trip_and_detect_mixing() {
        let head = sqlite_head();
        let d = draft::draft(&head.revision.desired, "app").unwrap();
        let dir = tempfile::tempdir().unwrap();
        write_draft(&d, &dir.path().join("pg")).unwrap();
        let input = acore::database::ProjectInput { entry: "schema.acore".into(), files: BTreeMap::from([("schema.acore".into(), d.source.clone()), ("AxiomDeps.toml".into(), d.manifest.clone())]), packages: Default::default() };
        let compiled = acore::database::compile(&input).unwrap();
        let baseline = db::sign_with_key(db::revision(&compiled, input, None, "migration", None).unwrap(), [7; 32]).unwrap();
        conv::check_baseline(&head, &baseline).unwrap();
        let report = conv::report(&head, &baseline, &Mapping::default()).unwrap();
        assert!(report.compatible(), "{:?}", report.refused);
        let binding = Binding { project_id: "p".into(), environment_id: "e".into(), resource_id: "2b9c7f6e-2a51-4c1e-9d1a-3f5e6a7b8c9d".into(), resource_generation: 1 };
        let receipt = Receipt::new(&binding, &report, "2026-11-01T00:00:00Z".into()).unwrap().sign(|m| {
            use ed25519_dalek::Signer as _;
            let key = ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
            Ok((hex::encode(key.verifying_key().as_bytes()), hex::encode(key.sign(m).to_bytes())))
        }).unwrap();
        let out = dir.path().join("plan");
        write_plan(&out, &baseline, &report, &receipt, &binding).unwrap();
        let plan = read_plan(&out).unwrap();
        assert_eq!(plan.baseline.sha256, baseline.sha256);
        assert_eq!(plan.receipt, receipt);
        // A report from another plan is refused before anything is sent.
        let other = conv::report(&head, &baseline, &Mapping { acknowledge_semantics: true, ..Default::default() }).unwrap();
        std::fs::write(out.join(REPORT), other.canonical().unwrap()).unwrap();
        assert!(read_plan(&out).is_err());
        let mut buf = vec![];
        print_report(&report, &mut buf).unwrap();
        assert!(String::from_utf8(buf).unwrap().starts_with("Compatibility report for cli.convert: 1 tables, 3 columns (3 exact"));
    }
}
