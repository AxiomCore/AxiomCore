//! `axiom auth` against a loopback stub of the Cloud admin API (auth-surfaces/v1 W4.5).
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output, Stdio},
    sync::{Arc, Mutex},
};

const DEV: &str = "0b5e2c1a-9f3d-4c7e-8a21-5d6f7e8a9b0c";
const PROD: &str = "1c2d3e4f-5a6b-4c7d-8e9f-0a1b2c3d4e5f";
const USER: &str = "7d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
const SHOP_ID: &str = "6a0c1f8e-3b2d-4c5e-9f70-1a2b3c4d5e6f";
/// A credential-shaped canary that must never be echoed by template output.
const TOKEN: &str = "axr1_SYNTHETICTEMPLATECANARY0123";
/// Synthetic credential shapes that heuristic redaction let through (P1 re-verification,
/// runtime-probes.log), plus an ordinary word and URL: by default none of the template
/// text may be printed.
const LEAKS: [&str; 13] = [
    "AbcDef0123456789AbcDef/+GhIjKlMnOpQrStUv==",
    "QWxhZGRpbjpvcGVuIHNlc2FtZQ==",
    "abcdefabcdefabcdefabcdefabcdefab",
    "AbcDefghiJklMnopQrStUvWxYz_-abcDEF",
    "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b",
    "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ4In0.c2lnbmF0dXJl",
    TOKEN,
    "axr1_SYNTHETIC\nCANARY0123456789",
    "9f86d081884c7d65\n9a2feaa0c55ad015a",
    "<a href=\"https://example.test/?token=AbcDefghiJklMnopQrStUvWxYz_-abcDEF\">Sign in</a>",
    "https://example.com/documentation/version2026gettingstarted",
    "internationalization",
    "Zanzibar",
];
/// The verifier's hex canary as an unsupported `{{variable}}`.
const PROBE_HEX: &str = "abcdefabcdefabcdefabcdefabcdefab";
const PROBE_SUBJECT: &str = "{{abcdefabcdefabcdefabcdefabcdefab}}";
const PROBE_KIND: &str = "AbcDefghiJklMnopQrStUvWxYz_-abcDEF";
const SUBJECT: &str = "Welcome to {{app_name}} Zanzibar AbcDefghiJklMnopQrStUvWxYz_-abcDEF";

fn template_text() -> String {
    format!("Hello {{{{recipient_email}}}}\n{}", LEAKS.join("\n"))
}

fn sha256(text: &str) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(text.as_bytes()))
}

/// `GET /api/v1/projects`: "storefront" is a lowercase display name whose slug differs,
/// and two projects share the name "twin".
fn projects() -> String {
    let project = |id: &str, name: &str, slug: &str| format!(r#"{{"id":"{id}","name":"{name}","slug":"{slug}","visibility":"private","created_at":"","updated_at":""}}"#);
    format!(
        "[{},{},{},{}]",
        project(SHOP_ID, "Web Shop", "shop"),
        project("2b3c4d5e-6f70-4a1b-8c2d-3e4f5a6b7c8d", "storefront", "shop-eu"),
        project("3c4d5e6f-7081-4b2c-9d3e-4f5a6b7c8d9e", "twin", "twin-a"),
        project("4d5e6f70-8192-4c3d-8e4f-5a6b7c8d9e0f", "Twin", "twin-b")
    )
}

type Log = Arc<Mutex<Vec<(String, String, String)>>>;

/// Serve canned responses on 127.0.0.1; `route` maps (method, path with
/// query) to (status, body). Every request is recorded.
fn stub(route: impl Fn(&str, &str, &str) -> (u16, String) + Send + 'static) -> (String, Log) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let log: Log = Arc::default();
    let recorded = log.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let mut parts = line.split_whitespace();
            let method = parts.next().unwrap_or_default().to_string();
            let path = parts.next().unwrap_or_default().to_string();
            let mut length = 0usize;
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                if header == "\r\n" || header.is_empty() {
                    break;
                }
                if let Some((name, value)) = header.split_once(':') {
                    if name.eq_ignore_ascii_case("content-length") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let body = String::from_utf8_lossy(&body).to_string();
            let (status, response) = route(&method, &path, &body);
            recorded.lock().unwrap().push((method, path, body));
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                response.len()
            );
        }
    });
    (address, log)
}

fn profile(directory: &Path) {
    std::fs::write(
        directory.join("auth.json"),
        r#"{"access_token":"stub-access","refresh_token":"stub-refresh","projects":{}}"#,
    )
    .unwrap();
}

fn axiom(cloud: &str, profile: &Path, args: &[&str], stdin: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_axiom-cli"))
        .current_dir(profile)
        .args(args)
        .env("CI", "true")
        .env("AXIOM_CLOUD_URL", cloud)
        .env("AXIOM_LOCAL_PROFILE_DIR", profile)
        .env_remove("AXIOM_REFERRAL_CODE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = stdin {
        child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    }
    drop(child.stdin.take());
    child.wait_with_output().unwrap()
}

