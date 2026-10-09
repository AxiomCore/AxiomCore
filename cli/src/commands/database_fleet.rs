//! Frozen Axiom fleet protocol. No CLI path invokes Atlas, a shell hook or login.
use anyhow::Result;
use axiom_database::delivery::{self, Artifact};
use clap::{Subcommand, ValueEnum};
use std::{collections::HashMap, path::PathBuf};
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Stage {
    Dev,
    Staging,
    Production,
}
impl From<Stage> for delivery::Stage {
    fn from(s: Stage) -> Self {
        match s {
            Stage::Dev => Self::Dev,
            Stage::Staging => Self::Staging,
            Stage::Production => Self::Production,
        }
    }
}
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Kind {
    Source,
    Security,
    Operations,
}
impl From<Kind> for delivery::ReviewKind {
    fn from(k: Kind) -> Self {
        match k {
            Kind::Source => Self::Source,
            Kind::Security => Self::Security,
            Kind::Operations => Self::Operations,
        }
    }
}
#[derive(Subcommand, Debug)]
pub enum Action {
    /// Freeze an explicit private file inventory, signed histories and custom policy offline
    Prepare {
        #[arg(long)]
        inventory: PathBuf,
        #[arg(long)]
        policy: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Regenerate every target's signed source/history and inspect masked typed impact
    Inspect { artifact: PathBuf },
    /// Sign an exact source-owner/security/operations review with bounded expiry
    Approve {
        artifact: PathBuf,
        #[arg(long)]
        reviewer: String,
        #[arg(long, value_enum)]
        kind: Kind,
        #[arg(long)]
        key_file: PathBuf,
        #[arg(long, default_value_t = 3600)]
        ttl_seconds: u64,
    },
    /// Verify all required policy reviews without native database effects
    Verify {
        artifact: PathBuf,
        #[arg(long)]
        trusted_policy_sha256: String,
    },
    /// Transport exactly the same private bytes and reviews; no cloud login
    Export {
        artifact: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Expand one parsed PostgreSQL schema sqlName before planning; never interpolate SQL
    Expand {
        source: PathBuf,
        #[arg(long)]
        schema: String,
        #[arg(long)]
        tenant: String,
        #[arg(long)]
        sql_name: String,
        #[arg(long)]
        out: PathBuf,
    },
    /// Apply/reconcile a frozen stage with native checks and durable private events
    #[command(aliases=["ci","operator","terraform-apply"])]
    Run {
        artifact: PathBuf,
        #[arg(long)]
        inventory: PathBuf,
        #[arg(long)]
        providers: PathBuf,
        #[arg(long)]
        journal: PathBuf,
        #[arg(long, value_enum, default_value = "dev")]
        stage: Stage,
        #[arg(long)]
        promotion: Option<PathBuf>,
        #[arg(long)]
        production_admission: Option<PathBuf>,
        #[arg(long)]
        receipt_key: PathBuf,
        #[arg(long)]
        trusted_policy_sha256: String,
    },
    /// Fresh read-only drift/history/checkpoint reports; no migration or repair
    Monitor {
        artifact: PathBuf,
        #[arg(long)]
        inventory: PathBuf,
        #[arg(long)]
        providers: PathBuf,
        #[arg(long)]
        journal: PathBuf,
        #[arg(long, value_enum, default_value = "dev")]
        stage: Stage,
        #[arg(long)]
        receipt_key: PathBuf,
        #[arg(long)]
        trusted_policy_sha256: String,
    },
    /// Promote immutable artifact bytes using a fresh complete signed stage receipt
    Promote {
        artifact: PathBuf,
        #[arg(long)]
        receipt: PathBuf,
        #[arg(long, value_enum)]
        to: Stage,
        #[arg(long)]
        trusted_policy_sha256: String,
        #[arg(long)]
        key_file: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Write a self-contained masked column/impact/lineage dashboard with no network assets
    Dashboard {
        artifact: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Terraform external-data protocol: a flat string map, containing no authority
    TerraformPlan { artifact: PathBuf },
    /// Compare a populated, isolated physical restore with the exact enrolled checkpoint
    RecoveryCheck {
        artifact: PathBuf,
        #[arg(long)]
        member: String,
        #[arg(long)]
        restored_binding: PathBuf,
        #[arg(long)]
        providers: PathBuf,
        #[arg(long)]
        provider_evidence: PathBuf,
        #[arg(long)]
        trusted_policy_sha256: String,
        #[arg(long)]
        key_file: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Sign one-hour production admission from exact build and native recovery receipts
    AdmitProduction {
        artifact: PathBuf,
        #[arg(long, required = true)]
        recovery: Vec<PathBuf>,
        #[arg(long)]
        trusted_policy_sha256: String,
        #[arg(long)]
        key_file: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
}
pub async fn run(action: &Action) -> Result<()> {
    let mut incomplete = false;
    let value = match action {
        Action::Prepare {
            inventory,
            policy,
            out,
        } => delivery::prepare(inventory, policy, out)?,
        Action::Inspect { artifact } => delivery::inspect(&Artifact::load(artifact)?)?,
        Action::Approve {
            artifact,
            reviewer,
            kind,
            key_file,
            ttl_seconds,
        } => {
            anyhow::ensure!(
                (1..=604800).contains(ttl_seconds),
                "ADB806: review lifetime exceeds budget"
            );
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs();
            delivery::approve(
                artifact,
                reviewer,
                (*kind).into(),
                key_file,
                now + ttl_seconds,
            )?
        }
        Action::Verify {
            artifact,
            trusted_policy_sha256,
        } => {
            let a = Artifact::load(artifact)?;
            delivery::check_reviews(artifact, &a, trusted_policy_sha256)?;
            serde_json::json!({"format":"axiom-database-fleet-verification/v1","artifactSha256":a.sha256()?,"reviews":"verified","effects":"offline-only","migrationAuthority":false})
        }
        Action::Export { artifact, out } => delivery::export(artifact, out)?,
        Action::Expand {
            source,
            schema,
            tenant,
            sql_name,
            out,
        } => {
            let (_, input) = acore::database::project::load(source, &HashMap::new())
                .map_err(anyhow::Error::msg)?;
            anyhow::ensure!(
                input.packages.is_empty(),
                "ADB811: tenant template package imports require a separately proved expansion slice"
            );
            let (expanded, report) = delivery::expand_schema(&input, schema, tenant, sql_name)?;
            delivery::io::dir(out, true)?;
            let temp = delivery::io::materialize(&expanded.files)?;
            for (path, bytes) in delivery::io::tree(temp.path())? {
                let destination = out.join(path);
                let parent = destination.parent().unwrap();
                if parent != out {
                    let mut p = out.clone();
                    for c in parent.strip_prefix(out)?.components() {
                        p.push(c);
                        if !p.exists() {
                            delivery::io::dir(&p, true)?;
                        } else {
                            delivery::io::dir(&p, false)?;
                        }
                    }
                }
                delivery::io::write_bytes(&destination, bytes.as_bytes())?;
            }
            delivery::io::write(&out.join("tenant-expansion.json"), &report)?;
            report
        }
        Action::Run {
            artifact,
            inventory,
            providers,
            journal,
            stage,
            promotion,
            production_admission,
            receipt_key,
            trusted_policy_sha256,
        } => {
            let options = delivery::RunOptions {
                inventory,
                providers,
                journal,
                stage: (*stage).into(),
                monitor: false,
                promotion: promotion.as_deref(),
                receipt_key,
                trusted_policy_sha256,
            };
            let v = if let Some(admission) = production_admission {
                delivery::run_production(artifact, options, admission).await?
            } else {
                delivery::run(artifact, options).await?
            };
            incomplete = v["complete"] != true;
            v
        }
        Action::Monitor {
            artifact,
            inventory,
            providers,
            journal,
            stage,
            receipt_key,
            trusted_policy_sha256,
        } => {
            delivery::run(
                artifact,
                delivery::RunOptions {
                    inventory,
                    providers,
                    journal,
                    stage: (*stage).into(),
                    monitor: true,
                    promotion: None,
                    receipt_key,
                    trusted_policy_sha256,
                },
            )
            .await?
        }
        Action::Promote {
            artifact,
            receipt,
            to,
            trusted_policy_sha256,
            key_file,
            out,
        } => delivery::promote(
            artifact,
            receipt,
            (*to).into(),
            trusted_policy_sha256,
            key_file,
            out,
        )?,
        Action::Dashboard { artifact, out } => {
            let a = Artifact::load(artifact)?;
            delivery::dashboard(&a, out)?;
            serde_json::json!({"format":"axiom-database-fleet-dashboard/v1","artifactSha256":a.sha256()?,"networkAssets":0,"masking":"allowlisted-digests-and-operation-kinds"})
        }
        Action::TerraformPlan { artifact } => {
            let a = Artifact::load(artifact)?;
            serde_json::json!({"artifact_sha256":a.sha256()?,"inventory_sha256":a.inventory.sha256()?,"target_count":a.targets.len().to_string(),"profile":delivery::PROFILE,"migration_authority":"false"})
        }
        Action::RecoveryCheck {
            artifact,
            member,
            restored_binding,
            providers,
            provider_evidence,
            trusted_policy_sha256,
            key_file,
            out,
        } => {
            delivery::production::recovery_check(
                artifact,
                delivery::production::RecoveryOptions {
                    member,
                    restored_binding,
                    providers,
                    provider_evidence,
                    key: key_file,
                    out,
                    trusted_policy_sha256,
                },
            )
            .await?
        }
        Action::AdmitProduction {
            artifact,
            recovery,
            trusted_policy_sha256,
            key_file,
            out,
        } => delivery::production::admit(artifact, recovery, trusted_policy_sha256, key_file, out)?,
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    anyhow::ensure!(
        !incomplete,
        "ADB808: fleet rollout is incomplete; inspect masked outcomes and explicitly repair/reconcile failed targets"
    );
    Ok(())
}
