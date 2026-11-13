//! Shared plumbing for the dashboard-parity commands `axiom workspace`, `axiom project
//! environments` and `axiom contract releases` (W4-L7 part 2, P4).
//!
//! Every request goes through [`Api::request`]: the CLI session from `axiom login`, refreshed
//! once on a 401, no redirects, bounded bodies and a 20 s timeout. Nothing here re-implements
//! authentication. Errors are worded from the HTTP status, an `AX*` code when the server sends
//! one, and the server's short plain-text reason with control characters removed.
use super::resources_recovery::{Api, Reply};
use anyhow::{anyhow, bail, Result};
use reqwest::Method;
use serde_json::Value;
use std::io::IsTerminal;

/// One API call: the JSON body of a 2xx reply (`null` for an empty one), otherwise an error
/// in plain words.
pub async fn call(api: &Api, method: Method, segments: &[&str], query: &[(&str, String)], body: Option<&Value>) -> Result<Value> {
    let reply = api.request(&method, segments, query, body, None).await?;
    if (200..300).contains(&reply.status) {
        return Ok(reply.body);
    }
    Err(error(&reply))
}

/// A GET with no query.
pub async fn get(api: &Api, segments: &[&str]) -> Result<Value> {
    call(api, Method::GET, segments, &[], None).await
}

fn code_shaped(code: &str) -> bool {
    code.len() > 2 && code.len() <= 64 && code.starts_with("AX") && code.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

/// Plain words for a failed reply.
pub fn error(reply: &Reply) -> anyhow::Error {
    let code = reply.body.pointer("/error/code").or_else(|| reply.body.get("code")).and_then(Value::as_str).filter(|c| code_shaped(c));
    let reason = reply
        .text
        .clone()
        .or_else(|| ["/message", "/error/message", "/error"].iter().find_map(|p| reply.body.pointer(p).and_then(Value::as_str)).map(clean))
        .filter(|r| !r.is_empty());
    let words = match reply.status {
        400 | 422 => "The request was refused",
        401 => "Your session has expired or is not valid. Run `axiom login` and retry",
        403 => "Your role does not allow this",
        404 => "Not found",
        409 => "This conflicts with the current state",
        413 => "The request is too large",
        429 => "Too many requests or no capacity right now. Retry later",
        _ => "The service is unavailable. Retry later",
    };
    let mut message = String::new();
    if let Some(code) = code {
        message.push_str(code);
        message.push_str(": ");
    }
    message.push_str(words);
    if let Some(reason) = reason {
        message.push_str(": ");
        message.push_str(&reason);
    }
    anyhow!("{message} (HTTP {})", reply.status)
}

/// Terminal-safe text from a server value: no control characters, at most 200 characters.
pub fn clean(value: &str) -> String {
    value.chars().filter(|c| !c.is_control()).take(200).collect()
}

/// The text at a JSON pointer, or `-`.
pub fn text(value: &Value, pointer: &str) -> String {
    match value.pointer(pointer) {
        Some(Value::String(s)) if !s.is_empty() => clean(s),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => "-".into(),
    }
}

/// The items of a list reply: a bare array, or the first array under one of `keys`.
pub fn items<'a>(value: &'a Value, keys: &[&str]) -> Vec<&'a Value> {
    if let Some(list) = value.as_array() {
        return list.iter().collect();
    }
    keys.iter().find_map(|k| value.get(*k).and_then(Value::as_array)).map(|l| l.iter().collect()).unwrap_or_default()
}

/// Prints `value` as pretty JSON with `--json`, otherwise the human lines.
pub fn print(json: bool, value: &Value, human: impl FnOnce(&Value) -> Vec<String>) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        for line in human(value) {
            println!("{line}");
        }
    }
    Ok(())
}

/// The item whose `id` or one of `keys` equals `wanted` (names compare case-insensitively).
pub fn pick<'a>(list: &[&'a Value], wanted: &str, keys: &[&str], what: &str) -> Result<&'a Value> {
    let wanted = wanted.trim();
    let matches = |item: &&Value, exact: bool| {
        std::iter::once("id").chain(keys.iter().copied()).any(|k| match item.get(k).and_then(Value::as_str) {
            Some(v) if exact => v == wanted,
            Some(v) => v.eq_ignore_ascii_case(wanted),
            None => false,
        })
    };
    if let Some(found) = list.iter().find(|i| matches(i, true)) {
        return Ok(found);
    }
    let loose: Vec<_> = list.iter().filter(|i| matches(i, false)).collect();
    match loose.as_slice() {
        [one] => Ok(one),
        [] => bail!("no {what} `{}` here; list them to see the names and ids", clean(wanted)),
        _ => bail!("`{}` matches more than one {what}; pass its id", clean(wanted)),
    }
}

