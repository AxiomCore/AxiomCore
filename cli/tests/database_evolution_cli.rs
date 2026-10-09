use std::{path::Path, process::Command};

fn schema(body: &str) -> String {
    format!("profile database 1\nmodule storage\ndatabase storage(engine: DatabaseEngine.postgres) {{ manage(schemas: [Schemas.app], mode: ManagementMode.ownedObjects) schema app {{ {body} }} }}")
}
fn run(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_axiom-cli"))
        .current_dir(root)
        .args(args)
        .env("CI", "true")
        .env(
            "DATABASE_URL",
            "postgres://private:PRIVATE_ENDPOINT_SECRET@invalid.invalid/db",
        )
        .env("PGHOST", "invalid.invalid")
        .env("AXIOM_CLOUD_URL", "invalid-cloud-endpoint")
        .output()
        .unwrap()
}
fn project() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("AxiomDeps.toml"), "type = \"database\"\ntargets = [\"server\"]\n[application]\nname = \"example.storage\"\nversion = \"0.0.0\"\n").unwrap();
    root
}

#[test]
fn offline_evolution_check_binds_contracts_and_redacts_reference_literals() {
    let root = project();
    std::fs::write(root.path().join("schema.acore"), schema("table codes { id: Pg.Int64() owner: Pg.Text() label: Pg.Text() primaryKey(columns: [.id]) }")).unwrap();
    std::fs::write(root.path().join("evolution.acore"), "evolution seed(from: Empty, to: Current, coordination: EvolutionCoordination.maintenanceFenced) { referenceData codes(table: Current.Tables.app.codes, keys: [Current.Columns.app.codes.id], scope: scope(column: Current.Columns.app.codes.owner, value: \"acore\"), mode: ReferenceMode.insert, rows: [{id: 1, owner: \"acore\", label: \"PRIVATE_ROW_SECRET\"}], maxRows: 10, maxChanges: 10) }").unwrap();
    let out = run(
        root.path(),
        &[
            "database",
            "evolution-check",
            "schema.acore",
            "--evolution",
            "evolution.acore",
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(!text.contains("PRIVATE_ROW_SECRET"));
    assert!(!text.contains("PRIVATE_ENDPOINT_SECRET"));
    assert!(!text.contains("INSERT INTO"));
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["migrationAuthority"], false);
    assert_eq!(value["referenceOwners"][0]["rows"], 1);
    assert_eq!(
        value["referenceOwnership"],
        "requires-verified-private-history"
    );
    assert!(value["toSha256"].as_str().unwrap().len() == 64);
    assert!(!root.path().join(".axiom").exists());
}

#[test]
fn missing_transform_and_unknown_coordination_fail_without_providers_or_history() {
    let root = project();
    let old = "table items { id: Pg.Int64() source: Pg.Text() primaryKey(columns: [.id]) }";
    std::fs::write(root.path().join("previous.acore"), schema(old)).unwrap();
    std::fs::write(
        root.path().join("schema.acore"),
        schema(&old.replace("Pg.Text()", "Pg.Int64()")),
    )
    .unwrap();
    for body in [
        "evolution upgrade(from: Previous, to: Current, coordination: EvolutionCoordination.maintenanceFenced) {}",
        "evolution upgrade(from: Previous, to: Current, coordination: EvolutionCoordination.dualWrite) {}",
    ] {
        std::fs::write(root.path().join("evolution.acore"), body).unwrap();
        let out = run(root.path(), &["database", "evolution-check", "schema.acore", "--previous", "previous.acore", "--evolution", "evolution.acore"]);
        assert!(!out.status.success());
        let text = String::from_utf8_lossy(&out.stderr);
        assert!(text.contains("ADB50"), "{text}");
        assert!(!text.contains("PRIVATE_ENDPOINT_SECRET"));
        assert!(!root.path().join(".axiom").exists());
    }
}

#[test]
fn checkpoint_and_authored_evolution_are_mutually_exclusive_cli_intents() {
    let root = project();
    let out = run(
        root.path(),
        &[
            "database",
            "plan",
            "schema.acore",
            "--deployment",
            "missing.json",
            "--dev-profile",
            "missing-dev.json",
            "--baseline",
            "empty",
            "--checkpoint",
            "bundle",
            "--evolution",
            "evolution.acore",
        ],
    );
    assert!(!out.status.success());
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(text.contains("cannot be used with"), "{text}");
    assert!(!root.path().join(".axiom").exists());
}
