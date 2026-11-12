//! Placement commands (placement/v1 10.3): bring-your-own Turso `connect`, `connection`,
//! `connection check` and `disconnect`; `placement`; `relocation-consent`; `receipt verify`;
//! and the `convert` action of `request`.
//!
//! The Turso platform API token comes only from standard input (`--token-stdin`) or a masked
//! prompt, never argv, and is never printed, logged or written. Owner actions that need a
//! recent sign-in (connect, disconnect, consent, convert) run the step-up device flow on
//! `AXRESOURCE_REAUTH_REQUIRED` and retry with the same Idempotency-Key. Server text is never
//! echoed; known codes select fixed messages.
use super::resources_recovery::Api;
use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use reqwest::Method;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::time::Duration;
use zeroize::Zeroizing;

/// Which resource: its ID, or the environment it serves.
#[derive(clap::Args, Clone, Debug, Default)]
pub struct Select {
    /// Resource ID (UUID)
    #[arg(long, conflicts_with = "environment")]
    pub resource: Option<String>,
    /// The environment whose database this is, for example `production`
    #[arg(long)]
    pub environment: Option<String>,
}

#[derive(clap::Subcommand, Debug)]
pub enum ConnectionAction {
    /// Ask AxiomCore to verify the connection again now (Owner; 6 per hour)
    Check {
        #[command(flatten)]
        select: Select,
        #[arg(long)]
        operation_id: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(clap::Subcommand, Debug)]
pub enum ReceiptAction {
    /// Verify a move receipt offline against the platform's published receipt keys
    Verify {
        /// The move ID (`placement` lists receipts)
        move_id: String,
        #[command(flatten)]
        select: Select,
        /// A saved `placement --json` output to verify instead of fetching it
        #[arg(long)]
        file: Option<PathBuf>,
        /// A saved `GET /api/v1/resources/receipt-keys` response instead of fetching it
        #[arg(long)]
        keys: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
}

const TOKEN_LIMIT: usize = 8 * 1024;
const CONVERT_WAIT: Duration = Duration::from_secs(60 * 60);
const SHORT_WAIT: Duration = Duration::from_secs(5 * 60);

/// Terminal-safe text from a server value: no control characters, bounded.
fn clean(value: &str) -> String {
    value.chars().filter(|c| !c.is_control()).take(128).collect()
}
fn text(value: &Value, pointer: &str) -> String {
    match value.pointer(pointer) {
        Some(Value::String(s)) if !s.is_empty() => clean(s),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => "-".into(),
    }
}

fn operation_key(operation_id: Option<&str>) -> Result<String> {
    let key = operation_id.map(str::to_string).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    uuid::Uuid::parse_str(&key).map_err(|_| anyhow!("--operation-id must be a UUID"))?;
    Ok(key)
}

fn valid_phrase(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
}

/// A Turso organization, group or database name: 1 to 64 lowercase letters, digits and `-`.
pub fn turso_name(value: &str) -> bool {
    !value.is_empty() && value.len() <= 64 && value.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The resource of `select` from the project's resource summary.
pub async fn resolve(api: &Api, project: &str, select: &Select) -> Result<Value> {
    if select.resource.is_none() && select.environment.is_none() {
        bail!("Pass --resource <UUID> or --environment <name>");
    }
    if let Some(id) = &select.resource {
        uuid::Uuid::parse_str(id).map_err(|_| anyhow!("--resource must be a UUID"))?;
    }
    let summary = api.get(&["projects", project, "resources"]).await?;
    let found = summary["resources"].as_array().into_iter().flatten().find(|r| match (&select.resource, &select.environment) {
        (Some(id), _) => r["id"].as_str() == Some(id.as_str()),
        (None, Some(environment)) => r["environment"].as_str() == Some(environment.as_str()),
        _ => false,
    });
    found.cloned().ok_or_else(|| anyhow!("No database matches; run `axiom project resources --project {project}` to list them"))
}

fn resource_id(resource: &Value) -> Result<String> {
    resource["id"].as_str().map(str::to_string).context("AXRESOURCE_INVALID_RESPONSE: resource without id")
}

// ---------------------------------------------------------------------------
// Connect
// ---------------------------------------------------------------------------

/// Flags of `connect turso`; the token is read separately.
#[derive(Clone, Debug, Default)]
pub struct ConnectInput {
    pub provider: String,
    pub organization: String,
    pub group: String,
    pub database: String,
    pub fence_mode: String,
    pub acknowledge_split_store: bool,
    pub acknowledge_rotate: bool,
    pub reclaim: Option<String>,
}

/// What connecting means, shown before anything is sent.
pub fn connect_consequences(input: &ConnectInput) -> String {
    let mut text = format!(
        "Application data stays in your Turso database {}/{}/{} and is billed by Turso. Authentication data \
         (users, sessions, keys) stays with AxiomCore in its own region; the two stores are not atomic. AxiomCore \
         mints short-lived database tokens with your platform API token and never shows or returns it.",
        clean(&input.organization),
        clean(&input.group),
        clean(&input.database)
    );
    if input.fence_mode == "rotate" {
        text.push_str(" With --fence-mode rotate, AxiomCore rotates the database credentials during moves and disconnects, which invalidates database tokens you minted yourself.");
    }
    text
}

/// Validates the flags and builds `PUT …/connection` (placement/v1 10.1). Nothing here
/// contacts the server.
pub fn connect_body(input: &ConnectInput, token: &str) -> Result<Value> {
    if input.provider != "turso" {
        bail!("Only `turso` can be connected");
    }
    for (flag, value) in [("--organization", &input.organization), ("--group", &input.group), ("--database", &input.database)] {
        if !turso_name(value) {
            bail!("{flag} must be 1 to 64 lowercase letters, digits or -");
        }
    }
    if !["expiry", "rotate"].contains(&input.fence_mode.as_str()) {
        bail!("--fence-mode is expiry or rotate");
    }
    if !input.acknowledge_split_store {
        bail!("{}\n\nTo go ahead, repeat the command with --acknowledge-split-store.", connect_consequences(input));
    }
    if input.fence_mode == "rotate" && !input.acknowledge_rotate {
        bail!("{}\n\n--fence-mode rotate also needs --acknowledge-rotate.", connect_consequences(input));
    }
    if token.is_empty() || token.len() > TOKEN_LIMIT || token.chars().any(char::is_whitespace) {
        bail!("The Turso API token is empty, too long or has spaces; pipe exactly one token to --token-stdin");
    }
    let mut acknowledge = json!({"split_store": true});
    if input.fence_mode == "rotate" {
        acknowledge["rotate_invalidates_my_tokens"] = json!(true);
    }
    let mut body = json!({
        "provider": "turso",
        "organization": input.organization,
        "group": input.group,
        "database": input.database,
        "token": token,
        "fence_mode": input.fence_mode,
        "acknowledge": acknowledge,
    });
    if let Some(reclaim) = &input.reclaim {
        if reclaim != &format!("reclaim {}", input.database) {
            bail!("--reclaim must be exactly \"reclaim {}\"", input.database);
        }
        body["reclaim"] = json!(reclaim);
    }
    Ok(body)
}

/// The token from standard input (bounded, one line) or a masked prompt; never argv.
pub fn read_token(stdin: bool) -> Result<Zeroizing<String>> {
    let raw = if stdin {
        let mut bytes = Zeroizing::new(Vec::new());
        std::io::stdin().take(TOKEN_LIMIT as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > TOKEN_LIMIT {
            bail!("The Turso API token is at most 8 KiB");
        }
        Zeroizing::new(String::from_utf8(bytes.to_vec()).map_err(|_| anyhow!("The Turso API token is not UTF-8"))?)
    } else {
        if !std::io::stdin().is_terminal() {
            bail!("No terminal for the masked prompt; pass --token-stdin and pipe the token");
        }
        Zeroizing::new(dialoguer::Password::new().with_prompt("Turso platform API token").interact()?)
    };
    Ok(Zeroizing::new(raw.trim_end_matches(['\r', '\n']).to_string()))
}

pub async fn connect(api: &Api, project: &str, select: &Select, input: &ConnectInput, token: &str, operation_id: Option<&str>) -> Result<Value> {
    let mut body = connect_body(input, token)?;
    let key = operation_key(operation_id)?;
    let resource = resolve(api, project, select).await?;
    let id = resource_id(&resource)?;
    eprintln!("Operation idempotency key: {key}");
    let result = api.step_up_action(&Method::PUT, &["projects", project, "resources", &id, "connection"], &body, &key).await;
    if let Some(map) = body.as_object_mut() {
        map.remove("token");
    }
    result
}

pub fn render_connection(view: &Value) -> String {
    let c = view.get("connection").unwrap_or(view);
    let mut lines = vec![
        format!("Connection {} {}/{}/{}: {}", text(c, "/provider"), text(c, "/organization"), text(c, "/group"), text(c, "/database"), text(c, "/status")),
        format!("Host {} ({}), fence mode {}", text(c, "/hostname"), text(c, "/primary_location"), text(c, "/fence_mode")),
        format!(
            "Token: {}, expires {}",
            if c.pointer("/token/set") == Some(&Value::Bool(true)) { "set" } else { "not set" },
            match c.pointer("/token/expires_at") {
                Some(Value::String(s)) => clean(s),
                _ => "never".into(),
            }
        ),
        format!("Verified {}; last check {} {} {}", text(c, "/verified_at"), text(c, "/last_check/at"), text(c, "/last_check/outcome"), text(c, "/last_check/code")),
    ];
    if c.get("split").is_some() {
        lines.push(format!(
            "Stores: application data in your Turso database ({}); authentication with AxiomCore ({}, {}); not atomic",
            text(c, "/split/application/location"),
            text(c, "/split/authentication/region"),
            text(c, "/split/authentication/location")
        ));
    }
    if c.pointer("/usage/measured_at").is_some_and(|v| !v.is_null()) {
        lines.push(format!(
            "Usage (billed by Turso): rows read {}, rows written {}, storage {} bytes, measured {}",
            text(c, "/usage/rows_read"),
            text(c, "/usage/rows_written"),
            text(c, "/usage/storage_bytes"),
            text(c, "/usage/measured_at")
        ));
    }
    lines.join("\n")
}

/// The connection view without anything secret-shaped: only `token.set` and
/// `token.expires_at` leave the token object.
pub fn safe_connection(mut view: Value) -> Value {
    fn strip(value: &mut Value) {
        match value {
            Value::Object(map) => {
                if let Some(Value::Object(token)) = map.get_mut("token") {
                    token.retain(|k, _| k == "set" || k == "expires_at");
                } else {
                    map.remove("token");
                }
                map.retain(|k, _| !k.ends_with("_token") && !k.contains("secret"));
                map.values_mut().for_each(strip);
            }
            Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    strip(&mut view);
    view
}

// ---------------------------------------------------------------------------
// Disconnect, consent, convert
// ---------------------------------------------------------------------------

pub fn disconnect_consequences(database: &str) -> String {
    format!(
        "Disconnecting {database} stops AxiomCore from serving application data: leases, editor sessions and \
         migrations answer AXRESOURCE_DISCONNECTED. Authentication keeps working. Database tokens AxiomCore minted \
         stay valid until they expire (at most 5 minutes). Nothing is copied or deleted. AxiomCore cannot revoke \
         your Turso API token: revoke it at Turso with `turso auth api-tokens revoke`."
    )
}

pub fn disconnect_body(generation: u64, database: &str, confirm: Option<&str>) -> Result<Value> {
    let phrase = format!("disconnect {database}");
    match confirm {
        None => bail!("{}\n\nTo go ahead, repeat the command with --confirm \"{phrase}\".", disconnect_consequences(&clean(database))),
        Some(c) if c != phrase || !valid_phrase(c) => bail!("The confirmation does not match. Use --confirm \"{}\".", clean(&phrase)),
        Some(c) => Ok(json!({"generation": generation, "confirm": c})),
    }
}

pub fn consent_body(kind: &str, to_region: Option<&str>, acknowledge_costs: bool, database: &str, confirm: Option<&str>) -> Result<Value> {
    let phrase = match (kind, to_region) {
        ("dedicated", None) => format!("move {database} to dedicated"),
        ("region", Some(region)) if !region.is_empty() && region.len() <= 64 && !region.chars().any(char::is_control) => format!("move {database} to {region}"),
        ("region", _) => bail!("--kind region needs --to-region <region id>"),
        ("dedicated", Some(_)) => bail!("--to-region applies to --kind region only"),
        _ => bail!("--kind is dedicated or region"),
    };
    if !acknowledge_costs {
        bail!("A consent lets an operator move this database once within 7 days; the destination's costs apply. Repeat with --acknowledge-costs.");
    }
    let confirm = confirm.ok_or_else(|| anyhow!("Repeat the command with --confirm \"{}\".", clean(&phrase)))?;
    if confirm != phrase {
        bail!("The confirmation does not match. Use --confirm \"{}\".", clean(&phrase));
    }
    let mut body = json!({"kind": kind, "acknowledge_costs": true, "confirm": confirm});
    if let Some(region) = to_region {
        body["to_region"] = json!(region);
    }
    Ok(body)
}

/// Acknowledgements of the `convert` action (placement/v1 10.1).
#[derive(Clone, Debug, Default)]
pub struct ConvertAcknowledge {
    pub sessions_end: bool,
    pub redeploy: bool,
    pub semantics: bool,
    pub copy: Option<String>,
    pub external_writers: Option<String>,
}

pub fn convert_body(generation: u64, conversion: &str, confirm: Option<&str>, acknowledge: &ConvertAcknowledge) -> Result<Value> {
    uuid::Uuid::parse_str(conversion).map_err(|_| anyhow!("--conversion must be the conversion ID from `axiom database convert submit`"))?;
    if generation == 0 {
        bail!("--generation must be positive");
    }
    if !acknowledge.sessions_end || !acknowledge.redeploy {
        bail!(
            "Converting to PostgreSQL ends every signed-in session and needs a runtime redeploy before the app serves again \
             (W7 runtimes show a lease failure until then). Repeat with --acknowledge-sessions-end and --acknowledge-redeploy."
        );
    }
    let confirm = confirm.ok_or_else(|| anyhow!("Repeat the command with --confirm \"convert <database> to postgresql\"."))?;
    let named = confirm.strip_prefix("convert ").and_then(|r| r.strip_suffix(" to postgresql")).is_some_and(|n| !n.trim().is_empty());
    if !named || !valid_phrase(confirm) {
        bail!("The confirmation does not match. Use --confirm \"convert <database> to postgresql\".");
    }
    let mut ack = json!({"sessions_end": true, "redeploy": true});
    if acknowledge.semantics {
        ack["semantics"] = json!(true);
    }
    for (key, value) in [("copy", &acknowledge.copy), ("external_writers", &acknowledge.external_writers)] {
        if let Some(value) = value {
            if !valid_phrase(value) {
                bail!("--acknowledge-{} must be the text the dashboard shows", key.replace('_', "-"));
            }
            ack[key] = json!(value);
        }
    }
    Ok(json!({"generation": generation, "conversion_id": conversion, "confirm": confirm, "acknowledge": ack}))
}

/// Poll the project's operations until `id` finishes.
pub async fn wait_operation(api: &Api, project: &str, id: &str, limit: Duration) -> Result<Value> {
    let deadline = tokio::time::Instant::now() + limit;
    while tokio::time::Instant::now() < deadline {
        let status = api.get(&["projects", project, "resources"]).await?;
        if let Some(operation) = status["operations"].as_array().and_then(|ops| ops.iter().find(|op| op["id"].as_str() == Some(id))) {
            match operation["status"].as_str() {
                Some("succeeded") => return Ok(operation.clone()),
                Some("failed") => {
                    let code = operation["failure_code"].as_str().unwrap_or_default();
                    match axiom_cloud::resources::code_message(code) {
                        Some(message) => bail!("The operation failed. {code}: {message}"),
                        None => bail!("The operation failed. Inspect the resource for its safe failure code."),
                    }
                }
                _ => {}
            }
        }
        tokio::time::sleep(api.wait_interval).await;
    }
    bail!("The operation is still running. Run `axiom project resources --project {project}` to follow it.")
}

// ---------------------------------------------------------------------------
// Placement and receipts
// ---------------------------------------------------------------------------

pub fn render_placement(view: &Value) -> String {
    let c = &view["current"];
    let mut lines = vec![format!(
        "Generation {}: profile {}, pool {}, region {} ({}), location {}",
        text(c, "/generation"),
        text(c, "/profile"),
        text(c, "/pool_kind"),
        text(c, "/region"),
        text(c, "/jurisdiction"),
        text(c, "/location")
    )];
    for store in c["stores"].as_array().into_iter().flatten() {
        lines.push(format!("  store {}: owner {}, location {}", text(store, "/kind"), text(store, "/owner"), text(store, "/location")));
    }
    lines.push(format!("History: {} placement(s)", view["history"].as_array().map_or(0, Vec::len)));
    for r in view["receipts"].as_array().into_iter().flatten() {
        lines.push(format!(
            "Receipt {} ({}) {}: {} -> {}",
            text(r, "/body/move_id"),
            text(r, "/body/kind"),
            if r["signature"].as_str().is_some_and(|s| !s.is_empty()) { "signed" } else { "not signed yet" },
            text(r, "/body/before/region_id"),
            text(r, "/body/after/region_id")
        ));
    }
    for c in view["consents"].as_array().into_iter().flatten() {
        lines.push(format!("Consent {} {} {} expires {}{}", text(c, "/id"), text(c, "/kind"), text(c, "/to_region"), text(c, "/expires_at"), if c["used"] == true { " (used)" } else { "" }));
    }
    for c in view["conversions"].as_array().into_iter().flatten() {
        lines.push(format!("Conversion {}: {}", text(c, "/id"), text(c, "/status")));
    }
    lines.join("\n")
}

/// Verifies receipt `move_id` of a `placement` view against the receipt keys: the canonical
/// bytes hash to `body_sha256`, they are the displayed body, and the platform's Ed25519
/// signature over them holds for the stated key version. Returns a summary.
pub fn verify_receipt(placement: &Value, keys: &Value, move_id: &str) -> Result<Value> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let receipt = placement["receipts"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|r| r.pointer("/body/move_id").and_then(Value::as_str) == Some(move_id))
        .ok_or_else(|| anyhow!("AXRESOURCE_RECEIPT_INVALID: no receipt for move {}", clean(move_id)))?;
    let canonical = b64.decode(receipt["canonical"].as_str().unwrap_or_default()).map_err(|_| anyhow!("AXRESOURCE_RECEIPT_INVALID: the canonical body is not base64"))?;
    let digest = hex::encode(Sha256::digest(&canonical));
    if receipt["body_sha256"].as_str() != Some(digest.as_str()) {
        bail!("AXRESOURCE_RECEIPT_INVALID: the body does not match body_sha256");
    }
    let parsed: Value = serde_json::from_slice(&canonical).map_err(|_| anyhow!("AXRESOURCE_RECEIPT_INVALID: the canonical body is not JSON"))?;
    if parsed != receipt["body"] || parsed["move_id"].as_str() != Some(move_id) {
        bail!("AXRESOURCE_RECEIPT_INVALID: the displayed body differs from the signed bytes");
    }
    let signature = match receipt["signature"].as_str() {
        Some(s) if !s.is_empty() => b64.decode(s).map_err(|_| anyhow!("AXRESOURCE_RECEIPT_INVALID: the signature is not base64"))?,
        _ => bail!("The receipt for move {} is not signed yet; the platform signs it shortly after activation. Retry later.", clean(move_id)),
    };
    let version = receipt["key_version"].as_str().unwrap_or_default();
    let key = keys["keys"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|k| k["version"].as_str() == Some(version))
        .ok_or_else(|| anyhow!("AXRESOURCE_RECEIPT_INVALID: no published receipt key version {}", clean(version)))?;
    let public: [u8; 32] = b64
        .decode(key["public_key"].as_str().unwrap_or_default())
        .ok()
        .and_then(|k| k.try_into().ok())
        .ok_or_else(|| anyhow!("AXRESOURCE_RECEIPT_INVALID: the receipt key is not a raw Ed25519 key"))?;
    let signature: [u8; 64] = signature.try_into().map_err(|_| anyhow!("AXRESOURCE_RECEIPT_INVALID: the signature is not 64 bytes"))?;
    use ed25519_dalek::Verifier;
    ed25519_dalek::VerifyingKey::from_bytes(&public)
        .map_err(|_| anyhow!("AXRESOURCE_RECEIPT_INVALID: invalid receipt key"))?
        .verify(&canonical, &ed25519_dalek::Signature::from_bytes(&signature))
        .map_err(|_| anyhow!("AXRESOURCE_RECEIPT_INVALID: the platform signature does not hold"))?;
    Ok(json!({
        "move_id": move_id,
        "verified": true,
        "key_version": version,
        "body_sha256": digest,
        "kind": parsed["kind"],
        "before": parsed["before"],
        "after": parsed["after"],
        "retained_until": parsed["retained_until"],
    }))
}

fn read_json(path: &PathBuf) -> Result<Value> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    anyhow::ensure!(bytes.len() <= 4 * 1024 * 1024, "{} is above 4 MiB", path.display());
    serde_json::from_slice(&bytes).with_context(|| format!("{} is not JSON", path.display()))
}

pub async fn receipt(api: &Api, project: &str, action: &ReceiptAction) -> Result<Value> {
    let ReceiptAction::Verify { move_id, select, file, keys, .. } = action;
    let placement = match file {
        Some(path) => read_json(path)?,
        None => {
            let resource = resolve(api, project, select).await?;
            api.get(&["projects", project, "resources", &resource_id(&resource)?, "placement"]).await?
        }
    };
    let keys = match keys {
        Some(path) => read_json(path)?,
        None => api.get(&["resources", "receipt-keys"]).await?,
    };
    verify_receipt(&placement, &keys, move_id)
}

// ---------------------------------------------------------------------------
// Entry points from `axiom project resources`
// ---------------------------------------------------------------------------

fn print(value: &Value, json: bool, human: impl FnOnce(&Value) -> String) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        println!("{}", human(value));
    }
    Ok(())
}

/// `connect turso`.
#[allow(clippy::too_many_arguments)]
pub async fn handle_connect(project: &str, select: &Select, input: &ConnectInput, token_stdin: bool, operation_id: Option<&str>, json: bool) -> Result<()> {
    connect_body(input, "placeholder-validation-only")?;
    let token = read_token(token_stdin)?;
    let api = Api::from_profile()?;
    eprintln!("{}", connect_consequences(input));
    let value = connect(&api, project, select, input, &token, operation_id).await?;
    drop(token);
    print(&safe_connection(value), json, |v| {
        format!("{}\nVerification runs now; follow it with `axiom project resources --project {project} connection`.", render_connection(v))
    })
}

pub async fn handle_connection(project: &str, select: &Select, action: Option<&ConnectionAction>, json: bool) -> Result<()> {
    let api = Api::from_profile()?;
    match action {
        None => {
            let resource = resolve(&api, project, select).await?;
            let view = api.get(&["projects", project, "resources", &resource_id(&resource)?, "connection"]).await?;
            print(&safe_connection(view), json, render_connection)
        }
        Some(ConnectionAction::Check { select, operation_id, json }) => {
            let key = operation_key(operation_id.as_deref())?;
            let resource = resolve(&api, project, select).await?;
            let value = api.write(&Method::POST, &["projects", project, "resources", &resource_id(&resource)?, "connection", "check"], &json!({}), Some(&key)).await?;
            print(&value, *json, |v| format!("Connection check started (operation {}).", text(v, "/id")))
        }
    }
}

pub async fn handle_disconnect(project: &str, select: &Select, confirm: Option<&str>, operation_id: Option<&str>, wait: bool, json: bool) -> Result<()> {
    let key = operation_key(operation_id)?;
    let api = Api::from_profile()?;
    let resource = resolve(&api, project, select).await?;
    let id = resource_id(&resource)?;
    let connection = api.get(&["projects", project, "resources", &id, "connection"]).await?;
    let database = connection.pointer("/connection/database").and_then(Value::as_str).context("AXRESOURCE_INVALID: this resource has no customer connection")?;
    let generation = resource["generation"].as_u64().context("AXRESOURCE_INVALID_RESPONSE: resource without generation")?;
    let body = disconnect_body(generation, database, confirm)?;
    eprintln!("{}", disconnect_consequences(&clean(database)));
    eprintln!("Operation idempotency key: {key}");
    let value = api.step_up_action(&Method::POST, &["projects", project, "resources", &id, "detach"], &body, &key).await?;
    let value = match (wait, value["operation_id"].as_str().or_else(|| value.pointer("/operation/id").and_then(Value::as_str))) {
        (true, Some(op)) => json!({"operation": wait_operation(&api, project, op, SHORT_WAIT).await?}),
        _ => value,
    };
    print(&value, json, |_| format!("Disconnect of {} requested. Revoke the Turso API token at Turso when you no longer need it.", clean(database)))
}

pub async fn handle_placement(project: &str, select: &Select, json: bool) -> Result<()> {
    let api = Api::from_profile()?;
    let resource = resolve(&api, project, select).await?;
    let view = api.get(&["projects", project, "resources", &resource_id(&resource)?, "placement"]).await?;
    print(&view, json, render_placement)
}

#[allow(clippy::too_many_arguments)]
pub async fn handle_consent(project: &str, select: &Select, kind: &str, to_region: Option<&str>, acknowledge_costs: bool, confirm: Option<&str>, operation_id: Option<&str>, json: bool) -> Result<()> {
    let key = operation_key(operation_id)?;
    let api = Api::from_profile()?;
    let resource = resolve(&api, project, select).await?;
    let database = resource["environment"].as_str().unwrap_or_default().to_string();
    let body = consent_body(kind, to_region, acknowledge_costs, &database, confirm)?;
    eprintln!("Operation idempotency key: {key}");
    let value = api.step_up_action(&Method::POST, &["projects", project, "resources", &resource_id(&resource)?, "relocation-consents"], &body, &key).await?;
    print(&value, json, |v| format!("Consent {} recorded; valid until {} and used once.", text(v, "/consent/id"), text(v, "/consent/expires_at")))
}

pub async fn handle_receipt(project: &str, action: &ReceiptAction) -> Result<()> {
    let ReceiptAction::Verify { move_id, json, file, keys, .. } = action;
    let summary = match (file, keys) {
        // Fully offline: no session needed.
        (Some(file), Some(keys)) => verify_receipt(&read_json(file)?, &read_json(keys)?, move_id)?,
        _ => receipt(&Api::from_profile()?, project, action).await?,
    };
    print(&summary, *json, |s| format!("Receipt {} verified: {} signed with key version {}, body {}.", text(s, "/move_id"), text(s, "/kind"), text(s, "/key_version"), text(s, "/body_sha256")))
}

#[allow(clippy::too_many_arguments)]
pub async fn handle_convert(project: &str, resource: &str, generation: u64, conversion: &str, confirm: Option<&str>, acknowledge: &ConvertAcknowledge, operation_id: Option<&str>, wait: bool, json: bool) -> Result<()> {
    uuid::Uuid::parse_str(resource).map_err(|_| anyhow!("--resource must be a UUID"))?;
    let body = convert_body(generation, conversion, confirm, acknowledge)?;
    let key = operation_key(operation_id)?;
    let api = Api::from_profile()?;
    eprintln!("Operation idempotency key: {key}");
    let value = api.step_up_action(&Method::POST, &["projects", project, "resources", resource, "convert"], &body, &key).await?;
    let value = match (wait, value["operation_id"].as_str().or_else(|| value.pointer("/operation/id").and_then(Value::as_str))) {
        (true, Some(op)) => json!({"operation": wait_operation(&api, project, op, CONVERT_WAIT).await?}),
        _ => value,
    };
    print(&value, json, |_| "Conversion started. Redeploy the runtime once it finishes; the receipt is listed by `placement`.".to_string())
}

#[cfg(test)]
mod tests;
