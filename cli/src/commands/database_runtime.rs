//! Offline schema import/derivation and private backend runtime preparation.
use anyhow::{bail, Context, Result};
use axiom_lib::{
    backend::{Capability, ServerPlan},
    contract::AxiomFile,
    database::{runtime::*, PgType},
    ir::TypeRef,
};
use clap::Subcommand;
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
};
#[derive(Subcommand, Debug)]
pub enum Action {
    /// Generate only explicitly selected API/domain fields from a database schema
    Derive {
        #[arg(long)]
        schema: PathBuf,
        #[arg(long)]
        table: String,
        #[arg(long)]
        model: String,
        #[arg(long)]
        entity: bool,
        #[arg(long = "field", required = true)]
        fields: Vec<String>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Validate typed programs and attach them to a private server plan, offline
    Prepare {
        #[arg(long)]
        schema: PathBuf,
        #[arg(long)]
        contract: PathBuf,
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        resource: String,
        #[arg(long)]
        spec: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Source-linked schema-to-action/endpoint impact; no live database calls
    Impact {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        resource: String,
        #[arg(long)]
        before: PathBuf,
        #[arg(long)]
        after: PathBuf,
        #[arg(long, conflicts_with = "application")]
        application_evidence: Option<PathBuf>,
        #[arg(long)]
        application: Option<PathBuf>,
    },
}
fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut f = tempfile::NamedTempFile::new_in(parent)?;
    f.write_all(bytes)?;
    f.as_file().sync_all()?;
    f.persist(path)?;
    Ok(())
}
pub fn run(a: &Action) -> Result<()> {
    match a {
        Action::Derive {
            schema,
            table,
            model,
            entity,
            fields,
            out,
        } => {
            let schema = super::database::contract(schema)?;
            axiom_lib::database::identifier(model)?;
            let (s, t) = table
                .split_once('.')
                .context("ADB1000: use logical schema.table")?;
            let t = schema
                .schemas
                .iter()
                .find(|v| v.name == s)
                .and_then(|s| s.tables.iter().find(|v| v.name == t))
                .context("ADB1000: unknown derived table")?;
            let mut selected = BTreeMap::new();
            for field in fields {
                let (alias, name) = field
                    .split_once('=')
                    .context("ADB1000: field syntax is APIField=storageField")?;
                axiom_lib::database::identifier(alias)?;
                let c = t
                    .columns
                    .iter()
                    .find(|c| c.name == name)
                    .context("ADB1000: unknown derived column")?;
                if selected.insert(alias, c).is_some() {
                    bail!("ADB1000: duplicate API field");
                }
            }
            let mut source=format!("// Schema-derived explicit projection; regenerate after schema changes.\nmodel {model} {{\n");
            for (alias, c) in selected {
                if c.nullable {
                    bail!("ADB1000: nullable fields require an explicit reviewed model rule; the initial source generator handles nonnull projections");
                }
                let ty = match backend_type(&c.storage_type)? {
                    TypeRef::String => "String",
                    TypeRef::Int32 => "Int32",
                    TypeRef::Bool => "Bool",
                    TypeRef::Bytes => "Bytes",
                    TypeRef::DateTime => "DateTime",
                    TypeRef::Json => "Json",
                    _ => bail!("ADB1000: derived codec unavailable"),
                };
                let declaration = match c.storage_type {
                    PgType::Int64 => "String(pattern: \"^(0|-?[1-9][0-9]*)$\")".into(),
                    PgType::VarChar { length } => format!("String(maxLength: {length})"),
                    _ => ty.into(),
                };
                source.push_str(&format!("  {alias}: {declaration}\n"));
            }
            source.push_str(if *entity { "} also entity\n" } else { "}\n" });
            write(out, source.as_bytes())?;
            println!(
                "{}",
                serde_json::json!({"format":"axiom-database-derived-model/v1","model":model,"schemaSha256":schema.sha256()?,"effects":"offline-only","publicFields":"explicit-projection-only"})
            );
        }
        Action::Prepare {
            schema,
            contract,
            plan,
            resource,
            spec,
            out,
        } => {
            let origins = if schema.extension().is_some_and(|e| e == "axiom") {
                BTreeMap::new()
            } else {
                super::database::source(schema)?
                    .1
                    .symbols
                    .values()
                    .map(|s| {
                        (
                            s.id.clone(),
                            serde_json::to_value(&s.origin).expect("source origin"),
                        )
                    })
                    .collect()
            };
            let schema = super::database::contract(schema)?;
            let public: AxiomFile = axiom_server::serve::read_json(contract, 16_777_216)?;
            let mut server: ServerPlan = axiom_server::serve::read_json(plan, 16_777_216)?;
            server.validate(&public)?;
            let mut spec: serde_json::Value = axiom_server::serve::read_json(spec, 4_194_304)?;
            if spec.get("schema").is_some() {
                bail!("ADB1000: schema authority must come from --schema");
            }
            spec.as_object_mut()
                .context("ADB1000: program specification requires an object")?
                .insert("schema".into(), serde_json::to_value(schema)?);
            if spec.get("sourceOrigins").is_some() {
                bail!("ADB1000: source origins must come from --schema");
            }
            let mut runtime: RuntimePlan = serde_json::from_value(spec)
                .context("ADB1000: invalid typed runtime specification")?;
            runtime.source_origins = origins;
            server.prepare_transactions(resource, runtime, &public)?;
            server
                .required_capabilities
                .insert(Capability::DatabasePersistence);
            server.validate(&public)?;
            write(out, &server.canonical_bytes()?)?;
            println!(
                "{}",
                serde_json::json!({"format":"axiom-database-runtime-prepared/v1","planSha256":server.sha256()?,"effects":"offline-only","migrationAuthority":false,"backendMapping":"schema-derived-field-level","replicaPolicy":"primary-only"})
            );
        }
        Action::Impact {
            plan,
            resource,
            before,
            after,
            application_evidence,
            application,
        } => {
            let server: ServerPlan = axiom_server::serve::read_json(plan, 16_777_216)?;
            let runtime = server
                .persistence
                .get(resource)
                .context("ADB1000: resource has no schema-derived mapping")?;
            let mut report = runtime.impact(
                resource,
                &super::database::contract(before)?,
                &super::database::contract(after)?,
                &server,
            )?;
            if application_evidence.is_some() || application.is_some() {
                use axiom_lib::application_evidence::{
                    ApplicationEvidence, EvidenceEdgeKind, EvidenceNodeKind,
                };
                let graph: ApplicationEvidence = if let Some(path) = application_evidence {
                    axiom_server::serve::read_json(path, 16_777_216)?
                } else {
                    super::inspector_frontend::enrich(
                        application.as_ref().expect("application path"),
                        axiom_lib::application_inspector::inspect_workspace(
                            application.as_ref().expect("application path"),
                            env!("CARGO_PKG_VERSION"),
                        )?,
                    )?
                };
                graph.validate()?;
                let services = graph
                    .nodes
                    .iter()
                    .filter(|n| {
                        n.kind == EvidenceNodeKind::BackendService
                            && n.attributes
                                .get("contractSha256")
                                .and_then(serde_json::Value::as_str)
                                == Some(server.contract_sha256.as_str())
                    })
                    .map(|n| n.id.as_str())
                    .collect::<std::collections::BTreeSet<_>>();
                if services.len() != 1 {
                    bail!("ADB1000: application graph must contain exactly one matching public backend contract");
                }
                let mut impacts = Vec::new();
                for endpoint in report["endpoints"]
                    .as_array()
                    .context("invalid impact report")?
                {
                    let candidates = graph
                        .nodes
                        .iter()
                        .filter(|n| {
                            n.kind == EvidenceNodeKind::Operation
                                && n.label == endpoint["endpoint"].as_str().unwrap_or("")
                                && graph.edges.iter().any(|e| {
                                    services.contains(e.from.as_str())
                                        && e.to == n.id
                                        && e.kind == EvidenceEdgeKind::Exposes
                                })
                        })
                        .collect::<Vec<_>>();
                    if candidates.len() != 1 {
                        bail!("ADB1000: affected endpoint is missing or ambiguous in the application graph");
                    }
                    impacts.push(axiom_lib::application_change::impact(
                        &graph,
                        &candidates[0].id,
                        64,
                        10_000,
                    )?);
                }
                report["frontendImpact"] = serde_json::json!({"status":"compiler-graph-observed","reports":impacts,"graphRevision":graph.graph_revision});
            }
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
    }
    Ok(())
}
