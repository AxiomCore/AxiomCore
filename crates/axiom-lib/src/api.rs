use crate::error::AxiomError;
use reqwest::{multipart, Client};

const AXIOM_CLOUD_URL: &str = "https://httpbin.org/post"; // Mock endpoint

pub async fn upload_artifact(file_path: &str) -> anyhow::Result<()> {
    let client = Client::new();
    let file_content = tokio::fs::read(file_path).await?;

    let form = multipart::Form::new().part(
        "file",
        multipart::Part::bytes(file_content).file_name(file_path.to_string()),
    );

    let response = client.post(AXIOM_CLOUD_URL).multipart(form).send().await?;

    if !response.status().is_success() {
        anyhow::bail!("Upload failed with status: {}", response.status());
    }

    // println!("Mock response: {:?}", response.json::<serde_json::Value>().await?);
    Ok(())
}
