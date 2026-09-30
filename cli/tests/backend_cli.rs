use std::path::Path;
use std::process::{Command, Output};

const SOURCE: &str = r#"
project { id = "backend-cli-regression" version = "1.0.0" }
variants {
  ["default"] { include = Listing { "*" } }
  ["mobile"] { include = Listing { "getProject" } }
}
endpoints {
  ["getProject"] = EndpointDef { name = "getProject" method = "GET" path = "/projects/{id}" }
  ["listProjects"] = EndpointDef { name = "listProjects" method = "GET" path = "/projects" }
}
testConfig = TestConfig { suites = Listing {} }
"#;

fn cli(directory: &Path, arguments: &[&str], ci: bool) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_axiom-cli"));
    command
        .current_dir(directory)
        .args(arguments)
        // The default production endpoint makes the classification itself
        // responsible for skipping registration, never a developer's local
        // control-plane override inherited by the test process.
        .env_remove("AXIOM_CLOUD_URL")
        .env_remove("AXIOM_REFERRAL_CODE");
    if ci {
        command.env("CI", "true");
    } else {
        command.env_remove("CI");
    }
    command.output().unwrap()
}

fn success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Private Alpha"));
}

#[test]
fn manifest_selected_contract_build_and_inferred_dependency_resolution_work_offline() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("AxiomDeps.toml"),
        "type = \"backend\"\naudiences = [\"mobile\"]\n",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("axiom.acore"),
        r#"
project(id: "inferred-contract", version: "1.0.0")
variant default(include: ["*"])
model Task { id: String } also entity
projection card(entity: Entities.task, fields: [.id], audience: Audience.mobile)
endpoint list(method: GET, path: "/tasks") {
  response(type: List<Entities.task>, projection: Projections.card)
  errors("unavailable")
  cacheIdentity("tasks:list")
}
"#,
    )
    .unwrap();
    success(&cli(directory.path(), &["build", "axiom.acore"], true));
    std::fs::write(directory.path().join("AxiomDeps.toml"), "type = \"backend\"\naudiences = [\"mobile\"]\n[contracts.tasks]\nartifact = \"axiom.axiom\"\nbaseUrl = \"http://localhost:8080\"\n").unwrap();
    for action in ["resolve", "verify"] {
        let result = Command::new(env!("CARGO_BIN_EXE_axiom-cli"))
            .current_dir(directory.path())
            .args(["contract", action])
            .env("CI", "true")
            .env("AXIOM_CLOUD_URL", "invalid-cloud-endpoint")
            .env_remove("AXIOM_REFERRAL_CODE")
            .output()
            .unwrap();
        success(&result);
    }
    let lock: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(directory.path().join("axiom.ui.lock.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(lock["contracts"]["tasks"]["audience"], "mobile");
    assert_eq!(
        lock["contracts"]["tasks"]["operations"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(lock["contracts"]["tasks"]["operations"][0]["name"], "list");
}

#[test]
fn local_authoring_does_not_consult_cloud_configuration() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("axiom.acore"), SOURCE).unwrap();
    for arguments in [
        vec!["check", "--json"],
        vec!["eval", "axiom.acore", "--format", "json"],
        vec!["explain", "axiom.acore"],
        vec!["onboard", "--role", "backend"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_axiom-cli"))
            .current_dir(directory.path())
            .args(arguments)
            .env("CI", "true")
            .env("AXIOM_CLOUD_URL", "invalid-cloud-endpoint")
            .env_remove("AXIOM_REFERRAL_CODE")
            .output()
            .unwrap();
        success(&output);
    }
    let initialized = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_axiom-cli"))
        .current_dir(initialized.path())
        .args(["init", "main.py:app", "--module", "axiom-fastapi"])
        .env("CI", "true")
        .env("AXIOM_CLOUD_URL", "invalid-cloud-endpoint")
        .env_remove("AXIOM_REFERRAL_CODE")
        .output()
        .unwrap();
    success(&output);
    assert!(
        std::fs::read_to_string(initialized.path().join("axiom.acore"))
            .unwrap()
            .starts_with("amends ")
    );
    assert!(
        std::fs::read_to_string(initialized.path().join("AxiomDeps.toml"))
            .unwrap()
            .contains("type = \"backend\"")
    );
}

#[test]
fn local_artifact_pull_validates_options_without_cloud_access_or_a_terminal() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("contract.axiom"), b"local artifact").unwrap();
    for ci in [true, false] {
        for source in [vec!["contract.axiom"], vec!["--contract", "contract.axiom"]] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_axiom-cli"));
            command
                .current_dir(directory.path())
                .arg("pull")
                .args(source)
                // Reject options before invoking an external generator or
                // installing anything in the user's contract cache.
                .args([
                    "--framework",
                    "unsupported",
                    "--name",
                    "local",
                    "--out",
                    "generated",
                ])
                .env("AXIOM_CLOUD_URL", "invalid-cloud-endpoint")
                .env_remove("AXIOM_REFERRAL_CODE");
            if ci {
                command.env("CI", "true");
            } else {
                command.env_remove("CI");
            }
            let output = command.output().unwrap();
            assert!(!output.status.success());
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("Unknown framework 'unsupported'"),
                "{output:?}"
            );
            assert!(!String::from_utf8_lossy(&output.stdout).contains("Private Alpha"));
        }
    }
    assert!(!directory.path().join("AxiomDeps.toml").exists());
}

