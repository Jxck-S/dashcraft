//! Row-level security "View as": roles whose filters restrict the data and
//! the effective user (Power BI Desktop: Modeling → View as).
//!
//! [`Session::set_view_as`] evaluates each selected role's table filter
//! expressions (DAX, row by row, on the unrestricted data) and builds a
//! restricted copy of the engine ([`pbi_engine::Engine::with_row_security`]:
//! filters propagate along relationships in their security direction; the
//! rows a user sees are the union over the roles). From then on every query
//! of the session — measures, DAX queries, grid pages, SQL, exports and the
//! report visuals that read [`Session::snapshot`] — sees only those rows,
//! and `USERPRINCIPALNAME()`, `USERNAME()`, `USEROBJECTID()` and
//! `CUSTOMDATA()` return the configured user. Refreshes, CSV loads and
//! derived columns work on the unrestricted data and re-apply the view.
//! If a role filter cannot be evaluated after such a change, the view fails
//! closed (every table empty) and [`Session::view_as_error`] says why.

use std::sync::Arc;

use pbi_dax::EvalContext;
use pbi_engine::{Engine, RoleFilter, SecurityContext, SecurityRole, UserIdentity};
use pbi_model::{Diagnostic, Model};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::error::SessionError;
use crate::refresh::runtime;
use crate::session::Session;

/// Diagnostic code of a view whose role filters failed.
pub const VIEW_AS_FAILED: &str = "RLS001";

/// "View as" settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ViewAs {
    /// Roles whose row filters apply (model role names). Empty: no
    /// row-level security, only the effective user.
    pub roles: Vec<String>,
    /// `USERPRINCIPALNAME()` and `USERNAME()`; empty text when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
    /// `USEROBJECTID()`; empty text when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_object_id: Option<String>,
    /// `CUSTOMDATA()`; BLANK when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<String>,
}

impl ViewAs {
    /// Roles plus an optional user name.
    pub fn roles<S: AsRef<str>>(roles: &[S], user_name: Option<String>) -> Self {
        Self {
            roles: roles.iter().map(|r| r.as_ref().to_owned()).collect(),
            user_name,
            ..Self::default()
        }
    }

    fn identity(&self) -> UserIdentity {
        UserIdentity {
            user_name: self.user_name.clone(),
            user_object_id: self.user_object_id.clone(),
            custom_data: self.custom_data.clone(),
        }
    }
}

/// An active view: its settings and the restricted engine built for one
/// generation of the session's (unrestricted) engine.
#[derive(Debug)]
pub(crate) struct ActiveView {
    pub view: ViewAs,
    pub engine: Engine,
    pub base_generation: u64,
    pub error: Option<String>,
}

impl ActiveView {
    /// The view for `base`; a failure fails closed (no rows) with the
    /// error kept.
    pub fn build_or_deny(base: &Engine, model: &Model, view: ViewAs) -> ActiveView {
        let base_generation = base.generation();
        match secure(base, model, &view, None) {
            Ok(engine) => ActiveView {
                view,
                engine,
                base_generation,
                error: None,
            },
            Err(e) => ActiveView {
                engine: deny_all(base, &view),
                view,
                base_generation,
                error: Some(e.to_string()),
            },
        }
    }

    pub fn diagnostic(&self) -> Option<Diagnostic> {
        self.error.as_ref().map(|e| {
            Diagnostic::error(
                VIEW_AS_FAILED,
                format!(
                    "View as {}: {e}; every table is shown empty",
                    self.view.roles.join(", ")
                ),
            )
        })
    }
}

/// `base` with every table empty (a view whose filters cannot be applied).
fn deny_all(base: &Engine, view: &ViewAs) -> Engine {
    let mut engine = base.clone();
    engine.set_security_context(Some(SecurityContext {
        roles: view.roles.clone(),
        identity: view.identity(),
    }));
    engine.with_row_security(&[]).unwrap_or(engine)
}

