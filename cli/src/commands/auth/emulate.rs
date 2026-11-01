//! `axiom auth emulate`: local auth emulation (examples-matrix 7, CA9). A loopback-only auth
//! server for a local project: issuer `http://127.0.0.1:<port>/local/<project>`, Ed25519
//! signing keys generated per run under `.axiom/auth-local/` (mode 0600; `reset` deletes the
//! folder), storage a disposable native SQLite file holding the hosted `axiom-auth` stream
//! (installed from the releases in this CLI by axiom-database's system-mode installer), mail
//! captured into the stream's `captured_mail` table and shown at `/local/mail`, and a mock
//! OIDC issuer at `/local/oidc` that signs synthetic identities only (`--identity`).
//!
//! It allocates no hosted database and opens no outbound connection. Tokens carry the local
//! issuer, so every Cloud runtime and issuer refuses them; a local runtime verifies them with
//! a `jwt` EdDSA provider (`axiom auth emulate deployment`). Local users, keys and callbacks
//! never leave the folder: `axiom auth config export|diff|push --from local` read only the
//! emulated settings, without loopback callbacks or origins and without the mock provider
//! ([`promotable`]).
use super::config::{self, Config};
use super::{print, usage, AuthArgs};
use anyhow::{Context, Result};
use axiom_database::developer::{self as db, Session};
use axum::{
    body::Bytes,
    extract::{Path as UrlPath, Query, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Router,
};
use base64::Engine;
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The emulator's folder under the project directory.
pub const STATE_DIR: &str = ".axiom/auth-local";
/// The runtime reference of the local verification key (`jwt` provider, EdDSA).
pub const KEY_ENV: &str = "AXIOM_AUTH_LOCAL_PUBLIC_KEY";
/// The mock provider every emulator offers besides the configured ones; never exported.
pub const MOCK_PROVIDER: &str = "local";
const DEFAULT_IDENTITY: &str = "user@example.com";
const PBKDF2_ROUNDS: u32 = 100_000;
const CODE_TTL_MS: i64 = 10 * 60 * 1000;

#[derive(clap::Args, Debug)]
#[command(args_conflicts_with_subcommands = true)]
pub struct EmulateArgs {
    #[command(subcommand)]
    pub action: Option<EmulateAction>,
    /// Project directory; state lives in <dir>/.axiom/auth-local
    #[arg(long, default_value = ".")]
    pub dir: PathBuf,
    /// Loopback port (0: a free port)
    #[arg(long, default_value_t = 0)]
    pub port: u16,
    /// A synthetic identity the mock OIDC issuer offers (an email); repeatable
    #[arg(long = "identity")]
    pub identities: Vec<String>,
    /// The access-token audience
    #[arg(long, default_value = "api")]
    pub audience: String,
    /// axiom-auth-config/v1 file with the local settings (default auth/config.local.json when present)
    #[arg(long)]
    pub config: Option<PathBuf>,
}

#[derive(clap::Subcommand, Debug)]
pub enum EmulateAction {
    /// Delete the local store, keys, captured mail and settings
    Reset {
        #[arg(long, default_value = ".")]
        dir: PathBuf,
    },
    /// Point a server deployment at the running emulator: a `jwt` EdDSA provider with the
    /// local issuer, its public key from AXIOM_AUTH_LOCAL_PUBLIC_KEY
    Deployment {
        deployment: PathBuf,
        #[arg(long, default_value = ".")]
        dir: PathBuf,
    },
}

// ---------------------------------------------------------------------------
// Local configuration and promotion
// ---------------------------------------------------------------------------

fn defaults(project: &str) -> Config {
    let settings = json!({
        "enabled": true,
        "signup_policy": "open",
        "enabled_methods": ["password", "email_code"],
        "require_verified_email": true,
        "password_min_length": 8,
        "access_ttl_seconds": 900,
        "session_idle_seconds": 1_209_600,
        "session_absolute_seconds": 2_592_000,
        "redirect_urls": ["http://127.0.0.1:5173/auth/callback", "http://localhost:5173/auth/callback"],
        "allowed_origins": ["http://127.0.0.1:5173", "http://localhost:5173"],
        "app_name": "Local app",
        "email_sender": "platform",
    });
    Config {
        format: config::CONFIG_FORMAT.into(),
        source: json!({"kind": "local", "project": project}),
        settings: settings.as_object().cloned().unwrap_or_default(),
        providers: vec![],
        trusted_issuers: vec![],
        policy: json!({"format": "axiom-auth-policy/v1", "permissions": [], "roles": []}),
        outbound: json!({"webhooks": [], "hooks": []}),
    }
}

fn loopback(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| match u.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost") || d.to_ascii_lowercase().ends_with(".localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback() || ip.is_unspecified(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback() || ip.is_unspecified(),
        None => false,
    })
}

/// The part of a local configuration that may become Cloud state: loopback callbacks,
/// origins and WebAuthn origins are dropped (local callbacks never become production
/// state), as are the mock provider and any provider or trusted issuer on a loopback issuer.
/// Users, keys, sessions and captured mail are never part of a configuration.
pub fn promotable(mut config: Config) -> Config {
    for key in ["redirect_urls", "logout_urls", "allowed_origins", "webauthn_origins"] {
        if let Some(Value::Array(items)) = config.settings.get_mut(key) {
            items.retain(|v| !v.as_str().is_some_and(loopback));
        }
    }
    if config.settings.get("default_redirect_url").and_then(Value::as_str).is_some_and(loopback) {
        config.settings.remove("default_redirect_url");
    }
    if config.settings.get("webauthn_rp_id").and_then(Value::as_str).is_some_and(|h| h == "localhost" || h == "127.0.0.1") {
        config.settings.remove("webauthn_rp_id");
    }
    config.providers.retain(|p| p["slug"].as_str() != Some(MOCK_PROVIDER) && !p["issuer"].as_str().is_some_and(loopback));
    config.trusted_issuers.retain(|i| !i["issuer"].as_str().is_some_and(loopback) && !i["jwks_url"].as_str().is_some_and(loopback));
    let project = config.source.get("project").cloned();
    config.source = json!({"kind": "local"});
    if let Some(p) = project.filter(Value::is_string) {
        config.source["project"] = p;
    }
    config
}

/// `--from local`: the emulated configuration of `dir`, as [`promotable`] leaves it.
pub fn local_config(dir: &Path) -> Result<Config> {
    let path = dir.join(STATE_DIR).join("config.json");
    let bytes = std::fs::read(&path).map_err(|_| usage("no local auth emulation here: run `axiom auth emulate` in the project directory first"))?;
    Ok(promotable(config::parse(&bytes)?))
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

fn private_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Writes a new 0600 file in place of any earlier one.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let _ = std::fs::remove_file(path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    options.open(path).with_context(|| format!("write {}", path.display()))?.write_all(bytes)?;
    Ok(())
}

fn project_name(args: &AuthArgs, dir: &Path) -> String {
    let raw = args.project.clone().unwrap_or_else(|| dir.canonicalize().ok().and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_else(|| "app".into()));
    let name: String = raw.to_ascii_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>().trim_matches('-').chars().take(64).collect();
    if name.is_empty() {
        "app".into()
    } else {
        name
    }
}

/// SubjectPublicKeyInfo PEM of an Ed25519 key (what the runtime's EdDSA `jwt` provider reads).
pub fn public_pem(key: &VerifyingKey) -> String {
    let mut der = vec![0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00];
    der.extend_from_slice(key.as_bytes());
    format!("-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n", base64::engine::general_purpose::STANDARD.encode(der))
}

// ---------------------------------------------------------------------------
// Primitives
// ---------------------------------------------------------------------------

fn random() -> Result<[u8; 32]> {
    db::ephemeral_signing_seed().map_err(|_| anyhow::anyhow!("system entropy unavailable"))
}
fn b64u(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}
fn sha_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
fn rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}
/// A stable UUID-shaped id for a name (the provider row id of a configured slug).
fn named_id(name: &str) -> String {
    let h = Sha256::digest(name.as_bytes());
    let mut b = [0u8; 16];
    b.copy_from_slice(&h[..16]);
    b[6] = (b[6] & 0x0f) | 0x50;
    b[8] = (b[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(b).to_string()
}
fn normalize_email(email: &str) -> Option<String> {
    let e = email.trim().to_ascii_lowercase();
    let (local, domain) = e.split_once('@')?;
    (!local.is_empty() && domain.contains('.') && e.len() <= 254 && !e.contains(char::is_whitespace)).then_some(e)
}

/// PBKDF2-HMAC-SHA256 in PHC form. The emulator holds synthetic credentials only.
pub fn hash_password(password: &str) -> Result<String> {
    let salt = &random()?[..16];
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2::<hmac::Hmac<Sha256>>(password.as_bytes(), salt, PBKDF2_ROUNDS, &mut out);
    Ok(format!("$pbkdf2-sha256$i={PBKDF2_ROUNDS}${}${}", b64u(salt), b64u(&out)))
}
pub fn verify_password(password: &str, phc: &str) -> bool {
    let parts: Vec<&str> = phc.split('$').collect();
    let [_, "pbkdf2-sha256", rounds, salt, hash] = parts.as_slice() else { return false };
    let (Some(rounds), Ok(salt), Ok(hash)) = (rounds.strip_prefix("i=").and_then(|r| r.parse::<u32>().ok()), base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(salt), base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(hash)) else { return false };
    let mut out = vec![0u8; hash.len()];
    pbkdf2::pbkdf2::<hmac::Hmac<Sha256>>(password.as_bytes(), &salt, rounds, &mut out);
    Sha256::digest(&out) == Sha256::digest(&hash)
}

/// A compact EdDSA JWT.
pub fn sign_jwt(key: &SigningKey, kid: &str, claims: &Value) -> String {
    let head = b64u(json!({"alg": "EdDSA", "typ": "JWT", "kid": kid}).to_string().as_bytes());
    let body = b64u(claims.to_string().as_bytes());
    let signature = key.sign(format!("{head}.{body}").as_bytes());
    format!("{head}.{body}.{}", b64u(&signature.to_bytes()))
}
/// The claims of a JWT signed by `key`, if its signature verifies (claims are not checked).
pub fn verify_jwt(key: &VerifyingKey, token: &str) -> Option<Value> {
    let mut parts = token.split('.');
    let (head, body, sig) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let d = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s).ok();
    let header: Value = serde_json::from_slice(&d(head)?).ok()?;
    if header["alg"] != "EdDSA" {
        return None;
    }
    let signature = ed25519_dalek::Signature::from_slice(&d(sig)?).ok()?;
    key.verify(format!("{head}.{body}").as_bytes(), &signature).ok()?;
    serde_json::from_slice(&d(body)?).ok()
}
fn kid_of(key: &VerifyingKey) -> String {
    sha_hex(key.as_bytes())[..16].to_string()
}
fn jwk(key: &VerifyingKey) -> Value {
    json!({"kty": "OKP", "crv": "Ed25519", "x": b64u(key.as_bytes()), "kid": kid_of(key), "alg": "EdDSA", "use": "sig"})
}
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}
fn s(v: impl ToString) -> Option<String> {
    Some(v.to_string())
}

