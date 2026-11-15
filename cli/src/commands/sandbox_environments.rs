//! `axiom project sandboxes` (P4): the managed sandboxes and hosted mocks of a project, the
//! same lifecycle the dashboard's Sandbox page drives, under
//! `/api/v1/projects/{project_slug}/sandboxes`. These are not configuration environments
//! (`axiom secrets`) and not authentication environments (`axiom auth`). The command was called
//! `axiom project environments` up to CLI 0.153.0; that name now fails locally (exit 2).
//!
//! Every request goes through [`call`] with the CLI session from `axiom login`. The
//! connection reply carries the sandbox access key; it is printed only with `--show-secret`
//! (the dashboard reveals it behind "Reveal connection"), never by default, also not in `--json`.
use super::cloud_cli;
use super::resources_recovery::{Api, Reply};
use anyhow::{bail, Result};
use clap::{Args, Subcommand};
use reqwest::Method;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

#[derive(Args, Debug)]
pub struct SandboxesArgs {
    /// Cloud project: slug, name or id (default: the project linked to this directory)
    #[arg(long, global = true)]
    pub project: Option<String>,
    /// Machine-readable output (the server reply as pretty JSON)
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Subcommand, Debug)]
pub enum Action {
    /// Managed sandboxes and hosted mocks of the project, newest first (ended ones included)
    List,
    /// Request a managed sandbox from a verified deployable release (project managers)
    Create {
        /// Sandbox name: lowercase letters, digits and hyphens, starting with a letter
        #[arg(long, default_value = "sandbox")]
        name: String,
        /// `sandbox` (stateful mock runtime) or `hosted_mock`
        #[arg(long, value_parser = ["sandbox", "hosted_mock"], default_value = "sandbox")]
        kind: String,
        /// Contract version to run (default: the release on the `latest` channel)
        #[arg(long)]
        version: Option<String>,
        /// `token` (requests need the sandbox access key) or `public`
        #[arg(long, value_parser = ["token", "public"], default_value = "token")]
        access: String,
        /// Lifetime in minutes, 5 to 10080 (7 days)
        #[arg(long, default_value_t = 1440, value_parser = clap::value_parser!(u32).range(5..=10080))]
        expires_in_minutes: u32,
        /// Poll until the sandbox is ready or has failed (at most 10 minutes)
        #[arg(long)]
        wait: bool,
    },
    /// One sandbox or hosted mock: status, endpoint, health, expiry and failure
    Show {
        /// Sandbox name or id
        #[arg(long)]
        sandbox: String,
    },
    /// Lifecycle events of a sandbox or hosted mock, newest first
    Events {
        /// Sandbox name or id
        #[arg(long)]
        sandbox: String,
        /// At most this many events (1 to 200; the server default is 100)
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=200))]
        limit: Option<u32>,
    },
    /// The URL and access header of a ready sandbox; the access key only with --show-secret
    Connection {
        /// Sandbox name or id
        #[arg(long)]
        sandbox: String,
        /// Print the sandbox access key (as the dashboard's "Reveal connection" does)
        #[arg(long)]
        show_secret: bool,
    },
    /// Reset a ready sandbox's stateful mock data to the declared initial state
    Reset {
        /// Sandbox name or id
        #[arg(long)]
        sandbox: String,
        /// Do not ask for confirmation
        #[arg(long)]
        yes: bool,
    },
    /// Delete a sandbox or hosted mock: its URL stops serving and the provider runtime is removed
    Delete {
        /// Sandbox name or id
        #[arg(long)]
        sandbox: String,
        /// Do not ask for confirmation
        #[arg(long)]
        yes: bool,
    },
}

/// How long `create --wait` follows a sandbox.
const WAIT_LIMIT: Duration = Duration::from_secs(10 * 60);

/// Confirmation eligibility is checked before authentication or sandbox lookup.
pub(crate) fn asks_first(args: &SandboxesArgs) -> Option<bool> {
    match &args.action {
        Action::Reset { yes, .. } | Action::Delete { yes, .. } => Some(*yes),
        _ => None,
    }
}

pub async fn handle(args: &SandboxesArgs) -> Result<()> {
    let api = Api::from_profile()?;
    run(&api, args).await
}

