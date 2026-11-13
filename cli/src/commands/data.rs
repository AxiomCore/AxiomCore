//! Dashboard data API commands. Writes retain the reviewed payload and idempotency key.
use super::resources_recovery::Api;
use anyhow::{bail, ensure, Context, Result};
use clap::{Args, Subcommand};
use reqwest::Method;
use serde_json::{json, Value};
use std::{io::IsTerminal, path::PathBuf};

#[derive(Debug, Args)]
pub struct DataArgs {
    /// Project ID, slug, or unambiguous display name.
    #[arg(long, global = true)]
    project: Option<String>,
    /// Environment ID, slug, or unambiguous display name.
    #[arg(long, global = true)]
    environment: Option<String>,
    /// Emit only the JSON result on stdout.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    action: Action,
}
#[derive(Debug, Subcommand)]
pub enum Action {
    /// Inspect admission, permissions, limits, and the current schema head.
    Overview {
        #[command(subcommand)]
        action: Get,
    },
    /// Inspect tables, columns, keys, and side effects.
    Catalog {
        #[command(subcommand)]
        action: Get,
    },
    /// List query and write history.
    History {
        #[command(subcommand)]
        action: List,
    },
    /// List data audit events.
    Events {
        #[command(subcommand)]
        action: List,
    },
    /// Read SQL or preview and confirm a SQL write.
    Sql {
        #[command(subcommand)]
        action: Sql,
    },
    /// Query, count, export, or review row changes for a table.
    Tables {
        table: String,
        #[command(subcommand)]
        action: Table,
    },
    /// Manage saved SQL queries.
    SavedQueries {
        #[command(subcommand)]
        action: Saved,
    },
    /// Preview schema changes and capture a migration release.
    Capture {
        #[command(subcommand)]
        action: Capture,
    },
}
#[derive(Debug, Subcommand)]
pub enum Get {
    Get,
}
#[derive(Debug, Subcommand)]
pub enum List {
    List {
        #[arg(long)]
        before: Option<String>,
        #[arg(long)]
        action: Option<String>,
    },
}
#[derive(Debug, Args)]
pub struct Input {
    /// JSON request body, read once from disk.
    #[arg(long)]
    file: PathBuf,
}
#[derive(Debug, Args)]
pub struct Write {
    #[command(flatten)]
    input: Input,
    /// Accept the displayed preview without prompting.
    #[arg(long)]
    yes: bool,
    /// Reuse this UUID when resolving an uncertain write outcome.
    #[arg(long)]
    idempotency_key: Option<uuid::Uuid>,
}
#[derive(Debug, Subcommand)]
pub enum Sql {
    Read(Input),
    Preview(Input),
    Write(Write),
}
#[derive(Debug, Subcommand)]
pub enum Table {
    Query(Input),
    Cell(Input),
    Count(Input),
    Export(Input),
    Write(Write),
}
#[derive(Debug, Subcommand)]
pub enum Saved {
    List,
    Create(Write),
    Update {
        id: uuid::Uuid,
        #[command(flatten)]
        write: Write,
    },
    Delete {
        id: uuid::Uuid,
        #[arg(long)]
        yes: bool,
    },
}
#[derive(Debug, Subcommand)]
pub enum Capture {
    Preview(Input),
    Create(Write),
    Settings {
        #[command(subcommand)]
        action: Settings,
    },
}
#[derive(Debug, Subcommand)]
pub enum Settings {
    Get,
    Set(Write),
}