#[test]
fn cloud_commands_retain_cloud_configuration_and_access_checks() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("AxiomDeps.toml"),
        "framework = \"atmx-web\"\n[contracts.remote]\nsource = \"https://api.axiomcore.dev/org/project\"\n",
    )
    .unwrap();
    std::fs::write(directory.path().join("contracts.json"), "{}").unwrap();
    for arguments in [
        vec!["build", "--release"],
        vec!["release"],
        vec!["deploy", "mock-server"],
        vec!["serve"],
        vec!["watch"],
        vec!["pull", "org/project"],
        vec![
            "pull",
            "--contract",
            "https://api.axiomcore.dev/org/project",
        ],
        vec!["pull", "AxiomDeps.toml"],
        vec!["pull", "--contract", "contracts.json"],
        vec!["pull", "--contract-config", "AxiomDeps.toml"],
        vec!["pull"],
        vec!["onboard", "--role", "frontend", "--contract", "org/project"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_axiom-cli"))
            .current_dir(directory.path())
            .args(&arguments)
            .env("CI", "true")
            .env("AXIOM_CLOUD_URL", "invalid-cloud-endpoint")
            .env_remove("AXIOM_REFERRAL_CODE")
            .output()
            .unwrap();
        assert!(!output.status.success(), "{arguments:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("AXIOM_CLOUD_URL"),
            "{arguments:?}: {output:?}"
        );
    }
    assert!(!directory.path().join("axiom.axiom").exists());
}

#[test]
fn ci_and_nonterminal_builds_write_json_and_diff_without_cloud_access() {
    for ci in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("axiom.acore");
        std::fs::write(&source, SOURCE).unwrap();
        success(&cli(directory.path(), &["build"], ci));
        let original = std::fs::read(source.with_extension("acore.lockfile")).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&original).unwrap();
        assert_eq!(json["endpoints"].as_object().unwrap().len(), 2);

        success(&cli(
            directory.path(),
            &["build", "--variant", "mobile"],
            ci,
        ));
        let mobile: serde_json::Value = serde_json::from_slice(
            &std::fs::read(directory.path().join("axiom.acore.mobile.lockfile")).unwrap(),
        )
        .unwrap();
        assert_eq!(mobile["endpoints"].as_object().unwrap().len(), 1);
        assert!(mobile["endpoints"].get("getProject").is_some());
        assert_eq!(
            std::fs::read(source.with_extension("acore.lockfile")).unwrap(),
            original
        );
        for variant in ["default", "mobile"] {
            let diff = cli(
                directory.path(),
                &[
                    "diff",
                    "axiom.acore",
                    "--format",
                    "atom",
                    "--variant",
                    variant,
                ],
                ci,
            );
            success(&diff);
            let patch: serde_json::Value = serde_json::from_slice(&diff.stdout).unwrap();
            assert!(
                patch["operations"].as_array().unwrap().is_empty(),
                "{patch}"
            );
        }

        std::fs::write(&source, "project { id =").unwrap();
        let failure = cli(directory.path(), &["build"], ci);
        assert!(!failure.status.success());
        assert_eq!(
            std::fs::read(source.with_extension("acore.lockfile")).unwrap(),
            original
        );
    }
}