// ---------------------------------------------------------------------------
// The server
// ---------------------------------------------------------------------------

enum Pending {
    /// `/v1/providers/<slug>/start`, waiting for the mock issuer's callback.
    Provider { slug: String, redirect_uri: String, state: String, challenge: String, nonce: String, expires: i64 },
    /// A mock OIDC authorization code.
    Code { email: String, client_id: String, redirect_uri: String, nonce: Option<String>, challenge: Option<String>, expires: i64 },
}

pub struct Emulator {
    db: Session,
    lock: tokio::sync::Mutex<()>,
    key: SigningKey,
    oidc_key: SigningKey,
    pub issuer: String,
    pub origin: String,
    pub audience: String,
    config: Config,
    identities: Vec<String>,
    pending: std::sync::Mutex<HashMap<String, Pending>>,
}

type Reply = std::result::Result<Response, Response>;

fn fail(status: StatusCode, code: &str, message: &str) -> Response {
    (status, axum::Json(json!({"error": {"code": code, "message": message}}))).into_response()
}
fn bad(message: &str) -> Response {
    fail(StatusCode::BAD_REQUEST, "AXAUTH_INVALID_REQUEST", message)
}
fn unavailable<E>(_: E) -> Response {
    fail(StatusCode::SERVICE_UNAVAILABLE, "AXAUTH_UNAVAILABLE", "the local store refused the operation")
}
fn found(location: &str) -> Response {
    let mut r = StatusCode::FOUND.into_response();
    if let Ok(v) = HeaderValue::from_str(location) {
        r.headers_mut().insert(header::LOCATION, v);
    }
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}
fn ok(value: Value) -> Reply {
    Ok((StatusCode::OK, [(header::CACHE_CONTROL, "no-store")], axum::Json(value)).into_response())
}
fn body(bytes: &Bytes) -> std::result::Result<Map<String, Value>, Response> {
    if bytes.len() > 65_536 {
        return Err(bad("request body too large"));
    }
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Err(bad("a JSON object body is required")),
    }
}
fn field<'a>(map: &'a Map<String, Value>, key: &str) -> std::result::Result<&'a str, Response> {
    map.get(key).and_then(Value::as_str).filter(|v| !v.is_empty() && v.len() <= 4096).ok_or_else(|| bad(&format!("`{key}` is required")))
}

impl Emulator {
    /// Opens (installing when new) the store in `state` and prepares fresh signing keys.
    pub async fn open(state: &Path, origin: &str, project: &str, audience: &str, config: Config, identities: Vec<String>) -> Result<(Arc<Self>, db::local_auth::Installed)> {
        let store = state.join("store.sqlite");
        let installed = db::local_auth::install(&store).await.map_err(|e| anyhow::anyhow!("installing the auth stream on the local store failed ({})", e.code))?;
        let session = db::connect(&db::Connection::Sqlite { file: store }, true).await.map_err(|e| anyhow::anyhow!("opening the local store failed ({})", e.code))?;
        let key = SigningKey::from_bytes(&random()?);
        let oidc_key = SigningKey::from_bytes(&random()?);
        write_private(&state.join("signing.key"), &key.to_bytes())?;
        write_private(&state.join("oidc-signing.key"), &oidc_key.to_bytes())?;
        write_private(&state.join("public.pem"), public_pem(&key.verifying_key()).as_bytes())?;
        let identities = if identities.is_empty() { vec![DEFAULT_IDENTITY.to_string()] } else { identities };
        let emulator = Self {
            db: session,
            lock: tokio::sync::Mutex::new(()),
            key,
            oidc_key,
            issuer: format!("{origin}/local/{project}"),
            origin: origin.into(),
            audience: audience.into(),
            config,
            identities,
            pending: std::sync::Mutex::new(HashMap::new()),
        };
        Ok((Arc::new(emulator), installed))
    }

    pub fn verifying_key(&self) -> VerifyingKey {
        self.key.verifying_key()
    }
    fn oidc_issuer(&self) -> String {
        format!("{}/local/oidc", self.origin)
    }
    fn setting(&self, key: &str) -> Option<&Value> {
        self.config.settings.get(key)
    }
    fn number(&self, key: &str, default: i64) -> i64 {
        self.setting(key).and_then(Value::as_i64).unwrap_or(default)
    }
    fn strings(&self, key: &str) -> Vec<String> {
        self.setting(key).and_then(Value::as_array).into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect()
    }
    fn method_enabled(&self, method: &str) -> bool {
        self.strings("enabled_methods").iter().any(|m| m == method)
    }
    fn provider(&self, slug: &str) -> Option<(String, String)> {
        if slug == MOCK_PROVIDER {
            return Some(("oidc".into(), "local".into()));
        }
        self.config.providers.iter().find(|p| p["slug"].as_str() == Some(slug)).map(|p| {
            let kind = p["kind"].as_str().filter(|k| ["google", "github"].contains(k)).unwrap_or("oidc");
            (kind.to_string(), p["client_id"].as_str().unwrap_or("local").to_string())
        })
    }

    async fn q(&self, sql: &str, params: Vec<Option<String>>) -> std::result::Result<Vec<Vec<Option<String>>>, Response> {
        self.db.query_parameters(sql, params).await.map(|r| r.rows).map_err(unavailable)
    }
    async fn tx<T>(&self, work: impl std::future::Future<Output = std::result::Result<T, Response>>) -> std::result::Result<T, Response> {
        self.db.execute("BEGIN IMMEDIATE").await.map_err(unavailable)?;
        match work.await {
            Ok(v) => {
                self.db.execute("COMMIT").await.map_err(unavailable)?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.db.execute("ROLLBACK").await;
                Err(e)
            }
        }
    }

    /// Captures a message (branches-v1 `captured_mail` shape) instead of sending it.
    async fn capture(&self, template: &str, to: &str, subject: &str, text: &str, challenge: Option<&str>) -> std::result::Result<(), Response> {
        self.q(
            "INSERT INTO __axiom_auth_captured_mail(id,created_at,channel,template,to_address,subject,body_text,challenge_id) VALUES (?1,?2,'email',?3,?4,?5,?6,?7)",
            vec![s(new_id()), s(now_ms()), s(template), s(to), s(subject), s(text), challenge.map(str::to_string)],
        )
        .await
        .map(|_| ())
    }

    /// A new email challenge; the code goes to the mail capture only.
    async fn challenge(&self, purpose: &str, email: &str, user: Option<&str>, pending_phc: Option<String>) -> Reply {
        let id = new_id();
        let r = random().map_err(unavailable)?;
        let code = format!("{:06}", u32::from_le_bytes([r[0], r[1], r[2], r[3]]) % 1_000_000);
        let now = now_ms();
        self.q(
            "INSERT INTO __axiom_auth_challenges(id,purpose,email_hmac,email,user_id,proof_hash,pending_phc,sender,delivery_state,attempts,created_at,expires_at) VALUES (?1,?2,CAST(?3 AS BLOB),?4,?5,CAST(?6 AS BLOB),?7,'platform','queued',0,?8,?9)",
            vec![s(&id), s(purpose), s(sha_hex(email.as_bytes())), s(email), user.map(str::to_string), s(sha_hex(format!("{id}:{code}").as_bytes())), pending_phc, s(now), s(now + CODE_TTL_MS)],
        )
        .await?;
        let app = self.setting("app_name").and_then(Value::as_str).unwrap_or("Local app");
        let (template, subject) = if purpose == "signup" { ("signup", format!("Confirm your {app} account")) } else { ("email_code", format!("Your {app} sign-in code")) };
        self.capture(template, email, &subject, &format!("Your code is {code}. It expires in 10 minutes."), Some(&id)).await?;
        ok(json!({"challenge_id": id, "resend_after": 60}))
    }

