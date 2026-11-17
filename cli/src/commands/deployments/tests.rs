//! Every `axiom deploy` / `axiom deployments` command against the scripted loopback API:
//! method, path, body, `Idempotency-Key` and `If-Match`. The parity session is written to
//! `cli/tests/fixtures/deployments-parity.json` for the dashboard's recorder tests
//! (regenerate with `AXIOM_UPDATE_FIXTURES=1`).
use super::*;
use crate::commands::cloud_mock::{Mock, Seen};
use clap::Parser;

const ENV: &str = "11111111-2222-3333-4444-555555555555";
const B: &str = "/api/v1/projects/demo/environments/11111111-2222-3333-4444-555555555555/deployments";
const URL: &str = "https://demo-development-a1b2c3.axiomcore.page";
const HTML: &str = "<h1>hi</h1>";
const APP: &str = "console.log(1)";

#[derive(Parser)]
struct FrontendCli {
    #[command(flatten)]
    args: FrontendArgs,
}
#[derive(Parser)]
struct BackendCli {
    #[command(flatten)]
    args: BackendArgs,
}
#[derive(Parser)]
struct DeploymentsCli {
    #[command(flatten)]
    args: DeploymentsArgs,
}

fn argv<'a>(rest: &[&'a str]) -> Vec<&'a str> {
    std::iter::once("axiom").chain(rest.iter().copied()).collect()
}

struct Harness {
    mock: Mock,
    base: String,
    ctx: Ctx,
    _dir: tempfile::TempDir,
}

impl Harness {
    async fn new() -> Self {
        let mock = Mock::default();
        let base = mock.start().await;
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx {
            client: CloudClient::for_loopback(&base, "access".into(), "refresh".into()).unwrap(),
            poll: Duration::from_millis(1),
            timeout: Duration::from_secs(10),
            journal_dir: dir.path().join("deployments"),
            cwd: dir.path().to_path_buf(),
        };
        mock.script("GET /api/v1/projects", 200, json!([{"id": "p1", "slug": "demo", "name": "Demo", "visibility": "private", "created_at": "t", "updated_at": "t"}]));
        Harness { mock, base, ctx, _dir: dir }
    }
    fn script(&self, route: &str, status: u16, body: Value) -> &Self {
        self.mock.script(route, status, body);
        self
    }
    /// Requests to the deployments API and the upload URLs (no project/environment lookups).
    fn deployment_requests(&self) -> Vec<Seen> {
        self.mock.requests().into_iter().filter(|s| s.route.contains("/deployments") || s.route.starts_with("PUT /r2/")).collect()
    }
    fn routes(&self) -> Vec<String> {
        self.deployment_requests().into_iter().map(|s| s.route).collect()
    }
    async fn deploy_frontend(&self, site: &Path, extra: &[&str]) -> Result<()> {
        let mut args = vec![site.to_str().unwrap(), "--project", "demo", "--environment", ENV];
        args.extend_from_slice(extra);
        let cli = FrontendCli::try_parse_from(argv(&args)).unwrap();
        let b = frontend_bundle(&cli.args, &self.ctx.cwd)?;
        deploy(&self.ctx, Kind::Frontend, &b, &cli.args.flags).await
    }
    async fn deployments(&self, args: &[&str]) -> Result<()> {
        let mut full = vec!["--project", "demo", "--environment", ENV];
        full.extend_from_slice(args);
        let cli = DeploymentsCli::try_parse_from(argv(&full)).unwrap();
        run(&self.ctx, &cli.args).await
    }
}

fn fixture_site() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    std::fs::write(dir.path().join("index.html"), HTML).unwrap();
    std::fs::write(dir.path().join("assets/app.js"), APP).unwrap();
    dir
}

fn manifest_sha(site: &Path) -> String {
    bundle::frontend(site, Spa::Auto).unwrap().manifest_sha256
}

fn slot(generation: i64, state: &str, active: Option<&str>) -> Value {
    json!({"schema": SCHEMA, "deployment_id": "dpl_1", "kind": "frontend", "generation": generation, "state": state, "active_version_id": active, "url": URL, "updated_at": "2026-11-16T10:00:00Z"})
}

fn op(id: &str, state: &str) -> Value {
    json!({"operation_id": id, "state": state, "phase": if state == "running" { "verifying" } else { "done" }})
}

fn not_found() -> Value {
    json!({"schema": SCHEMA, "code": "AXDEPLOY_NOT_FOUND", "message": "not found", "request_id": "r1"})
}

/// Scripts one frontend deploy of the fixture site: upload `upl_1` asks for index.html and the
/// manifest (twice, to check dedupe), version `ver_2` builds through `op_v`, activation `op_a`.
fn script_deploy(h: &Harness, site: &Path, running_first: bool) {
    let manifest = manifest_sha(site);
    let html = bundle::sha256_hex(HTML.as_bytes());
    let target = |sha: &str| json!({"sha256": sha, "url": format!("{}/r2/{sha}", h.base), "method": "PUT", "headers": {"x-amz-checksum-sha256": "c2hh"}, "expires_at": "2026-11-16T10:15:00Z"});
    h.script(&format!("POST {B}/frontend/uploads"), 201, json!({"id": "upl_1", "state": "open", "missing_hashes": [html, manifest, html], "upload_urls": [target(&html), target(&manifest)]}));
    h.script(&format!("PUT /r2/{html}"), 200, json!({}));
    h.script(&format!("PUT /r2/{manifest}"), 200, json!({}));
    h.script(&format!("POST {B}/frontend/uploads/upl_1/complete"), 200, json!({"id": "upl_1", "state": "sealed"}));
    h.script(&format!("POST {B}/frontend/versions"), 202, json!({"deployment_id": "dpl_1", "generation": 1, "state": "empty", "version_id": "ver_2", "operation_id": "op_v", "url": URL}));
    if running_first {
        h.script(&format!("GET {B}/frontend/operations/op_v"), 200, op("op_v", "running"));
    }
    h.script(&format!("GET {B}/frontend/operations/op_v"), 200, op("op_v", "succeeded"));
    h.script(&format!("GET {B}/frontend/versions/ver_2"), 200, json!({"id": "ver_2", "status": "ready", "manifest_sha256": manifest}));
    h.script(&format!("POST {B}/frontend/versions/ver_2/activate"), 202, json!({"generation": 2, "state": "activating", "operation_id": "op_a", "desired_version_id": "ver_2", "url": URL}));
    h.script(&format!("GET {B}/frontend/operations/op_a"), 200, op("op_a", "succeeded"));
}