pub async fn run(api: &Api, args: &SandboxesArgs) -> Result<()> {
    let slug = cloud_cli::project_slug(api, args.project.as_deref()).await?;
    let slug = slug.as_str();
    let json = args.json;
    match &args.action {
        Action::List => {
            let listed = list(api, slug).await?;
            cloud_cli::print(json, &listed, list_lines)
        }
        Action::Create { name, kind, version, access, expires_in_minutes, wait } => {
            let body = create_body(name, kind, version.as_deref(), access, *expires_in_minutes)?;
            let created = call(api, Method::POST, &["projects", slug, "sandboxes"], &[], Some(&body)).await?;
            let id = cloud_cli::text(&created, "/id");
            let value = if *wait && id != "-" { wait_ready(api, slug, &id).await? } else { created };
            cloud_cli::print(json, &value, |v| create_lines(v, *wait))
        }
        Action::Show { sandbox } => {
            let id = sandbox_id(api, slug, sandbox).await?;
            let value = fetch(api, slug, &id).await?;
            cloud_cli::print(json, &value, show_lines)
        }
        Action::Events { sandbox, limit } => {
            let id = sandbox_id(api, slug, sandbox).await?;
            let query: Vec<(&str, String)> = limit.map(|n| ("limit", n.to_string())).into_iter().collect();
            let events = call(api, Method::GET, &["projects", slug, "sandboxes", &id, "events"], &query, None).await?;
            cloud_cli::print(json, &events, event_lines)
        }
        Action::Connection { sandbox, show_secret } => {
            let id = sandbox_id(api, slug, sandbox).await?;
            let connection = call(api, Method::POST, &["projects", slug, "sandboxes", &id, "connection"], &[], None).await?;
            let shown = redact(&connection, *show_secret);
            cloud_cli::print(json, &shown, |v| connection_lines(v, *show_secret))
        }
        Action::Reset { sandbox, yes } => {
            let (id, described) = resolve(api, slug, sandbox).await?;
            cloud_cli::confirm(*yes, &format!("Reset {described}? Its stateful mock data returns to the initial state declared by the project's current mock configuration; the URL and access key stay the same."))?;
            let value = call(api, Method::POST, &["projects", slug, "sandboxes", &id, "reset"], &[], None).await?;
            cloud_cli::print(json, &value, |v| vec![format!("Reset requested for `{}`: status {}. Follow it with `axiom project sandboxes events --sandbox {}`.", cloud_cli::text(v, "/name"), cloud_cli::text(v, "/status"), cloud_cli::text(v, "/id"))])
        }
        Action::Delete { sandbox, yes } => {
            let (id, described) = resolve(api, slug, sandbox).await?;
            cloud_cli::confirm(*yes, &format!("Delete {described}? Its URL stops serving, the managed runtime is removed and its access key no longer works."))?;
            let value = call(api, Method::DELETE, &["projects", slug, "sandboxes", &id], &[], None).await?;
            cloud_cli::print(json, &value, |v| vec![format!("Deletion of `{}` accepted: status {}.", cloud_cli::text(v, "/name"), cloud_cli::text(v, "/status"))])
        }
    }
}

/// The release a user goes back to when the server has no `/sandboxes` routes (N−1, the last
/// CLI with `axiom project environments`).
pub const PREVIOUS_CLI: &str = "0.153.0";

/// The error for a server that predates the `/sandboxes` routes.
pub fn predates_message() -> String {
    format!("the server predates this CLI; install CLI {PREVIOUS_CLI}: `curl -fsSL https://axiomcore.dev/install | VERSION={PREVIOUS_CLI} bash` (HTTP 404)")
}

/// A server from before the rename has no `/sandboxes` route, so its router answers the
/// default not-found reply: `404` with the plain-text body `404 page not found` (chi's default
/// `NotFound`, which the backend does not override) and no `AX*` code. Every 404 the current
/// server sends here is different: the moved-API tombstone carries `AXSANDBOX_MOVED`, and a
/// missing project or sandbox says `project not found` or `managed environment not found`.
/// Those keep the normal wording.
pub fn predates_sandboxes(reply: &Reply) -> bool {
    reply.status == 404 && reply.body.pointer("/error/code").or_else(|| reply.body.get("code")).is_none() && reply.text.as_deref().map(str::trim) == Some("404 page not found")
}

/// [`cloud_cli::call`] for the `/sandboxes` routes, with the old-server reply worded as such.
async fn call(api: &Api, method: Method, segments: &[&str], query: &[(&str, String)], body: Option<&Value>) -> Result<Value> {
    let reply = api.request(&method, segments, query, body, None).await?;
    if (200..300).contains(&reply.status) {
        return Ok(reply.body);
    }
    if predates_sandboxes(&reply) {
        bail!(predates_message());
    }
    Err(cloud_cli::error(&reply))
}

async fn list(api: &Api, slug: &str) -> Result<Value> {
    call(api, Method::GET, &["projects", slug, "sandboxes"], &[], None).await
}

async fn fetch(api: &Api, slug: &str, id: &str) -> Result<Value> {
    call(api, Method::GET, &["projects", slug, "sandboxes", id], &[], None).await
}