    /// Consumes a challenge: `(email, user_id, pending_phc)`.
    async fn consume(&self, id: &str, purpose: &str, code: &str) -> std::result::Result<(String, Option<String>, Option<String>), Response> {
        let rows = self.q("SELECT email,user_id,pending_phc,CAST(proof_hash AS TEXT),attempts,expires_at,consumed_at FROM __axiom_auth_challenges WHERE id=?1 AND purpose=?2", vec![s(id), s(purpose)]).await?;
        let invalid = || fail(StatusCode::BAD_REQUEST, "AXAUTH_INVALID_CODE", "the code is wrong or expired");
        let row = rows.into_iter().next().ok_or_else(invalid)?;
        let attempts: i64 = row[4].as_deref().and_then(|v| v.parse().ok()).unwrap_or(5);
        let expires: i64 = row[5].as_deref().and_then(|v| v.parse().ok()).unwrap_or(0);
        if row[6].is_some() || attempts >= 5 || expires <= now_ms() {
            return Err(invalid());
        }
        if row[3].as_deref() != Some(sha_hex(format!("{id}:{code}").as_bytes()).as_str()) {
            self.q("UPDATE __axiom_auth_challenges SET attempts=attempts+1 WHERE id=?1", vec![s(id)]).await?;
            return Err(invalid());
        }
        self.q("UPDATE __axiom_auth_challenges SET consumed_at=?2 WHERE id=?1", vec![s(id), s(now_ms())]).await?;
        Ok((row[0].clone().unwrap_or_default(), row[1].clone(), row[2].clone()))
    }

    async fn user_by_email(&self, email: &str) -> std::result::Result<Option<(String, String)>, Response> {
        Ok(self.q("SELECT id,status FROM __axiom_auth_users WHERE email_normalized=?1", vec![s(email)]).await?.into_iter().next().map(|r| (r[0].clone().unwrap_or_default(), r[1].clone().unwrap_or_default())))
    }

    async fn create_user(&self, email: &str, phc: Option<&str>, verified: bool) -> std::result::Result<String, Response> {
        let (id, now) = (new_id(), now_ms());
        self.q("INSERT INTO __axiom_auth_users(id,kind,status,email,email_normalized,authz_rev,security_rev,created_at,updated_at) VALUES (?1,'password','active',?2,?2,1,1,?3,?3)", vec![s(&id), s(email), s(now)]).await?;
        self.q("INSERT INTO __axiom_auth_user_profiles(user_id,email_verified_at,previous_email_hmacs,updated_at) VALUES (?1,?2,CAST('' AS BLOB),?3)", vec![s(&id), verified.then(|| now.to_string()), s(now)]).await?;
        if let Some(phc) = phc {
            self.q("INSERT INTO __axiom_auth_password_credentials(user_id,phc,params_version,revision,updated_at) VALUES (?1,?2,1,1,?3)", vec![s(&id), s(phc), s(now)]).await?;
        }
        Ok(id)
    }

    /// Starts a session and answers with the hosted token response shape.
    async fn session(&self, user: &str, method: &str, provider: Option<&str>) -> Reply {
        let rows = self.q("SELECT kind,status,email,security_rev FROM __axiom_auth_users WHERE id=?1", vec![s(user)]).await?;
        let row = rows.into_iter().next().ok_or_else(|| fail(StatusCode::UNAUTHORIZED, "AXAUTH_INVALID_CREDENTIALS", "unknown user"))?;
        if row[1].as_deref() != Some("active") {
            return Err(fail(StatusCode::FORBIDDEN, "AXAUTH_USER_DISABLED", "the user is banned"));
        }
        let (sid, now) = (new_id(), now_ms());
        let refresh = format!("{sid}.{}", b64u(&random().map_err(unavailable)?));
        let idle = self.number("session_idle_seconds", 1_209_600) * 1000;
        let absolute = now + self.number("session_absolute_seconds", 2_592_000) * 1000;
        let access = self.number("access_ttl_seconds", 900) * 1000;
        let idle_at = (now + idle).min(absolute);
        self.q(
            "INSERT INTO __axiom_auth_sessions(id,user_id,method,amr,auth_time,counter,current_hash,previous_hashes,security_rev,epoch,status,created_at,last_rotated_at,access_expires_at,idle_expires_at,absolute_expires_at) VALUES (?1,?2,'password','pwd',?3,0,CAST(?4 AS BLOB),CAST('' AS BLOB),?5,1,'active',?3,?3,?6,?7,?8)",
            vec![s(&sid), s(user), s(now), s(sha_hex(refresh.as_bytes())), row[3].clone(), s(now + access), s(idle_at), s(absolute)],
        )
        .await?;
        if method != "password" {
            self.q("INSERT INTO __axiom_auth_session_methods(session_id,auth_method,provider_id,created_at) VALUES (?1,?2,?3,?4)", vec![s(&sid), s(method), provider.map(named_id), s(now)]).await?;
        }
        ok(self.tokens(user, &row, &sid, &refresh, method, now, now, idle_at, absolute))
    }

    #[allow(clippy::too_many_arguments)]
    fn tokens(&self, user: &str, row: &[Option<String>], sid: &str, refresh: &str, method: &str, auth_time: i64, now: i64, idle_at: i64, absolute: i64) -> Value {
        let ttl = self.number("access_ttl_seconds", 900);
        let amr = match method {
            "password" => "pwd",
            "email_code" => "otp",
            _ => "fed",
        };
        let claims = json!({
            "iss": self.issuer, "aud": self.audience, "sub": user, "sid": sid,
            "iat": now / 1000, "nbf": now / 1000, "exp": now / 1000 + ttl, "jti": new_id(), "token_use": "access",
            "amr": [amr], "auth_time": auth_time / 1000, "roles": [], "permissions": [], "acr": "aal1", "epoch": 1,
            "claims": {"sid": sid, "acr": "aal1"},
        });
        let access = sign_jwt(&self.key, &kid_of(&self.key.verifying_key()), &claims);
        let mut user_json = json!({"id": user, "kind": row[0]});
        if let Some(email) = &row[2] {
            user_json["email"] = json!(email);
        }
        json!({"access_token": access, "token_type": "Bearer", "expires_in": ttl, "refresh_token": refresh,
            "refresh_expires_at": rfc3339(idle_at), "session": {"id": sid, "absolute_expires_at": rfc3339(absolute), "tenant": null}, "user": user_json})
    }

    /// Verified bearer claims of this issuer with an active session.
    async fn bearer(&self, headers: &HeaderMap) -> std::result::Result<Value, Response> {
        let invalid = || fail(StatusCode::UNAUTHORIZED, "AXAUTH_INVALID_TOKEN", "missing, expired or invalid access token");
        let token = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).ok_or_else(invalid)?;
        let claims = verify_jwt(&self.key.verifying_key(), token).ok_or_else(invalid)?;
        if claims["iss"] != self.issuer || claims["exp"].as_i64().unwrap_or(0) <= now_ms() / 1000 {
            return Err(invalid());
        }
        let sid = claims["sid"].as_str().unwrap_or_default().to_string();
        let active = self.q("SELECT 1 FROM __axiom_auth_sessions WHERE id=?1 AND status='active'", vec![s(sid)]).await?;
        if active.is_empty() {
            return Err(invalid());
        }
        Ok(claims)
    }

    fn put(&self, key: String, value: Pending) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let now = now_ms();
        pending.retain(|_, p| match p {
            Pending::Provider { expires, .. } | Pending::Code { expires, .. } => *expires > now,
        });
        pending.insert(key, value);
    }
    fn take(&self, key: &str) -> Option<Pending> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(key).filter(|p| match p {
            Pending::Provider { expires, .. } | Pending::Code { expires, .. } => *expires > now_ms(),
        })
    }

    /// The mock issuer's token endpoint logic: an ID token for a pending code.
    fn redeem(&self, code: &str, redirect_uri: &str, verifier: Option<&str>) -> Option<Value> {
        let Some(Pending::Code { email, client_id, redirect_uri: expected, nonce, challenge, .. }) = self.take(&format!("code:{code}")) else { return None };
        if expected != redirect_uri {
            return None;
        }
        if let Some(challenge) = challenge {
            if verifier.map(|v| b64u(&Sha256::digest(v.as_bytes()))) != Some(challenge) {
                return None;
            }
        }
        let now = now_ms() / 1000;
        let mut claims = json!({"iss": self.oidc_issuer(), "sub": format!("synthetic-{}", &sha_hex(email.as_bytes())[..24]), "aud": client_id,
            "iat": now, "exp": now + 300, "email": email, "email_verified": true, "name": email.split('@').next().unwrap_or("user")});
        if let Some(nonce) = nonce {
            claims["nonce"] = json!(nonce);
        }
        let id_token = sign_jwt(&self.oidc_key, &kid_of(&self.oidc_key.verifying_key()), &claims);
        Some(json!({"id_token": id_token, "access_token": b64u(&random().ok()?), "token_type": "Bearer", "expires_in": 300}))
    }
}

