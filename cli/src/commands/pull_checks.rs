//! Project checks that run after `axiom pull --framework flutter`.
//!
//! Each check reports a problem that would make a freshly generated Axiom
//! client fail at build or run time on one platform, and offers a fix.
//! `axiom pull --fix` applies the fixes; nothing is changed otherwise.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub const MIN_IOS: (u32, u32) = (13, 0);
pub const MIN_MACOS: (u32, u32) = (11, 0);

/// One problem found in the app project, with the edit that fixes it.
pub struct Finding {
    pub platform: &'static str,
    pub message: String,
    pub file: PathBuf,
    fix: Option<Box<dyn Fn(&str) -> String>>,
}

impl Finding {
    pub fn fixable(&self) -> bool {
        self.fix.is_some()
    }
}

/// Run every check, print findings and hints, and apply fixes when asked.
/// Returns the findings that remain unfixed.
pub fn run_flutter_checks(root: &Path, base_urls: &[String], apply: bool) -> Result<usize> {
    let findings = collect(root);
    let hints = network_hints(root, base_urls);

    if findings.is_empty() && hints.is_empty() {
        println!("🩺 Project checks: iOS, macOS and Android settings look right for Axiom.");
        return Ok(0);
    }

    let mut remaining = 0;
    if !findings.is_empty() {
        println!("\n🩺 Project checks found {} problem(s):", findings.len());
    }
    for finding in &findings {
        let relative = finding
            .file
            .strip_prefix(root)
            .unwrap_or(&finding.file)
            .display()
            .to_string();
        if apply {
            if let Some(fix) = &finding.fix {
                let before = fs::read_to_string(&finding.file)
                    .with_context(|| format!("read {}", finding.file.display()))?;
                let after = fix(&before);
                if after != before {
                    fs::write(&finding.file, after)
                        .with_context(|| format!("write {}", finding.file.display()))?;
                }
                println!("   ✅ fixed [{}] {} ({})", finding.platform, finding.message, relative);
                continue;
            }
        }
        remaining += 1;
        let marker = if finding.fixable() { "fixable" } else { "manual" };
        println!("   ⚠️  [{}] {} ({}, {})", finding.platform, finding.message, relative, marker);
    }
    if !apply && findings.iter().any(Finding::fixable) {
        println!("   → Run `axiom pull --fix` to apply the fixable ones.");
    }
    for hint in hints {
        println!("\n💡 {hint}");
    }
    Ok(remaining)
}

/// Every problem, without printing or changing anything (used by `doctor`).
pub fn collect(root: &Path) -> Vec<Finding> {
    let mut findings = Vec::new();
    check_podfile(root, "ios", "ios", MIN_IOS, &mut findings);
    check_pbxproj(root, "ios", "IPHONEOS_DEPLOYMENT_TARGET", MIN_IOS, &mut findings);
    check_podfile(root, "macos", "osx", MIN_MACOS, &mut findings);
    check_pbxproj(root, "macos", "MACOSX_DEPLOYMENT_TARGET", MIN_MACOS, &mut findings);
    check_entitlements(root, &mut findings);
    check_android_internet(root, &mut findings);
    findings
}

pub fn summarize(findings: &[Finding]) -> Vec<String> {
    findings
        .iter()
        .map(|f| format!("[{}] {}", f.platform, f.message))
        .collect()
}

fn parse_version(text: &str) -> Option<(u32, u32)> {
    let mut parts = text.trim().trim_matches(|c| c == '\'' || c == '"').split('.');
    let major = parts.next()?.trim().parse().ok()?;
    let minor = parts.next().map(|m| m.trim().parse().unwrap_or(0)).unwrap_or(0);
    Some((major, minor))
}

fn fmt_version(v: (u32, u32)) -> String {
    format!("{}.{}", v.0, v.1)
}

/// `platform :ios, '12.0'` (often commented out in a fresh app).
fn check_podfile(root: &Path, dir: &'static str, pod_platform: &'static str, min: (u32, u32), out: &mut Vec<Finding>) {
    let path = root.join(dir).join("Podfile");
    let Ok(text) = fs::read_to_string(&path) else {
        return;
    };
    let needle = format!("platform :{pod_platform}");
    let active = text
        .lines()
        .find(|l| l.trim_start().starts_with(&needle))
        .and_then(|l| l.split(',').nth(1))
        .and_then(parse_version);
    let problem = match active {
        Some(v) if v >= min => return,
        Some(v) => format!(
            "Podfile targets {} {}, the Axiom plugin needs {}",
            pod_platform,
            fmt_version(v),
            fmt_version(min)
        ),
        None => format!(
            "Podfile has no active `platform :{pod_platform}` line; CocoaPods then defaults below the {} the Axiom plugin needs",
            fmt_version(min)
        ),
    };
    let wanted = format!("platform :{pod_platform}, '{}'", fmt_version(min));
    out.push(Finding {
        platform: if dir == "ios" { "iOS" } else { "macOS" },
        message: problem,
        file: path,
        fix: Some(Box::new(move |text: &str| {
            let mut replaced = false;
            let mut lines: Vec<String> = text
                .lines()
                .map(|line| {
                    let t = line.trim_start().trim_start_matches('#').trim_start();
                    if !replaced && t.starts_with(&needle) {
                        replaced = true;
                        wanted.clone()
                    } else {
                        line.to_string()
                    }
                })
                .collect();
            if !replaced {
                lines.insert(0, wanted.clone());
            }
            let mut joined = lines.join("\n");
            if text.ends_with('\n') {
                joined.push('\n');
            }
            joined
        })),
    });
}

