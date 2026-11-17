//! Configuration values of `axiom deploy backend` (`--env KEY=VALUE`, `--env-file PATH`):
//! public configuration for the `env:NAME` references in deployment.json's
//! `bindings[*].configuration`. They are sent as `env` in create-version and sealed into
//! the signed launch manifest; they are not secrets.
//!
//! The key rules mirror axiom-backend `validateEnvironment`
//! (internal/domain/deployments/model.go), which answers `AXDEPLOY_INVALID_ENV` (400/413).
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// At most this many values per version.
pub const MAX_KEYS: usize = 64;
/// At most this many bytes for the JSON-encoded map.
pub const MAX_JSON_BYTES: usize = 32768;

pub type Env = BTreeMap<String, String>;

/// The rules in one sentence, for a refused name and for the server's `AXDEPLOY_INVALID_ENV`.
pub const RULES: &str = "Names use letters, digits and _ and do not start with a digit; names starting with AXIOM_, PORT, and names containing PROXY, TOKEN, PASSWORD, SECRET or CREDENTIAL or ending in _KEY are refused; at most 64 values and 32 KiB in total";

/// Where secrets go instead.
pub const SECRETS_HINT: &str = "Configuration values are public (they are sealed into the deployment); keep secrets in `axiom secrets` and bind them as secrets.";

fn is_name(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Why the server refuses this name, in plain words; `None` when it is accepted.
pub fn refused(key: &str) -> Option<String> {
    if !is_name(key) {
        return Some(format!("{} is not a valid name (use letters, digits and _, and do not start with a digit)", super::clean(&format!("{key:?}"))));
    }
    let upper = key.to_ascii_uppercase();
    if upper.starts_with("AXIOM_") || upper == "PORT" {
        return Some(format!("{key} is reserved for the hosting runtime (names starting with AXIOM_, and PORT)"));
    }
    if ["PROXY", "TOKEN", "PASSWORD", "SECRET", "CREDENTIAL"].iter().any(|w| upper.contains(w)) || upper.ends_with("_KEY") {
        return Some(format!("{key} looks like a secret (names containing PROXY, TOKEN, PASSWORD, SECRET or CREDENTIAL, or ending in _KEY, are refused)"));
    }
    None
}

/// The map's size as Go's `json.Marshal` writes it (what the server measures): serde_json
/// writes `<`, `>`, `&`, U+2028 and U+2029 as they are; Go escapes them (`\u003c`: +5 bytes,
/// `\u2028`: +3 bytes). Every other escape is the same in both.
pub fn go_json_len(env: &Env) -> usize {
    let extra = |s: &str| s.chars().map(|c| match c { '<' | '>' | '&' => 5, '\u{2028}' | '\u{2029}' => 3, _ => 0 }).sum::<usize>();
    serde_json::to_vec(env).map(|b| b.len()).unwrap_or(usize::MAX).saturating_add(env.iter().map(|(k, v)| extra(k) + extra(v)).sum())
}

/// The server's checks on the whole map, locally.
pub fn validate(env: &Env) -> Result<(), String> {
    let bad: Vec<String> = env.keys().filter_map(|k| refused(k)).collect();
    if !bad.is_empty() {
        let secret = env.keys().any(|k| is_name(k) && refused(k).is_some_and(|r| r.contains("looks like a secret")));
        return Err(format!("Invalid configuration name: {}.{}", bad.join("; "), if secret { format!(" {SECRETS_HINT}") } else { String::new() }));
    }
    if env.len() > MAX_KEYS {
        return Err(format!("Too many configuration values: {} given, at most {MAX_KEYS}.", env.len()));
    }
    let bytes = go_json_len(env);
    if bytes > MAX_JSON_BYTES {
        return Err(format!("The configuration values are too large: {bytes} bytes as JSON, at most {MAX_JSON_BYTES} (32 KiB)."));
    }
    Ok(())
}

/// A dotenv file: `KEY=VALUE` lines, `#` comment lines, blank lines, an optional `export `
/// prefix and optional single or double quotes around the value (inside double quotes,
/// `\n`, `\"` and `\\` are escapes). A later line wins over an earlier one.
pub fn parse_dotenv(text: &str) -> Result<Env, String> {
    let mut env = Env::new();
    for (i, raw) in text.trim_start_matches('\u{feff}').lines().enumerate() {
        let n = i + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").map(str::trim_start).unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("line {n} is not KEY=VALUE"));
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(format!("line {n} has no name before ="));
        }
        let value = unquote(value.trim()).map_err(|e| format!("line {n} ({}): {e}", super::clean(key)))?;
        env.insert(key.to_string(), value);
    }
    Ok(env)
}

