#![cfg(unix)]
//! `axiom semantic` and `axiom mcp` are byte-transparent delegations to
//! `acore-semantic`; a fake binary records what it received.
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const FAKE: &str = r#"#!/bin/sh
printf 'fake:%s\n' "$FAKE_LABEL"
for argument in "$@"; do printf 'arg[%s]\n' "$argument"; done
/bin/cat
printf 'diagnostic on stderr\n' >&2
exit "${FAKE_EXIT:-0}"
"#;

fn fake(directory: &Path, name: &str, label: &str) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, FAKE.replace("$FAKE_LABEL", label)).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

fn run(program: &Path, arguments: &[&str], stdin: &[u8], env: &[(&str, &str)]) -> Output {
    use std::io::Write;
    let mut command = Command::new(program);
    command
        .args(arguments)
        .env_remove("AXIOM_ACORE_SEMANTIC")
        .env_remove("AXIOM_CLOUD_URL")
        .env_remove("AXIOM_REFERRAL_CODE")
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = command.spawn().unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    child.wait_with_output().unwrap()
}

fn axiom() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_axiom-cli"))
}

/// A copy of the CLI in its own directory, so a shared target directory that
/// happens to contain a real acore-semantic cannot become the sibling.
fn isolated_axiom(directory: &Path) -> PathBuf {
    let installed = directory.join("axiom");
    std::fs::hard_link(axiom(), &installed).unwrap();
    installed
}

fn scratch() -> tempfile::TempDir {
    // The target directory keeps hard links to the CLI on one filesystem.
    tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap()
}

#[test]
fn arguments_stdin_output_and_exit_codes_pass_through_unchanged() {
    let directory = scratch();
    let binary = fake(directory.path(), "acore-semantic", "env");
    let binary_text = binary.to_str().unwrap();
    let stdin = b"{\"op\":\"workspace.open\"}\n\0binary\xff";
    let cases: &[(&str, &[&str], &[&str])] = &[
        (
            "semantic",
            &[
                "--root",
                "x",
                "--help",
                "--version",
                "--",
                "-V",
                "has space",
                "",
            ],
            &[],
        ),
        (
            "semantic",
            &["serve", "--json", "--variant", "mobile"],
            &["serve", "--json", "--variant", "mobile"],
        ),
        (
            "mcp",
            &["--state-dir", "s", "--help", "--unknown"],
            &["mcp", "--state-dir", "s", "--help", "--unknown"],
        ),
        ("semantic", &[], &[]),
        ("mcp", &[], &["mcp"]),
    ];
    for exit in ["0", "1", "3"] {
        for (subcommand, arguments, direct) in cases {
            let direct: Vec<&str> = if *subcommand == "semantic" && direct.is_empty() {
                arguments.to_vec()
            } else {
                direct.to_vec()
            };
            let mut through = vec![*subcommand];
            through.extend_from_slice(arguments);
            let env = [("AXIOM_ACORE_SEMANTIC", binary_text), ("FAKE_EXIT", exit)];
            let delegated = run(axiom(), &through, stdin, &env);
            let expected = run(&binary, &direct, stdin, &env);
            assert_eq!(delegated.stdout, expected.stdout, "{through:?}");
            assert_eq!(delegated.stderr, expected.stderr, "{through:?}");
            assert_eq!(delegated.status.code(), Some(exit.parse().unwrap()));
            assert_eq!(delegated.status.code(), expected.status.code());
            assert!(delegated.stdout.ends_with(stdin));
        }
    }
}

#[test]
fn lookup_prefers_env_then_resolved_sibling_then_path() {
    let install = scratch();
    let links = scratch();
    let on_path = scratch();
    let environment = scratch();
    let bare = scratch();
    let installed = isolated_axiom(install.path());
    std::os::unix::fs::symlink(&installed, links.path().join("axiom")).unwrap();
    let explicit = fake(environment.path(), "custom-semantic", "env");
    fake(install.path(), "acore-semantic", "sibling");
    fake(on_path.path(), "acore-semantic", "path");
    let path = format!("{}:/usr/bin:/bin", on_path.path().display());
    let label = |program: &Path, env: &[(&str, &str)]| {
        let mut env = env.to_vec();
        env.push(("PATH", &path));
        let output = run(program, &["semantic", "--capabilities"], b"", &env);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .to_string()
    };
    let explicit = explicit.to_str().unwrap();
    assert_eq!(
        label(&installed, &[("AXIOM_ACORE_SEMANTIC", explicit)]),
        "fake:env"
    );
    assert_eq!(label(&installed, &[]), "fake:sibling");
    // Homebrew links bin/axiom into its keg; the keg's sibling wins.
    assert_eq!(label(&links.path().join("axiom"), &[]), "fake:sibling");
    // Without a sibling, PATH is used.
    assert_eq!(label(&isolated_axiom(bare.path()), &[]), "fake:path");
}

#[test]
fn missing_binary_reports_the_search_on_stderr_only() {
    let empty = scratch();
    let axiom = isolated_axiom(empty.path());
    let path = format!("{}:/usr/bin:/bin", empty.path().display());
    let output = run(&axiom, &["mcp"], b"{}", &[("PATH", &path)]);
    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("acore-semantic is not installed"),
        "{stderr}"
    );
    assert!(
        stderr.contains("brew install axiomcore/tap/acore-semantic"),
        "{stderr}"
    );
    assert!(stderr.contains("AXIOM_ACORE_SEMANTIC"), "{stderr}");

    let missing = empty.path().join("nope");
    let output = run(
        &axiom,
        &["semantic", "--capabilities"],
        b"",
        &[("AXIOM_ACORE_SEMANTIC", missing.to_str().unwrap())],
    );
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not an existing file"));
}