#[tokio::test]
async fn deploy_frontend_uploads_builds_activates_and_prints_the_url() {
    let h = Harness::new().await;
    let site = fixture_site();
    let manifest = manifest_sha(site.path());
    let html = bundle::sha256_hex(HTML.as_bytes());
    h.script(&format!("GET {B}/frontend"), 404, not_found()).script(&format!("GET {B}/frontend"), 200, slot(1, "empty", None)).script(&format!("GET {B}/frontend"), 200, slot(2, "active", Some("ver_2")));
    script_deploy(&h, site.path(), true);
    h.deploy_frontend(site.path(), &["--yes", "--message", "first"]).await.unwrap();

    let mut routes = h.routes();
    // The two PUTs run concurrently.
    routes[2..4].sort();
    let mut puts = [format!("PUT /r2/{html}"), format!("PUT /r2/{manifest}")];
    puts.sort();
    let want: Vec<String> = [
        format!("GET {B}/frontend"),
        format!("POST {B}/frontend/uploads"),
        puts[0].clone(),
        puts[1].clone(),
        format!("POST {B}/frontend/uploads/upl_1/complete"),
        format!("POST {B}/frontend/versions"),
        format!("GET {B}/frontend/operations/op_v"),
        format!("GET {B}/frontend/operations/op_v"),
        format!("GET {B}/frontend/versions/ver_2"),
        format!("GET {B}/frontend"),
        format!("POST {B}/frontend/versions/ver_2/activate"),
        format!("GET {B}/frontend/operations/op_a"),
        format!("GET {B}/frontend"),
    ]
    .to_vec();
    assert_eq!(routes, want);

    let one = |route: String| h.mock.seen(&route).remove(0);
    let upload = one(format!("POST {B}/frontend/uploads"));
    assert_eq!(
        upload.body,
        json!({"manifest_sha256": manifest, "manifest_bytes": bundle::frontend(site.path(), Spa::Auto).unwrap().manifest_bytes(), "total_bytes": 25, "file_count": 2, "purpose": "static_bundle", "files": [
            {"sha256": bundle::sha256_hex(APP.as_bytes()), "size": 14}, {"sha256": html, "size": 11}]})
    );
    // PUTs carry the server's headers and bytes, never the session token.
    let put = one(format!("PUT /r2/{html}"));
    assert_eq!((put.raw.as_slice(), put.bearer.as_deref()), (HTML.as_bytes(), None));
    let put = one(format!("PUT /r2/{manifest}"));
    assert_eq!(bundle::sha256_hex(&put.raw), manifest);
    assert!(String::from_utf8(put.raw).unwrap().starts_with(r#"{"schema":"axiom-static-bundle/v1","entrypoint":"/index.html","spa_fallback":true,"files":[{"path":"/assets/app.js""#));
    assert_eq!(one(format!("POST {B}/frontend/uploads/upl_1/complete")).body, json!({}));
    assert_eq!(one(format!("POST {B}/frontend/versions")).body, json!({"artifact": {"type": "static_bundle", "upload_id": "upl_1"}, "message": "first"}));
    let activate = one(format!("POST {B}/frontend/versions/ver_2/activate"));
    assert_eq!((activate.body.clone(), activate.if_match.as_deref()), (json!({}), Some("\"g1\"")));
    // Every mutation is keyed; the keys are distinct, deterministic and hex SHA-256.
    let keys: Vec<String> = h.deployment_requests().iter().filter(|s| s.route.starts_with("POST")).map(|s| s.key.clone().expect("Idempotency-Key")).collect();
    assert_eq!(keys.len(), 4);
    assert!(keys.iter().all(|k| k.len() == 64 && k.bytes().all(|b| b.is_ascii_hexdigit())));
    let mut unique = keys.clone();
    unique.dedup();
    assert_eq!(unique.len(), 4);
    let journal: Journal = serde_json::from_slice(&std::fs::read(h.ctx.journal_dir.join(format!("demo-{ENV}-frontend.json"))).unwrap()).unwrap();
    assert_eq!(keys[0], idempotency_key(&journal.intent_id, "create_upload", &manifest, 0));
    assert_eq!(keys[3], idempotency_key(&journal.intent_id, "activate", &manifest, 1));
    assert!(journal.activated && journal.version_id.as_deref() == Some("ver_2") && journal.url.as_deref() == Some(URL));

    // The same build again: the server confirms it is still live (one slot read), nothing
    // else is sent.
    let before = h.deployment_requests().len();
    h.deploy_frontend(site.path(), &["--yes"]).await.unwrap();
    assert_eq!(h.routes()[before..], [format!("GET {B}/frontend")]);
    // --new starts a fresh intent with new keys.
    h.deploy_frontend(site.path(), &["--yes", "--new", "--json"]).await.unwrap();
    let uploads = h.mock.seen(&format!("POST {B}/frontend/uploads"));
    assert_eq!(uploads.len(), 2);
    assert_ne!(uploads[0].key, uploads[1].key);
}

#[tokio::test]
async fn deploy_frontend_no_activate_stops_at_ready_and_a_rerun_activates_that_version() {
    let h = Harness::new().await;
    let site = fixture_site();
    // Each run reads the slot once at the start (the pilot check); the rerun's activation
    // reads it again.
    for (generation, active) in [(4, "ver_1"), (4, "ver_1"), (4, "ver_1"), (5, "ver_2")] {
        h.script(&format!("GET {B}/frontend"), 200, slot(generation, "active", Some(active)));
    }
    script_deploy(&h, site.path(), false);
    h.deploy_frontend(site.path(), &["--no-activate", "--wait", "--json"]).await.unwrap();
    let routes = h.routes();
    assert!(!routes.iter().any(|r| r.ends_with("/activate")), "{routes:?}");
    assert_eq!(routes.last().unwrap(), &format!("GET {B}/frontend/versions/ver_2"));
    let first_upload = h.mock.seen(&format!("POST {B}/frontend/uploads"))[0].key.clone();
    assert!(first_upload.is_some());

    // Re-running without --no-activate resumes the journal: no new upload or version.
    h.deploy_frontend(site.path(), &["--yes"]).await.unwrap();
    assert_eq!(h.mock.seen(&format!("POST {B}/frontend/uploads")).len(), 1);
    assert_eq!(h.mock.seen(&format!("POST {B}/frontend/versions")).len(), 1);
    let activate = h.mock.seen(&format!("POST {B}/frontend/versions/ver_2/activate"));
    assert_eq!(activate.len(), 1);
    assert_eq!(activate[0].if_match.as_deref(), Some("\"g4\""));
}

#[tokio::test]
async fn an_interrupted_deploy_resumes_with_the_same_keys() {
    let site = fixture_site();
    // Order the complete replies: 503 first, then sealed.
    let h2 = Harness::new().await;
    let manifest = manifest_sha(site.path());
    let html = bundle::sha256_hex(HTML.as_bytes());
    let target = |sha: &str| json!({"sha256": sha, "url": format!("{}/r2/{sha}", h2.base), "method": "PUT", "headers": {}});
    // No slot at the start; the upload creates one (H2 ensureSlot), which the rerun reads.
    h2.script(&format!("GET {B}/frontend"), 404, not_found()).script(&format!("GET {B}/frontend"), 200, slot(1, "empty", None));
    h2.script(&format!("POST {B}/frontend/uploads"), 201, json!({"id": "upl_1", "state": "open", "missing_hashes": [html, manifest], "upload_urls": [target(&html), target(&manifest)]}));
    h2.script(&format!("GET {B}/frontend/uploads/upl_1"), 200, json!({"id": "upl_1", "state": "open", "missing_hashes": []}));
    h2.script(&format!("PUT /r2/{html}"), 200, json!({})).script(&format!("PUT /r2/{manifest}"), 200, json!({}));
    h2.script(&format!("POST {B}/frontend/uploads/upl_1/complete"), 503, json!({"code": "AXDEPLOY_VERIFIER_UNAVAILABLE", "retryable": true}))
        .script(&format!("POST {B}/frontend/uploads/upl_1/complete"), 200, json!({"id": "upl_1", "state": "sealed"}));
    h2.script(&format!("POST {B}/frontend/versions"), 202, json!({"version_id": "ver_2", "operation_id": "op_v", "url": URL}));
    h2.script(&format!("GET {B}/frontend/operations/op_v"), 200, op("op_v", "succeeded"));
    h2.script(&format!("GET {B}/frontend/versions/ver_2"), 200, json!({"id": "ver_2", "status": "ready"}));
    let e = h2.deploy_frontend(site.path(), &["--no-activate"]).await.unwrap_err().to_string();
    assert!(e.contains("AXDEPLOY_VERIFIER_UNAVAILABLE") && e.contains("Try again in a minute"), "{e}");
    h2.deploy_frontend(site.path(), &["--no-activate"]).await.unwrap();
    // The rerun read the upload instead of creating another, and completed with the same key.
    assert_eq!(h2.mock.seen(&format!("POST {B}/frontend/uploads")).len(), 1);
    assert_eq!(h2.mock.seen(&format!("GET {B}/frontend/uploads/upl_1")).len(), 1);
    let completes = h2.mock.seen(&format!("POST {B}/frontend/uploads/upl_1/complete"));
    assert_eq!(completes.len(), 2);
    assert_eq!(completes[0].key, completes[1].key);
    // One slot read per run (the pilot check); the resumed intent keeps its original generation.
    assert_eq!(h2.mock.seen(&format!("GET {B}/frontend")).len(), 2);
    let journal: Journal = serde_json::from_slice(&std::fs::read(h2.ctx.journal_dir.join(format!("demo-{ENV}-frontend.json"))).unwrap()).unwrap();
    assert_eq!(journal.generation, 0);
    // A different build starts a fresh intent.
    std::fs::write(site.path().join("index.html"), "<h1>changed</h1>").unwrap();
    let _ = h2.deploy_frontend(site.path(), &["--no-activate"]).await;
    let uploads = h2.mock.seen(&format!("POST {B}/frontend/uploads"));
    assert_eq!(uploads.len(), 2);
    assert_ne!(uploads[0].body["manifest_sha256"], uploads[1].body["manifest_sha256"]);
}

#[tokio::test]
async fn missing_upload_urls_and_server_errors_show_the_next_step() {
    let h = Harness::new().await;
    let site = fixture_site();
    let html = bundle::sha256_hex(HTML.as_bytes());
    h.script(&format!("GET {B}/frontend"), 404, not_found());
    h.script(&format!("POST {B}/frontend/uploads"), 201, json!({"id": "upl_1", "state": "open", "missing_hashes": [html]}));
    h.script(&format!("GET {B}/frontend/uploads/upl_1"), 200, json!({"id": "upl_1", "state": "open", "missing_hashes": [html]}));
    let e = h.deploy_frontend(site.path(), &["--yes"]).await.unwrap_err().to_string();
    assert!(e.contains("AXDEPLOY_UPLOAD_URLS_UNAVAILABLE") && e.contains("hosting uploads are not enabled"), "{e}");
    assert_eq!(h.mock.seen(&format!("GET {B}/frontend/uploads/upl_1")).len(), 1);

    let h = Harness::new().await;
    h.script(&format!("GET {B}/frontend"), 503, json!({"schema": SCHEMA, "code": "AXDEPLOY_DISABLED", "request_id": "req_9", "retryable": false}));
    let e = h.deploy_frontend(site.path(), &["--yes"]).await.unwrap_err().to_string();
    assert_eq!(e, "AXDEPLOY_DISABLED: This project is not in the Axiom hosting pilot yet (deployments are invite-only). Ask the Axiom team to add it. (request req_9)");
    h.script(&format!("GET {B}/backend"), 404, not_found());
    let e = h.deployments(&["status", "backend"]).await.unwrap_err().to_string();
    assert_eq!(e, "There is no backend deployment in this environment yet. Deploy one with `axiom deploy backend`.");
}

/// A4.4: a code the CLI does not special-case prints the server's message with the code, for
/// a sync error and for an operation's failure.message (the migrate hint reaches the user).
#[tokio::test]
async fn unknown_codes_print_the_server_message_for_sync_errors_and_operation_failures() {
    const HINT: &str = "This backend was built for a different database schema than the environment's applied database head. Run `axiom database migrate` for this environment first, then deploy again.";
    let input = backend_input();
    let b = bundle::backend(input.path()).unwrap();
    let cli = BackendCli::try_parse_from(argv(&["--from-input", input.path().to_str().unwrap(), "--project", "demo", "--environment", ENV, "--yes", "--json"])).unwrap();
    // The sync 409 of version-create.
    let h = Harness::new().await;
    h.script(&format!("GET {B}/backend"), 404, not_found());
    h.script(&format!("POST {B}/backend/uploads"), 201, json!({"id": "upl_b", "state": "sealed", "missing_hashes": []}));
    h.script(&format!("POST {B}/backend/versions"), 409, json!({"schema": SCHEMA, "code": "AXDEPLOY_INCOMPATIBLE_SCHEMA", "message": HINT, "request_id": "req_7", "retryable": false, "details": {}}));
    let e = deploy(&h.ctx, Kind::Backend, &b, &cli.args.flags).await.unwrap_err().to_string();
    assert_eq!(e, format!("AXDEPLOY_INCOMPATIBLE_SCHEMA: {HINT} (request req_7)"));
    // The same hint as the stored failure of the seal/prepare operation.
    let h = Harness::new().await;
    h.script(&format!("GET {B}/backend"), 404, not_found());
    h.script(&format!("POST {B}/backend/uploads"), 201, json!({"id": "upl_b", "state": "sealed", "missing_hashes": []}));
    h.script(&format!("POST {B}/backend/versions"), 202, json!({"version_id": "ver_b", "operation_id": "op_s"}));
    h.script(&format!("GET {B}/backend/operations/op_s"), 200, json!({"operation_id": "op_s", "state": "failed", "failure": {"code": "AXDEPLOY_INCOMPATIBLE_SCHEMA", "message": HINT, "retryable": false}}));
    let e = deploy(&h.ctx, Kind::Backend, &b, &cli.args.flags).await.unwrap_err().to_string();
    assert!(e.contains(&format!("Operation op_s failed: AXDEPLOY_INCOMPATIBLE_SCHEMA: {HINT}")) && e.contains("axiom database migrate"), "{e}");
}

/// A package input folder as `axiom server package --output-input` writes it, plus a stray file.
fn backend_input() -> tempfile::TempDir {
    let input = tempfile::tempdir().unwrap();
    let files = [("contract.axiom", "c"), ("contract.server.json", "{}"), ("deployment.json", "{}")];
    let mut listed = serde_json::Map::new();
    for (name, body) in files {
        std::fs::write(input.path().join(name), body).unwrap();
        listed.insert(name.into(), json!({"bytes": body.len(), "executable": false, "sha256": bundle::sha256_hex(body.as_bytes())}));
    }
    std::fs::write(input.path().join("stray.txt"), "not listed").unwrap();
    let manifest = json!({"artifactReferences": {}, "files": listed, "format": "axiom-container-input/v1", "target": {"id": "container-amd64"}, "targetSha256": "a".repeat(64)});
    std::fs::write(input.path().join("input.json"), manifest.to_string()).unwrap();
    input
}

#[tokio::test]
async fn a_failed_version_ends_the_intent_with_its_reason() {
    let site = fixture_site();
    let h = Harness::new().await;
    h.script(&format!("GET {B}/frontend"), 404, not_found());
    let manifest = manifest_sha(site.path());
    h.script(&format!("POST {B}/frontend/uploads"), 201, json!({"id": "upl_1", "state": "sealed", "missing_hashes": []}));
    h.script(&format!("POST {B}/frontend/versions"), 202, json!({"version_id": "ver_2", "operation_id": "op_v"}));
    h.script(&format!("GET {B}/frontend/operations/op_v"), 200, json!({"operation_id": "op_v", "state": "failed", "failure": {"code": "AXDEPLOY_DIGEST_MISMATCH", "message": "index.html"}}));
    let e = h.deploy_frontend(site.path(), &["--yes"]).await.unwrap_err().to_string();
    assert!(e.contains("Operation op_v failed: AXDEPLOY_DIGEST_MISMATCH: index.html. Uploaded files did not match the manifest."), "{e}");
    let journal: Journal = serde_json::from_slice(&std::fs::read(h.ctx.journal_dir.join(format!("demo-{ENV}-frontend.json"))).unwrap()).unwrap();
    assert_eq!(journal.manifest_sha256, manifest);
    assert!(journal.failed && !journal.activated);
}

#[tokio::test]
async fn deploy_backend_from_input_uploads_container_input() {
    let h = Harness::new().await;
    let input = backend_input();
    let b = bundle::backend(input.path()).unwrap();
    let file = bundle::sha256_hex(b"c");
    let empty_json = bundle::sha256_hex(b"{}");
    let api = "https://demo-development-a1b2c3-api.axiomcore.page";
    let target = |sha: &str| json!({"sha256": sha, "url": format!("{}/r2/{sha}", h.base), "method": "PUT", "headers": {}});
    h.script(&format!("GET {B}/backend"), 404, not_found()).script(&format!("GET {B}/backend"), 200, json!({"generation": 1, "state": "empty", "url": api})).script(&format!("GET {B}/backend"), 200, json!({"generation": 2, "state": "active", "active_version_id": "ver_b", "url": api}));
    h.script(&format!("POST {B}/backend/uploads"), 201, json!({"id": "upl_b", "state": "open", "missing_hashes": [file, b.manifest_sha256], "upload_urls": [target(&file), target(&b.manifest_sha256), target(&empty_json)]}));
    h.script(&format!("PUT /r2/{file}"), 200, json!({})).script(&format!("PUT /r2/{}", b.manifest_sha256), 200, json!({}));
    h.script(&format!("POST {B}/backend/uploads/upl_b/complete"), 200, json!({"id": "upl_b", "state": "sealed"}));
    h.script(&format!("POST {B}/backend/versions"), 202, json!({"version_id": "ver_b", "operation_id": "op_s"}));
    h.script(&format!("GET {B}/backend/operations/op_s"), 200, op("op_s", "running")).script(&format!("GET {B}/backend/operations/op_s"), 200, op("op_s", "succeeded"));
    h.script(&format!("GET {B}/backend/versions/ver_b"), 200, json!({"id": "ver_b", "status": "ready"}));
    h.script(&format!("POST {B}/backend/versions/ver_b/activate"), 202, json!({"operation_id": "op_ba"}));
    h.script(&format!("GET {B}/backend/operations/op_ba"), 200, op("op_ba", "succeeded"));
    let cli = BackendCli::try_parse_from(argv(&["--from-input", input.path().to_str().unwrap(), "--project", "demo", "--environment", ENV, "--yes", "--json"])).unwrap();
    let prepared = backend_bundle(&cli.args, &h.ctx.cwd).unwrap();
    deploy(&h.ctx, Kind::Backend, &prepared, &cli.args.flags).await.unwrap();
    let up = &h.mock.seen(&format!("POST {B}/backend/uploads"))[0];
    // A4.1/A4.2: input.json is the manifest (digest and exact length); the content files are
    // the three it lists (one entry each, the stray file left out), counted without it.
    let input_json = std::fs::read(input.path().join("input.json")).unwrap();
    assert_eq!(b.manifest, input_json);
    assert_eq!(
        up.body,
        json!({"manifest_sha256": bundle::sha256_hex(&input_json), "manifest_bytes": input_json.len(), "total_bytes": 5, "file_count": 3, "purpose": "container_input",
            "files": [{"sha256": file, "size": 1}, {"sha256": empty_json, "size": 2}, {"sha256": empty_json, "size": 2}]})
    );
    // Only what the server listed is uploaded: the manifest (byte for byte) and contract.axiom;
    // the `{}` object it did not list is not sent although it has a URL.
    assert_eq!(h.mock.seen(&format!("PUT /r2/{}", b.manifest_sha256))[0].raw, input_json);
    assert!(h.mock.seen(&format!("PUT /r2/{empty_json}")).is_empty());
    assert_eq!(h.mock.seen(&format!("POST {B}/backend/versions"))[0].body, json!({"artifact": {"type": "container_input", "upload_id": "upl_b", "target": "container-amd64"}}));
    assert_eq!(h.mock.seen(&format!("POST {B}/backend/versions/ver_b/activate"))[0].if_match.as_deref(), Some("\"g1\""));
    assert_eq!(h.mock.seen(&format!("GET {B}/backend/operations/op_s")).len(), 2);
    // Without --from-input and nothing to package, the error names the fixes.
    let empty = tempfile::tempdir().unwrap();
    let cli = BackendCli::try_parse_from(argv(&[])).unwrap();
    let e = backend_bundle(&cli.args, empty.path()).err().unwrap().to_string();
    assert!(e.contains("No backend contract found") && e.contains("--from-input"), "{e}");
    // Every package-input hint is a complete command (S3).
    assert!(e.contains("axiom server package <contract> <plan> --deployment <file> --target container-amd64 --output-input <dir>") && e.contains("axiom deploy backend --from-input <dir>"), "{e}");
    std::fs::write(empty.path().join("backend.axiom"), "c").unwrap();
    let e = backend_bundle(&cli.args, empty.path()).err().unwrap().to_string();
    assert!(e.contains("No server plan at") && e.contains("--target container-amd64 --output-input <dir>"), "{e}");
    std::fs::write(empty.path().join("backend.server.json"), "{}").unwrap();
    let e = backend_bundle(&cli.args, empty.path()).err().unwrap().to_string();
    assert!(e.contains("axiom server deployment ") && e.contains("--output deployment.json") && e.contains("--output-input <dir>"), "{e}");
}

/// list, status, versions, activate, rollback (previous and --to), logs, delete, health.
#[tokio::test]
async fn every_deployments_command_sends_the_contract_requests() {
    let h = Harness::new().await;
    h.script(&format!("GET {B}"), 200, json!({"items": [slot(3, "active", Some("ver_2"))], "next_cursor": null}));
    h.script(&format!("GET {B}/frontend"), 200, json!({"generation": 3, "state": "active", "active_version_id": "ver_2", "previous_version_id": "ver_1", "operation_id": "op_last", "url": URL}));
    h.script(&format!("GET {B}/frontend/operations/op_last"), 200, json!({"operation_id": "op_last", "action": "activate", "state": "failed", "failure": {"code": "AXDEPLOY_NOT_READY"}}));
    h.script(&format!("GET {B}/frontend/versions"), 200, json!({"items": [{"id": "ver_2", "status": "ready", "created_at": "t2"}, {"id": "ver_1", "status": "ready", "created_at": "t1"}], "next_cursor": null}));
    h.script(&format!("POST {B}/frontend/versions/ver_1/activate"), 202, json!({"operation_id": "op_1", "state": "activating"}));
    h.script(&format!("GET {B}/frontend/operations/op_1"), 200, op("op_1", "succeeded"));
    h.script(&format!("POST {B}/frontend/rollback"), 202, json!({"operation_id": "op_r", "state": "activating"}));
    h.script(&format!("GET {B}/frontend/operations/op_r"), 200, op("op_r", "succeeded"));
    h.script(&format!("GET {B}/frontend/logs"), 200, json!({"items": [{"id": "e1", "action": "activate", "step": "done", "version_id": "ver_2", "created_at": "t", "result": {"state": "succeeded"}}], "next_cursor": null}));
    h.script(&format!("DELETE {B}/frontend"), 202, json!({"operation_id": "op_d", "state": "deleting"}));
    h.script(&format!("GET {B}/frontend/health"), 200, json!({"deployment_id": "dpl_1", "health": "healthy"}));
    for args in [
        vec!["list"],
        vec!["status", "frontend"],
        vec!["show", "frontend", "--json"],
        vec!["versions", "frontend"],
        vec!["activate", "frontend", "ver_1", "--yes", "--wait"],
        vec!["rollback", "frontend", "--yes"],
        vec!["rollback", "frontend", "--to", "ver_1", "--yes", "--wait"],
        vec!["logs", "frontend", "--since", "2026-11-16T09:00:00+01:00", "--version", "ver_2"],
        vec!["logs", "frontend", "--stream", "runtime"],
        vec!["delete", "frontend", "--yes"],
        vec!["health", "frontend"],
    ] {
        h.deployments(&args).await.unwrap_or_else(|e| panic!("{args:?}: {e}"));
    }
    type Row = (String, Option<String>, Value, Option<String>, bool);
    let seen: Vec<Row> =
        h.deployment_requests().into_iter().map(|s| (s.route, s.query, s.body, s.if_match, s.key.is_some())).collect();
    let get = |path: &str| (format!("GET {B}{path}"), None, Value::Null, None, false);
    let want = vec![
        get(""),
        get("/frontend"),
        get("/frontend/operations/op_last"),
        get("/frontend"),
        get("/frontend/operations/op_last"),
        get("/frontend/versions"),
        get("/frontend"),
        // activate --wait
        get("/frontend"),
        (format!("POST {B}/frontend/versions/ver_1/activate"), None, json!({}), Some("\"g3\"".into()), true),
        get("/frontend/operations/op_1"),
        get("/frontend"),
        // rollback (previous)
        get("/frontend"),
        (format!("POST {B}/frontend/rollback"), None, json!({}), Some("\"g3\"".into()), true),
        // rollback --to --wait
        get("/frontend"),
        (format!("POST {B}/frontend/rollback"), None, json!({"to_version_id": "ver_1"}), Some("\"g3\"".into()), true),
        get("/frontend/operations/op_r"),
        get("/frontend"),
        // logs
        (format!("GET {B}/frontend/logs"), Some("stream=lifecycle&since=2026-11-16T08%3A00%3A00Z&version=ver_2".into()), Value::Null, None, false),
        (format!("GET {B}/frontend/logs"), Some("stream=runtime".into()), Value::Null, None, false),
        // delete (no body)
        get("/frontend"),
        (format!("DELETE {B}/frontend"), None, Value::Null, Some("\"g3\"".into()), true),
        get("/frontend/health"),
    ];
    assert_eq!(seen, want);
    let delete = &h.mock.seen(&format!("DELETE {B}/frontend"))[0];
    assert!(delete.raw.is_empty());
    // Standalone traffic keys are fresh per invocation: the same request on the same
    // generation (e.g. a re-created slot) never replays an earlier answer.
    let rollbacks = h.mock.seen(&format!("POST {B}/frontend/rollback"));
    assert_ne!(rollbacks[0].key, rollbacks[1].key);
    h.deployments(&["rollback", "frontend", "--yes"]).await.unwrap();
    h.deployments(&["delete", "frontend", "--yes"]).await.unwrap();
    let again = h.mock.seen(&format!("POST {B}/frontend/rollback"));
    assert_eq!(again[2].if_match, again[0].if_match);
    assert_ne!(again[2].key, again[0].key);
    let deletes = h.mock.seen(&format!("DELETE {B}/frontend"));
    assert_eq!(deletes[1].if_match, deletes[0].if_match);
    assert_ne!(deletes[1].key, deletes[0].key);
}

#[tokio::test]
async fn deploy_activation_keeps_its_original_generation_and_stops_on_412() {
    let h = Harness::new().await;
    let site = fixture_site();
    // The slot is at g5 when the deploy starts; the activation serves ver_2 at g6; afterwards
    // someone else's change moves it to g7 and serves ver_0. The mock does not check If-Match:
    // the first activation is scripted 202, the second 412.
    // Reads: deploy start, before activating, after activating; then the rerun's reads.
    h.script(&format!("GET {B}/frontend"), 200, slot(5, "active", Some("ver_1")));
    h.script(&format!("GET {B}/frontend"), 200, slot(5, "active", Some("ver_1")));
    h.script(&format!("GET {B}/frontend"), 200, slot(6, "active", Some("ver_2")));
    h.script(&format!("GET {B}/frontend"), 200, slot(7, "active", Some("ver_0")));
    script_deploy(&h, site.path(), false);
    h.script(&format!("POST {B}/frontend/versions/ver_2/activate"), 412, json!({"schema": SCHEMA, "code": "AXDEPLOY_PRECONDITION_FAILED", "request_id": "r9"}));
    h.deploy_frontend(site.path(), &["--yes"]).await.unwrap();
    let activations = h.mock.seen(&format!("POST {B}/frontend/versions/ver_2/activate"));
    assert_eq!(activations[0].if_match.as_deref(), Some("\"g5\""), "the generation read when the deploy started, not the latest");
    let journal: Journal = serde_json::from_slice(&std::fs::read(h.ctx.journal_dir.join(format!("demo-{ENV}-frontend.json"))).unwrap()).unwrap();
    assert_eq!(activations[0].key.as_deref(), Some(idempotency_key(&journal.intent_id, "activate", &journal.manifest_sha256, 5).as_str()));
    // The server now serves another version: a rerun of the same build does not claim it is
    // live; it re-activates under the original precondition and the 412 says what to do.
    let e = h.deploy_frontend(site.path(), &["--yes"]).await.unwrap_err().to_string();
    assert!(e.contains("changed while you were deploying") && e.contains("axiom deployments activate frontend ver_2"), "{e}");
    let journal: Journal = serde_json::from_slice(&std::fs::read(h.ctx.journal_dir.join(format!("demo-{ENV}-frontend.json"))).unwrap()).unwrap();
    assert!(!journal.failed && journal.version_id.as_deref() == Some("ver_2"), "the journal keeps the version for a manual activate");
    let activations = h.mock.seen(&format!("POST {B}/frontend/versions/ver_2/activate"));
    assert_eq!(activations[1].if_match.as_deref(), Some("\"g5\""));
    // N1: the re-activation sends a fresh key, so a real server checks the precondition
    // instead of replaying the first activation's stored success.
    assert_ne!(activations[1].key, activations[0].key);
}

#[tokio::test]
async fn environment_names_resolve_through_configuration_environments() {
    let h = Harness::new().await;
    h.script("GET /api/v1/projects/demo/configuration-environments", 200, json!([{"id": ENV, "slug": "development"}]));
    h.script(&format!("GET {B}"), 200, json!({"items": []}));
    let cli = DeploymentsCli::try_parse_from(argv(&["list", "--project", "Demo"])).unwrap();
    run(&h.ctx, &cli.args).await.unwrap();
    assert_eq!(h.mock.seen("GET /api/v1/projects/demo/configuration-environments").len(), 1);
    assert_eq!(h.mock.seen(&format!("GET {B}")).len(), 1);
    assert_eq!(list_lines(&[], "development"), vec!["No deployments in development yet. Deploy with `axiom deploy frontend` or `axiom deploy backend`."]);
}

#[test]
fn list_and_status_render_tables_and_failures() {
    let s: Slot = serde_json::from_value(slot(3, "active", Some("ver_2"))).unwrap();
    let lines = list_lines(std::slice::from_ref(&s), "development");
    assert_eq!(lines[0], format!("KIND      STATE   ACTIVE VERSION  {:<w$}  UPDATED", "URL", w = URL.len()));
    assert_eq!(lines[1], format!("frontend  active  ver_2           {URL}  2026-11-16T10:00:00Z"));
    let o: Operation = serde_json::from_value(json!({"operation_id": "op_1", "action": "activate", "state": "failed", "failure": {"code": "AXDEPLOY_NOT_READY", "message": "probe"}})).unwrap();
    let text = status_lines(Kind::Frontend, &s, Some(&o)).join("\n");
    assert!(text.contains(&format!("URL:       {URL}")) && text.contains("Last operation: op_1 activate failed"), "{text}");
    assert!(text.contains("Failure:   AXDEPLOY_NOT_READY: probe. That version is not ready. Check it with `axiom deployments versions`."), "{text}");
}

#[test]
fn since_and_until_take_durations_or_rfc3339() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-11-16T12:00:00Z").unwrap().with_timezone(&chrono::Utc);
    assert_eq!(parse_time_at("10m", "--since", now).unwrap(), "2026-11-16T11:50:00Z");
    assert_eq!(parse_time_at("1h", "--since", now).unwrap(), "2026-11-16T11:00:00Z");
    assert_eq!(parse_time_at("2d", "--since", now).unwrap(), "2026-11-14T12:00:00Z");
    assert_eq!(parse_time_at("30s", "--since", now).unwrap(), "2026-11-16T11:59:30Z");
    assert_eq!(parse_time_at("2026-11-16T09:00:00+01:00", "--until", now).unwrap(), "2026-11-16T08:00:00Z");
    for bad in ["10", "m", "10w", "yesterday", "-5m", "99999999999999d"] {
        assert!(parse_time_at(bad, "--since", now).unwrap_err().to_string().starts_with("--since takes"), "{bad}");
    }
}

#[test]
fn confirmation_covers_traffic_changes_only() {
    let parse = |a: &[&str]| DeploymentsCli::try_parse_from(argv(a)).unwrap().args;
    assert_eq!(asks_first(&parse(&["activate", "frontend", "ver_1"])), Some(false));
    assert_eq!(asks_first(&parse(&["rollback", "backend", "--yes"])), Some(true));
    assert_eq!(asks_first(&parse(&["delete", "frontend"])), Some(false));
    for read in [vec!["list"], vec!["status", "frontend"], vec!["versions", "frontend"], vec!["logs", "frontend"], vec!["health", "backend"]] {
        assert_eq!(asks_first(&parse(&read)), None, "{read:?}");
    }
    assert_eq!(action_question(&parse(&["--project", "demo", "--environment", "staging", "delete", "frontend"])).unwrap(), "Delete the frontend deployment of project demo (staging)? Its URL stops serving.");
    assert_eq!(action_question(&parse(&["rollback", "backend"])).unwrap(), "Roll the backend of the linked project (development) back to the previous version?");
    let t = parse(&["--project", "demo", "list"]).target;
    assert_eq!(deploy_question(Kind::Frontend, Some(Path::new("./dist")), &t), "Deploy ./dist as the frontend of project demo (development) and make it live?");
    // The D2 §8 live-checklist invocations parse.
    for line in [
        vec!["list", "--project", "p", "--environment", "e", "--json"],
        vec!["activate", "frontend", "ver_1", "--project", "p", "--environment", "e", "--yes", "--wait"],
        vec!["logs", "frontend", "--project", "p", "--environment", "e", "--since", "10m"],
        vec!["rollback", "frontend", "--to", "ver_0", "--project", "p", "--environment", "e", "--yes", "--wait"],
        vec!["delete", "frontend", "--project", "p", "--environment", "e", "--yes", "--wait"],
    ] {
        DeploymentsCli::try_parse_from(argv(&line)).unwrap_or_else(|e| panic!("{line:?}: {e}"));
    }
    let f = FrontendCli::try_parse_from(argv(&["./fixture-dist", "--project", "p", "--environment", "e", "--no-activate", "--wait", "--json"])).unwrap();
    assert!(f.args.flags.no_activate && f.args.flags.target.json);
    assert!(FrontendCli::try_parse_from(argv(&["--spa", "--no-spa"])).is_err());
}

/// The scripted session the dashboard must reproduce: deploy the fixture site → activate the
/// previous version → roll back → delete. Ids are normalised to placeholders.
#[tokio::test]
async fn parity_session_matches_the_recorded_fixture() {
    let h = Harness::new().await;
    let site = fixture_site();
    let g = |n: i64, active: &str| slot(n, "active", Some(active));
    for (status, body) in [(404, not_found()), (200, slot(1, "empty", None)), (200, g(2, "ver_2")), (200, g(2, "ver_2")), (200, g(3, "ver_1")), (200, g(3, "ver_1")), (200, g(4, "ver_2")), (200, g(4, "ver_2")), (404, not_found())] {
        h.script(&format!("GET {B}/frontend"), status, body);
    }
    script_deploy(&h, site.path(), false);
    h.script(&format!("POST {B}/frontend/versions/ver_1/activate"), 202, json!({"operation_id": "op_b", "state": "activating"}));
    h.script(&format!("GET {B}/frontend/operations/op_b"), 200, op("op_b", "succeeded"));
    h.script(&format!("POST {B}/frontend/rollback"), 202, json!({"operation_id": "op_r", "state": "activating"}));
    h.script(&format!("GET {B}/frontend/operations/op_r"), 200, op("op_r", "succeeded"));
    h.script(&format!("DELETE {B}/frontend"), 202, json!({"operation_id": "op_d", "state": "deleting"}));
    h.script(&format!("GET {B}/frontend/operations/op_d"), 200, op("op_d", "succeeded"));

    h.deploy_frontend(site.path(), &["--yes", "--json"]).await.unwrap();
    h.deployments(&["activate", "frontend", "ver_1", "--yes", "--wait", "--json"]).await.unwrap();
    h.deployments(&["rollback", "frontend", "--yes", "--wait", "--json"]).await.unwrap();
    h.deployments(&["delete", "frontend", "--yes", "--wait", "--json"]).await.unwrap();

    let mut records: Vec<Entry> = h.deployment_requests().iter().map(record).collect();
    // Concurrent PUTs: order each run by object hash.
    let mut i = 0;
    while i < records.len() {
        let mut j = i;
        while j < records.len() && records[j].action == "put_object" {
            j += 1;
        }
        records[i..j].sort_by(|a, b| a.object_sha256.cmp(&b.object_sha256));
        i = j.max(i + 1);
    }
    let text = serde_json::to_string_pretty(&records).unwrap() + "\n";
    let text = text.replace("upl_1", "<upload_id>").replace("ver_2", "<version_id>").replace("ver_1", "<previous_version_id>");
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/deployments-parity.json");
    if std::env::var_os("AXIOM_UPDATE_FIXTURES").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    let recorded = std::fs::read_to_string(&path).expect("tests/fixtures/deployments-parity.json (regenerate with AXIOM_UPDATE_FIXTURES=1)");
    assert_eq!(recorded, text, "the parity fixture is stale; regenerate it with AXIOM_UPDATE_FIXTURES=1 and update the dashboard");
}

/// Field order of each fixture entry.
#[derive(Serialize)]
struct Entry {
    action: &'static str,
    method: String,
    path: String,
    body: Value,
    if_match: Option<String>,
    idempotency_key: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    object_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<usize>,
    /// The canonical manifest, when this PUT uploads it.
    #[serde(skip_serializing_if = "Option::is_none")]
    manifest: Option<String>,
}

fn record(s: &Seen) -> Entry {
    let (method, path) = s.route.split_once(' ').unwrap();
    let rest = path.strip_prefix(B).unwrap_or(path);
    let segments: Vec<&str> = rest.trim_start_matches('/').split('/').collect();
    let action = match (method, segments.as_slice()) {
        ("PUT", _) => "put_object",
        ("GET", [_kind]) => "slot",
        ("DELETE", [_kind]) => "delete",
        ("POST", [_, "uploads"]) => "create_upload",
        ("POST", [_, "uploads", _, "complete"]) => "complete_upload",
        ("POST", [_, "versions"]) => "create_version",
        ("GET", [_, "versions", _]) => "version",
        ("POST", [_, "versions", _, "activate"]) => "activate",
        ("POST", [_, "rollback"]) => "rollback",
        ("GET", [_, "operations", _]) => "operation",
        other => panic!("unrecorded request {other:?}"),
    };
    let put = method == "PUT";
    let text = String::from_utf8(s.raw.clone()).ok().filter(|t| t.starts_with("{\"schema\":"));
    let mut path = path.to_string();
    for op in ["op_v", "op_a", "op_b", "op_r", "op_d"] {
        path = path.replace(&format!("/{op}"), "/<operation_id>");
    }
    Entry {
        action,
        method: method.into(),
        path: if put { "<upload_urls[sha256].url>".into() } else { path },
        body: if put { Value::Null } else { s.body.clone() },
        if_match: s.if_match.clone(),
        idempotency_key: s.key.is_some(),
        object_sha256: put.then(|| bundle::sha256_hex(&s.raw)),
        bytes: put.then_some(s.raw.len()),
        manifest: if put { text } else { None },
    }
}

#[test]
fn logs_print_oldest_first() {
    let ev = |id: &str, at: &str| axiom_cloud::deployments::LogEvent { id: Some(id.into()), created_at: Some(at.into()), ..Default::default() };
    // The server pages newest first.
    let mut items = vec![ev("e3", "2026-11-16T10:00:02Z"), ev("e2b", "2026-11-16T10:00:01Z"), ev("e2a", "2026-11-16T10:00:01Z"), ev("e1", "2026-11-16T10:00:00Z")];
    oldest_first(&mut items);
    let ids: Vec<_> = items.iter().map(|e| e.id.clone().unwrap()).collect();
    assert_eq!(ids, ["e1", "e2a", "e2b", "e3"]);
}

/// H2's reply to `GET …/{kind}` after `axiom deployments delete`: the newest deleted slot
/// (`findSlot` with live=false), its generation bumped by the delete, no URL.
fn deleted_slot(generation: i64) -> Value {
    json!({"schema": SCHEMA, "deployment_id": "dpl_old", "kind": "frontend", "generation": generation, "state": "deleted", "suspend_reason": null,
        "active_version_id": "ver_1", "desired_version_id": "ver_1", "previous_version_id": null, "version": null, "operation_id": "op_d", "status_url": format!("{B}/frontend/operations/op_d"),
        "url": null, "placement": {"policy": "apac-hint", "observed": null}, "health": {}, "quota": null, "failure": null, "created_at": "2026-11-16T09:00:00Z", "updated_at": "2026-11-16T10:00:00Z"})
}

/// H2's shared slot shape for the slot `ensureSlot` creates (generation 1, state empty).
fn new_slot(generation: i64, state: &str, active: Option<&str>) -> Value {
    json!({"schema": SCHEMA, "deployment_id": "dpl_new", "kind": "frontend", "generation": generation, "state": state, "suspend_reason": null,
        "active_version_id": active, "desired_version_id": active, "previous_version_id": null, "version": null, "operation_id": null, "status_url": null,
        "url": "https://demo-development-d4e5f6.axiomcore.page", "placement": {"policy": "apac-hint", "observed": null}, "health": {}, "quota": null, "failure": null,
        "created_at": "2026-11-16T11:00:00Z", "updated_at": "2026-11-16T11:00:00Z"})
}

/// Scripts the upload, version and activation of a deploy onto the slot H2 creates after a
/// delete; activate accepts only `If-Match: "g1"` (the new slot's generation), like H2.
fn script_redeploy(h: &Harness, site: &Path) {
    let manifest = manifest_sha(site);
    let html = bundle::sha256_hex(HTML.as_bytes());
    let app = bundle::sha256_hex(APP.as_bytes());
    let target = |sha: &str| json!({"sha256": sha, "url": format!("{}/r2/{sha}", h.base), "method": "PUT", "headers": {"Content-Length": "11", "x-amz-checksum-sha256": "c2hh"}, "expires_at": "2026-11-16T11:15:00Z"});
    h.script(&format!("POST {B}/frontend/uploads"), 201, json!({"id": "upl_9", "state": "open", "purpose": "static_bundle", "expires_at": "2026-11-16T12:00:00Z", "file_count": 2, "total_bytes": 25,
        "missing_hashes": [app, html, manifest], "upload_urls": [target(&app), target(&html), target(&manifest)]}));
    for sha in [&app, &html, &manifest] {
        h.script(&format!("PUT /r2/{sha}"), 200, json!({}));
    }
    h.script(&format!("POST {B}/frontend/uploads/upl_9/complete"), 200, json!({"id": "upl_9", "state": "sealed", "purpose": "static_bundle", "missing_hashes": []}));
    let mut created = new_slot(1, "empty", None);
    created["version_id"] = json!("ver_9");
    created["operation_id"] = json!("op_v9");
    created["version"] = json!({"id": "ver_9", "status": "pending", "artifact": {"type": "static_bundle"}, "failure": null});
    h.script(&format!("POST {B}/frontend/versions"), 202, created);
    h.script(&format!("GET {B}/frontend/operations/op_v9"), 200, op("op_v9", "succeeded"));
    h.script(&format!("GET {B}/frontend/versions/ver_9"), 200, json!({"id": "ver_9", "deployment_id": "dpl_new", "status": "ready", "manifest_sha256": manifest}));
    h.mock.require_if_match(&format!("POST {B}/frontend/versions/ver_9/activate"), "\"g1\"");
    let mut activating = new_slot(2, "activating", None);
    activating["desired_version_id"] = json!("ver_9");
    activating["version_id"] = json!("ver_9");
    activating["operation_id"] = json!("op_a9");
    h.script(&format!("POST {B}/frontend/versions/ver_9/activate"), 202, activating);
    h.script(&format!("GET {B}/frontend/operations/op_a9"), 200, op("op_a9", "succeeded"));
}

/// M1: after `axiom deployments delete`, `GET …/frontend` answers the deleted slot (g3). The
/// upload creates a new live slot at g1, so the activation pins g1, not the deleted slot's g3.
#[tokio::test]
async fn a_deploy_after_a_delete_activates_the_new_slot_at_generation_1() {
    let h = Harness::new().await;
    let site = fixture_site();
    h.script(&format!("GET {B}/frontend"), 200, deleted_slot(3)) // deploy start
        .script(&format!("GET {B}/frontend"), 200, new_slot(1, "empty", None)) // activate reads the slot
        .script(&format!("GET {B}/frontend"), 200, new_slot(2, "active", Some("ver_9")));
    script_redeploy(&h, site.path());
    h.deploy_frontend(site.path(), &["--yes", "--json"]).await.unwrap();
    let activations = h.mock.seen(&format!("POST {B}/frontend/versions/ver_9/activate"));
    assert_eq!(activations.len(), 1);
    assert_eq!(activations[0].if_match.as_deref(), Some("\"g1\""));
    let journal: Journal = serde_json::from_slice(&std::fs::read(h.ctx.journal_dir.join(format!("demo-{ENV}-frontend.json"))).unwrap()).unwrap();
    assert_eq!((journal.generation, journal.slot_id.as_deref()), (0, None), "no live slot when the deploy started");
    assert_eq!(activations[0].key.as_deref(), Some(idempotency_key(&journal.intent_id, "activate", &journal.manifest_sha256, 1).as_str()));
    assert!(journal.activated && !journal.failed);
}

/// M1, resumed: a journal an earlier CLI wrote pinned the deleted slot's g3 (no `slot_id`) and
/// stopped at the 412. A plain rerun resumes that intent and activates with g1.
#[tokio::test]
async fn a_resumed_deploy_pinned_to_a_deleted_slot_recovers() {
    let h = Harness::new().await;
    let site = fixture_site();
    let manifest = manifest_sha(site.path());
    let stale = json!({"schema": JOURNAL_SCHEMA, "project": "demo", "environment_id": ENV, "kind": "frontend", "intent_id": "intent-old", "manifest_sha256": manifest,
        "generation": 3, "upload_id": "upl_9", "upload_sealed": true, "version_id": "ver_9", "version_operation_id": "op_v9", "version_ready": true,
        "activate_operation_id": null, "activated": false, "url": null, "failed": false, "steps": {"create_version": "empty"}});
    std::fs::create_dir_all(&h.ctx.journal_dir).unwrap();
    std::fs::write(h.ctx.journal_dir.join(format!("demo-{ENV}-frontend.json")), serde_json::to_vec(&stale).unwrap()).unwrap();
    h.script(&format!("GET {B}/frontend"), 200, new_slot(1, "empty", None)).script(&format!("GET {B}/frontend"), 200, new_slot(1, "empty", None)).script(&format!("GET {B}/frontend"), 200, new_slot(2, "active", Some("ver_9")));
    script_redeploy(&h, site.path());
    h.deploy_frontend(site.path(), &["--yes"]).await.unwrap();
    // Nothing re-uploaded or re-created: only the activation, with the new slot's generation.
    assert!(h.mock.seen(&format!("POST {B}/frontend/uploads")).is_empty());
    assert!(h.mock.seen(&format!("POST {B}/frontend/versions")).is_empty());
    let activations = h.mock.seen(&format!("POST {B}/frontend/versions/ver_9/activate"));
    assert_eq!(activations.len(), 1);
    assert_eq!(activations[0].if_match.as_deref(), Some("\"g1\""));
    assert_eq!(activations[0].key.as_deref(), Some(idempotency_key("intent-old", "activate", &manifest, 1).as_str()));

    // The same stale pin on a slot that has since changed (someone activated: g2) still
    // stops with the 412 and the manual next step, never a silent refresh.
    let h = Harness::new().await;
    std::fs::create_dir_all(&h.ctx.journal_dir).unwrap();
    std::fs::write(h.ctx.journal_dir.join(format!("demo-{ENV}-frontend.json")), serde_json::to_vec(&stale).unwrap()).unwrap();
    h.script(&format!("GET {B}/frontend"), 200, new_slot(2, "active", Some("ver_8")));
    script_redeploy(&h, site.path());
    h.mock.require_if_match(&format!("POST {B}/frontend/versions/ver_9/activate"), "\"g2\"");
    let e = h.deploy_frontend(site.path(), &["--yes"]).await.unwrap_err().to_string();
    assert!(e.contains("changed while you were deploying") && e.contains("axiom deployments activate frontend ver_9"), "{e}");
    assert_eq!(h.mock.seen(&format!("POST {B}/frontend/versions/ver_9/activate"))[0].if_match.as_deref(), Some("\"g1\""));
}

#[test]
fn the_activation_pin_is_the_start_generation_or_1_for_a_new_slot() {
    let at = |g: i64| Slot { generation: Some(g), ..Default::default() };
    assert_eq!(activation_generation(0, None, &at(1)), 1, "no live slot at the start");
    assert_eq!(activation_generation(0, None, &at(4)), 1, "changed since: the 412 tells the user");
    assert_eq!(activation_generation(5, Some("dpl_1"), &at(6)), 5, "never refreshed");
    assert_eq!(activation_generation(5, Some("dpl_1"), &at(1)), 5, "a recorded slot keeps its pin");
    assert_eq!(activation_generation(3, None, &at(1)), 1, "an old journal's pin from a deleted slot");
    assert_eq!(activation_generation(3, None, &at(3)), 3);
    assert_eq!(activation_generation(3, None, &at(7)), 3);
}

/// N1: re-activating a build that is no longer live (rolled back) must not replay the first
/// activation's stored success: each re-activation sends a fresh key, so the server checks
/// the original precondition (412) instead of answering the old 202.
#[test]
fn a_reactivation_uses_a_fresh_activation_key() {
    assert_eq!(activation_step(0), "activate");
    let keys: std::collections::BTreeSet<String> = (0..3).map(|n| idempotency_key("intent", &activation_step(n), "m", 2)).collect();
    assert_eq!(keys.len(), 3, "every re-activation has its own key");
    assert_eq!(idempotency_key("intent", &activation_step(0), "m", 2), idempotency_key("intent", "activate", "m", 2), "first activation keeps the key older journals used");
    let old: Journal = serde_json::from_value(json!({"intent_id": "i", "activated": true})).unwrap();
    assert_eq!(old.reactivations, 0, "older journals load without the field");
}

/// N1: a finished activation is reported live only when the slot serves this version.
#[test]
fn an_activation_is_live_only_if_the_slot_serves_the_version() {
    let at = |active: Option<&str>| Slot { active_version_id: active.map(Into::into), state: Some("active".into()), ..Default::default() };
    assert!(confirm_live(&at(Some("ver_2")), "ver_2", Kind::Frontend, "").is_ok());
    let e = confirm_live(&at(Some("ver_1")), "ver_2", Kind::Frontend, "").unwrap_err().to_string();
    assert!(e.contains("ver_2 did not become live") && e.contains("now serves ver_1") && e.contains("axiom deployments activate frontend ver_2"), "{e}");
    assert!(confirm_live(&at(None), "ver_2", Kind::Frontend, "").unwrap_err().to_string().contains("no version"));
    let e = confirm_live(&at(Some("ver_1")), "ver_2", Kind::Frontend, " --project demo").unwrap_err().to_string();
    assert!(e.contains("status frontend --project demo") && e.contains("activate frontend ver_2 --project demo"), "{e}");
}

/// A slot still being deleted: the deploy pins nothing from it, and H2's 409 says to wait.
#[tokio::test]
async fn a_deploy_while_the_slot_is_being_deleted_says_to_wait() {
    let h = Harness::new().await;
    let site = fixture_site();
    let mut deleting = deleted_slot(3);
    deleting["state"] = json!("deleting");
    h.script(&format!("GET {B}/frontend"), 200, deleting);
    h.script(&format!("POST {B}/frontend/uploads"), 409, json!({"schema": SCHEMA, "code": "AXDEPLOY_DELETING", "message": "Deployment request could not be completed", "request_id": "r5", "retryable": false, "details": {}}));
    let e = h.deploy_frontend(site.path(), &["--yes"]).await.unwrap_err().to_string();
    assert!(e.contains("AXDEPLOY_DELETING") && e.contains("Wait for the delete to finish"), "{e}");
    let journal: Journal = serde_json::from_slice(&std::fs::read(h.ctx.journal_dir.join(format!("demo-{ENV}-frontend.json"))).unwrap()).unwrap();
    assert_eq!(journal.generation, 0);
}

/// A project outside the hosting pilot hears so from the first deployments read: before the
/// files are hashed (a file removed after the folder check is never read) and before y/N.
#[tokio::test]
async fn a_project_outside_the_pilot_is_told_before_hashing_or_asking() {
    let h = Harness::new().await;
    let site = fixture_site();
    let cli = FrontendCli::try_parse_from(argv(&[site.path().to_str().unwrap(), "--project", "demo", "--environment", ENV])).unwrap();
    let pending = frontend_pending(&cli.args, &h.ctx.cwd).unwrap();
    std::fs::remove_file(site.path().join("assets/app.js")).unwrap();
    h.script(&format!("GET {B}/frontend"), 503, json!({"schema": SCHEMA, "code": "AXDEPLOY_DISABLED", "message": "Deployment request could not be completed", "request_id": "r3", "retryable": true, "details": {}}));
    let e = deploy_pending(&h.ctx, Kind::Frontend, pending, &cli.args.flags, Some("Deploy?"), |_| panic!("asked before the pilot check")).await.unwrap_err().to_string();
    assert!(e.starts_with("AXDEPLOY_DISABLED: This project is not in the Axiom hosting pilot yet"), "{e}");
    assert_eq!(h.routes(), [format!("GET {B}/frontend")]);
    h.script(&format!("GET {B}"), 503, json!({"schema": SCHEMA, "code": "AXDEPLOY_DISABLED", "request_id": "r4", "retryable": true}));
    let e = h.deployments(&["list"]).await.unwrap_err().to_string();
    assert!(e.contains("not in the Axiom hosting pilot yet"), "{e}");
}

/// On a terminal the question comes after the one read and the hashing; declining sends
/// nothing else and exits 2.
#[tokio::test]
async fn declining_the_question_sends_nothing() {
    let h = Harness::new().await;
    let site = fixture_site();
    let cli = FrontendCli::try_parse_from(argv(&[site.path().to_str().unwrap(), "--project", "demo", "--environment", ENV])).unwrap();
    let pending = frontend_pending(&cli.args, &h.ctx.cwd).unwrap();
    h.script(&format!("GET {B}/frontend"), 404, not_found());
    let asked = std::cell::Cell::new(false);
    let e = deploy_pending(&h.ctx, Kind::Frontend, pending, &cli.args.flags, Some("Deploy?"), |q| {
        asked.set(q == "Deploy?");
        false
    })
    .await
    .unwrap_err();
    assert!(asked.get());
    assert_eq!(e.downcast_ref::<crate::commands::auth::Exit>().map(|x| x.code), Some(2));
    assert_eq!(h.routes(), [format!("GET {B}/frontend")]);
    assert!(!h.ctx.journal_dir.exists(), "no journal for a declined deploy");
}

/// S2: an unreachable Cloud is named as such, with the reason and without OS error numbers.
#[tokio::test]
async fn an_unreachable_cloud_is_not_reported_as_signed_out() {
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);
    let client = CloudClient::for_loopback(&url, "access".into(), "refresh".into()).unwrap();
    let e = client.list_projects().await.unwrap_err();
    let text = projects_error(&e);
    assert!(text.starts_with("Could not reach Axiom Cloud (") && !text.contains("os error") && !text.contains("axiom login"), "{text}");
    let text = projects_error(&anyhow!("Failed to list projects: {{\"error\":\"unauthorized\"}}"));
    assert!(text.starts_with("Not signed in to Axiom Cloud") && text.contains("`axiom login`"), "{text}");
    let text = projects_error(&anyhow!("unable to authenticate (Error: Failed to send refresh token request)"));
    assert!(text.starts_with("Could not reach Axiom Cloud"), "{text}");
    let text = projects_error(&anyhow!("Failed to list projects: <html><body>session gateway</body></html>"));
    assert_eq!(text, "Axiom Cloud did not return your projects (an unexpected web page). Try again in a minute.");
    let text = projects_error(&anyhow!("Failed to list projects: upstream exploded"));
    assert_eq!(text, "Axiom Cloud did not return your projects (upstream exploded). Try again in a minute.");
}

