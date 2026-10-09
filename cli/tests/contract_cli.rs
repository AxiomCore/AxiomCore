use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Command, Output, Stdio},
    time::Duration,
};

const BACKEND: &str = "profile backend 1\nproject(id: \"unified-cli\", version: \"1.0.0\")\nvariant default(include: [\"*\"])\nendpoint health(method: GET, path: \"/health\") { response(type: Void) }\n";
const FRONTEND: &str = "module acceptance.ui\napp Demo { route \"/\" => Home }\npage Home { view { Text(\"Hello\") } }\n";
const DATABASE: &str = "profile database 1\nmodule storage\ndatabase storage(engine: DatabaseEngine.postgres) { manage(schemas: [Schemas.app], mode: ManagementMode.ownedObjects) schema app { table items { id: Pg.Int64() primaryKey(columns: [.id]) } } }\n";

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_axiom-cli"))
        .current_dir(root)
        .args(args)
        .env("CI", "true")
        .env("AXIOM_CLOUD_URL", "invalid-cloud-endpoint")
        .env_remove("AXIOM_REFERRAL_CODE")
        .output()
        .unwrap()
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
fn project(root: &Path, kind: &str, text: &str) -> std::path::PathBuf {
    let directory = root.join(format!("{kind} project"));
    std::fs::create_dir_all(directory.join("src")).unwrap();
    let manifest = if kind == "database" {
        "type = \"database\"\ntargets = [\"server\"]\n[application]\nname = \"example.storage\"\nversion = \"1.0.0\"\n".to_string()
    } else if kind == "frontend" {
        "type = \"frontend\"\ntargets = [\"ios\", \"web\"]\n".to_string()
    } else {
        "type = \"backend\"\n".to_string()
    };
    std::fs::write(directory.join("AxiomDeps.toml"), manifest).unwrap();
    let source = directory.join("src/main.acore");
    std::fs::write(&source, text).unwrap();
    source
}

#[test]
fn all_contracts_check_without_outputs_and_build_beside_the_input() {
    let root = tempfile::tempdir().unwrap();
    for (kind, text) in [
        ("backend", BACKEND),
        ("frontend", FRONTEND),
        ("database", DATABASE),
    ] {
        let source = project(root.path(), kind, text);
        let file = source.to_str().unwrap();
        let checked = run(root.path(), &["check", file, "--json"]);
        success(&checked);
        let report: serde_json::Value = serde_json::from_slice(&checked.stdout).unwrap();
        assert_eq!(report["valid"], true);
        assert_eq!(report["profile"], kind);
        assert!(!source.with_extension("acore.lockfile").exists());
        assert!(!source
            .parent()
            .unwrap()
            .join(format!("{kind}.axiom"))
            .exists());
        assert!(!source
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".axiom")
            .exists());
        success(&run(root.path(), &["build", file]));
        let output = source.parent().unwrap().join(format!("{kind}.axiom"));
        let before = std::fs::read(&output).unwrap();
        assert!(!before.is_empty());
        assert!(!root.path().join(format!("{kind}.axiom")).exists());
        assert!(!source
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".axiom")
            .exists());
        match kind {
            "backend" => {
                axiom_lib::unpackager::unpack_axiom_bytes(&before).unwrap();
            }
            "database" => {
                let axiom_lib::package::DecodedAxiomPackage::Package(package) =
                    axiom_lib::package::decode_axiom_package(&before).unwrap()
                else {
                    panic!("expected database package")
                };
                axiom_lib::database::contract_from_package(&package).unwrap();
                success(&run(root.path(), &["inspect", output.to_str().unwrap()]));
            }
            _ => {
                let artifact: serde_json::Value = serde_json::from_slice(&before).unwrap();
                assert_eq!(artifact["format"], "axiom-frontend-contract/v1");
                assert_eq!(artifact["targets"].as_array().unwrap().len(), 2);
                assert_eq!(artifact["targets"][0]["target"], "ios");
                success(&run(root.path(), &["inspect", output.to_str().unwrap()]));
                success(&run(root.path(), &["build", file]));
                assert_eq!(
                    std::fs::read(&output).unwrap(),
                    before,
                    "frontend output must be deterministic"
                );
            }
        }
        // A failed compilation must preserve the previous valid artifact.
        std::fs::write(&source, "invalid source ???").unwrap();
        let failed = run(root.path(), &["check", file, "--json"]);
        assert!(!failed.status.success());
        let report: serde_json::Value = serde_json::from_slice(&failed.stdout).unwrap();
        assert_eq!(report["valid"], false);
        assert!(!report["diagnostics"].as_array().unwrap().is_empty());
        assert!(!run(root.path(), &["build", file]).status.success());
        assert_eq!(std::fs::read(output).unwrap(), before);
    }
}

