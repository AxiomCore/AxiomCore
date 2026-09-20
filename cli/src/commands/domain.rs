use anyhow::{anyhow, Context, Result};
use axiom_extractor::evaluate_acore_config;
use clap::ValueEnum;
use regex::Regex;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, ValueEnum)]
pub enum DomainSource {
    Openapi,
    Sql,
}

/// Validate and bootstrap the optional Domain Model v1 layer. Bootstrap emits
/// source suggestions only; it never edits a working contract because entity
/// ownership, exposure, and invariants need an explicit developer decision.
pub async fn handle_validate(file: PathBuf, json: bool) -> Result<()> {
    let config = evaluate_acore_config(&file.to_string_lossy(), None)?;
    config.validate_domain()?;
    let manifest = config.compile_domain_manifest()?;
    let report = match manifest {
        Some(value) => serde_json::json!({
            "format": value.format,
            "hash": value.hash,
            "entities": value.entities.len(),
            "relationships": value.relationships.len(),
            "invariants": value.invariants.len(),
            "projections": value.projections.len(),
            "endpoint_bindings": value.endpoint_bindings.len(),
        }),
        None => {
            serde_json::json!({"format": "none", "message": "No optional domain block is declared."})
        }
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else if report["format"] == "none" {
        println!(
            "ℹ️  No Domain Model v1 block is declared in {}.",
            file.display()
        );
    } else {
        println!(
            "✅ Domain Model v1 valid · {} entities · {} relationships · {} projections\n   Manifest: {}",
            report["entities"], report["relationships"], report["projections"], report["hash"]
        );
    }
    Ok(())
}

pub async fn handle_bootstrap(input: PathBuf, source: DomainSource, output: PathBuf) -> Result<()> {
    let raw = fs::read_to_string(&input)
        .with_context(|| format!("read migration input {}", input.display()))?;
    if raw.len() > 8 * 1024 * 1024 {
        return Err(anyhow!("migration input exceeds the 8 MiB safety limit"));
    }
    let suggestion = match source {
        DomainSource::Openapi => bootstrap_openapi(&raw)?,
        DomainSource::Sql => bootstrap_sql(&raw)?,
    };
    if output.exists() {
        return Err(anyhow!(
            "refusing to overwrite {}; choose a new --output path",
            output.display()
        ));
    }
    fs::write(&output, suggestion)
        .with_context(|| format!("write domain suggestions to {}", output.display()))?;
    println!(
        "✅ Wrote Domain Model v1 suggestions to {}\n   Review model names, ownership, audiences, and invariants before copying this block into axiom.acore.",
        output.display()
    );
    Ok(())
}

#[derive(Debug, Clone)]
struct SuggestionEntity {
    name: String,
    model: String,
    key: String,
    fields: Vec<String>,
}

#[derive(Debug, Clone)]
struct SuggestionRelationship {
    id: String,
    from: String,
    to: String,
    via: String,
}

fn bootstrap_openapi(raw: &str) -> Result<String> {
    let value: Value = serde_json::from_str(raw).context("OpenAPI bootstrap expects JSON input")?;
    let schemas = value
        .pointer("/components/schemas")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("OpenAPI document has no components.schemas object"))?;
    let mut entities = Vec::new();
    for (name, schema) in schemas {
        let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
            continue;
        };
        let fields: Vec<_> = properties.keys().cloned().collect();
        if !properties.contains_key("id") {
            continue;
        }
        entities.push(SuggestionEntity {
            name: sanitize_name(name),
            model: name.clone(),
            key: "id".to_string(),
            fields,
        });
    }
    if entities.is_empty() {
        return Err(anyhow!("no object schema with an `id` property was found; choose durable entity identities manually"));
    }
    entities.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(render_suggestions("OpenAPI", entities, vec![]))
}

