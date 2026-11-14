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
    ///
    /// Recovery actions: `restore` (with --at or --backup), `archive`, `unarchive` (resume an
    /// archived database) and `retire_previous`. Restore, archive and retire_previous need a
    /// recent sign-in; the CLI runs `axiom login --step-up` when the server asks for it.
    Request {
        #[arg(long)]
        resource: String,
        #[arg(long,value_parser=["provision","retry","suspend","resume","delete","rotate","restore","archive","unarchive","retire_previous","convert"])]
        action: String,
        #[arg(long)]
        generation: u64,
        #[arg(long)]
        operation_id: Option<String>,
        /// Typed confirmation: "delete database", "restore <database name> to <at or backup id>",
        /// "archive <database name>", or the retire_previous phrase
        #[arg(long)]
        confirm: Option<String>,
        /// Restore: the point in time to return application data to (RFC 3339)
        #[arg(long, conflicts_with = "backup")]
        at: Option<String>,
        /// Restore: a daily backup id from `recovery`
        #[arg(long)]
        backup: Option<String>,
        /// Poll the operation until it finishes (five minutes; 40 for restore, archive and resume)
        #[arg(long)]
        wait: bool,
        /// Recovery actions: print the JSON response instead of plain text
        #[arg(long)]
        json: bool,
        /// Convert: the validated conversion ID from `axiom database convert submit`
        #[arg(long)]
        conversion: Option<String>,
        /// Convert: every signed-in session ends
        #[arg(long)]
        acknowledge_sessions_end: bool,
        /// Convert: the runtime must be redeployed before it serves again
        #[arg(long)]
        acknowledge_redeploy: bool,
        /// Convert: the report's semantic differences are accepted
        #[arg(long)]
        acknowledge_semantics: bool,
        /// Convert: the copy acknowledgement text the dashboard shows
        #[arg(long)]
        acknowledge_copy: Option<String>,
        /// Convert: the external-writers acknowledgement text the dashboard shows
        #[arg(long)]
        acknowledge_external_writers: Option<String>,
    },
    /// Connect your own Turso database (bring your own; placement/v1 4). The platform API
    /// token is read from stdin with --token-stdin (or a masked prompt), never from arguments
    Connect {
        #[arg(value_parser = ["turso"])]
        provider: String,
        #[command(flatten)]
        select: crate::commands::resources_placement::Select,
        #[arg(long)]
        organization: String,
        #[arg(long)]
        group: String,
        #[arg(long)]
        database: String,
        /// Read the Turso platform API token from standard input
        #[arg(long)]
        token_stdin: bool,
        /// `expiry` (default) or `rotate` (needs the db:rotate-creds scope)
        #[arg(long, default_value = "expiry", value_parser = ["expiry", "rotate"])]
        fence_mode: String,
        /// Application data stays in your Turso database, authentication with AxiomCore; not atomic
        #[arg(long)]
        acknowledge_split_store: bool,
        /// With --fence-mode rotate: rotation invalidates database tokens you minted yourself
        #[arg(long)]
        acknowledge_rotate: bool,
        /// "reclaim <database>": take over a database another resource claimed
        #[arg(long)]
        reclaim: Option<String>,
        #[arg(long)]
        operation_id: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Show the customer database connection (token shown only as set and its expiry)
    Connection {
        #[command(flatten)]
        select: crate::commands::resources_placement::Select,
        #[arg(long)]
        json: bool,
        #[command(subcommand)]
        action: Option<crate::commands::resources_placement::ConnectionAction>,
    },
    /// Disconnect your own Turso database (Owner, recent sign-in)
    Disconnect {
        #[command(flatten)]
        select: crate::commands::resources_placement::Select,
        /// Typed confirmation: "disconnect <database>"
        #[arg(long)]
        confirm: Option<String>,
        #[arg(long)]
        operation_id: Option<String>,
        #[arg(long)]
        wait: bool,
        #[arg(long)]
        json: bool,
    },
    /// Show where the database lives, its placement history, move receipts and consents
    Placement {
        #[command(flatten)]
        select: crate::commands::resources_placement::Select,
        #[arg(long)]
        json: bool,
    },
    /// Consent to one operator move to a dedicated pool or another region (Owner, recent sign-in; 7 days, used once)
    RelocationConsent {
        #[command(flatten)]
        select: crate::commands::resources_placement::Select,
        #[arg(long, value_parser = ["dedicated", "region"])]
        kind: String,
        #[arg(long)]
        to_region: Option<String>,
        #[arg(long)]
        acknowledge_costs: bool,
        /// "move <database> to <region id>" or "move <database> to dedicated"
        #[arg(long)]
        confirm: Option<String>,
        #[arg(long)]
        operation_id: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Move receipts
    Receipt {
        #[command(subcommand)]
        action: crate::commands::resources_placement::ReceiptAction,
    },
    /// Show restore options: the point-in-time window, daily backups, the kept previous
    /// generation, archive state, inactivity and recent moves
    Recovery {
        #[arg(long)]
        resource: String,
        #[arg(long)]
        json: bool,
    },
    /// List the database access grants of one database (Owner)
    ///
    /// Newest first: requirement, workload subject (or ID), state (Active, Expired or Revoked),
    /// expiry and grant ID.
    Grants {
        #[command(flatten)]
        select: crate::commands::resources_placement::Select,
        /// Print the grants as JSON, each with `state` and `workload_subject` added
        #[arg(long)]
        json: bool,
    },
    /// Grant a server workload credentials for one database requirement (Owner)
    ///
    /// The workload must be enabled and registered for database access in the database's
    /// environment; its compiled backend plan is used. Nothing is sent when the requirement,
    /// workload or expiry is refused. A workload holds at most one active grant per requirement.
    Grant {
        #[command(flatten)]
        select: crate::commands::resources_placement::Select,
        /// The `connectDatabase` requirement name in your app: a lowercase letter, then up to
        /// 63 letters or digits
        #[arg(long)]
        requirement: String,
        /// The workload ID or its Google workload subject (`axiom secrets workloads` lists them)
        #[arg(long)]
        workload: String,
        /// Days until the grant expires: 7, 30 or 90 (90 ends five minutes before the API's limit)
        #[arg(long, default_value_t = 30)]
        expires_days: u32,
        /// Exact expiry (RFC 3339), at most 90 days ahead. Pass the value an unconfirmed grant
        /// printed, with its --operation-id, to repeat it without creating a second grant
        #[arg(long, conflicts_with = "expires_days")]
        expires_at: Option<String>,
        /// Idempotency key (UUID); reuse it with the same details to repeat a lost request
        #[arg(long)]
        operation_id: Option<String>,
        /// Print the created grant as JSON
        #[arg(long)]
        json: bool,
    },
    /// Revoke a database access grant (Owner)
    ///
    /// New credentials are refused at once. A credential already held keeps working for the
    /// engine's bound: on SQLite until it expires, at most 15 minutes; on PostgreSQL open
    /// connections end within about 10 minutes.
    Revoke {
        /// The grant ID (`grants` lists them)
        grant_id: String,
        #[command(flatten)]
        select: crate::commands::resources_placement::Select,
        /// Idempotency key (UUID); reuse it to repeat a lost request
        #[arg(long)]
        operation_id: Option<String>,
        /// Print the revoked grant as JSON
        #[arg(long)]
        json: bool,
        /// Confirm without a prompt (required without a terminal)
        #[arg(long)]
        yes: bool,
    },
    /// Show currently configured databases and location admission
    Catalog,
}
pub async fn handle(project: &str, action: Option<&Action>) -> Result<()> {
    use crate::commands::resources_placement as placement;
    use crate::commands::resources_recovery as recovery;
    use crate::commands::resources_access as access;
    match action {
        Some(Action::Grants { select, json }) => return access::handle_grants(project, select, *json).await,
        Some(Action::Grant { select, requirement, workload, expires_days, expires_at, operation_id, json }) => {
            let input = access::GrantInput { requirement, workload, expires_days: *expires_days, expires_at: expires_at.as_deref() };
            return access::handle_grant(project, select, &input, operation_id.as_deref(), *json).await;
        }
        Some(Action::Revoke { grant_id, select, operation_id, json, yes }) => {
            return access::handle_revoke(project, select, grant_id, operation_id.as_deref(), *json, *yes).await;
        }
        Some(Action::Connect { provider, select, organization, group, database, token_stdin, fence_mode, acknowledge_split_store, acknowledge_rotate, reclaim, operation_id, json }) => {
            let input = placement::ConnectInput {
                provider: provider.clone(),
                organization: organization.clone(),
                group: group.clone(),
                database: database.clone(),
                fence_mode: fence_mode.clone(),
                acknowledge_split_store: *acknowledge_split_store,
                acknowledge_rotate: *acknowledge_rotate,
                reclaim: reclaim.clone(),
            };
            return placement::handle_connect(project, select, &input, *token_stdin, operation_id.as_deref(), *json).await;
        }
        Some(Action::Connection { select, json, action }) => return placement::handle_connection(project, select, action.as_ref(), *json).await,
        Some(Action::Disconnect { select, confirm, operation_id, wait, json }) => {
            return placement::handle_disconnect(project, select, confirm.as_deref(), operation_id.as_deref(), *wait, *json).await;
        }
        Some(Action::Placement { select, json }) => return placement::handle_placement(project, select, *json).await,
        Some(Action::RelocationConsent { select, kind, to_region, acknowledge_costs, confirm, operation_id, json }) => {
            return placement::handle_consent(project, select, kind, to_region.as_deref(), *acknowledge_costs, confirm.as_deref(), operation_id.as_deref(), *json).await;
        }
        Some(Action::Receipt { action }) => return placement::handle_receipt(project, action).await,
        Some(Action::Request { resource, action, generation, operation_id, confirm, wait, json, conversion, acknowledge_sessions_end, acknowledge_redeploy, acknowledge_semantics, acknowledge_copy, acknowledge_external_writers, .. }) if action == "convert" => {
            let acknowledge = placement::ConvertAcknowledge {
                sessions_end: *acknowledge_sessions_end,
                redeploy: *acknowledge_redeploy,
                semantics: *acknowledge_semantics,
                copy: acknowledge_copy.clone(),
                external_writers: acknowledge_external_writers.clone(),
            };
            let conversion = conversion.as_deref().ok_or_else(|| anyhow::anyhow!("--action convert needs --conversion <ID>"))?;
            return placement::handle_convert(project, resource, *generation, conversion, confirm.as_deref(), &acknowledge, operation_id.as_deref(), *wait, *json).await;
        }
        Some(Action::Recovery { resource, json }) => {
            return recovery::handle_view(project, resource, *json).await;
        }
        Some(Action::Request {
            resource,
            action,
            generation,
            operation_id,
            confirm,
            at,
            backup,
            wait,
            json,
            ..
        }) if recovery::is_recovery_action(action) => {
            let input = recovery::ActionInput {
                resource,
                action,
                generation: *generation,
                at: at.as_deref(),
                backup: backup.as_deref(),
                confirm: confirm.as_deref(),
            };
            return recovery::handle_action(project, &input, operation_id.as_deref(), *wait, *json)
                .await;
        }
        Some(Action::Request { at, backup, .. }) if at.is_some() || backup.is_some() => {
            bail!("--at and --backup are only used with --action restore");
        }
        Some(Action::Request { conversion: Some(_), .. }) => bail!("--conversion is only used with --action convert"),
        _ => {}
    }
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
        Some(Action::Grants { .. } | Action::Grant { .. } | Action::Revoke { .. } | Action::Recovery { .. } | Action::Connect { .. } | Action::Connection { .. } | Action::Disconnect { .. } | Action::Placement { .. } | Action::RelocationConsent { .. } | Action::Receipt { .. }) => unreachable!("handled above"),
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
            ..
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