#[test]
fn output_overrides_targets_and_profile_specific_flags_are_enforced() {
    let root = tempfile::tempdir().unwrap();
    for (kind, text) in [
        ("backend", BACKEND),
        ("frontend", FRONTEND),
        ("database", DATABASE),
    ] {
        let source = project(root.path(), kind, text);
        let custom = format!("custom/{kind}.axiom");
        success(&run(
            root.path(),
            &["build", source.to_str().unwrap(), "--out", &custom],
        ));
        assert!(root.path().join(&custom).is_file());
        assert!(!source
            .parent()
            .unwrap()
            .join(format!("{kind}.axiom"))
            .exists());
        assert!(!run(
            root.path(),
            &["build", source.to_str().unwrap(), "--out", "bad.acore"]
        )
        .status
        .success());
        assert!(!root.path().join("bad.acore").exists());
        if kind != "backend" {
            let rejected_release = run(root.path(), &["release", &custom]);
            assert!(!rejected_release.status.success());
            assert!(String::from_utf8_lossy(&rejected_release.stderr)
                .contains("compiled backend contract"));
            assert!(!String::from_utf8_lossy(&rejected_release.stderr).contains("AXIOM_CLOUD_URL"));
            let failed = run(
                root.path(),
                &["build", source.to_str().unwrap(), "--release"],
            );
            assert!(!failed.status.success());
            assert!(String::from_utf8_lossy(&failed.stderr).contains("backend-only"));
            assert!(!String::from_utf8_lossy(&failed.stderr).contains("AXIOM_CLOUD_URL"));
            assert!(!run(
                root.path(),
                &["check", source.to_str().unwrap(), "--variant", "mobile"]
            )
            .status
            .success());
        }
        if kind == "frontend" {
            success(&run(
                root.path(),
                &[
                    "build",
                    source.to_str().unwrap(),
                    "--target",
                    "web",
                    "--out",
                    "web.axiom",
                ],
            ));
            let artifact: serde_json::Value =
                serde_json::from_slice(&std::fs::read(root.path().join("web.axiom")).unwrap())
                    .unwrap();
            assert_eq!(artifact["targets"].as_array().unwrap().len(), 1);
            assert_eq!(artifact["targets"][0]["target"], "web");
        } else {
            assert!(!run(
                root.path(),
                &["check", source.to_str().unwrap(), "--target", "web"]
            )
            .status
            .success());
        }
    }
    let database = root.path().join("database project/src/main.acore");
    success(&run(
        root.path(),
        &["database", "build", database.to_str().unwrap()],
    ));
    assert!(database.parent().unwrap().join("database.axiom").is_file());
    assert!(!root.path().join("database project/.axiom").exists());
}

#[test]
fn typed_manifest_errors_and_legacy_sources_report_without_project_writes() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("main.acore"), FRONTEND).unwrap();
    success(&run(root.path(), &["check", "main.acore", "--json"]));
    std::fs::write(
        root.path().join("AxiomDeps.toml"),
        "type = \"frontend\"\nunknownField = true\n",
    )
    .unwrap();
    let result = run(root.path(), &["check", "main.acore", "--json"]);
    assert!(!result.status.success());
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["valid"], false);
    assert!(!root.path().join("frontend.axiom").exists());
    let missing = run(root.path(), &["check", "missing.acore", "--json"]);
    assert!(!missing.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&missing.stdout).unwrap()["valid"],
        false
    );
}