// --- issuer routes ---------------------------------------------------------

async fn discovery(State(e): State<Arc<Emulator>>) -> Reply {
    ok(json!({"issuer": e.issuer, "jwks_uri": format!("{}/.well-known/jwks.json", e.issuer), "token_endpoint": format!("{}/v1/token/authorization-code", e.issuer),
        "id_token_signing_alg_values_supported": ["EdDSA"], "axiom_local_emulation": true}))
}
async fn jwks(State(e): State<Arc<Emulator>>) -> Reply {
    ok(json!({"keys": [jwk(&e.key.verifying_key())]}))
}

async fn signup(State(e): State<Arc<Emulator>>, bytes: Bytes) -> Reply {
    let b = body(&bytes)?;
    if !e.method_enabled("password") {
        return Err(fail(StatusCode::FORBIDDEN, "AXAUTH_METHOD_DISABLED", "password sign-up is disabled"));
    }
    if e.setting("signup_policy").and_then(Value::as_str).is_some_and(|p| p != "open") {
        return Err(fail(StatusCode::FORBIDDEN, "AXAUTH_SIGNUP_DISABLED", "sign-up is closed"));
    }
    let email = normalize_email(field(&b, "email")?).ok_or_else(|| bad("invalid email"))?;
    let password = field(&b, "password")?;
    if (password.chars().count() as i64) < e.number("password_min_length", 8) {
        return Err(fail(StatusCode::BAD_REQUEST, "AXAUTH_WEAK_PASSWORD", "the password is too short"));
    }
    let phc = hash_password(password).map_err(unavailable)?;
    let _g = e.lock.lock().await;
    if e.user_by_email(&email).await?.is_some() {
        return Err(fail(StatusCode::CONFLICT, "AXAUTH_ACCOUNT_EXISTS", "an account exists for this email"));
    }
    if e.setting("require_verified_email").and_then(Value::as_bool).unwrap_or(true) {
        return e.challenge("signup", &email, None, Some(phc)).await;
    }
    e.tx(async {
        let user = e.create_user(&email, Some(&phc), false).await?;
        e.session(&user, "password", None).await
    })
    .await
}

async fn signup_verify(State(e): State<Arc<Emulator>>, bytes: Bytes) -> Reply {
    let b = body(&bytes)?;
    let (id, code) = (field(&b, "challenge_id")?, field(&b, "code")?);
    let _g = e.lock.lock().await;
    e.tx(async {
        let (email, _, phc) = e.consume(id, "signup", code).await?;
        if e.user_by_email(&email).await?.is_some() {
            return Err(fail(StatusCode::CONFLICT, "AXAUTH_ACCOUNT_EXISTS", "an account exists for this email"));
        }
        let user = e.create_user(&email, phc.as_deref(), true).await?;
        e.session(&user, "password", None).await
    })
    .await
}

async fn sign_in_password(State(e): State<Arc<Emulator>>, bytes: Bytes) -> Reply {
    let b = body(&bytes)?;
    let invalid = || fail(StatusCode::UNAUTHORIZED, "AXAUTH_INVALID_CREDENTIALS", "wrong email or password");
    let email = normalize_email(field(&b, "email")?).ok_or_else(invalid)?;
    let password = field(&b, "password")?;
    let _g = e.lock.lock().await;
    let rows = e.q("SELECT u.id,c.phc,p.email_verified_at FROM __axiom_auth_users u JOIN __axiom_auth_password_credentials c ON c.user_id=u.id LEFT JOIN __axiom_auth_user_profiles p ON p.user_id=u.id WHERE u.email_normalized=?1", vec![s(&email)]).await?;
    let row = rows.into_iter().next().ok_or_else(invalid)?;
    if !verify_password(password, row[1].as_deref().unwrap_or_default()) {
        return Err(invalid());
    }
    if row[2].is_none() && e.setting("require_verified_email").and_then(Value::as_bool).unwrap_or(true) {
        return Err(fail(StatusCode::FORBIDDEN, "AXAUTH_EMAIL_UNVERIFIED", "verify the email address first"));
    }
    let user = row[0].clone().unwrap_or_default();
    e.tx(e.session(&user, "password", None)).await
}

async fn email_code(State(e): State<Arc<Emulator>>, bytes: Bytes) -> Reply {
    let b = body(&bytes)?;
    if !e.method_enabled("email_code") {
        return Err(fail(StatusCode::FORBIDDEN, "AXAUTH_METHOD_DISABLED", "email codes are disabled"));
    }
    let email = normalize_email(field(&b, "email")?).ok_or_else(|| bad("invalid email"))?;
    let _g = e.lock.lock().await;
    let user = e.user_by_email(&email).await?.map(|(id, _)| id);
    e.challenge("email_code", &email, user.as_deref(), None).await
}

async fn email_code_verify(State(e): State<Arc<Emulator>>, bytes: Bytes) -> Reply {
    let b = body(&bytes)?;
    let (id, code) = (field(&b, "challenge_id")?, field(&b, "code")?);
    let _g = e.lock.lock().await;
    e.tx(async {
        let (email, user, _) = e.consume(id, "email_code", code).await?;
        let user = match user.or(e.user_by_email(&email).await?.map(|(id, _)| id)) {
            Some(user) => {
                e.q("UPDATE __axiom_auth_user_profiles SET email_verified_at=coalesce(email_verified_at,?2) WHERE user_id=?1", vec![s(&user), s(now_ms())]).await?;
                user
            }
            None if e.setting("signup_policy").and_then(Value::as_str).unwrap_or("open") == "open" => e.create_user(&email, None, true).await?,
            None => return Err(fail(StatusCode::FORBIDDEN, "AXAUTH_SIGNUP_DISABLED", "sign-up is closed")),
        };
        e.session(&user, "email_code", None).await
    })
    .await
}

async fn refresh(State(e): State<Arc<Emulator>>, bytes: Bytes) -> Reply {
    let b = body(&bytes)?;
    let presented = field(&b, "refresh_token")?;
    let invalid = || fail(StatusCode::UNAUTHORIZED, "AXAUTH_INVALID_REFRESH", "the refresh token is not valid");
    let sid = presented.split_once('.').map(|(sid, _)| sid).ok_or_else(invalid)?;
    let hash = sha_hex(presented.as_bytes());
    let _g = e.lock.lock().await;
    e.tx(async {
        let rows = e.q("SELECT s.user_id,s.status,CAST(s.current_hash AS TEXT),CAST(s.previous_hashes AS TEXT),s.idle_expires_at,s.absolute_expires_at,s.auth_time,coalesce(m.auth_method,'password'),u.kind,u.status,u.email FROM __axiom_auth_sessions s JOIN __axiom_auth_users u ON u.id=s.user_id LEFT JOIN __axiom_auth_session_methods m ON m.session_id=s.id WHERE s.id=?1", vec![s(sid)]).await?;
        let row = rows.into_iter().next().ok_or_else(invalid)?;
        let int = |i: usize| row[i].as_deref().and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
        let now = now_ms();
        if row[1].as_deref() != Some("active") {
            return Err(invalid());
        }
        if row[2].as_deref() != Some(hash.as_str()) {
            if row[3].as_deref().unwrap_or_default().split(',').any(|h| h == hash) {
                // Reuse of a rotated token ends the session (auth-core/v1 reuse detection).
                e.q("UPDATE __axiom_auth_sessions SET status='reused',end_reason='reuse',ended_at=?2 WHERE id=?1", vec![s(sid), s(now)]).await?;
                return Ok(Err(invalid()));
            }
            return Err(invalid());
        }
        if row[9].as_deref() != Some("active") {
            return Err(fail(StatusCode::FORBIDDEN, "AXAUTH_USER_DISABLED", "the user is banned"));
        }
        let absolute = int(5);
        if int(4) <= now || absolute <= now {
            return Err(invalid());
        }
        let next = format!("{sid}.{}", b64u(&random().map_err(unavailable)?));
        let idle_at = (now + e.number("session_idle_seconds", 1_209_600) * 1000).min(absolute);
        let previous = format!("{}{hash},", row[3].as_deref().unwrap_or_default());
        e.q(
            "UPDATE __axiom_auth_sessions SET current_hash=CAST(?2 AS BLOB),previous_hashes=CAST(?3 AS BLOB),counter=counter+1,last_rotated_at=?4,access_expires_at=?5,idle_expires_at=?6 WHERE id=?1",
            vec![s(sid), s(sha_hex(next.as_bytes())), s(previous), s(now), s(now + e.number("access_ttl_seconds", 900) * 1000), s(idle_at)],
        )
        .await?;
        let user = row[0].clone().unwrap_or_default();
        let user_row = [row[8].clone(), row[9].clone(), row[10].clone()];
        Ok(Ok(e.tokens(&user, &user_row, sid, &next, row[7].as_deref().unwrap_or("password"), int(6), now, idle_at, absolute)))
    })
    .await?
    .and_then(ok)
}