fn route(method: &str, path: &str, body: &str) -> (u16, String) {
    let base = "/api/v1/projects/shop";
    let env = |e: &str| format!("{base}/auth/environments/{e}");
    match (method, path) {
        ("GET", "/api/v1/projects") => (200, projects()),
        ("GET", p) if p.ends_with("/configuration-environments") && p.starts_with("/api/v1/projects/shop") => (
            200,
            format!(r#"[{{"id":"{DEV}","slug":"development","kind":"development"}},{{"id":"{PROD}","slug":"production","kind":"production"}}]"#),
        ),
        ("GET", p) if p.starts_with(&format!("{}/users?", env(PROD))) => (
            200,
            format!(r#"{{"users":[{{"id":"{USER}","kind":"password","email":"jane@example.com","display_name":null,"status":"active","email_verified":true,"providers":["google"],"created_at":"2026-10-26T00:00:00Z","last_sign_in_at":null,"active_sessions":1,"roles_count":0}}],"next_cursor":null}}"#),
        ),
        ("POST", p) if p == format!("{}/users/search", env(PROD)) => (200, r#"{"users":[],"next_cursor":null}"#.into()),
        ("POST", p) if p == format!("{}/users/{USER}/ban", env(PROD)) => (409, r#"{"error":{"code":"AXAUTH_CONFLICT"}}"#.into()),
        ("GET", p) if p == format!("{}/settings", env(DEV)) => (403, r#"{"error":{"code":"AXAUTH_FORBIDDEN"}}"#.into()),
        ("GET", p) if p == format!("{}/providers", env(DEV)) => (200, r#"{"providers":[]}"#.into()),
        ("GET", "/api/v1/projects/shop-eu/auth/environments/0b5e2c1a-9f3d-4c7e-8a21-5d6f7e8a9b0c/email/usage") => (200, r#"{"sent":1,"budget":10,"suppressions":{}}"#.into()),
        ("POST", p) if p == format!("{}/email/suppressions/remove", env(DEV)) => (200, r#"{"removed":true}"#.into()),
        ("GET", p) if p == format!("{}/templates/signup", env(DEV)) => (
            200,
            serde_json::json!({"kind": "signup", "overridden": true, "subject": SUBJECT, "intro": template_text(), "outro": null, "revision": 2,
                "updated_at": "2026-11-01T00:00:00Z", "defaults": {"subject": "Welcome", "intro": "Hello", "outro": "Bye {{app_name}}"}})
            .to_string(),
        ),
        // P1 re-verification round 3 (variable-probe.log, probes.py): an unsupported
        // `{{variable}}` and an unknown nested field named `kind`.
        ("GET", p) if p == format!("{}/templates/invitation", env(DEV)) => (
            200,
            serde_json::json!({"kind": "invitation", "overridden": true, "subject": PROBE_SUBJECT, "intro": "Join {{app_name}}", "outro": null, "revision": 4,
                "updated_at": "2026-11-01T00:00:00Z", "defaults": {"subject": "Welcome", "intro": "Hello", "outro": "Bye"}, "new_field": {"kind": PROBE_KIND}})
            .to_string(),
        ),
        ("GET", p) if p == format!("{}/templates", env(DEV)) => (200, serde_json::json!({"templates": [{"kind": "signup", "overridden": true, "subject": SUBJECT, "revision": 2}]}).to_string()),
        ("POST", p) if p == format!("{}/templates/signup/preview", env(DEV)) => (200, serde_json::json!({"subject": SUBJECT, "text": template_text(), "html": format!("<p>{}</p>", template_text())}).to_string()),
        ("GET", "/api/v1/projects/other-shop") => (200, r#"{"id":"5e6f7081-92a3-4d4e-8f5a-6b7c8d9e0f1a","name":"Other Shop","slug":"other-shop","visibility":"private","created_at":"","updated_at":""}"#.into()),
        ("GET", "/api/v1/projects/not-mine") => (403, "workspace access denied".into()),
        ("GET", p) if p == format!("/api/v1/projects/other-shop/auth/environments/{DEV}/email/usage") => (200, r#"{"sent":2,"budget":10,"suppressions":{}}"#.into()),
        ("PUT", p) if p == format!("{}/templates/signup", env(DEV)) => {
            let mut saved: serde_json::Value = serde_json::from_str(body).unwrap();
            saved["kind"] = "signup".into();
            saved["overridden"] = true.into();
            saved["revision"] = 3.into();
            (200, saved.to_string())
        }
        ("POST", p) if p == format!("{}/providers", env(DEV)) => {
            let request: serde_json::Value = serde_json::from_str(body).unwrap();
            (201, format!(r#"{{"id":"{USER}","kind":"google","client_id":{},"client_secret_set":true,"enabled":false,"revision":1}}"#, request["client_id"]))
        }
        _ => (404, r#"{"error":{"code":"AXAUTH_NOT_FOUND"}}"#.into()),
    }
}

#[test]
fn users_list_resolves_the_environment_and_prints_golden_masked_json() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let output = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "--environment", "production", "users", "list", "--status", "active", "--limit", "10", "--json"], None);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let golden = format!(
        "{{\n  \"next_cursor\": null,\n  \"users\": [\n    {{\n      \"active_sessions\": 1,\n      \"created_at\": \"2026-10-26T00:00:00Z\",\n      \"display_name\": null,\n      \"email\": \"j***@e***.com\",\n      \"email_verified\": true,\n      \"id\": \"{USER}\",\n      \"kind\": \"password\",\n      \"last_sign_in_at\": null,\n      \"providers\": [\n        \"google\"\n      ],\n      \"roles_count\": 0,\n      \"status\": \"active\"\n    }}\n  ]\n}}\n"
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), golden);
    let requests = log.lock().unwrap().clone();
    assert_eq!(requests[0].1, "/api/v1/projects");
    assert_eq!(requests[1].1, "/api/v1/projects/shop/configuration-environments");
    assert_eq!(requests[2].1, format!("/api/v1/projects/shop/auth/environments/{PROD}/users?status=active&limit=10"));
    // An email search never puts the address in a URL.
    let output = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "--environment", PROD, "users", "list", "--search", "jane@"], None);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let requests = log.lock().unwrap().clone();
    let search = requests.last().unwrap();
    assert!(search.1.ends_with("/users/search") && !search.1.contains("jane"));
    assert!(search.2.contains("\"email_prefix\":\"jane@\""));
    let revealed = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "--environment", "production", "users", "list", "--limit", "5", "--reveal-email"], None);
    assert!(String::from_utf8_lossy(&revealed.stdout).contains("jane@example.com"));
}

#[test]
fn forbidden_and_conflict_map_to_exit_codes_3_and_4_and_usage_to_2() {
    let (cloud, _) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let forbidden = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "settings", "get"], None);
    assert_eq!(forbidden.status.code(), Some(3), "{}", String::from_utf8_lossy(&forbidden.stderr));
    assert!(String::from_utf8_lossy(&forbidden.stderr).contains("AXAUTH_FORBIDDEN"));
    let conflict = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "--environment", "production", "users", "ban", USER, "--reason", "abuse"], None);
    assert_eq!(conflict.status.code(), Some(4), "{}", String::from_utf8_lossy(&conflict.stderr));
    let unknown = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "--environment", "staging", "status"], None);
    assert_eq!(unknown.status.code(), Some(2));
    let unconfirmed = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "users", "revoke", USER], None);
    assert_eq!(unconfirmed.status.code(), Some(2));
    let flag = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "providers", "set", "google", "--client-secret", "x"], None);
    assert_eq!(flag.status.code(), Some(2));
}

