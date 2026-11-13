//! `axiom auth roles|permissions|assignments`: role-based access administration
//! (auth-core/v1 RBAC), the same routes as the dashboard's Roles page.
//!
//! Roles and permissions are addressed by name or UUID; assignments name a
//! user and an organization by UUID (as the dashboard does). Changes reach
//! each user's next access token; tokens already issued keep their authority
//! until they expire.
use super::advanced::confirm_yes;
use super::{checked_id, print, sanitize, target, text, usage, uuid_shaped, AuthArgs, Target};
use anyhow::Result;
use clap::Subcommand;
use reqwest::Method;
use serde_json::{json, Value};
use std::collections::HashMap;

/// Role names: a lowercase letter, then lowercase letters, digits, `_` and `-` (at most 64).
fn valid_role_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 64 && chars.next().is_some_and(|c| c.is_ascii_lowercase()) && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Permission names: a lowercase letter, then lowercase letters, digits, `_ . : -` (at most 64).
fn valid_permission_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 64 && chars.next().is_some_and(|c| c.is_ascii_lowercase()) && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_.:-".contains(c))
}

#[derive(Subcommand, Debug)]
pub enum RolesAction {
    /// List the roles and the permissions each one grants
    List,
    /// Create a role, optionally granting permissions right away
    Create {
        /// Role name: a lowercase letter, then lowercase letters, digits, _ and - (at most 64)
        name: String,
        /// What the role is for (at most 256 characters)
        #[arg(long, default_value = "")]
        description: String,
        /// Permission name or UUID the role grants; repeatable
        #[arg(long = "permission")]
        permissions: Vec<String>,
    },
    /// Change a role's description (the name cannot change)
    Update {
        /// Role name or UUID
        role: String,
        /// The new description (at most 256 characters; "" clears it)
        #[arg(long)]
        description: String,
    },
    /// Delete a role; refused while the role is assigned to anyone
    Delete {
        /// Role name or UUID
        role: String,
        /// Delete without asking for confirmation
        #[arg(long)]
        yes: bool,
    },
    /// Replace the full set of permissions a role grants
    SetPermissions {
        /// Role name or UUID
        role: String,
        /// Permission name or UUID; repeatable. The role grants exactly these afterwards
        #[arg(long = "permission", required_unless_present = "none")]
        permissions: Vec<String>,
        /// Remove every permission from the role
        #[arg(long, conflicts_with = "permissions")]
        none: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum PermissionsAction {
    /// List the permission keys roles can grant
    List,
    /// Create a permission key
    Create {
        /// Permission name, equal to the claim your guards check: a lowercase letter, then lowercase letters, digits and _ . : - (at most 64)
        name: String,
        /// What the permission allows (at most 256 characters)
        #[arg(long, default_value = "")]
        description: String,
    },
    /// Delete a permission; refused while any role grants it
    Delete {
        /// Permission name or UUID
        permission: String,
        /// Delete without asking for confirmation
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum AssignmentsAction {
    /// List the roles assigned to one user
    List {
        /// User ID (UUID; find it with `axiom auth users list`)
        #[arg(long)]
        user: String,
    },
    /// Assign a role to a user, environment-wide or within one organization
    Add {
        /// User ID (UUID)
        #[arg(long)]
        user: String,
        /// Role name or UUID
        #[arg(long)]
        role: String,
        /// Organization ID (UUID); without it the role applies environment-wide
        #[arg(long)]
        organization: Option<String>,
        /// When the assignment ends, an RFC 3339 time in the future (for example 2027-01-31T00:00:00Z)
        #[arg(long)]
        valid_until: Option<String>,
    },
    /// Remove an assignment by its ID
    Remove {
        /// Assignment ID (UUID; shown by `axiom auth assignments list`)
        id: String,
        /// Remove without asking for confirmation
        #[arg(long)]
        yes: bool,
    },
}

// ---------------------------------------------------------------------------
// Lookups
// ---------------------------------------------------------------------------

/// A lowercase UUID argument, or a usage error naming the flag.
fn uuid_arg(value: &str, what: &str) -> Result<String> {
    let lowered = value.trim().to_ascii_lowercase();
    checked_id(&lowered).map_err(|_| usage(format!("{what} `{value}` is not a UUID")))?;
    Ok(lowered)
}

/// The items of a list reply (`{"roles": [...]}`).
fn items(listed: &Value, key: &str) -> Vec<Value> {
    listed[key].as_array().cloned().unwrap_or_default()
}

/// The item whose UUID or name is `given`.
fn find<'a>(list: &'a [Value], given: &str, what: &str) -> Result<&'a Value> {
    let lowered = given.to_ascii_lowercase();
    list.iter()
        .find(|i| i["id"].as_str() == Some(lowered.as_str()) || i["name"].as_str() == Some(given))
        .ok_or_else(|| usage(format!("unknown {what} `{given}`")))
}

async fn all_roles(target: &Target) -> Result<Vec<Value>> {
    let listed = target.call(Method::GET, "roles", &[], None).await?;
    Ok(items(&listed, "roles"))
}

async fn all_permissions(target: &Target) -> Result<Vec<Value>> {
    let listed = target.call(Method::GET, "permissions", &[], None).await?;
    Ok(items(&listed, "permissions"))
}

/// A role's UUID, looked up by name only when `given` is not a UUID.
async fn role_id(target: &Target, given: &str) -> Result<String> {
    let lowered = given.to_ascii_lowercase();
    if uuid_shaped(&lowered) {
        return Ok(lowered);
    }
    let roles = all_roles(target).await?;
    Ok(text(find(&roles, given, "role")?, "id"))
}

/// Permission UUIDs in the given order without repeats; the list is read only when a name is given.
async fn permission_ids(target: &Target, given: &[String]) -> Result<Vec<String>> {
    let known = if given.iter().all(|g| uuid_shaped(&g.to_ascii_lowercase())) { Vec::new() } else { all_permissions(target).await? };
    let mut out: Vec<String> = Vec::new();
    for g in given {
        let lowered = g.to_ascii_lowercase();
        let id = if uuid_shaped(&lowered) { lowered } else { text(find(&known, g, "permission")?, "id") };
        if !out.contains(&id) {
            out.push(id);
        }
    }
    Ok(out)
}

/// `id -> name`, for readable output; empty when the lookup fails.
async fn names(target: &Target, rest: &str) -> HashMap<String, String> {
    match target.call(Method::GET, rest, &[], None).await {
        Ok(listed) => items(&listed, rest).iter().map(|i| (text(i, "id"), text(i, "name"))).collect(),
        Err(_) => HashMap::new(),
    }
}

fn description(value: &str) -> Result<&str> {
    let value = value.trim();
    if value.chars().count() > 256 {
        return Err(usage("the description is at most 256 characters"));
    }
    Ok(value)
}

/// `--valid-until`: RFC 3339, in the future, sent in UTC like the dashboard's `toISOString()`.
pub fn valid_until(value: &str) -> Result<String> {
    let at = chrono::DateTime::parse_from_rfc3339(value.trim()).map_err(|_| usage(format!("--valid-until `{value}` is not an RFC 3339 time such as 2027-01-31T00:00:00Z")))?;
    let at = at.with_timezone(&chrono::Utc);
    if at <= chrono::Utc::now() {
        return Err(usage("--valid-until must be in the future"));
    }
    Ok(at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

const BOUND_NOTE: &str = "Changes apply to each user's next access token; tokens already issued keep their roles until they expire.";

fn role_line(r: &Value, permission_names: &HashMap<String, String>) -> String {
    let granted = r["permission_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|id| permission_names.get(id).cloned().unwrap_or_else(|| id.to_string()))
        .collect::<Vec<_>>();
    let granted = if granted.is_empty() { "no permissions".to_string() } else { granted.join(", ") };
    let about = r["description"].as_str().filter(|d| !d.is_empty()).map(|d| format!("  \"{d}\"")).unwrap_or_default();
    format!("{}  {}  revision {}  {}{}", text(r, "id"), text(r, "name"), text(r, "revision"), granted, about)
}

fn permission_line(p: &Value) -> String {
    let about = p["description"].as_str().filter(|d| !d.is_empty()).map(|d| format!("  \"{d}\"")).unwrap_or_default();
    format!("{}  {}{}", text(p, "id"), text(p, "name"), about)
}

fn assignment_line(a: &Value, role_names: &HashMap<String, String>) -> String {
    let role_id = text(a, "role_id");
    let role = role_names.get(&role_id).cloned().unwrap_or(role_id);
    let scope = a["organization_id"].as_str().map(|o| format!("organization {o}")).unwrap_or_else(|| "environment-wide".into());
    let until = a["valid_until"].as_str().map(|u| format!("  until {u}")).unwrap_or_default();
    format!("{}  {}  {}{}", text(a, "id"), role, scope, until)
}

// ---------------------------------------------------------------------------
// Roles
// ---------------------------------------------------------------------------

pub async fn roles(args: &AuthArgs, action: &RolesAction) -> Result<()> {
    // Local checks first: no request is sent for a mistake.
    match action {
        RolesAction::Create { name, description: d, .. } => {
            if !valid_role_name(name) {
                return Err(usage("the role name starts with a lowercase letter and may use lowercase letters, digits, _ and - (at most 64)"));
            }
            description(d)?;
        }
        RolesAction::Update { description: d, .. } => {
            description(d)?;
        }
        _ => {}
    }
    let target = target(args).await?;
    roles_with(args, &target, action).await
}

pub(super) async fn roles_with(args: &AuthArgs, target: &Target, action: &RolesAction) -> Result<()> {
    match action {
        RolesAction::List => {
            let mut listed = target.call(Method::GET, "roles", &[], None).await?;
            sanitize(&mut listed, true);
            let permission_names = if args.json { HashMap::new() } else { names(target, "permissions").await };
            print(args, &listed, |l| {
                let lines = l["roles"].as_array().into_iter().flatten().map(|r| role_line(r, &permission_names)).collect::<Vec<_>>();
                if lines.is_empty() {
                    vec!["No roles.".into()]
                } else {
                    lines
                }
            })
        }
        RolesAction::Create { name, description: d, permissions } => {
            // Resolve permission names before creating anything.
            let wanted = permission_ids(target, permissions).await?;
            let body = json!({"name": name, "description": description(d)?});
            let mut role = target.call(Method::POST, "roles", &[], Some(&body)).await?;
            if !wanted.is_empty() {
                let id = checked_id(role["id"].as_str().unwrap_or_default())?.to_string();
                let body = json!({"permission_ids": wanted, "revision": role["revision"]});
                role = target.call(Method::PUT, &format!("roles/{id}/permissions"), &[], Some(&body)).await?;
            }
            sanitize(&mut role, true);
            print(args, &role, |r| vec![format!("Role {} created ({}).", text(r, "name"), text(r, "id"))])
        }
        RolesAction::Update { role, description: d } => {
            let roles = all_roles(target).await?;
            let current = find(&roles, role, "role")?;
            let id = checked_id(current["id"].as_str().unwrap_or_default())?;
            let body = json!({"description": description(d)?, "revision": current["revision"]});
            let mut saved = target.call(Method::PATCH, &format!("roles/{id}"), &[], Some(&body)).await?;
            sanitize(&mut saved, true);
            print(args, &saved, |r| vec![format!("Role {} updated (revision {}).", text(r, "name"), text(r, "revision"))])
        }
        RolesAction::Delete { role, yes } => {
            let id = role_id(target, role).await?;
            confirm_yes(*yes, &format!("Delete the role {role}?"))?;
            let deleted = target.call(Method::DELETE, &format!("roles/{}", checked_id(&id)?), &[], None).await?;
            print(args, &deleted, |_| vec![format!("Role {role} deleted.")])
        }
        RolesAction::SetPermissions { role, permissions, none } => {
            let wanted = if *none { Vec::new() } else { permission_ids(target, permissions).await? };
            let roles = all_roles(target).await?;
            let current = find(&roles, role, "role")?;
            let id = checked_id(current["id"].as_str().unwrap_or_default())?;
            let body = json!({"permission_ids": wanted, "revision": current["revision"]});
            let mut saved = target.call(Method::PUT, &format!("roles/{id}/permissions"), &[], Some(&body)).await?;
            sanitize(&mut saved, true);
            let count = saved["permission_ids"].as_array().map_or(0, Vec::len);
            print(args, &saved, |r| vec![format!("Role {} now grants {count} permission(s). {BOUND_NOTE}", text(r, "name"))])
        }
    }
}

// ---------------------------------------------------------------------------
// Permissions
// ---------------------------------------------------------------------------

pub async fn permissions(args: &AuthArgs, action: &PermissionsAction) -> Result<()> {
    if let PermissionsAction::Create { name, description: d } = action {
        if !valid_permission_name(name) {
            return Err(usage("the permission name starts with a lowercase letter and may use lowercase letters, digits and _ . : - (at most 64)"));
        }
        description(d)?;
    }
    let target = target(args).await?;
    permissions_with(args, &target, action).await
}

pub(super) async fn permissions_with(args: &AuthArgs, target: &Target, action: &PermissionsAction) -> Result<()> {
    match action {
        PermissionsAction::List => {
            let mut listed = target.call(Method::GET, "permissions", &[], None).await?;
            sanitize(&mut listed, true);
            print(args, &listed, |l| {
                let lines = l["permissions"].as_array().into_iter().flatten().map(permission_line).collect::<Vec<_>>();
                if lines.is_empty() {
                    vec!["No permissions.".into()]
                } else {
                    lines
                }
            })
        }
        PermissionsAction::Create { name, description: d } => {
            let body = json!({"name": name, "description": description(d)?});
            let mut created = target.call(Method::POST, "permissions", &[], Some(&body)).await?;
            sanitize(&mut created, true);
            print(args, &created, |p| vec![format!("Permission {} created ({}).", text(p, "name"), text(p, "id"))])
        }
        PermissionsAction::Delete { permission, yes } => {
            let id = permission_ids(target, std::slice::from_ref(permission)).await?.remove(0);
            confirm_yes(*yes, &format!("Delete the permission {permission}?"))?;
            let deleted = target.call(Method::DELETE, &format!("permissions/{}", checked_id(&id)?), &[], None).await?;
            print(args, &deleted, |_| vec![format!("Permission {permission} deleted.")])
        }
    }
}

// ---------------------------------------------------------------------------
// Assignments
// ---------------------------------------------------------------------------

pub async fn assignments(args: &AuthArgs, action: &AssignmentsAction) -> Result<()> {
    match action {
        AssignmentsAction::List { user } => {
            uuid_arg(user, "--user")?;
        }
        AssignmentsAction::Add { user, organization, valid_until: until, .. } => {
            uuid_arg(user, "--user")?;
            if let Some(organization) = organization {
                uuid_arg(organization, "--organization")?;
            }
            if let Some(until) = until {
                valid_until(until)?;
            }
        }
        AssignmentsAction::Remove { id, .. } => {
            uuid_arg(id, "assignment ID")?;
        }
    }
    let target = target(args).await?;
    assignments_with(args, &target, action).await
}

pub(super) async fn assignments_with(args: &AuthArgs, target: &Target, action: &AssignmentsAction) -> Result<()> {
    match action {
        AssignmentsAction::List { user } => {
            let query = [("user_id", uuid_arg(user, "--user")?)];
            let mut listed = target.call(Method::GET, "assignments", &query, None).await?;
            sanitize(&mut listed, false);
            let role_names = if args.json { HashMap::new() } else { names(target, "roles").await };
            print(args, &listed, |l| {
                let lines = l["assignments"].as_array().into_iter().flatten().map(|a| assignment_line(a, &role_names)).collect::<Vec<_>>();
                if lines.is_empty() {
                    vec!["No roles are assigned to this user.".into()]
                } else {
                    lines
                }
            })
        }
        AssignmentsAction::Add { user, role, organization, valid_until: until } => {
            let mut body = json!({"user_id": uuid_arg(user, "--user")?, "role_id": role_id(target, role).await?});
            if let Some(organization) = organization {
                body["organization_id"] = json!(uuid_arg(organization, "--organization")?);
            }
            if let Some(until) = until {
                body["valid_until"] = json!(valid_until(until)?);
            }
            let mut created = target.call(Method::POST, "assignments", &[], Some(&body)).await?;
            sanitize(&mut created, false);
            print(args, &created, |a| vec![format!("Role {role} assigned (assignment {}). {BOUND_NOTE}", text(a, "id"))])
        }
        AssignmentsAction::Remove { id, yes } => {
            let id = uuid_arg(id, "assignment ID")?;
            confirm_yes(*yes, &format!("Remove the assignment {id}?"))?;
            let removed = target.call(Method::DELETE, &format!("assignments/{}", checked_id(&id)?), &[], None).await?;
            print(args, &removed, |_| vec![format!("Assignment {id} removed. {BOUND_NOTE}")])
        }
    }
}

#[cfg(test)]
mod tests;