fn bootstrap_sql(raw: &str) -> Result<String> {
    let table_re = Regex::new(
        r#"(?is)create\s+table\s+(?:if\s+not\s+exists\s+)?(?:[\w]+\.)?\"?([a-zA-Z_][\w]*)\"?\s*\((.*?)\)\s*;"#,
    )?;
    let ref_re = Regex::new(
        r#"(?i)^\s*\"?([a-zA-Z_][\w]*)\"?\s+[^,]*?references\s+(?:[\w]+\.)?\"?([a-zA-Z_][\w]*)\"?"#,
    )?;
    let primary_re = Regex::new(r#"(?i)primary\s+key\s*\(\s*\"?([a-zA-Z_][\w]*)\"?\s*\)"#)?;
    let mut entities = Vec::new();
    let mut relationships = Vec::new();
    for capture in table_re.captures_iter(raw) {
        let table = capture[1].to_string();
        let body = capture[2].to_string();
        let mut fields = BTreeSet::new();
        let mut key = String::new();
        for line in body.split(',') {
            let line = line.trim();
            if let Some(pk) = primary_re.captures(line) {
                key = pk[1].to_string();
                continue;
            }
            let words: Vec<_> = line.split_whitespace().collect();
            if words.is_empty()
                || matches!(
                    words[0].to_lowercase().as_str(),
                    "constraint" | "primary" | "foreign" | "unique" | "check"
                )
            {
                continue;
            }
            let field = words[0].trim_matches('"').to_string();
            fields.insert(field.clone());
            if line.to_lowercase().contains("primary key") {
                key = field.clone();
            }
            if let Some(reference) = ref_re.captures(line) {
                relationships.push(SuggestionRelationship {
                    id: format!(
                        "{}-{}",
                        sanitize_name(&table).to_lowercase(),
                        sanitize_name(&reference[2]).to_lowercase()
                    ),
                    from: sanitize_name(&table),
                    to: sanitize_name(&reference[2]),
                    via: reference[1].to_string(),
                });
            }
        }
        if key.is_empty() && fields.contains("id") {
            key = "id".to_string();
        }
        if key.is_empty() {
            continue;
        }
        entities.push(SuggestionEntity {
            name: sanitize_name(&table),
            model: sanitize_name(&table),
            key,
            fields: fields.into_iter().collect(),
        });
    }
    if entities.is_empty() {
        return Err(anyhow!(
            "no CREATE TABLE with a primary key was found in the SQL schema"
        ));
    }
    entities.sort_by(|a, b| a.name.cmp(&b.name));
    relationships.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(render_suggestions("SQL schema", entities, relationships))
}

fn render_suggestions(
    source: &str,
    entities: Vec<SuggestionEntity>,
    relationships: Vec<SuggestionRelationship>,
) -> String {
    let q = |value: &str| serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string());
    let mut output = format!(
        "// Generated from {source}. This is a reviewable bootstrap, not an authority.\n// It expects matching extracted Models.* symbols in axiom.acore. IDs, relationship\n// cardinality, and ownership are compiler-managed or extractor-inferred.\n\nEntities = Entities {{\n"
    );
    for entity in &entities {
        let alias = entity_alias(&entity.name);
        let key_override = if entity.key.eq_ignore_ascii_case("id") {
            String::new()
        } else {
            format!(" {{ key = Listing {{ {} }} }}", q(&entity.key))
        };
        output.push_str(&format!(
            "  [{alias:?}] = extend Models.{}{key_override}\n",
            entity.model
        ));
    }
    output.push_str("}\n\ndomain {\n  entities = Entities\n  relationships = Relationships {\n");
    for relationship in &relationships {
        output.push_str(&format!(
            "    {} {{ from = Entities.{} to = Entities.{} via = {} required = true }}\n",
            entity_alias(&relationship.id),
            entity_alias(&relationship.from),
            entity_alias(&relationship.to),
            q(&relationship.via)
        ));
    }
    output.push_str("  }\n  projections = Projections {\n");
    for entity in &entities {
        let mut fields = entity.fields.clone();
        fields.sort();
        // This projection remains review-only until an author deliberately
        // chooses its external audience and exposure list.
        let rendered_fields = fields
            .iter()
            .map(|field| q(field))
            .collect::<Vec<_>>()
            .join(" ");
        output.push_str(&format!(
            "    [{}] = DomainProjection {{ entity = Entities.{} fields = Listing {{ {} }} }}\n",
            q(&format!("internal{}", entity.name)),
            entity_alias(&entity.name),
            rendered_fields
        ));
    }
    output.push_str("  }\n}\n");
    output
}

fn entity_alias(value: &str) -> String {
    let name = sanitize_name(value);
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return "entity".to_string();
    };
    format!("{}{}", first.to_ascii_lowercase(), characters.as_str())
}

fn sanitize_name(value: &str) -> String {
    value
        .split(|character: char| !(character.is_ascii_alphanumeric()))
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            let first = characters.next().unwrap_or_default().to_ascii_uppercase();
            format!("{}{}", first, characters.as_str())
        })
        .collect::<String>()
}