/// `IPHONEOS_DEPLOYMENT_TARGET = 12.0;` / `MACOSX_DEPLOYMENT_TARGET = 10.14;`
fn check_pbxproj(root: &Path, dir: &'static str, key: &'static str, min: (u32, u32), out: &mut Vec<Finding>) {
    let path = root.join(dir).join("Runner.xcodeproj").join("project.pbxproj");
    let Ok(text) = fs::read_to_string(&path) else {
        return;
    };
    let low: Vec<(u32, u32)> = text
        .lines()
        .filter_map(|l| {
            let t = l.trim();
            t.strip_prefix(key)
                .and_then(|rest| rest.trim().strip_prefix('='))
                .and_then(|v| parse_version(v.trim().trim_end_matches(';')))
        })
        .filter(|v| *v < min)
        .collect();
    let Some(lowest) = low.iter().min().copied() else {
        return;
    };
    let wanted = fmt_version(min);
    out.push(Finding {
        platform: if dir == "ios" { "iOS" } else { "macOS" },
        message: format!(
            "Xcode project sets {key} = {} in {} build configuration(s); the Axiom plugin needs {wanted}",
            fmt_version(lowest),
            low.len()
        ),
        file: path,
        fix: Some(Box::new(move |text: &str| {
            let mut out: Vec<String> = Vec::new();
            for line in text.lines() {
                let t = line.trim();
                let current = t
                    .strip_prefix(key)
                    .and_then(|rest| rest.trim().strip_prefix('='))
                    .and_then(|v| parse_version(v.trim().trim_end_matches(';')));
                match current {
                    Some(v) if v < min => {
                        let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
                        out.push(format!("{indent}{key} = {wanted};"));
                    }
                    _ => out.push(line.to_string()),
                }
            }
            let mut joined = out.join("\n");
            if text.ends_with('\n') {
                joined.push('\n');
            }
            joined
        })),
    });
}

const NETWORK_CLIENT: &str = "com.apple.security.network.client";

fn check_entitlements(root: &Path, out: &mut Vec<Finding>) {
    for name in ["DebugProfile.entitlements", "Release.entitlements"] {
        let path = root.join("macos").join("Runner").join(name);
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        if !text.contains("com.apple.security.app-sandbox") || has_true_key(&text, NETWORK_CLIENT) {
            continue;
        }
        out.push(Finding {
            platform: "macOS",
            message: format!("sandboxed app lacks {NETWORK_CLIENT}, so every outgoing request fails"),
            file: path,
            fix: Some(Box::new(|text: &str| {
                if text.contains(NETWORK_CLIENT) {
                    // Present but false: flip it.
                    let key = format!("<key>{NETWORK_CLIENT}</key>");
                    if let Some(at) = text.find(&key) {
                        let rest = &text[at + key.len()..];
                        if let Some(false_at) = rest.find("<false/>") {
                            let abs = at + key.len() + false_at;
                            return format!("{}<true/>{}", &text[..abs], &text[abs + "<false/>".len()..]);
                        }
                    }
                    return text.to_string();
                }
                match text.rfind("</dict>") {
                    Some(at) => format!(
                        "{}\t<key>{NETWORK_CLIENT}</key>\n\t<true/>\n{}",
                        &text[..at],
                        &text[at..]
                    ),
                    None => text.to_string(),
                }
            })),
        });
    }
}

fn has_true_key(text: &str, key: &str) -> bool {
    let tag = format!("<key>{key}</key>");
    text.find(&tag)
        .map(|at| text[at + tag.len()..].trim_start().starts_with("<true/>"))
        .unwrap_or(false)
}

const INTERNET: &str = "android.permission.INTERNET";

fn check_android_internet(root: &Path, out: &mut Vec<Finding>) {
    let path = root.join("android/app/src/main/AndroidManifest.xml");
    let Ok(text) = fs::read_to_string(&path) else {
        return;
    };
    if text.contains(INTERNET) {
        return;
    }
    out.push(Finding {
        platform: "Android",
        message: "main AndroidManifest.xml lacks the INTERNET permission (debug builds get it from Flutter, release builds fail every request)".into(),
        file: path,
        fix: Some(Box::new(|text: &str| insert_after_manifest_open(text, &format!(
            "    <uses-permission android:name=\"{INTERNET}\"/>"
        )))),
    });
}