#[test]
fn provider_secrets_come_from_stdin_and_are_never_printed() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let output = axiom(
        &cloud,
        directory.path(),
        &["auth", "--project", "shop", "providers", "set", "google", "--client-id", "client.apps", "--client-secret-stdin", "--json"],
        Some("SYNTHETIC_PROVIDER_SECRET\n"),
    );
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stdout.contains("SYNTHETIC_PROVIDER_SECRET") && !stderr.contains("SYNTHETIC_PROVIDER_SECRET"));
    assert!(stdout.contains("\"client_secret_set\": true"));
    let requests = log.lock().unwrap().clone();
    let create = requests.iter().find(|(m, p, _)| m == "POST" && p.ends_with("/providers")).unwrap();
    let body: serde_json::Value = serde_json::from_str(&create.2).unwrap();
    assert_eq!(body["client_secret"], "SYNTHETIC_PROVIDER_SECRET");
    assert_eq!(body["enabled"], false);
    // No prompt is possible without a terminal and no secret was piped.
    let refused = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "providers", "set", "google", "--client-id", "x"], None);
    assert_eq!(refused.status.code(), Some(2));
}

fn admin_requests(log: &Log) -> Vec<(String, String, String)> {
    log.lock().unwrap().iter().filter(|(_, p, _)| p.contains("/auth/environments/")).cloned().collect()
}

#[test]
fn projects_resolve_by_name_slug_or_id_through_the_project_list() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    // A lowercase display name that differs from its slug resolves to that slug.
    let named = axiom(&cloud, directory.path(), &["auth", "--project", "storefront", "--environment", DEV, "email", "usage", "--json"], None);
    assert!(named.status.success(), "{}", String::from_utf8_lossy(&named.stderr));
    assert_eq!(admin_requests(&log).last().unwrap().1, format!("/api/v1/projects/shop-eu/auth/environments/{DEV}/email/usage"));
    // Slugs and ids still resolve.
    for given in ["shop", SHOP_ID, "web shop"] {
        let output = axiom(&cloud, directory.path(), &["auth", "--project", given, "--environment", "production", "users", "list", "--limit", "1"], None);
        assert!(output.status.success(), "{given}: {}", String::from_utf8_lossy(&output.stderr));
        assert!(admin_requests(&log).last().unwrap().1.starts_with("/api/v1/projects/shop/"), "{given}");
    }
    // A name two projects share, and an unknown slug-shaped value: exit 2, nothing sent to a project.
    let before = log.lock().unwrap().len();
    let ambiguous = axiom(&cloud, directory.path(), &["auth", "--project", "twin", "--environment", DEV, "email", "usage"], None);
    assert_eq!(ambiguous.status.code(), Some(2), "{}", String::from_utf8_lossy(&ambiguous.stderr));
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("more than one project"));
    let unknown = axiom(&cloud, directory.path(), &["auth", "--project", "no-such-project", "--environment", DEV, "email", "usage"], None);
    assert_eq!(unknown.status.code(), Some(2), "{}", String::from_utf8_lossy(&unknown.stderr));
    let after: Vec<_> = log.lock().unwrap()[before..].to_vec();
    assert!(after.iter().all(|(m, p, _)| m == "GET" && p == "/api/v1/projects"), "{after:?}");
}