async fn logout(State(e): State<Arc<Emulator>>, bytes: Bytes) -> Reply {
    end(e, bytes, false).await
}
async fn logout_all(State(e): State<Arc<Emulator>>, bytes: Bytes) -> Reply {
    end(e, bytes, true).await
}
/// Ends the presented session (or every session of its user); unknown tokens answer the same.
async fn end(e: Arc<Emulator>, bytes: Bytes, all: bool) -> Reply {
    let b = body(&bytes)?;
    let presented = field(&b, "refresh_token")?;
    let Some((sid, _)) = presented.split_once('.') else { return ok(json!({})) };
    let _g = e.lock.lock().await;
    let rows = e.q("SELECT user_id FROM __axiom_auth_sessions WHERE id=?1 AND current_hash=CAST(?2 AS BLOB)", vec![s(sid), s(sha_hex(presented.as_bytes()))]).await?;
    if let Some(row) = rows.into_iter().next() {
        if all {
            e.q("UPDATE __axiom_auth_sessions SET status='revoked',end_reason='logout',ended_at=?2 WHERE user_id=?1 AND status='active'", vec![row[0].clone(), s(now_ms())]).await?;
        } else {
            e.q("UPDATE __axiom_auth_sessions SET status='revoked',end_reason='logout',ended_at=?2 WHERE id=?1 AND status='active'", vec![s(sid), s(now_ms())]).await?;
        }
    }
    ok(json!({}))
}

async fn user(State(e): State<Arc<Emulator>>, headers: HeaderMap) -> Reply {
    let claims = e.bearer(&headers).await?;
    let id = claims["sub"].as_str().unwrap_or_default().to_string();
    let rows = e.q("SELECT u.kind,u.email,p.email_verified_at,(SELECT count(*) FROM __axiom_auth_password_credentials c WHERE c.user_id=u.id) FROM __axiom_auth_users u LEFT JOIN __axiom_auth_user_profiles p ON p.user_id=u.id WHERE u.id=?1", vec![s(&id)]).await?;
    let row = rows.into_iter().next().ok_or_else(|| fail(StatusCode::NOT_FOUND, "AXAUTH_NOT_FOUND", "unknown user"))?;
    let identities = e.q("SELECT p.provider_kind FROM __axiom_auth_identities i JOIN __axiom_auth_provider_identities p ON p.identity_id=i.id WHERE i.user_id=?1", vec![s(&id)]).await?;
    ok(json!({"id": id, "kind": row[0], "email": row[1], "email_verified": row[2].is_some(), "has_password": row[3].as_deref() != Some("0"),
        "identities": identities.into_iter().map(|r| json!({"provider": r[0]})).collect::<Vec<_>>()}))
}

async fn provider_start(State(e): State<Arc<Emulator>>, UrlPath(slug): UrlPath<String>, Query(q): Query<HashMap<String, String>>) -> Reply {
    let (kind, client_id) = e.provider(&slug).ok_or_else(|| fail(StatusCode::NOT_FOUND, "AXAUTH_NOT_FOUND", "no such provider"))?;
    if slug != MOCK_PROVIDER && !e.method_enabled(&kind) && !e.method_enabled(&slug) {
        return Err(fail(StatusCode::FORBIDDEN, "AXAUTH_METHOD_DISABLED", "the provider is disabled"));
    }
    let redirect = q.get("redirect_uri").cloned().unwrap_or_default();
    if !e.strings("redirect_urls").contains(&redirect) {
        return Err(fail(StatusCode::BAD_REQUEST, "AXAUTH_REDIRECT_URI", "the redirect URI is not configured"));
    }
    let (state, challenge) = (q.get("state").cloned().unwrap_or_default(), q.get("code_challenge").cloned().unwrap_or_default());
    if state.is_empty() || challenge.len() < 43 || q.get("code_challenge_method").map(String::as_str) != Some("S256") {
        return Err(bad("state and an S256 code challenge are required"));
    }
    let (tx, nonce) = (b64u(&random().map_err(unavailable)?), b64u(&random().map_err(unavailable)?[..16]));
    e.put(format!("tx:{tx}"), Pending::Provider { slug: slug.clone(), redirect_uri: redirect, state, challenge, nonce: nonce.clone(), expires: now_ms() + CODE_TTL_MS });
    let mut url = url::Url::parse(&format!("{}/authorize", e.oidc_issuer())).map_err(unavailable)?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &client_id)
        .append_pair("redirect_uri", &format!("{}/v1/providers/{slug}/callback", e.issuer))
        .append_pair("scope", "openid email")
        .append_pair("state", &tx)
        .append_pair("nonce", &nonce);
    if let Some(hint) = q.get("login_hint") {
        url.query_pairs_mut().append_pair("login_hint", hint);
    }
    Ok(found(url.as_str()))
}

async fn provider_callback(State(e): State<Arc<Emulator>>, UrlPath(slug): UrlPath<String>, Query(q): Query<HashMap<String, String>>) -> Reply {
    let tx = q.get("state").cloned().unwrap_or_default();
    let Some(Pending::Provider { slug: expected, redirect_uri, state, challenge, nonce, .. }) = e.take(&format!("tx:{tx}")) else {
        return Err(fail(StatusCode::BAD_REQUEST, "AXAUTH_INVALID_GRANT", "unknown or expired sign-in"));
    };
    let back = |params: &[(&str, &str)]| {
        let mut url = url::Url::parse(&redirect_uri).map_err(unavailable)?;
        url.query_pairs_mut().extend_pairs(params);
        Ok::<_, Response>(found(url.as_str()))
    };
    if expected != slug {
        return back(&[("error", "invalid_grant"), ("state", &state), ("iss", &e.issuer)]);
    }
    // The mock issuer answers in process: no network call, as a real token endpoint would.
    let (kind, client_id) = e.provider(&slug).ok_or_else(|| fail(StatusCode::NOT_FOUND, "AXAUTH_NOT_FOUND", "no such provider"))?;
    let callback = format!("{}/v1/providers/{slug}/callback", e.issuer);
    let response = q.get("code").and_then(|code| e.redeem(code, &callback, None));
    let claims = response.as_ref().and_then(|r| r["id_token"].as_str()).and_then(|t| verify_jwt(&e.oidc_key.verifying_key(), t));
    let Some(claims) = claims.filter(|c| c["iss"] == e.oidc_issuer() && c["aud"] == client_id.as_str() && c["nonce"] == nonce.as_str() && c["exp"].as_i64().unwrap_or(0) > now_ms() / 1000) else {
        return back(&[("error", "provider_error"), ("state", &state), ("iss", &e.issuer)]);
    };
    let (subject, email) = (claims["sub"].as_str().unwrap_or_default().to_string(), claims["email"].as_str().and_then(normalize_email));
    let code = b64u(&random().map_err(unavailable)?);
    let method = if kind == "oidc" { "oidc" } else { kind.as_str() };
    let _g = e.lock.lock().await;
    e.q(
        "INSERT INTO __axiom_auth_authorization_codes(id,code_hash,purpose,redirect_uri,code_challenge,auth_method,provider_id,identity_issuer,identity_subject,identity_email,identity_email_verified,display_name,created_at,expires_at) VALUES (?1,CAST(?2 AS BLOB),'sign_in',?3,?4,?5,?6,?7,?8,?9,1,?10,?11,?12)",
        vec![s(new_id()), s(sha_hex(code.as_bytes())), s(&redirect_uri), s(&challenge), s(method), s(named_id(&slug)), s(e.oidc_issuer()), s(subject), email, claims["name"].as_str().map(str::to_string), s(now_ms()), s(now_ms() + 60_000)],
    )
    .await?;
    back(&[("code", &code), ("state", &state), ("iss", &e.issuer)])
}

async fn authorization_code(State(e): State<Arc<Emulator>>, bytes: Bytes) -> Reply {
    let b = body(&bytes)?;
    let (code, verifier, redirect) = (field(&b, "code")?, field(&b, "code_verifier")?, field(&b, "redirect_uri")?);
    let invalid = || fail(StatusCode::BAD_REQUEST, "AXAUTH_INVALID_GRANT", "the code is not valid");
    let _g = e.lock.lock().await;
    e.tx(async {
        let rows = e.q("SELECT id,redirect_uri,code_challenge,auth_method,provider_id,identity_issuer,identity_subject,identity_email,expires_at,consumed_at FROM __axiom_auth_authorization_codes WHERE code_hash=CAST(?1 AS BLOB)", vec![s(sha_hex(code.as_bytes()))]).await?;
        let row = rows.into_iter().next().ok_or_else(invalid)?;
        let expires: i64 = row[8].as_deref().and_then(|v| v.parse().ok()).unwrap_or(0);
        if row[9].is_some() || expires <= now_ms() || row[1].as_deref() != Some(redirect) || row[2].as_deref() != Some(b64u(&Sha256::digest(verifier.as_bytes())).as_str()) {
            return Err(invalid());
        }
        e.q("UPDATE __axiom_auth_authorization_codes SET consumed_at=?2 WHERE id=?1", vec![row[0].clone(), s(now_ms())]).await?;
        let (method, provider, issuer, subject) = (row[3].clone().unwrap_or_default(), row[4].clone().unwrap_or_default(), row[5].clone().unwrap_or_default(), row[6].clone().unwrap_or_default());
        let now = now_ms();
        let linked = e.q("SELECT user_id,id FROM __axiom_auth_identities WHERE issuer=?1 AND subject=?2", vec![s(&issuer), s(&subject)]).await?;
        let user = match linked.into_iter().next() {
            Some(r) => {
                e.q("UPDATE __axiom_auth_identities SET last_used_on=?2 WHERE id=?1", vec![r[1].clone(), s(now)]).await?;
                r[0].clone().unwrap_or_default()
            }
            None => {
                let email = row[7].clone().ok_or_else(invalid)?;
                if e.user_by_email(&email).await?.is_some() {
                    // As hosted: an existing account is linked only from a signed-in session.
                    return Err(fail(StatusCode::CONFLICT, "AXAUTH_ACCOUNT_LINK_REQUIRED", "sign in to the existing account and link the provider"));
                }
                let user = e.create_user(&email, None, true).await?;
                let identity = new_id();
                e.q("INSERT INTO __axiom_auth_identities(id,user_id,trusted_issuer_id,issuer,subject,email_claim,created_at,last_used_on) VALUES (?1,?2,?3,?4,?5,?6,?7,?7)", vec![s(&identity), s(&user), s(&provider), s(&issuer), s(&subject), s(&email), s(now)]).await?;
                let kind = if ["google", "github"].contains(&method.as_str()) { method.clone() } else { "oidc".into() };
                e.q("INSERT INTO __axiom_auth_provider_identities(identity_id,provider_id,provider_kind,email_verified,created_at,updated_at) VALUES (?1,?2,?3,1,?4,?4)", vec![s(&identity), s(&provider), s(kind), s(now)]).await?;
                user
            }
        };
        e.session(&user, &method, Some(&provider)).await
    })
    .await
}