#[test]
fn backend_companion_plan_and_baseline_follow_custom_output_without_stale_plans() {
    let root = tempfile::tempdir().unwrap();
    let text = "project(id: \"unified-cli\", version: \"1.0.0\")\nvariant default(include: [\"*\"])\naction health(kind: ActionKind.query, output: String) { managed { result(value: \"ready\") } }\nendpoint health(method: GET, path: \"/health\") { handle(action: Actions.health) response(type: String) }\n";
    let source = project(root.path(), "backend", text);
    let args = [
        "build",
        source.to_str().unwrap(),
        "--out",
        "custom/api.axiom",
    ];
    success(&run(root.path(), &args));
    let artifact = root.path().join("custom/api.axiom");
    let plan = root.path().join("custom/api.server.json");
    let baseline = source.with_extension("acore.lockfile");
    let preserved = [&artifact, &plan, &baseline].map(|p| std::fs::read(p).unwrap());
    success(&run(
        root.path(),
        &[
            "server",
            "inspect",
            "custom/api.axiom",
            "custom/api.server.json",
        ],
    ));
    std::fs::write(&source, "invalid source").unwrap();
    assert!(!run(root.path(), &["check", source.to_str().unwrap()])
        .status
        .success());
    assert!(!run(root.path(), &args).status.success());
    assert_eq!(
        [&artifact, &plan, &baseline].map(|p| std::fs::read(p).unwrap()),
        preserved
    );
    std::fs::write(&source, BACKEND).unwrap();
    success(&run(root.path(), &args));
    assert!(
        !plan.exists(),
        "removing source-owned actions must remove the generated private plan"
    );
}

#[test]
fn frontend_configuration_targets_and_local_imports_use_the_owning_project() {
    let root = tempfile::tempdir().unwrap();
    let text = "module acceptance.ui\nuse function greeting from \"./helper.acore\"\npage Home {\n derived message = greeting(\"Hello\")\n view { Text(message) }\n}\n";
    let source = project(root.path(), "frontend", text);
    std::fs::write(
        source.parent().unwrap().join("helper.acore"),
        "module helper.ui\npure fn greeting(value: String): String = value.trim()\n",
    )
    .unwrap();
    let configuration = source
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("AcoreUI.json");
    std::fs::write(
        &configuration,
        r#"{"profile":"acore-ui/v6","entry":"src/main.acore","targets":["web"]}"#,
    )
    .unwrap();
    success(&run(
        root.path(),
        &["check", source.to_str().unwrap(), "--json"],
    ));
    success(&run(
        root.path(),
        &["build", source.to_str().unwrap(), "--out", "frontend.axiom"],
    ));
    let bytes = std::fs::read(root.path().join("frontend.axiom")).unwrap();
    let contract: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(contract["targets"].as_array().unwrap().len(), 1);
    assert_eq!(contract["targets"][0]["target"], "web");
    assert!(!source
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(".axiom")
        .exists());
    std::fs::write(
        &configuration,
        r#"{"profile":"unsupported","entry":"src/main.acore","targets":["web"]}"#,
    )
    .unwrap();
    assert!(!run(
        root.path(),
        &["build", source.to_str().unwrap(), "--out", "frontend.axiom"]
    )
    .status
    .success());
    assert_eq!(
        std::fs::read(root.path().join("frontend.axiom")).unwrap(),
        bytes
    );
}

#[cfg(unix)]
#[test]
fn outputs_and_installation_do_not_follow_symlinks() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let source = project(root.path(), "frontend", FRONTEND);
    let target = root.path().join("protected.axiom");
    std::fs::write(&target, "keep").unwrap();
    symlink(&target, root.path().join("linked.axiom")).unwrap();
    assert!(!run(
        root.path(),
        &["build", source.to_str().unwrap(), "--out", "linked.axiom"]
    )
    .status
    .success());
    assert_eq!(std::fs::read_to_string(target).unwrap(), "keep");
    let home = root.path().join("home");
    std::fs::create_dir(&home).unwrap();
    symlink(root.path(), home.join(".axiom")).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_axiom-cli"))
        .args(["install", "lsp-server"])
        .env("HOME", home)
        .env("AXIOM_CLOUD_URL", "invalid")
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("symlink"));
}

fn frame(stdin: &mut impl Write, value: serde_json::Value) {
    let bytes = serde_json::to_vec(&value).unwrap();
    write!(stdin, "Content-Length: {}\r\n\r\n", bytes.len()).unwrap();
    stdin.write_all(&bytes).unwrap();
    stdin.flush().unwrap();
}