#[test]
fn email_unsuppress_needs_yes_without_a_terminal() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let unconfirmed = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "--environment", DEV, "email", "unsuppress", "bounced@example.com"], None);
    assert_eq!(unconfirmed.status.code(), Some(2), "{}", String::from_utf8_lossy(&unconfirmed.stderr));
    assert!(String::from_utf8_lossy(&unconfirmed.stderr).contains("--yes"));
    assert!(!log.lock().unwrap().iter().any(|(m, p, _)| m == "POST" && p.ends_with("/email/suppressions/remove")));
    let confirmed = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "--environment", DEV, "email", "unsuppress", "bounced@example.com", "--yes"], None);
    assert!(confirmed.status.success(), "{}", String::from_utf8_lossy(&confirmed.stderr));
    let requests = log.lock().unwrap().clone();
    let removed: Vec<_> = requests.iter().filter(|(m, p, _)| m == "POST" && p.ends_with("/email/suppressions/remove")).collect();
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].2, r#"{"email":"bounced@example.com"}"#);
}

/// Every piece of template text a test sample holds, split at line breaks as well (a
/// wrapped token must not leak half of itself).
fn pieces() -> Vec<String> {
    let mut pieces = vec![SUBJECT.to_string()];
    for leak in LEAKS {
        pieces.push(leak.to_string());
        pieces.extend(leak.split('\n').filter(|p| p.len() >= 8).map(str::to_string));
    }
    pieces.extend(["AbcDef0123456789AbcDef", "GhIjKlMnOpQrStUv", "example.test", "Hello", "Welcome", "Bye"].map(String::from));
    pieces
}

const TEMPLATE_COMMANDS: [&[&str]; 4] = [&["show", "signup"], &["list"], &["preview", "signup"], &["preview", "signup", "--html"]];

fn templates(cloud: &str, directory: &Path, command: &[&str], extra: &[&str]) -> Output {
    let mut args = vec!["auth", "--project", "shop", "--environment", DEV, "templates"];
    args.extend_from_slice(command);
    args.extend_from_slice(extra);
    axiom(cloud, directory, &args, None)
}

#[test]
fn template_text_is_not_printed_by_default_only_its_metadata() {
    let (cloud, _) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    for command in TEMPLATE_COMMANDS {
        for json in [false, true] {
            let shown = templates(&cloud, directory.path(), command, if json { &["--json"] } else { &[] });
            assert!(shown.status.success(), "{command:?}: {}", String::from_utf8_lossy(&shown.stderr));
            let stdout = String::from_utf8_lossy(&shown.stdout);
            let stderr = String::from_utf8_lossy(&shown.stderr);
            for piece in pieces() {
                assert!(!stdout.contains(&piece) && !stderr.contains(&piece), "{command:?} json={json} printed {piece:?}: {stdout}");
            }
            assert!(stdout.contains(&sha256(SUBJECT)), "{command:?}: {stdout}");
            if json {
                let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
                let subject = if command == ["list"] { &value["templates"][0]["subject"] } else { &value["subject"] };
                assert_eq!(subject["set"], true);
                assert_eq!(subject["bytes"], SUBJECT.len());
                assert_eq!(subject["variables"], serde_json::json!(["{{app_name}}"]));
                assert_eq!(subject["sha256"], sha256(SUBJECT));
                if command == ["show", "signup"] {
                    assert_eq!(value["kind"], "signup");
                    assert_eq!(value["revision"], 2);
                    assert_eq!(value["updated_at"], "2026-11-01T00:00:00Z");
                    assert_eq!(value["intro"]["sha256"], sha256(&template_text()));
                    assert_eq!(value["intro"]["variables"], serde_json::json!(["{{recipient_email}}"]));
                    assert_eq!(value["outro"], serde_json::json!({"set": false}));
                    assert_eq!(value["defaults"]["outro"]["variables"], serde_json::json!(["{{app_name}}"]));
                }
            } else {
                assert!(stdout.contains("--reveal-text"), "{command:?}: {stdout}");
                assert!(stdout.contains("variables {{app_name}}"), "{command:?}: {stdout}");
            }
        }
    }
}

#[test]
fn reveal_text_prints_template_text_exactly_after_a_warning() {
    let (cloud, _) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let intro = template_text();
    let html = format!("<p>{intro}</p>");
    let expected: [(&[&str], String); 4] = [
        (&["show", "signup"], format!("signup: customized (revision 2)\nSubject: {SUBJECT}\nIntro: {intro}\nOutro: (default) Bye {{{{app_name}}}}\n")),
        (&["list"], format!("signup  customized  revision 2  subject: {SUBJECT}\n")),
        (&["preview", "signup"], format!("Subject: {SUBJECT}\n\n{intro}\n")),
        (&["preview", "signup", "--html"], format!("Subject: {SUBJECT}\n\n{html}\n")),
    ];
    for (command, human) in expected {
        let shown = templates(&cloud, directory.path(), command, &["--reveal-text"]);
        assert!(shown.status.success(), "{command:?}: {}", String::from_utf8_lossy(&shown.stderr));
        assert_eq!(String::from_utf8_lossy(&shown.stdout), human, "{command:?}");
        let stderr = String::from_utf8_lossy(&shown.stderr);
        assert_eq!(stderr.lines().count(), 1, "{stderr}");
        assert!(stderr.contains("warning") && stderr.contains("--reveal-text"), "{stderr}");
        let json = templates(&cloud, directory.path(), command, &["--reveal-text", "--json"]);
        let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
        match command {
            ["list"] => assert_eq!(value["templates"][0]["subject"], SUBJECT),
            ["show", _] => {
                assert_eq!(value["subject"], SUBJECT);
                assert_eq!(value["intro"], intro);
                assert_eq!(value["defaults"]["outro"], "Bye {{app_name}}");
            }
            _ => {
                assert_eq!(value["text"], intro);
                assert_eq!(value["html"], html);
            }
        }
    }
}

