//! `security.roles`, `security.viewAs`: the model's row-level security
//! roles and "View as" (Power BI Desktop: Modeling → View as).
//!
//! "View as" is view state of the open document, never written to a file:
//! while it is active every query of the document (measures, DAX, grid
//! pages, statistics, exports, the canvas) sees only the rows the roles
//! allow, as the given user (`pbi_session::Session::set_view_as`).
//! Applying or clearing it bumps the document's data generation; the canvas
//! re-queries because its cached results are keyed by the engine snapshot,
//! which the view replaces.

use pbi_model::{PropValue, Role};
use pbi_session::{Session, ViewAs};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{BASE, DocParams, doc};
use crate::command::{Category, Command, CommandMeta};
use crate::ctx::Ctx;
use crate::error::OpError;
use crate::inspect_model::TablePermissionOut;
use crate::resolve;
use crate::workspace::Workspace;

/// Longest `userName` / `customData` (characters).
const MAX_IDENTITY_CHARS: usize = 1024;

/// DAX functions that make a role filter depend on the effective user.
const IDENTITY_FUNCTIONS: [&str; 4] = [
    "USERPRINCIPALNAME",
    "USERNAME",
    "USEROBJECTID",
    "CUSTOMDATA",
];

/// A security role of the model.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecurityRoleOut {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `read`, `readRefresh`, `administrator`, `none`, … (absent when the
    /// model does not say).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_permission: Option<String>,
    /// Table permissions with their DAX row filters (rows for which the
    /// filter is TRUE are visible), in model order.
    pub table_permissions: Vec<TablePermissionOut>,
    /// Members listed in the model (absent when it lists none: members are
    /// usually assigned in the Power BI service).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub members: Option<usize>,
    /// A filter calls `USERPRINCIPALNAME()`, `USERNAME()`, `USEROBJECTID()`
    /// or `CUSTOMDATA()`: view it with a `userName` (Desktop's "Other
    /// user").
    pub dynamic: bool,
}

/// The "View as" state of a document.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ViewAsOut {
    /// A view is applied: queries see only the rows `roles` allow, as
    /// `userName`.
    pub active: bool,
    /// The roles applied (model spelling); empty when only a user is set.
    pub roles: Vec<String>,
    /// What `USERPRINCIPALNAME()` and `USERNAME()` return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
    /// What `CUSTOMDATA()` returns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<String>,
    /// Why the view shows no rows at all: its role filters could not be
    /// re-applied after a refresh or a derived column change (it fails
    /// closed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ViewAsOut {
    fn of(session: &Session) -> Self {
        match session.view_as() {
            Some(view) => Self {
                active: true,
                roles: view.roles,
                user_name: view.user_name,
                custom_data: view.custom_data,
                error: session.view_as_error(),
            },
            None => Self::default(),
        }
    }
}

fn member_count(role: &Role) -> Option<usize> {
    let mut found = false;
    let mut count = 0;
    for (key, value) in &role.extra {
        if !key.eq_ignore_ascii_case("member") && !key.eq_ignore_ascii_case("members") {
            continue;
        }
        found = true;
        count += match value {
            PropValue::List(items) => items.len(),
            PropValue::Json(serde_json::Value::Array(items)) => items.len(),
            _ => 1,
        };
    }
    found.then_some(count)
}

fn calls_identity(expression: &str) -> bool {
    let upper = expression.to_ascii_uppercase();
    IDENTITY_FUNCTIONS.iter().any(|f| {
        upper.match_indices(f).any(|(at, _)| {
            let before = upper.get(..at).and_then(|s| s.chars().next_back());
            let after = upper.get(at + f.len()..).unwrap_or_default();
            !before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
                && after.trim_start().starts_with('(')
        })
    })
}

fn role_out(role: &Role) -> SecurityRoleOut {
    SecurityRoleOut {
        name: role.name.clone(),
        description: role.description.clone().filter(|d| !d.trim().is_empty()),
        model_permission: role.model_permission.clone(),
        table_permissions: role
            .table_permissions
            .iter()
            .map(|p| TablePermissionOut {
                table: p.table.clone(),
                filter_expression: p.filter_expression.clone(),
            })
            .collect(),
        members: member_count(role),
        dynamic: role
            .table_permissions
            .iter()
            .filter_map(|p| p.filter_expression.as_deref())
            .any(calls_identity),
    }
}

// ---------------------------------------------------------------------------

/// `security.roles`.
#[derive(Debug)]
pub struct SecurityRoles;

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecurityRolesOut {
    /// The model's roles, in model order (empty without row-level
    /// security).
    pub roles: Vec<SecurityRoleOut>,
    /// The current "View as" state.
    pub view_as: ViewAsOut,
}