/// The id of `--sandbox`: a UUID is used as given; a name is looked up in the list.
async fn sandbox_id(api: &Api, slug: &str, wanted: &str) -> Result<String> {
    let wanted = wanted.trim();
    if uuid::Uuid::parse_str(wanted).is_ok() && wanted.len() == 36 {
        return Ok(wanted.to_string());
    }
    Ok(resolve(api, slug, wanted).await?.0)
}

/// The id of `--sandbox` (an id or a name) and its description for a confirmation, from the
/// list. Live environments are matched first; deleted or expired ones only when no live one
/// matches. Two matches at the same step (for example a `sandbox` and a `hosted_mock` both named
/// `sandbox`) are refused with their ids.
async fn resolve(api: &Api, slug: &str, wanted: &str) -> Result<(String, String)> {
    let listed = list(api, slug).await?;
    let all = cloud_cli::items(&listed, &["environments", "items"]);
    let ended = |v: &&Value| matches!(v.get("status").and_then(Value::as_str), Some("deleted" | "expired"));
    let live: Vec<&Value> = all.iter().filter(|v| !ended(v)).copied().collect();
    let gone: Vec<&Value> = all.iter().filter(|v| ended(v)).copied().collect();
    let found = match cloud_cli::pick(&live, wanted, &["name"], "environment") {
        Ok(found) => found,
        Err(error) if error.to_string().starts_with("no environment") && !gone.is_empty() => cloud_cli::pick(&gone, wanted, &["name"], "environment")?,
        Err(error) => return Err(error),
    };
    let id = found.get("id").and_then(Value::as_str).filter(|id| !id.is_empty()).ok_or_else(|| anyhow::anyhow!("the environment list has no id for `{}`", cloud_cli::clean(wanted)))?.to_string();
    let described = format!("{} environment `{}` ({}, {})", cloud_cli::text(found, "/kind"), cloud_cli::text(found, "/name"), cloud_cli::text(found, "/status"), id);
    Ok((id, described))
}

/// Validates the flags as the server does and builds `POST …/sandboxes`.
pub fn create_body(name: &str, kind: &str, version: Option<&str>, access: &str, expires_in_minutes: u32) -> Result<Value> {
    let name = name.trim().to_ascii_lowercase();
    let valid = name.len() <= 49 && name.starts_with(|c: char| c.is_ascii_lowercase()) && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !valid {
        bail!("--name must start with a letter and use only lowercase letters, numbers and hyphens (at most 49 characters)");
    }
    if !matches!(kind, "sandbox" | "hosted_mock") {
        bail!("--kind must be sandbox or hosted_mock");
    }
    if !matches!(access, "token" | "public") {
        bail!("--access must be token or public");
    }
    if !(5..=10080).contains(&expires_in_minutes) {
        bail!("--expires-in-minutes must be between 5 and 10080 (7 days)");
    }
    let mut body = json!({"kind": kind, "name": name, "access_mode": access, "expires_in_minutes": expires_in_minutes});
    if let Some(version) = version.map(str::trim).filter(|v| !v.is_empty()) {
        body["version"] = json!(version);
    }
    Ok(body)
}

/// Follows a requested sandbox until it is ready; a failed, expired or deleted one is an error.
async fn wait_ready(api: &Api, slug: &str, id: &str) -> Result<Value> {
    let deadline = Instant::now() + WAIT_LIMIT;
    loop {
        let value = fetch(api, slug, id).await?;
        let status = cloud_cli::text(&value, "/status");
        match status.as_str() {
            "ready" => return Ok(value),
            "failed" => bail!("The sandbox failed: {} {}", cloud_cli::text(&value, "/failure_code"), cloud_cli::text(&value, "/failure_message")),
            "expired" | "deleted" => bail!("The sandbox is {status}."),
            _ => {}
        }
        if Instant::now() >= deadline {
            bail!("The sandbox is still {status} after 10 minutes. Follow it with `axiom project sandboxes show --sandbox {id}`.");
        }
        tokio::time::sleep(api.wait_interval).await;
    }
}

/// The connection reply without the access key unless `show` is set.
pub fn redact(connection: &Value, show: bool) -> Value {
    let mut value = connection.clone();
    if !show {
        if let Some(object) = value.as_object_mut() {
            object.remove("access_token");
        }
    }
    value
}