#[test]
fn template_text_comes_from_files_or_stdin_and_is_not_echoed() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    // Template text is never argv.
    let argv = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "--environment", DEV, "templates", "set", "signup", "--subject", "Welcome"], None);
    assert_eq!(argv.status.code(), Some(2), "{}", String::from_utf8_lossy(&argv.stderr));
    assert!(!log.lock().unwrap().iter().any(|(m, _, _)| m == "PUT"));

    // Empty stdin is refused rather than resetting the subject to the default.
    let empty = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "--environment", DEV, "templates", "set", "signup", "--subject-file", "-"], Some(""));
    assert_eq!(empty.status.code(), Some(2), "{}", String::from_utf8_lossy(&empty.stderr));
    assert!(!log.lock().unwrap().iter().any(|(m, _, _)| m == "PUT"));

    // The subject from stdin is sent as given; the saved template echoed back shows no text.
    let intro = directory.path().join("intro.txt");
    std::fs::write(&intro, "Welcome aboard, {{recipient_email}}.\n").unwrap();
    let intro = intro.to_str().unwrap();
    for json in [true, false] {
        let mut args = vec!["auth", "--project", "shop", "--environment", DEV, "templates", "set", "signup", "--subject-file", "-", "--intro-file", intro];
        if json {
            args.push("--json");
        }
        let set = axiom(&cloud, directory.path(), &args, Some(&format!("Welcome {TOKEN}\n")));
        assert!(set.status.success(), "{}", String::from_utf8_lossy(&set.stderr));
        let stdout = String::from_utf8_lossy(&set.stdout);
        assert!(!stdout.contains(TOKEN) && !stdout.contains("Welcome") && !stdout.contains("Zanzibar"), "{stdout}");
        assert!(stdout.contains(&sha256(&format!("Welcome {TOKEN}"))), "{stdout}");
    }
    let requests = log.lock().unwrap().clone();
    let put = requests.iter().find(|(m, _, _)| m == "PUT").unwrap();
    let body: serde_json::Value = serde_json::from_str(&put.2).unwrap();
    assert_eq!(body["subject"], format!("Welcome {TOKEN}"));
    assert_eq!(body["intro"], "Welcome aboard, {{recipient_email}}.");
    assert_eq!(body["revision"], 2);
}

/// Every command that asks before it removes, revokes or suspends something.
const DESTRUCTIVE: [&[&str]; 21] = [
    &["email", "unsuppress", "test@example.com"],
    &["users", "delete", USER],
    &["users", "remove-factor", USER, USER],
    &["users", "revoke", USER],
    &["organizations", "update", USER, "--status", "suspended"],
    &["organizations", "remove-member", USER, USER],
    &["invitations", "revoke", USER],
    &["roles", "delete", USER],
    &["permissions", "delete", USER],
    &["assignments", "remove", USER],
    &["providers", "remove", "google"],
    &["trusted-issuers", "remove", USER],
    &["smtp", "remove"],
    &["templates", "reset", "signup"],
    // A role by name (looked up with a request when confirmed).
    &["roles", "delete", "editor"],
    // Older commands with the same prompt.
    &["organizations", "add-member", USER, USER, "--status", "suspended"],
    &["users", "reset-factors", USER, "--reason", "lost phone"],
    &["users", "revoke-consent", USER, USER],
    &["webhooks", "delete", USER],
    &["hooks", "remove", "token_claims"],
    &["sms", "remove"],
];

