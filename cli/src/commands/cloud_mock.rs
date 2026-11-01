//! A scripted loopback Cloud API for command tests: each `METHOD /path` answers its queued
//! replies in order (the last one repeats) and every request is recorded.
use axum::body::Bytes;
use axum::http::{HeaderMap, Method, Uri};
use axum::response::Response;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct Seen {
    pub route: String,
    pub query: Option<String>,
    pub bearer: Option<String>,
    pub key: Option<String>,
    pub body: Value,
}

#[derive(Default)]
struct State {
    scripts: HashMap<String, VecDeque<(u16, Value)>>,
    seen: Vec<Seen>,
}

#[derive(Clone, Default)]
pub struct Mock(Arc<Mutex<State>>);

impl Mock {
    /// Serves on a free loopback port; returns the base URL.
    pub async fn start(&self) -> String {
        let router = axum::Router::new().fallback(handler).with_state(self.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        base
    }
    pub fn script(&self, route: &str, status: u16, body: Value) -> &Self {
        self.0.lock().unwrap().scripts.entry(route.to_string()).or_default().push_back((status, body));
        self
    }
    pub fn seen(&self, route: &str) -> Vec<Seen> {
        self.0.lock().unwrap().seen.iter().filter(|s| s.route == route).cloned().collect()
    }
}

async fn handler(axum::extract::State(mock): axum::extract::State<Mock>, method: Method, uri: Uri, headers: HeaderMap, body: Bytes) -> Response {
    let route = format!("{method} {}", uri.path());
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_string);
    let mut state = mock.0.lock().unwrap();
    state.seen.push(Seen {
        route: route.clone(),
        query: uri.query().map(str::to_string),
        bearer: header("authorization").and_then(|v| v.strip_prefix("Bearer ").map(str::to_string)),
        key: header("idempotency-key"),
        body: serde_json::from_slice(&body).unwrap_or(Value::Null),
    });
    let answer = match state.scripts.get_mut(&route) {
        Some(queue) if queue.len() > 1 => queue.pop_front(),
        Some(queue) => queue.front().cloned(),
        None => None,
    };
    let (status, body) = answer.unwrap_or((404, json!({"code": "AXRESOURCE_INVALID"})));
    Response::builder().status(status).header("content-type", "application/json").body(axum::body::Body::from(body.to_string())).unwrap()
}
