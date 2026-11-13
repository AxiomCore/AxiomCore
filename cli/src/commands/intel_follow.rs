//! Streaming and snapshot following through the existing Cloud transport.
use super::{print_value, read_json};
use anyhow::{bail, Context, Result};
use axiom_cloud::CloudClient;
use serde_json::Value;
use std::collections::{HashSet, VecDeque};
use std::time::Duration;

pub(super) async fn activity(
    client: &CloudClient,
    segments: &[&str],
    query: &[(String, String)],
    json: bool,
    follow: bool,
) -> Result<()> {
    let run = async {
        let mut attempt = 0;
        loop {
            let result = async {
                let response = client
                    .dashboard_request(reqwest::Method::GET, segments, query, None)
                    .await?;
                consume(response, follow, |value| print_value(value, json)).await
            }
            .await;
            if !follow {
                return result;
            }
            // EOF and failed requests both reconnect. Never spin when the server
            // repeatedly accepts and immediately closes the connection.
            let delay = reconnect_delay(attempt);
            attempt = attempt.saturating_add(1);
            if result.is_err() {
                eprintln!(
                    "Activity connection interrupted; reconnecting in {} seconds.",
                    delay.as_secs()
                );
            }
            tokio::time::sleep(delay).await;
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => Ok(()),
        result = run => result,
    }
}

fn reconnect_delay(attempt: u32) -> Duration {
    Duration::from_secs((1_u64 << attempt.min(5)).min(30))
}

pub(super) async fn logs(
    client: &CloudClient,
    segments: &[&str],
    query: &[(String, String)],
    json: bool,
    interval: Duration,
) -> Result<()> {
    let run = async {
        let mut seen = LogLines::default();
        loop {
            let response = client
                .dashboard_request(reqwest::Method::GET, segments, query, None)
                .await?;
            let value = read_json(response).await?;
            // The API returns ProjectLog[], whose id uniquely identifies a row.
            for row in seen.new_lines(value)? {
                print_value(&row, json)?;
            }
            tokio::time::sleep(interval).await;
        }
    };
    // This also cancels a pending connection, body read or retry, not just sleep.
    tokio::select! {
        _ = tokio::signal::ctrl_c() => Ok(()),
        result = run => result,
    }
}

#[derive(Default)]
struct LogLines {
    ids: HashSet<String>,
    order: VecDeque<String>,
}
impl LogLines {
    fn new_lines(&mut self, value: Value) -> Result<Vec<Value>> {
        let Value::Array(rows) = value else {
            bail!("Cloud returned invalid logs: expected an array");
        };
        let mut fresh = Vec::new();
        for row in rows {
            let id = row
                .get("id")
                .and_then(Value::as_str)
                .context("Cloud log row is missing its id")?
                .to_owned();
            if self.ids.insert(id.clone()) {
                self.order.push_back(id);
                fresh.push(row);
            }
        }
        // Bound memory for long-running follows while retaining far more rows
        // than the default 100-row snapshot.
        while self.order.len() > 10_000 {
            if let Some(id) = self.order.pop_front() {
                self.ids.remove(&id);
            }
        }
        Ok(fresh)
    }
}

#[cfg(test)]
pub(super) async fn stream(response: reqwest::Response, json: bool, follow: bool) -> Result<()> {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => Ok(()),
        result = consume(response, follow, |v| print_value(v,json)) => result,
    }
}

async fn consume(
    mut response: reqwest::Response,
    follow: bool,
    mut emit: impl FnMut(&Value) -> Result<()>,
) -> Result<()> {
    if response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_none_or(|v| v.split(';').next() != Some("text/event-stream"))
    {
        bail!("Cloud returned an invalid activity stream content type");
    }
    let mut buffer = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        buffer.extend_from_slice(&chunk);
        while let Some((end, separator)) = frame_end(&buffer) {
            if end > 2 * 1024 * 1024 {
                bail!("Activity event exceeds 2 MiB");
            }
            let frame: Vec<_> = buffer.drain(..end + separator).collect();
            let frame = String::from_utf8(frame).context("Invalid UTF-8 activity event")?;
            let event = frame
                .lines()
                .find_map(|l| l.strip_prefix("event:").map(str::trim));
            let data = frame
                .lines()
                .filter_map(|l| l.strip_prefix("data:").map(str::trim_start))
                .collect::<Vec<_>>()
                .join("\n");
            if data.is_empty() {
                continue;
            }
            let value: Value =
                serde_json::from_str(&data).context("Invalid activity event JSON")?;
            if event == Some("ready")
                || matches!(
                    value.get("type").and_then(Value::as_str),
                    Some("ready" | "dashboard.ready")
                )
            {
                continue;
            }
            emit(&value)?;
            if !follow {
                return Ok(());
            }
        }
        if buffer.len() > 2 * 1024 * 1024 {
            bail!("Activity event exceeds 2 MiB");
        }
    }
    Ok(())
}

