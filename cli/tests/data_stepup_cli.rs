use std::{
    net::TcpListener,
    process::{Command, Stdio},
};

#[test]
fn writes_refuse_before_login_lookup_or_step_up_without_yes() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    for (action, command) in [
        (vec!["sql", "write"], "data sql write"),
        (vec!["tables", "items", "write"], "data tables write"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_axiom-cli"))
            .args(["data", "--project", "demo", "--environment", "development"])
            .args(action)
            .args(["--file", "does-not-exist.json"])
            .current_dir(dir.path())
            .env("AXIOM_LOCAL_PROFILE_DIR", dir.path())
            .env("AXIOM_CLOUD_URL", &base)
            .env("CI", "true")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert_eq!(
            String::from_utf8_lossy(&output.stderr).trim(),
            format!("axiom {command}: pass --yes to confirm without a prompt")
        );
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
}