#[test]
fn check_explain_eval_and_empty_tests_remain_local_and_do_not_write_baselines() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("axiom.acore"), SOURCE).unwrap();
    let check = cli(directory.path(), &["check", "--json"], true);
    success(&check);
    let checked: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
    assert_eq!(checked["valid"], true);
    let schema = cli(directory.path(), &["explain", "--schema"], true);
    success(&schema);
    let catalog: serde_json::Value = serde_json::from_slice(&schema.stdout).unwrap();
    assert_eq!(catalog["format"], "acore-backend-language/v1");
    assert!(catalog["declarations"].get("cache").is_some());
    let explained = cli(
        directory.path(),
        &["explain", "axiom.acore", "--variant", "mobile"],
        true,
    );
    success(&explained);
    let evaluated = cli(
        directory.path(),
        &[
            "eval",
            "axiom.acore",
            "--format",
            "json",
            "--variant",
            "mobile",
        ],
        true,
    );
    success(&evaluated);
    let explain_json: serde_json::Value = serde_json::from_slice(&explained.stdout).unwrap();
    let eval_json: serde_json::Value = serde_json::from_slice(&evaluated.stdout).unwrap();
    assert_eq!(explain_json, eval_json);
    let evidence = cli(
        directory.path(),
        &[
            "explain",
            "axiom.acore",
            "--variant",
            "mobile",
            "--provenance",
        ],
        true,
    );
    success(&evidence);
    let evidence: serde_json::Value = serde_json::from_slice(&evidence.stdout).unwrap();
    assert_eq!(evidence["format"], "acore-backend-explanation/v1");
    assert_eq!(evidence["contract"], eval_json);
    assert!(
        evidence["endpointIdentities"]["getProject"]["id"]
            .as_u64()
            .unwrap()
            > 0
    );
    success(&cli(
        directory.path(),
        &["test", "--variant", "mobile"],
        true,
    ));
    assert!(!directory.path().join("axiom.axiom").exists());
    assert!(!directory.path().join("axiom.acore.lockfile").exists());
}

#[test]
fn invalid_paths_and_legacy_source_baselines_report_errors_without_panicking() {
    let directory = tempfile::tempdir().unwrap();
    let missing = cli(directory.path(), &["eval", "missing.acore"], true);
    assert!(!missing.status.success());
    assert!(!String::from_utf8_lossy(&missing.stderr).contains("panicked"));
    let invalid_check = cli(
        directory.path(),
        &["check", "missing.acore", "--json"],
        true,
    );
    assert!(!invalid_check.status.success());
    let report: serde_json::Value = serde_json::from_slice(&invalid_check.stdout).unwrap();
    assert_eq!(report["valid"], false);
    assert!(!report["diagnostics"].as_array().unwrap().is_empty());
    std::fs::write(directory.path().join("axiom.acore"), SOURCE).unwrap();
    std::fs::write(directory.path().join("axiom.acore.lockfile"), SOURCE).unwrap();
    let legacy = cli(directory.path(), &["diff", "axiom.acore"], true);
    assert!(!legacy.status.success());
    assert!(String::from_utf8_lossy(&legacy.stderr).contains("Invalid JSON baseline"));
}

