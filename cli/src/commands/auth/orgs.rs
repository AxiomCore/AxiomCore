//! `axiom auth organizations|invitations`: organizations, their members, and invitations.
//!
//! A membership carries only a status (`active` or `suspended`); roles within an organization
//! are role assignments (`axiom auth assignments`). An organization's revision is the
//! environment's policy revision, read from the organization list before a change (as the
//! dashboard does). Sending a new invitation is `axiom auth invite`.
use super::advanced::confirm_yes;
use super::{checked_id, print, sanitize, target, text, usage, uuid_shaped, AuthArgs, Exit, Target};
use anyhow::Result;
use clap::Subcommand;
use reqwest::Method;
use serde_json::{json, Value};

const STATUSES: [&str; 2] = ["active", "suspended"];
const INVITATION_STATUSES: [&str; 3] = ["pending", "accepted", "revoked"];

#[derive(Subcommand, Debug)]
pub enum OrganizationsAction {
    /// List organizations, ordered by slug
    List {
        /// Page size, 1 to 100 (default 50)
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: Option<u32>,
        /// Continue from the cursor a previous page printed
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Create an active organization
    Create {
        /// Unique slug: lowercase letters, digits and "-", at most 63, starting with a letter or digit
        #[arg(long)]
        slug: String,
        /// Display name, 1 to 128 bytes
        #[arg(long)]
        name: String,
    },
    /// Suspend or reactivate an organization; while suspended, roles assigned within it do not count
    Update {
        /// Organization UUID or slug
        organization: String,
        /// The new status
        #[arg(long, value_parser = STATUSES)]
        status: String,
        /// Suspend without the confirmation prompt
        #[arg(long)]
        yes: bool,
    },
    /// List an organization's members (user IDs and membership status)
    Members {
        /// Organization UUID or slug
        organization: String,
        /// Page size, 1 to 100 (default 50)
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: Option<u32>,
        /// Continue from the cursor a previous page printed
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Add a user to an organization, or change a member's status (a user belongs to at most 64)
    #[command(visible_alias = "update-member")]
    AddMember {
        /// Organization UUID or slug
        organization: String,
        /// User UUID
        user: String,
        /// Membership status; a suspended member's roles in the organization do not count
        #[arg(long, value_parser = STATUSES, default_value = "active")]
        status: String,
    },
    /// Remove a user from an organization
    RemoveMember {
        /// Organization UUID or slug
        organization: String,
        /// User UUID
        user: String,
        /// Remove without the confirmation prompt
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum InvitationsAction {
    /// List invitations, newest first
    List {
        /// Only invitations with this status
        #[arg(long, value_parser = INVITATION_STATUSES)]
        status: Option<String>,
        /// Page size, 1 to 100 (default 50)
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: Option<u32>,
        /// Continue from the cursor a previous page printed
        #[arg(long)]
        cursor: Option<String>,
        /// Show full email addresses instead of masked ones
        #[arg(long)]
        reveal_email: bool,
    },
    /// Email a pending invitation again with a new link; the earlier link stops working (once a minute)
    Resend {
        /// Invitation UUID
        id: String,
        /// Show the full email address instead of a masked one
        #[arg(long)]
        reveal_email: bool,
    },
    /// Revoke a pending invitation; its link stops working
    Revoke {
        /// Invitation UUID
        id: String,
        /// Revoke without the confirmation prompt
        #[arg(long)]
        yes: bool,
        /// Show the full email address instead of a masked one
        #[arg(long)]
        reveal_email: bool,
    },
}

// ---------------------------------------------------------------------------
// Local checks
// ---------------------------------------------------------------------------

/// The dashboard's organization slug rule (`^[a-z0-9][a-z0-9-]{0,62}$`).
pub fn valid_slug(slug: &str) -> bool {
    let bytes = slug.as_bytes();
    (1..=63).contains(&bytes.len())
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes.iter().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

/// The `POST organizations` body; slug and name are trimmed, as in the dashboard.
pub fn create_body(slug: &str, name: &str) -> Result<Value> {
    let (slug, name) = (slug.trim(), name.trim());
    if !valid_slug(slug) {
        return Err(usage("--slug may use lowercase letters, digits and \"-\" (at most 63, starting with a letter or digit)"));
    }
    if name.is_empty() || name.len() > 128 {
        return Err(usage("--name is 1 to 128 bytes"));
    }
    Ok(json!({"slug": slug, "name": name}))
}

/// An organization named by UUID or slug.
#[derive(Debug, PartialEq)]
pub enum OrgRef<'a> {
    Id(&'a str),
    Slug(&'a str),
}

pub fn org_ref(given: &str) -> Result<OrgRef<'_>> {
    if uuid_shaped(given) {
        Ok(OrgRef::Id(given))
    } else if valid_slug(given) {
        Ok(OrgRef::Slug(given))
    } else {
        Err(usage(format!("`{given}` is neither an organization UUID (lowercase) nor a slug")))
    }
}

fn valid_cursor(cursor: &str) -> bool {
    (1..=512).contains(&cursor.len()) && cursor.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// `limit` and `cursor` query parameters, checked like the dashboard client.
pub fn page_query(limit: Option<u32>, cursor: Option<&str>) -> Result<Vec<(&'static str, String)>> {
    let mut query = vec![];
    if let Some(limit) = limit {
        query.push(("limit", limit.to_string()));
    }
    if let Some(cursor) = cursor {
        if !valid_cursor(cursor) {
            return Err(usage("--cursor is the value a previous page printed"));
        }
        query.push(("cursor", cursor.to_string()));
    }
    Ok(query)
}

fn check_organizations(action: &OrganizationsAction) -> Result<()> {
    match action {
        OrganizationsAction::List { cursor, .. } => page_query(None, cursor.as_deref()).map(drop),
        OrganizationsAction::Create { slug, name } => create_body(slug, name).map(drop),
        OrganizationsAction::Update { organization, .. } => org_ref(organization).map(drop),
        OrganizationsAction::Members { organization, cursor, .. } => {
            org_ref(organization)?;
            page_query(None, cursor.as_deref()).map(drop)
        }
        OrganizationsAction::AddMember { organization, user, .. } | OrganizationsAction::RemoveMember { organization, user, .. } => {
            org_ref(organization)?;
            checked_id(user).map(drop)
        }
    }
}

fn check_invitations(action: &InvitationsAction) -> Result<()> {
    match action {
        InvitationsAction::List { cursor, .. } => page_query(None, cursor.as_deref()).map(drop),
        InvitationsAction::Resend { id, .. } | InvitationsAction::Revoke { id, .. } => checked_id(id).map(drop),
    }
}

// ---------------------------------------------------------------------------
// Organizations
// ---------------------------------------------------------------------------

fn org_line(o: &Value) -> String {
    format!("{}  {}  {}  {}", text(o, "id"), text(o, "slug"), text(o, "name"), text(o, "status"))
}

fn more(lines: &mut Vec<String>, page: &Value, empty: &str) {
    if lines.is_empty() {
        lines.push(empty.to_string());
    }
    if let Some(next) = page["next_cursor"].as_str() {
        lines.push(format!("More: --cursor {next}"));
    }
}

/// Finds an organization by UUID or slug in the organization list (at most 10,000, 100 a page).
async fn find_organization(target: &Target, given: &OrgRef<'_>) -> Result<Value> {
    let (key, wanted) = match given {
        OrgRef::Id(id) => ("id", *id),
        OrgRef::Slug(slug) => ("slug", *slug),
    };
    let mut cursor: Option<String> = None;
    for _ in 0..101 {
        let query = page_query(Some(100), cursor.as_deref())?;
        let page = target.call(Method::GET, "organizations", &query, None).await?;
        if let Some(found) = page["organizations"].as_array().into_iter().flatten().find(|o| o[key].as_str() == Some(wanted)) {
            return Ok(found.clone());
        }
        match page["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_string()),
            None => break,
        }
    }
    Err(usage(format!("unknown organization `{wanted}`")))
}

/// The organization's UUID: as given, or looked up by slug.
async fn organization_id(target: &Target, given: &str) -> Result<String> {
    match org_ref(given)? {
        OrgRef::Id(id) => Ok(id.to_string()),
        slug => {
            let found = find_organization(target, &slug).await?;
            found["id"].as_str().filter(|id| uuid_shaped(id)).map(str::to_string).ok_or_else(|| Exit { code: 5, message: "AXAUTH_UNAVAILABLE: the organization list carried no id".into() }.into())
        }
    }
}

pub async fn organizations(args: &AuthArgs, action: &OrganizationsAction) -> Result<()> {
    check_organizations(action)?;
    let target = target(args).await?;
    organizations_with(args, &target, action).await
}

pub(super) async fn organizations_with(args: &AuthArgs, target: &Target, action: &OrganizationsAction) -> Result<()> {
    check_organizations(action)?;
    match action {
        OrganizationsAction::List { limit, cursor } => {
            let query = page_query(*limit, cursor.as_deref())?;
            let mut listed = target.call(Method::GET, "organizations", &query, None).await?;
            sanitize(&mut listed, true);
            print(args, &listed, |l| {
                let mut lines = l["organizations"].as_array().into_iter().flatten().map(org_line).collect::<Vec<_>>();
                more(&mut lines, l, "No organizations.");
                lines
            })
        }
        OrganizationsAction::Create { slug, name } => {
            let body = create_body(slug, name)?;
            let mut created = target.call(Method::POST, "organizations", &[], Some(&body)).await?;
            sanitize(&mut created, true);
            print(args, &created, |o| vec![format!("Created {}", org_line(o))])
        }
        OrganizationsAction::Update { organization, status, yes } => {
            let given = org_ref(organization)?;
            if status == "suspended" {
                confirm_yes(*yes, &format!("Suspend organization {organization}? Roles assigned within it stop counting"))?;
            }
            let current = find_organization(target, &given).await?;
            let id = current["id"].as_str().filter(|id| uuid_shaped(id)).map(str::to_string);
            let (Some(id), true) = (id, current["revision"].is_u64()) else {
                return Err(Exit { code: 5, message: "AXAUTH_UNAVAILABLE: the organization list carried no id or revision".into() }.into());
            };
            let body = json!({"status": status, "revision": current["revision"]});
            let mut saved = target.call(Method::PATCH, &format!("organizations/{id}"), &[], Some(&body)).await?;
            sanitize(&mut saved, true);
            print(args, &saved, |o| vec![format!("Organization {} is {}.", text(o, "slug"), text(o, "status"))])
        }
        OrganizationsAction::Members { organization, limit, cursor } => {
            let id = organization_id(target, organization).await?;
            let query = page_query(*limit, cursor.as_deref())?;
            let mut listed = target.call(Method::GET, &format!("organizations/{id}/members"), &query, None).await?;
            sanitize(&mut listed, true);
            print(args, &listed, |l| {
                let mut lines = l["members"].as_array().into_iter().flatten().map(|m| format!("{}  {}", text(m, "user_id"), text(m, "status"))).collect::<Vec<_>>();
                more(&mut lines, l, "No members.");
                lines
            })
        }
        OrganizationsAction::AddMember { organization, user, status } => {
            let user = checked_id(user)?;
            let id = organization_id(target, organization).await?;
            let mut saved = target.call(Method::PUT, &format!("organizations/{id}/members/{user}"), &[], Some(&json!({"status": status}))).await?;
            sanitize(&mut saved, true);
            print(args, &saved, |m| vec![format!("User {} is a {} member of {organization}.", text(m, "user_id"), text(m, "status"))])
        }
        OrganizationsAction::RemoveMember { organization, user, yes } => {
            let user = checked_id(user)?;
            confirm_yes(*yes, &format!("Remove user {user} from organization {organization}?"))?;
            let id = organization_id(target, organization).await?;
            target.call(Method::DELETE, &format!("organizations/{id}/members/{user}"), &[], None).await?;
            print(args, &json!({"deleted": true, "organization_id": id, "user_id": user}), |_| vec![format!("Removed user {user} from {organization}.")])
        }
    }
}

// ---------------------------------------------------------------------------
// Invitations
// ---------------------------------------------------------------------------

fn invitation_line(i: &Value) -> String {
    let org = i["organization_id"].as_str().map(|o| format!("  organization {o}")).unwrap_or_default();
    format!("{}  {}  {}  delivery {}  expires {}{org}", text(i, "id"), text(i, "email"), text(i, "status"), text(i, "delivery_state"), text(i, "expires_at"))
}

pub async fn invitations(args: &AuthArgs, action: &InvitationsAction) -> Result<()> {
    check_invitations(action)?;
    let target = target(args).await?;
    invitations_with(args, &target, action).await
}

pub(super) async fn invitations_with(args: &AuthArgs, target: &Target, action: &InvitationsAction) -> Result<()> {
    check_invitations(action)?;
    match action {
        InvitationsAction::List { status, limit, cursor, reveal_email } => {
            let mut query = vec![];
            if let Some(status) = status {
                query.push(("status", status.clone()));
            }
            query.extend(page_query(*limit, cursor.as_deref())?);
            let mut listed = target.call(Method::GET, "invitations", &query, None).await?;
            sanitize(&mut listed, *reveal_email);
            print(args, &listed, |l| {
                let mut lines = l["invitations"].as_array().into_iter().flatten().map(invitation_line).collect::<Vec<_>>();
                more(&mut lines, l, "No invitations.");
                lines
            })
        }
        InvitationsAction::Resend { id, reveal_email } => {
            let id = checked_id(id)?;
            let mut sent = target.call(Method::POST, &format!("invitations/{id}/resend"), &[], Some(&json!({}))).await?;
            sanitize(&mut sent, *reveal_email);
            print(args, &sent, |i| {
                let i = i.get("invitation").unwrap_or(i);
                vec![format!("Invitation {} sent again to {} ({}); the earlier link no longer works. Expires {}.", text(i, "id"), text(i, "email"), text(i, "delivery_state"), text(i, "expires_at"))]
            })
        }
        InvitationsAction::Revoke { id, yes, reveal_email } => {
            let id = checked_id(id)?;
            confirm_yes(*yes, &format!("Revoke invitation {id}? Its link stops working"))?;
            let mut revoked = target.call(Method::POST, &format!("invitations/{id}/revoke"), &[], Some(&json!({}))).await?;
            sanitize(&mut revoked, *reveal_email);
            print(args, &revoked, |i| {
                let i = i.get("invitation").unwrap_or(i);
                vec![format!("Invitation {} for {} is {}.", text(i, "id"), text(i, "email"), text(i, "status"))]
            })
        }
    }
}

#[cfg(test)]
mod tests;