fn insert_after_manifest_open(text: &str, line: &str) -> String {
    let Some(start) = text.find("<manifest") else {
        return text.to_string();
    };
    let Some(close) = text[start..].find('>') else {
        return text.to_string();
    };
    let at = start + close + 1;
    format!("{}\n{}{}", &text[..at], line, &text[at..])
}

fn host_of(url: &str) -> Option<(String, String, Option<u16>)> {
    let parsed = url::Url::parse(url).ok()?;
    Some((
        parsed.scheme().to_string(),
        parsed.host_str()?.to_string(),
        parsed.port_or_known_default(),
    ))
}

/// Hints that depend on the configured base URL rather than a file problem.
pub fn network_hints(root: &Path, base_urls: &[String]) -> Vec<String> {
    let mut hints = Vec::new();
    let android = root.join("android").is_dir();
    for url in base_urls {
        let Some((scheme, host, port)) = host_of(url) else {
            continue;
        };
        let local = matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1" | "[::1]" | "0.0.0.0");
        if local && android {
            let port = port.map(|p| format!(":{p}")).unwrap_or_default();
            hints.push(format!(
                "Base URL {url} points at localhost. On the Android emulator localhost is the emulator itself: \
use http://10.0.2.2{port} instead (re-run `axiom pull --base-url http://10.0.2.2{port}`, or override the base URL at runtime for Android). \
On a physical device use your machine's LAN address or `adb reverse tcp{port} tcp{port}`."
            ));
        }
        if scheme == "http" && android && !cleartext_allowed(root) {
            hints.push(format!(
                "Base URL {url} uses plain http. Android 9+ blocks cleartext traffic unless the app allows it; \
for local development add android:usesCleartextTraffic=\"true\" to <application> in android/app/src/debug/AndroidManifest.xml \
(or a network security config for 10.0.2.2), and use https in release."
            ));
        }
    }
    hints
}

fn cleartext_allowed(root: &Path) -> bool {
    ["debug", "main", "profile"].iter().any(|flavor| {
        fs::read_to_string(root.join(format!("android/app/src/{flavor}/AndroidManifest.xml")))
            .map(|t| t.contains("usesCleartextTraffic=\"true\"") || t.contains("networkSecurityConfig"))
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixes_fresh_flutter_project_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("ios/Runner.xcodeproj")).unwrap();
        fs::create_dir_all(root.join("macos/Runner.xcodeproj")).unwrap();
        fs::create_dir_all(root.join("android/app/src/main")).unwrap();
        fs::write(root.join("ios/Podfile"), "# platform :ios, '12.0'\ntarget 'Runner' do\nend\n").unwrap();
        fs::write(root.join("macos/Podfile"), "platform :osx, '10.14'\n").unwrap();
        fs::write(
            root.join("ios/Runner.xcodeproj/project.pbxproj"),
            "\t\t\t\tIPHONEOS_DEPLOYMENT_TARGET = 12.0;\n\t\t\t\tIPHONEOS_DEPLOYMENT_TARGET = 12.0;\n",
        )
        .unwrap();
        fs::write(
            root.join("macos/Runner.xcodeproj/project.pbxproj"),
            "\t\t\t\tMACOSX_DEPLOYMENT_TARGET = 10.14;\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("macos/Runner")).unwrap();
        fs::write(
            root.join("macos/Runner/Release.entitlements"),
            "<plist>\n<dict>\n\t<key>com.apple.security.app-sandbox</key>\n\t<true/>\n</dict>\n</plist>\n",
        )
        .unwrap();
        fs::write(
            root.join("android/app/src/main/AndroidManifest.xml"),
            "<manifest xmlns:android=\"http://schemas.android.com/apk/res/android\">\n    <application/>\n</manifest>\n",
        )
        .unwrap();
        assert_eq!(collect(root).len(), 6);
        assert_eq!(run_flutter_checks(root, &[], true).unwrap(), 0);
        assert!(collect(root).is_empty());
        assert!(fs::read_to_string(root.join("ios/Podfile")).unwrap().starts_with("platform :ios, '13.0'\n"));
        assert!(fs::read_to_string(root.join("macos/Runner/Release.entitlements")).unwrap().contains(NETWORK_CLIENT));
    }

    #[test]
    fn localhost_base_url_gets_emulator_hint() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("android")).unwrap();
        let hints = network_hints(dir.path(), &["http://localhost:8000".into()]);
        assert!(hints.iter().any(|h| h.contains("10.0.2.2:8000")));
    }
}