fn input(input: &Input, max: usize) -> Result<Value> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(&input.file)?
        .take((max + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= max, "Request body exceeds {max} bytes");
    let body: Value = serde_json::from_slice(&bytes).context("Expected a JSON request object")?;
    ensure!(body.is_object(), "Expected a JSON request object");
    Ok(body)
}
fn confirm(preview: &Value, yes: bool) -> Result<()> {
    eprintln!(
        "Review the proposed change:\n{}",
        serde_json::to_string_pretty(preview)?
    );
    if yes {
        return Ok(());
    }
    ensure!(
        std::io::stdin().is_terminal(),
        "Use --yes to accept the preview in noninteractive use"
    );
    for field in ["affected", "total_changed"] {
        if let Some(n) = preview[field].as_u64() {
            let answer: String = dialoguer::Input::new()
                .with_prompt(format!("Type {field} ({n}) to confirm"))
                .interact_text()?;
            ensure!(
                answer.trim() == n.to_string(),
                "Confirmation did not match; nothing executed"
            );
        }
    }
    ensure!(
        dialoguer::Confirm::new()
            .with_prompt("Execute this change, including listed side effects?")
            .default(false)
            .interact()?,
        "Cancelled; nothing executed"
    );
    Ok(())
}
fn print(value: &Value, as_json: bool) -> Result<()> {
    if !as_json {
        println!("Data API result:");
    }
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
fn sql_body(body: &Value) -> Result<Value> {
    let sql = body["sql"].as_str().context("sql must be a string")?;
    ensure!(
        !sql.trim().is_empty() && sql.len() <= 65536,
        "SQL must contain 1..65536 bytes"
    );
    let params = body.get("params").cloned().unwrap_or(json!([]));
    ensure!(
        params.as_array().is_some_and(|v| v.len() <= 32),
        "params must be an array of at most 32 typed values"
    );
    Ok(json!({"sql":sql,"params":params}))
}
struct Target<'a> {
    api: &'a Api,
    project: &'a str,
    environment: &'a str,
}
impl Target<'_> {
    fn path<'a>(&'a self, rest: &'a str) -> Vec<&'a str> {
        let mut path = vec![
            "projects",
            self.project,
            "data",
            "environments",
            self.environment,
        ];
        path.extend(rest.split('/'));
        path
    }
    async fn call(
        &self,
        method: Method,
        rest: &str,
        body: Option<&Value>,
        key: Option<&str>,
    ) -> Result<Value> {
        self.api
            .data_request(&method, &self.path(rest), body, key, &[])
            .await
    }
    async fn write(
        &self,
        rest: &str,
        method: Method,
        body: &Value,
        write: &Write,
    ) -> Result<Value> {
        let key = write
            .idempotency_key
            .unwrap_or_else(uuid::Uuid::new_v4)
            .to_string();
        eprintln!("Idempotency key: {key}. Retain it if the outcome is uncertain.");
        self.call(method, rest, Some(body), Some(&key)).await
    }
}
fn execute_body(body: &Value, preview: &Value, limit: u64) -> Result<Value> {
    let affected = preview["affected"]
        .as_u64()
        .context("Preview has no affected count")?;
    ensure!(
        affected <= limit,
        "Preview exceeds the environment confirmation limit"
    );
    let total = &preview["total_changed"];
    ensure!(
        total.is_null() || total.as_u64().is_some(),
        "Invalid preview total count"
    );
    ensure!(!preview["head"].is_null(), "Preview has no schema head");
    let mut result = sql_body(body)?;
    result["confirmed_affected"] = json!(affected);
    result["confirmed_total"] = total.clone();
    result["head"] = preview["head"].clone();
    Ok(result)
}
pub async fn handle(args: &DataArgs) -> Result<()> {
    let client = crate::auth_store::authenticated_cloud_client()?;
    let (project, environment) = super::data_metadata::resolve(
        &client,
        args.project.as_deref().context("--project is required")?,
        args.environment
            .as_deref()
            .context("--environment is required")?,
    )
    .await?;
    let api = Api::from_profile()?;
    let value = run(&api, &project, &environment, &args.action).await?;
    print(&value, args.json)
}
async fn run(api: &Api, project: &str, environment: &str, action: &Action) -> Result<Value> {
    let t = Target {
        api,
        project,
        environment,
    };
    match action {
        Action::Overview { .. } => t.call(Method::GET, "overview", None, None).await,
        Action::Catalog { .. } => t.call(Method::GET, "catalog", None, None).await,
        Action::History {
            action:
                List::List {
                    before,
                    action: filter,
                },
        }
        | Action::Events {
            action:
                List::List {
                    before,
                    action: filter,
                },
        } => {
            let mut query = Vec::new();
            if let Some(before) = before {
                query.push(("before", before.clone()));
            }
            if let Some(action) = filter {
                query.push(("action", action.clone()));
            }
            // The outer variant selects the endpoint, regardless of filters.
            let rest = if matches!(action, Action::Events { .. }) {
                "events"
            } else {
                ensure!(filter.is_none(), "--action is only supported for events");
                "history"
            };
            api.data_request(&Method::GET, &t.path(rest), None, None, &query)
                .await
        }
        Action::Sql { action } => match action {
            Sql::Read(i) => {
                t.call(
                    Method::POST,
                    "sql/read",
                    Some(&sql_body(&input(i, 262144)?)?),
                    None,
                )
                .await
            }
            Sql::Preview(i) => {
                t.call(
                    Method::POST,
                    "sql/write/preview",
                    Some(&sql_body(&input(i, 262144)?)?),
                    None,
                )
                .await
            }
            Sql::Write(w) => {
                ensure!(w.idempotency_key.is_none(), "SQL writes use a fresh preview; manual idempotency-key replay is not supported. Inspect history for uncertain outcomes.");
                let body = sql_body(&input(&w.input, 262144)?)?;
                let overview = t.call(Method::GET, "overview", None, None).await?;
                let preview = t
                    .call(Method::POST, "sql/write/preview", Some(&body), None)
                    .await?;
                let execute = execute_body(
                    &body,
                    &preview,
                    overview["limits"]["confirm_max"].as_u64().unwrap_or(10000),
                )?;
                confirm(&preview, w.yes)?;
                let result = t
                    .write("sql/write/execute", Method::POST, &execute, w)
                    .await?;
                receipt(result)
            }
        },
        Action::Tables { table, action } => {
            ensure!(
                regex::Regex::new(
                    r"^[A-Za-z_][A-Za-z0-9_$]{0,62}(\.[A-Za-z_][A-Za-z0-9_$]{0,62})?$"
                )?
                .is_match(table),
                "Invalid table name"
            );
            match action {
                Table::Query(i) => {
                    t.call(
                        Method::POST,
                        &format!("tables/{table}/rows:query"),
                        Some(&input(i, 65536)?),
                        None,
                    )
                    .await
                }
                Table::Cell(i) => {
                    t.call(
                        Method::POST,
                        &format!("tables/{table}/cell"),
                        Some(&input(i, 65536)?),
                        None,
                    )
                    .await
                }
                Table::Count(i) => {
                    t.call(
                        Method::POST,
                        &format!("tables/{table}/count"),
                        Some(&input(i, 65536)?),
                        None,
                    )
                    .await
                }
                Table::Export(i) => Ok(
                    json!({"csv":api.data_export(&t.path(&format!("tables/{table}/export")), &input(i,65536)?).await?}),
                ),
                Table::Write(w) => {
                    let body = input(&w.input, 2097152)?;
                    ensure!(body["operations"].as_array().is_some_and(|v| !v.is_empty() && v.len() <= 100), "Expected 1..100 operations with keys and original values for updates/deletes");
                    let catalog = t.call(Method::GET, "catalog", None, None).await?;
                    confirm(
                        &json!({"table":table,"operations":body["operations"],"catalog":catalog}),
                        w.yes,
                    )?;
                    receipt(
                        t.write(
                            &format!("tables/{table}/rows:write"),
                            Method::POST,
                            &body,
                            w,
                        )
                        .await?,
                    )
                }
            }
        }
        Action::SavedQueries { action } => match action {
            Saved::List => t.call(Method::GET, "saved-queries", None, None).await,
            Saved::Create(w) => {
                let body = input(&w.input, 65536)?;
                t.write("saved-queries", Method::POST, &body, w).await
            }
            Saved::Update { id, write: w } => {
                let body = input(&w.input, 65536)?;
                t.write(&format!("saved-queries/{id}"), Method::PUT, &body, w)
                    .await
            }
            Saved::Delete { id, yes } => {
                confirm(&json!({"delete_saved_query":id}), *yes)?;
                t.call(Method::DELETE, &format!("saved-queries/{id}"), None, None)
                    .await
            }
        },
        Action::Capture { action } => match action {
            Capture::Preview(i) => {
                t.call(
                    Method::POST,
                    "capture/preview",
                    Some(&input(i, 262144)?),
                    None,
                )
                .await
            }
            Capture::Create(w) => {
                ensure!(w.idempotency_key.is_none(), "Capture uses a fresh preview; manual idempotency-key replay is not supported. Inspect migration history for uncertain outcomes.");
                let body = input(&w.input, 262144)?;
                let preview = t
                    .call(
                        Method::POST,
                        "capture/preview",
                        Some(&json!({"sql":body["sql"],"title":body["title"]})),
                        None,
                    )
                    .await?;
                let execute = capture_body(&body, &preview)?;
                confirm(&preview, w.yes)?;
                t.write("capture", Method::POST, &execute, w).await
            }
            Capture::Settings {
                action: Settings::Get,
            } => t.call(Method::GET, "capture/settings", None, None).await,
            Capture::Settings {
                action: Settings::Set(w),
            } => {
                let body = input(&w.input, 262144)?;
                confirm(&body, w.yes)?;
                t.write("capture/settings", Method::PUT, &body, w).await
            }
        },
    }
}
fn capture_body(body: &Value, preview: &Value) -> Result<Value> {
    use sha2::{Digest, Sha256};
    let statements = preview["statements"]
        .as_array()
        .context("Capture preview has no statements")?;
    let digest = hex::encode(Sha256::digest(serde_jcs::to_vec(statements)?));
    ensure!(
        preview["statements_sha256"].as_str() == Some(&digest),
        "Capture statements hash does not match the review"
    );
    let mut result = json!({"sql":body["sql"], "title":body["title"], "statements_sha256":digest});
    for field in ["parent_sha256", "sql_sha256"] {
        let value = preview[field].as_str().context("Missing capture hash")?;
        ensure!(
            value.len() == 64
                && value
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "Invalid capture hash"
        );
        result[field] = json!(value);
    }
    let version = preview["version"]
        .as_str()
        .filter(|s| !s.is_empty())
        .context("Missing capture version")?;
    result["version"] = json!(version);
    Ok(result)
}
fn receipt(result: Value) -> Result<Value> {
    if result["receipt"]["state"] == "committed" {
        return Ok(result);
    }
    bail!("Write was not confirmed committed; inspect history and retain the idempotency key before deciding whether to retry")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{cloud_mock::Mock, step_up::StepUpStore};
    use clap::Parser;
    fn file(dir: &tempfile::TempDir, body: Value) -> Input {
        let path = dir.path().join("input.json");
        std::fs::write(&path, body.to_string()).unwrap();
        Input { file: path }
    }
    fn write(input: Input) -> Write {
        Write {
            input,
            yes: true,
            idempotency_key: Some(uuid::Uuid::nil()),
        }
    }
    fn fresh_write(input: Input) -> Write {
        Write {
            input,
            yes: true,
            idempotency_key: None,
        }
    }
    #[test]
    fn command_tree_accepts_data_options() {
        for cmd in [
            vec!["overview", "get"],
            vec!["catalog", "get"],
            vec!["history", "list"],
            vec!["events", "list"],
            vec!["sql", "read", "--file", "query.json"],
            vec!["sql", "preview", "--file", "query.json"],
            vec!["sql", "write", "--file", "query.json", "--yes"],
            vec![
                "tables",
                "public.items",
                "write",
                "--file",
                "rows.json",
                "--yes",
            ],
            vec!["capture", "settings", "get"],
        ] {
            let mut args = vec![
                "axiom",
                "data",
                "--project",
                "demo",
                "--environment",
                "staging",
                "--json",
            ];
            args.extend(cmd);
            crate::Cli::try_parse_from(args).unwrap();
        }
    }
    #[tokio::test]
    async fn commands_use_dashboard_methods_paths_bodies_and_keys() {
        let dir = tempfile::tempdir().unwrap();
        let mock = Mock::default();
        let base = mock.start().await;
        let api = Api::new(
            &base,
            "access".into(),
            "refresh".into(),
            false,
            StepUpStore::at(dir.path().join("step.json")),
        )
        .unwrap();
        let body = json!({"name":"query","sql":"select 1","revision":1});
        let actions = vec![
            (
                Action::Overview { action: Get::Get },
                "GET",
                "overview",
                Value::Null,
                false,
            ),
            (
                Action::Catalog { action: Get::Get },
                "GET",
                "catalog",
                Value::Null,
                false,
            ),
            (
                Action::History {
                    action: List::List {
                        before: None,
                        action: None,
                    },
                },
                "GET",
                "history",
                Value::Null,
                false,
            ),
            (
                Action::Events {
                    action: List::List {
                        before: None,
                        action: None,
                    },
                },
                "GET",
                "events",
                Value::Null,
                false,
            ),
            (
                Action::SavedQueries {
                    action: Saved::List,
                },
                "GET",
                "saved-queries",
                Value::Null,
                false,
            ),
            (
                Action::Capture {
                    action: Capture::Settings {
                        action: Settings::Get,
                    },
                },
                "GET",
                "capture/settings",
                Value::Null,
                false,
            ),
            (
                Action::SavedQueries {
                    action: Saved::Create(write(file(&dir, body.clone()))),
                },
                "POST",
                "saved-queries",
                body.clone(),
                true,
            ),
            (
                Action::SavedQueries {
                    action: Saved::Update {
                        id: uuid::Uuid::nil(),
                        write: write(file(&dir, body.clone())),
                    },
                },
                "PUT",
                "saved-queries/00000000-0000-0000-0000-000000000000",
                body.clone(),
                true,
            ),
            (
                Action::SavedQueries {
                    action: Saved::Delete {
                        id: uuid::Uuid::nil(),
                        yes: true,
                    },
                },
                "DELETE",
                "saved-queries/00000000-0000-0000-0000-000000000000",
                Value::Null,
                false,
            ),
            (
                Action::Sql {
                    action: Sql::Read(file(&dir, body.clone())),
                },
                "POST",
                "sql/read",
                json!({"sql":"select 1","params":[]}),
                false,
            ),
            (
                Action::Sql {
                    action: Sql::Preview(file(&dir, body.clone())),
                },
                "POST",
                "sql/write/preview",
                json!({"sql":"select 1","params":[]}),
                false,
            ),
            (
                Action::Tables {
                    table: "items".into(),
                    action: Table::Query(file(&dir, body.clone())),
                },
                "POST",
                "tables/items/rows:query",
                body.clone(),
                false,
            ),
            (
                Action::Tables {
                    table: "items".into(),
                    action: Table::Cell(file(&dir, body.clone())),
                },
                "POST",
                "tables/items/cell",
                body.clone(),
                false,
            ),
            (
                Action::Tables {
                    table: "items".into(),
                    action: Table::Count(file(&dir, body.clone())),
                },
                "POST",
                "tables/items/count",
                body.clone(),
                false,
            ),
            (
                Action::Capture {
                    action: Capture::Preview(file(&dir, body.clone())),
                },
                "POST",
                "capture/preview",
                body.clone(),
                false,
            ),
            (
                Action::Capture {
                    action: Capture::Settings {
                        action: Settings::Set(write(file(&dir, body.clone()))),
                    },
                },
                "PUT",
                "capture/settings",
                body.clone(),
                true,
            ),
        ];
        for (action, method, path, expected, keyed) in actions {
            let route = format!("{method} /api/v1/projects/demo/data/environments/env/{path}");
            mock.script(&route, 200, json!({"ok":true}));
            run(&api, "demo", "env", &action).await.unwrap();
            let seen = mock.seen(&route);
            let seen = seen.last().unwrap();
            assert_eq!(seen.body, expected);
            assert_eq!(seen.key.is_some(), keyed);
            assert_eq!(seen.bearer.as_deref(), Some("access"));
        }
    }
    #[tokio::test]
    async fn sql_write_binds_preview_counts_head_and_key() {
        let dir = tempfile::tempdir().unwrap();
        let mock = Mock::default();
        let base = mock.start().await;
        let api = Api::new(
            &base,
            "access".into(),
            "refresh".into(),
            false,
            StepUpStore::at(dir.path().join("step")),
        )
        .unwrap();
        let root = "/api/v1/projects/demo/data/environments/env";
        mock.script(
            &format!("GET {root}/overview"),
            200,
            json!({"limits":{"confirm_max":10}}),
        );
        mock.script(
            &format!("POST {root}/sql/write/preview"),
            200,
            json!({"affected":2,"total_changed":4,"head":{"version":"v1"}}),
        );
        mock.script(
            &format!("POST {root}/sql/write/execute"),
            200,
            json!({"receipt":{"state":"committed"}}),
        );
        let action = Action::Sql {
            action: Sql::Write(fresh_write(file(
                &dir,
                json!({"sql":"delete from items","params":[],"confirmed_affected":999}),
            ))),
        };
        run(&api, "demo", "env", &action).await.unwrap();
        let seen = mock.seen(&format!("POST {root}/sql/write/execute"));
        assert_eq!(seen.len(), 1);
        assert_eq!(
            seen[0].body,
            json!({"sql":"delete from items","params":[],"confirmed_affected":2,"confirmed_total":4,"head":{"version":"v1"}})
        );
        assert!(uuid::Uuid::parse_str(seen[0].key.as_deref().unwrap()).is_ok());
    }
    #[tokio::test]
    async fn row_write_reviews_catalog_and_keeps_originals() {
        let dir = tempfile::tempdir().unwrap();
        let mock = Mock::default();
        let base = mock.start().await;
        let api = Api::new(
            &base,
            "access".into(),
            "refresh".into(),
            false,
            StepUpStore::at(dir.path().join("step")),
        )
        .unwrap();
        let root = "/api/v1/projects/demo/data/environments/env";
        mock.script(&format!("GET {root}/catalog"), 200, json!({"tables":[]}));
        let route = format!("POST {root}/tables/items/rows:write");
        mock.script(&route, 200, json!({"receipt":{"state":"committed"}}));
        let body = json!({"operations":[{"op":"delete","key":{"id":{"t":"integer","v":1}},"original":{"name":{"t":"text","v":"old"}}}]});
        run(
            &api,
            "demo",
            "env",
            &Action::Tables {
                table: "items".into(),
                action: Table::Write(write(file(&dir, body.clone()))),
            },
        )
        .await
        .unwrap();
        assert_eq!(mock.seen(&route)[0].body, body);
        assert_eq!(mock.seen(&format!("GET {root}/catalog")).len(), 1);
    }
    #[tokio::test]
    async fn capture_uses_fresh_review_hashes_and_version() {
        use sha2::{Digest, Sha256};
        let dir = tempfile::tempdir().unwrap();
        let mock = Mock::default();
        let base = mock.start().await;
        let api = Api::new(
            &base,
            "access".into(),
            "refresh".into(),
            false,
            StepUpStore::at(dir.path().join("step")),
        )
        .unwrap();
        let statements = json!([{"sql":"create table items(id integer)"}]);
        let hash = hex::encode(Sha256::digest(serde_jcs::to_vec(&statements).unwrap()));
        let preview = json!({"statements":statements,"statements_sha256":hash,"parent_sha256":"a".repeat(64),"sql_sha256":"b".repeat(64),"version":"fresh"});
        let root = "/api/v1/projects/demo/data/environments/env";
        mock.script(&format!("POST {root}/capture/preview"), 200, preview);
        mock.script(&format!("POST {root}/capture"), 200, json!({"ok":true}));
        let body =
            json!({"sql":"create table items(id integer)","title":"Items","version":"stale"});
        run(
            &api,
            "demo",
            "env",
            &Action::Capture {
                action: Capture::Create(fresh_write(file(&dir, body))),
            },
        )
        .await
        .unwrap();
        let seen = mock.seen(&format!("POST {root}/capture"));
        assert_eq!(
            seen[0].body,
            json!({"sql":"create table items(id integer)","title":"Items","version":"fresh","parent_sha256":"a".repeat(64),"sql_sha256":"b".repeat(64),"statements_sha256":hash})
        );
        assert!(seen[0].key.is_some());
    }
    #[tokio::test]
    async fn table_export_posts_filters_to_dashboard_route() {
        use axum::{routing::post, Json};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let app = axum::Router::new().route(
            "/api/v1/projects/demo/data/environments/env/tables/items/export",
            post(|Json(body): Json<Value>| async move {
                assert_eq!(body, json!({"filters":[]}));
                ([("content-type", "text/csv")], "id\n1\n# complete 1 rows\n")
            }),
        );
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let dir = tempfile::tempdir().unwrap();
        let api = Api::new(
            &base,
            "access".into(),
            "refresh".into(),
            false,
            StepUpStore::at(dir.path().join("step")),
        )
        .unwrap();
        let result = run(
            &api,
            "demo",
            "env",
            &Action::Tables {
                table: "items".into(),
                action: Table::Export(file(&dir, json!({"filters":[]}))),
            },
        )
        .await
        .unwrap();
        assert_eq!(result["csv"], "id\n1\n# complete 1 rows\n");
    }
    #[test]
    fn capture_hash_mismatch_refuses_execution() {
        assert!(capture_body(
            &json!({"sql":"select 1","title":"test"}),
            &json!({"statements":[],"statements_sha256":"wrong"})
        )
        .is_err());
    }
    #[test]
    fn unsafe_previews_and_uncertain_receipts_do_not_succeed() {
        let body = json!({"sql":"delete from items"});
        for preview in [
            json!({}),
            json!({"affected":-1}),
            json!({"affected":10001,"head":{}}),
            json!({"affected":1,"total_changed":-1,"head":{}}),
        ] {
            assert!(execute_body(&body, &preview, 10000).is_err());
        }
        for state in ["pending", "unknown", "rolled_back", "failed", ""] {
            assert!(receipt(json!({"receipt":{"state":state}})).is_err());
        }
    }
}