pub fn list_lines(value: &Value) -> Vec<String> {
    let list = cloud_cli::items(value, &["environments", "items"]);
    if list.is_empty() {
        return vec!["No managed sandboxes. Create one with `axiom project sandboxes create`.".into()];
    }
    list.iter()
        .map(|e| format!("{}  {}  {}  {}  expires {}  {}", cloud_cli::text(e, "/name"), cloud_cli::text(e, "/kind"), cloud_cli::text(e, "/status"), cloud_cli::text(e, "/endpoint"), cloud_cli::text(e, "/expires_at"), cloud_cli::text(e, "/id")))
        .collect()
}

fn create_lines(value: &Value, waited: bool) -> Vec<String> {
    let name = cloud_cli::text(value, "/name");
    let status = cloud_cli::text(value, "/status");
    if waited || status == "ready" {
        return vec![format!("Environment `{name}` is {status} at {}. Get the access header with `axiom project sandboxes connection --sandbox {name}`.", cloud_cli::text(value, "/endpoint"))];
    }
    vec![format!("Requested environment `{name}` ({}): status {status}. Follow it with `axiom project sandboxes show --sandbox {name}`, or pass --wait.", cloud_cli::text(value, "/id"))]
}

pub fn show_lines(e: &Value) -> Vec<String> {
    let mut lines = vec![
        format!("Name: {}", cloud_cli::text(e, "/name")),
        format!("Id: {}", cloud_cli::text(e, "/id")),
        format!("Kind: {}", cloud_cli::text(e, "/kind")),
        format!("Status: {}", cloud_cli::text(e, "/status")),
        format!("Access: {}", cloud_cli::text(e, "/access_mode")),
        format!("Endpoint: {}", cloud_cli::text(e, "/endpoint")),
        format!("Health: {} {}", cloud_cli::text(e, "/health_status"), cloud_cli::text(e, "/health_detail")),
        format!("State resets: {}", cloud_cli::text(e, "/reset_generation")),
        format!("Requested: {}", cloud_cli::text(e, "/requested_at")),
        format!("Ready: {}", cloud_cli::text(e, "/ready_at")),
        format!("Expires: {}", cloud_cli::text(e, "/expires_at")),
    ];
    if cloud_cli::text(e, "/failure_code") != "-" || cloud_cli::text(e, "/failure_message") != "-" {
        lines.push(format!("Failure: {} {}", cloud_cli::text(e, "/failure_code"), cloud_cli::text(e, "/failure_message")));
    }
    lines
}

pub fn event_lines(value: &Value) -> Vec<String> {
    let list = cloud_cli::items(value, &["events", "items"]);
    if list.is_empty() {
        return vec!["No events.".into()];
    }
    list.iter()
        .map(|e| format!("{}  {}  {}  {}", cloud_cli::text(e, "/created_at"), cloud_cli::text(e, "/kind"), cloud_cli::text(e, "/status"), cloud_cli::text(e, "/message")))
        .collect()
}

/// Human lines of a connection reply; the key appears only when `show` is set.
pub fn connection_lines(value: &Value, show: bool) -> Vec<String> {
    let mut lines = vec![format!("Endpoint: {}", cloud_cli::text(value, "/endpoint"))];
    match value.get("access_token").and_then(Value::as_str).filter(|t| !t.is_empty()) {
        Some(token) if show => {
            lines.push(format!("Header: {}", cloud_cli::text(value, "/header_name")));
            lines.push(format!("Key: {}", cloud_cli::clean(token)));
        }
        _ if cloud_cli::text(value, "/header_name") != "-" => {
            lines.push(format!("Header: {}", cloud_cli::text(value, "/header_name")));
            lines.push("Key: hidden; pass --show-secret to print it".into());
        }
        _ => lines.push("Access: public (no key needed)".into()),
    }
    lines.push(format!("Expires: {}", cloud_cli::text(value, "/expires_at")));
    lines
}

#[cfg(test)]
mod tests {
    use super::super::cloud_cli::test_support::harness;
    use super::*;
    use clap::Parser;

