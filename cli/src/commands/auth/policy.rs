//! `axiom auth policy` (branches-v1 "CI policy"): pull, diff, check, release, plan and apply
//! of `axiom-auth-policy/v1` documents. `check` writes the `axiom-auth-impact/v1` report and
//! exits 0 clean, 6 unacknowledged widening, 7 unacknowledged narrowing, 8 stale approvals,
//! 9 partial coverage with `--require-complete`, 2 usage. Releases are signed with an enrolled
//! migration signer key and carry the impact sidecar.
use super::{admin, print, project_target, target, text, usage, AuthArgs, Exit, Target};
use anyhow::{Context, Result};
use axiom_database::developer as db;
use axiom_lib::auth_impact::{self as ai, Approvals, Policy, Report, Subject};
use clap::Subcommand;
use reqwest::Method;
use serde_json::{json, Value};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

#[derive(Subcommand, Debug)]
pub enum PolicyAction {
    /// Write the environment's current policy (canonical document)
    Pull {
        /// Write it here (default: print)
        #[arg(long)]
        write: Option<PathBuf>,
    },
    /// List the change items from a base to the policy file
    Diff {
        /// `<file>` or `env:<environment>`
        #[arg(long)]
        base: String,
        #[arg(long, default_value = "auth/policy.json")]
        policy: PathBuf,
    },
    /// Write the impact report and check it against the approvals file
    Check {
        /// `<file>` or `env:<environment>`
        #[arg(long)]
        base: String,
        #[arg(long, default_value = "auth/policy.json")]
        policy: PathBuf,
        /// Backend server plan after the change (<output>.server.json)
        #[arg(long)]
        plan: Option<PathBuf>,
        /// Backend server plan before the change (default: the plan did not change)
        #[arg(long)]
        base_plan: Option<PathBuf>,
        #[arg(long)]
        approvals: Option<PathBuf>,
        /// Exit 9 when coverage is partial
        #[arg(long)]
        require_complete: bool,
        /// Write the report to this file
        #[arg(long)]
        report: Option<PathBuf>,
    },
    /// Sign the policy as a release with an enrolled signer key, upload it and attach its impact sidecar
    Release {
        #[arg(long, default_value = "auth/policy.json")]
        policy: PathBuf,
        /// Database contract whose local signing key signs (keys are per contract; or AXIOM_DATABASE_SIGNING_KEY)
        #[arg(long, value_name = "CONTRACT")]
        schema: Option<PathBuf>,
        /// The parent release SHA-256 (default: the newest uploaded release, or none)
        #[arg(long)]
        parent: Option<String>,
        /// `<file>` or `env:<environment>` for the impact report (default: the parent release's policy)
        #[arg(long)]
        base: Option<String>,
        #[arg(long)]
        plan: Option<PathBuf>,
        #[arg(long)]
        base_plan: Option<PathBuf>,
    },
    /// List the signed policy releases uploaded to the project, newest first
    Releases,
    /// Ask the server for the change items of a release against the environment
    Plan {
        #[arg(long)]
        release: String,
    },
    /// Apply a release to the environment; widening and narrowing items need acknowledgement
    Apply {
        #[arg(long)]
        release: String,
        /// Acknowledge exactly the items approved in this approvals file
        #[arg(long)]
        acknowledge_from: Option<PathBuf>,
    },
}

const MAX_FILE: u64 = 1024 * 1024;

fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    let meta = std::fs::metadata(path).with_context(|| format!("read {}", path.display()))?;
    if meta.len() > MAX_FILE {
        return Err(usage(format!("{} is above 1 MiB", path.display())));
    }
    Ok(std::fs::read(path)?)
}

pub fn read_policy(path: &Path) -> Result<Policy> {
    Policy::parse(&read_bounded(path)?).map_err(|e| usage(format!("{}: {e:#}", path.display())))
}

fn read_plan(path: Option<&Path>) -> Result<Option<Value>> {
    path.map(|p| serde_json::from_slice::<Value>(&read_bounded(p)?).map_err(|_| usage(format!("{} is not a server plan (JSON)", p.display())))).transpose()
}