/// Builds the restricted engine of `view` from the unrestricted `base`.
fn secure(
    base: &Engine,
    model: &Model,
    view: &ViewAs,
    cancel: Option<&CancellationToken>,
) -> Result<Engine, SessionError> {
    let mut engine = base.clone();
    engine.set_security_context(Some(SecurityContext {
        roles: view.roles.clone(),
        identity: view.identity(),
    }));
    if view.roles.is_empty() {
        return Ok(engine);
    }
    let model = Arc::new(model.clone());
    let mut ctx = EvalContext::new(engine.snapshot(), Arc::clone(&model));
    if let Some(token) = cancel {
        ctx = ctx.with_cancellation_token(token.clone());
    }
    let mut roles = Vec::with_capacity(view.roles.len());
    for name in &view.roles {
        let role = model
            .role(name)
            .ok_or_else(|| SessionError::UnknownRole(name.clone()))?;
        let mut filters = Vec::new();
        for permission in &role.table_permissions {
            let Some(expression) = permission
                .filter_expression
                .as_deref()
                .filter(|e| !e.trim().is_empty())
            else {
                continue;
            };
            let fail = |message: String| SessionError::RoleFilter {
                role: role.name.clone(),
                table: permission.table.clone(),
                message,
            };
            if engine.table(&permission.table).is_err() {
                return Err(fail("the table is not loaded".to_owned()));
            }
            let (rows, blank_row) = ctx
                .evaluate_row_filter(&permission.table, expression)
                .map_err(|e| match e {
                    pbi_dax::DaxError::Cancelled => SessionError::Cancelled,
                    other => fail(other.to_string()),
                })?;
            filters.push(RoleFilter {
                table: permission.table.clone(),
                rows,
                blank_row,
            });
        }
        roles.push(SecurityRole {
            name: role.name.clone(),
            filters,
        });
    }
    Ok(engine.with_row_security(&roles)?)
}

impl Session {
    /// Applies "View as": the rows every query sees are restricted by the
    /// row filters of `view.roles` (union over the roles; filters propagate
    /// along relationships in their security direction), and
    /// `USERPRINCIPALNAME()` & co. return `view`'s user. Replaces any
    /// previous view. Roles are matched case-insensitively.
    ///
    /// # Errors
    /// [`SessionError::UnknownRole`] for a role that is not in the model,
    /// [`SessionError::RoleFilter`] when a role's filter expression cannot
    /// be evaluated (the previous view stays), [`SessionError::Cancelled`].
    pub async fn set_view_as(
        &self,
        view: ViewAs,
        cancel: CancellationToken,
    ) -> Result<(), SessionError> {
        let handle = runtime()?;
        let model = self.base_model();
        let mut view = view;
        for name in &mut view.roles {
            let role = model
                .role(name)
                .ok_or_else(|| SessionError::UnknownRole(name.clone()))?;
            *name = role.name.clone();
        }
        let _guard = self.refresh_lock().lock().await;
        let (base, session_model) = self.base_engine_and_model();
        let cancel = cancel.child_token();
        let _stop_on_drop = cancel.clone().drop_guard();
        let (b, v, c) = (base.clone(), view.clone(), cancel.clone());
        let engine = handle
            .spawn_blocking(move || secure(&b, &session_model, &v, Some(&c)))
            .await??;
        self.set_active_view(Some(ActiveView {
            view,
            engine,
            base_generation: base.generation(),
            error: None,
        }));
        Ok(())
    }

    /// [`Session::set_view_as`] with roles and a user name
    /// (`USERPRINCIPALNAME()`); an empty `roles` only sets the user.
    ///
    /// # Errors
    /// See [`Session::set_view_as`].
    pub async fn set_effective_roles<S: AsRef<str>>(
        &self,
        roles: &[S],
        user_name: Option<String>,
    ) -> Result<(), SessionError> {
        self.set_view_as(ViewAs::roles(roles, user_name), CancellationToken::new())
            .await
    }

    /// Stops "View as": queries see all rows again and the effective user
    /// is unset.
    pub fn clear_view_as(&self) {
        self.set_active_view(None);
    }

    /// The active "View as" settings, if any.
    #[must_use]
    pub fn view_as(&self) -> Option<ViewAs> {
        self.active_view_settings()
    }

    /// Why the active view shows no rows (its role filters failed after a
    /// refresh or a derived-column change), if it does.
    #[must_use]
    pub fn view_as_error(&self) -> Option<String> {
        self.active_view_error()
    }
}