#[test]
fn destructive_commands_without_yes_or_a_terminal_exit_2_before_any_request() {
    let (cloud, log) = stub(route);
    let signed_in = tempfile::tempdir().unwrap();
    profile(signed_in.path());
    let signed_out = tempfile::tempdir().unwrap();
    for command in DESTRUCTIVE {
        for directory in [signed_in.path(), signed_out.path()] {
            for project in [&["--project", "shop"][..], &[]] {
                let mut args = vec!["auth", "--environment", "development"];
                args.extend_from_slice(project);
                args.extend_from_slice(command);
                let before = log.lock().unwrap().len();
                let refused = axiom(&cloud, directory, &args, None);
                let stderr = String::from_utf8_lossy(&refused.stderr);
                assert_eq!(refused.status.code(), Some(2), "{args:?}: {stderr}");
                assert_eq!(stderr.trim_end(), "axiom auth: pass --yes to confirm without a prompt", "{args:?}");
                let sent: Vec<_> = log.lock().unwrap()[before..].to_vec();
                assert!(sent.is_empty(), "{args:?} sent {sent:?}");
            }
        }
        // With --yes the command goes ahead (it signs in and sends requests).
        let mut args = vec!["auth", "--project", "shop", "--environment", "development"];
        args.extend_from_slice(command);
        args.push("--yes");
        let before = log.lock().unwrap().len();
        let _ = axiom(&cloud, signed_in.path(), &args, None);
        assert!(log.lock().unwrap().len() > before, "{args:?} sent nothing with --yes");
    }
    // Reactivating asks nothing, so it runs without --yes or a terminal.
    for command in [&["organizations", "update", USER, "--status", "active"][..], &["organizations", "add-member", USER, USER]] {
        let mut args = vec!["auth", "--project", "shop", "--environment", "development"];
        args.extend_from_slice(command);
        let before = log.lock().unwrap().len();
        let ran = axiom(&cloud, signed_in.path(), &args, None);
        assert!(!String::from_utf8_lossy(&ran.stderr).contains("--yes"), "{args:?}");
        assert!(log.lock().unwrap().len() > before, "{args:?} was refused");
    }
    // An OAuth client delete without its typed phrase is refused the same way, before anything.
    let before = log.lock().unwrap().len();
    let refused = axiom(&cloud, signed_out.path(), &["auth", "--project", "shop", "oauth-apps", "delete", USER], None);
    assert_eq!(refused.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&refused.stderr).contains(&format!("--confirm \"delete client {USER}\"")));
    assert_eq!(log.lock().unwrap().len(), before);
}

#[test]
fn org_selects_a_project_outside_the_current_organization_by_exact_slug() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    // Not in the current organization's list: exit 2, and the error names --org.
    let listed_only = axiom(&cloud, directory.path(), &["auth", "--project", "other-shop", "--environment", DEV, "email", "usage"], None);
    assert_eq!(listed_only.status.code(), Some(2), "{}", String::from_utf8_lossy(&listed_only.stderr));
    assert!(String::from_utf8_lossy(&listed_only.stderr).contains("--org"));
    // With --org the exact slug is looked up directly and used.
    let output = axiom(&cloud, directory.path(), &["auth", "--project", "other-shop", "--org", "acme-labs", "--environment", DEV, "email", "usage", "--json"], None);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let requests = log.lock().unwrap().clone();
    let tail: Vec<_> = requests[requests.len() - 2..].iter().map(|(m, p, _)| format!("{m} {p}")).collect();
    assert_eq!(tail, ["GET /api/v1/projects/other-shop".to_string(), format!("GET /api/v1/projects/other-shop/auth/environments/{DEV}/email/usage")]);
    // Names, ids, malformed values and projects you cannot use: exit 2, nothing sent to a project's auth.
    let before = log.lock().unwrap().len();
    for (project, org) in [("Other Shop", "acme-labs"), ("5e6f7081-92a3-4d4e-8f5a-6b7c8d9e0f1a", "acme-labs"), ("not-mine", "acme-labs"), ("other-shop", "Acme Labs")] {
        let refused = axiom(&cloud, directory.path(), &["auth", "--project", project, "--org", org, "--environment", DEV, "email", "usage"], None);
        assert_eq!(refused.status.code(), Some(2), "{project} {org}: {}", String::from_utf8_lossy(&refused.stderr));
    }
    let refused = axiom(&cloud, directory.path(), &["auth", "--org", "acme-labs", "--environment", DEV, "email", "usage"], None);
    assert_eq!(refused.status.code(), Some(2));
    // A malformed --org is a usage error even when signed out.
    let signed_out = tempfile::tempdir().unwrap();
    for args in [&["auth", "--org", "acme-labs", "email", "usage"][..], &["auth", "--project", "Other Shop", "--org", "acme-labs", "email", "usage"]] {
        let refused = axiom(&cloud, signed_out.path(), args, None);
        assert_eq!(refused.status.code(), Some(2), "{args:?}: {}", String::from_utf8_lossy(&refused.stderr));
    }
    let after: Vec<_> = log.lock().unwrap()[before..].iter().map(|(m, p, _)| format!("{m} {p}")).collect();
    // A UUID is slug-shaped, so the API is asked and answers that no such project exists.
    assert_eq!(after, ["GET /api/v1/projects/5e6f7081-92a3-4d4e-8f5a-6b7c8d9e0f1a", "GET /api/v1/projects/not-mine"], "{after:?}");
}