/// Review fix: the same build again after `axiom deployments delete` starts a fresh intent
/// (new keys) instead of replaying the old activation onto the deleted slot.
#[tokio::test]
async fn the_same_build_after_a_delete_deploys_afresh() {
    let h = Harness::new().await;
    let site = fixture_site();
    // First deploy onto live slot dpl_new (g1), then it is deleted (GET answers dpl_old-style
    // deleted slot), then the same build again.
    h.script(&format!("GET {B}/frontend"), 200, new_slot(1, "empty", None))
        .script(&format!("GET {B}/frontend"), 200, new_slot(1, "empty", None))
        .script(&format!("GET {B}/frontend"), 200, new_slot(2, "active", Some("ver_9")))
        .script(&format!("GET {B}/frontend"), 200, deleted_slot(3))
        .script(&format!("GET {B}/frontend"), 200, new_slot(1, "empty", None))
        .script(&format!("GET {B}/frontend"), 200, new_slot(2, "active", Some("ver_9")));
    script_redeploy(&h, site.path());
    h.deploy_frontend(site.path(), &["--yes"]).await.unwrap();
    let journal: Journal = serde_json::from_slice(&std::fs::read(h.ctx.journal_dir.join(format!("demo-{ENV}-frontend.json"))).unwrap()).unwrap();
    assert_eq!((journal.slot_id.as_deref(), journal.version_slot_id.as_deref()), (Some("dpl_new"), Some("dpl_new")));
    let first = journal.intent_id.clone();
    h.deploy_frontend(site.path(), &["--yes"]).await.unwrap();
    let uploads = h.mock.seen(&format!("POST {B}/frontend/uploads"));
    assert_eq!(uploads.len(), 2, "a fresh upload after the delete");
    assert_ne!(uploads[0].key, uploads[1].key);
    let journal: Journal = serde_json::from_slice(&std::fs::read(h.ctx.journal_dir.join(format!("demo-{ENV}-frontend.json"))).unwrap()).unwrap();
    assert_ne!(journal.intent_id, first);
    let activations = h.mock.seen(&format!("POST {B}/frontend/versions/ver_9/activate"));
    assert_eq!(activations.len(), 2);
    assert_ne!(activations[0].key, activations[1].key, "no replay of the first activation");
    assert_eq!(activations[1].if_match.as_deref(), Some("\"g1\""));
}

#[test]
fn a_journal_resumes_only_on_its_own_live_slot() {
    let j = |slot: Option<&str>, version: Option<&str>| Journal { slot_id: slot.map(Into::into), version_slot_id: version.map(Into::into), ..Default::default() };
    let at = |id: &str, state: &str| Slot { deployment_id: Some(id.into()), state: Some(state.into()), generation: Some(1), ..Default::default() };
    assert!(!same_slot(&j(Some("a"), Some("a")), None));
    assert!(!same_slot(&j(None, None), live(Some(&at("a", "deleted")))));
    assert!(same_slot(&j(Some("a"), Some("a")), Some(&at("a", "active"))));
    assert!(!same_slot(&j(Some("a"), Some("a")), Some(&at("b", "active"))));
    assert!(!same_slot(&j(None, Some("a")), Some(&at("b", "empty"))));
    assert!(same_slot(&j(None, None), Some(&at("b", "empty"))), "an earlier build's journal resumes on a live slot");
}