fn unquote(value: &str) -> Result<String, &'static str> {
    let Some(first) = value.chars().next().filter(|c| *c == '"' || *c == '\'') else {
        // ` # note` after an unquoted value is a comment; `a#b` (no space before #) is kept.
        let cut = value.char_indices().find(|&(i, c)| c == '#' && value[..i].ends_with(char::is_whitespace)).map(|(i, _)| i);
        return Ok(cut.map(|i| value[..i].trim_end()).unwrap_or(value).to_string());
    };
    // The closing quote: the first unescaped `first` after the opening one.
    let mut close = None;
    let mut escaped = false;
    for (i, c) in value.char_indices().skip(1) {
        if first == '"' && !escaped && c == '\\' {
            escaped = true;
            continue;
        }
        if c == first && !escaped {
            close = Some(i);
            break;
        }
        escaped = false;
    }
    let Some(close) = close else {
        return Err("the quote around the value is not closed");
    };
    let rest = value[close + 1..].trim_start();
    if !rest.is_empty() && !rest.starts_with('#') {
        return Err("only a # comment may follow the closing quote");
    }
    let inner = &value[1..close];
    if first == '\'' {
        return Ok(inner.to_string());
    }
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some(e @ ('"' | '\\')) => out.push(e),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    Ok(out)
}

/// One `--env` value: `KEY=VALUE` (the value may be empty or contain `=`).
pub fn parse_flag(flag: &str) -> Result<(String, String), String> {
    match flag.split_once('=') {
        Some((key, value)) if !key.is_empty() => Ok((key.to_string(), value.to_string())),
        _ => Err(format!("--env takes KEY=VALUE, got {}", super::clean(&format!("{flag:?}")))),
    }
}

/// `--env-file` first, then every `--env` over it.
pub fn collect(flags: &[String], file: Option<&Path>) -> Result<Env, String> {
    let mut env = match file {
        Some(path) => {
            let shown = path.display().to_string();
            let text = std::fs::read_to_string(path).map_err(|e| {
                let reason = e.to_string();
                let reason = reason.find(" (os error").map(|i| reason[..i].to_string()).unwrap_or(reason);
                format!("Could not read --env-file {shown} ({}).", reason.to_lowercase())
            })?;
            parse_dotenv(&text).map_err(|e| format!("--env-file {shown}: {e}."))?
        }
        None => Env::new(),
    };
    for flag in flags {
        let (key, value) = parse_flag(flag)?;
        env.insert(key, value);
    }
    Ok(env)
}

/// Every environment name referenced in `bindings[*].configuration` (any depth), with the
/// bindings that use it. As in the runtime, every configuration value is a reference and the
/// `env:` prefix is optional (`"GREETING"` and `"env:GREETING"` both read GREETING); protected
/// spellings (`secret:…`, `config:…`) are not environment names and are skipped.
pub fn references(deployment: &Value) -> BTreeMap<String, BTreeSet<String>> {
    fn walk(v: &Value, binding: &str, out: &mut BTreeMap<String, BTreeSet<String>>) {
        match v {
            Value::String(s) => {
                let name = s.strip_prefix("env:").unwrap_or(s);
                if is_name(name) {
                    out.entry(name.to_string()).or_default().insert(binding.to_string());
                }
            }
            Value::Array(items) => items.iter().for_each(|i| walk(i, binding, out)),
            Value::Object(map) => map.values().for_each(|i| walk(i, binding, out)),
            _ => {}
        }
    }
    let mut out = BTreeMap::new();
    if let Some(bindings) = deployment.get("bindings").and_then(Value::as_object) {
        for (name, binding) in bindings {
            if let Some(configuration) = binding.get("configuration") {
                walk(configuration, name, &mut out);
            }
        }
    }
    out
}

/// What the precheck found for a deploy that is allowed to go on.
#[derive(Debug, Default, PartialEq)]
pub struct Checked {
    /// Values given that no binding references (sent anyway).
    pub unreferenced: Vec<String>,
}

/// The local precheck: names the server would refuse, then references without a value.
pub fn precheck(env: &Env, deployment: Option<&Value>) -> Result<Checked, String> {
    validate(env)?;
    let refs = deployment.map(references).unwrap_or_default();
    let bound = |bindings: &BTreeSet<String>| bindings.iter().map(|b| super::clean(b)).collect::<Vec<_>>().join(", ");
    let unusable: Vec<String> = refs.iter().filter_map(|(name, by)| refused(name).map(|why| format!("{why}, bound by {}", bound(by)))).collect();
    if !unusable.is_empty() {
        return Err(format!(
            "deployment.json binds configuration to names a hosted deployment cannot receive: {}. Rename them in deployment.json (and the --env values), then deploy again. {SECRETS_HINT}",
            unusable.join("; ")
        ));
    }
    let missing: Vec<(&String, &BTreeSet<String>)> = refs.iter().filter(|(name, _)| !env.contains_key(*name)).collect();
    if !missing.is_empty() {
        let list = missing.iter().map(|(name, by)| format!("{name} (bound by {})", bound(by))).collect::<Vec<_>>().join(", ");
        let flags = missing.iter().map(|(name, _)| format!("--env {name}=...")).collect::<Vec<_>>().join(" ");
        let pronoun = if missing.len() == 1 { "it" } else { "them" };
        return Err(format!("Missing configuration for this backend: {list}. Pass {pronoun} with {flags} or --env-file .env, then deploy again."));
    }
    Ok(Checked { unreferenced: env.keys().filter(|k| !refs.contains_key(*k)).cloned().collect() })
}