// --- mock OIDC issuer --------------------------------------------------------

async fn oidc_discovery(State(e): State<Arc<Emulator>>) -> Reply {
    let i = e.oidc_issuer();
    ok(json!({"issuer": i, "authorization_endpoint": format!("{i}/authorize"), "token_endpoint": format!("{i}/token"), "jwks_uri": format!("{i}/jwks.json"),
        "response_types_supported": ["code"], "subject_types_supported": ["public"], "id_token_signing_alg_values_supported": ["EdDSA"],
        "code_challenge_methods_supported": ["S256"], "scopes_supported": ["openid", "email", "profile"], "axiom_synthetic_identities_only": true}))
}
async fn oidc_jwks(State(e): State<Arc<Emulator>>) -> Reply {
    ok(json!({"keys": [jwk(&e.oidc_key.verifying_key())]}))
}

/// Without `login_hint`: a page listing the synthetic identities. With one of them: a code.
async fn oidc_authorize(State(e): State<Arc<Emulator>>, Query(q): Query<HashMap<String, String>>) -> Reply {
    let redirect = q.get("redirect_uri").cloned().unwrap_or_default();
    // Only loopback clients: the mock issuer never answers a non-local application.
    if !redirect.starts_with(&format!("{}/", e.origin)) && !loopback(&redirect) {
        return Err(bad("the mock issuer redirects to loopback clients only"));
    }
    let Some(hint) = q.get("login_hint") else {
        let mut page = String::from("<!doctype html><meta charset=utf-8><title>Local OIDC</title><h1>Local mock OIDC issuer</h1><p>Synthetic identities only. Choose one:</p><ul>");
        for identity in &e.identities {
            let mut url = url::Url::parse(&format!("{}/authorize", e.oidc_issuer())).map_err(unavailable)?;
            url.query_pairs_mut().extend_pairs(q.iter().filter(|(k, _)| k.as_str() != "login_hint")).append_pair("login_hint", identity);
            page.push_str(&format!("<li><a href=\"{}\">{}</a></li>", escape(url.as_str()), escape(identity)));
        }
        page.push_str("</ul>");
        return Ok(Html(page).into_response());
    };
    let Some(email) = normalize_email(hint).filter(|h| e.identities.iter().any(|i| normalize_email(i).as_deref() == Some(h.as_str()))) else {
        return Err(bad("not a synthetic identity of this emulator (--identity)"));
    };
    let code = b64u(&random().map_err(unavailable)?);
    e.put(format!("code:{code}"), Pending::Code {
        email,
        client_id: q.get("client_id").cloned().unwrap_or_default(),
        redirect_uri: redirect.clone(),
        nonce: q.get("nonce").cloned(),
        challenge: q.get("code_challenge").cloned(),
        expires: now_ms() + 60_000,
    });
    let mut url = url::Url::parse(&redirect).map_err(unavailable)?;
    url.query_pairs_mut().append_pair("code", &code);
    if let Some(state) = q.get("state") {
        url.query_pairs_mut().append_pair("state", state);
    }
    Ok(found(url.as_str()))
}

async fn oidc_token(State(e): State<Arc<Emulator>>, bytes: Bytes) -> Reply {
    let form: HashMap<String, String> = url::form_urlencoded::parse(&bytes).into_owned().collect();
    if form.get("grant_type").map(String::as_str) != Some("authorization_code") {
        return Err((StatusCode::BAD_REQUEST, axum::Json(json!({"error": "unsupported_grant_type"}))).into_response());
    }
    let answer = e.redeem(form.get("code").map(String::as_str).unwrap_or_default(), form.get("redirect_uri").map(String::as_str).unwrap_or_default(), form.get("code_verifier").map(String::as_str));
    answer.map_or_else(|| Err((StatusCode::BAD_REQUEST, axum::Json(json!({"error": "invalid_grant"}))).into_response()), ok)
}

// --- mail viewer -----------------------------------------------------------

async fn mail_rows(e: &Emulator) -> std::result::Result<Vec<Value>, Response> {
    let rows = e.q("SELECT id,created_at,channel,template,to_address,subject,body_text,challenge_id FROM __axiom_auth_captured_mail ORDER BY created_at DESC, id LIMIT 200", vec![]).await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            json!({"id": r[0], "created_at": r[1].as_deref().and_then(|v| v.parse::<i64>().ok()), "channel": r[2], "template": r[3], "to_address": r[4], "subject": r[5], "body_text": r[6], "challenge_id": r[7]})
        })
        .collect())
}
async fn mail_json(State(e): State<Arc<Emulator>>) -> Reply {
    ok(json!({"mail": mail_rows(&e).await?}))
}
async fn mail_page(State(e): State<Arc<Emulator>>) -> Reply {
    let mut page = String::from("<!doctype html><meta charset=utf-8><meta http-equiv=refresh content=5><title>Local mail</title><h1>Local mail</h1><p>Captured by <code>axiom auth emulate</code>; nothing is delivered.</p><table border=1 cellpadding=4><tr><th>When</th><th>To</th><th>Subject</th><th>Text</th></tr>");
    for m in mail_rows(&e).await? {
        let when = m["created_at"].as_i64().map(rfc3339).unwrap_or_default();
        page.push_str(&format!("<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>", escape(&when), escape(m["to_address"].as_str().unwrap_or_default()), escape(m["subject"].as_str().unwrap_or_default()), escape(m["body_text"].as_str().unwrap_or_default())));
    }
    page.push_str("</table>");
    Ok(Html(page).into_response())
}

async fn not_emulated() -> Reply {
    Err(fail(StatusCode::NOT_FOUND, "AXAUTH_NOT_FOUND", "not part of local auth emulation (password, email code, mock OIDC, refresh and sign-out are)"))
}

/// CORS for the issuer's API: only the configured allowed origins; a request from another
/// origin is refused (`AXAUTH_ORIGIN`), as hosted.
async fn cors(State(e): State<Arc<Emulator>>, request: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let origin = request.headers().get(header::ORIGIN).and_then(|v| v.to_str().ok()).map(str::to_string);
    let allowed = origin.as_ref().is_some_and(|o| e.strings("allowed_origins").contains(o));
    if origin.is_some() && !allowed && request.method() != Method::GET {
        return fail(StatusCode::FORBIDDEN, "AXAUTH_ORIGIN", "this origin is not allowed");
    }
    let preflight = request.method() == Method::OPTIONS;
    let mut response = if preflight { StatusCode::NO_CONTENT.into_response() } else { next.run(request).await };
    if let (true, Some(origin)) = (allowed, origin) {
        let h = response.headers_mut();
        if let Ok(v) = HeaderValue::from_str(&origin) {
            h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, v);
        }
        h.insert(header::VARY, HeaderValue::from_static("Origin"));
        if preflight {
            h.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, POST, DELETE"));
            h.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("authorization, content-type, accept"));
            h.insert(header::ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("600"));
        }
    }
    response
}

