//! `axiom server package` for containers without `--runtime-source` (published builder
//! image) and `--output-input` (DEPLOYMENTS-DESIGN §10). Offline; no Docker.
use std::path::Path;
use std::process::{Command, Output};

const SOURCE: &str = r#"project(id: "docs.hello", version: "1.0.0")
variant default(include: ["*"])
model Greeting { message: String }
action hello(kind: ActionKind.query, output: Models.Greeting) {
  managed { return {message: "Hello from Acore"} }
}
endpoint hello(method: GET, path: "/hello") {
  handle(action: Actions.hello)
  response(type: Models.Greeting)
}
"#;
const DEPS: &str = "type = \"backend\"\ntargets = [\"server\"]\naudiences = [\"web\"]\n";
const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn cli(directory: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_axiom-cli"))
        .current_dir(directory)
        .args(arguments)
        .env("HOME", directory.join("home"))
        .env_remove("AXIOM_CLOUD_URL")
        .env_remove("AXIOM_REFERRAL_CODE")
        .env("CI", "true")
        .output()
        .unwrap()
}
fn ok(output: &Output) -> String {
    assert!(output.status.success(), "stdout: {}\nstderr: {}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout.clone()).unwrap()
}
fn refused(output: &Output) -> String {
    assert!(!output.status.success(), "unexpected success: {}", String::from_utf8_lossy(&output.stdout));
    String::from_utf8_lossy(&output.stderr).into_owned()
}
fn files(root: &Path) -> Vec<String> {
    let mut found = vec![];
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                pending.push(p);
            } else {
                found.push(p.strip_prefix(root).unwrap().to_str().unwrap().to_owned());
            }
        }
    }
    found.sort();
    found
}

#[test]
fn container_packaging_needs_no_runtime_source() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    std::fs::create_dir(d.join("home")).unwrap();
    std::fs::write(d.join("main.acore"), SOURCE).unwrap();
    std::fs::write(d.join("AxiomDeps.toml"), DEPS).unwrap();
    ok(&cli(d, &["build", "main.acore", "--out", "backend.axiom"]));
    ok(&cli(d, &["server", "deployment", "backend.axiom", "backend.server.json", "--output", "deployment.json"]));
    let base = ["server", "package", "backend.axiom", "backend.server.json", "--deployment", "deployment.json", "--target", "container-amd64"];
    let package = |extra: &[&str]| cli(d, &[&base[..], extra].concat());

    // This build pins no published builder yet: refused with the remedies, nothing written.
    let error = refused(&package(&["--output", "unpinned"]));
    assert!(error.contains("AXPACKAGE_BUILDER_UNPINNED") && error.contains("--builder-image") && error.contains("--runtime-source"), "{error}");
    assert!(!d.join("unpinned").exists());
    // A tag, or anything but one digest-pinned reference, is refused.
    for bad in ["ghcr.io/axiomcore/axiom-server-host:0.1.0", &format!("ghcr.io/a/b@sha256:{DIGEST}\nRUN id")] {
        let error = refused(&package(&["--builder-image", bad, "--output", "tagged"]));
        assert!(error.contains("AXPACKAGE_BUILDER_REFERENCE"), "{error}");
    }

    // A digest-pinned builder: the context is input/ plus a Dockerfile FROM that image.
    let image = format!("ghcr.io/axiomcore/axiom-server-host@sha256:{DIGEST}");
    let out: serde_json::Value = serde_json::from_str(&ok(&package(&["--builder-image", &image, "--output", "context"]))).unwrap();
    assert_eq!(out["builder"]["image"], image.as_str());
    assert_eq!(out["builder"]["pinnedBy"], "--builder-image");
    assert_eq!(out["metadata"]["runtimeBinary"], "published-builder-image");
    let context = files(&d.join("context"));
    assert_eq!(context.iter().filter(|n| !n.starts_with("input/")).collect::<Vec<_>>(), [".dockerignore", "Dockerfile", "compose.yaml", "container.json"]);
    assert!(context.contains(&"input/input.json".to_owned()));
    let dockerfile = std::fs::read_to_string(d.join("context/Dockerfile")).unwrap();
    assert!(dockerfile.lines().any(|l| l == format!("FROM --platform=linux/amd64 {image} AS build")), "{dockerfile}");
    assert!(!dockerfile.contains("COPY source"));
    // Immutable output.
    assert!(refused(&package(&["--builder-image", &image, "--output", "context"])).contains("immutable"));

    // --output-input: the same input, alone.
    let out: serde_json::Value = serde_json::from_str(&ok(&package(&["--output-input", "input"]))).unwrap();
    assert_eq!(out["metadata"]["format"], "axiom-container-input/v1");
    let input = files(&d.join("input"));
    assert_eq!(input, context.iter().filter_map(|n| n.strip_prefix("input/").map(str::to_owned)).collect::<Vec<_>>());
    for name in &input {
        assert_eq!(std::fs::read(d.join("input").join(name)).unwrap(), std::fs::read(d.join("context/input").join(name)).unwrap(), "{name}");
    }

    // Flag combinations.
    assert!(refused(&package(&["--output-input", "a", "--output", "b"])).contains("cannot be used with"));
    assert!(refused(&package(&["--output-input", "a", "--runtime-source", "."])).contains("cannot be used with"));
    assert!(refused(&package(&["--builder-image", &image, "--runtime-source", ".", "--output", "b"])).contains("cannot be used with"));
    assert!(refused(&package(&[])).contains("--output"));
    let native = ["server", "package", "backend.axiom", "backend.server.json", "--deployment", "deployment.json", "--target", "native-server", "--output-input", "n"];
    assert!(refused(&cli(d, &native)).contains("AXPACKAGE_CONTAINER_ONLY"));
}
