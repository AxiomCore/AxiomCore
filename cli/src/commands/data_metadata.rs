//! Cloud metadata reads shared with the data editor command group.
use anyhow::{bail, Context, Result};
use axiom_cloud::CloudClient;
use clap::{Args, Subcommand};
use reqwest::Method;
use serde_json::Value;

#[derive(Args, Clone, Debug)]
pub struct Selection {
    /// Project slug, display name, or ID
    #[arg(long)]
    pub project: String,
    /// Configuration environment slug, name, or ID
    #[arg(long)]
    pub environment: String,
    /// Print the API response as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Subcommand, Clone, Debug)]
pub enum SecretsAction {
    /// List secret metadata visible in this environment (never secret material)
    List(Selection),
    /// List secret bindings for this environment
    Bindings(Selection),
}

#[derive(Args, Debug)]
pub struct ImpactArgs {
    /// SHA-256 of the uploaded migration release
    pub release: String,
    #[command(flatten)]
    pub selection: Selection,
}

fn select<'a>(items: &'a Value, input: &str, kind: &str) -> Result<&'a Value> {
    let items = items.as_array().context("Invalid selection response")?;
    // An exact identifier takes precedence over a coincidentally equal display name.
    let exact: Vec<_> = items
        .iter()
        .filter(|v| ["id", "slug"].iter().any(|k| v[*k].as_str() == Some(input)))
        .collect();
    let matches = if exact.is_empty() {
        items
            .iter()
            .filter(|v| v["name"].as_str() == Some(input))
            .collect::<Vec<_>>()
    } else {
        exact
    };
    match matches.as_slice() {
        [item] => Ok(item),
        [] => bail!("No matching {kind}; use an existing name or ID"),
        _ => bail!("Ambiguous {kind} name; pass its ID"),
    }
}

/// Resolve dashboard names/IDs to a project slug and environment UUID.
pub async fn resolve(
    client: &CloudClient,
    project: &str,
    environment: &str,
) -> Result<(String, String)> {
    let projects = client
        .resource_request(Method::GET, &["projects"], None, None)
        .await?;
    let project = select(&projects, project, "project")?["slug"]
        .as_str()
        .context("Project has no slug")?
        .to_owned();
    let environments = client
        .secret_metadata(&project, &["configuration-environments"], Method::GET, None)
        .await?;
    let environment = select(&environments, environment, "environment")?["id"]
        .as_str()
        .context("Environment has no ID")?
        .to_owned();
    Ok((project, environment))
}

pub async fn secrets_read(client: &CloudClient, action: &SecretsAction) -> Result<Value> {
    let (selection, part) = match action {
        SecretsAction::List(s) => (s, "secrets"),
        SecretsAction::Bindings(s) => (s, "secret-bindings"),
    };
    let (project, environment) =
        resolve(client, &selection.project, &selection.environment).await?;
    client
        .secret_metadata(
            &project,
            &["configuration-environments", &environment, part],
            Method::GET,
            None,
        )
        .await
}

pub async fn handle_secrets(action: &SecretsAction) -> Result<()> {
    let client = crate::auth_store::authenticated_cloud_client()?;
    let result = secrets_read(&client, action).await?;
    let (selection, label) = match action {
        SecretsAction::List(s) => (s, "Environment secrets"),
        SecretsAction::Bindings(s) => (s, "Environment secret bindings"),
    };
    print(&result, selection.json, label)
}

pub async fn impact_read(client: &CloudClient, args: &ImpactArgs) -> Result<Value> {
    let (project, _) =
        resolve(client, &args.selection.project, &args.selection.environment).await?;
    match client.migration_impact(&project, &args.release).await? {
        Some(bytes) => serde_json::from_slice(&bytes).context("Invalid impact response"),
        None => Ok(Value::Null),
    }
}

pub async fn handle_impact(args: &ImpactArgs) -> Result<()> {
    let client = crate::auth_store::authenticated_cloud_client()?;
    let result = impact_read(&client, args).await?;
    print(&result, args.selection.json, "Migration release impact")
}

fn print(value: &Value, json: bool, label: &str) -> Result<()> {
    if !json {
        println!("{label}:");
        if value.is_null() {
            println!("No stored impact report is available.");
            return Ok(());
        }
        if value.as_array().is_some_and(Vec::is_empty) {
            println!("No entries.");
            return Ok(());
        }
    }
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cloud_mock::Mock;
    use serde_json::json;
    const ENV: &str = "11111111-1111-4111-8111-111111111111";
    async fn setup() -> (Mock, CloudClient) {
        let mock = Mock::default();
        mock.script(
            "GET /api/v1/projects",
            200,
            json!([{"id":"project-id","slug":"demo","name":"Demo project"}]),
        )
        .script(
            "GET /api/v1/projects/demo/configuration-environments",
            200,
            json!([{"id":ENV,"slug":"development","name":"Development"}]),
        );
        let client =
            CloudClient::for_loopback(&mock.start().await, "test-token".into(), "refresh".into())
                .unwrap();
        (mock, client)
    }
    fn selection() -> Selection {
        Selection {
            project: "Demo project".into(),
            environment: "development".into(),
            json: true,
        }
    }
    #[tokio::test]
    async fn environment_reads_use_dashboard_routes_without_bodies() {
        let (mock, client) = setup().await;
        for (action, part) in [
            (SecretsAction::List(selection()), "secrets"),
            (SecretsAction::Bindings(selection()), "secret-bindings"),
        ] {
            let route =
                format!("GET /api/v1/projects/demo/configuration-environments/{ENV}/{part}");
            mock.script(&route, 200, json!([{ "id": "metadata-id" }]));
            assert_eq!(
                secrets_read(&client, &action).await.unwrap(),
                json!([{ "id": "metadata-id" }])
            );
            let seen = mock.seen(&route);
            assert_eq!(seen.len(), 1);
            assert!(seen[0].body.is_null());
            assert_eq!(seen[0].bearer.as_deref(), Some("test-token"));
            assert!(seen[0].query.is_none());
        }
    }
    #[tokio::test]
    async fn release_impact_uses_dashboard_route_and_handles_missing_report() {
        let (mock, client) = setup().await;
        let args = ImpactArgs {
            release: "a".repeat(64),
            selection: selection(),
        };
        let route = format!(
            "GET /api/v1/projects/demo/migrations/releases/{}/impact",
            args.release
        );
        mock.script(&route, 200, json!({"report": {"coverage": "complete"}}));
        mock.script(&route, 404, json!({"code":"AXRESOURCE_DENIED"}));
        assert_eq!(
            impact_read(&client, &args).await.unwrap()["report"]["coverage"],
            "complete"
        );
        assert!(impact_read(&client, &args).await.unwrap().is_null());
        assert!(mock
            .seen(&route)
            .iter()
            .all(|r| r.body.is_null() && r.query.is_none()));
    }
    #[tokio::test]
    async fn resolve_accepts_ids_and_rejects_unknown_names() {
        let (_, client) = setup().await;
        assert_eq!(
            resolve(&client, "project-id", ENV).await.unwrap(),
            ("demo".into(), ENV.into())
        );
        assert!(resolve(&client, "absent", ENV).await.is_err());
        assert!(resolve(&client, "demo", "absent").await.is_err());
    }
    #[test]
    fn ambiguous_names_fail_and_exact_identifiers_win() {
        let entries = json!([{"id":"a","name":"same"},{"id":"b","name":"same"}]);
        assert!(select(&entries, "same", "environment").is_err());
        assert_eq!(select(&entries, "a", "environment").unwrap()["id"], "a");
    }
}