/// The emulator's routes.
pub fn router(e: Arc<Emulator>, project: &str) -> Router {
    let base = format!("/local/{project}");
    let api = Router::new()
        .route(&format!("{base}/.well-known/openid-configuration"), get(discovery))
        .route(&format!("{base}/.well-known/jwks.json"), get(jwks))
        .route(&format!("{base}/v1/signup"), post(signup))
        .route(&format!("{base}/v1/signup/verify"), post(signup_verify))
        .route(&format!("{base}/v1/sign-in/password"), post(sign_in_password))
        .route(&format!("{base}/v1/email/code"), post(email_code))
        .route(&format!("{base}/v1/email/code/verify"), post(email_code_verify))
        .route(&format!("{base}/v1/token/refresh"), post(refresh))
        .route(&format!("{base}/v1/token/authorization-code"), post(authorization_code))
        .route(&format!("{base}/v1/logout"), post(logout))
        .route(&format!("{base}/v1/logout-all"), post(logout_all))
        .route(&format!("{base}/v1/user"), get(user))
        .route(&format!("{base}/v1/providers/:slug/start"), get(provider_start))
        .route(&format!("{base}/v1/providers/:slug/callback"), get(provider_callback))
        .route(&format!("{base}/*rest"), get(not_emulated).post(not_emulated))
        .layer(axum::middleware::from_fn_with_state(e.clone(), cors));
    Router::new()
        .merge(api)
        .route("/local/oidc/.well-known/openid-configuration", get(oidc_discovery))
        .route("/local/oidc/jwks.json", get(oidc_jwks))
        .route("/local/oidc/authorize", get(oidc_authorize))
        .route("/local/oidc/token", post(oidc_token))
        .route("/local/mail", get(mail_page))
        .route("/local/mail.json", get(mail_json))
        .with_state(e)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn effective_config(args: &EmulateArgs, project: &str) -> Result<Config> {
    let path = args.config.clone().or_else(|| Some(args.dir.join("auth/config.local.json")).filter(|p| p.exists()));
    let mut config = match path {
        Some(path) => config::parse(&std::fs::read(&path).with_context(|| format!("read {}", path.display()))?)?,
        None => defaults(project),
    };
    for (key, value) in defaults(project).settings {
        config.settings.entry(key).or_insert(value);
    }
    config.source = json!({"kind": "local", "project": project});
    Ok(config)
}

pub async fn run(auth: &AuthArgs, args: &EmulateArgs) -> Result<()> {
    match &args.action {
        Some(EmulateAction::Reset { dir }) => {
            let state = dir.join(STATE_DIR);
            let running = std::fs::read(state.join("state.json")).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| v["port"].as_u64()).is_some_and(|port| {
                std::net::TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], port as u16)), std::time::Duration::from_millis(300)).is_ok()
            });
            if running {
                return Err(usage("the emulator is running: stop it, then reset"));
            }
            if state.exists() {
                std::fs::remove_dir_all(&state).with_context(|| format!("remove {}", state.display()))?;
            }
            print(auth, &json!({"reset": state}), |_| vec![format!("Removed {} (local users, keys, mail and settings).", state.display())])
        }
        Some(EmulateAction::Deployment { deployment, dir }) => {
            let state: Value = serde_json::from_slice(&std::fs::read(dir.join(STATE_DIR).join("state.json")).map_err(|_| usage("the emulator is not running: start `axiom auth emulate` first"))?)?;
            let mut value: Value = serde_json::from_slice(&std::fs::read(deployment).with_context(|| format!("read {}", deployment.display()))?)?;
            if !value.is_object() {
                return Err(usage("not a server deployment JSON file"));
            }
            value["authentication"] = json!({"kind": "jwt", "issuer": state["issuer"], "audience": state["audience"], "algorithm": "EdDSA", "keyEnv": KEY_ENV});
            std::fs::write(deployment, serde_json::to_vec_pretty(&value)?)?;
            let pem = dir.join(STATE_DIR).join("public.pem");
            print(auth, &json!({"deployment": deployment, "issuer": state["issuer"], "key_env": KEY_ENV, "public_key_file": pem}), |_| {
                vec![
                    format!("{} now verifies tokens of {} (EdDSA).", deployment.display(), state["issuer"].as_str().unwrap_or_default()),
                    format!("Run the server with {KEY_ENV}=\"$(cat {})\" (the key changes on every emulator start).", pem.display()),
                ]
            })
        }
        None => serve(auth, args).await,
    }
}

