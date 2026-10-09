//! Explicit cloud contract mock retrieval; never reached from local serving.
use anyhow::{anyhow, Context, Result};
use console::style;
use std::fs;
use std::path::PathBuf;

fn get_cache_paths(project_id: &str) -> Result<(PathBuf, PathBuf)> {
    anyhow::ensure!(
        !project_id.is_empty()
            && project_id.len() <= 128
            && project_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
        "invalid cloud project identifier"
    );
    let mut cache_dir = crate::auth_store::get_config_dir()?;
    cache_dir.push("cache");
    cache_dir.push("mocks");

    fs::create_dir_all(&cache_dir)?;

    let json_path = cache_dir.join(format!("{}.json", project_id));
    let etag_path = cache_dir.join(format!("{}.etag", project_id));

    Ok((json_path, etag_path))
}

pub async fn cloud_configuration() -> Result<String> {
    let current_dir = std::env::current_dir()?;
    let project_slug = crate::auth_store::get_project_id(&current_dir)?.ok_or_else(|| {
        anyhow!("Directory not linked to an Axiom project. Run 'axiom project link'.")
    })?;

    let (json_path, etag_path) = get_cache_paths(&project_slug)?;

    println!(
        "Downloading mock configuration for project {}...",
        style(&project_slug).cyan()
    );

    let client = crate::auth_store::authenticated_cloud_client()?;

    cached_configuration(&json_path, &etag_path, move |etag| async move {
        client.get_mock_config(&project_slug, etag.as_deref()).await
    })
    .await
}

async fn cached_configuration<F, Fut>(
    json_path: &std::path::Path,
    etag_path: &std::path::Path,
    fetch: F,
) -> Result<String>
where
    F: FnOnce(Option<String>) -> Fut,
    Fut: std::future::Future<Output = Result<axiom_cloud::project::MockResponse>>,
{
    let cached = if json_path.is_file() {
        Some(axiom_server::serve::read(json_path, 4_194_304)?)
    } else {
        None
    };
    let etag = if cached.is_some() && etag_path.is_file() {
        Some(
            String::from_utf8(axiom_server::serve::read(etag_path, 1024)?)
                .context("invalid cached mock ETag")?,
        )
    } else {
        None
    };
    match fetch(etag).await? {
        axiom_cloud::project::MockResponse::NotModified => {
            println!("Using cached contract mock configuration (Not Modified).");
            String::from_utf8(
                cached
                    .context("cloud returned Not Modified without an owned cached configuration")?,
            )
            .context("cached contract mock is not UTF-8")
        }
        axiom_cloud::project::MockResponse::Data { json, etag } => {
            anyhow::ensure!(
                json.len() <= 4_194_304 && etag.len() <= 1024,
                "cloud mock configuration or ETag exceeds bounds"
            );
            for (path, bytes) in [(json_path, json.as_bytes()), (etag_path, etag.as_bytes())] {
                use std::io::Write;
                let mut file = tempfile::NamedTempFile::new_in(
                    path.parent().context("invalid cache directory")?,
                )?;
                file.write_all(bytes)?;
                file.as_file().sync_all()?;
                file.persist(path)
                    .context("cannot atomically save cloud mock cache")?;
            }
            println!("Downloaded explicit cloud contract mock configuration.");
            Ok(json)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn explicit_cloud_http_retrieval_preserves_etag_cache_without_touching_user_credentials()
    {
        use axum::{
            http::{HeaderMap, StatusCode},
            response::IntoResponse,
            routing::get,
            Router,
        };
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        let app = Router::new().route(
            "/api/v1/projects/fixture/mocks",
            get(move |headers: HeaderMap| {
                let count = seen.clone();
                async move {
                    assert_eq!(
                        headers.get("authorization").unwrap(),
                        "Bearer isolated-test-token"
                    );
                    let index = count.fetch_add(1, Ordering::SeqCst);
                    if index == 0 {
                        assert!(!headers.contains_key("if-none-match"));
                        (
                            StatusCode::OK,
                            [("etag", "\"fixture-v1\"")],
                            "{\"endpoints\":{}}",
                        )
                            .into_response()
                    } else {
                        assert_eq!(headers.get("if-none-match").unwrap(), "\"fixture-v1\"");
                        StatusCode::NOT_MODIFIED.into_response()
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let temporary = tempfile::tempdir().unwrap();
        let json = temporary.path().join("fixture.json");
        let etag = temporary.path().join("fixture.etag");
        let client = reqwest::Client::new();
        for _ in 0..2 {
            let base = base.clone();
            let client = client.clone();
            let body = cached_configuration(&json, &etag, move |etag| async move {
                axiom_cloud::project::get_mock_config(
                    &client,
                    &base,
                    "fixture",
                    "isolated-test-token",
                    etag.as_deref(),
                )
                .await
            })
            .await
            .unwrap();
            assert_eq!(body, "{\"endpoints\":{}}");
        }
        assert_eq!(count.load(Ordering::SeqCst), 2);
        assert_eq!(std::fs::read_to_string(etag).unwrap(), "\"fixture-v1\"");
        server.abort();
    }
    #[tokio::test]
    async fn cache_miss_not_modified_and_oversize_data_fail_explicitly() {
        let temporary = tempfile::tempdir().unwrap();
        let json = temporary.path().join("fixture.json");
        let etag = temporary.path().join("fixture.etag");
        assert!(cached_configuration(&json, &etag, |etag| async move {
            assert!(etag.is_none());
            Ok(axiom_cloud::project::MockResponse::NotModified)
        })
        .await
        .is_err());
        assert!(cached_configuration(&json, &etag, |_| async {
            Ok(axiom_cloud::project::MockResponse::Data {
                json: "x".repeat(4_194_305),
                etag: String::new(),
            })
        })
        .await
        .is_err());
        assert!(!json.exists());
    }
}
