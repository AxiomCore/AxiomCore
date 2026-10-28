//! Impact against the W5.4 reference application (`qual/w5-impact/app`): a hosted
//! PostgreSQL database, a backend whose operations map its columns, and a frontend with a
//! UI lock and two form-field consumers (`Home.composer.title` submits `create_task`,
//! `Home.renamer.title` submits `update_task`). Skipped when the fixture is absent.
use super::*;
use std::collections::BTreeMap;

const FIXTURE: &str = "/Volumes/ExternalSSD/axiom-w1a/qual/w5-impact/app";
const KEY: [u8; 32] = [5; 32];

fn fixture() -> Option<PathBuf> {
    Path::new(FIXTURE).join("AxiomDeps.toml").is_file().then(|| PathBuf::from(FIXTURE))
}
/// A copy of the fixture in a fresh directory, so a test may break one input.
fn copy(app: &Path) -> tempfile::TempDir {
    fn go(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap().flatten() {
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                if entry.file_name() != ".axiom" {
                    go(&entry.path(), &target);
                }
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    let dir = tempfile::tempdir().unwrap();
    go(app, &dir.path().join("app"));
    dir
}
/// The database project's sources with `edit` applied to the schema and the version set.
fn release(app: &Path, version: &str, edit: impl Fn(&str) -> String, parent: Option<&Release>) -> Release {
    let dir = app.join("database/postgres");
    let schema = std::fs::read_to_string(dir.join("schema.acore")).unwrap().replace("version: \"1.0.0\"", &format!("version: \"{version}\""));
    let input = acore::database::ProjectInput {
        entry: "schema.acore".into(),
        files: BTreeMap::from([("AxiomDeps.toml".into(), std::fs::read_to_string(dir.join("AxiomDeps.toml")).unwrap()), ("schema.acore".into(), edit(&schema))]),
        packages: BTreeMap::new(),
    };
    let compiled = acore::database::compile(&input).unwrap_or_else(|e| panic!("{e:#?}"));
    db::sign_with_key(db::revision(&compiled, input, parent, "migration", None).unwrap(), KEY).unwrap()
}
fn base(app: &Path) -> Release {
    release(app, "1.0.0", |s| s.to_string(), None)
}
fn report(app: &Path, release: &Release) -> Report {
    compute(&app.join("database/postgres/schema.acore"), &Options::default(), release).unwrap()
}
fn typed(app: &Path) -> Release {
    release(app, "1.1.0", |s| s.replace("      title: Pg.Text", "      title: Pg.VarChar(length: 120)"), Some(&base(app)))
}
fn field_consumers(report: &Report) -> Vec<(&str, &str)> {
    report.frontend.iter().filter_map(|f| Some((f.consumer.as_str(), f.field.as_deref()?))).collect()
}

#[test]
fn the_application_is_discovered_from_the_database_source() {
    let Some(app) = fixture() else { return };
    assert_eq!(discover_application(&app.join("database/postgres/schema.acore")), Some(app.canonicalize().unwrap()));
    assert_eq!(discover_application(Path::new("/")), None);
}

#[test]
fn a_column_type_change_reaches_the_endpoints_and_the_locked_field_consumers() {
    let Some(app) = fixture() else { return };
    let r = report(&app, &typed(&app));
    let title = r.changed.iter().find(|c| c.logical_name == "Columns.app.tasks.title").expect("title changed");
    assert_eq!(r.resources[0].operations, ["create", "delete", "list", "update"]);
    for endpoint in &r.endpoints {
        assert_eq!(endpoint.fields.iter().map(|f| (f.api_field.as_str(), f.column_id.as_str())).collect::<Vec<_>>(), [("title", title.id.as_str())], "{}", endpoint.endpoint);
    }
    assert_eq!(r.endpoints.iter().map(|e| e.endpoint.as_str()).collect::<Vec<_>>(), ["create_task", "delete_task", "list_tasks", "update_task"]);
    // Both form fields that submit a marked endpoint, at their source lines.
    assert_eq!(field_consumers(&r), [("Home.composer.title", "title"), ("Home.renamer.title", "title")]);
    let main = std::fs::read_to_string(app.join("frontend/main.acore")).unwrap();
    for consumer in &r.frontend {
        assert_eq!(consumer.source.file, "frontend/main.acore");
        let line = main.lines().nth(consumer.source.line as usize - 1).unwrap();
        assert!(consumer.source.column as usize <= line.chars().count() + 1, "{consumer:?}");
    }
    let renamer = r.frontend.iter().find(|f| f.consumer == "Home.renamer.title").unwrap();
    assert!(main.lines().nth(renamer.source.line as usize - 1).unwrap().contains("field title"), "{renamer:?}");
    assert!(r.frontend.iter().any(|f| f.consumer == "Home.add" && f.field.is_none()));
    assert_eq!((r.coverage.as_str(), r.unknown.as_slice()), ("complete", [].as_slice()));
}

#[test]
fn a_new_required_column_marks_the_inserting_operation_only() {
    let Some(app) = fixture() else { return };
    let r = report(&app, &release(&app, "1.1.0", |s| s.replace("      title: Pg.Text\n", "      title: Pg.Text\n      priority: Pg.Int32\n"), Some(&base(&app))));
    assert_eq!(r.resources[0].operations, ["create"]);
    assert_eq!(r.endpoints.iter().map(|e| (e.endpoint.as_str(), e.fields.len())).collect::<Vec<_>>(), [("create_task", 0)]);
    assert!(field_consumers(&r).is_empty() && r.frontend.iter().any(|f| f.consumer == "Home.add"));
    assert!(!r.frontend.iter().any(|f| f.consumer.starts_with("Home.renamer")));
}

#[test]
fn an_unrelated_column_marks_nothing() {
    let Some(app) = fixture() else { return };
    let r = report(&app, &release(&app, "1.1.0", |s| s.replace("      title: Pg.Text\n", "      title: Pg.Text\n      note: Pg.Text(nullable: true)\n"), Some(&base(&app))));
    assert!(r.changed.iter().any(|c| c.logical_name == "Columns.app.tasks.note"));
    assert!(r.resources[0].operations.is_empty() && r.endpoints.is_empty() && r.frontend.is_empty());
    assert_eq!(r.coverage, "complete");
}

#[test]
fn an_object_outside_the_model_is_unknown() {
    let Some(app) = fixture() else { return };
    let view = "    view titles(query: Sql.query(source: \"SELECT tasks.title FROM app.tasks\", dependencies: [Tables.app.tasks], effects: SqlEffects.unknown), columns: [{name: \"title\", storageType: Pg.Text, nullable: true}], securityInvoker: true, securityBarrier: false, checkOption: \"none\")\n  }\n}";
    let r = report(&app, &release(&app, "1.1.0", |s| s.trim_end().trim_end_matches('}').trim_end().trim_end_matches('}').to_string() + "\n" + view, Some(&base(&app))));
    let sql = r.changed.iter().find(|c| c.kind == "sql_object").expect("view changed");
    assert_eq!(r.unknown, [Unknown::new("outside_model", Some(&sql.id), "sql_object")]);
    assert_eq!(r.coverage, "partial");
}

#[test]
fn missing_inputs_give_unknown_coverage_never_no_impact() {
    let Some(app) = fixture() else { return };
    let release = typed(&app);
    // No UI lock.
    let dir = copy(&app);
    let root = dir.path().join("app");
    std::fs::remove_file(root.join("frontend/axiom.ui.lock.json")).unwrap();
    let r = report(&root, &release);
    assert!(r.unknown.contains(&Unknown::new("missing_lock", Some("frontend"), "lock_absent")), "{:?}", r.unknown);
    assert_eq!(r.coverage, "partial");
    // A stale lock: the backend artifact changed after the lock was resolved.
    let dir = copy(&app);
    let root = dir.path().join("app");
    let mut artifact = std::fs::read(root.join("contracts/backend.axiom")).unwrap();
    artifact.push(b'\n');
    std::fs::write(root.join("contracts/backend.axiom"), artifact).unwrap();
    let r = report(&root, &release);
    assert!(r.unknown.contains(&Unknown::new("stale_lock", Some("frontend"), "lock_artifact_mismatch")), "{:?}", r.unknown);
    // No server plan: nothing can be traced, and the report says so.
    let dir = copy(&app);
    let root = dir.path().join("app");
    std::fs::remove_file(root.join("contracts/backend.server.json")).unwrap();
    let r = report(&root, &release);
    assert_eq!((r.endpoints.len(), r.coverage.as_str()), (0, "partial"));
    assert!(r.unknown.contains(&Unknown::new("no_plan", None, "plan_absent")));
    // An explicit plan that does not read.
    let r = compute(&root.join("database/postgres/schema.acore"), &Options { application: None, plan: Some(root.join("missing.server.json")) }, &release).unwrap();
    assert!(r.unknown.contains(&Unknown::new("no_plan", None, "plan_unreadable")));
    // No application at all (the database project alone).
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("db")).unwrap();
    std::fs::copy(app.join("database/postgres/schema.acore"), dir.path().join("db/schema.acore")).unwrap();
    let r = compute(&dir.path().join("db/schema.acore"), &Options::default(), &release).unwrap();
    assert_eq!((r.unknown.as_slice(), r.coverage.as_str()), ([Unknown::new("no_plan", None, "plan_absent")].as_slice(), "partial"));
}

#[test]
fn the_sidecar_is_signed_by_the_release_key_and_only_for_it() {
    use ed25519_dalek::{Signer, SigningKey};
    let Some(app) = fixture() else { return };
    let dir = copy(&app);
    let root = dir.path().join("app");
    let release = typed(&root);
    let source = root.join("database/postgres/schema.acore");
    let options = Options { application: Some(root.clone()), plan: Some(root.join("contracts/backend.server.json")) };
    let key = SigningKey::from_bytes(&KEY);
    let sign = |m: &[u8]| Ok((hex::encode(key.verifying_key().as_bytes()), hex::encode(key.sign(m).to_bytes())));
    let path = write_with(&root, &source, &options, &release, sign).unwrap().unwrap();
    assert_eq!(path, root.join(".axiom/database/migrations/1.1.0/impact.json"));
    let envelope = read(&path, &release).unwrap();
    assert_eq!(envelope.report.endpoints.len(), 4);
    assert_eq!(upload_body(&root, &release).unwrap(), std::fs::read(&path).unwrap());
    // Another key: no sidecar.
    let other = SigningKey::from_bytes(&[6; 32]);
    let sign = |m: &[u8]| Ok((hex::encode(other.verifying_key().as_bytes()), hex::encode(other.sign(m).to_bytes())));
    std::fs::remove_file(&path).unwrap();
    assert_eq!(write_with(&root, &source, &options, &release, sign).unwrap(), None);
    assert!(!path.exists() && upload_body(&root, &release).is_none());
    // A sidecar edited after signing is refused on read.
    let sign = |m: &[u8]| Ok((hex::encode(key.verifying_key().as_bytes()), hex::encode(key.sign(m).to_bytes())));
    write_with(&root, &source, &options, &release, sign).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    value["report"]["frontend"] = serde_json::json!([]);
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(read(&path, &release).is_err());
}