pub(super) fn frame_end(buffer: &[u8]) -> Option<(usize, usize)> {
    let lf = buffer.windows(2).position(|w| w == b"\n\n").map(|i| (i, 2));
    let crlf = buffer
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| (i, 4));
    lf.into_iter().chain(crlf).min_by_key(|(i, _)| *i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn activity_reconnect_backoff_is_bounded() {
        assert_eq!(
            (0..8)
                .map(|i| reconnect_delay(i).as_secs())
                .collect::<Vec<_>>(),
            [1, 2, 4, 8, 16, 30, 30, 30]
        );
        assert_eq!(reconnect_delay(u32::MAX), Duration::from_secs(30));
    }

    #[test]
    fn logs_follow_emits_only_new_ids_from_real_snapshot_shape() {
        let row = |id: &str, message: &str| json!({"id":id,"source":"runtime","level":"info","stage":"run","message":message,"context":"{}","created_at":"2026-11-13T12:00:00Z"});
        let a = row("a", "Started");
        let b = row("b", "Started");
        let c = row("c", "Completed");
        let mut seen = LogLines::default();
        assert_eq!(
            seen.new_lines(json!([a, b])).unwrap(),
            vec![a.clone(), b.clone()]
        );
        assert_eq!(seen.new_lines(json!([b, c])).unwrap(), vec![c.clone()]);
        assert!(seen.new_lines(json!([c, b])).unwrap().is_empty());
        assert!(seen.new_lines(json!([a, c])).unwrap().is_empty());
    }

    async fn sse_server(body: &'static str) -> String {
        use axum::{body::Body, response::Response, routing::get};
        let router = axum::Router::new().route(
            "/stream",
            get(move || async move {
                Response::builder()
                    .header("content-type", "text/event-stream")
                    .body(Body::from(body))
                    .unwrap()
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/stream", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        url
    }

    #[tokio::test]
    async fn activity_without_follow_skips_ready_and_emits_first_change() {
        let url = sse_server("event: ready\ndata: {\"type\":\"dashboard.ready\"}\n\nevent: change\ndata: {\"type\":\"project.updated\",\"project_id\":\"p1\"}\n\nevent: change\ndata: {\"type\":\"ignored.second\"}\n\n").await;
        let mut values = Vec::new();
        consume(reqwest::get(url).await.unwrap(), false, |v| {
            values.push(v.clone());
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(
            values,
            vec![json!({"type":"project.updated","project_id":"p1"})]
        );
    }

    #[tokio::test]
    async fn activity_follow_reconnects_after_eof() {
        use axum::{body::Body, response::Response, routing::get};
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let count = Arc::new(AtomicUsize::new(0));
        let requests = count.clone();
        let router = axum::Router::new().route(
            "/api/v1/activity/stream",
            get(move || {
                let requests = requests.clone();
                async move {
                    requests.fetch_add(1, Ordering::SeqCst);
                    Response::builder()
                        .header("content-type", "text/event-stream")
                        .body(Body::from(
                            "event: ready\ndata: {\"type\":\"dashboard.ready\"}\n\n",
                        ))
                        .unwrap()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let client = CloudClient::for_loopback(&url, "test".into(), "refresh".into()).unwrap();
        let observed = async {
            while count.load(Ordering::SeqCst) < 2 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };
        tokio::select! {
            result = activity(&client, &["activity", "stream"], &[], true, true) => panic!("follow ended unexpectedly: {result:?}"),
            result = tokio::time::timeout(Duration::from_secs(30), observed) => result.unwrap(),
        }
    }
}
