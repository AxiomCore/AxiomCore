//! Shared execution command contract; real serve uses these types.
//! Phases 1/2 must consume these types instead of inventing another flag set.
use anyhow::{bail, ensure, Context, Result};
use axiom_lib::backend::ServingMode;
use clap::{Args, ValueEnum};
use std::{
    net::IpAddr,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Args)]
pub struct CommonInput {
    /// .acore source or .axiom artifact; defaults to local axiom.acore
    pub input: Option<PathBuf>,
    /// Exact private companion plan (artifact input only)
    #[arg(long)]
    pub plan: Option<PathBuf>,
    /// Reviewed deployment; defaults to adjacent deployment.json for real serving
    #[arg(long)]
    pub deployment: Option<PathBuf>,
    /// Explicit, hash-bound sandbox implementation references
    #[arg(long)]
    pub bindings: Option<PathBuf>,
    /// Explicit exact-plan durable delivery configuration; real serving only.
    #[arg(long)]
    pub delivery: Option<PathBuf>,
    /// Private typed mock fixtures; invalid in real mode
    #[arg(long)]
    pub fixtures: Option<PathBuf>,
    /// Reviewed mock identities; defaults to adjacent mock-identities.json
    #[arg(long)]
    pub identities: Option<PathBuf>,
    /// Source variant; artifact variants are immutable
    #[arg(long)]
    pub variant: Option<String>,
    /// Reload local source/fixtures while retaining mode and last valid service
    #[arg(long)]
    pub watch: bool,
    /// Listener IP address
    #[arg(long, default_value = "127.0.0.1")]
    pub host: IpAddr,
    /// Listener port; 0 selects an ephemeral port outside watch
    #[arg(short, long, default_value_t = 8080)]
    pub port: u16,
    /// Redacted diagnostics; never raw credentials or request/response bodies.
    #[arg(short, long)]
    pub debug: bool,
}

#[derive(Clone, Debug, Args)]
pub struct ServeInput {
    #[command(flatten)]
    pub common: CommonInput,
    /// Simulate business execution inside the real server boundary
    #[arg(long)]
    pub mock: bool,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum, PartialEq, Eq)]
pub enum MockProfile {
    #[default]
    Auto,
    Server,
    Contract,
}