#[test]
fn installed_server_is_self_contained_compatible_and_speaks_stdio_lsp() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home with spaces");
    std::fs::create_dir(&home).unwrap();
    let installed = || {
        Command::new(env!("CARGO_BIN_EXE_axiom-cli"))
            .args(["install", "lsp-server"])
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("AXIOM_CLOUD_URL", "invalid")
            .output()
            .unwrap()
    };
    success(&installed());
    let server = home.join(".axiom/bin").join(if cfg!(windows) {
        "acore-lsp.exe"
    } else {
        "acore-lsp"
    });
    assert!(!server.is_symlink());
    let before = std::fs::metadata(&server).unwrap().modified().unwrap();
    success(&installed());
    assert_eq!(
        std::fs::metadata(&server).unwrap().modified().unwrap(),
        before
    );
    let native = Command::new(&server)
        .arg("--version-json")
        .env("AXIOM_CLOUD_URL", "invalid")
        .output()
        .unwrap();
    success(&native);
    let direct = run(root.path(), &["lsp", "--version-json"]);
    success(&direct);
    let metadata: serde_json::Value = serde_json::from_slice(&native.stdout).unwrap();
    assert_eq!(
        metadata,
        serde_json::from_slice::<serde_json::Value>(&direct.stdout).unwrap()
    );
    assert_eq!(metadata["protocolVersion"], "axiom-editor/v1");
    assert!(metadata["serverVersion"]
        .as_str()
        .unwrap()
        .starts_with("acore/"));
    assert_eq!(metadata["compilerVersion"].as_str().unwrap().len(), 64);
    assert_eq!(
        metadata["editorFeatures"]["virtualDocumentNavigationOptOut"],
        true
    );
    assert_eq!(native.stdout, run(root.path(), &["--version-json"]).stdout);
    let mut child = Command::new(&server)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env("AXIOM_CLOUD_URL", "invalid")
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || loop {
        let mut length = None;
        loop {
            let mut line = String::new();
            if stdout.read_line(&mut line).unwrap() == 0 {
                return;
            }
            if line == "\r\n" {
                break;
            }
            if let Some(n) = line.strip_prefix("Content-Length: ") {
                length = Some(n.trim().parse::<usize>().unwrap());
            }
        }
        let mut bytes = vec![0; length.expect("stdout must contain only framed LSP")];
        stdout.read_exact(&mut bytes).unwrap();
        tx.send(serde_json::from_slice::<serde_json::Value>(&bytes).unwrap())
            .unwrap();
    });
    frame(
        &mut stdin,
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":null,"capabilities":{}}}),
    );
    let initialized = rx
        .recv_timeout(Duration::from_secs(20))
        .unwrap_or_else(|e| {
            let _ = child.kill();
            panic!("initialize: {e}")
        });
    assert_eq!(initialized["id"], 1);
    assert_eq!(initialized["result"]["capabilities"]["hoverProvider"], true);
    frame(
        &mut stdin,
        serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
    );
    frame(
        &mut stdin,
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"shutdown"}),
    );
    loop {
        let response = rx
            .recv_timeout(Duration::from_secs(20))
            .unwrap_or_else(|e| {
                let _ = child.kill();
                panic!("shutdown: {e}")
            });
        if response["id"] == 2 {
            assert!(response.get("error").is_none(), "{response}");
            break;
        }
    }
    frame(
        &mut stdin,
        serde_json::json!({"jsonrpc":"2.0","method":"exit"}),
    );
    drop(stdin);
    assert!(child.wait().unwrap().success());
    reader.join().unwrap();
}

#[test]
fn amended_backend_without_variants_uses_implicit_default() {
    let root = tempfile::tempdir().unwrap();
    let source = project(root.path(), "backend", r#"amends "models.acore"
endpoint listTasks(method: GET, path: "/tasks") { response(type: Entities.task) }
"#);
    std::fs::write(source.parent().unwrap().join("models.acore"),
        r#"project(id: "zed.example.backend", version: "1.0.0")
model Task { id: String title: String completed: Bool } also entity
"#).unwrap();
    let file = source.to_str().unwrap();
    success(&run(root.path(), &["check", file]));
    let artifact = source.parent().unwrap().join("backend.axiom");
    assert!(!artifact.exists());
    success(&run(root.path(), &["build", file]));
    let before = std::fs::read(&artifact).unwrap();
    let compiled = axiom_lib::unpackager::unpack_axiom_bytes(&before).unwrap();
    assert!(compiled.endpoints.iter().any(|endpoint| endpoint.name == "listTasks"));
    for command in ["check", "build"] {
        let failed = run(root.path(), &[command, file, "--variant", "missing"]);
        assert!(!failed.status.success());
        assert!(String::from_utf8_lossy(&failed.stderr).contains("Variant 'missing' not found"));
        assert_eq!(std::fs::read(&artifact).unwrap(), before);
    }
}
