//! Wire schema parity is checked against actual Serde readers, never domain execution.
use axiom_lib::editor_artifact as wire;
use serde_json::{json, Value};
fn minimum(schema: &Value, depth: usize) -> Value {
    assert!(depth < 64);
    let s = wire::deref(schema);
    if let Some(v) = s.get("const") {
        return v.clone();
    }
    if let Some(v) = s.get("anyOf").and_then(Value::as_array) {
        return minimum(&v[0], depth + 1);
    }
    match s["type"].as_str() {
        Some("object") => {
            let mut m = serde_json::Map::new();
            for key in s["required"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                m.insert(key.into(), minimum(&s["properties"][key], depth + 1));
            }
            Value::Object(m)
        }
        Some("array") => Value::Array(
            s["prefixItems"]
                .as_array()
                .map(|a| a.iter().map(|s| minimum(s, depth + 1)).collect())
                .unwrap_or_default(),
        ),
        Some("string") => json!(""),
        Some("boolean") => json!(false),
        Some("integer" | "number") => s.get("minimum").cloned().unwrap_or(json!(0)),
        Some("null") => Value::Null,
        _ => json!({}),
    }
}
fn parity<T: serde::de::DeserializeOwned>(name: &str) {
    let role = wire::SCHEMAS["roles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["role"] == name)
        .unwrap();
    let mut sample = minimum(&role["schema"], 0);
    if let Some(format) = role["format"].as_str() {
        if sample.get("format").is_some() {
            sample["format"] = json!(format)
        } else if sample.get("body").is_some() {
            sample["body"]["format"] = json!(format)
        } else if sample.get("manifest").is_some() {
            sample["manifest"]["format"] = json!(format)
        }
    }
    let result = serde_json::from_value::<T>(sample.clone());
    assert!(result.is_ok(), "{name}: {:?}, {}", result.err(), sample);
    assert!(
        wire::validate(&sample, role).is_empty(),
        "{name}: {:?}",
        wire::validate(&sample, role)
    );
    // Exercise requiredness and primitive type boundaries at each root property.
    if let Some(properties) = wire::deref(&role["schema"])
        .get("properties")
        .and_then(Value::as_object)
    {
        for (key, schema) in properties {
            if key == "format" {
                continue;
            }
            for value in [
                json!(false),
                json!(17),
                json!("example"),
                json!([]),
                Value::Null,
            ] {
                let schema = wire::deref(schema);
                let object_shape = schema["type"] == "object"
                    || schema
                        .get("anyOf")
                        .and_then(Value::as_array)
                        .is_some_and(|vs| vs.iter().any(|v| wire::deref(v)["type"] == "object"));
                if value.is_array() && object_shape {
                    continue;
                }
                let mut changed = sample.clone();
                changed[key] = value;
                let native = serde_json::from_value::<T>(changed.clone()).is_ok();
                assert_eq!(
                    native,
                    wire::validate(&changed, role).is_empty(),
                    "{name}.{key}: {changed}"
                );
            }
        }
    }
    if sample.is_object() {
        sample["unknownEditorField"] = json!(true);
        assert_eq!(
            serde_json::from_value::<T>(sample.clone()).is_ok(),
            wire::validate(&sample, role).is_empty(),
            "{name}: unknown field"
        );
    }
}
#[test]
fn every_generated_private_wire_root_matches_serde() {
    parity::<axiom_lib::database::DatabaseContractIR>("database-contract");
    parity::<axiom_lib::backend::ServerPlan>("server-plan");
    parity::<axiom_lib::backend::Deployment>("server-deployment");
    parity::<axiom_lib::backend::ActionBindings>("action-bindings");
    parity::<axiom_lib::backend::TargetProfile>("server-target");
    parity::<axiom_lib::backend::MockFixtures>("server-mocks");
    parity::<axiom_lib::backend::ServerSession>("server-session");
    parity::<axiom_server::delivery::DeliveryDeployment>("delivery-deployment");
    parity::<axiom_lib::ui_contract::UiContractLock>("ui-lock");
    parity::<axiom_lib::package_resolver::PackageDependencyLock>("package-lock");
    parity::<axiom_lib::authored_dependencies::AuthoredDependencyLock>("dependency-lock");
    parity::<axiom_ui::application_test::Suite>("application-suite");
    parity::<axiom_database::inspection::bindings::Deployment>("database-deployment");
    parity::<axiom_database::inspection::bindings::Endpoint>("database-endpoint");
    parity::<axiom_database::inspection::bindings::Credential>("database-credential");
    parity::<axiom_database::execution::model::Target>("database-target");
    parity::<axiom_database::sqlite::Binding>("database-sqlite-binding");
    parity::<axiom_database::planning::validation::Development>("database-development");
    parity::<axiom_database::planning::tests::Descriptor>("database-fixtures");
    parity::<axiom_database::planning::checkpoint::Archive>("database-fresh-install");
    parity::<axiom_database::execution::repair::Permit>("database-repair");
    parity::<axiom_database::delivery::model::Artifact>("database-fleet");
    parity::<axiom_database::delivery::model::Inventory>("database-fleet-inventory");
    parity::<axiom_database::delivery::model::Policy>("database-fleet-policy");
    parity::<axiom_database::delivery::model::Signed<axiom_database::delivery::model::Review>>(
        "database-fleet-review",
    );
    parity::<axiom_database::delivery::model::Signed<axiom_database::delivery::model::Promotion>>(
        "database-fleet-promotion",
    );
    parity::<axiom_database::delivery::model::Signed<axiom_database::delivery::model::Receipt>>(
        "database-fleet-receipt",
    );
    parity::<axiom_database::delivery::production::ProviderEvidence>("database-provider-evidence");
    parity::<axiom_database::delivery::model::Signed<axiom_database::delivery::production::Recovery>>(
        "database-recovery",
    );
    parity::<
        axiom_database::delivery::model::Signed<axiom_database::delivery::production::Admission>,
    >("database-admission");
    parity::<axiom_lib::database::runtime::RuntimePlan>("runtime-prepared");
    parity::<axiom_server::persistence::Binding>("runtime-binding");
    parity::<axiom_lib::database::inspection::Report>("database-inspection");
    parity::<axiom_lib::database::planning::Report>("database-change");
}
