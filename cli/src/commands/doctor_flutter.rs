//! Flutter section of `axiom doctor`: real `flutter`/`dart` binaries and
//! versions (through fvm when the project pins one), CocoaPods on macOS, the
//! Android SDK, saved pub credentials that need an environment variable, and
//! the same project settings `axiom pull` checks.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::doctor::{DoctorCheck, DoctorStatus};

fn check(id: &'static str, status: DoctorStatus, summary: String, detail: String, fix: Option<String>) -> DoctorCheck {
    DoctorCheck {
        id,
        status,
        summary,
        detail,
        fix,
    }
}

/// Whether the Flutter section applies to this directory.
pub fn applies(root: &Path, flutter_framework: bool) -> bool {
    flutter_framework || root.join("pubspec.yaml").exists()
}

fn uses_fvm(root: &Path) -> bool {
    root.join(".fvmrc").exists() || root.join(".fvm").exists()
}

enum Probe {
    Ok(String),
    Missing,
    NotReal(String),
}

fn probe(tool: &str, marker: &str, fvm: bool) -> Probe {
    let mut command = if fvm {
        let mut c = Command::new("fvm");
        c.arg(tool);
        c
    } else {
        Command::new(tool)
    };
    command.arg("--version").env("FLUTTER_SUPPRESS_ANALYTICS", "true");
    match command.output() {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Probe::Missing,
        Err(error) => Probe::NotReal(error.to_string()),
        Ok(output) => {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            match text.lines().find(|l| l.contains(marker)) {
                Some(line) if output.status.success() => Probe::Ok(line.trim().to_string()),
                _ => Probe::NotReal(text.trim().lines().take(4).collect::<Vec<_>>().join(" | ")),
            }
        }
    }
}

fn which(tool: &str) -> Option<PathBuf> {
    axiom_build::core::utils::find_on_path(tool)
}

fn tool_check(id: &'static str, tool: &str, marker: &str, fvm: bool) -> DoctorCheck {
    let runner = if fvm { format!("fvm {tool}") } else { tool.to_string() };
    let install = "Install Flutter (https://docs.flutter.dev/get-started/install) and put <flutter>/bin on PATH (it contains both `flutter` and `dart`).";
    match probe(tool, marker, fvm) {
        Probe::Ok(version) => {
            let location = if fvm {
                "through fvm".to_string()
            } else {
                which(tool)
                    .and_then(|p| std::fs::canonicalize(p).ok())
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            };
            check(id, DoctorStatus::Pass, format!("{runner}: {version}"), location, None)
        }
        Probe::Missing => check(
            id,
            DoctorStatus::Fail,
            format!("`{runner}` was not found"),
            if fvm {
                "This project pins its SDK with fvm (.fvmrc or .fvm/).".into()
            } else {
                "Looked on PATH.".into()
            },
            Some(if fvm {
                "Install fvm (https://fvm.app) and run `fvm install` in the project.".into()
            } else {
                install.into()
            }),
        ),
        Probe::NotReal(output) => check(
            id,
            DoctorStatus::Fail,
            format!("`{runner}` is not a working {} SDK binary", if tool == "dart" { "Dart" } else { "Flutter" }),
            format!(
                "{} printed: {}",
                which(tool).map(|p| p.display().to_string()).unwrap_or_else(|| runner.clone()),
                if output.is_empty() { "(nothing)".into() } else { output }
            ),
            Some(format!(
                "Usually a version-manager shim with no SDK selected (asdf/mise/fvm) or another program named `{tool}` earlier on PATH. {install}"
            )),
        ),
    }
}

fn cocoapods_check() -> DoctorCheck {
    match Command::new("pod").arg("--version").output() {
        Ok(output) if output.status.success() => check(
            "flutter.cocoapods",
            DoctorStatus::Pass,
            format!("CocoaPods {}", String::from_utf8_lossy(&output.stdout).trim()),
            "Needed for the iOS and macOS Axiom runtime pod.".into(),
            None,
        ),
        _ => check(
            "flutter.cocoapods",
            DoctorStatus::Fail,
            "CocoaPods is missing; iOS and macOS builds cannot fetch the Axiom runtime".into(),
            "Looked for `pod` on PATH.".into(),
            Some("`brew install cocoapods` (or `sudo gem install cocoapods`), then `pod setup`.".into()),
        ),
    }
}

fn android_sdk_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
        .iter()
        .filter_map(|v| std::env::var_os(v))
        .map(PathBuf::from)
        .collect();
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join("Android/Sdk"));
        dirs.push(home.join("Library/Android/sdk"));
        dirs.push(home.join("AppData/Local/Android/Sdk"));
    }
    dirs
}

