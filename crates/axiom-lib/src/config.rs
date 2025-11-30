use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::error::AxiomError;

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub project_id: String,
    pub version: String,
    pub variants: HashMap<String, VariantConfig>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VariantConfig {
    pub include: Option<Vec<String>>,
    pub exclude: Option<Vec<String>>,
    pub offline_support: Option<bool>,
}

pub fn load_config() -> anyhow::Result<Config> {
    let content = std::fs::read_to_string("axiom.yaml").map_err(|_| AxiomError::ConfigNotFound)?;

    serde_yaml::from_str(&content).context("Could not parse axiom.yaml")
}

pub fn default_config() -> Config {
    let mut variants = HashMap::new();
    variants.insert(
        "mobile".to_string(),
        VariantConfig {
            include: Some(vec!["endpoints.*".to_string()]),
            exclude: Some(vec!["endpoints.admin.*".to_string()]),
            offline_support: Some(true),
        },
    );

    Config {
        project_id: "my-org/my-project".to_string(),
        version: "0.1.0".to_string(),
        variants,
    }
}
