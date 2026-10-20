use anyhow::{bail, Result};
use axiom_cloud::resources::ResourceSelection;
use clap::Subcommand;
use reqwest::Method;
use serde_json::json;

#[derive(Subcommand, Debug)]
pub enum Action {
    /// Create or reuse the project's hosted Stripe Checkout session
    Checkout {
        #[arg(long)]
        operation_id: Option<String>,
    },
    /// Open the project's Stripe subscription management URL
    BillingPortal,

    /// Save database preferences for an existing, unconfigured project
    Configure {
        #[arg(long,default_value="free",value_parser=["free","pro"])]
        tier: String,
        #[arg(long,value_parser=["sqlite","postgres"])]
        database: Option<String>,
        #[arg(long)]
        region: Option<String>,
        #[arg(long, default_value = "development")]
        environment: String,
    },
    /// Request a lifecycle operation. The ID can be reused after a network failure.
    Request {
        #[arg(long)]
        resource: String,
        #[arg(long,value_parser=["provision","retry","suspend","resume","delete","rotate"])]
        action: String,
        #[arg(long)]
        generation: u64,
        #[arg(long)]
        operation_id: Option<String>,
        /// Required phrase for deletion: "delete database"
        #[arg(long)]
        confirm: Option<String>,
        /// Poll operation metadata for up to five minutes
        #[arg(long)]
        wait: bool,
    },
    /// Show currently configured databases and location admission
    Catalog,
}
pub async fn handle(project: &str, action: Option<&Action>) -> Result<()> {
    let client = crate::auth_store::authenticated_cloud_client()?;
    let base = ["projects", project, "resources"];
    let value = match action {
        None => {
            client
                .resource_request(Method::GET, &base, None, None)
                .await?
        }
        Some(Action::Checkout { operation_id }) => {
            let key = operation_id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let _ = uuid::Uuid::parse_str(&key)?;
            eprintln!("Checkout idempotency key: {key}");
            client
                .resource_request(
                    Method::POST,
                    &["projects", project, "resources", "checkout"],
                    Some(&json!({})),
                    Some(&key),
                )
                .await?
        }
        Some(Action::BillingPortal) => {
            client
                .resource_request(
                    Method::POST,
                    &["projects", project, "resources", "billing-portal"],
                    Some(&json!({})),
                    None,
                )
                .await?
        }
        Some(Action::Catalog) => {
            client
                .resource_request(Method::GET, &["resource-catalog"], None, None)
                .await?
        }
        Some(Action::Configure {
            tier,
            database,
            region,
            environment,
        }) => {
            let selection =
                ResourceSelection::new(tier, database.as_deref(), region.as_deref(), environment)?;
            client
                .resource_request(
                    Method::POST,
                    &["projects", project, "resources", "configuration"],
                    Some(&serde_json::to_value(selection)?),
                    None,
                )
                .await?
        }
        Some(Action::Request {
            resource,
            action,
            generation,
            operation_id,
            confirm,
            wait,
        }) => {
            if *generation == 0 {
                bail!("--generation must be positive");
            }
            let _ = uuid::Uuid::parse_str(resource)
                .map_err(|_| anyhow::anyhow!("--resource must be a UUID"))?;
            if action == "delete" && confirm.as_deref() != Some("delete database") {
                bail!("Deletion requires --confirm 'delete database'");
            }
            let key = operation_id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let _ = uuid::Uuid::parse_str(&key)
                .map_err(|_| anyhow::anyhow!("--operation-id must be a UUID"))?;
            eprintln!("Operation idempotency key: {key}");
            let value = client
                .resource_request(
                    Method::POST,
                    &["projects", project, "resources", resource, action],
                    Some(&json!({"generation":generation,"confirm":confirm})),
                    Some(&key),
                )
                .await?;
            if *wait {
                let id = value["operation_id"].as_str().unwrap_or_default();
                let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
                while tokio::time::Instant::now() < deadline {
                    let status = client
                        .resource_request(Method::GET, &base, None, None)
                        .await?;
                    if let Some(operation) = status["operations"]
                        .as_array()
                        .and_then(|ops| ops.iter().find(|op| op["id"].as_str() == Some(id)))
                    {
                        match operation["status"].as_str(){Some("succeeded")=>{println!("{}",serde_json::to_string_pretty(&status)?);return Ok(());},Some("failed")=>bail!("Resource operation failed. Inspect project resources for its safe failure code."),_=>{}}
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
                bail!("Operation is still pending. Run `axiom project resources --project {project}` to inspect it.");
            }
            value
        }
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
