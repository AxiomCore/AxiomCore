use thiserror::Error;

#[derive(Error, Debug)]
pub enum AxiomError {
    #[error("Configuration file 'axiom.yaml' not found in the current directory.")]
    ConfigNotFound,

    #[error("Failed to parse configuration file: {0}")]
    ConfigParse(#[from] serde_yaml::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Network request failed: {0}")]
    Network(#[from] reqwest::Error),

    #[error("Build failed: {0}")]
    BuildFailed(String),

    #[error("Extractor failed for language '{0}': {1}")]
    ExtractorFailed(String, String),

    #[error(
        "Extractor '{0}' is not installed. Run `axiom install --extractor {0}` to install it."
    )]
    ExtractorNotFound(String),
}
