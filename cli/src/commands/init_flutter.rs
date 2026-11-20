//! `axiom init --template flutter <dir>`: a backend contract plus a Flutter
//! app wired to it, with the generated client and platform settings in place.

use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const T_DEPS: &str = include_str!("../../templates/flutter/AxiomDeps.toml");
const T_ACORE: &str = include_str!("../../templates/flutter/main.acore");
const T_RUN: &str = include_str!("../../templates/flutter/run-backend.sh");
const T_BACKEND_GITIGNORE: &str = include_str!("../../templates/flutter/backend.gitignore");
const T_MAIN_DART: &str = include_str!("../../templates/flutter/main.dart");
const T_README: &str = include_str!("../../templates/flutter/README.md");

const PORT: &str = "8000";

struct Names {
    project: String,
    package: String,
    namespace: String,
    project_id: String,
}

fn names(dir: &Path) -> Result<Names> {
    let base = dir
        .file_name()
        .and_then(|n| n.to_str())
        .context("choose a directory with a name, for example `axiom init --template flutter my_app`")?;
    let mut package: String = base
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    package = package.trim_matches('_').to_string();
    while package.contains("__") {
        package = package.replace("__", "_");
    }
    if package.is_empty() || package.starts_with(|c: char| c.is_ascii_digit()) {
        package = format!("app_{package}");
    }
    let namespace = "api".to_string();
    Ok(Names {
        project: base.to_string(),
        project_id: format!("dev.example.{}", package.replace('_', "-")),
        package,
        namespace,
    })
}

fn render(template: &str, n: &Names) -> String {
    template
        .replace("{{project}}", &n.project)
        .replace("{{project_id}}", &n.project_id)
        .replace("{{package}}", &n.package)
        .replace("{{namespace}}", &n.namespace)
        .replace("{{port}}", PORT)
}

fn run(step: &str, cwd: &Path, program: &Path, args: &[&str]) -> Result<()> {
    println!("▶ {step}: {} {}", program.display(), args.join(" "));
    let status = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .status()
        .with_context(|| format!("could not start {}", program.display()))?;
    if !status.success() {
        bail!(
            "{step} failed (exit code {}). Command: {} {} in {}",
            status.code().map(|c| c.to_string()).unwrap_or_else(|| "signal".into()),
            program.display(),
            args.join(" "),
            cwd.display()
        );
    }
    Ok(())
}

fn flutter_command(root: &Path) -> Result<(PathBuf, Vec<String>)> {
    let fvm = root.join(".fvmrc").exists() && axiom_build::core::utils::find_on_path("fvm").is_some();
    if fvm {
        return Ok((PathBuf::from("fvm"), vec!["flutter".into()]));
    }
    let ok = Command::new("flutter")
        .arg("--version")
        .env("FLUTTER_SUPPRESS_ANALYTICS", "true")
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains("Flutter "))
        .unwrap_or(false);
    if !ok {
        bail!(
            "`flutter` is missing or not a working Flutter SDK. Install Flutter (https://docs.flutter.dev/get-started/install), put <flutter>/bin on PATH, and run `axiom doctor`."
        );
    }
    Ok((PathBuf::from("flutter"), vec![]))
}

pub async fn scaffold(dir: &Path) -> Result<()> {
    let root = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        std::env::current_dir()?.join(dir)
    };
    if root.exists() && fs::read_dir(&root)?.next().is_some() {
        bail!("{} exists and is not empty; choose a new directory", root.display());
    }
    let n = names(&root)?;
    let (flutter, flutter_prefix) = flutter_command(&root)?;
    let axiom = std::env::current_exe().context("locate the running axiom executable")?;

    // 1. Backend.
    let backend = root.join("backend");
    fs::create_dir_all(&backend)?;
    fs::write(backend.join("AxiomDeps.toml"), T_DEPS)?;
    fs::write(backend.join("main.acore"), render(T_ACORE, &n))?;
    fs::write(backend.join(".gitignore"), T_BACKEND_GITIGNORE)?;
    let script = backend.join("run-backend.sh");
    fs::write(&script, render(T_RUN, &n))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755))?;
    }
    run("Check backend contract", &backend, &axiom, &["check", "main.acore"])?;
    run(
        "Build backend contract",
        &backend,
        &axiom,
        &["build", "main.acore", "--out", "backend.axiom"],
    )?;

    // 2. Flutter app.
    let mut create_args: Vec<String> = flutter_prefix.clone();
    create_args.extend(
        [
            "create",
            "--no-pub",
            "--project-name",
            &n.package,
            "--org",
            "dev.example",
            "--platforms",
            "android,ios,macos,web",
            "app",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    let refs: Vec<&str> = create_args.iter().map(String::as_str).collect();
    run("Create Flutter app", &root, &flutter, &refs)?;
    let app = root.join("app");
    android_settings(&app)?;

    // 3. Generated client, with --fix for deployment targets, entitlements
    // and the INTERNET permission.
    let base_url = format!("http://localhost:{PORT}");
    run(
        "Generate Dart client",
        &app,
        &axiom,
        &[
            "pull",
            "../backend/backend.axiom",
            "--framework",
            "flutter",
            "--name",
            &n.namespace,
            "--out",
            "lib/axiom_generated",
            "--base-url",
            &base_url,
            "--fix",
        ],
    )?;

    // 4. App code, README, tests that match the new main.dart.
    fs::write(app.join("lib/main.dart"), render(T_MAIN_DART, &n))?;
    let widget_test = app.join("test/widget_test.dart");
    if widget_test.exists() {
        // The counter test from `flutter create` does not apply to this app.
        fs::write(
            &widget_test,
            "import 'package:flutter_test/flutter_test.dart';\n\nvoid main() {\n  test('placeholder', () => expect(1 + 1, 2));\n}\n",
        )?;
    }
    fs::write(root.join("README.md"), render(T_README, &n))?;
    fs::write(root.join(".gitignore"), ".DS_Store\n")?;

    println!("\n✅ Created {}", root.display());
    println!("\nNext:");
    println!("  ./{}/backend/run-backend.sh", dir.display());
    println!("  cd {}/app && flutter run", dir.display());
    println!("  (cd {}/app && axiom pull --watch)   # while editing backend/main.acore", dir.display());
    Ok(())
}

/// minSdk 24 for the Axiom Android runtime and cleartext http for debug builds
/// (the emulator reaches a local backend on http://10.0.2.2).
fn android_settings(app: &Path) -> Result<()> {
    for name in ["build.gradle.kts", "build.gradle"] {
        let gradle = app.join("android/app").join(name);
        if let Ok(text) = fs::read_to_string(&gradle) {
            let updated = text
                .replace("minSdk = flutter.minSdkVersion", "minSdk = 24")
                .replace("minSdkVersion flutter.minSdkVersion", "minSdkVersion 24");
            fs::write(&gradle, updated)?;
        }
    }
    let debug = app.join("android/app/src/debug/AndroidManifest.xml");
    if let Ok(text) = fs::read_to_string(&debug) {
        if !text.contains("usesCleartextTraffic") {
            if let Some(at) = text.rfind("</manifest>") {
                let updated = format!(
                    "{}    <!-- Local development only: allow http to the host (10.0.2.2). -->\n    <application android:usesCleartextTraffic=\"true\"/>\n{}",
                    &text[..at],
                    &text[at..]
                );
                fs::write(&debug, updated)?;
            }
        }
    }
    Ok(())
}