/// The journal's fingerprint of the values: hex SHA-256 of the sorted map as JSON, empty
/// for no values (so a journal written before `--env` existed still resumes). The values
/// themselves are never written to the journal.
pub fn fingerprint(env: &Env) -> String {
    if env.is_empty() {
        return String::new();
    }
    super::bundle::sha256_hex(&serde_json::to_vec(env).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dotenv_lines_comments_quotes_and_export() {
        let text = "\u{feff}# comment\n\nGREETING=hello world\nexport MODE = 'prod'\nQUOTED=\"a \\\"b\\\"\\nc\"\nEMPTY=\nURL=https://x.test/?a=b#frag\nGREETING=again\n";
        let env = parse_dotenv(text).unwrap();
        assert_eq!(env["GREETING"], "again");
        assert_eq!(env["MODE"], "prod");
        assert_eq!(env["QUOTED"], "a \"b\"\nc");
        assert_eq!(env["EMPTY"], "");
        assert_eq!(env["URL"], "https://x.test/?a=b#frag");
        assert_eq!(env.len(), 5);
        assert_eq!(parse_dotenv("A=1\nnot a pair\n").unwrap_err(), "line 2 is not KEY=VALUE");
        assert_eq!(parse_dotenv("=1").unwrap_err(), "line 1 has no name before =");
        assert_eq!(parse_dotenv("A=\"open").unwrap_err(), "line 1 (A): the quote around the value is not closed");
        assert_eq!(parse_dotenv("A='").unwrap_err(), "line 1 (A): the quote around the value is not closed");
        let commented = parse_dotenv("A=\"x # kept\" # note\nB=plain # note\nC='y' #c\nD=a#b\n").unwrap();
        assert_eq!((commented["A"].as_str(), commented["B"].as_str(), commented["C"].as_str(), commented["D"].as_str()), ("x # kept", "plain", "y", "a#b"));
        assert_eq!(parse_dotenv("A=\"x\"y").unwrap_err(), "line 1 (A): only a # comment may follow the closing quote");
    }

    #[test]
    fn flags_override_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".env");
        std::fs::write(&file, "A=file\nB=file\n").unwrap();
        let env = collect(&["B=flag".into(), "C=x=y".into(), "D=".into()], Some(&file)).unwrap();
        assert_eq!(env, Env::from([("A".into(), "file".into()), ("B".into(), "flag".into()), ("C".into(), "x=y".into()), ("D".into(), "".into())]));
        assert_eq!(collect(&["NOVALUE".into()], None).unwrap_err(), "--env takes KEY=VALUE, got \"NOVALUE\"");
        assert_eq!(collect(&["=v".into()], None).unwrap_err(), "--env takes KEY=VALUE, got \"=v\"");
        let missing = collect(&[], Some(&dir.path().join("nope"))).unwrap_err();
        assert!(missing.starts_with("Could not read --env-file ") && missing.contains("no such file") && !missing.contains("os error"), "{missing}");
    }

    #[test]
    fn key_rules_match_the_server() {
        for ok in ["GREETING", "_x", "a1", "MONKEY", "KEYS", "API_KEYS", "PORTAL", "AXIOMATIC"] {
            assert_eq!(refused(ok), None, "{ok}");
        }
        for bad in ["1A", "A-B", "A B", "", "é", "AXIOM_X", "axiom_x", "PORT", "port", "HTTP_PROXY", "GH_TOKEN", "DB_PASSWORD", "MY_SECRET", "CREDENTIALS", "API_KEY", "api_key"] {
            assert!(refused(bad).is_some(), "{bad}");
        }
        assert_eq!(refused("PORT").unwrap(), "PORT is reserved for the hosting runtime (names starting with AXIOM_, and PORT)");
        assert_eq!(refused("1A").unwrap(), "\"1A\" is not a valid name (use letters, digits and _, and do not start with a digit)");
        let e = validate(&Env::from([("API_KEY".into(), "x".into())])).unwrap_err();
        assert!(e.starts_with("Invalid configuration name: API_KEY looks like a secret") && e.ends_with(SECRETS_HINT), "{e}");
        let many: Env = (0..65).map(|i| (format!("K{i}"), String::new())).collect();
        assert_eq!(validate(&many).unwrap_err(), "Too many configuration values: 65 given, at most 64.");
        let big = Env::from([("A".into(), "x".repeat(MAX_JSON_BYTES))]);
        assert!(validate(&big).unwrap_err().starts_with("The configuration values are too large: 32776 bytes as JSON"));
        let edge = Env::from([("A".into(), "x".repeat(MAX_JSON_BYTES - 8))]);
        assert_eq!(serde_json::to_vec(&edge).unwrap().len(), MAX_JSON_BYTES);
        assert!(validate(&edge).is_ok());
        // Measured like Go's json.Marshal: & < > are 6 bytes each, U+2028 is 6 bytes.
        let amps = Env::from([("A".into(), "&".repeat(6000))]);
        assert_eq!(go_json_len(&amps), 8 + 6000 * 6);
        assert!(validate(&amps).unwrap_err().starts_with("The configuration values are too large: 36008 bytes"));
        assert_eq!(go_json_len(&Env::from([("A".into(), "<\u{2028}é".into())])), 8 + 6 + 6 + 2);
    }

    #[test]
    fn references_come_from_binding_configuration_only() {
        let deployment = json!({"bindings": {
            "greeting": {"provider": "publicConfiguration", "configuration": {"greeting": "env:GREETING", "bare": "MODE", "words": "not a name", "nested": {"list": ["env:REGION"]}}},
            "store": {"provider": "typed", "configuration": {"region": "env:REGION", "key": "env:secret:credential"}, "secrets": {"key": "env:DB_SECRET"}},
            "empty": {"provider": "x"}
        }, "configuration": {"other": "env:IGNORED"}});
        let refs = references(&deployment);
        assert_eq!(refs.keys().collect::<Vec<_>>(), ["GREETING", "MODE", "REGION"]);
        assert_eq!(refs["REGION"].iter().collect::<Vec<_>>(), ["greeting", "store"]);
        assert!(references(&json!({})).is_empty());
        assert!(references(&json!({"bindings": []})).is_empty());
    }

    #[test]
    fn precheck_names_what_is_missing_and_what_is_extra() {
        let deployment = json!({"bindings": {"greeting": {"provider": "publicConfiguration", "configuration": {"greeting": "env:GREETING"}}}});
        assert_eq!(
            precheck(&Env::new(), Some(&deployment)).unwrap_err(),
            "Missing configuration for this backend: GREETING (bound by greeting). Pass it with --env GREETING=... or --env-file .env, then deploy again."
        );
        let two = json!({"bindings": {"a": {"configuration": {"x": "env:X", "y": "env:Y"}}, "b": {"configuration": {"y": "env:Y"}}}});
        assert_eq!(
            precheck(&Env::new(), Some(&two)).unwrap_err(),
            "Missing configuration for this backend: X (bound by a), Y (bound by a, b). Pass them with --env X=... --env Y=... or --env-file .env, then deploy again."
        );
        let given = Env::from([("GREETING".into(), "hi".into()), ("EXTRA".into(), "1".into())]);
        assert_eq!(precheck(&given, Some(&deployment)).unwrap(), Checked { unreferenced: vec!["EXTRA".into()] });
        assert_eq!(precheck(&given, None).unwrap().unreferenced, ["EXTRA", "GREETING"]);
        let refused_ref = json!({"bindings": {"api": {"configuration": {"k": "env:API_KEY"}}}});
        let e = precheck(&Env::new(), Some(&refused_ref)).unwrap_err();
        assert!(e.starts_with("deployment.json binds configuration to names a hosted deployment cannot receive: API_KEY looks like a secret") && e.contains("bound by api"), "{e}");
        assert!(precheck(&Env::from([("PORT".into(), "1".into())]), Some(&deployment)).unwrap_err().starts_with("Invalid configuration name: PORT is reserved"));
    }

    #[test]
    fn fingerprint_is_order_free_and_empty_for_none() {
        assert_eq!(fingerprint(&Env::new()), "");
        let a = Env::from([("A".into(), "1".into()), ("B".into(), "2".into())]);
        let mut b = Env::new();
        b.insert("B".into(), "2".into());
        b.insert("A".into(), "1".into());
        assert_eq!(fingerprint(&a), fingerprint(&b));
        assert_eq!(fingerprint(&a).len(), 64);
        assert_ne!(fingerprint(&a), fingerprint(&Env::from([("A".into(), "1".into()), ("B".into(), "3".into())])));
    }
}
