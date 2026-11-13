//! Dashboard intelligence commands and their exact Cloud API routes.
use anyhow::{bail, Result};
use clap::{Args, Subcommand};
use serde_json::Value;

#[derive(Debug)]
pub struct IntelRequest {
    pub method: &'static str,
    pub path: String,
    pub body: Option<Value>,
    pub destructive: bool,
    pub sensitive: bool,
    pub follow: bool,
}

#[derive(Debug, Subcommand)]
pub enum IntelCommand {
    /// Cloud activity operations.
    Activity {
        #[command(subcommand)]
        action: ActivityAction,
    },
    /// Cloud me operations.
    Me {
        #[command(subcommand)]
        action: MeAction,
    },
    /// Cloud project operations.
    Project {
        #[command(subcommand)]
        action: ProjectAction,
    },
    /// Cloud alerts operations.
    Alerts {
        #[command(subcommand)]
        action: AlertsAction,
    },
    /// Cloud alert policies operations.
    AlertPolicies {
        #[command(subcommand)]
        action: AlertPoliciesAction,
    },
    /// Cloud logs operations.
    Logs {
        #[command(subcommand)]
        action: LogsAction,
    },
    /// Cloud observability operations.
    Observability {
        #[command(subcommand)]
        action: ObservabilityAction,
    },
    /// Cloud pipeline runs operations.
    PipelineRuns {
        #[command(subcommand)]
        action: PipelineRunsAction,
    },
    /// Cloud pull requests operations.
    PullRequests {
        #[command(subcommand)]
        action: PullRequestsAction,
    },
    /// Cloud release channel operations.
    ReleaseChannel {
        #[command(subcommand)]
        action: ReleaseChannelAction,
    },
    /// Cloud security operations.
    Security {
        #[command(subcommand)]
        action: SecurityAction,
    },
    /// Cloud speed insights operations.
    SpeedInsights {
        #[command(subcommand)]
        action: SpeedInsightsAction,
    },
    /// Cloud test runs operations.
    TestRuns {
        #[command(subcommand)]
        action: TestRunsAction,
    },
    /// Cloud traces operations.
    Traces {
        #[command(subcommand)]
        action: TracesAction,
    },
    /// Cloud traffic operations.
    Traffic {
        #[command(subcommand)]
        action: TrafficAction,
    },
    /// Cloud settings operations.
    Settings {
        #[command(subcommand)]
        action: SettingsAction,
    },
    /// Cloud CLI installation operations.
    CliInstalled {
        #[command(subcommand)]
        action: CliInstalledAction,
    },
}