    const ID: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    const OLD: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
    const BASE: &str = "/api/v1/projects/demo/sandboxes";

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        args: SandboxesArgs,
    }

    fn args(argv: &[&str]) -> SandboxesArgs {
        Cli::try_parse_from(std::iter::once("axiom").chain(argv.iter().copied())).unwrap().args
    }

    fn sandbox(id: &str, status: &str) -> Value {
        json!({"id": id, "name": "sandbox", "kind": "sandbox", "status": status, "access_mode": "token", "endpoint": "https://sbx.example.test", "expires_at": "2026-11-14T00:00:00Z"})
    }

    async fn setup() -> super::super::cloud_cli::test_support::Harness {
        let h = harness().await;
        h.mock.script("GET /api/v1/projects", 200, json!([{"id": "p1", "slug": "demo", "name": "Demo"}]));
        h.mock.script(&format!("GET {BASE}"), 200, json!([sandbox(ID, "ready"), sandbox(OLD, "deleted")]));
        h
    }

    #[tokio::test]
    async fn list_calls_the_project_sandboxes() {
        let h = setup().await;
        run(&h.api, &args(&["--project", "Demo", "list"])).await.unwrap();
        let seen = h.mock.seen(&format!("GET {BASE}"));
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].query, None);
        assert_eq!(seen[0].bearer.as_deref(), Some("access"));
        assert_eq!(list_lines(&json!([]))[0], "No managed sandboxes. Create one with `axiom project sandboxes create`.");
        assert!(list_lines(&json!([sandbox(ID, "ready")]))[0].starts_with("sandbox  sandbox  ready  https://sbx.example.test"));
    }

    #[tokio::test]
    async fn create_sends_the_dashboard_body_and_waits_until_ready() {
        let h = setup().await;
        h.mock.script(&format!("POST {BASE}"), 202, sandbox(ID, "requested"));
        h.mock.script(&format!("GET {BASE}/{ID}"), 200, sandbox(ID, "provisioning")).script(&format!("GET {BASE}/{ID}"), 200, sandbox(ID, "ready"));
        run(&h.api, &args(&["--project", "demo", "create", "--wait"])).await.unwrap();
        let seen = h.mock.seen(&format!("POST {BASE}"));
        assert_eq!(seen[0].body, json!({"kind": "sandbox", "name": "sandbox", "access_mode": "token", "expires_in_minutes": 1440}));
        assert_eq!(h.mock.seen(&format!("GET {BASE}/{ID}")).len(), 2);

        run(&h.api, &args(&["--project", "demo", "create", "--name", "Preview-1", "--kind", "hosted_mock", "--version", "1.2.0", "--access", "public", "--expires-in-minutes", "60"])).await.unwrap();
        let seen = h.mock.seen(&format!("POST {BASE}"));
        assert_eq!(seen[1].body, json!({"kind": "hosted_mock", "name": "preview-1", "version": "1.2.0", "access_mode": "public", "expires_in_minutes": 60}));
        assert_eq!(h.mock.seen(&format!("GET {BASE}/{ID}")).len(), 2, "no polling without --wait");
    }

    #[tokio::test]
    async fn create_wait_reports_a_failed_sandbox() {
        let h = setup().await;
        h.mock.script(&format!("POST {BASE}"), 202, sandbox(ID, "requested"));
        let mut failed = sandbox(ID, "failed");
        failed["failure_code"] = json!("provider_error");
        failed["failure_message"] = json!("health check failed");
        h.mock.script(&format!("GET {BASE}/{ID}"), 200, failed);
        let e = run(&h.api, &args(&["--project", "demo", "create", "--wait"])).await.unwrap_err().to_string();
        assert_eq!(e, "The sandbox failed: provider_error health check failed");
    }

    #[test]
    fn create_body_mirrors_the_server_validation() {
        assert!(create_body("1abc", "sandbox", None, "token", 60).is_err());
        assert!(create_body("has_underscore", "sandbox", None, "token", 60).is_err());
        assert!(create_body(&"a".repeat(50), "sandbox", None, "token", 60).is_err());
        assert!(create_body(&"a".repeat(49), "sandbox", None, "token", 60).is_ok());
        assert!(create_body("ok", "sandbox", None, "token", 4).is_err());
        assert!(create_body("ok", "sandbox", None, "token", 10081).is_err());
        assert!(create_body("ok", "vm", None, "token", 60).is_err());
        assert!(create_body("ok", "sandbox", None, "open", 60).is_err());
        assert_eq!(create_body(" Ok ", "sandbox", Some("  "), "token", 5).unwrap(), json!({"kind": "sandbox", "name": "ok", "access_mode": "token", "expires_in_minutes": 5}));
        assert!(Cli::try_parse_from(["axiom", "create", "--expires-in-minutes", "4"]).is_err());
        assert!(Cli::try_parse_from(["axiom", "create", "--expires-in-minutes", "10081"]).is_err());
    }

    #[tokio::test]
    async fn disabled_managed_sandboxes_show_the_server_reason() {
        // The real server answers http.Error plain text; the shared mock always answers JSON,
        // so this test serves the plain-text 409 itself.
        use axum::http::StatusCode;
        use axum::routing::{get, post};
        let reason = "managed sandbox provisioning is disabled for this environment; run your contract locally with `axiom serve`";
        let router = axum::Router::new()
            .route("/api/v1/projects", get(|| async { axum::Json(json!([{"id": "p1", "slug": "demo", "name": "Demo"}])) }))
            .route("/api/v1/projects/demo/sandboxes", post(move || async move { (StatusCode::CONFLICT, format!("{reason}\n")) }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        let dir = tempfile::tempdir().unwrap();
        let api = Api::new(&base, "access".into(), "refresh".into(), false, super::super::step_up::StepUpStore::at(dir.path().join("step-up.json"))).unwrap().fast();
        let e = run(&api, &args(&["--project", "demo", "create"])).await.unwrap_err().to_string();
        assert_eq!(e, format!("This conflicts with the current state: {reason} (HTTP 409)"));

        // The same reason as a JSON message through the scripted mock.
        let h = setup().await;
        h.mock.script(&format!("POST {BASE}/{ID}/reset"), 409, json!({"message": reason}));
        let e = run(&h.api, &args(&["--project", "demo", "reset", "--sandbox", "sandbox", "--yes"])).await.unwrap_err().to_string();
        assert_eq!(e, format!("This conflicts with the current state: {reason} (HTTP 409)"));
    }

    #[tokio::test]
    async fn names_resolve_to_the_live_environment_and_ids_skip_the_lookup() {
        let h = setup().await;
        h.mock.script(&format!("GET {BASE}/{ID}"), 200, sandbox(ID, "ready"));
        run(&h.api, &args(&["--project", "demo", "show", "--sandbox", "sandbox"])).await.unwrap();
        assert_eq!(h.mock.seen(&format!("GET {BASE}/{ID}")).len(), 1, "the live one, not the deleted one");
        assert_eq!(h.mock.seen(&format!("GET {BASE}")).len(), 1);
        h.mock.script(&format!("GET {BASE}/{OLD}"), 200, sandbox(OLD, "deleted"));
        run(&h.api, &args(&["--project", "demo", "show", "--sandbox", OLD])).await.unwrap();
        assert_eq!(h.mock.seen(&format!("GET {BASE}/{OLD}")).len(), 1);
        assert_eq!(h.mock.seen(&format!("GET {BASE}")).len(), 1, "a UUID skips the list");
        let e = run(&h.api, &args(&["--project", "demo", "show", "--sandbox", "nope"])).await.unwrap_err().to_string();
        assert!(e.contains("no environment `nope`"), "{e}");
    }

    #[tokio::test]
    async fn events_send_the_limit_query() {
        let h = setup().await;
        h.mock.script(&format!("GET {BASE}/{ID}/events"), 200, json!([{"id": "e1", "kind": "ready", "status": "passed", "message": "Ready.", "created_at": "2026-11-13T10:00:00Z"}]));
        run(&h.api, &args(&["--project", "demo", "events", "--sandbox", "sandbox", "--limit", "20"])).await.unwrap();
        run(&h.api, &args(&["--project", "demo", "events", "--sandbox", "sandbox"])).await.unwrap();
        let seen = h.mock.seen(&format!("GET {BASE}/{ID}/events"));
        assert_eq!(seen[0].query.as_deref(), Some("limit=20"));
        assert_eq!(seen[1].query, None);
        assert_eq!(event_lines(&json!([])), vec!["No events."]);
        assert!(Cli::try_parse_from(["axiom", "events", "--sandbox", "x", "--limit", "201"]).is_err());
    }

    #[tokio::test]
    async fn connection_posts_an_empty_body_and_hides_the_key() {
        let h = setup().await;
        let reply = json!({"endpoint": "https://sbx.example.test", "header_name": "X-Axiom-Sandbox-Key", "access_token": "s3cr3t-key-value", "environment_id": ID});
        h.mock.script(&format!("POST {BASE}/{ID}/connection"), 200, reply.clone());
        run(&h.api, &args(&["--project", "demo", "connection", "--sandbox", "sandbox"])).await.unwrap();
        run(&h.api, &args(&["--project", "demo", "--json", "connection", "--sandbox", "sandbox", "--show-secret"])).await.unwrap();
        let seen = h.mock.seen(&format!("POST {BASE}/{ID}/connection"));
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].body, Value::Null);

        let hidden = connection_lines(&reply, false).join("\n");
        assert!(!hidden.contains("s3cr3t"), "{hidden}");
        assert!(hidden.contains("Header: X-Axiom-Sandbox-Key") && hidden.contains("--show-secret"), "{hidden}");
        assert!(!redact(&reply, false).to_string().contains("s3cr3t"));
        assert!(connection_lines(&reply, true).join("\n").contains("Key: s3cr3t-key-value"));
        assert_eq!(redact(&reply, true), reply);
        let public = json!({"endpoint": "https://sbx.example.test", "environment_id": ID});
        assert!(connection_lines(&public, false).contains(&"Access: public (no key needed)".to_string()));
    }

    #[tokio::test]
    async fn reset_and_delete_need_yes() {
        let h = setup().await;
        h.mock.script(&format!("POST {BASE}/{ID}/reset"), 202, sandbox(ID, "resetting"));
        h.mock.script(&format!("DELETE {BASE}/{ID}"), 202, sandbox(ID, "deleting"));
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            let e = run(&h.api, &args(&["--project", "demo", "reset", "--sandbox", "sandbox"])).await.unwrap_err().to_string();
            assert!(e.contains("--yes") && e.contains("initial state"), "{e}");
            let e = run(&h.api, &args(&["--project", "demo", "delete", "--sandbox", "sandbox"])).await.unwrap_err().to_string();
            assert!(e.contains("--yes") && e.contains("stops serving"), "{e}");
            assert!(h.mock.seen(&format!("POST {BASE}/{ID}/reset")).is_empty());
            assert!(h.mock.seen(&format!("DELETE {BASE}/{ID}")).is_empty());
        }
        run(&h.api, &args(&["--project", "demo", "reset", "--sandbox", "sandbox", "--yes"])).await.unwrap();
        run(&h.api, &args(&["--project", "demo", "delete", "--sandbox", ID, "--yes"])).await.unwrap();
        assert_eq!(h.mock.seen(&format!("POST {BASE}/{ID}/reset"))[0].body, Value::Null);
        assert_eq!(h.mock.seen(&format!("DELETE {BASE}/{ID}")).len(), 1);
        assert_eq!(h.mock.seen(&format!("DELETE {BASE}/{ID}"))[0].body, Value::Null);
    }

    #[tokio::test]
    async fn a_name_shared_by_two_live_environments_is_refused_with_their_ids() {
        let h = harness().await;
        h.mock.script("GET /api/v1/projects", 200, json!([{"id": "p1", "slug": "demo", "name": "Demo"}]));
        let mut mock_env = sandbox(OLD, "ready");
        mock_env["kind"] = json!("hosted_mock");
        h.mock.script(&format!("GET {BASE}"), 200, json!([sandbox(ID, "ready"), mock_env]));
        let e = run(&h.api, &args(&["--project", "demo", "delete", "--sandbox", "sandbox", "--yes"])).await.unwrap_err().to_string();
        assert!(e.contains("matches 2 environments") && e.contains(&format!("{ID} (sandbox, ready)")) && e.contains(&format!("{OLD} (hosted_mock, ready)")), "{e}");
        assert!(h.mock.seen(&format!("DELETE {BASE}/{ID}")).is_empty() && h.mock.seen(&format!("DELETE {BASE}/{OLD}")).is_empty());
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            let e = run(&h.api, &args(&["--project", "demo", "reset", "--sandbox", ID])).await.unwrap_err().to_string();
            assert!(e.contains(&format!("Reset sandbox environment `sandbox` (ready, {ID})?")), "{e}");
        }
    }

    /// Every subcommand, in order, against the scripted server: exactly these methods, paths,
    /// queries and bodies, all under `/sandboxes` (never `/environments`).
    #[tokio::test]
    async fn every_subcommand_calls_the_sandboxes_routes() {
        let h = setup().await;
        h.mock.script(&format!("POST {BASE}"), 202, sandbox(ID, "requested"));
        h.mock.script(&format!("GET {BASE}/{ID}"), 200, sandbox(ID, "ready"));
        h.mock.script(&format!("GET {BASE}/{ID}/events"), 200, json!([]));
        h.mock.script(&format!("POST {BASE}/{ID}/connection"), 200, json!({"endpoint": "https://sbx.example.test", "environment_id": ID}));
        h.mock.script(&format!("POST {BASE}/{ID}/reset"), 202, sandbox(ID, "resetting"));
        h.mock.script(&format!("DELETE {BASE}/{ID}"), 202, sandbox(ID, "deleting"));
        for argv in [
            vec!["list"],
            vec!["show", "--sandbox", ID],
            vec!["create", "--name", "preview", "--wait"],
            vec!["events", "--sandbox", ID, "--limit", "5"],
            vec!["connection", "--sandbox", ID],
            vec!["reset", "--sandbox", ID, "--yes"],
            vec!["delete", "--sandbox", ID, "--yes"],
        ] {
            let full: Vec<&str> = ["--project", "demo"].into_iter().chain(argv.iter().copied()).collect();
            run(&h.api, &args(&full)).await.unwrap_or_else(|e| panic!("{argv:?}: {e}"));
        }
        let seen: Vec<(String, Option<String>, Value)> = h.mock.requests().into_iter().filter(|s| s.route != "GET /api/v1/projects").map(|s| (s.route, s.query, s.body)).collect();
        let created = json!({"kind": "sandbox", "name": "preview", "access_mode": "token", "expires_in_minutes": 1440});
        let want = vec![
            (format!("GET {BASE}"), None, Value::Null),
            (format!("GET {BASE}/{ID}"), None, Value::Null),
            (format!("POST {BASE}"), None, created),
            (format!("GET {BASE}/{ID}"), None, Value::Null),
            (format!("GET {BASE}/{ID}/events"), Some("limit=5".to_string()), Value::Null),
            (format!("POST {BASE}/{ID}/connection"), None, Value::Null),
            (format!("GET {BASE}"), None, Value::Null),
            (format!("POST {BASE}/{ID}/reset"), None, Value::Null),
            (format!("GET {BASE}"), None, Value::Null),
            (format!("DELETE {BASE}/{ID}"), None, Value::Null),
        ];
        assert_eq!(seen, want);
        assert!(h.mock.requests().iter().all(|s| !s.route.contains("/environments")));
    }

    /// The new server keeps the old `/environments` shapes as tombstones; their reply reads
    /// through the shared formatter with the code first.
    #[tokio::test]
    async fn the_moved_api_tombstone_is_shown_with_its_code() {
        let message = "The sandbox API has moved. Reload the dashboard, or install the current Axiom CLI and use `axiom project sandboxes`.";
        let h = harness().await;
        h.mock.script("GET /api/v1/projects", 200, json!([{"id": "p1", "slug": "demo", "name": "Demo"}]));
        h.mock.script(&format!("GET {BASE}"), 404, json!({"code": "AXSANDBOX_MOVED", "message": message}));
        let e = run(&h.api, &args(&["--project", "demo", "list"])).await.unwrap_err().to_string();
        assert_eq!(e, format!("AXSANDBOX_MOVED: Not found: {message} (HTTP 404)"));
        let reply = Reply { status: 404, body: json!({"code": "AXSANDBOX_MOVED", "message": message}), retry_after: None, text: None };
        assert!(!predates_sandboxes(&reply));
        assert_eq!(cloud_cli::error(&reply).to_string(), format!("AXSANDBOX_MOVED: Not found: {message} (HTTP 404)"));
    }

    /// A server from before the rename answers chi's plain `404 page not found` on
    /// `/sandboxes`: the CLI says to install the previous release, before any mutation.
    /// The current server's own 404s (a missing project or sandbox) keep their wording.
    #[tokio::test]
    async fn an_old_server_plain_404_names_the_previous_cli() {
        use axum::http::StatusCode;
        use axum::routing::get;
        let router = axum::Router::new()
            .route("/api/v1/projects", get(|| async { axum::Json(json!([{"id": "p1", "slug": "demo", "name": "Demo"}])) }))
            .route("/api/v1/projects/demo/environments", get(|| async { axum::Json(json!([])) }))
            .fallback(|| async { (StatusCode::NOT_FOUND, "404 page not found\n") });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        let dir = tempfile::tempdir().unwrap();
        let api = Api::new(&base, "access".into(), "refresh".into(), false, super::super::step_up::StepUpStore::at(dir.path().join("step-up.json"))).unwrap().fast();
        let want = "the server predates this CLI; install CLI 0.153.0: `curl -fsSL https://axiomcore.dev/install | VERSION=0.153.0 bash` (HTTP 404)";
        for argv in [vec!["list"], vec!["show", "--sandbox", ID], vec!["create"], vec!["reset", "--sandbox", "sandbox", "--yes"], vec!["delete", "--sandbox", "sandbox", "--yes"]] {
            let full: Vec<&str> = ["--project", "demo"].into_iter().chain(argv.iter().copied()).collect();
            assert_eq!(run(&api, &args(&full)).await.unwrap_err().to_string(), want, "{argv:?}");
        }
        assert_eq!(predates_message(), want);

        let plain = |text: &str| Reply { status: 404, body: Value::Null, retry_after: None, text: Some(text.into()) };
        assert!(predates_sandboxes(&plain("404 page not found")));
        assert!(!predates_sandboxes(&plain("managed environment not found")));
        assert!(!predates_sandboxes(&plain("project not found")));
        assert!(!predates_sandboxes(&Reply { status: 404, body: json!({"code": "AXRESOURCE_INVALID"}), retry_after: None, text: None }));
        assert!(!predates_sandboxes(&Reply { status: 405, body: Value::Null, retry_after: None, text: Some("404 page not found".into()) }));
        assert_eq!(cloud_cli::error(&plain("managed environment not found")).to_string(), "Not found: managed environment not found (HTTP 404)");
    }
}
