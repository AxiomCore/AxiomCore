//! Project intelligence through the shared authenticated Cloud transport.
use super::intel_routes::IntelCommand;
use anyhow::{bail, Context, Result};
use axiom_cloud::CloudClient;
use clap::Args;
use serde_json::Value;
use std::io::Write;
#[path = "intel_follow.rs"]
mod follow;
#[cfg(test)]
use follow::{frame_end, stream};
use std::time::Duration;

#[derive(Debug, Args)]
pub struct IntelArgs {
    /// Project name, slug, or ID (defaults to this directory's link).
    #[arg(long, global = true)]
    pub project: Option<String>,
    /// Validate an environment name or ID. These APIs return project-wide data.
    #[arg(long, global = true)]
    pub environment: Option<String>,
    #[arg(long, global = true)]
    pub json: bool,
    /// Confirm a destructive change without a prompt.
    #[arg(long, global = true)]
    pub yes: bool,
    /// Reveal the one-time OTLP source credential.
    #[arg(long, global = true)]
    pub show_secrets: bool,
    /// API query filter, repeatable (for example --query limit=20).
    #[arg(long, global = true, value_parser = parse_query)]
    pub query: Vec<(String, String)>,
    #[command(subcommand)]
    pub command: IntelCommand,
}
fn parse_query(input: &str) -> Result<(String, String), String> {
    let (key, value) = input.split_once('=').ok_or("expected KEY=VALUE")?;
    if key.is_empty() {
        return Err("query key must not be empty".into());
    }
    Ok((key.into(), value.into()))
}
pub async fn handle(args: &IntelArgs) -> Result<()> {
    execute(&crate::auth_store::authenticated_cloud_client()?, args).await
}
pub async fn execute(client: &CloudClient, args: &IntelArgs) -> Result<()> {
    let mut request = args.command.request()?;
    if request.destructive && !args.yes {
        bail!("This changes project configuration or revokes access. Review the request and pass --yes to confirm.");
    }
    let mut segments: Vec<String> = request
        .path
        .trim_start_matches("/api/v1/")
        .split('/')
        .map(decode_segment)
        .collect::<Result<_>>()?;
    if segments.iter().any(|s| s == "{project}")
        || args.project.is_some()
        || args.environment.is_some()
    {
        let selected = match &args.project {
            Some(p) => p.clone(),
            None => crate::auth_store::get_project_id(&std::env::current_dir()?)?
                .context("Pass --project with a project name, slug, or ID")?,
        };
        let projects = client.list_projects().await?;
        let matches: Vec<_> = projects
            .iter()
            .filter(|p| p.id == selected || p.slug == selected || p.name == selected)
            .collect();
        if matches.len() != 1 {
            bail!("Project selection is missing or ambiguous; pass a unique project slug or ID");
        }
        let project = matches[0];
        if segments.first().map(String::as_str) == Some("projects")
            && segments.get(1).map(String::as_str) == Some("{project}")
        {
            segments[1] = project.slug.clone();
        }
        if let Some(environment) = &args.environment {
            let environments = client.configuration_environments(&project.slug).await?;
            let matches = environments
                .iter()
                .filter(|v| {
                    ["id", "slug", "name"]
                        .iter()
                        .any(|k| v.get(k).and_then(Value::as_str) == Some(environment))
                })
                .count();
            if matches != 1 {
                bail!(
                    "Environment selection is missing or ambiguous; pass a unique environment ID"
                );
            }
            if !args.json {
                eprintln!("Environment validated. This endpoint returns project-wide data.");
            }
        }
    }
    let segments: Vec<&str> = segments.iter().map(String::as_str).collect();
    if let Some(path) = args.command.update_read_path(&segments) {
        let path: Vec<&str> = path.iter().map(String::as_str).collect();
        let current = read_json(
            client
                .dashboard_request(reqwest::Method::GET, &path, &[], None)
                .await?,
        )
        .await?;
        request.body = args.command.merge_current_body(&current, request.body)?;
    }
    if request.path == "/api/v1/activity/stream" {
        return follow::activity(client, &segments, &args.query, args.json, request.follow).await;
    }
    if request.follow {
        return follow::logs(
            client,
            &segments,
            &args.query,
            args.json,
            args.command.poll_interval(),
        )
        .await;
    }
    let response = client
        .dashboard_request(
            reqwest::Method::from_bytes(request.method.as_bytes())?,
            &segments,
            &args.query,
            request.body.as_ref(),
        )
        .await?;
    let mut value = read_json(response).await?;
    if request.sensitive {
        eprintln!("OTLP credentials are shown only once. Use --show-secrets at creation and store them securely.");
        if !args.show_secrets {
            redact(&mut value);
        }
    }
    print_value(&value, args.json)
}

fn decode_segment(input: &str) -> Result<String> {
    // Paths from the command enum already encode each individual identifier.
    // Decode once before passing segments to the shared URL builder.
    let bytes = input.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            out.push(u8::from_str_radix(&input[i + 1..i + 3], 16)?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Ok(String::from_utf8(out)?)
}
async fn read_json(mut response: reqwest::Response) -> Result<Value> {
    tokio::time::timeout(Duration::from_secs(20), async {
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
                bail!("Cloud response exceeds 2 MiB");
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&bytes).context("Cloud returned invalid JSON")
    })
    .await
    .context("Cloud response timed out")?
}
fn redact(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if [
                    "dsn",
                    "token",
                    "secret",
                    "credential",
                    "authorization",
                    "acore_snippet",
                ]
                .iter()
                .any(|part| key.to_ascii_lowercase().contains(part))
                {
                    *value = Value::String("[redacted; use --show-secrets]".into());
                } else {
                    redact(value);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                redact(item);
            }
        }
        _ => {}
    }
}
fn print_value(value: &Value, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string(value)?);
    } else if value.is_null() {
        println!("Request completed.");
    } else {
        print!("{}", human_value(value));
    }
    std::io::stdout().flush()?;
    Ok(())
}
#[cfg(test)]
#[path = "intel_tests.rs"]
mod tests;

fn human_value(value: &Value) -> String {
    fn render(value: &Value, indent: usize, out: &mut String) {
        let pad = " ".repeat(indent);
        match value {
            Value::Object(fields) => {
                for (key, value) in fields {
                    let label = key.replace('_', " ");
                    let mut chars = label.chars();
                    let label = chars
                        .next()
                        .map(|c| c.to_uppercase().to_string() + chars.as_str())
                        .unwrap_or_default();
                    if value.is_object() || value.is_array() {
                        out.push_str(&format!("{pad}{label}:\n"));
                        render(value, indent + 2, out);
                    } else {
                        let text = match value {
                            Value::String(s) => s.clone(),
                            Value::Bool(b) => if *b { "yes" } else { "no" }.into(),
                            Value::Null => "none".into(),
                            _ => value.to_string(),
                        };
                        out.push_str(&format!("{pad}{label}: {text}\n"));
                    }
                }
            }
            Value::Array(items) => {
                if items.is_empty() {
                    out.push_str(&format!("{pad}No results.\n"));
                }
                for item in items {
                    render(item, indent, out);
                    out.push('\n');
                }
            }
            Value::String(text) => out.push_str(&format!("{pad}{text}\n")),
            _ => out.push_str(&format!("{pad}{value}\n")),
        }
    }
    let mut out = String::new();
    render(value, 0, &mut out);
    out
}