#[test]
fn unsupported_variables_are_counted_and_never_echoed() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    // An unsupported variable read from stdin: exit 2, the error names the field and a count only.
    for command in [&["set", "signup", "--subject-file", "-"][..], &["preview", "signup", "--subject-file", "-"]] {
        let mut args = vec!["auth", "--project", "shop", "--environment", DEV, "templates"];
        args.extend_from_slice(command);
        let refused = axiom(&cloud, directory.path(), &args, Some(&format!("Hi {PROBE_SUBJECT} and {{{{zzz_other}}}}\n")));
        assert_eq!(refused.status.code(), Some(2), "{command:?}");
        let stderr = String::from_utf8_lossy(&refused.stderr);
        assert!(!stderr.contains(PROBE_HEX) && !stderr.contains("zzz_other"), "{command:?}: {stderr}");
        assert!(stderr.contains("the subject") && stderr.contains("2 unsupported placeholders"), "{command:?}: {stderr}");
    }
    assert!(!log.lock().unwrap().iter().any(|(m, p, _)| m == "PUT" || p.ends_with("/preview")));

    // A stored subject with an unsupported variable, and an unknown nested `kind`: only counts and metadata.
    for json in [false, true] {
        let shown = templates(&cloud, directory.path(), &["show", "invitation"], if json { &["--json"] } else { &[] });
        assert!(shown.status.success(), "{}", String::from_utf8_lossy(&shown.stderr));
        let stdout = String::from_utf8_lossy(&shown.stdout);
        let stderr = String::from_utf8_lossy(&shown.stderr);
        for piece in [PROBE_HEX, PROBE_KIND, "Join"] {
            assert!(!stdout.contains(piece) && !stderr.contains(piece), "json={json} printed {piece}: {stdout}");
        }
        if json {
            let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
            assert_eq!(value["subject"]["variables"], serde_json::json!([]));
            assert_eq!(value["subject"]["unsupported_placeholders"], 1);
            assert_eq!(value["subject"]["sha256"], sha256(PROBE_SUBJECT));
            assert_eq!(value["intro"]["variables"], serde_json::json!(["{{app_name}}"]));
            assert_eq!(value["new_field"], serde_json::Value::Null);
            assert_eq!(value["unknown_fields"], 1);
            assert_eq!(value["kind"], "invitation");
            assert_eq!(value["updated_at"], "2026-11-01T00:00:00Z");
        } else {
            assert!(stdout.contains("1 unsupported placeholder,"), "{stdout}");
        }
    }
}

/// Every path to a string in `value`, as JSON pointers.
fn string_paths(value: &serde_json::Value, at: String, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(_) => out.push(at),
        serde_json::Value::Object(map) => map.iter().for_each(|(k, v)| string_paths(v, format!("{at}/{k}"), out)),
        serde_json::Value::Array(items) => items.iter().enumerate().for_each(|(i, v)| string_paths(v, format!("{at}/{i}"), out)),
        _ => {}
    }
}

/// Every path to an object in `value`, as JSON pointers (the root is "").
fn object_paths(value: &serde_json::Value, at: String, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            out.push(at.clone());
            map.iter().for_each(|(k, v)| object_paths(v, format!("{at}/{k}"), out));
        }
        serde_json::Value::Array(items) => items.iter().enumerate().for_each(|(i, v)| object_paths(v, format!("{at}/{i}"), out)),
        _ => {}
    }
}

#[test]
fn a_marker_in_any_string_of_a_templates_response_is_never_printed_by_default() {
    let future = serde_json::json!({"kind": "signup", "created_at": "2026-11-01T00:00:00Z", "name": "n", "id": "i", "items": ["s", {"updated_at": "2026-11-01T00:00:00Z", "kind": "magic_link"}]});
    let list = serde_json::json!({"templates": [{"kind": "signup", "overridden": true, "subject": "Welcome", "revision": 2, "updated_at": "2026-11-01T00:00:00Z", "future": future}], "note": "n"});
    let view = serde_json::json!({"kind": "signup", "overridden": true, "subject": "Welcome", "intro": "Hi", "outro": "Bye", "revision": 2, "updated_at": "2026-11-01T00:00:00Z",
        "defaults": {"subject": "a", "intro": "b", "outro": "c", "kind": "signup"}, "new_field": future, "name": "j", "id": "k", "created_at": "2026-11-01T00:00:00Z"});
    let preview = serde_json::json!({"subject": "Welcome", "text": "Hi", "html": "<p>Hi</p>", "kind": "signup", "meta": future});
    // `set` reads the current template (the response) and sends it back with the new intro.
    let set: &[&str] = &["set", "signup", "--intro-file", "intro.txt"];
    let samples: [(&serde_json::Value, Vec<&[&str]>); 3] = [
        (&list, vec![&["list"]]),
        (&view, vec![&["show", "signup"], set]),
        (&preview, vec![&["preview", "signup"], &["preview", "signup", "--html"]]),
    ];
    // A marker alone, as an unsupported variable, and inside text; in every string, and
    // as a key added to every object (`email/tests.rs` has more forms).
    let markers = ["QzMarker0123456789Zq", "{{qzmarkerzq}}", "Hi {{qzmarkerzq}} QzMarker0123456789Zq"];
    let mut jobs = Vec::new();
    for (sample, commands) in &samples {
        let mut probes = Vec::new();
        let mut paths = Vec::new();
        string_paths(sample, String::new(), &mut paths);
        let mut objects = Vec::new();
        object_paths(sample, String::new(), &mut objects);
        for marker in markers {
            for path in &paths {
                let mut probe = (*sample).clone();
                *probe.pointer_mut(path).unwrap() = marker.into();
                probes.push((format!("{path} = {marker:?}"), probe, marker));
            }
        }
        for path in &objects {
            let mut probe = (*sample).clone();
            probe.pointer_mut(path).unwrap()[markers[0]] = serde_json::json!({markers[2]: markers[0]});
            probes.push((format!("{path}/{:?} key", markers[0]), probe, markers[0]));
        }
        for (label, probe, marker) in probes {
            for command in commands {
                for json in [false, true] {
                    jobs.push((format!("{command:?} json={json} {label}"), probe.to_string(), *command, json, false));
                }
                // Positive control: with --reveal-text the marker is printed (unless the text is refused).
                if marker == markers[0] {
                    jobs.push((format!("{command:?} reveal {label}"), probe.to_string(), *command, true, true));
                }
            }
        }
    }
    assert!(jobs.len() > 300, "{}", jobs.len());
    // Four stubs (each answers every templates route with the current probe) in parallel.
    let chunk = jobs.len().div_ceil(4);
    let (hidden_ok, revealed) = (std::sync::atomic::AtomicUsize::new(0), std::sync::atomic::AtomicUsize::new(0));
    let counts = (&hidden_ok, &revealed);
    std::thread::scope(|scope| {
        for part in jobs.chunks(chunk) {
            scope.spawn(move || {
                let response = Arc::new(Mutex::new(String::new()));
                let answer = response.clone();
                let (cloud, _) = stub(move |method, path, body| if path.contains("/templates") { (200, answer.lock().unwrap().clone()) } else { route(method, path, body) });
                let directory = tempfile::tempdir().unwrap();
                profile(directory.path());
                std::fs::write(directory.path().join("intro.txt"), "Welcome aboard.\n").unwrap();
                for (label, probe, command, json, reveal) in part {
                    *response.lock().unwrap() = probe.clone();
                    let extra: &[&str] = match (json, reveal) {
                        (_, true) => &["--json", "--reveal-text"],
                        (true, false) => &["--json"],
                        (false, false) => &[],
                    };
                    let shown = templates(&cloud, directory.path(), command, extra);
                    let printed = format!("{}{}", String::from_utf8_lossy(&shown.stdout), String::from_utf8_lossy(&shown.stderr)).to_lowercase();
                    let ordering = std::sync::atomic::Ordering::Relaxed;
                    if *reveal {
                        if shown.status.success() {
                            assert!(printed.contains("qzmarker"), "{label}: {printed}");
                            counts.1.fetch_add(1, ordering);
                        }
                    } else {
                        assert!(!printed.contains("qzmarker"), "{label}: {printed}");
                        if shown.status.success() {
                            counts.0.fetch_add(1, ordering);
                        }
                    }
                }
            });
        }
    });
    // Most probes print a summary; the rest are refused before sending (a marker in text a `set` keeps).
    let ordering = std::sync::atomic::Ordering::Relaxed;
    let hidden_jobs = jobs.iter().filter(|j| !j.4).count();
    assert!(hidden_ok.load(ordering) * 10 > hidden_jobs * 8, "{hidden_ok:?} of {hidden_jobs}");
    assert!(revealed.load(ordering) * 10 > (jobs.len() - hidden_jobs) * 8, "{revealed:?} of {}", jobs.len() - hidden_jobs);
}