async fn serve(auth: &AuthArgs, args: &EmulateArgs) -> Result<()> {
    for identity in &args.identities {
        if normalize_email(identity).is_none() {
            return Err(usage(format!("--identity {identity}: a synthetic email address is required")));
        }
    }
    if args.audience.is_empty() || args.audience.len() > 256 {
        return Err(usage("--audience is 1 to 256 bytes"));
    }
    let project = project_name(auth, &args.dir);
    let state = args.dir.join(STATE_DIR);
    private_dir(&state)?;
    let config = effective_config(args, &project)?;
    let mut bytes = serde_json::to_vec_pretty(&serde_json::to_value(&config)?)?;
    bytes.push(b'\n');
    write_private(&state.join("config.json"), &bytes)?;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, args.port)).await.with_context(|| format!("bind 127.0.0.1:{}", args.port))?;
    let origin = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    let (emulator, installed) = Emulator::open(&state, &origin, &project, &args.audience, config, args.identities.clone()).await?;
    let summary = json!({
        "issuer": emulator.issuer, "audience": emulator.audience, "project": project, "port": listener.local_addr()?.port(), "pid": std::process::id(),
        "mail": format!("{origin}/local/mail"), "oidc": format!("{origin}/local/oidc"), "public_key_file": state.join("public.pem"),
        "store": {"file": state.join("store.sqlite"), "stream": "axiom-auth", "version": installed.version, "head": installed.head},
    });
    write_private(&state.join("state.json"), &serde_json::to_vec_pretty(&summary)?)?;
    print(auth, &summary, |s| {
        vec![
            format!("Local auth emulation for {project} (no network, no hosted database)."),
            format!("  issuer   {}", s["issuer"].as_str().unwrap_or_default()),
            format!("  mail     {}", s["mail"].as_str().unwrap_or_default()),
            format!("  oidc     {} (synthetic identities)", s["oidc"].as_str().unwrap_or_default()),
            format!("  store    axiom-auth {} on {}", installed.version, state.join("store.sqlite").display()),
            "Ctrl-C (or SIGTERM) stops it; `axiom auth emulate reset` deletes local users, keys and mail.".into(),
        ]
    })?;
    let app = router(emulator, &project);
    let served = axum::serve(listener, app).with_graceful_shutdown(async {
        #[cfg(unix)]
        {
            let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = async { match term.as_mut() { Some(t) => { t.recv().await; } None => std::future::pending::<()>().await } } => {}
            }
        }
        #[cfg(not(unix))]
        let _ = tokio::signal::ctrl_c().await;
    });
    let result = served.await;
    let _ = std::fs::remove_file(state.join("state.json"));
    result.context("the local auth server stopped")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn promotion_drops_local_callbacks_and_the_mock_provider() {
        let mut c = defaults("demo");
        c.settings.insert("redirect_urls".into(), json!(["http://127.0.0.1:5173/auth/callback", "https://app.example.com/auth/callback"]));
        c.settings.insert("webauthn_rp_id".into(), json!("localhost"));
        c.providers = vec![
            json!({"slug": "google", "kind": "google", "client_id": "cid", "scopes": ["openid"], "secret": {"ref": "AUTH_PROVIDER_GOOGLE_CLIENT_SECRET"}}),
            json!({"slug": MOCK_PROVIDER, "kind": "oidc", "client_id": "local", "scopes": [], "secret": {"ref": "LOCAL"}}),
            json!({"slug": "oidc-dev", "kind": "oidc", "client_id": "x", "issuer": "http://127.0.0.1:9999/local/oidc", "scopes": [], "secret": {"ref": "DEV"}}),
        ];
        let p = promotable(c);
        assert_eq!(p.settings["redirect_urls"], json!(["https://app.example.com/auth/callback"]));
        assert_eq!(p.settings["allowed_origins"], json!([]));
        assert!(!p.settings.contains_key("webauthn_rp_id"));
        assert_eq!(p.providers.len(), 1);
        assert_eq!(p.source, json!({"kind": "local", "project": "demo"}));
        // Still a valid export: references only.
        config::parse(&serde_json::to_vec(&p).unwrap()).unwrap();
    }

    #[test]
    fn passwords_and_tokens() {
        let phc = hash_password("correct horse").unwrap();
        assert!(phc.starts_with("$pbkdf2-sha256$i=100000$") && verify_password("correct horse", &phc) && !verify_password("wrong", &phc));
        let key = SigningKey::from_bytes(&[7; 32]);
        let token = sign_jwt(&key, "k", &json!({"sub": "u"}));
        assert_eq!(verify_jwt(&key.verifying_key(), &token).unwrap()["sub"], "u");
        assert!(verify_jwt(&SigningKey::from_bytes(&[8; 32]).verifying_key(), &token).is_none());
        let (signed, _) = token.rsplit_once('.').unwrap();
        let other = sign_jwt(&key, "k", &json!({"sub": "v"}));
        assert!(verify_jwt(&key.verifying_key(), &format!("{signed}.{}", other.rsplit_once('.').unwrap().1)).is_none());
        assert!(public_pem(&key.verifying_key()).starts_with("-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEA"));
    }

    /// The whole local flow over loopback HTTP: sign-up with a captured code, password and
    /// email-code sign-in, refresh rotation and reuse, the mock OIDC issuer, sign-out, CORS;
    /// tokens verified by the runtime's own `jwt` EdDSA authenticator and refused as `axiomAuth`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn local_flow_issues_tokens_the_local_runtime_verifies() {
        use axiom_lib::backend::{AuthenticationProvider, JwtAlgorithm};
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join(STATE_DIR);
        private_dir(&state).unwrap();
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        assert!(listener.local_addr().unwrap().ip().is_loopback());
        let origin = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let (e, installed) = Emulator::open(&state, &origin, "demo", "api", defaults("demo"), vec!["jo@example.com".into()]).await.unwrap();
        assert_eq!(installed.version, "3.0.0");
        let (issuer, key) = (e.issuer.clone(), e.verifying_key());
        tokio::spawn(async move { axum::serve(listener, router(e, "demo")).await });
        let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).no_proxy().build().unwrap();
        let post = |path: &str, body: Value| http.post(format!("{issuer}{path}")).json(&body).send();
        let canary = "Synthetic-Canary-Password-71";
        let mail_code = |challenge: String| {
            let http = http.clone();
            let origin = origin.clone();
            async move {
                let mail: Value = http.get(format!("{origin}/local/mail.json")).send().await.unwrap().json().await.unwrap();
                let m = mail["mail"].as_array().unwrap().iter().find(|m| m["challenge_id"] == challenge.as_str()).cloned().unwrap();
                assert!(!m.to_string().contains(canary));
                m["body_text"].as_str().unwrap().split_whitespace().find(|w| w.len() == 7 && w.ends_with('.')).unwrap().trim_end_matches('.').to_string()
            }
        };
        // Sign-up: the code goes to the mail capture; a wrong code is refused.
        let r: Value = post("/v1/signup", json!({"email": "Ada@Example.com", "password": canary})).await.unwrap().json().await.unwrap();
        let challenge = r["challenge_id"].as_str().unwrap().to_string();
        let code = mail_code(challenge.clone()).await;
        let wrong = post("/v1/signup/verify", json!({"challenge_id": challenge, "code": "000000x"})).await.unwrap();
        assert_eq!(wrong.status(), 400);
        let t: Value = post("/v1/signup/verify", json!({"challenge_id": challenge, "code": code})).await.unwrap().json().await.unwrap();
        let user_id = t["user"]["id"].as_str().unwrap().to_string();
        assert_eq!(t["user"]["email"], "ada@example.com");
        assert!(t["refresh_expires_at"].is_string() && t["session"]["id"].is_string());
        // The local runtime verifies the access token (jwt, EdDSA, public key by reference).
        let pem = public_pem(&key);
        let provider = AuthenticationProvider::Jwt { issuer: issuer.clone(), audience: "api".into(), algorithm: JwtAlgorithm::EdDSA, key_env: KEY_ENV.into() };
        let runtime = axiom_server::authentication::Authenticator::with_environment(provider, |n| (n == KEY_ENV).then(|| pem.clone())).unwrap();
        let bearer = |token: &str| std::collections::BTreeMap::from([("authorization".to_string(), format!("Bearer {token}"))]);
        let principal = runtime.authenticate("GET", &bearer(t["access_token"].as_str().unwrap())).unwrap().unwrap();
        assert_eq!(principal.subject, user_id);
        assert_eq!(principal.claims.get("sid").map(String::as_str), t["session"]["id"].as_str());
        // A Cloud runtime refuses the local issuer outright.
        assert!(AuthenticationProvider::AxiomAuth { issuer: issuer.clone(), audience: "api".into(), oauth_clients: Default::default() }.validate().is_err());
        // Another key (another emulator run) does not verify.
        let other = public_pem(&SigningKey::from_bytes(&[9; 32]).verifying_key());
        let stranger = axiom_server::authentication::Authenticator::with_environment(AuthenticationProvider::Jwt { issuer: issuer.clone(), audience: "api".into(), algorithm: JwtAlgorithm::EdDSA, key_env: KEY_ENV.into() }, |_| Some(other.clone())).unwrap();
        assert!(stranger.authenticate("GET", &bearer(t["access_token"].as_str().unwrap())).is_err());
        // Refresh rotates; presenting the rotated token again ends the session.
        let first = t["refresh_token"].as_str().unwrap().to_string();
        let r2: Value = post("/v1/token/refresh", json!({"refresh_token": first})).await.unwrap().json().await.unwrap();
        let second = r2["refresh_token"].as_str().unwrap().to_string();
        assert_ne!(first, second);
        assert_eq!(post("/v1/token/refresh", json!({"refresh_token": first})).await.unwrap().status(), 401);
        assert_eq!(post("/v1/token/refresh", json!({"refresh_token": second})).await.unwrap().status(), 401);
        // Password sign-in, the user view, sign-out.
        assert_eq!(post("/v1/sign-in/password", json!({"email": "ada@example.com", "password": "wrong-password"})).await.unwrap().status(), 401);
        let p: Value = post("/v1/sign-in/password", json!({"email": "ada@example.com", "password": canary})).await.unwrap().json().await.unwrap();
        let me: Value = http.get(format!("{issuer}/v1/user")).bearer_auth(p["access_token"].as_str().unwrap()).send().await.unwrap().json().await.unwrap();
        assert_eq!((me["id"].as_str(), me["email_verified"].as_bool(), me["has_password"].as_bool()), (Some(user_id.as_str()), Some(true), Some(true)));
        assert_eq!(post("/v1/logout", json!({"refresh_token": p["refresh_token"]})).await.unwrap().status(), 200);
        assert_eq!(post("/v1/token/refresh", json!({"refresh_token": p["refresh_token"]})).await.unwrap().status(), 401);
        assert_eq!(http.get(format!("{issuer}/v1/user")).bearer_auth(p["access_token"].as_str().unwrap()).send().await.unwrap().status(), 401);
        // Email code sign-in.
        let c: Value = post("/v1/email/code", json!({"email": "ada@example.com"})).await.unwrap().json().await.unwrap();
        let challenge = c["challenge_id"].as_str().unwrap().to_string();
        let code = mail_code(challenge.clone()).await;
        let ec: Value = post("/v1/email/code/verify", json!({"challenge_id": challenge, "code": code})).await.unwrap().json().await.unwrap();
        assert_eq!(ec["user"]["id"].as_str(), Some(user_id.as_str()));
        // Mock OIDC: provider start, the issuer's code, the callback, the app's code exchange.
        let verifier = "v".repeat(43);
        let challenge = b64u(&Sha256::digest(verifier.as_bytes()));
        let app = "http://127.0.0.1:5173/auth/callback";
        let start = http.get(format!("{issuer}/v1/providers/{MOCK_PROVIDER}/start")).query(&[("redirect_uri", app), ("state", "s1"), ("code_challenge", &challenge), ("code_challenge_method", "S256"), ("login_hint", "jo@example.com")]).send().await.unwrap();
        assert_eq!(start.status(), 302);
        let authorize = start.headers()["location"].to_str().unwrap().to_string();
        assert!(authorize.starts_with(&format!("{origin}/local/oidc/authorize?")));
        let refused = http.get(authorize.replace("jo%40example.com", "eve%40example.com")).send().await.unwrap();
        assert_eq!(refused.status(), 400);
        let callback = http.get(&authorize).send().await.unwrap().headers()["location"].to_str().unwrap().to_string();
        let back = http.get(&callback).send().await.unwrap().headers()["location"].to_str().unwrap().to_string();
        let back = url::Url::parse(&back).unwrap();
        let q: HashMap<String, String> = back.query_pairs().into_owned().collect();
        assert_eq!((back.as_str().starts_with(app), q["state"].as_str(), q["iss"].as_str()), (true, "s1", issuer.as_str()));
        assert_eq!(post("/v1/token/authorization-code", json!({"code": q["code"], "code_verifier": "w".repeat(43), "redirect_uri": app})).await.unwrap().status(), 400);
        let o: Value = post("/v1/token/authorization-code", json!({"code": q["code"], "code_verifier": verifier, "redirect_uri": app})).await.unwrap().json().await.unwrap();
        // The failed exchange above consumed nothing; this one signs the synthetic identity in.
        assert_eq!(o["user"]["email"], "jo@example.com", "{o}");
        let principal = runtime.authenticate("GET", &bearer(o["access_token"].as_str().unwrap())).unwrap().unwrap();
        assert_ne!(principal.subject, user_id);
        // CORS: configured origins only.
        let evil = http.post(format!("{issuer}/v1/email/code")).header("origin", "https://evil.example").json(&json!({"email": "ada@example.com"})).send().await.unwrap();
        assert_eq!(evil.status(), 403);
        let pre = http.request(reqwest::Method::OPTIONS, format!("{issuer}/v1/token/refresh")).header("origin", "http://127.0.0.1:5173").header("access-control-request-method", "POST").send().await.unwrap();
        assert_eq!((pre.status().as_u16(), pre.headers()["access-control-allow-origin"].to_str().unwrap()), (204, "http://127.0.0.1:5173"));
        // Discovery and the mail page; the password is in no stored row.
        let jwks: Value = http.get(format!("{issuer}/.well-known/jwks.json")).send().await.unwrap().json().await.unwrap();
        assert_eq!(jwks["keys"][0]["x"], b64u(key.as_bytes()));
        assert!(http.get(format!("{origin}/local/mail")).send().await.unwrap().text().await.unwrap().contains("Local mail"));
        let store = std::fs::read(state.join("store.sqlite")).unwrap();
        assert!(!store.windows(canary.len()).any(|w| w == canary.as_bytes()));
        use std::os::unix::fs::PermissionsExt;
        for f in ["store.sqlite", "signing.key", "oidc-signing.key", "public.pem"] {
            assert_eq!(std::fs::metadata(state.join(f)).unwrap().permissions().mode() & 0o777, 0o600, "{f}");
        }
    }
}
