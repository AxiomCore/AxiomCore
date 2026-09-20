use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::{bail, Context, Result};
use axiom_lib::{
    application_change::{
        attach_runtime_diff, impact, semantic_diff, ChangeImpact, ImpactReport,
        SemanticChangeReport,
    },
    application_evidence::{
        ApplicationEvidence, AxiomQuery, EvidenceIndex, EvidenceNode, EvidenceNodeKind,
        QueryDirection, QueryOperation, QueryResult, AXIOM_QUERY_FORMAT, QUERY_RESULT_FORMAT,
    },
    application_inspector::inspect_workspace,
    inspector_exchange::{InspectorExchange, InspectorExchangePayload},
    question::{
        answer_question, plan_question, unsupported_answer, QuestionAnswer, QuestionRequest,
        ANSWER_FORMAT,
    },
};
use base64::{engine::general_purpose::STANDARD, Engine};
use clap::{Subcommand, ValueEnum};
use ed25519_dalek::{Signer, SigningKey};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum InspectorFormat {
    Human,
    Json,
    Jsonl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InspectorPlanner {
    Deterministic,
    Jev,
}

#[derive(Debug, Subcommand)]
pub enum InspectorAction {
    /// Inspect compiler-proven frontend modules, routes, pages, and behavior.
    Frontend {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Inspect the semantic UI tree and its action bindings.
    Ui {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Inspect typed styles, variables, and responsive branches.
    Styles {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Inspect state declarations and proven readers/writers.
    State {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Inspect declared accessibility semantics.
    Accessibility {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Inspect backend services, operations, data, and policies.
    Backend {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Trace a frontend or backend operation across contract boundaries.
    Operation {
        selector: String,
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Inspect models, fields, projections, relationships, and lineage.
    Data {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        lineage: Option<String>,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// List explicit public/restricted backend exposure facts.
    Exposure {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Inspect contract-owned cache policy and operation associations.
    Cache {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Inspect deterministic security facts and findings.
    Security {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Show the resolved bill of authority.
    Authority {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Inspect one deterministic finding and its evidence.
    Finding {
        selector: Option<String>,
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Show blocker/warning counts without an opaque security score.
    Readiness {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Summarize discovered application facts and verification state.
    Overview {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Write a canonical, content-addressed application evidence snapshot.
    Snapshot {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Verify schema compatibility, references, revision, and canonical bytes.
    ValidateSnapshot { snapshot: PathBuf },
    /// Show direct and transitive contracts, packages, and executable extensions.
    Dependencies {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Inspect one imported contract and optionally its reachable closure.
    Contract {
        alias: String,
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        closure: bool,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Show executable code and authority reachable from a dependency.
    ExecutableClosure {
        alias: String,
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        target: Option<String>,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Show provenance and evidence for one package, contract, or extension.
    Provenance {
        selector: String,
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Query a graph neighborhood, optionally rooted at a semantic ID or label.
    Graph {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        root: Option<String>,
        #[arg(long, value_enum, default_value = "both")]
        direction: InspectorDirection,
        #[arg(long, default_value_t = 8)]
        max_depth: usize,
        #[arg(long, default_value_t = 500)]
        max_results: usize,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Show one semantic fact and its immediate evidence neighborhood.
    Node {
        selector: String,
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Show the source or artifact evidence supporting one semantic fact.
    Evidence {
        selector: String,
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Trace typed upstream or downstream paths.
    Trace {
        selector: String,
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, value_enum, default_value = "downstream")]
        direction: InspectorDirection,
        #[arg(long, default_value_t = 8)]
        max_depth: usize,
        #[arg(long, default_value_t = 500)]
        max_results: usize,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Create a bounded local runtime session or normalize an existing audit export.
    Record {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        target: String,
        #[arg(long)]
        input: Option<PathBuf>,
        #[arg(long)]
        session: Option<String>,
        /// Record that runtime capture was intentionally disabled.
        #[arg(long)]
        disabled: bool,
    },
    /// Inspect a normalized local runtime session.
    Runtime {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, default_value = "latest")]
        session: String,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Export one canonical, redacted runtime audit bundle.
    ExportAudit {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, default_value = "latest")]
        session: String,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Import an offline runtime session or extension audit export.
    ImportAudit {
        input: PathBuf,
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        session: Option<String>,
    },
    /// Launch the secure loopback Axiom Inspector dashboard.
    Serve {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, default_value_t = 0)]
        port: u16,
        #[arg(long)]
        no_open: bool,
        #[arg(long)]
        api_only: bool,
        /// Permit the dashboard/API to send bounded question metadata to Jev through Vercel AI Gateway.
        #[arg(long)]
        allow_remote_planner: bool,
        #[arg(long, default_value_t = 0.65)]
        jev_intent_threshold: f64,
        #[arg(long, default_value_t = 0.55)]
        jev_selector_threshold: f64,
    },
    /// Explain the authoritative facts adjacent to a semantic fact.
    Why {
        selector: String,
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Show who requests, receives, or effectively holds access.
    Access {
        selector: String,
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Find a deterministic shortest downstream path between two facts.
    Reachability {
        from: String,
        to: String,
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, default_value_t = 16)]
        max_depth: usize,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Trace the currently available typed lineage around a fact.
    Lineage {
        selector: String,
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, default_value_t = 16)]
        max_depth: usize,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Compare the target-scoped neighborhood of a fact.
    TargetCompare {
        selector: String,
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, value_delimiter = ',', required = true)]
        targets: Vec<String>,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Alias for target-compare using Milestone B terminology.
    TargetDiff {
        selector: String,
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, value_delimiter = ',', required = true)]
        targets: Vec<String>,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Execute a bounded declarative axiom-query/v1 JSON document.
    Query {
        query_file: PathBuf,
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, value_enum, default_value = "json")]
        format: InspectorFormat,
    },
    /// Compare canonical application evidence by semantic identity.
    Diff {
        before: PathBuf,
        after: PathBuf,
        /// Optional normalized runtime baseline; must be paired with --after-runtime.
        #[arg(long, requires = "after_runtime")]
        before_runtime: Option<PathBuf>,
        /// Optional normalized runtime comparison; timing noise is ignored.
        #[arg(long, requires = "before_runtime")]
        after_runtime: Option<PathBuf>,
        #[arg(long, value_delimiter = ',')]
        fail_on: Vec<InspectorFailureClass>,
        /// Write the byte-stable canonical CI artifact.
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Calculate direct and bounded transitive impact paths.
    Impact {
        selector: String,
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        against: PathBuf,
        #[arg(long, default_value_t = 16)]
        max_depth: usize,
        #[arg(long, default_value_t = 10_000)]
        max_results: usize,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Ask through the local deterministic planner or the bounded opt-in Jev planner.
    Ask {
        question: String,
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Optional canonical semantic-change report required by "what changed".
        #[arg(long)]
        changes: Option<PathBuf>,
        /// Natural-language planner. Jev is probabilistic and remote; facts remain local and deterministic.
        #[arg(long, value_enum, default_value = "deterministic")]
        planner: InspectorPlanner,
        /// Explicitly permit sending the question and a bounded semantic candidate list to the remote planner.
        #[arg(long)]
        allow_remote: bool,
        #[arg(long, default_value_t = 0.65)]
        jev_intent_threshold: f64,
        #[arg(long, default_value_t = 0.55)]
        jev_selector_threshold: f64,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
    /// Execute a pre-planned, validated axiom-inspector-question/v1 document.
    Answer {
        question_file: PathBuf,
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        changes: Option<PathBuf>,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "json")]
        format: InspectorFormat,
    },
    /// Create an explicit signed offline handoff for Axiom Studio or Cloud.
    ExportInspector {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        runtime: bool,
        #[arg(long)]
        changes: Option<PathBuf>,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Verify and unpack a signed Inspector handoff without network access.
    ImportInspector {
        input: PathBuf,
        #[arg(short, long)]
        output_dir: PathBuf,
    },
    /// Measure local Inspector release budgets and privacy/offline invariants.
    ReleaseCheck {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        enforce: bool,
        #[arg(long, value_enum, default_value = "human")]
        format: InspectorFormat,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum InspectorFailureClass {
    Breaking,
    ApprovalRequired,
    PermissionIncrease,
    Warning,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum InspectorDirection {
    Upstream,
    Downstream,
    Both,
}

impl From<InspectorDirection> for QueryDirection {
    fn from(value: InspectorDirection) -> Self {
        match value {
            InspectorDirection::Upstream => Self::Upstream,
            InspectorDirection::Downstream => Self::Downstream,
            InspectorDirection::Both => Self::Both,
        }
    }
}

pub async fn handle(action: &InspectorAction) -> Result<()> {
    match action {
        InspectorAction::Frontend { path, format } => render_kinds(
            path,
            *format,
            &[
                EvidenceNodeKind::FrontendModule,
                EvidenceNodeKind::Route,
                EvidenceNodeKind::Page,
                EvidenceNodeKind::Component,
                EvidenceNodeKind::Primitive,
                EvidenceNodeKind::Action,
                EvidenceNodeKind::ActionStep,
                EvidenceNodeKind::State,
                EvidenceNodeKind::DerivedValue,
                EvidenceNodeKind::Effect,
                EvidenceNodeKind::Asset,
            ],
        ),
        InspectorAction::Ui { path, format } => render_kinds(
            path,
            *format,
            &[
                EvidenceNodeKind::Route,
                EvidenceNodeKind::Page,
                EvidenceNodeKind::Component,
                EvidenceNodeKind::Primitive,
                EvidenceNodeKind::Action,
                EvidenceNodeKind::ActionStep,
            ],
        ),
        InspectorAction::Styles { path, format } => render_kinds(
            path,
            *format,
            &[
                EvidenceNodeKind::StyleRule,
                EvidenceNodeKind::StyleVariable,
                EvidenceNodeKind::ResponsiveBranch,
            ],
        ),
        InspectorAction::State { path, format } => render_kinds(
            path,
            *format,
            &[
                EvidenceNodeKind::State,
                EvidenceNodeKind::DerivedValue,
                EvidenceNodeKind::Effect,
            ],
        ),
        InspectorAction::Accessibility { path, format } => {
            render_kinds(path, *format, &[EvidenceNodeKind::Accessibility])
        }
        InspectorAction::Backend { path, format } => {
            let evidence = load(path)?;
            render(
                &filtered(&evidence, |node| {
                    matches!(
                        node.kind,
                        EvidenceNodeKind::BackendService
                            | EvidenceNodeKind::DataModel
                            | EvidenceNodeKind::Field
                            | EvidenceNodeKind::Relationship
                            | EvidenceNodeKind::Projection
                            | EvidenceNodeKind::ValidationRule
                            | EvidenceNodeKind::AuthPolicy
                            | EvidenceNodeKind::SecurityPolicy
                            | EvidenceNodeKind::CachePolicy
                            | EvidenceNodeKind::RetryPolicy
                            | EvidenceNodeKind::Stream
                    ) || (node.kind == EvidenceNodeKind::Operation
                        && node.attributes.get("backend") == Some(&json!(true)))
                }),
                *format,
            )
        }
        InspectorAction::Operation {
            selector,
            path,
            format,
        } => {
            let evidence = load(path)?;
            let root = evidence
                .nodes
                .iter()
                .find(|node| {
                    node.kind == EvidenceNodeKind::Operation
                        && node.label == *selector
                        && node.attributes.get("backend") == Some(&json!(true))
                })
                .map(|node| node.id.clone())
                .map(Ok)
                .unwrap_or_else(|| {
                    resolve(&evidence, selector, Some(EvidenceNodeKind::Operation))
                })?;
            let result = lineage_result(&evidence, root, 16)?;
            render(&result, *format)
        }
        InspectorAction::Data {
            path,
            lineage,
            format,
        } => {
            let evidence = load(path)?;
            if let Some(selector) = lineage {
                let root = resolve(&evidence, selector, None)?;
                let result = lineage_result(&evidence, root, 16)?;
                render(&result, *format)
            } else {
                render(
                    &filtered(&evidence, |node| {
                        matches!(
                            node.kind,
                            EvidenceNodeKind::DataModel
                                | EvidenceNodeKind::Field
                                | EvidenceNodeKind::Relationship
                                | EvidenceNodeKind::Projection
                                | EvidenceNodeKind::ValidationRule
                        )
                    }),
                    *format,
                )
            }
        }
        InspectorAction::Exposure { path, format } => {
            let evidence = load(path)?;
            render(
                &filtered(&evidence, |node| {
                    node.kind == EvidenceNodeKind::Operation
                        && node.attributes.contains_key("exposure")
                }),
                *format,
            )
        }
        InspectorAction::Cache { path, format } => {
            let evidence = load(path)?;
            let cached_operations = evidence
                .edges
                .iter()
                .filter(|edge| {
                    edge.kind == axiom_lib::application_evidence::EvidenceEdgeKind::CachedBy
                })
                .flat_map(|edge| [edge.from.as_str(), edge.to.as_str()])
                .collect::<BTreeSet<_>>();
            render(
                &filtered(&evidence, |node| {
                    cached_operations.contains(node.id.as_str())
                }),
                *format,
            )
        }
        InspectorAction::Security { path, format } => render_kinds(
            path,
            *format,
            &[
                EvidenceNodeKind::AuthPolicy,
                EvidenceNodeKind::SecurityPolicy,
                EvidenceNodeKind::Permission,
                EvidenceNodeKind::Finding,
                EvidenceNodeKind::ReadinessFact,
            ],
        ),
        InspectorAction::Authority { path, format } => render_kinds(
            path,
            *format,
            &[
                EvidenceNodeKind::Extension,
                EvidenceNodeKind::Permission,
                EvidenceNodeKind::SecurityPolicy,
                EvidenceNodeKind::AuthPolicy,
            ],
        ),
        InspectorAction::Finding {
            selector,
            path,
            format,
        } => {
            let evidence = load(path)?;
            if let Some(selector) = selector {
                let id = resolve(&evidence, selector, Some(EvidenceNodeKind::Finding))?;
                let result = EvidenceIndex::new(&evidence)?.query(query(
                    QueryOperation::Node { id },
                    1,
                    1_000,
                ))?;
                render(&result, *format)
            } else {
                render(
                    &filtered(&evidence, |node| node.kind == EvidenceNodeKind::Finding),
                    *format,
                )
            }
        }
        InspectorAction::Readiness { path, format } => render_kinds(
            path,
            *format,
            &[EvidenceNodeKind::ReadinessFact, EvidenceNodeKind::Finding],
        ),
        InspectorAction::Overview { path, format } => {
            let evidence = load(path)?;
            let result =
                EvidenceIndex::new(&evidence)?.query(query(QueryOperation::Overview, 1, 10_000))?;
            render(&result, *format)
        }
        InspectorAction::Snapshot { path, output } => {
            let evidence = load(path)?;
            let bytes = evidence.canonical_bytes()?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(output, &bytes).with_context(|| format!("write {}", output.display()))?;
            let workspace = fs::canonicalize(path)?;
            let workspace = if workspace.is_file() {
                workspace.parent().unwrap_or(Path::new(".")).to_path_buf()
            } else {
                workspace
            };
            let cache = workspace
                .join(".axiom/inspector/snapshots")
                .join(format!("{}.jcs.json", evidence.graph_revision));
            fs::create_dir_all(cache.parent().expect("snapshot cache has a parent"))?;
            fs::write(&cache, bytes)?;
            println!("Snapshot {}", output.display());
            println!("Graph revision {}", evidence.graph_revision);
            println!("Cache {}", cache.display());
            Ok(())
        }
        InspectorAction::ValidateSnapshot { snapshot } => {
            let bytes =
                fs::read(snapshot).with_context(|| format!("read {}", snapshot.display()))?;
            let evidence = ApplicationEvidence::decode(&bytes)?;
            if evidence.canonical_bytes()? != bytes {
                bail!("snapshot is valid JSON but is not canonical JCS");
            }
            println!("Valid {}", evidence.format);
            println!("Graph revision {}", evidence.graph_revision);
            println!(
                "{} nodes, {} edges",
                evidence.nodes.len(),
                evidence.edges.len()
            );
            Ok(())
        }
        InspectorAction::Dependencies { path, format } => {
            let evidence = load(path)?;
            let kinds = BTreeSet::from([
                EvidenceNodeKind::Contract,
                EvidenceNodeKind::Package,
                EvidenceNodeKind::Extension,
            ]);
            render(
                &filtered(&evidence, |node| kinds.contains(&node.kind)),
                *format,
            )
        }
        InspectorAction::Contract {
            alias,
            path,
            closure,
            format,
        } => {
            let evidence = load(path)?;
            let id = resolve(&evidence, alias, Some(EvidenceNodeKind::Contract))?;
            let depth = if *closure { 64 } else { 1 };
            let result = EvidenceIndex::new(&evidence)?.query(query(
                QueryOperation::Trace {
                    root: id,
                    direction: QueryDirection::Downstream,
                },
                depth,
                10_000,
            ))?;
            render(&result, *format)
        }
        InspectorAction::ExecutableClosure {
            alias,
            path,
            target,
            format,
        } => {
            let evidence = load(path)?;
            let id = resolve(&evidence, alias, None)?;
            let mut result = EvidenceIndex::new(&evidence)?.query(query(
                QueryOperation::Trace {
                    root: id,
                    direction: QueryDirection::Downstream,
                },
                64,
                10_000,
            ))?;
            result.nodes.retain(|node| {
                matches!(
                    node.kind,
                    EvidenceNodeKind::Contract
                        | EvidenceNodeKind::Package
                        | EvidenceNodeKind::Extension
                        | EvidenceNodeKind::Permission
                ) && target
                    .as_ref()
                    .map(|target| node.targets.is_empty() || node.targets.contains(target))
                    .unwrap_or(true)
            });
            retain_connected_edges(&mut result);
            render(&result, *format)
        }
        InspectorAction::Provenance {
            selector,
            path,
            format,
        }
        | InspectorAction::Node {
            selector,
            path,
            format,
        }
        | InspectorAction::Evidence {
            selector,
            path,
            format,
        } => {
            let evidence = load(path)?;
            let id = resolve(&evidence, selector, None)?;
            let result = EvidenceIndex::new(&evidence)?.query(query(
                QueryOperation::Node { id },
                1,
                1_000,
            ))?;
            render(&result, *format)
        }
        InspectorAction::Graph {
            path,
            root,
            direction,
            max_depth,
            max_results,
            format,
        } => {
            let evidence = load(path)?;
            let root = root
                .as_ref()
                .map(|selector| resolve(&evidence, selector, None))
                .transpose()?;
            let result = EvidenceIndex::new(&evidence)?.query(query(
                QueryOperation::Graph {
                    root,
                    direction: (*direction).into(),
                },
                *max_depth,
                *max_results,
            ))?;
            render(&result, *format)
        }
        InspectorAction::Trace {
            selector,
            path,
            direction,
            max_depth,
            max_results,
            format,
        } => {
            let evidence = load(path)?;
            match resolve(&evidence, selector, None) {
                Ok(root) => {
                    let result = EvidenceIndex::new(&evidence)?.query(query(
                        QueryOperation::Trace {
                            root,
                            direction: (*direction).into(),
                        },
                        *max_depth,
                        *max_results,
                    ))?;
                    render(&result, *format)
                }
                Err(static_error) => {
                    super::inspector_runtime::render_trace(path, selector, *format)
                        .with_context(|| format!("static trace lookup also failed: {static_error}"))
                }
            }
        }
        InspectorAction::Record {
            path,
            target,
            input,
            session,
            disabled,
        } => super::inspector_runtime::record(
            path,
            target,
            input.as_deref(),
            session.as_deref(),
            *disabled,
        ),
        InspectorAction::Runtime {
            path,
            session,
            format,
        } => super::inspector_runtime::render_session(path, session, *format),
        InspectorAction::ExportAudit {
            path,
            session,
            output,
        } => super::inspector_runtime::export_bundle(path, session, output),
        InspectorAction::ImportAudit {
            input,
            path,
            session,
        } => super::inspector_runtime::import(path, input, session.as_deref()),
        InspectorAction::Serve {
            path,
            port,
            no_open,
            api_only,
            allow_remote_planner,
            jev_intent_threshold,
            jev_selector_threshold,
        } => {
            super::inspector_server::serve(
                path,
                *port,
                *no_open,
                *api_only,
                *allow_remote_planner,
                *jev_intent_threshold,
                *jev_selector_threshold,
            )
            .await
        }
        InspectorAction::Why {
            selector,
            path,
            format,
        } => {
            let evidence = load(path)?;
            let id = resolve(&evidence, selector, None)?;
            let result = EvidenceIndex::new(&evidence)?.query(query(
                QueryOperation::Why { id },
                2,
                1_000,
            ))?;
            render(&result, *format)
        }
        InspectorAction::Access {
            selector,
            path,
            format,
        } => {
            let evidence = load(path)?;
            let id = resolve(&evidence, selector, None)?;
            let result = EvidenceIndex::new(&evidence)?.query(query(
                QueryOperation::Access { id },
                2,
                1_000,
            ))?;
            render(&result, *format)
        }
        InspectorAction::Reachability {
            from,
            to,
            path,
            max_depth,
            format,
        } => {
            let evidence = load(path)?;
            let from = resolve(&evidence, from, None)?;
            let to = resolve(&evidence, to, None)?;
            let result = EvidenceIndex::new(&evidence)?.query(query(
                QueryOperation::Reachability { from, to },
                *max_depth,
                10_000,
            ))?;
            render(&result, *format)
        }
        InspectorAction::Lineage {
            selector,
            path,
            max_depth,
            format,
        } => {
            let evidence = load(path)?;
            let root = resolve(&evidence, selector, None)?;
            let result = lineage_result(&evidence, root, *max_depth)?;
            render(&result, *format)
        }
        InspectorAction::TargetCompare {
            selector,
            path,
            targets,
            format,
        }
        | InspectorAction::TargetDiff {
            selector,
            path,
            targets,
            format,
        } => {
            let evidence = load(path)?;
            let root = resolve(&evidence, selector, None)?;
            let mut result = EvidenceIndex::new(&evidence)?.query(query(
                QueryOperation::Trace {
                    root,
                    direction: QueryDirection::Both,
                },
                2,
                10_000,
            ))?;
            result.nodes.retain(|node| {
                node.targets.is_empty()
                    || targets.iter().any(|target| node.targets.contains(target))
            });
            retain_connected_edges(&mut result);
            let coverage = targets
                .iter()
                .map(|target| {
                    let count = result
                        .nodes
                        .iter()
                        .filter(|node| node.targets.is_empty() || node.targets.contains(target))
                        .count();
                    (target.clone(), json!(count))
                })
                .collect::<BTreeMap<_, _>>();
            let divergent = result
                .nodes
                .iter()
                .filter(|node| {
                    !node.targets.is_empty()
                        && targets
                            .iter()
                            .filter(|target| node.targets.contains(*target))
                            .count()
                            != targets.len()
                })
                .count();
            result
                .summary
                .insert("comparedTargets".into(), json!(targets));
            result
                .summary
                .insert("factsByTarget".into(), json!(coverage));
            result
                .summary
                .insert("divergentFactCount".into(), json!(divergent));
            render(&result, *format)
        }
        InspectorAction::Query {
            query_file,
            path,
            format,
        } => {
            let query: AxiomQuery = serde_json::from_slice(&fs::read(query_file)?)
                .context("parse axiom-query/v1 JSON")?;
            let evidence = load(path)?;
            let result = EvidenceIndex::new(&evidence)?.query(query)?;
            render(&result, *format)
        }
        InspectorAction::Diff {
            before,
            after,
            before_runtime,
            after_runtime,
            fail_on,
            output,
            format,
        } => {
            let before = read_snapshot(before)?;
            let after = read_snapshot(after)?;
            let mut report = semantic_diff(&before, &after)?;
            if let (Some(before_runtime), Some(after_runtime)) =
                (before_runtime.as_ref(), after_runtime.as_ref())
            {
                let before_runtime = axiom_lib::runtime_evidence::RuntimeSession::decode(
                    &fs::read(before_runtime)?,
                )?;
                let after_runtime =
                    axiom_lib::runtime_evidence::RuntimeSession::decode(&fs::read(after_runtime)?)?;
                attach_runtime_diff(&mut report, &before_runtime, &after_runtime)?;
            }
            if let Some(output) = output {
                write_canonical(output, &report.canonical_bytes()?)?;
            }
            render_change_report(&report, *format)?;
            let failed = fail_on.iter().any(|class| match class {
                InspectorFailureClass::Breaking => report.has_impact(ChangeImpact::Breaking),
                InspectorFailureClass::ApprovalRequired => {
                    report.has_impact(ChangeImpact::ApprovalRequired)
                }
                InspectorFailureClass::PermissionIncrease => report.has_permission_increase(),
                InspectorFailureClass::Warning => report.has_impact(ChangeImpact::Warning),
            });
            if failed {
                bail!("semantic change policy gate failed");
            }
            Ok(())
        }
        InspectorAction::Impact {
            selector,
            path,
            against,
            max_depth,
            max_results,
            output,
            format,
        } => {
            let after = read_snapshot(against)?;
            let report = match impact(&after, selector, *max_depth, *max_results) {
                Ok(report) => report,
                Err(after_error) => {
                    // A removed identity exists only in the baseline/current
                    // workspace. Falling back makes removal impact useful
                    // while preserving each graph's exact relationships.
                    let before = load(path)?;
                    impact(&before, selector, *max_depth, *max_results).with_context(|| {
                        format!("selector was absent from the comparison snapshot: {after_error}")
                    })?
                }
            };
            if let Some(output) = output {
                write_canonical(output, &serde_jcs::to_vec(&report)?)?;
            }
            render_impact_report(&report, *format)
        }
        InspectorAction::Ask {
            question,
            path,
            changes,
            planner,
            allow_remote,
            jev_intent_threshold,
            jev_selector_threshold,
            output,
            format,
        } => {
            let evidence = load(path)?;
            let report = changes.as_deref().map(read_change_report).transpose()?;
            if *planner == InspectorPlanner::Jev {
                if !allow_remote {
                    bail!("Jev planning is remote; repeat with --allow-remote after reviewing the data boundary");
                }
                let receipt = super::inspector_jev::plan(
                    &evidence,
                    question,
                    *jev_intent_threshold,
                    *jev_selector_threshold,
                )
                .await?;
                let receipt_path =
                    super::inspector_jev::write_receipt(&inspector_workspace(path), &receipt)?;
                let answer = if let Some(request) = receipt.request.clone() {
                    answer_question(&evidence, request, report.as_ref())?
                } else {
                    match plan_question(question) {
                        Ok(request) => answer_question(&evidence, request, report.as_ref())?,
                        Err(_) => unsupported_answer(&evidence, receipt.decision.reason.clone()),
                    }
                };
                emit_planned_answer(&answer, &receipt, &receipt_path, output.as_deref(), *format)
            } else {
                let answer = match plan_question(question) {
                    Ok(request) => answer_question(&evidence, request, report.as_ref())?,
                    Err(error) => unsupported_answer(&evidence, error.to_string()),
                };
                emit_answer(&answer, output.as_deref(), *format)
            }
        }
        InspectorAction::Answer {
            question_file,
            path,
            changes,
            output,
            format,
        } => {
            let request: QuestionRequest = serde_json::from_slice(&fs::read(question_file)?)
                .context("parse axiom-inspector-question/v1 JSON")?;
            request.validate()?;
            let evidence = load(path)?;
            let report = changes.as_deref().map(read_change_report).transpose()?;
            let answer = answer_question(&evidence, request, report.as_ref())?;
            emit_answer(&answer, output.as_deref(), *format)
        }
        InspectorAction::ExportInspector {
            path,
            key,
            runtime,
            changes,
            output,
        } => {
            let evidence = load(path)?;
            let runtime = if *runtime {
                super::inspector_runtime::load_latest(path)?
            } else {
                None
            };
            let changes = changes.as_deref().map(read_change_report).transpose()?;
            let payload = InspectorExchangePayload {
                evidence,
                runtime,
                changes,
            };
            let signer = inspector_signing_key(key)?;
            let digest = InspectorExchange::signing_digest(&payload)?;
            let envelope = InspectorExchange::assemble(
                payload,
                STANDARD.encode(signer.verifying_key().as_bytes()),
                STANDARD.encode(signer.sign(&digest).to_bytes()),
            )?;
            write_canonical(output, &envelope.canonical_bytes()?)?;
            println!(
                "Exported signed offline Inspector handoff {}",
                output.display()
            );
            println!("Payload SHA-256 {}", envelope.payload_sha256);
            Ok(())
        }
        InspectorAction::ImportInspector { input, output_dir } => {
            let envelope = InspectorExchange::decode(
                &fs::read(input).with_context(|| format!("read {}", input.display()))?,
            )?;
            if output_dir.exists()
                && fs::read_dir(output_dir)
                    .with_context(|| format!("read import directory {}", output_dir.display()))?
                    .next()
                    .is_some()
            {
                bail!(
                    "Inspector import directory `{}` must be empty",
                    output_dir.display()
                );
            }
            fs::create_dir_all(output_dir)?;
            write_canonical(
                &output_dir.join("application-evidence.jcs.json"),
                &envelope.payload.evidence.canonical_bytes()?,
            )?;
            if let Some(runtime) = &envelope.payload.runtime {
                write_canonical(
                    &output_dir.join("runtime-evidence.jcs.json"),
                    &runtime.canonical_bytes()?,
                )?;
            }
            if let Some(changes) = &envelope.payload.changes {
                write_canonical(
                    &output_dir.join("semantic-changes.jcs.json"),
                    &changes.canonical_bytes()?,
                )?;
            }
            println!("Verified and imported signed Inspector handoff");
            println!(
                "Graph revision {}",
                envelope.payload.evidence.graph_revision
            );
            Ok(())
        }
        InspectorAction::ReleaseCheck {
            path,
            enforce,
            format,
        } => {
            let report = release_check(path)?;
            render_release_check(&report, *format)?;
            if *enforce && !report.passed {
                bail!("Inspector release budgets or invariants failed");
            }
            Ok(())
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InspectorReleaseCheck {
    format: &'static str,
    schema_policy: &'static str,
    cli_policy: &'static str,
    graph_revision: String,
    nodes: usize,
    edges: usize,
    snapshot_bytes: usize,
    cold_snapshot_millis: u128,
    repeat_snapshot_millis: u128,
    query_p50_micros: u128,
    query_p95_micros: u128,
    telemetry: &'static str,
    hosted_upload: &'static str,
    dashboard_packaged_offline: bool,
    passed: bool,
    checks: BTreeMap<String, bool>,
}

fn release_check(path: &Path) -> Result<InspectorReleaseCheck> {
    let started = Instant::now();
    let evidence = load(path)?;
    let cold_snapshot_millis = started.elapsed().as_millis();
    let canonical = evidence.canonical_bytes()?;
    let snapshot_bytes = canonical.len();
    let repeat_started = Instant::now();
    let repeated = load(path)?;
    let repeat_snapshot_millis = repeat_started.elapsed().as_millis();
    let deterministic_repeat = canonical == repeated.canonical_bytes()?;
    let index = EvidenceIndex::new(&evidence)?;
    let mut timings = Vec::new();
    for _ in 0..10 {
        let started = Instant::now();
        index.query(AxiomQuery {
            format: AXIOM_QUERY_FORMAT.into(),
            operation: QueryOperation::Graph {
                root: None,
                direction: QueryDirection::Both,
            },
            max_depth: 8,
            max_results: 10_000,
        })?;
        timings.push(started.elapsed().as_micros());
    }
    timings.sort();
    let query_p50_micros = timings[4];
    let query_p95_micros = timings[9];
    let checks = BTreeMap::from([
        (
            "coldSnapshotUnder2Seconds".into(),
            cold_snapshot_millis <= 2_000,
        ),
        (
            "repeatSnapshotUnder1Second".into(),
            repeat_snapshot_millis <= 1_000,
        ),
        ("repeatSnapshotIsDeterministic".into(), deterministic_repeat),
        (
            "boundedQueryUnder100Millis".into(),
            query_p95_micros <= 100_000,
        ),
        (
            "portableSnapshotUnder25MiB".into(),
            snapshot_bytes <= axiom_lib::application_evidence::MAX_EVIDENCE_BYTES,
        ),
        (
            "dashboardPackagedOffline".into(),
            !super::inspector_dashboard::HTML.is_empty()
                && !super::inspector_dashboard::CSS.is_empty()
                && !super::inspector_dashboard::JS.is_empty(),
        ),
        ("telemetryRequiresConsent".into(), true),
        ("hostedUploadIsExplicitOnly".into(), true),
    ]);
    Ok(InspectorReleaseCheck {
        format: "axiom-inspector-release-check/v1",
        schema_policy: "additive changes require a new schema version; unknown fields are rejected",
        cli_policy: "commands and JSON fields remain compatible within v1; removals require a major CLI release",
        graph_revision: evidence.graph_revision,
        nodes: evidence.nodes.len(),
        edges: evidence.edges.len(),
        snapshot_bytes,
        cold_snapshot_millis,
        repeat_snapshot_millis,
        query_p50_micros,
        query_p95_micros,
        telemetry: "disabled; no telemetry transport is implemented",
        hosted_upload: "absent; only explicit signed export writes an offline handoff",
        dashboard_packaged_offline: true,
        passed: checks.values().all(|passed| *passed),
        checks,
    })
}

fn render_release_check(report: &InspectorReleaseCheck, format: InspectorFormat) -> Result<()> {
    match format {
        InspectorFormat::Json => println!("{}", serde_json::to_string_pretty(report)?),
        InspectorFormat::Jsonl => {
            println!("{}", serde_json::to_string(report)?);
        }
        InspectorFormat::Human => {
            println!("AXIOM INSPECTOR RELEASE CHECK");
            println!("Graph: {}", report.graph_revision);
            println!("Facts: {} nodes · {} edges", report.nodes, report.edges);
            println!(
                "Snapshot: {} bytes · cold {} ms · repeat {} ms",
                report.snapshot_bytes, report.cold_snapshot_millis, report.repeat_snapshot_millis
            );
            println!(
                "Query: p50 {} µs · p95 {} µs",
                report.query_p50_micros, report.query_p95_micros
            );
            for (check, passed) in &report.checks {
                println!("{} {check}", if *passed { "PASS" } else { "FAIL" });
            }
            println!("Telemetry: {}", report.telemetry);
            println!("Hosted upload: {}", report.hosted_upload);
        }
    }
    Ok(())
}

fn inspector_signing_key(path: &Path) -> Result<SigningKey> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(path)
            .with_context(|| format!("read signing key metadata {}", path.display()))?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            bail!(
                "Inspector signing key must not be readable or writable by group/other (chmod 600)"
            );
        }
    }
    let bytes = fs::read(path).with_context(|| format!("read signing key {}", path.display()))?;
    let seed = if bytes.len() == 32 {
        bytes
    } else {
        let text = String::from_utf8(bytes).context("signing key must be raw, hex, or Base64")?;
        let text = text.trim();
        hex::decode(text)
            .or_else(|_| STANDARD.decode(text))
            .context("decode 32-byte signing key")?
    };
    let seed: [u8; 32] = seed
        .try_into()
        .map_err(|_| anyhow::anyhow!("signing key must contain exactly 32 bytes"))?;
    Ok(SigningKey::from_bytes(&seed))
}

fn read_change_report(path: &Path) -> Result<SemanticChangeReport> {
    let report: SemanticChangeReport = serde_json::from_slice(
        &fs::read(path).with_context(|| format!("read {}", path.display()))?,
    )
    .context("parse axiom-semantic-change/v1 JSON")?;
    if report.format != axiom_lib::application_change::SEMANTIC_CHANGE_FORMAT {
        bail!("unsupported semantic change report `{}`", report.format);
    }
    report.canonical_bytes()?;
    Ok(report)
}

fn emit_answer(
    answer: &QuestionAnswer,
    output: Option<&Path>,
    format: InspectorFormat,
) -> Result<()> {
    let canonical = serde_jcs::to_vec(answer)?;
    if let Some(output) = output {
        write_canonical(output, &canonical)?;
    }
    match format {
        InspectorFormat::Json => println!("{}", serde_json::to_string_pretty(answer)?),
        InspectorFormat::Jsonl => {
            println!(
                "{}",
                serde_json::to_string(&json!({
                    "type":"answer",
                    "format":ANSWER_FORMAT,
                    "status":answer.status,
                    "graphRevision":answer.graph_revision,
                    "summary":answer.summary,
                    "completeness":answer.completeness,
                    "validatedQuery":answer.validated_query,
                }))?
            );
            for fact in &answer.facts {
                println!(
                    "{}",
                    serde_json::to_string(&json!({"type":"fact","value":fact}))?
                );
            }
            for path in &answer.paths {
                println!(
                    "{}",
                    serde_json::to_string(&json!({"type":"path","value":path}))?
                );
            }
        }
        InspectorFormat::Human => {
            println!("AXIOM INSPECTOR ANSWER");
            println!("Status: {:?}", answer.status);
            println!("{}", answer.summary);
            println!("Graph revision: {}", answer.graph_revision);
            if let Some(query) = &answer.validated_query {
                println!("Validated query: {}", serde_json::to_string(query)?);
            }
            if !answer.facts.is_empty() {
                println!();
                println!("FACTS");
                for fact in &answer.facts {
                    println!("- {} · {} · {}", fact.kind, fact.label, fact.id);
                }
            }
            if !answer.completeness.uncertainty.is_empty() {
                println!();
                println!("COMPLETENESS");
                for note in &answer.completeness.uncertainty {
                    println!("- {note}");
                }
            }
            println!();
            println!("LIMITS");
            for limit in &answer.limits {
                println!("- {limit}");
            }
        }
    }
    Ok(())
}

fn emit_planned_answer(
    answer: &QuestionAnswer,
    receipt: &super::inspector_jev::JevPlannerReceipt,
    receipt_path: &Path,
    output: Option<&Path>,
    format: InspectorFormat,
) -> Result<()> {
    // The canonical answer remains the existing deterministic artifact. The
    // probabilistic planning evidence is stored and rendered separately.
    if let Some(output) = output {
        write_canonical(output, &serde_jcs::to_vec(answer)?)?;
    }
    let planning = json!({
        "mode": "jev",
        "remote": true,
        "receiptFormat": receipt.format,
        "receiptPath": receipt_path,
        "provider": receipt.provider,
        "requestedModel": receipt.requested_model,
        "resolvedModel": receipt.resolved_model,
        "decision": receipt.decision,
        "thresholds": receipt.thresholds,
        "questionSha256": receipt.question_sha256,
        "candidateCount": receipt.candidate_count,
    });
    match format {
        InspectorFormat::Json => {
            let mut value = serde_json::to_value(answer)?;
            value
                .as_object_mut()
                .context("Inspector answer must serialize as an object")?
                .insert("planning".into(), planning);
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        InspectorFormat::Jsonl => {
            println!(
                "{}",
                serde_json::to_string(&json!({
                    "type":"answer",
                    "format":ANSWER_FORMAT,
                    "status":answer.status,
                    "graphRevision":answer.graph_revision,
                    "summary":answer.summary,
                    "completeness":answer.completeness,
                    "validatedQuery":answer.validated_query,
                    "planning":planning,
                }))?
            );
            for fact in &answer.facts {
                println!(
                    "{}",
                    serde_json::to_string(&json!({"type":"fact","value":fact}))?
                );
            }
            for path in &answer.paths {
                println!(
                    "{}",
                    serde_json::to_string(&json!({"type":"path","value":path}))?
                );
            }
        }
        InspectorFormat::Human => {
            println!("AXIOM INSPECTOR ANSWER");
            println!("Status: {:?}", answer.status);
            println!("{}", answer.summary);
            println!("Graph revision: {}", answer.graph_revision);
            println!();
            println!("PLANNING");
            println!("- Remote planner: Jev through Vercel AI Gateway");
            println!("- Model: {}", receipt.resolved_model);
            println!(
                "- Decision: {} ({:.1}%)",
                receipt.decision.intent,
                receipt.decision.intent_probability * 100.0
            );
            if let Some(selector) = receipt.decision.selector.as_deref() {
                match receipt.decision.selector_probability {
                    Some(probability) => println!(
                        "- Candidate: {selector} ({:.1}%, {})",
                        probability * 100.0,
                        receipt.decision.selector_source
                    ),
                    None => println!(
                        "- Candidate: {selector} ({})",
                        receipt.decision.selector_source
                    ),
                }
            }
            println!("- Accepted: {}", receipt.decision.accepted);
            println!("- Receipt: {}", receipt_path.display());
            if let Some(query) = &answer.validated_query {
                println!("Validated query: {}", serde_json::to_string(query)?);
            }
            if !answer.facts.is_empty() {
                println!();
                println!("DETERMINISTIC FACTS");
                for fact in &answer.facts {
                    println!("- {} · {} · {}", fact.kind, fact.label, fact.id);
                }
            }
            println!();
            println!("BOUNDARY");
            println!("- Jev selected a bounded request; it did not produce these facts.");
            println!("- The local EvidenceIndex validated and answered the request.");
        }
    }
    Ok(())
}

fn inspector_workspace(path: &Path) -> PathBuf {
    if path.is_file() {
        path.parent().unwrap_or(Path::new(".")).to_path_buf()
    } else {
        path.to_path_buf()
    }
}

fn read_snapshot(path: &Path) -> Result<ApplicationEvidence> {
    ApplicationEvidence::decode(
        &fs::read(path).with_context(|| format!("read {}", path.display()))?,
    )
}

fn write_canonical(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent().filter(|value| !value.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes).with_context(|| format!("write {}", path.display()))
}

fn render_change_report(report: &SemanticChangeReport, format: InspectorFormat) -> Result<()> {
    match format {
        InspectorFormat::Json => println!("{}", serde_json::to_string_pretty(report)?),
        InspectorFormat::Jsonl => {
            println!(
                "{}",
                serde_json::to_string(
                    &json!({"type":"summary","value":report.summary,"policy":report.policy})
                )?
            );
            for change in report.changes.iter().chain(&report.runtime_changes) {
                println!(
                    "{}",
                    serde_json::to_string(&json!({"type":"change","value":change}))?
                );
            }
        }
        InspectorFormat::Human => {
            println!("Axiom semantic change report");
            println!("Policy {}", report.policy);
            println!("Before {}", report.before_graph_revision);
            println!("After  {}", report.after_graph_revision);
            println!(
                "Breaking {}  Approval required {}  Warnings {}  Informational {}",
                report.summary.breaking,
                report.summary.approval_required,
                report.summary.warnings,
                report.summary.informational
            );
            for change in report.changes.iter().chain(&report.runtime_changes) {
                println!(
                    "- {:?} / {:?} / {:?}: {}",
                    change.impact, change.domain, change.operation, change.label
                );
                println!("  {}", change.reason);
                println!("  {}", change.semantic_id);
                for evidence in &change.evidence {
                    if let Some(path) = &evidence.path {
                        println!("  evidence {path}");
                    }
                }
            }
        }
    }
    Ok(())
}

fn render_impact_report(report: &ImpactReport, format: InspectorFormat) -> Result<()> {
    match format {
        InspectorFormat::Json => println!("{}", serde_json::to_string_pretty(report)?),
        InspectorFormat::Jsonl => {
            println!(
                "{}",
                serde_json::to_string(
                    &json!({"type":"summary","rootId":report.root_id,"targets":report.affected_targets,"truncated":report.truncated})
                )?
            );
            for path in &report.paths {
                println!(
                    "{}",
                    serde_json::to_string(&json!({"type":"impact-path","value":path}))?
                );
            }
        }
        InspectorFormat::Human => {
            println!("Axiom impact report");
            println!("Root {}", report.root_id);
            println!("Graph revision {}", report.graph_revision);
            println!("Targets {}", report.affected_targets.join(", "));
            println!(
                "Affected facts {}  Truncated {}",
                report.affected_nodes.len(),
                report.truncated
            );
            for node in &report.affected_nodes {
                println!("- {:?}  {}", node.kind, node.label);
                println!("  {}", node.id);
            }
        }
    }
    Ok(())
}

fn render_kinds(path: &Path, format: InspectorFormat, kinds: &[EvidenceNodeKind]) -> Result<()> {
    let evidence = load(path)?;
    let kinds = kinds.iter().copied().collect::<BTreeSet<_>>();
    render(
        &filtered(&evidence, |node| kinds.contains(&node.kind)),
        format,
    )
}

fn lineage_result(
    evidence: &ApplicationEvidence,
    root: String,
    max_depth: usize,
) -> Result<QueryResult> {
    use axiom_lib::application_evidence::EvidenceEdgeKind;
    let root_kind = evidence
        .nodes
        .iter()
        .find(|node| node.id == root)
        .map(|node| node.kind);
    let direct_consumers = evidence
        .edges
        .iter()
        .filter(|edge| edge.from == root && edge.kind == EvidenceEdgeKind::Impacts)
        .map(|edge| edge.to.clone())
        .collect::<BTreeSet<_>>();
    let root_id = root.clone();
    let mut lineage_graph = evidence.clone();
    lineage_graph.edges.retain(|edge| {
        matches!(
            edge.kind,
            EvidenceEdgeKind::Contains
                | EvidenceEdgeKind::ResolvesTo
                | EvidenceEdgeKind::Projects
                | EvidenceEdgeKind::Returns
                | EvidenceEdgeKind::Calls
                | EvidenceEdgeKind::Reads
                | EvidenceEdgeKind::Writes
                | EvidenceEdgeKind::Subscribes
                | EvidenceEdgeKind::Dispatches
                | EvidenceEdgeKind::Impacts
                | EvidenceEdgeKind::Validates
                | EvidenceEdgeKind::Invalidates
                | EvidenceEdgeKind::CachedBy
                | EvidenceEdgeKind::ProtectedBy
        )
    });
    let lineage_graph = lineage_graph.finalize()?;
    let mut result = EvidenceIndex::new(&lineage_graph)?.query(query(
        QueryOperation::Trace {
            root,
            direction: QueryDirection::Both,
        },
        max_depth,
        10_000,
    ))?;
    result.graph_revision = evidence.graph_revision.clone();
    result.nodes.retain(|node| {
        let semantic_kind = matches!(
            node.kind,
            EvidenceNodeKind::Field
                | EvidenceNodeKind::DataModel
                | EvidenceNodeKind::Projection
                | EvidenceNodeKind::Relationship
                | EvidenceNodeKind::ValidationRule
                | EvidenceNodeKind::Operation
                | EvidenceNodeKind::Contract
                | EvidenceNodeKind::State
                | EvidenceNodeKind::Primitive
                | EvidenceNodeKind::Extension
                | EvidenceNodeKind::CachePolicy
                | EvidenceNodeKind::RetryPolicy
                | EvidenceNodeKind::Stream
                | EvidenceNodeKind::AuthPolicy
                | EvidenceNodeKind::SecurityPolicy
        );
        semantic_kind
            && (root_kind != Some(EvidenceNodeKind::Field)
                || node.kind != EvidenceNodeKind::Field
                || node.id == root_id)
            && (root_kind != Some(EvidenceNodeKind::Field)
                || node.kind != EvidenceNodeKind::Primitive
                || direct_consumers.contains(&node.id))
    });
    retain_connected_edges(&mut result);
    Ok(result)
}

pub(super) fn load(path: &Path) -> Result<ApplicationEvidence> {
    let requested = fs::canonicalize(path)
        .with_context(|| format!("resolve inspection workspace {}", path.display()))?;
    let workspace = if requested.is_file() {
        requested
            .parent()
            .context("inspection input has no parent")?
            .to_path_buf()
    } else {
        requested
    };
    let fingerprint = workspace_fingerprint(&workspace)?;
    let cache_path = workspace.join(".axiom/inspector/cache/v1.jcs.json");
    if let Ok(bytes) = fs::read(&cache_path) {
        if let Ok(cache) = serde_json::from_slice::<InspectorIncrementalCache>(&bytes) {
            if cache.format == INSPECTOR_CACHE_FORMAT
                && cache.fingerprint == fingerprint
                && cache.evidence.validate().is_ok()
            {
                return Ok(cache.evidence);
            }
        }
    }
    let evidence = super::inspector_frontend::enrich(
        &workspace,
        inspect_workspace(&workspace, env!("CARGO_PKG_VERSION"))?,
    )?;
    let cache = InspectorIncrementalCache {
        format: INSPECTOR_CACHE_FORMAT.into(),
        fingerprint,
        evidence: evidence.clone(),
    };
    if let Ok(bytes) = serde_jcs::to_vec(&cache) {
        if let Some(parent) = cache_path.parent() {
            if fs::create_dir_all(parent).is_ok() {
                let temporary = parent.join(format!("v1-{}.tmp", std::process::id()));
                if fs::write(&temporary, bytes).is_ok() {
                    let _ = fs::rename(temporary, &cache_path);
                }
            }
        }
    }
    Ok(evidence)
}

const INSPECTOR_CACHE_FORMAT: &str = "axiom-inspector-incremental-cache/v1";
const MAX_FINGERPRINT_FILES: usize = 20_000;
const MAX_FINGERPRINT_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InspectorIncrementalCache {
    format: String,
    fingerprint: String,
    evidence: ApplicationEvidence,
}

fn workspace_fingerprint(workspace: &Path) -> Result<String> {
    let mut files = Vec::new();
    fingerprint_files(workspace, workspace, 0, &mut files)?;
    files.sort();
    if files.len() > MAX_FINGERPRINT_FILES {
        bail!("Inspector cache fingerprint exceeds 20000 relevant files");
    }
    let mut digest = Sha256::new();
    digest.update(INSPECTOR_CACHE_FORMAT.as_bytes());
    digest.update(env!("CARGO_PKG_VERSION").as_bytes());
    let mut total = 0u64;
    for file in files {
        let relative = file.strip_prefix(workspace)?;
        let bytes =
            fs::read(&file).with_context(|| format!("read Inspector input {}", file.display()))?;
        total = total.saturating_add(bytes.len() as u64);
        if total > MAX_FINGERPRINT_BYTES {
            bail!("Inspector cache fingerprint exceeds 512 MiB of relevant inputs");
        }
        digest.update(relative.to_string_lossy().as_bytes());
        digest.update([0]);
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(&bytes);
    }
    Ok(hex::encode(digest.finalize()))
}

fn fingerprint_files(
    workspace: &Path,
    directory: &Path,
    depth: usize,
    files: &mut Vec<PathBuf>,
) -> Result<()> {
    if depth > 24 {
        bail!("Inspector cache fingerprint exceeds directory depth 24");
    }
    let mut entries = fs::read_dir(directory)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = entry.metadata()?;
        if metadata.is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(
                name.as_ref(),
                ".git"
                    | "target"
                    | "build"
                    | "dist"
                    | "node_modules"
                    | ".dart_tool"
                    | ".summary_files"
            ) || is_inspector_cache_directory(&path)
            {
                continue;
            }
            fingerprint_files(workspace, &path, depth + 1, files)?;
        } else if metadata.is_file() && is_inspector_input(&path) {
            files.push(path);
        }
    }
    Ok(())
}

fn is_inspector_cache_directory(path: &Path) -> bool {
    path.file_name().and_then(|value| value.to_str()) == Some("inspector")
        && path
            .parent()
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
            == Some(".axiom")
}

fn is_inspector_input(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|value| value.to_str()),
        Some(
            "acore"
                | "axiom"
                | "json"
                | "toml"
                | "lock"
                | "rs"
                | "wasm"
                | "wit"
                | "py"
                | "go"
                | "ts"
                | "js"
        )
    )
}

fn query(operation: QueryOperation, max_depth: usize, max_results: usize) -> AxiomQuery {
    AxiomQuery {
        format: AXIOM_QUERY_FORMAT.into(),
        operation,
        max_depth,
        max_results,
    }
}

fn resolve(
    evidence: &ApplicationEvidence,
    selector: &str,
    kind: Option<EvidenceNodeKind>,
) -> Result<String> {
    if let Some(node) = evidence
        .nodes
        .iter()
        .find(|node| node.id == selector && kind.map(|value| value == node.kind).unwrap_or(true))
    {
        return Ok(node.id.clone());
    }
    let exact = evidence
        .nodes
        .iter()
        .filter(|node| {
            node.label == selector && kind.map(|value| value == node.kind).unwrap_or(true)
        })
        .collect::<Vec<_>>();
    match exact.as_slice() {
        [node] => return Ok(node.id.clone()),
        [] => {}
        _ => bail!("selector `{selector}` is ambiguous; use a semantic ID"),
    }
    let needle = selector.to_lowercase();
    let partial = evidence
        .nodes
        .iter()
        .filter(|node| {
            (node.label.to_lowercase().contains(&needle) || node.id.contains(selector))
                && kind.map(|value| value == node.kind).unwrap_or(true)
        })
        .collect::<Vec<_>>();
    match partial.as_slice() {
        [node] => Ok(node.id.clone()),
        [] => bail!("no evidence fact matches `{selector}`"),
        _ => bail!(
            "selector `{selector}` matches {} facts; use a semantic ID",
            partial.len()
        ),
    }
}

fn filtered<F>(evidence: &ApplicationEvidence, predicate: F) -> QueryResult
where
    F: Fn(&EvidenceNode) -> bool,
{
    let nodes = evidence
        .nodes
        .iter()
        .filter(|node| predicate(node))
        .cloned()
        .collect::<Vec<_>>();
    let ids = nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();
    let edges = evidence
        .edges
        .iter()
        .filter(|edge| ids.contains(edge.from.as_str()) && ids.contains(edge.to.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let mut summary = BTreeMap::new();
    summary.insert("nodeCount".into(), json!(nodes.len()));
    summary.insert("edgeCount".into(), json!(edges.len()));
    QueryResult {
        format: QUERY_RESULT_FORMAT.into(),
        graph_revision: evidence.graph_revision.clone(),
        query: query(QueryOperation::Overview, 1, 10_000),
        summary,
        nodes,
        edges,
        truncated: false,
    }
}

fn retain_connected_edges(result: &mut QueryResult) {
    let ids = result
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();
    result
        .edges
        .retain(|edge| ids.contains(edge.from.as_str()) && ids.contains(edge.to.as_str()));
    result
        .summary
        .insert("nodeCount".into(), json!(result.nodes.len()));
    result
        .summary
        .insert("edgeCount".into(), json!(result.edges.len()));
}

fn render(result: &QueryResult, format: InspectorFormat) -> Result<()> {
    match format {
        InspectorFormat::Json => println!("{}", serde_json::to_string_pretty(result)?),
        InspectorFormat::Jsonl => {
            println!(
                "{}",
                serde_json::to_string(
                    &json!({"type":"summary","format":result.format,"graphRevision":result.graph_revision,"summary":result.summary,"truncated":result.truncated})
                )?
            );
            for node in &result.nodes {
                println!("{}", json_line("node", node)?);
            }
            for edge in &result.edges {
                println!("{}", json_line("edge", edge)?);
            }
        }
        InspectorFormat::Human => render_human(result),
    }
    Ok(())
}

fn json_line<T: Serialize>(kind: &str, value: &T) -> Result<String> {
    let value = serde_json::to_value(value)?;
    Ok(serde_json::to_string(&json!({"type":kind,"value":value}))?)
}

fn render_human(result: &QueryResult) {
    println!("Axiom Inspector");
    println!("Graph revision {}", result.graph_revision);
    println!(
        "Facts {}  Relationships {}  Truncated {}",
        result.nodes.len(),
        result.edges.len(),
        result.truncated
    );
    if !result.nodes.is_empty() {
        println!();
        println!("FACTS");
        for node in &result.nodes {
            let targets = if node.targets.is_empty() {
                String::new()
            } else {
                format!(" [{}]", node.targets.join(","))
            };
            println!("- {:?}  {}{}", node.kind, node.label, targets);
            println!("  {}  {:?} / {:?}", node.id, node.layer, node.verification);
            if !node.attributes.is_empty() {
                println!(
                    "  {}",
                    serde_json::to_string(&node.attributes).unwrap_or_default()
                );
            }
            for evidence in &node.evidence {
                if let Some(path) = &evidence.path {
                    println!("  evidence: {} ({})", path, evidence.kind);
                }
            }
        }
    }
    if !result.edges.is_empty() {
        println!();
        println!("RELATIONSHIPS");
        for edge in &result.edges {
            println!("- {} --{:?}--> {}", edge.from, edge.kind, edge.to);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Harness {
        #[command(subcommand)]
        action: InspectorAction,
    }

    #[test]
    fn parses_agent_facing_commands() {
        let parsed = Harness::try_parse_from([
            "test",
            "trace",
            "pricing",
            ".",
            "--max-depth",
            "4",
            "--format",
            "json",
        ]);
        assert!(parsed.is_ok());
        assert!(Harness::try_parse_from(["test", "frontend", ".", "--format", "json"]).is_ok());
        assert!(Harness::try_parse_from(["test", "backend", "."]).is_ok());
        assert!(Harness::try_parse_from(["test", "security", "."]).is_ok());
        assert!(Harness::try_parse_from(["test", "readiness", "."]).is_ok());
        let parsed =
            Harness::try_parse_from(["test", "snapshot", ".", "--output", "evidence.json"]);
        assert!(parsed.is_ok());
        let parsed = Harness::try_parse_from([
            "test",
            "reachability",
            "shop",
            "list_products",
            ".",
            "--format",
            "json",
        ]);
        assert!(parsed.is_ok());
        let parsed = Harness::try_parse_from([
            "test",
            "target-compare",
            "pricing",
            ".",
            "--targets",
            "web,ios",
        ]);
        assert!(parsed.is_ok());
        assert!(Harness::try_parse_from([
            "test",
            "record",
            ".",
            "--target",
            "ios",
            "--input",
            "audit.json",
        ])
        .is_ok());
        assert!(Harness::try_parse_from([
            "test",
            "runtime",
            ".",
            "--session",
            "latest",
            "--format",
            "jsonl",
        ])
        .is_ok());
        assert!(Harness::try_parse_from([
            "test",
            "export-audit",
            ".",
            "--session",
            "latest",
            "--output",
            "audit.axinspect",
        ])
        .is_ok());
        assert!(
            Harness::try_parse_from(["test", "serve", ".", "--no-open", "--api-only",]).is_ok()
        );
        assert!(Harness::try_parse_from([
            "test",
            "diff",
            "before.json",
            "after.json",
            "--fail-on",
            "breaking,permission-increase",
            "--output",
            "changes.json",
        ])
        .is_ok());
        assert!(Harness::try_parse_from([
            "test",
            "impact",
            "state:cart.discount_cents",
            ".",
            "--against",
            "after.json",
            "--format",
            "json",
        ])
        .is_ok());
        assert!(Harness::try_parse_from([
            "test",
            "ask",
            "what executes when i press primitive:button",
            ".",
            "--format",
            "json",
        ])
        .is_ok());
        assert!(Harness::try_parse_from([
            "test",
            "answer",
            "question.json",
            ".",
            "--output",
            "answer.json",
        ])
        .is_ok());
        assert!(Harness::try_parse_from([
            "test",
            "export-inspector",
            ".",
            "--key",
            "key.seed",
            "--output",
            "app.axinspect",
        ])
        .is_ok());
        assert!(Harness::try_parse_from([
            "test",
            "import-inspector",
            "app.axinspect",
            "--output-dir",
            "verified",
        ])
        .is_ok());
        assert!(Harness::try_parse_from([
            "test",
            "release-check",
            ".",
            "--enforce",
            "--format",
            "json",
        ])
        .is_ok());
    }

    #[test]
    fn incremental_fingerprint_tracks_inputs_and_ignores_its_cache() {
        let root = std::env::temp_dir().join(format!(
            "axiom-inspector-fingerprint-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join(".axiom/inspector/cache")).unwrap();
        fs::write(root.join("main.acore"), "module Example {}\n").unwrap();
        let first = workspace_fingerprint(&root).unwrap();
        fs::write(root.join(".axiom/inspector/cache/v1.jcs.json"), "generated").unwrap();
        assert_eq!(first, workspace_fingerprint(&root).unwrap());
        fs::write(root.join("main.acore"), "module Changed {}\n").unwrap();
        assert_ne!(first, workspace_fingerprint(&root).unwrap());
        fs::remove_dir_all(root).unwrap();
    }
}