impl Command for SecurityRoles {
    type Params = DocParams;
    type Output = SecurityRolesOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "security.roles",
            title: "Security roles",
            summary: "The model's row-level security roles (permission, DAX table filters, members, dynamic or not) and the current \"View as\" state.",
            category: Category::Inspect,
            examples: &[("Roles of d1", r#"{"docId": "d1"}"#)],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &p.doc_id)?;
        let session = entry.session();
        Ok(SecurityRolesOut {
            roles: session.base_model().roles.iter().map(role_out).collect(),
            view_as: ViewAsOut::of(session),
        })
    }
}

// ---------------------------------------------------------------------------

/// `security.viewAs`.
#[derive(Debug)]
pub struct SecurityViewAs;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecurityViewAsParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Roles whose row filters apply (role names, case-insensitive); a row
    /// is visible if any role allows it. Empty with no `userName` and no
    /// `customData`: stop viewing as (all rows again).
    pub roles: Vec<String>,
    /// The effective user ("Other user"): what `USERPRINCIPALNAME()` and
    /// `USERNAME()` return, for dynamic roles. Without roles it only sets
    /// the user.
    #[serde(default)]
    pub user_name: Option<String>,
    /// What `CUSTOMDATA()` returns.
    #[serde(default)]
    pub custom_data: Option<String>,
}

/// Rows of one table under the view.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VisibleTableOut {
    pub name: String,
    /// Rows visible under the view (all rows when no view is active).
    pub rows: usize,
    /// Rows loaded by the last refresh or CSV load, before row-level
    /// security (absent before the first load).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_rows: Option<usize>,
    /// Data has been loaded.
    pub loaded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecurityViewAsOut {
    /// The view now in effect.
    pub view_as: ViewAsOut,
    /// Visible rows per table, in model order.
    pub tables: Vec<VisibleTableOut>,
    /// Data generation after the change (bumped when the view changed:
    /// cached results and renders are stale).
    pub generation: u64,
}

/// `None` for an absent or blank value; `invalidParams` beyond
/// [`MAX_IDENTITY_CHARS`].
fn identity(name: &str, value: Option<String>) -> Result<Option<String>, OpError> {
    let Some(value) = value.as_deref().map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    if value.chars().count() > MAX_IDENTITY_CHARS {
        return Err(OpError::invalid_params(format!(
            "{name} is longer than {MAX_IDENTITY_CHARS} characters"
        )));
    }
    Ok(Some(value.to_owned()))
}

/// The model spelling of `requested`, without duplicates.
fn resolve_roles(roles: &[Role], requested: &[String]) -> Result<Vec<String>, OpError> {
    let mut out: Vec<String> = Vec::with_capacity(requested.len());
    for name in requested {
        let role = roles
            .iter()
            .find(|r| pbi_model::names_eq(&r.name, name.trim()))
            .ok_or_else(|| {
                let names: Vec<String> = roles.iter().map(|r| r.name.clone()).collect();
                let hint = if names.is_empty() {
                    "the model has no roles".to_owned()
                } else {
                    format!("roles: {}", names.join(", "))
                };
                OpError::not_found(format!("no role named '{name}' ({hint})"))
                    .with_data(json!({ "roles": names }))
            })?;
        if !out.contains(&role.name) {
            out.push(role.name.clone());
        }
    }
    Ok(out)
}

impl Command for SecurityViewAs {
    type Params = SecurityViewAsParams;
    type Output = SecurityViewAsOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "security.viewAs",
            title: "View as roles",
            summary: "Views the data as security roles and/or a user (row-level security, union of roles); empty roles and no user stop it. Visible rows per table.",
            category: Category::Data,
            mutates_document: true,
            examples: &[
                ("One role", r#"{"docId": "d1", "roles": ["East"]}"#),
                (
                    "A dynamic role as a user",
                    r#"{"docId": "d1", "roles": ["Managers"], "userName": "ann@contoso.com"}"#,
                ),
                ("Stop viewing as", r#"{"docId": "d1", "roles": []}"#),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &p.doc_id)?;
        resolve::require_model(&entry)?;
        let roles = resolve_roles(&entry.session().base_model().roles, &p.roles)?;
        let user_name = identity("userName", p.user_name)?;
        let custom_data = identity("customData", p.custom_data)?;
        // Role filters are evaluated on the data: load it first (autoRefresh).
        entry.ensure_loaded(ctx).await?;
        let session = entry.session();
        let before = session.view_as();
        if roles.is_empty() && user_name.is_none() && custom_data.is_none() {
            session.clear_view_as();
            if before.is_some() {
                entry.bump_generation();
            }
        } else {
            ctx.progress(
                if roles.is_empty() {
                    "setting the effective user".to_owned()
                } else {
                    format!("applying the row filters of {}", roles.join(", "))
                },
                None,
            );
            let view = ViewAs {
                roles,
                user_name,
                custom_data,
                ..ViewAs::default()
            };
            session
                .set_view_as(view, ctx.cancel_token().child_token())
                .await?;
            entry.bump_generation();
        }
        let tables = session
            .tables()
            .into_iter()
            .map(|t| VisibleTableOut {
                rows: t.rows,
                total_rows: t.last_refresh.as_ref().map(|r| r.rows),
                loaded: t.loaded,
                name: t.name,
            })
            .collect();
        Ok(SecurityViewAsOut {
            view_as: ViewAsOut::of(session),
            tables,
            generation: entry.generation(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_functions_are_detected_as_calls() {
        assert!(calls_identity("[Email] = USERPRINCIPALNAME()"));
        assert!(calls_identity("[Key] = customdata ( )"));
        assert!(calls_identity("PATHCONTAINS([Path], USERNAME())"));
        assert!(!calls_identity("[Region] = \"East\""));
        assert!(!calls_identity("[MyUSERNAME] = 1"));
        assert!(!calls_identity("[USERNAME] = 1"));
    }
}