/// The project slug from `--project` (a slug, name or id) or the project linked to this
/// directory (`axiom project link`), through `GET /api/v1/projects`.
pub async fn project_slug(api: &Api, project: Option<&str>) -> Result<String> {
    let wanted = match project {
        Some(p) if !p.trim().is_empty() => p.trim().to_string(),
        _ => crate::auth_store::get_project_id(&std::env::current_dir()?)?
            .ok_or_else(|| anyhow!("no project: pass --project <slug, name or id>, or link this directory with `axiom project link`"))?,
    };
    let listed = get(api, &["projects"]).await?;
    let list = items(&listed, &["projects", "items"]);
    let found = pick(&list, &wanted, &["slug", "name"], "project")?;
    found.get("slug").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string).ok_or_else(|| anyhow!("the project list has no slug for `{}`", clean(&wanted)))
}

/// Destructive actions: `--yes`, or an interactive confirmation on a terminal.
pub fn confirm(yes: bool, question: &str) -> Result<()> {
    if yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        bail!("{question} Nothing was changed: pass --yes to confirm when not on a terminal.");
    }
    if dialoguer::Confirm::new().with_prompt(question).default(false).interact()? {
        Ok(())
    } else {
        bail!("Cancelled; nothing was changed.")
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! An [`Api`] against the scripted loopback [`Mock`] with a throwaway step-up store.
    use super::super::cloud_mock::Mock;
    use super::super::resources_recovery::Api;
    use super::super::step_up::StepUpStore;

    pub struct Harness {
        pub mock: Mock,
        pub api: Api,
        _dir: tempfile::TempDir,
    }

    pub async fn harness() -> Harness {
        let mock = Mock::default();
        let base = mock.start().await;
        let dir = tempfile::tempdir().unwrap();
        let api = Api::new(&base, "access".into(), "refresh".into(), false, StepUpStore::at(dir.path().join("step-up.json"))).unwrap().fast();
        Harness { mock, api, _dir: dir }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::harness;
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn project_names_ids_and_slugs_resolve_to_the_slug() {
        let h = harness().await;
        h.mock.script("GET /api/v1/projects", 200, json!([{"id": "11111111-1111-4111-8111-111111111111", "slug": "demo", "name": "Demo App"}, {"id": "2", "slug": "other", "name": "Other"}]));
        assert_eq!(project_slug(&h.api, Some("demo")).await.unwrap(), "demo");
        assert_eq!(project_slug(&h.api, Some("Demo App")).await.unwrap(), "demo");
        assert_eq!(project_slug(&h.api, Some("demo app")).await.unwrap(), "demo");
        assert_eq!(project_slug(&h.api, Some("11111111-1111-4111-8111-111111111111")).await.unwrap(), "demo");
        assert!(project_slug(&h.api, Some("missing")).await.unwrap_err().to_string().contains("no project `missing`"));
        assert_eq!(h.mock.seen("GET /api/v1/projects")[0].bearer.as_deref(), Some("access"));
    }

    #[tokio::test]
    async fn errors_carry_status_words_code_and_a_clean_reason() {
        let h = harness().await;
        h.mock.script("GET /api/v1/workspace", 403, json!({"code": "AXRESOURCE_DENIED", "message": "nope\u{1b}[31m"}));
        let e = get(&h.api, &["workspace"]).await.unwrap_err().to_string();
        assert_eq!(e, "AXRESOURCE_DENIED: Your role does not allow this: nope[31m (HTTP 403)");
        h.mock.script("GET /api/v1/missing", 404, json!({}));
        assert_eq!(get(&h.api, &["missing"]).await.unwrap_err().to_string(), "Not found (HTTP 404)");
        // Go's http.Error replies are plain text; the reason is kept, cleaned and bounded.
        let plain = Reply { status: 409, body: Value::Null, retry_after: None, text: Some("sandboxes can only be provisioned from a verified deployable release".into()) };
        assert_eq!(error(&plain).to_string(), "This conflicts with the current state: sandboxes can only be provisioned from a verified deployable release (HTTP 409)");
    }

    #[test]
    fn confirmation_without_a_terminal_needs_yes() {
        assert!(confirm(true, "Delete?").is_ok());
        if !std::io::stdin().is_terminal() {
            assert!(confirm(false, "Delete?").unwrap_err().to_string().contains("--yes"));
        }
    }
}