/// P1 re-verification round 4 (p1-r4-evidence/VERIFY.txt): `updated_at` values of the
/// right shape that are not real instants are summarized like any other text, and
/// secret-shaped unknown keys are counted in `unknown_fields` like every other one.
#[test]
fn impossible_timestamps_are_hidden_and_secret_shaped_unknown_keys_are_counted() {
    const IMPOSSIBLE: [&str; 9] = [
        "9999-99-99T99:99:99Z",
        "2026-02-30T12:00:00Z",
        "2026-11-14T12:30:00+99:99",
        "2026-13-01T00:00:00Z",
        "2026-11-14T24:00:00Z",
        "2026-11-14T23:59:60Z",
        "2026-11-14T12:30:00Z junk",
        "+10000-01-01T00:00:00Z",
        "99999-01-01T00:00:00Z",
    ];
    const SECRET: &str = "zzz-synthetic-secret-value";
    let mut entries: Vec<serde_json::Value> = IMPOSSIBLE.iter().map(|at| serde_json::json!({"kind": "signup", "overridden": false, "revision": 0, "updated_at": at})).collect();
    entries.push(serde_json::json!({"kind": "magic_link", "overridden": true, "revision": 1, "updated_at": "2026-11-14T12:30:00Z",
        "secret": SECRET, "password": SECRET, "access_token": SECRET, "new_field": SECRET}));
    let listed = serde_json::json!({"templates": entries, "updated_at": IMPOSSIBLE[0], "client_secret": SECRET, "note": SECRET}).to_string();
    let (cloud, _log) = stub(move |method, path, body| {
        if method == "GET" && path.ends_with(&format!("/auth/environments/{DEV}/templates")) {
            (200, listed.clone())
        } else {
            route(method, path, body)
        }
    });
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let output = templates(&cloud, directory.path(), &["list"], &["--json"]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    for piece in IMPOSSIBLE.iter().chain(&[SECRET]) {
        assert!(!stdout.contains(piece), "printed {piece:?}: {stdout}");
    }
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["updated_at"]["set"], true, "{value}");
    for (index, at) in IMPOSSIBLE.iter().enumerate() {
        assert_eq!(value["templates"][index]["updated_at"]["set"], true, "{at}: {value}");
    }
    let last = &value["templates"][IMPOSSIBLE.len()];
    assert_eq!(last["updated_at"], "2026-11-14T12:30:00Z");
    assert_eq!(last["unknown_fields"], 4, "{value}");
    assert_eq!(value["unknown_fields"], 2, "{value}");
}