#[derive(Debug, Subcommand)]
pub enum ActivityAction {
    /// List activity.
    List,
    /// Stream activity.
    Stream {
        /// Continue receiving events until interrupted.
        #[arg(long)]
        follow: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum MeAction {
    /// Get me.
    Get,
}

#[derive(Debug, Subcommand)]
pub enum ProjectAction {
    /// Get project.
    Get,
    /// Overview project.
    Overview,
}

#[derive(Debug, Subcommand)]
pub enum AlertsAction {
    /// List alerts.
    List,
}

#[derive(Debug, Subcommand)]
pub enum AlertPoliciesAction {
    /// List alert policies.
    List,
    /// Update alert policies.
    Update {
        policy_kind: String,
        #[command(flatten)]
        fields: AlertPolicyFields,
    },
}

#[derive(Debug, Subcommand)]
pub enum LogsAction {
    /// List logs.
    List {
        /// Continue receiving events until interrupted.
        #[arg(long)]
        follow: bool,
        /// Seconds between log snapshots.
        #[arg(long, default_value_t = 15, value_parser = clap::value_parser!(u64).range(1..))]
        interval: u64,
    },
}

#[derive(Debug, Subcommand)]
pub enum ObservabilityAction {
    /// Get observability.
    Get,
    /// Read or update collection settings.
    Settings {
        #[command(subcommand)]
        action: ObservabilitySettingsAction,
    },
    /// Manage OTLP ingestion sources.
    Sources {
        #[command(subcommand)]
        action: ObservabilitySourcesAction,
    },
}

#[derive(Debug, Subcommand)]
pub enum ObservabilitySettingsAction {
    /// Get observability settings.
    Get,
    /// Update observability settings.
    Update {
        /// API request object, as JSON.
        #[arg(long)]
        body: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum ObservabilitySourcesAction {
    /// List OTLP sources.
    List,
    /// Create OTLP sources.
    Create {
        /// API request object, as JSON.
        #[arg(long)]
        body: String,
    },
    /// Revoke OTLP sources.
    Revoke { source_id: String },
}

#[derive(Debug, Subcommand)]
pub enum PipelineRunsAction {
    /// List pipeline runs.
    List,
    /// Steps pipeline runs.
    Steps { run_id: String },
    /// Attempts pipeline runs.
    Attempts { run_id: String },
}

#[derive(Debug, Subcommand)]
pub enum PullRequestsAction {
    /// List pull requests.
    List,
    /// Get pull requests.
    Get { number: String },
    /// Create pull requests.
    Create {
        /// API request object, as JSON.
        #[arg(long)]
        body: String,
    },
    /// Update pull requests.
    Update {
        number: String,
        /// API request object, as JSON.
        #[arg(long)]
        body: String,
    },
    /// Changes pull requests.
    Changes { number: String },
    /// Comments pull requests.
    Comments { number: String },
    /// Comment pull requests.
    Comment {
        number: String,
        /// API request object, as JSON.
        #[arg(long)]
        body: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum ReleaseChannelAction {
    /// Latest release channel.
    Latest,
}

#[derive(Debug, Subcommand)]
pub enum SecurityAction {
    /// Audit security.
    Audit,
    /// Posture security.
    Posture,
}

#[derive(Debug, Subcommand)]
pub enum SpeedInsightsAction {
    /// Get speed insights.
    Get,
}

#[derive(Debug, Subcommand)]
pub enum TestRunsAction {
    /// List test runs.
    List,
    /// Results test runs.
    Results { test_run_id: String },
    /// Queue test runs.
    Queue {
        /// Signed contract version; omit to test the latest version.
        #[arg(long)]
        version: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum TracesAction {
    /// List traces.
    List,
    /// Get traces.
    Get { trace_id: String },
}

#[derive(Debug, Subcommand)]
pub enum TrafficAction {
    /// List traffic.
    List,
}

#[derive(Debug, Subcommand)]
pub enum SettingsAction {
    /// Update selected settings, preserving unspecified values.
    Update {
        #[command(flatten)]
        fields: ProjectSettingsFields,
    },
}

#[derive(Debug, Args, serde::Serialize)]
pub struct ProjectSettingsFields {
    #[arg(long)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Set an empty string to clear the description.
    #[arg(long)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[arg(long)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
    #[arg(long, value_parser = ["private", "public"])]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
}

#[derive(Debug, Args, serde::Serialize)]
pub struct AlertPolicyFields {
    /// Explicit true/false; omission preserves the existing setting.
    #[arg(long, action = clap::ArgAction::Set)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[arg(long, value_parser = clap::value_parser!(u64).range(60..=86400))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_seconds: Option<u64>,
    #[arg(long, value_parser = finite_nonnegative)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum_samples: Option<u64>,
    #[arg(long, value_parser = ["info", "warning", "critical"])]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum CliInstalledAction {
    /// Confirm CLI installation.
    Confirm,
}

fn body_object(text: &str) -> Result<Value> {
    let value: Value = serde_json::from_str(text)?;
    if !value.is_object() {
        bail!("--body must be a JSON object");
    }
    Ok(value)
}

/// Encode one path segment without allowing slash, query or fragment injection.
fn segment(value: &str) -> Result<String> {
    if value.is_empty() || value == "." || value == ".." {
        bail!("Identifier must be a nonempty path segment");
    }
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    Ok(out)
}

fn finite_nonnegative(input: &str) -> Result<f64, String> {
    let value: f64 = input.parse().map_err(|_| "expected a number".to_owned())?;
    if !value.is_finite() || value < 0.0 {
        return Err("threshold must be a finite nonnegative number".to_owned());
    }
    Ok(value)
}

fn typed_patch(fields: &impl serde::Serialize) -> Result<Value> {
    let value = serde_json::to_value(fields)?;
    if value.as_object().is_none_or(|v| v.is_empty()) {
        bail!("Pass at least one setting to update");
    }
    Ok(value)
}

impl IntelCommand {
    pub fn poll_interval(&self) -> std::time::Duration {
        let seconds = match self {
            Self::Logs {
                action: LogsAction::List { interval, .. },
            } => *interval,
            _ => 15,
        };
        std::time::Duration::from_secs(seconds)
    }

    /// Read current values before calling the backend's replacement-style writes.
    pub fn update_read_path(&self, segments: &[&str]) -> Option<Vec<String>> {
        match self {
            Self::Settings { .. }
            | Self::AlertPolicies {
                action: AlertPoliciesAction::Update { .. },
            } => Some(
                segments[..segments.len() - 1]
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect(),
            ),
            _ => None,
        }
    }

    pub fn merge_current_body(
        &self,
        current: &Value,
        patch: Option<Value>,
    ) -> Result<Option<Value>> {
        let (source, keys): (&Value, &[&str]) = match self {
            Self::Settings { .. } => (
                current,
                &["name", "description", "default_branch", "visibility"],
            ),
            Self::AlertPolicies {
                action: AlertPoliciesAction::Update { policy_kind, .. },
            } => {
                let policies = current
                    .as_array()
                    .ok_or_else(|| anyhow::anyhow!("Invalid alert policy response"))?;
                let policy = policies
                    .iter()
                    .find(|p| p.get("kind").and_then(Value::as_str) == Some(policy_kind))
                    .ok_or_else(|| anyhow::anyhow!("Alert policy not found: {policy_kind}"))?;
                if policy.get("configured").and_then(Value::as_bool) == Some(false) {
                    let required = [
                        "enabled",
                        "window_seconds",
                        "threshold",
                        "minimum_samples",
                        "severity",
                    ];
                    let missing: Vec<_> = required
                        .iter()
                        .filter(|key| patch.as_ref().and_then(|body| body.get(**key)).is_none())
                        .map(|key| format!("--{}", key.replace('_', "-")))
                        .collect();
                    if !missing.is_empty() {
                        bail!("Alert policy {policy_kind} is not configured. Supply all initial values; missing: {}", missing.join(", "));
                    }
                }
                (
                    policy,
                    &[
                        "enabled",
                        "window_seconds",
                        "threshold",
                        "minimum_samples",
                        "severity",
                    ],
                )
            }
            _ => return Ok(patch),
        };
        let mut merged = serde_json::Map::new();
        for key in keys {
            let value = source.get(*key).filter(|v| !v.is_null()).ok_or_else(|| {
                anyhow::anyhow!("Current response is missing {key}; refusing to replace settings")
            })?;
            merged.insert((*key).to_owned(), value.clone());
        }
        if let Some(Value::Object(patch)) = patch {
            for (key, value) in patch {
                if !keys.contains(&key.as_str()) {
                    bail!("Unknown update key: {key}");
                }
                merged.insert(key, value);
            }
        }
        Ok(Some(Value::Object(merged)))
    }

    pub fn request(&self) -> Result<IntelRequest> {
        let (method, path, body, destructive, sensitive, follow) = match self {
            IntelCommand::Activity {
                action: ActivityAction::List,
            } => (
                "GET",
                "/api/v1/activity".to_string(),
                None,
                false,
                false,
                false,
            ),
            IntelCommand::Activity {
                action: ActivityAction::Stream { follow },
            } => (
                "GET",
                "/api/v1/activity/stream".to_string(),
                None,
                false,
                false,
                *follow,
            ),
            IntelCommand::Me {
                action: MeAction::Get,
            } => ("GET", "/api/v1/me".to_string(), None, false, false, false),
            IntelCommand::Project {
                action: ProjectAction::Get,
            } => (
                "GET",
                "/api/v1/projects/{project}".to_string(),
                None,
                false,
                false,
                false,
            ),
            IntelCommand::Project {
                action: ProjectAction::Overview,
            } => (
                "GET",
                "/api/v1/projects/{project}/overview".to_string(),
                None,
                false,
                false,
                false,
            ),
            IntelCommand::Alerts {
                action: AlertsAction::List,
            } => (
                "GET",
                "/api/v1/projects/{project}/alerts".to_string(),
                None,
                false,
                false,
                false,
            ),
            IntelCommand::AlertPolicies {
                action: AlertPoliciesAction::List,
            } => (
                "GET",
                "/api/v1/projects/{project}/alert-policies".to_string(),
                None,
                false,
                false,
                false,
            ),
            IntelCommand::AlertPolicies {
                action:
                    AlertPoliciesAction::Update {
                        policy_kind,
                        fields,
                    },
            } => (
                "PUT",
                "/api/v1/projects/{project}/alert-policies/{id}"
                    .to_string()
                    .replace("{id}", &segment(policy_kind)?),
                Some(typed_patch(fields)?),
                true,
                false,
                false,
            ),
            IntelCommand::Logs {
                action: LogsAction::List { follow, .. },
            } => (
                "GET",
                "/api/v1/projects/{project}/logs".to_string(),
                None,
                false,
                false,
                *follow,
            ),
            IntelCommand::Observability {
                action: ObservabilityAction::Get,
            } => (
                "GET",
                "/api/v1/projects/{project}/observability".to_string(),
                None,
                false,
                false,
                false,
            ),
            IntelCommand::Observability {
                action:
                    ObservabilityAction::Settings {
                        action: ObservabilitySettingsAction::Get,
                    },
            } => (
                "GET",
                "/api/v1/projects/{project}/observability/settings".to_string(),
                None,
                false,
                false,
                false,
            ),
            IntelCommand::Observability {
                action:
                    ObservabilityAction::Settings {
                        action: ObservabilitySettingsAction::Update { body },
                    },
            } => (
                "PUT",
                "/api/v1/projects/{project}/observability/settings".to_string(),
                Some(body_object(body)?),
                true,
                false,
                false,
            ),
            IntelCommand::Observability {
                action:
                    ObservabilityAction::Sources {
                        action: ObservabilitySourcesAction::List,
                    },
            } => (
                "GET",
                "/api/v1/projects/{project}/observability/otlp/sources".to_string(),
                None,
                false,
                false,
                false,
            ),
            IntelCommand::Observability {
                action:
                    ObservabilityAction::Sources {
                        action: ObservabilitySourcesAction::Create { body },
                    },
            } => (
                "POST",
                "/api/v1/projects/{project}/observability/otlp/sources".to_string(),
                Some(body_object(body)?),
                false,
                true,
                false,
            ),
            IntelCommand::Observability {
                action:
                    ObservabilityAction::Sources {
                        action: ObservabilitySourcesAction::Revoke { source_id },
                    },
            } => (
                "POST",
                "/api/v1/projects/{project}/observability/otlp/sources/{id}/revoke"
                    .to_string()
                    .replace("{id}", &segment(source_id)?),
                Some(serde_json::json!({})),
                true,
                false,
                false,
            ),
            IntelCommand::PipelineRuns {
                action: PipelineRunsAction::List,
            } => (
                "GET",
                "/api/v1/projects/{project}/pipeline-runs".to_string(),
                None,
                false,
                false,
                false,
            ),
            IntelCommand::PipelineRuns {
                action: PipelineRunsAction::Steps { run_id },
            } => (
                "GET",
                "/api/v1/projects/{project}/pipeline-runs/{id}/steps"
                    .to_string()
                    .replace("{id}", &segment(run_id)?),
                None,
                false,
                false,
                false,
            ),
            IntelCommand::PipelineRuns {
                action: PipelineRunsAction::Attempts { run_id },
            } => (
                "GET",
                "/api/v1/projects/{project}/pipeline-runs/{id}/attempts"
                    .to_string()
                    .replace("{id}", &segment(run_id)?),
                None,
                false,
                false,
                false,
            ),
            IntelCommand::PullRequests {
                action: PullRequestsAction::List,
            } => (
                "GET",
                "/api/v1/projects/{project}/pull-requests".to_string(),
                None,
                false,
                false,
                false,
            ),
            IntelCommand::PullRequests {
                action: PullRequestsAction::Get { number },
            } => (
                "GET",
                "/api/v1/projects/{project}/pull-requests/{id}"
                    .to_string()
                    .replace("{id}", &segment(number)?),
                None,
                false,
                false,
                false,
            ),
            IntelCommand::PullRequests {
                action: PullRequestsAction::Create { body },
            } => (
                "POST",
                "/api/v1/projects/{project}/pull-requests".to_string(),
                Some(body_object(body)?),
                false,
                false,
                false,
            ),
            IntelCommand::PullRequests {
                action: PullRequestsAction::Update { number, body },
            } => (
                "PATCH",
                "/api/v1/projects/{project}/pull-requests/{id}"
                    .to_string()
                    .replace("{id}", &segment(number)?),
                Some(body_object(body)?),
                true,
                false,
                false,
            ),
            IntelCommand::PullRequests {
                action: PullRequestsAction::Changes { number },
            } => (
                "GET",
                "/api/v1/projects/{project}/pull-requests/{id}/changes"
                    .to_string()
                    .replace("{id}", &segment(number)?),
                None,
                false,
                false,
                false,
            ),
            IntelCommand::PullRequests {
                action: PullRequestsAction::Comments { number },
            } => (
                "GET",
                "/api/v1/projects/{project}/pull-requests/{id}/comments"
                    .to_string()
                    .replace("{id}", &segment(number)?),
                None,
                false,
                false,
                false,
            ),
            IntelCommand::PullRequests {
                action: PullRequestsAction::Comment { number, body },
            } => (
                "POST",
                "/api/v1/projects/{project}/pull-requests/{id}/comments"
                    .to_string()
                    .replace("{id}", &segment(number)?),
                Some(body_object(body)?),
                false,
                false,
                false,
            ),
            IntelCommand::ReleaseChannel {
                action: ReleaseChannelAction::Latest,
            } => (
                "GET",
                "/api/v1/projects/{project}/release-channels/latest".to_string(),
                None,
                false,
                false,
                false,
            ),
            IntelCommand::Security {
                action: SecurityAction::Audit,
            } => (
                "GET",
                "/api/v1/projects/{project}/security-audit".to_string(),
                None,
                false,
                false,
                false,
            ),
            IntelCommand::Security {
                action: SecurityAction::Posture,
            } => (
                "GET",
                "/api/v1/projects/{project}/security-posture".to_string(),
                None,
                false,
                false,
                false,
            ),
            IntelCommand::SpeedInsights {
                action: SpeedInsightsAction::Get,
            } => (
                "GET",
                "/api/v1/projects/{project}/speed-insights".to_string(),
                None,
                false,
                false,
                false,
            ),
            IntelCommand::TestRuns {
                action: TestRunsAction::List,
            } => (
                "GET",
                "/api/v1/projects/{project}/test-runs".to_string(),
                None,
                false,
                false,
                false,
            ),
            IntelCommand::TestRuns {
                action: TestRunsAction::Results { test_run_id },
            } => (
                "GET",
                "/api/v1/projects/{project}/test-runs/{id}/results"
                    .to_string()
                    .replace("{id}", &segment(test_run_id)?),
                None,
                false,
                false,
                false,
            ),
            IntelCommand::TestRuns {
                action: TestRunsAction::Queue { version },
            } => (
                "POST",
                "/api/v1/projects/{project}/test-runs".to_string(),
                Some(match version {
                    Some(v) => serde_json::json!({"version": v}),
                    None => serde_json::json!({}),
                }),
                false,
                false,
                false,
            ),
            IntelCommand::Traces {
                action: TracesAction::List,
            } => (
                "GET",
                "/api/v1/projects/{project}/traces".to_string(),
                None,
                false,
                false,
                false,
            ),
            IntelCommand::Traces {
                action: TracesAction::Get { trace_id },
            } => (
                "GET",
                "/api/v1/projects/{project}/traces/{id}"
                    .to_string()
                    .replace("{id}", &segment(trace_id)?),
                None,
                false,
                false,
                false,
            ),
            IntelCommand::Traffic {
                action: TrafficAction::List,
            } => (
                "GET",
                "/api/v1/projects/{project}/traffic".to_string(),
                None,
                false,
                false,
                false,
            ),
            IntelCommand::Settings {
                action: SettingsAction::Update { fields },
            } => (
                "PATCH",
                "/api/v1/projects/{project}/settings".to_string(),
                Some(typed_patch(fields)?),
                true,
                false,
                false,
            ),
            IntelCommand::CliInstalled {
                action: CliInstalledAction::Confirm,
            } => (
                "POST",
                "/api/v1/dashboard/setup/cli-installed".to_string(),
                Some(serde_json::json!({"installed": true})),
                false,
                false,
                false,
            ),
        };
        Ok(IntelRequest {
            method,
            path,
            body,
            destructive,
            sensitive,
            follow,
        })
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use clap::Parser;
    #[derive(Parser)]
    pub struct ParserHarness {
        #[command(subcommand)]
        pub command: IntelCommand,
    }
    pub type Case = (Vec<&'static str>, &'static str, &'static str, Option<Value>);

    pub fn cases() -> Vec<Case> {
        vec![
            (
                vec!["intel", "activity", "list"],
                "GET",
                "/api/v1/activity",
                None,
            ),
            (
                vec!["intel", "activity", "stream"],
                "GET",
                "/api/v1/activity/stream",
                None,
            ),
            (vec!["intel", "me", "get"], "GET", "/api/v1/me", None),
            (
                vec!["intel", "project", "get"],
                "GET",
                "/api/v1/projects/{project}",
                None,
            ),
            (
                vec!["intel", "project", "overview"],
                "GET",
                "/api/v1/projects/{project}/overview",
                None,
            ),
            (
                vec!["intel", "alerts", "list"],
                "GET",
                "/api/v1/projects/{project}/alerts",
                None,
            ),
            (
                vec!["intel", "alert-policies", "list"],
                "GET",
                "/api/v1/projects/{project}/alert-policies",
                None,
            ),
            (
                vec![
                    "intel",
                    "alert-policies",
                    "update",
                    "item",
                    "--threshold",
                    "5",
                ],
                "PUT",
                "/api/v1/projects/{project}/alert-policies/item",
                Some(serde_json::json!({"threshold":5.0})),
            ),
            (
                vec!["intel", "logs", "list"],
                "GET",
                "/api/v1/projects/{project}/logs",
                None,
            ),
            (
                vec!["intel", "observability", "get"],
                "GET",
                "/api/v1/projects/{project}/observability",
                None,
            ),
            (
                vec!["intel", "observability", "settings", "get"],
                "GET",
                "/api/v1/projects/{project}/observability/settings",
                None,
            ),
            (
                vec![
                    "intel",
                    "observability",
                    "settings",
                    "update",
                    "--body",
                    "{\"name\":\"example\"}",
                ],
                "PUT",
                "/api/v1/projects/{project}/observability/settings",
                Some(serde_json::json!({"name":"example"})),
            ),
            (
                vec!["intel", "observability", "sources", "list"],
                "GET",
                "/api/v1/projects/{project}/observability/otlp/sources",
                None,
            ),
            (
                vec![
                    "intel",
                    "observability",
                    "sources",
                    "create",
                    "--body",
                    "{\"name\":\"example\"}",
                ],
                "POST",
                "/api/v1/projects/{project}/observability/otlp/sources",
                Some(serde_json::json!({"name":"example"})),
            ),
            (
                vec!["intel", "observability", "sources", "revoke", "item"],
                "POST",
                "/api/v1/projects/{project}/observability/otlp/sources/item/revoke",
                Some(serde_json::json!({})),
            ),
            (
                vec!["intel", "pipeline-runs", "list"],
                "GET",
                "/api/v1/projects/{project}/pipeline-runs",
                None,
            ),
            (
                vec!["intel", "pipeline-runs", "steps", "item"],
                "GET",
                "/api/v1/projects/{project}/pipeline-runs/item/steps",
                None,
            ),
            (
                vec!["intel", "pipeline-runs", "attempts", "item"],
                "GET",
                "/api/v1/projects/{project}/pipeline-runs/item/attempts",
                None,
            ),
            (
                vec!["intel", "pull-requests", "list"],
                "GET",
                "/api/v1/projects/{project}/pull-requests",
                None,
            ),
            (
                vec!["intel", "pull-requests", "get", "item"],
                "GET",
                "/api/v1/projects/{project}/pull-requests/item",
                None,
            ),
            (
                vec![
                    "intel",
                    "pull-requests",
                    "create",
                    "--body",
                    "{\"name\":\"example\"}",
                ],
                "POST",
                "/api/v1/projects/{project}/pull-requests",
                Some(serde_json::json!({"name":"example"})),
            ),
            (
                vec![
                    "intel",
                    "pull-requests",
                    "update",
                    "item",
                    "--body",
                    "{\"name\":\"example\"}",
                ],
                "PATCH",
                "/api/v1/projects/{project}/pull-requests/item",
                Some(serde_json::json!({"name":"example"})),
            ),
            (
                vec!["intel", "pull-requests", "changes", "item"],
                "GET",
                "/api/v1/projects/{project}/pull-requests/item/changes",
                None,
            ),
            (
                vec!["intel", "pull-requests", "comments", "item"],
                "GET",
                "/api/v1/projects/{project}/pull-requests/item/comments",
                None,
            ),
            (
                vec![
                    "intel",
                    "pull-requests",
                    "comment",
                    "item",
                    "--body",
                    "{\"name\":\"example\"}",
                ],
                "POST",
                "/api/v1/projects/{project}/pull-requests/item/comments",
                Some(serde_json::json!({"name":"example"})),
            ),
            (
                vec!["intel", "release-channel", "latest"],
                "GET",
                "/api/v1/projects/{project}/release-channels/latest",
                None,
            ),
            (
                vec!["intel", "security", "audit"],
                "GET",
                "/api/v1/projects/{project}/security-audit",
                None,
            ),
            (
                vec!["intel", "security", "posture"],
                "GET",
                "/api/v1/projects/{project}/security-posture",
                None,
            ),
            (
                vec!["intel", "speed-insights", "get"],
                "GET",
                "/api/v1/projects/{project}/speed-insights",
                None,
            ),
            (
                vec!["intel", "test-runs", "list"],
                "GET",
                "/api/v1/projects/{project}/test-runs",
                None,
            ),
            (
                vec!["intel", "test-runs", "results", "item"],
                "GET",
                "/api/v1/projects/{project}/test-runs/item/results",
                None,
            ),
            (
                vec!["intel", "test-runs", "queue", "--version", "1.2.3"],
                "POST",
                "/api/v1/projects/{project}/test-runs",
                Some(serde_json::json!({"version":"1.2.3"})),
            ),
            (
                vec!["intel", "traces", "list"],
                "GET",
                "/api/v1/projects/{project}/traces",
                None,
            ),
            (
                vec!["intel", "traces", "get", "item"],
                "GET",
                "/api/v1/projects/{project}/traces/item",
                None,
            ),
            (
                vec!["intel", "traffic", "list"],
                "GET",
                "/api/v1/projects/{project}/traffic",
                None,
            ),
            (
                vec!["intel", "settings", "update", "--name", "example"],
                "PATCH",
                "/api/v1/projects/{project}/settings",
                Some(serde_json::json!({"name":"example"})),
            ),
            (
                vec!["intel", "cli-installed", "confirm"],
                "POST",
                "/api/v1/dashboard/setup/cli-installed",
                Some(serde_json::json!({"installed": true})),
            ),
        ]
    }
    #[test]
    fn every_command_maps_to_the_dashboard_route_and_body() {
        assert_eq!(cases().len(), 37);
        for (args, method, path, body) in cases() {
            let parsed = ParserHarness::try_parse_from(args).unwrap();
            let request = parsed.command.request().unwrap();
            assert_eq!(request.method, method);
            assert_eq!(request.path, path);
            assert_eq!(request.body, body);
        }
    }
    #[test]
    fn invalid_bodies_and_path_injection() {
        assert!(
            ParserHarness::try_parse_from(["intel", "settings", "update", "--body", "[]"]).is_err()
        );
        let parsed = ParserHarness::try_parse_from(["intel", "traces", "get", "a/b ?#"]).unwrap();
        assert!(parsed
            .command
            .request()
            .unwrap()
            .path
            .ends_with("a%2Fb%20%3F%23"));
        for value in ["", ".", ".."] {
            let parsed = ParserHarness::try_parse_from(["intel", "traces", "get", value]).unwrap();
            assert!(parsed.command.request().is_err());
        }
    }
    #[test]
    fn follow_and_sensitive_mutations_are_explicit() {
        for args in [
            vec!["intel", "activity", "stream", "--follow"],
            vec!["intel", "logs", "list", "--follow"],
        ] {
            assert!(
                ParserHarness::try_parse_from(args)
                    .unwrap()
                    .command
                    .request()
                    .unwrap()
                    .follow
            );
        }
        let parsed = ParserHarness::try_parse_from([
            "intel",
            "observability",
            "sources",
            "create",
            "--body",
            "{}",
        ])
        .unwrap();
        assert!(parsed.command.request().unwrap().sensitive);
        let parsed =
            ParserHarness::try_parse_from(["intel", "observability", "sources", "revoke", "item"])
                .unwrap();
        assert!(parsed.command.request().unwrap().destructive);
    }
}

#[cfg(test)]
#[path = "intel_update_tests.rs"]
mod update_tests;