fn android_check() -> DoctorCheck {
    let found = android_sdk_dirs().into_iter().find(|d| d.is_dir());
    match found {
        Some(dir) => {
            let platforms = std::fs::read_dir(dir.join("platforms"))
                .map(|r| {
                    let mut v: Vec<String> = r.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
                    v.sort();
                    v
                })
                .unwrap_or_default();
            let adb = dir.join("platform-tools").join("adb").exists() || dir.join("platform-tools").join("adb.exe").exists();
            let status = if platforms.is_empty() { DoctorStatus::Warn } else { DoctorStatus::Pass };
            check(
                "flutter.android_sdk",
                status,
                format!("Android SDK at {}", dir.display()),
                format!(
                    "platforms: {}; adb: {}",
                    if platforms.is_empty() { "none".to_string() } else { platforms.join(", ") },
                    if adb { "yes" } else { "no" }
                ),
                if platforms.is_empty() {
                    Some("Install an Android platform with `sdkmanager \"platforms;android-35\"` or Android Studio.".into())
                } else {
                    None
                },
            )
        }
        None => check(
            "flutter.android_sdk",
            DoctorStatus::Warn,
            "No Android SDK found; Android builds are unavailable".into(),
            "Looked at $ANDROID_HOME, $ANDROID_SDK_ROOT, ~/Android/Sdk and ~/Library/Android/sdk.".into(),
            Some("Install Android Studio or the command-line tools and set ANDROID_HOME (not needed for iOS, macOS or web).".into()),
        ),
    }
}

fn pub_tokens_path() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        let p = PathBuf::from(dir).join("dart/pub-tokens.json");
        if p.exists() {
            return Some(p);
        }
    }
    let home = dirs::home_dir()?;
    [
        home.join(".config/dart/pub-tokens.json"),
        home.join("Library/Application Support/dart/pub-tokens.json"),
        home.join("AppData/Roaming/dart/pub-tokens.json"),
    ]
    .into_iter()
    .find(|p| p.exists())
}

/// Saved pub tokens that read an environment variable which is unset make
/// every `pub get` against that host fail with an authentication error.
fn pub_credentials_check() -> DoctorCheck {
    let Some(path) = pub_tokens_path() else {
        return check(
            "flutter.pub_credentials",
            DoctorStatus::Pass,
            "No saved pub tokens".into(),
            "pub.dev packages need no credentials.".into(),
            None,
        );
    };
    let parsed = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    let Some(value) = parsed else {
        return check(
            "flutter.pub_credentials",
            DoctorStatus::Warn,
            "Saved pub tokens file is unreadable".into(),
            path.display().to_string(),
            Some("Inspect it with `dart pub token list`; remove broken entries with `dart pub token remove <url>`.".into()),
        );
    };
    let hosted = value.get("hosted").and_then(|h| h.as_array()).cloned().unwrap_or_default();
    let mut missing = Vec::new();
    let mut hosts = Vec::new();
    for entry in &hosted {
        let url = entry.get("url").and_then(|u| u.as_str()).unwrap_or("?");
        hosts.push(url.to_string());
        if let Some(env) = entry.get("env").and_then(|e| e.as_str()) {
            if std::env::var_os(env).map(|v| v.is_empty()).unwrap_or(true) {
                missing.push(format!("{url} reads ${env}"));
            }
        }
    }
    if missing.is_empty() {
        check(
            "flutter.pub_credentials",
            DoctorStatus::Pass,
            format!("{} saved pub token(s) usable", hosted.len()),
            if hosts.is_empty() { path.display().to_string() } else { hosts.join(", ") },
            None,
        )
    } else {
        check(
            "flutter.pub_credentials",
            DoctorStatus::Fail,
            "A saved pub token needs an environment variable that is not set; `pub get` will fail to authenticate".into(),
            missing.join("; "),
            Some("Export the variable before `axiom pull` (for example `export PUB_TOKEN=...`), or remove the entry with `dart pub token remove <url>`.".into()),
        )
    }
}

fn project_checks(root: &Path) -> Vec<DoctorCheck> {
    let findings = super::pull_checks::collect(root);
    if findings.is_empty() {
        if root.join("pubspec.yaml").exists() {
            return vec![check(
                "flutter.project",
                DoctorStatus::Pass,
                "Flutter project settings suit Axiom (deployment targets, entitlements, INTERNET permission)".into(),
                "Checked ios/, macos/ and android/ where present.".into(),
                None,
            )];
        }
        return vec![];
    }
    vec![check(
        "flutter.project",
        DoctorStatus::Warn,
        format!("{} Flutter project setting(s) will break Axiom calls", findings.len()),
        super::pull_checks::summarize(&findings).join("; "),
        Some("Run `axiom pull --fix` to apply them.".into()),
    )]
}

pub fn checks(root: &Path) -> Vec<DoctorCheck> {
    let fvm = uses_fvm(root);
    let mut out = vec![
        tool_check("flutter.flutter", "flutter", "Flutter ", fvm),
        tool_check("flutter.dart", "dart", "Dart SDK version", fvm),
    ];
    if cfg!(target_os = "macos") {
        out.push(cocoapods_check());
    }
    out.push(android_check());
    out.push(pub_credentials_check());
    out.extend(project_checks(root));
    out
}