#[test]
fn migration_preserves_explicit_identity_and_checks_all_variants_before_writing() {
    let directory = tempfile::tempdir().unwrap();
    let source = r#"
        project { id = "migration-regression" version = "1.0.0" }
        variants {
            ["default"] { include = Listing { "*" } }
            ["mobile"] { include = Listing { "GetProject" } }
        }
        endpoints {
            ["GetProject"] = EndpointDef {
                id = 91 name = "get_project" method = "GET" path = "/projects"
                returnType = TypeRefPrimitive { kind = "string" }
            }
            ["Admin"] = EndpointDef {
                id = 11 name = "admin" method = "DELETE" path = "/admin"
                returnType = TypeRefPrimitive { kind = "void" }
            }
        }
    "#;
    std::fs::write(directory.path().join("old.acore"), source).unwrap();
    let migrated = cli(
        directory.path(),
        &["migrate", "old.acore", "--output", "new.acore"],
        true,
    );
    success(&migrated);
    assert_eq!(
        std::fs::read_to_string(directory.path().join("old.acore")).unwrap(),
        source
    );
    let new = std::fs::read_to_string(directory.path().join("new.acore")).unwrap();
    assert!(!new.contains("profile backend"));
    assert!(
        std::fs::read_to_string(directory.path().join("AxiomDeps.toml"))
            .unwrap()
            .contains("type = \"backend\"")
    );
    assert!(!new.contains("id: 91"));
    let identities: serde_json::Value = serde_json::from_slice(
        &std::fs::read(directory.path().join("new.acore.identities.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(identities["endpoints"]["GetProject"]["id"], 91);
    assert_eq!(identities["endpoints"]["GetProject"]["name"], "get_project");
    assert_eq!(identities["endpoints"]["Admin"]["id"], 11);
    for variant in ["default", "mobile"] {
        let checked = cli(
            directory.path(),
            &["check", "new.acore", "--variant", variant],
            true,
        );
        success(&checked);
    }
    let repeated = cli(
        directory.path(),
        &["migrate", "old.acore", "--output", "new.acore"],
        true,
    );
    assert!(!repeated.status.success());
    assert_eq!(
        std::fs::read_to_string(directory.path().join("new.acore")).unwrap(),
        new
    );
}

#[test]
fn unsupported_migration_never_writes_partial_source_or_identity_files() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("legacy.acore"),
        "local secret = 5\nanswer = secret * 2",
    )
    .unwrap();
    let migrated = cli(
        directory.path(),
        &["migrate", "legacy.acore", "--output", "new.acore"],
        true,
    );
    assert!(!migrated.status.success());
    assert!(!directory.path().join("new.acore").exists());
    assert!(!directory.path().join("new.acore.identities.json").exists());
}

#[tokio::test]
async fn backend_profile_mock_and_contract_tests_share_variant_and_response_semantics() {
    use std::io::Read;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    struct Server(std::process::Child);
    impl Drop for Server {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let directory = tempfile::tempdir().unwrap();
    let source = r#"profile backend 1
project(id: "backend-http-regression", version: "1.0.0")
variant default(include: ["*"])
variant mobile(include: ["hello"])
endpoint hello(method: GET, path: "/hello") {
  response(type: Json)
  mock(responses: [MockResponse(data: StaticData(body: {message: "hello"}))])
}
endpoint privateRoute(method: GET, path: "/private") {
  response(type: Void)
}
testConfig(suites: [TestSuite(
  name: "live mock",
  baseUrl: "http://127.0.0.1:$PORT",
  checks: [CheckDef(route: "hello", expect: [StatusExpectation(value: 200)])]
)])
"#
    .replace("$PORT", &port.to_string());
    std::fs::write(directory.path().join("axiom.acore"), source).unwrap();
    let mut server = Server(
        Command::new(env!("CARGO_BIN_EXE_axiom-cli"))
            .current_dir(directory.path())
            .args([
                "serve",
                "axiom.acore",
                "--variant",
                "mobile",
                "--port",
                &port.to_string(),
            ])
            .env("CI", "true")
            .env_remove("AXIOM_CLOUD_URL")
            .env_remove("AXIOM_REFERRAL_CODE")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(400))
        .build()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let response = loop {
        if let Some(status) = server.0.try_wait().unwrap() {
            let mut stderr = String::new();
            server
                .0
                .stderr
                .take()
                .unwrap()
                .read_to_string(&mut stderr)
                .unwrap();
            panic!("mock server exited {status}: {stderr}");
        }
        if let Ok(response) = client
            .get(format!("http://127.0.0.1:{port}/hello"))
            .send()
            .await
        {
            break response;
        }
        assert!(
            Instant::now() < deadline,
            "mock server did not become ready"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap()["message"],
        "hello"
    );
    assert_eq!(
        client
            .get(format!("http://127.0.0.1:{port}/private"))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    let tests = cli(directory.path(), &["test", "--variant", "mobile"], true);
    success(&tests);
    assert!(String::from_utf8_lossy(&tests.stdout).contains("1 passed, 0 failed"));
}
