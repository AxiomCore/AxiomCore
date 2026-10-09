//! IDE adapters use the same saved-input snapshot as the LSP. No extractor,
//! service, database connection, dependency fetch or artifact build is implied.
use anyhow::Result;
use clap::{Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Clone, Debug, ValueEnum)]
pub enum Kind {
    Backend,
    Frontend,
    Database,
    Config,
    Package,
    Extension,
    Detached,
}
impl Kind {
    fn wire(&self) -> axiom_lib::editor_protocol::ProjectKind {
        use axiom_lib::editor_protocol::ProjectKind as K;
        match self {
            Self::Backend => K::Backend,
            Self::Frontend => K::Frontend,
            Self::Database => K::Database,
            Self::Config => K::Config,
            Self::Package => K::Package,
            Self::Extension => K::Extension,
            Self::Detached => K::Detached,
        }
    }
}

#[derive(Subcommand)]
pub enum Action {
    /// Read-only saved private-input and graph report; no provider is contacted
    Report {
        #[arg(long)] entry: PathBuf,
        #[arg(long)] root: PathBuf,
        #[arg(long,value_enum)] kind: Kind,
        #[arg(long="target")] targets: Vec<String>,
        #[arg(long)] variant: Option<String>,
        #[arg(long)] json: bool,
    },
    /// Describe explicit editor tasks using this CLI's actual argument contract
    Workflows { #[arg(long)] json: bool },
    /// Validate a typed workflow request without executing it
    ValidateWorkflow { #[arg(long)] request: String, #[arg(long)] root: PathBuf },
    /// Explicitly run declared native extractors and persist the fenced cache
    RefreshExtraction {
        #[arg(long)]
        entry: PathBuf,
        #[arg(long)]
        root: PathBuf,
        #[arg(long, value_enum)]
        kind: Kind,
        #[arg(long)]
        variant: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Advertise the exact saved-check contract supported by this CLI
    Info {
        #[arg(long)]
        json: bool,
    },
    /// Check saved source/manifests statically with the owning compiler adapters
    Check {
        #[arg(long)]
        entry: PathBuf,
        #[arg(long)]
        root: PathBuf,
        #[arg(long, value_enum)]
        kind: Kind,
        #[arg(long = "target")]
        targets: Vec<String>,
        #[arg(long)]
        variant: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

pub fn handle(action: &Action) -> Result<()> {
    use axiom_lib::editor_protocol::*;
    match action {
        Action::Report {entry,root,kind,targets,variant,..}=>println!("{}",serde_json::to_string_pretty(&acore::editor::check::report(entry,root,kind.wire(),targets.clone(),variant.clone())?)?),
        Action::Workflows { .. } => println!("{}", super::editor_workflows::catalog()),
        Action::ValidateWorkflow { request, root } => println!("{}", super::editor_workflows::validate(request, root)?),
        Action::RefreshExtraction {
            entry,
            root,
            kind,
            variant,
            json,
        } => {
            let result =
                acore::editor::preparation::refresh(entry, root, kind.wire(), variant.clone())?;
            if *json {
                println!("{}", serde_json::to_string(&result)?);
            } else {
                println!("Refreshed saved native interfaces in {}", root.display());
            }
        }
        Action::Info { json } => {
            let info = EditorCliInfo {
                format: CLI_INFO_FORMAT.into(),
                cli_version: env!("CARGO_PKG_VERSION").into(),
                protocol_version: ProtocolVersion::V1,
                check_format: CHECK_FORMAT.into(),
                compiler_version: acore::editor::project::compiler_identity(),
            };
            if *json {
                println!("{}", serde_json::to_string(&info)?);
            } else {
                println!(
                    "Axiom {} · {} · {}",
                    info.cli_version, PROTOCOL_VERSION, CHECK_FORMAT
                );
            }
        }
        Action::Check {
            entry,
            root,
            kind,
            targets,
            variant,
            json,
        } => {
            let report = acore::editor::check::saved(
                entry,
                root,
                kind.wire(),
                targets.clone(),
                variant.clone(),
            )?;
            if *json {
                println!("{}", serde_json::to_string(&report)?);
            } else {
                println!(
                    "Saved static check: {} · {} · {}",
                    entry.display(),
                    report.project.profile,
                    report.project.variant
                );
                for d in &report.diagnostics {
                    println!(
                        "{}:{}..{}: {}: {}",
                        d.origin.uri, d.origin.span.start, d.origin.span.end, d.code, d.message
                    );
                }
            }
            anyhow::ensure!(
                report.valid,
                "saved static check failed; no artifacts were written"
            );
        }
    }
    Ok(())
}