#[derive(Clone, Debug, Args)]
pub struct MockInput {
    #[command(flatten)]
    pub common: CommonInput,
    #[arg(long)]
    /// Explicit Cloud contract mocks; requires a linked project and Cloud session
    pub cloud: bool,
    #[arg(long, value_enum, default_value = "auto")]
    /// Auto follows compiled plan presence; server never downgrades to contract
    pub profile: MockProfile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileIntent {
    Real,
    FullMock,
    AutoMock,
    ContractMock,
    CloudMock,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreparationInput {
    Source {
        path: PathBuf,
        variant: Option<String>,
    },
    Artifact {
        contract: PathBuf,
        plan: Option<PathBuf>,
    },
    Cloud,
}

/// Resolution reads metadata only. No compiler, credentials, providers, cloud
/// request, process or listener is started. Source classification follows later.
#[derive(Clone, Debug)]
pub struct ResolvedInput {
    pub profile: ProfileIntent,
    pub input: PreparationInput,
    pub deployment: Option<PathBuf>,
    pub bindings: Option<PathBuf>,
    pub delivery: Option<PathBuf>,
    pub fixtures: Option<PathBuf>,
    /// Reviewed mock identities; defaults to adjacent mock-identities.json
    pub identities: Option<PathBuf>,
    pub host: IpAddr,
    pub port: u16,
    pub watch: bool,
    pub debug: bool,
}

impl ServeInput {
    pub fn resolve(&self, cwd: &Path) -> Result<ResolvedInput> {
        resolve(
            &self.common,
            if self.mock {
                ProfileIntent::FullMock
            } else {
                ProfileIntent::Real
            },
            cwd,
        )
    }
}
impl MockInput {
    pub fn resolve(&self, cwd: &Path) -> Result<ResolvedInput> {
        let profile = if self.cloud {
            ensure!(
                self.profile != MockProfile::Server,
                "cloud mocks only support the contract profile"
            );
            ProfileIntent::CloudMock
        } else {
            match self.profile {
                MockProfile::Auto => ProfileIntent::AutoMock,
                MockProfile::Server => ProfileIntent::FullMock,
                MockProfile::Contract => ProfileIntent::ContractMock,
            }
        };
        resolve(&self.common, profile, cwd)
    }
}

impl ResolvedInput {
    /// Caller supplies compiler-derived plan presence, never a raw regex hint.
    /// This selects the profile; it does not certify its runtime capabilities.
    pub fn mode(&self, has_server_plan: bool) -> Result<ServingMode> {
        if has_server_plan
            && matches!(
                self.profile,
                ProfileIntent::FullMock | ProfileIntent::AutoMock
            )
        {
            ensure!(
                self.fixtures.is_some(),
                "server mocks require --fixtures or adjacent mock.json"
            );
        }
        match self.profile {
            ProfileIntent::Real => Ok(ServingMode::Real),
            ProfileIntent::FullMock if has_server_plan => Ok(ServingMode::ServerMock),
            ProfileIntent::FullMock => bail!("full server mocks require a server plan; use `axiom mock --profile contract` for contract-only mocks"),
            ProfileIntent::AutoMock if has_server_plan => Ok(ServingMode::ServerMock),
            ProfileIntent::AutoMock | ProfileIntent::ContractMock | ProfileIntent::CloudMock => {
                ensure!(!has_server_plan, "contract mock profile cannot ignore a private server plan");
                ensure!(self.fixtures.is_none() && self.identities.is_none(), "private server fixtures/identities require a server plan");
                Ok(ServingMode::ContractMock)
            }
        }
    }
}

fn resolve(args: &CommonInput, profile: ProfileIntent, cwd: &Path) -> Result<ResolvedInput> {
    ensure!(
        args.variant
            .as_ref()
            .is_none_or(|v| !v.is_empty() && v.len() <= 128 && !v.chars().any(char::is_control)),
        "variant must be a nonempty bounded name"
    );
    let cloud = profile == ProfileIntent::CloudMock;
    ensure!(
        !cloud
            || (args.input.is_none()
                && args.plan.is_none()
                && args.deployment.is_none()
                && args.bindings.is_none()
                && args.delivery.is_none()
                && args.fixtures.is_none()
                && args.identities.is_none()
                && args.variant.is_none()
                && !args.watch),
        "cloud mocks conflict with local inputs, configuration, variants and watch"
    );
    ensure!(
        profile != ProfileIntent::Real || (args.fixtures.is_none() && args.identities.is_none()),
        "real serving cannot accept mock fixtures or identities"
    );
    // Mock credentials/providers must be separately configured by the mock
    // preparation path. A real deployment or sandbox binding is never reused.
    ensure!(
        matches!(profile, ProfileIntent::Real)
            || (args.deployment.is_none() && args.bindings.is_none() && args.delivery.is_none()),
        "mock mode cannot reuse real deployment or implementation bindings"
    );
    let root = cwd.canonicalize().context("resolve invocation directory")?;
    let file = |path: &Path| -> Result<PathBuf> {
        let candidate = if path.is_absolute() {
            path.to_owned()
        } else {
            root.join(path)
        };
        let metadata = std::fs::symlink_metadata(&candidate)
            .with_context(|| format!("input not found: {}", candidate.display()))?;
        ensure!(
            metadata.is_file(),
            "input must be a regular file: {}",
            candidate.display()
        );
        ensure!(
            metadata.len() <= 16_777_216,
            "input exceeds 16 MiB: {}",
            candidate.display()
        );
        candidate.canonicalize().context("resolve input")
    };
    let input = if cloud {
        PreparationInput::Cloud
    } else {
        let path = file(args.input.as_deref().unwrap_or(Path::new("axiom.acore")))?;
        match path.extension().and_then(|v| v.to_str()) {
            Some("acore") => {
                ensure!(
                    args.plan.is_none(),
                    "source compiles its own private plan; --plan is artifact-only"
                );
                PreparationInput::Source {
                    path,
                    variant: args.variant.clone(),
                }
            }
            Some("axiom") => {
                ensure!(args.variant.is_none(), "artifact variant is immutable");
                ensure!(!args.watch, "watch requires source input");
                let companion = path.with_extension("server.json");
                let plan = match &args.plan {
                    Some(explicit) => Some(file(explicit)?),
                    None if companion.exists() => Some(file(&companion)?),
                    None => None,
                };
                ensure!(
                    profile != ProfileIntent::Real && profile != ProfileIntent::FullMock
                        || plan.is_some(),
                    "artifact serving requires its exact private plan (--plan)"
                );
                PreparationInput::Artifact {
                    contract: path,
                    plan,
                }
            }
            _ => bail!("expected .acore source or .axiom contract"),
        }
    };
    // Explicit auxiliary paths are relative to invocation cwd. Defaults are
    // adjacent to input, never discovered in a different enclosing project.
    let base = match &input {
        PreparationInput::Source { path, .. } => path.parent(),
        PreparationInput::Artifact { contract, .. } => contract.parent(),
        PreparationInput::Cloud => None,
    };
    let auxiliary =
        |explicit: &Option<PathBuf>, default: Option<&str>| -> Result<Option<PathBuf>> {
            if let Some(path) = explicit {
                return file(path).map(Some);
            }
            if let (Some(base), Some(name)) = (base, default) {
                let candidate = base.join(name);
                if candidate.exists() {
                    return file(&candidate).map(Some);
                }
            }
            Ok(None)
        };
    let deployment = auxiliary(
        &args.deployment,
        if profile == ProfileIntent::Real {
            Some("deployment.json")
        } else {
            None
        },
    )?;
    let bindings = auxiliary(&args.bindings, None)?;
    let delivery = auxiliary(&args.delivery, None)?;
    let fixtures = auxiliary(
        &args.fixtures,
        if matches!(profile, ProfileIntent::FullMock | ProfileIntent::AutoMock) {
            Some("mock.json")
        } else {
            None
        },
    )?;
    if matches!(profile, ProfileIntent::FullMock) {
        ensure!(
            fixtures.is_some(),
            "full mock serving requires --fixtures or adjacent mock.json"
        );
    }
    let identities = auxiliary(
        &args.identities,
        if matches!(profile, ProfileIntent::FullMock | ProfileIntent::AutoMock) {
            Some("mock-identities.json")
        } else {
            None
        },
    )?;
    Ok(ResolvedInput {
        profile,
        input,
        deployment,
        bindings,
        delivery,
        fixtures,
        identities,
        host: args.host,
        port: args.port,
        watch: args.watch,
        debug: args.debug,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{Parser, Subcommand};
    #[derive(Parser)]
    struct ContractCli {
        #[command(subcommand)]
        command: ContractCommand,
    }
    #[derive(Subcommand)]
    enum ContractCommand {
        Serve(ServeInput),
        Mock(MockInput),
    }
    fn resolve(argv: &[&str], root: &Path) -> Result<ResolvedInput> {
        match ContractCli::try_parse_from(argv)?.command {
            ContractCommand::Serve(input) => input.resolve(root),
            ContractCommand::Mock(input) => input.resolve(root),
        }
    }
    fn project() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        for name in [
            "axiom.acore",
            "contract.axiom",
            "contract.server.json",
            "deployment.json",
            "mock.json",
        ] {
            std::fs::write(directory.path().join(name), "{}").unwrap();
        }
        directory
    }
    #[test]
    fn command_matrix_resolves_modes_without_starting_any_service() {
        let dir = project();
        for (argv, has_plan, expected) in [
            (vec!["axiom", "serve"], true, ServingMode::Real),
            (
                vec!["axiom", "serve", "axiom.acore", "--mock"],
                true,
                ServingMode::ServerMock,
            ),
            (vec!["axiom", "mock"], true, ServingMode::ServerMock),
            (
                vec!["axiom", "mock", "--profile", "contract"],
                false,
                ServingMode::ContractMock,
            ),
            (
                vec!["axiom", "mock", "--cloud"],
                false,
                ServingMode::ContractMock,
            ),
            (
                vec!["axiom", "serve", "contract.axiom"],
                true,
                ServingMode::Real,
            ),
        ] {
            assert_eq!(
                resolve(&argv, dir.path()).unwrap().mode(has_plan).unwrap(),
                expected
            );
        }
    }
    #[test]
    fn invalid_matrix_rejects_without_provider_or_listener_startup() {
        let dir = project();
        for argv in [
            vec!["axiom", "serve", "--fixtures", "mock.json"],
            vec![
                "axiom",
                "serve",
                "--mock",
                "--deployment",
                "deployment.json",
            ],
            vec![
                "axiom",
                "serve",
                "axiom.acore",
                "--plan",
                "contract.server.json",
            ],
            vec!["axiom", "serve", "contract.axiom", "--watch"],
            vec!["axiom", "serve", "contract.axiom", "--variant", "mobile"],
            vec!["axiom", "mock", "--cloud", "axiom.acore"],
            vec!["axiom", "mock", "--cloud", "--watch"],
            vec!["axiom", "mock", "--cloud", "--profile", "server"],
            vec!["axiom", "serve", "--host", "bad-host"],
            vec!["axiom", "serve", "--port", "65536"],
        ] {
            assert!(resolve(&argv, dir.path()).is_err(), "{argv:?}");
        }
    }
    #[test]
    fn missing_source_never_resolves_cloud_or_mock() {
        let dir = tempfile::tempdir().unwrap();
        assert!(resolve(&["axiom", "serve"], dir.path()).is_err());
        assert!(resolve(&["axiom", "mock"], dir.path()).is_err());
        assert!(resolve(&["axiom", "mock", "--cloud"], dir.path()).is_ok());
    }
    #[test]
    fn full_mock_never_downgrades_or_ignores_a_plan() {
        let dir = project();
        assert!(resolve(&["axiom", "serve", "--mock"], dir.path())
            .unwrap()
            .mode(false)
            .is_err());
        assert!(
            resolve(&["axiom", "mock", "--profile", "contract"], dir.path())
                .unwrap()
                .mode(true)
                .is_err()
        );
        std::fs::remove_file(dir.path().join("mock.json")).unwrap();
        assert!(resolve(&["axiom", "serve", "--mock"], dir.path()).is_err());
        assert!(resolve(&["axiom", "mock"], dir.path())
            .unwrap()
            .mode(true)
            .is_err());
    }
    #[test]
    fn real_serve_and_explicit_mock_profiles_are_available() {
        assert!(crate::Cli::try_parse_from(["axiom", "mock", "axiom.acore"]).is_ok());
        assert!(
            crate::Cli::try_parse_from(["axiom", "mock", "--cloud", "--profile", "contract"])
                .is_ok()
        );
        assert!(crate::Cli::try_parse_from(["axiom", "serve", "axiom.acore", "--mock"]).is_ok());
        assert!(crate::Cli::try_parse_from(["axiom", "serve", "axiom.acore"]).is_ok());
    }
    #[test]
    fn unsafe_files_missing_plans_and_invalid_variants_are_rejected() {
        let dir = project();
        std::fs::remove_file(dir.path().join("contract.server.json")).unwrap();
        assert!(resolve(&["axiom", "serve", "contract.axiom"], dir.path()).is_err());
        assert!(resolve(&["axiom", "serve", "--variant", ""], dir.path()).is_err());
        assert!(resolve(&["axiom", "serve", "--deployment", "."], dir.path()).is_err());
        let large = std::fs::File::create(dir.path().join("large.json")).unwrap();
        large.set_len(16_777_217).unwrap();
        assert!(resolve(
            &["axiom", "serve", "--deployment", "large.json"],
            dir.path()
        )
        .is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("deployment.json", dir.path().join("linked.json")).unwrap();
            assert!(resolve(
                &["axiom", "serve", "--deployment", "linked.json"],
                dir.path()
            )
            .is_err());
        }
    }
    #[test]
    fn explicit_paths_and_adjacent_defaults_do_not_search_parent_projects() {
        let dir = project();
        let child = dir.path().join("nested");
        std::fs::create_dir(&child).unwrap();
        std::fs::write(child.join("service.acore"), "{}").unwrap();
        let result = resolve(
            &[
                "axiom",
                "serve",
                "nested/service.acore",
                "--host",
                "::1",
                "--port",
                "0",
                "--watch",
                "--debug",
            ],
            dir.path(),
        )
        .unwrap();
        assert!(result.deployment.is_none());
        assert!(result.watch && result.debug && result.port == 0 && result.host.is_ipv6());
        let result = resolve(
            &[
                "axiom",
                "serve",
                "nested/service.acore",
                "--deployment",
                "deployment.json",
            ],
            dir.path(),
        )
        .unwrap();
        assert_eq!(
            result.deployment.unwrap(),
            dir.path().join("deployment.json").canonicalize().unwrap()
        );
    }
}