/// The environment's current policy document (`GET policy`).
pub async fn environment_policy(target: &Target) -> Result<(Policy, Value)> {
    let view = target.call(Method::GET, "policy", &[], None).await?;
    let policy: Policy = serde_json::from_value(view["policy"].clone()).map_err(|_| Exit { code: 5, message: "AXAUTH_UNAVAILABLE: the policy view is not a policy document".into() })?;
    policy.validate().map_err(|_| Exit { code: 5, message: "AXAUTH_UNAVAILABLE: the environment's policy does not validate".into() })?;
    Ok((policy, view))
}

/// `--base <file|env:<e>>`.
async fn base_policy(args: &AuthArgs, base: &str) -> Result<Policy> {
    match base.strip_prefix("env:") {
        Some(environment) => {
            let scoped = AuthArgs { project: args.project.clone(), org: args.org.clone(), environment: environment.to_string(), json: args.json, action: super::Action::Status };
            let target = target(&scoped).await?;
            Ok(environment_policy(&target).await?.0)
        }
        None => read_policy(Path::new(base)),
    }
}

/// Pretty JSON of the canonical document (same digest; easier to review).
pub fn policy_file_bytes(policy: &Policy) -> Result<Vec<u8>> {
    let canonical: Value = serde_json::from_slice(&policy.canonical())?;
    let mut bytes = serde_json::to_vec_pretty(&canonical)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn item_lines(items: &[ai::Item]) -> Vec<String> {
    if items.is_empty() {
        return vec!["No changes.".into()];
    }
    items.iter().map(|i| format!("{:<9} {}", i.direction, i.id)).collect()
}

/// Report plus approvals outcome; the process exit code is `check.exit`.
pub fn check_report(report: &Report, approvals: Option<&Path>, require_complete: bool) -> Result<ai::Check> {
    let approvals = approvals.map(|p| Approvals::parse(&read_bounded(p)?).map_err(|e| usage(format!("{}: {e:#}", p.display())))).transpose()?;
    Ok(ai::check(report, approvals.as_ref(), require_complete))
}

fn render_check(report: &Report, check: &ai::Check) -> Vec<String> {
    let mut lines = vec![format!(
        "Policy {} -> {}: {} item(s), {} access change(s), coverage {}",
        report.policy_before_sha256.as_deref().map_or("unknown", |s| &s[..12]),
        report.policy_after_sha256.as_deref().map_or("unknown", |s| &s[..12]),
        report.policy_changes.len(),
        report.access.len(),
        report.coverage
    )];
    for a in &report.access {
        lines.push(format!("  {:<9} {} {} {} -> {}", a.direction, a.endpoint, a.subject, a.before, a.after));
    }
    for u in &report.unknown {
        lines.push(format!("  unknown {} {} ({})", u.kind, u.object.as_deref().unwrap_or("-"), u.detail_code));
    }
    if !report.unmapped_permissions.is_empty() {
        lines.push(format!("  permissions no role grants: {}", report.unmapped_permissions.join(", ")));
    }
    if check.stale {
        lines.push("Approvals are stale (another policy or plan digest): approve this change set again.".into());
    }
    for id in &check.unapproved_widening {
        lines.push(format!("Unapproved widening: {id}"));
    }
    for id in &check.unapproved_narrowing {
        lines.push(format!("Unapproved narrowing: {id}"));
    }
    lines
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("axiom-tmp");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

fn exit_with(code: i32, message: &str) -> anyhow::Error {
    Exit { code, message: message.into() }.into()
}

pub async fn run(args: &AuthArgs, action: &PolicyAction) -> Result<()> {
    match action {
        PolicyAction::Pull { write } => {
            let target = target(args).await?;
            let (policy, view) = environment_policy(&target).await?;
            let bytes = policy_file_bytes(&policy)?;
            match write {
                Some(path) => {
                    write_file(path, &bytes)?;
                    print(args, &json!({"sha256": policy.sha256(), "revision": view["revision"], "policy_mode": view["policy_mode"], "written": path}), |r| {
                        vec![format!("Policy {} (revision {}, mode {}) written to {}.", text(r, "sha256"), text(r, "revision"), text(r, "policy_mode"), path.display())]
                    })
                }
                None => {
                    print!("{}", String::from_utf8_lossy(&bytes));
                    Ok(())
                }
            }
        }
        PolicyAction::Diff { base, policy } => {
            let after = read_policy(policy)?;
            let before = base_policy(args, base).await?;
            let items = ai::diff(&before, &after);
            print(args, &json!({"before_sha256": before.sha256(), "after_sha256": after.sha256(), "items": items}), |_| item_lines(&items))
        }
        PolicyAction::Check { base, policy, plan, base_plan, approvals, require_complete, report: out } => {
            let after = read_policy(policy)?;
            let plan_after = read_plan(plan.as_deref())?;
            let plan_before = read_plan(base_plan.as_deref())?;
            let before = base_policy(args, base).await.ok();
            let report = ai::report(ai::Inputs {
                subject: Subject { kind: "local".into(), sha256: after.sha256() },
                policy_before: before.as_ref(),
                policy_after: &after,
                plan_before: plan_before.as_ref(),
                plan_after: plan_after.as_ref(),
                unknown: vec![],
            });
            let check = check_report(&report, approvals.as_deref(), *require_complete)?;
            if let Some(path) = out {
                write_file(path, &serde_json::to_vec_pretty(&report)?)?;
            }
            print(args, &json!({"report": report, "check": check}), |_| render_check(&report, &check))?;
            if check.exit != 0 {
                let message = match check.exit {
                    6 => "unacknowledged widening",
                    7 => "unacknowledged narrowing",
                    8 => "stale approvals",
                    _ => "partial coverage",
                };
                return Err(exit_with(check.exit, &format!("AXAUTH_POLICY_CHECK: {message}")));
            }
            Ok(())
        }
        PolicyAction::Release { policy, schema, parent, base, plan, base_plan } => release(args, policy, schema.as_deref(), parent.as_deref(), base.as_deref(), plan.as_deref(), base_plan.as_deref()).await,
        PolicyAction::Releases => {
            // Releases are project-wide: --environment is not resolved.
            let target = project_target(args).await?;
            releases(args, &target).await
        }
        PolicyAction::Plan { release } => {
            hex64(release)?;
            let target = target(args).await?;
            let plan = target.call(Method::POST, "policy/plan", &[], Some(&json!({"release": release}))).await?;
            print(args, &plan, |p| {
                let items = serde_json::from_value::<Vec<ai::Item>>(p["items"].clone()).unwrap_or_default();
                let mut lines = vec![format!("Base {} -> {}", text(p, "base_sha256"), text(p, "policy_after_sha256"))];
                lines.extend(item_lines(&items));
                for (role, count) in p["holders"].as_object().into_iter().flatten() {
                    lines.push(format!("  role {role}: {count} holder(s)"));
                }
                lines
            })
        }
        PolicyAction::Apply { release, acknowledge_from } => {
            hex64(release)?;
            let approvals = acknowledge_from.as_deref().map(|p| Approvals::parse(&read_bounded(p)?).map_err(|e| usage(format!("{}: {e:#}", p.display())))).transpose()?;
            let target = target(args).await?;
            let plan = target.call(Method::POST, "policy/plan", &[], Some(&json!({"release": release}))).await?;
            let acknowledged = acknowledgements(&plan, approvals.as_ref())?;
            let body = json!({"release": release, "base_sha256": plan["base_sha256"], "acknowledged": acknowledged});
            let applied = target.call(Method::POST, "policy/apply", &[], Some(&body)).await?;
            print(args, &applied, |a| vec![format!("Applied {} item(s); policy {} revision {}.", text(a, "items"), text(a, "sha256"), text(a, "revision"))])
        }
    }
}

/// `GET /api/v1/projects/{project}/auth/policy/releases` (project-wide, newest first).
async fn releases(args: &AuthArgs, target: &Target) -> Result<()> {
    let listed = target.client.project_auth_request(Method::GET, &target.slug, "auth/policy/releases", None, None).await.map_err(admin)?;
    print(args, &listed, |l| {
        let short = |v: &Value| v.as_str().map(|s| s.chars().take(12).collect::<String>()).unwrap_or_else(|| "-".into());
        let mut lines = l["releases"].as_array().into_iter().flatten().map(|r| format!("{}  policy {}  parent {}  uploaded {}", text(r, "sha256"), short(&r["policy_sha256"]), short(&r["parent_sha256"]), text(r, "uploaded_at"))).collect::<Vec<_>>();
        if lines.is_empty() {
            lines.push("No policy releases.".into());
        }
        lines
    })
}

fn hex64(value: &str) -> Result<()> {
    if value.len() == 64 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        Ok(())
    } else {
        Err(usage("--release is a lowercase hex SHA-256"))
    }
}

/// The acknowledgement lists for `policy/apply`: exactly the plan's widening and narrowing
/// ids, each approved in the file (for the plan's `policy_after_sha256`) or confirmed on a
/// terminal; otherwise exit 6, 7 or 8 as `check`.
pub fn acknowledgements(plan: &Value, approvals: Option<&Approvals>) -> Result<Value> {
    let ids = |key: &str| plan[key].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>();
    let (widening, narrowing) = (ids("widening"), ids("narrowing"));
    if widening.is_empty() && narrowing.is_empty() {
        return Ok(json!({"widening": [], "narrowing": []}));
    }
    match approvals {
        Some(a) => {
            if plan["policy_after_sha256"].as_str() != Some(a.policy_after_sha256.as_str()) {
                return Err(exit_with(8, "AXAUTH_POLICY_CHECK: stale approvals (another policy digest)"));
            }
            let approved = a.approved.iter().map(|x| x.id.as_str()).collect::<std::collections::BTreeSet<_>>();
            if let Some(id) = widening.iter().find(|id| !approved.contains(id.as_str())) {
                return Err(exit_with(6, &format!("AXAUTH_POLICY_CHECK: unacknowledged widening {id}")));
            }
            if let Some(id) = narrowing.iter().find(|id| !approved.contains(id.as_str())) {
                return Err(exit_with(7, &format!("AXAUTH_POLICY_CHECK: unacknowledged narrowing {id}")));
            }
        }
        None => {
            if !std::io::stdin().is_terminal() {
                return Err(exit_with(if widening.is_empty() { 7 } else { 6 }, "AXAUTH_POLICY_CHECK: acknowledge the items with --acknowledge-from <approvals file>"));
            }
            for id in widening.iter().chain(&narrowing) {
                eprintln!("  {id}");
            }
            if !dialoguer::Confirm::new().with_prompt("Acknowledge these widening and narrowing items?").default(false).interact()? {
                return Err(usage("cancelled"));
            }
        }
    }
    Ok(json!({"widening": widening, "narrowing": narrowing}))
}

#[allow(clippy::too_many_arguments)]
async fn release(args: &AuthArgs, policy: &Path, schema: Option<&Path>, parent: Option<&str>, base: Option<&str>, plan: Option<&Path>, base_plan: Option<&Path>) -> Result<()> {
    let after = read_policy(policy)?;
    let contract = match schema {
        Some(schema) => super::super::database_dev::load_existing(schema)?.0.compiled.identity.name,
        None if std::env::var_os("AXIOM_DATABASE_SIGNING_KEY").is_some() => String::new(),
        None => return Err(usage("the signing key is per database contract; pass --schema <CONTRACT> (or set AXIOM_DATABASE_SIGNING_KEY)")),
    };
    if let Some(parent) = parent {
        hex64(parent)?;
    }
    let plan_after = read_plan(plan)?;
    let plan_before = read_plan(base_plan)?;
    let target = target(args).await?;
    let project_id = target.client.project_id(&target.slug).await.map_err(|_| Exit { code: 5, message: "AXAUTH_UNAVAILABLE: could not resolve the project".into() })?;
    let releases = target.client.project_auth_request(Method::GET, &target.slug, "auth/policy/releases", None, None).await.map_err(admin)?;
    let newest = releases["releases"].as_array().into_iter().flatten().max_by(|a, b| a["uploaded_at"].as_str().cmp(&b["uploaded_at"].as_str())).and_then(|r| r["sha256"].as_str()).map(str::to_string);
    let parent = parent.map(str::to_string).or(newest);
    let before = match (base, &parent) {
        (Some(base), _) => base_policy(args, base).await.ok(),
        (None, Some(parent)) => {
            let raw = target.client.project_auth_request(Method::GET, &target.slug, &format!("auth/policy/releases/{parent}"), None, None).await.map_err(admin)?;
            serde_json::from_value::<Policy>(raw["policy"].clone()).ok()
        }
        (None, None) => Some(Policy::empty()),
    };
    let sign = |message: &[u8]| db::sign_message_with_local_key(&contract, message).map_err(|e| anyhow::anyhow!(e.to_string()));
    let bytes = ai::seal_release(&project_id, parent.as_deref(), &after, sign)?;
    let verified = ai::verify_release(&bytes, &project_id)?;
    let key = uuid::Uuid::new_v4().to_string();
    target.client.project_auth_upload(Method::POST, &target.slug, "auth/policy/releases", &bytes, Some(&key)).await.map_err(admin)?;
    let report = ai::report(ai::Inputs {
        subject: Subject { kind: "auth_policy_release".into(), sha256: verified.sha256.clone() },
        policy_before: before.as_ref(),
        policy_after: &after,
        plan_before: plan_before.as_ref(),
        plan_after: plan_after.as_ref(),
        unknown: vec![],
    });
    let envelope = ai::Envelope::seal(report, sign)?;
    if envelope.public_key != verified.public_key {
        return Err(usage("the impact sidecar must be signed by the release's key"));
    }
    let sidecar = envelope.canonical_bytes()?;
    let key = uuid::Uuid::new_v4().to_string();
    target.client.project_auth_upload(Method::PUT, &target.slug, &format!("auth/policy/releases/{}/impact", verified.sha256), &sidecar, Some(&key)).await.map_err(admin)?;
    print(args, &json!({"release": verified.sha256, "policy_sha256": verified.policy_sha256, "parent_sha256": verified.parent_sha256, "public_key": verified.public_key, "impact_coverage": envelope.report.coverage}), |r| {
        vec![
            format!("Release {} uploaded (policy {}).", text(r, "release"), text(r, "policy_sha256")),
            format!("Impact sidecar attached (coverage {}). Plan it with `axiom auth policy plan --release {}`.", text(r, "impact_coverage"), text(r, "release")),
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_files_keep_the_canonical_digest() {
        let policy = Policy::parse(br#"{"format":"axiom-auth-policy/v1","permissions":[{"name":"b"},{"name":"a","description":"A"}],"roles":[{"name":"r","permissions":["b","a"]}]}"#).unwrap();
        let bytes = policy_file_bytes(&policy).unwrap();
        assert!(bytes.ends_with(b"\n") && bytes.contains(&b'\n'));
        assert_eq!(Policy::parse(&bytes).unwrap().sha256(), policy.sha256());
    }

    #[test]
    fn apply_acknowledges_exactly_the_approved_plan_items() {
        let plan = json!({"policy_after_sha256": "a".repeat(64), "widening": ["grant:+:r:p"], "narrowing": ["role:-:old"]});
        let approvals = |ids: &[&str], after: String| Approvals { format: ai::APPROVALS_FORMAT.into(), policy_after_sha256: after, plan_after_sha256: None, approved: ids.iter().map(|id| ai::Approval { id: id.to_string(), reason: "reviewed".into() }).collect() };
        let ok = acknowledgements(&plan, Some(&approvals(&["grant:+:r:p", "role:-:old", "access:x:role:r:widening"], "a".repeat(64)))).unwrap();
        assert_eq!(ok, json!({"widening": ["grant:+:r:p"], "narrowing": ["role:-:old"]}));
        let code = |r: Result<Value>| r.unwrap_err().downcast_ref::<Exit>().unwrap().code;
        assert_eq!(code(acknowledgements(&plan, Some(&approvals(&["role:-:old"], "a".repeat(64))))), 6);
        assert_eq!(code(acknowledgements(&plan, Some(&approvals(&["grant:+:r:p"], "a".repeat(64))))), 7);
        assert_eq!(code(acknowledgements(&plan, Some(&approvals(&["grant:+:r:p", "role:-:old"], "b".repeat(64))))), 8);
        assert_eq!(acknowledgements(&json!({"widening": [], "narrowing": []}), None).unwrap(), json!({"widening": [], "narrowing": []}));
    }

    #[test]
    fn check_reads_approvals_files() {
        let dir = tempfile::tempdir().unwrap();
        let before = Policy::empty();
        let after = Policy::parse(br#"{"format":"axiom-auth-policy/v1","permissions":[{"name":"p"}],"roles":[{"name":"r","permissions":["p"]}]}"#).unwrap();
        let report = ai::report(ai::Inputs { subject: Subject { kind: "local".into(), sha256: after.sha256() }, policy_before: Some(&before), policy_after: &after, plan_before: None, plan_after: None, unknown: vec![] });
        assert_eq!(check_report(&report, None, false).unwrap().exit, 6);
        let path = dir.path().join("approvals.json");
        std::fs::write(&path, serde_json::to_vec(&Approvals::template(&report, "new editor role")).unwrap()).unwrap();
        assert_eq!(check_report(&report, Some(&path), false).unwrap().exit, 0);
        assert_eq!(check_report(&report, Some(&path), true).unwrap().exit, 9);
        std::fs::write(&path, b"{}").unwrap();
        assert_eq!(check_report(&report, Some(&path), false).unwrap_err().downcast_ref::<Exit>().unwrap().code, 2);
    }

    #[tokio::test]
    async fn releases_lists_the_project_releases() {
        let mock = crate::commands::cloud_mock::Mock::default();
        let base = mock.start().await;
        let environment = "0b5e2c1a-9f3d-4c7e-8a21-5d6f7e8a9b0c";
        let target = Target { client: axiom_cloud::CloudClient::for_loopback(&base, "access".into(), "refresh".into()).unwrap(), slug: "demo".into(), environment: environment.into() };
        let args = AuthArgs { project: Some("demo".into()), org: None, environment: environment.into(), json: false, action: super::super::Action::Status };
        let route = "GET /api/v1/projects/demo/auth/policy/releases";
        mock.script(route, 200, json!({"releases": [{"sha256": "a".repeat(64), "policy_sha256": "b".repeat(64), "parent_sha256": null, "public_key": "k", "uploaded_at": "2026-11-01T00:00:00Z"}]}));
        releases(&args, &target).await.unwrap();
        let seen = mock.seen(route);
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].query, None);
        assert_eq!(seen[0].body, Value::Null);
        assert_eq!(seen[0].bearer.as_deref(), Some("access"));
        // A refusal keeps its exit code (3 forbidden).
        let mock = crate::commands::cloud_mock::Mock::default();
        let base = mock.start().await;
        let target = Target { client: axiom_cloud::CloudClient::for_loopback(&base, "access".into(), "refresh".into()).unwrap(), slug: "demo".into(), environment: environment.into() };
        mock.script(route, 403, json!({"code": "AXRESOURCE_DENIED"}));
        let error = releases(&args, &target).await.unwrap_err();
        assert_eq!(error.downcast_ref::<Exit>().unwrap().code, 3);
    }
}
