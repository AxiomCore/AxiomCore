//! Project intelligence through the shared authenticated Cloud transport.
use super::intel_routes::IntelCommand;
use anyhow::{bail, Context, Result};
use axiom_cloud::CloudClient;
use clap::Args;
use serde_json::Value;
use std::io::Write;
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
    let request = args.command.request()?;
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
        for segment in &mut segments {
            if segment == "{project}" {
                *segment = project.slug.clone();
            }
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
    loop {
        let response = client
            .dashboard_request(
                reqwest::Method::from_bytes(request.method.as_bytes())?,
                &segments,
                &args.query,
                request.body.as_ref(),
            )
            .await?;
        if request.path == "/api/v1/activity/stream" {
            return stream(response, args.json, request.follow).await;
        }
        let mut value = read_json(response).await?;
        if request.sensitive && !args.show_secrets {
            redact(&mut value);
        }
        print_value(&value, args.json)?;
        if !request.follow {
            return Ok(());
        }
        // Logs are a snapshot endpoint, like the dashboard: follow polls the same
        // route, emitting complete snapshots so repeated rows retain context.
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(()),
            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
        }
    }
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
                if ["dsn", "token", "secret", "credential", "authorization"]
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
        println!("Cloud result:\n{}", serde_json::to_string_pretty(value)?);
    }
    std::io::stdout().flush()?;
    Ok(())
}
async fn stream(mut response: reqwest::Response, json: bool, follow: bool) -> Result<()> {
    if response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_none_or(|v| v.split(';').next() != Some("text/event-stream"))
    {
        bail!("Cloud returned an invalid activity stream content type");
    }
    let mut buffer = Vec::new();
    loop {
        let chunk = tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(()),
            chunk = response.chunk() => chunk?,
        };
        let Some(chunk) = chunk else {
            return Ok(());
        };
        buffer.extend_from_slice(&chunk);
        while let Some((end, separator)) = frame_end(&buffer) {
            if end > 2 * 1024 * 1024 {
                bail!("Activity event exceeds 2 MiB");
            }
            let frame: Vec<_> = buffer.drain(..end + separator).collect();
            let frame = String::from_utf8(frame).context("Invalid UTF-8 activity event")?;
            let data = frame
                .lines()
                .filter_map(|l| l.strip_prefix("data:").map(str::trim_start))
                .collect::<Vec<_>>()
                .join("\n");
            if data.is_empty() {
                continue;
            }
            print_value(
                &serde_json::from_str::<Value>(&data).context("Invalid activity event JSON")?,
                json,
            )?;
            if !follow {
                return Ok(());
            }
        }
        if buffer.len() > 2 * 1024 * 1024 {
            bail!("Activity event exceeds 2 MiB");
        }
    }
}
fn frame_end(buffer: &[u8]) -> Option<(usize, usize)> {
    let lf = buffer.windows(2).position(|w| w == b"\n\n").map(|i| (i, 2));
    let crlf = buffer
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| (i, 4));
    lf.into_iter().chain(crlf).min_by_key(|(i, _)| *i)
}
#[cfg(test)]
#[path = "intel_tests.rs"]
mod tests;
