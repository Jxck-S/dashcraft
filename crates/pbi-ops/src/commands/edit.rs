//! Document edit and history commands.
//!
//! Model objects are named by `objectId` (`model.measures` lists measure
//! ids; edits return the ids they change) or by `table` with an optional
//! `column` or `measure` (case-insensitive). Visuals are named like
//! `render.visual`: `report`, `page` and the visual's name.

use pbi_model::{
    Annotation, Cardinality, ChangeEvent, Column, ColumnRef, CommandGroup, CreateColumn,
    CreateMeasure, CreateRelationship, CrossFilter, DataType, DeleteColumn, DeleteMeasure,
    DeleteRelationship, DeleteTable, HistoryResult, Measure, ObjectId, ObjectPath, Position,
    Relationship, SecurityFilteringBehavior, SetDescription, SetMeasureExpression,
    SetMeasureProperties, SetRelationshipProperties, SetVisualPosition, SummarizeBy, Visual,
    names_eq,
};
use pbi_session::{
    RenameKind, RenameObject, RenamePreview, Session, dax_syntax_error, preview_rename,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{BASE, doc, loaded_doc};
use crate::command::{Category, Command, CommandMeta};
use crate::ctx::{Ctx, DocChangedEvent, ObjectChange, ObjectChangeKind};
use crate::error::OpError;
use crate::inspect_model::PositionOut;
use crate::resolve;
use crate::workspace::{DocEntry, Workspace};

/// `model.setMeasureExpression`.
#[derive(Debug)]
pub struct ModelSetMeasureExpression;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetMeasureExpressionParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Stable measure id returned by `model.measures`.
    pub measure_id: u64,
    /// The replacement DAX expression.
    pub expression: String,
}

/// Result of a document edit or history operation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocumentEditOut {
    pub changed: bool,
    pub revision: u64,
    pub dirty: bool,
    pub changed_object_ids: Vec<u64>,
}

impl DocumentEditOut {
    /// The outcome of `result` on `entry`'s document; reports the change to
    /// `ctx` when the edit committed one (ADR-0012: failed and no-op edits
    /// report nothing).
    fn committed(ctx: &Ctx, entry: &DocEntry, result: &HistoryResult) -> Self {
        let out = Self {
            changed: result.changed,
            revision: result.revision,
            dirty: entry.session().is_document_dirty(),
            changed_object_ids: result.events.iter().map(|event| event.id().get()).collect(),
        };
        if result.changed {
            ctx.doc_changed(DocChangedEvent {
                doc_id: entry.id().to_owned(),
                command: ctx.command().unwrap_or_default().to_owned(),
                revision: out.revision,
                dirty: out.dirty,
                changes: result.events.iter().map(object_change).collect(),
                path: None,
            });
        }
        out
    }
}

fn object_change(event: &ChangeEvent) -> ObjectChange {
    let (kind, old_name, new_name) = match event {
        ChangeEvent::ObjectAdded { .. } => (ObjectChangeKind::Added, None, None),
        ChangeEvent::ObjectChanged { .. } => (ObjectChangeKind::Changed, None, None),
        ChangeEvent::ObjectRenamed { old, new, .. } => (
            ObjectChangeKind::Renamed,
            Some(old.clone()),
            Some(new.clone()),
        ),
        ChangeEvent::ObjectRemoved { .. } => (ObjectChangeKind::Removed, None, None),
    };
    ObjectChange {
        object_id: event.id().get(),
        kind,
        old_name,
        new_name,
    }
}

impl Command for ModelSetMeasureExpression {
    type Params = SetMeasureExpressionParams;
    type Output = DocumentEditOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "model.setMeasureExpression",
            title: "Edit a measure expression",
            summary: "Replaces a measure's DAX expression as one undoable document edit.",
            category: Category::Edit,
            mutates_document: true,
            examples: &[(
                "Update a measure",
                r#"{"docId": "d1", "measureId": 42, "expression": "SUM(Orders[Amount])"}"#,
            )],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, params: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &params.doc_id)?;
        resolve::require_model(&entry)?;
        let result = entry
            .session()
            .execute_document_command(SetMeasureExpression {
                measure: ObjectId(params.measure_id),
                expression: params.expression,
            })
            .await?;
        Ok(DocumentEditOut::committed(ctx, &entry, &result))
    }
}

/// A semantic-model object: `objectId`, or `table` with an optional
/// `column` or `measure` (names case-insensitive).
///
/// # Errors
/// `invalidParams` for a missing or contradictory target, `notFound`,
/// `noDocument` without a semantic model.
fn model_object(
    session: &Session,
    object_id: Option<u64>,
    table: Option<&str>,
    column: Option<&str>,
    measure: Option<&str>,
) -> Result<ObjectId, OpError> {
    let Some(table) = table else {
        return match (object_id, column, measure) {
            (Some(id), None, None) => Ok(ObjectId(id)),
            (Some(_), ..) => Err(OpError::invalid_params(
                "objectId names the object alone: drop column and measure",
            )),
            (None, ..) => Err(OpError::invalid_params(
                "name the object: objectId, or table with an optional column or measure",
            )),
        };
    };
    if object_id.is_some() {
        return Err(OpError::invalid_params(
            "give either objectId or table, not both",
        ));
    }
    let document = session.document();
    let model = document
        .model
        .as_ref()
        .ok_or_else(|| OpError::no_document("the document has no semantic model"))?;
    let name = resolve::table_name(model, table)?;
    let t = model
        .table(&name)
        .ok_or_else(|| OpError::not_found(format!("no table named '{table}'")))?;
    match (column, measure) {
        (None, None) => Ok(t.id),
        (Some(column), None) => t.column(column).map(|c| c.id).ok_or_else(|| {
            OpError::not_found(format!("no column '{column}' in table '{}'", t.name)).with_data(
                json!({ "columns": t.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>() }),
            )
        }),
        (None, Some(measure)) => t.measure(measure).map(|m| m.id).ok_or_else(|| {
            OpError::not_found(format!("no measure '{measure}' in table '{}'", t.name)).with_data(
                json!({ "measures": t.measures.iter().map(|m| m.name.as_str()).collect::<Vec<_>>() }),
            )
        }),
        (Some(_), Some(_)) => Err(OpError::invalid_params(
            "give either column or measure, not both",
        )),
    }
}

/// `model.object.rename`.
#[derive(Debug)]
pub struct ModelObjectRename;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenameObjectParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// The object's id (instead of `table`).
    #[serde(default)]
    pub object_id: Option<u64>,
    /// The table, or the table of `column` / `measure`.
    #[serde(default)]
    pub table: Option<String>,
    /// A column of `table`.
    #[serde(default)]
    pub column: Option<String>,
    /// A measure of `table`.
    #[serde(default)]
    pub measure: Option<String>,
    /// The new name.
    pub new_name: String,
    /// Only report what the rename would rewrite; the document is unchanged.
    #[serde(default)]
    pub dry_run: bool,
}

/// One DAX expression a rename rewrites.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DaxRewriteOut {
    /// The object holding the expression, with the names after the rename
    /// (e.g. `measure 'Orders'[Total Sales]`).
    pub location: String,
    /// `expression`, `formatStringDefinition`, `filterExpression`, …
    pub property: String,
    pub before: String,
    pub after: String,
}

/// Result of `model.object.rename`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RenameObjectOut {
    pub changed: bool,
    pub revision: u64,
    pub dirty: bool,
    pub changed_object_ids: Vec<u64>,
    /// The renamed object's id.
    pub object_id: u64,
    /// `table`, `column` or `measure`.
    pub kind: String,
    /// The renamed table's old name, or the table of the column or measure.
    pub table: String,
    pub old_name: String,
    pub new_name: String,
    pub dry_run: bool,
    /// DAX expressions rewritten (or, with `dryRun`, to be rewritten).
    pub rewrites: Vec<DaxRewriteOut>,
    /// References left as they are that may need a manual fix (M queries,
    /// expressions that do not parse, ambiguous unqualified names).
    pub warnings: Vec<String>,
}

fn rename_out(
    object: ObjectId,
    preview: RenamePreview,
    edit: DocumentEditOut,
    dry_run: bool,
) -> RenameObjectOut {
    let kind = match preview.kind {
        RenameKind::Table => "table",
        RenameKind::Column => "column",
        RenameKind::Measure => "measure",
    };
    RenameObjectOut {
        changed: edit.changed,
        revision: edit.revision,
        dirty: edit.dirty,
        changed_object_ids: edit.changed_object_ids,
        object_id: object.get(),
        kind: kind.to_owned(),
        table: preview.table,
        old_name: preview.old_name,
        new_name: preview.new_name,
        dry_run,
        rewrites: preview
            .rewrites
            .into_iter()
            .map(|r| DaxRewriteOut {
                location: r.location,
                property: r.property,
                before: r.before,
                after: r.after,
            })
            .collect(),
        warnings: preview.warnings,
    }
}

impl Command for ModelObjectRename {
    type Params = RenameObjectParams;
    type Output = RenameObjectOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "model.object.rename",
            title: "Rename a table, column or measure",
            summary: "Renames a table, column or measure as one undoable edit; references, report fields and DAX follow and the loaded data is kept. dryRun previews it.",
            category: Category::Edit,
            mutates_document: true,
            examples: &[
                (
                    "Rename a column",
                    r#"{"docId": "d1", "table": "Orders", "column": "Line Amount", "newName": "Line Total"}"#,
                ),
                (
                    "Preview a table rename",
                    r#"{"docId": "d1", "table": "Orders", "newName": "Sales Orders", "dryRun": true}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, params: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &params.doc_id)?;
        resolve::require_model(&entry)?;
        let session = entry.session();
        let object = model_object(
            session,
            params.object_id,
            params.table.as_deref(),
            params.column.as_deref(),
            params.measure.as_deref(),
        )?;
        if params.dry_run {
            let preview = preview_rename(&session.document(), object, &params.new_name)?;
            let history = session.document_history();
            let unchanged = DocumentEditOut {
                changed: false,
                revision: history.revision,
                dirty: history.dirty,
                changed_object_ids: Vec::new(),
            };
            return Ok(rename_out(object, preview, unchanged, true));
        }
        let (result, preview) = session.rename_object(object, &params.new_name).await?;
        Ok(rename_out(
            object,
            preview,
            DocumentEditOut::committed(ctx, &entry, &result),
            false,
        ))
    }
}

/// `model.object.setDescription`.
#[derive(Debug)]
pub struct ModelObjectSetDescription;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetObjectDescriptionParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// The object's id (instead of `table`): any model object with a
    /// description (tables, columns, measures, hierarchies, roles, …).
    #[serde(default)]
    pub object_id: Option<u64>,
    /// The table, or the table of `column` / `measure`.
    #[serde(default)]
    pub table: Option<String>,
    /// A column of `table`.
    #[serde(default)]
    pub column: Option<String>,
    /// A measure of `table`.
    #[serde(default)]
    pub measure: Option<String>,
    /// The description; absent, `null` or empty clears it.
    #[serde(default)]
    pub text: Option<String>,
}

impl Command for ModelObjectSetDescription {
    type Params = SetObjectDescriptionParams;
    type Output = DocumentEditOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "model.object.setDescription",
            title: "Set an object's description",
            summary: "Sets or clears the description of a table, column, measure or other model object as one undoable edit.",
            category: Category::Edit,
            mutates_document: true,
            examples: &[
                (
                    "Describe a measure",
                    r#"{"docId": "d1", "table": "Orders", "measure": "Total Sales", "text": "Sum of line amounts"}"#,
                ),
                ("Clear a description", r#"{"docId": "d1", "objectId": 42}"#),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, params: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &params.doc_id)?;
        resolve::require_model(&entry)?;
        let session = entry.session();
        let id = model_object(
            session,
            params.object_id,
            params.table.as_deref(),
            params.column.as_deref(),
            params.measure.as_deref(),
        )?;
        let text = params.text.filter(|t| !t.is_empty());
        let result = session
            .execute_document_command(SetDescription { id, text })
            .await?;
        Ok(DocumentEditOut::committed(ctx, &entry, &result))
    }
}

/// The visual `visual` of `page` of `report` (see `render.visual`).
///
/// # Errors
/// `notFound`, `noDocument`, `invalidParams`.
fn visual_target(
    session: &Session,
    report: Option<&str>,
    page: Option<&str>,
    visual: &str,
) -> Result<Visual, OpError> {
    let ri = resolve::report_index(session, report)?;
    let report = resolve::report_at(session, ri)?;
    let pi = resolve::page_index(&report, page)?;
    let page = report
        .pages
        .get(pi)
        .ok_or_else(|| OpError::not_found("no such page"))?;
    resolve::visual(page, visual).cloned()
}

/// The visual's position with the given values replaced.
///
/// # Errors
/// `invalidParams` for a non-finite coordinate, a size that is not
/// positive, nothing to change, or a visual group.
fn moved(
    visual: &Visual,
    x: Option<f64>,
    y: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
) -> Result<Position, OpError> {
    if visual.is_group() {
        return Err(OpError::unsupported(format!(
            "'{}' is a visual group: moving or resizing groups is not supported yet; edit the visuals in it",
            visual.name
        )));
    }
    if [x, y, width, height].iter().all(Option::is_none) {
        return Err(OpError::invalid_params(
            "give at least one of x, y, width and height",
        ));
    }
    let mut position = visual.position.clone();
    for (name, value, slot) in [("x", x, &mut position.x), ("y", y, &mut position.y)] {
        if let Some(v) = value {
            if !v.is_finite() {
                return Err(OpError::invalid_params(format!("{name} must be finite")));
            }
            *slot = v;
        }
    }
    for (name, value, slot) in [
        ("width", width, &mut position.width),
        ("height", height, &mut position.height),
    ] {
        if let Some(v) = value {
            if !(v.is_finite() && v > 0.0) {
                return Err(OpError::invalid_params(format!(
                    "{name} must be a positive number"
                )));
            }
            *slot = v;
        }
    }
    Ok(position)
}

fn position_out(p: &Position) -> PositionOut {
    PositionOut {
        x: p.x,
        y: p.y,
        z: p.z,
        width: p.width,
        height: p.height,
        tab_order: p.tab_order,
    }
}

/// Result of `report.visual.move` and `report.visual.resize`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VisualPositionOut {
    pub changed: bool,
    pub revision: u64,
    pub dirty: bool,
    pub changed_object_ids: Vec<u64>,
    /// The visual's name.
    pub visual: String,
    /// The visual's position after the edit (page units, as in
    /// `report.visuals`).
    pub position: PositionOut,
}

async fn set_visual_position(
    ws: &Workspace,
    ctx: &Ctx,
    doc_id: &str,
    target: (Option<&str>, Option<&str>, &str),
    (x, y): (Option<f64>, Option<f64>),
    (width, height): (Option<f64>, Option<f64>),
) -> Result<VisualPositionOut, OpError> {
    let entry = doc(ws, doc_id)?;
    let session = entry.session();
    let (report, page, name) = target;
    let visual = visual_target(session, report, page, name)?;
    let position = moved(&visual, x, y, width, height)?;
    let result = session
        .execute_document_command(SetVisualPosition {
            visual: visual.id,
            position: position.clone(),
        })
        .await?;
    let edit = DocumentEditOut::committed(ctx, &entry, &result);
    Ok(VisualPositionOut {
        changed: edit.changed,
        revision: edit.revision,
        dirty: edit.dirty,
        changed_object_ids: edit.changed_object_ids,
        visual: visual.name,
        position: position_out(&position),
    })
}

/// `report.visual.move`.
#[derive(Debug)]
pub struct ReportVisualMove;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MoveVisualParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report.
    #[serde(default)]
    pub report: Option<String>,
    /// Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page.
    #[serde(default)]
    pub page: Option<String>,
    /// The visual's name (see `report.visuals`).
    pub visual: String,
    /// New left edge (page units, as in `report.visuals`); unchanged when absent.
    #[serde(default)]
    pub x: Option<f64>,
    /// New top edge; unchanged when absent.
    #[serde(default)]
    pub y: Option<f64>,
}

impl Command for ReportVisualMove {
    type Params = MoveVisualParams;
    type Output = VisualPositionOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "report.visual.move",
            title: "Move a visual",
            summary: "Moves a visual on its page as one undoable edit (visual groups not yet).",
            category: Category::Edit,
            mutates_document: true,
            examples: &[(
                "Move a visual",
                r#"{"docId": "d1", "page": "1", "visual": "f7a9c9413fef68532313", "x": 40, "y": 120}"#,
            )],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, params: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        set_visual_position(
            ws,
            ctx,
            &params.doc_id,
            (
                params.report.as_deref(),
                params.page.as_deref(),
                &params.visual,
            ),
            (params.x, params.y),
            (None, None),
        )
        .await
    }
}

/// `report.visual.resize`.
#[derive(Debug)]
pub struct ReportVisualResize;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResizeVisualParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report.
    #[serde(default)]
    pub report: Option<String>,
    /// Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page.
    #[serde(default)]
    pub page: Option<String>,
    /// The visual's name (see `report.visuals`).
    pub visual: String,
    /// New width (page units, > 0); unchanged when absent.
    #[serde(default)]
    pub width: Option<f64>,
    /// New height (> 0); unchanged when absent.
    #[serde(default)]
    pub height: Option<f64>,
}

impl Command for ReportVisualResize {
    type Params = ResizeVisualParams;
    type Output = VisualPositionOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "report.visual.resize",
            title: "Resize a visual",
            summary: "Resizes a visual (top-left corner fixed) as one undoable edit (visual groups not yet).",
            category: Category::Edit,
            mutates_document: true,
            examples: &[(
                "Resize a visual",
                r#"{"docId": "d1", "page": "1", "visual": "f7a9c9413fef68532313", "width": 320, "height": 180}"#,
            )],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, params: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        set_visual_position(
            ws,
            ctx,
            &params.doc_id,
            (
                params.report.as_deref(),
                params.page.as_deref(),
                &params.visual,
            ),
            (None, None),
            (params.width, params.height),
        )
        .await
    }
}

/// The measure named by `measureId`, or by `table` and `measure`.
fn measure_target(
    session: &Session,
    measure_id: Option<u64>,
    table: Option<&str>,
    measure: Option<&str>,
) -> Result<ObjectId, OpError> {
    match (measure_id, table, measure) {
        (Some(id), None, None) => Ok(ObjectId(id)),
        (None, Some(_), Some(_)) => model_object(session, None, table, None, measure),
        _ => Err(OpError::invalid_params(
            "name the measure: measureId, or table and measure",
        )),
    }
}

/// `(table, name)` of measure `id` in the session's document.
fn measure_names(session: &Session, id: ObjectId) -> Option<(String, String)> {
    let document = session.document();
    let ObjectPath::Measure { table, measure } = document.locate(id)? else {
        return None;
    };
    let t = document.model.as_ref()?.tables.get(table)?;
    Some((t.name.clone(), t.measures.get(measure)?.name.clone()))
}

/// `Some("")` clears an optional text property.
fn clearable(text: Option<String>) -> Option<Option<String>> {
    text.map(|t| (!t.is_empty()).then_some(t))
}

fn check_expression(expression: &str, warnings: &mut Vec<String>) -> Result<(), OpError> {
    if expression.trim().is_empty() {
        return Err(OpError::invalid_params("the measure expression is empty"));
    }
    if let Some(err) = dax_syntax_error(expression) {
        warnings.push(format!(
            "the expression does not parse ({err}); the measure returns an error until it is fixed"
        ));
    }
    Ok(())
}

/// The properties of a new measure.
#[derive(Debug, Clone, Default)]
struct NewMeasure {
    table: String,
    name: String,
    expression: String,
    format_string: Option<String>,
    display_folder: Option<String>,
    description: Option<String>,
    is_hidden: bool,
}

fn create_measure(
    session: &Session,
    new: NewMeasure,
    warnings: &mut Vec<String>,
) -> Result<CreateMeasure, OpError> {
    check_expression(&new.expression, warnings)?;
    let table = model_object(session, None, Some(&new.table), None, None)?;
    let mut measure = Measure::new(new.name, new.expression);
    measure.format_string = new.format_string.filter(|s| !s.is_empty());
    measure.display_folder = new.display_folder.filter(|s| !s.is_empty());
    measure.description = new.description.filter(|s| !s.is_empty());
    measure.is_hidden = new.is_hidden;
    measure.lineage_tag = session.new_lineage_tag();
    Ok(CreateMeasure {
        table,
        measure,
        index: None,
    })
}

/// Changes to a measure's properties; `None` leaves one as it is and an
/// empty text clears it.
#[derive(Debug, Clone, Default)]
struct MeasureChanges {
    expression: Option<String>,
    format_string: Option<String>,
    display_folder: Option<String>,
    description: Option<String>,
    is_hidden: Option<bool>,
}

fn update_measure(
    measure: ObjectId,
    changes: MeasureChanges,
    warnings: &mut Vec<String>,
) -> Result<SetMeasureProperties, OpError> {
    if let Some(expression) = &changes.expression {
        check_expression(expression, warnings)?;
    }
    let command = SetMeasureProperties {
        measure,
        expression: changes.expression,
        format_string: clearable(changes.format_string),
        display_folder: clearable(changes.display_folder),
        description: clearable(changes.description),
        is_hidden: changes.is_hidden,
    };
    if command
        == (SetMeasureProperties {
            measure,
            ..SetMeasureProperties::default()
        })
    {
        return Err(OpError::invalid_params(
            "give at least one of expression, formatString, displayFolder, description, isHidden",
        ));
    }
    Ok(command)
}

/// Result of `model.measure.create`, `model.measure.update` and
/// `model.measure.delete`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MeasureEditOut {
    pub changed: bool,
    pub revision: u64,
    pub dirty: bool,
    pub changed_object_ids: Vec<u64>,
    pub measure_id: u64,
    /// The measure's table.
    pub table: String,
    /// The measure's name.
    pub name: String,
    /// `model.measure.delete` only: what refers to the measure (and breaks
    /// once it is deleted).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependents: Option<Vec<DependentOut>>,
    /// `model.measure.delete` only: whether this was a dry run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    /// Problems that do not stop the edit (DAX that does not parse,
    /// references that cannot be confirmed).
    pub warnings: Vec<String>,
}

/// An object property that refers to another object.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DependentOut {
    /// E.g. `measure 'Orders'[Twice]` or `visual 'card1' (card) on page
    /// 'Overview' of report 'Sales'`.
    pub location: String,
    /// E.g. `expression`, `fields`, `filters`, `filterExpression`.
    pub property: String,
}

impl MeasureEditOut {
    fn new(
        edit: DocumentEditOut,
        measure: ObjectId,
        (table, name): (String, String),
        warnings: Vec<String>,
    ) -> Self {
        Self {
            changed: edit.changed,
            revision: edit.revision,
            dirty: edit.dirty,
            changed_object_ids: edit.changed_object_ids,
            measure_id: measure.get(),
            table,
            name,
            dependents: None,
            dry_run: None,
            warnings,
        }
    }
}

/// `model.measure.create`.
#[derive(Debug)]
pub struct ModelMeasureCreate;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateMeasureParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// The measure's home table.
    pub table: String,
    /// The measure name: unique across the model and not a column of `table`.
    pub name: String,
    /// The DAX expression.
    pub expression: String,
    #[serde(default)]
    pub format_string: Option<String>,
    #[serde(default)]
    pub display_folder: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub is_hidden: bool,
}

impl Command for ModelMeasureCreate {
    type Params = CreateMeasureParams;
    type Output = MeasureEditOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "model.measure.create",
            title: "Create a measure",
            summary: "Adds a DAX measure to a table as one undoable edit and returns its id.",
            category: Category::Edit,
            mutates_document: true,
            examples: &[(
                "Add a measure",
                r##"{"docId": "d1", "table": "Orders", "name": "Order Count", "expression": "COUNTROWS(Orders)", "formatString": "#,0"}"##,
            )],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, params: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &params.doc_id)?;
        resolve::require_model(&entry)?;
        let session = entry.session();
        let mut warnings = Vec::new();
        let new = NewMeasure {
            table: params.table,
            name: params.name,
            expression: params.expression,
            format_string: params.format_string,
            display_folder: params.display_folder,
            description: params.description,
            is_hidden: params.is_hidden,
        };
        let command = create_measure(session, new, &mut warnings)?;
        let result = session.execute_document_command(command).await?;
        let id = result
            .events
            .iter()
            .find_map(|e| match e {
                ChangeEvent::ObjectAdded { id } => Some(*id),
                _ => None,
            })
            .ok_or_else(|| OpError::internal("the new measure has no id"))?;
        let names = measure_names(session, id).unwrap_or_default();
        Ok(MeasureEditOut::new(
            DocumentEditOut::committed(ctx, &entry, &result),
            id,
            names,
            warnings,
        ))
    }
}

/// `model.measure.update`.
#[derive(Debug)]
pub struct ModelMeasureUpdate;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateMeasureParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// The measure's id (instead of `table` and `measure`).
    #[serde(default)]
    pub measure_id: Option<u64>,
    /// The measure's table.
    #[serde(default)]
    pub table: Option<String>,
    /// The measure's name.
    #[serde(default)]
    pub measure: Option<String>,
    /// The new DAX expression.
    #[serde(default)]
    pub expression: Option<String>,
    /// The static format string; empty clears it.
    #[serde(default)]
    pub format_string: Option<String>,
    /// The display folder; empty clears it.
    #[serde(default)]
    pub display_folder: Option<String>,
    /// The description; empty clears it.
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub is_hidden: Option<bool>,
}

impl UpdateMeasureParams {
    fn changes(&self) -> MeasureChanges {
        MeasureChanges {
            expression: self.expression.clone(),
            format_string: self.format_string.clone(),
            display_folder: self.display_folder.clone(),
            description: self.description.clone(),
            is_hidden: self.is_hidden,
        }
    }
}

impl Command for ModelMeasureUpdate {
    type Params = UpdateMeasureParams;
    type Output = MeasureEditOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "model.measure.update",
            title: "Update a measure",
            summary: "Changes a measure's expression, format string, display folder, description or visibility as one undoable edit.",
            category: Category::Edit,
            mutates_document: true,
            examples: &[
                (
                    "New expression and format",
                    r#"{"docId": "d1", "table": "Orders", "measure": "Total Sales", "expression": "SUM(Orders[Line Amount])", "formatString": "$#,0"}"#,
                ),
                (
                    "Hide a measure",
                    r#"{"docId": "d1", "measureId": 42, "isHidden": true}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, params: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &params.doc_id)?;
        resolve::require_model(&entry)?;
        let session = entry.session();
        let id = measure_target(
            session,
            params.measure_id,
            params.table.as_deref(),
            params.measure.as_deref(),
        )?;
        let mut warnings = Vec::new();
        let command = update_measure(id, params.changes(), &mut warnings)?;
        let result = session.execute_document_command(command).await?;
        let names = measure_names(session, id).unwrap_or_default();
        Ok(MeasureEditOut::new(
            DocumentEditOut::committed(ctx, &entry, &result),
            id,
            names,
            warnings,
        ))
    }
}

/// `model.measure.delete`.
#[derive(Debug)]
pub struct ModelMeasureDelete;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteMeasureParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// The measure's id (instead of `table` and `measure`).
    #[serde(default)]
    pub measure_id: Option<u64>,
    /// The measure's table.
    #[serde(default)]
    pub table: Option<String>,
    /// The measure's name.
    #[serde(default)]
    pub measure: Option<String>,
    /// Only list what refers to the measure; the document is unchanged.
    #[serde(default)]
    pub dry_run: bool,
}

impl Command for ModelMeasureDelete {
    type Params = DeleteMeasureParams;
    type Output = MeasureEditOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "model.measure.delete",
            title: "Delete a measure",
            summary: "Deletes a measure as one undoable edit and lists what referred to it (DAX, report fields); dryRun only lists them.",
            category: Category::Edit,
            mutates_document: true,
            examples: &[
                (
                    "What uses a measure",
                    r#"{"docId": "d1", "table": "Orders", "measure": "Total Sales", "dryRun": true}"#,
                ),
                ("Delete by id", r#"{"docId": "d1", "measureId": 42}"#),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, params: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &params.doc_id)?;
        resolve::require_model(&entry)?;
        let session = entry.session();
        let id = measure_target(
            session,
            params.measure_id,
            params.table.as_deref(),
            params.measure.as_deref(),
        )?;
        let names = match measure_names(session, id) {
            Some(names) => names,
            None if session.document().locate(id).is_none() => {
                return Err(OpError::not_found(format!(
                    "no object with id {}",
                    id.get()
                )));
            }
            None => {
                return Err(OpError::invalid_params(format!(
                    "object {} is not a measure",
                    id.get()
                )));
            }
        };
        let found = session.object_dependents(id)?;
        let edit = if params.dry_run {
            let history = session.document_history();
            DocumentEditOut {
                changed: false,
                revision: history.revision,
                dirty: history.dirty,
                changed_object_ids: Vec::new(),
            }
        } else {
            let result = session
                .execute_document_command(DeleteMeasure { measure: id })
                .await?;
            DocumentEditOut::committed(ctx, &entry, &result)
        };
        let mut out = MeasureEditOut::new(edit, id, names, found.warnings);
        out.dependents = Some(
            found
                .dependents
                .into_iter()
                .map(|d| DependentOut {
                    location: d.location,
                    property: d.property,
                })
                .collect(),
        );
        out.dry_run = Some(params.dry_run);
        Ok(out)
    }
}

/// A column data type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum DataTypeParam {
    String,
    Int64,
    Double,
    Decimal,
    DateTime,
    Boolean,
}

impl From<DataTypeParam> for DataType {
    fn from(t: DataTypeParam) -> Self {
        match t {
            DataTypeParam::String => DataType::String,
            DataTypeParam::Int64 => DataType::Int64,
            DataTypeParam::Double => DataType::Double,
            DataTypeParam::Decimal => DataType::Decimal,
            DataTypeParam::DateTime => DataType::DateTime,
            DataTypeParam::Boolean => DataType::Boolean,
        }
    }
}

/// The properties of a new calculated column.
#[derive(Debug, Clone)]
struct NewColumn {
    table: String,
    name: String,
    expression: String,
    data_type: Option<DataTypeParam>,
    format_string: Option<String>,
    display_folder: Option<String>,
    description: Option<String>,
    is_hidden: bool,
}

/// The command that creates `new`, and whether its data type was inferred.
/// Like Power BI Desktop, numeric columns summarize by sum and the others do
/// not summarize.
fn create_column(session: &Session, new: NewColumn) -> Result<(CreateColumn, bool), OpError> {
    if new.expression.trim().is_empty() {
        return Err(OpError::invalid_params("the column expression is empty"));
    }
    if let Some(err) = dax_syntax_error(&new.expression) {
        return Err(OpError::invalid_params(format!(
            "the expression does not parse: {err}"
        )));
    }
    let table = model_object(session, None, Some(&new.table), None, None)?;
    let (data_type, was_inferred) = match new.data_type {
        Some(t) => (DataType::from(t), false),
        None => match session.infer_column_type(&new.table, &new.name, &new.expression)? {
            Some(t) => (t, true),
            None => {
                return Err(OpError::invalid_params(format!(
                    "the data type of {} cannot be inferred (its table has no rows loaded or every value is blank): give dataType",
                    pbi_model::names::dax_column_ref(&new.table, &new.name)
                )));
            }
        },
    };
    let numeric = data_type.is_numeric();
    let mut column = Column::calculated(new.name, data_type, new.expression);
    column.format_string = new.format_string.filter(|s| !s.is_empty());
    column.display_folder = new.display_folder.filter(|s| !s.is_empty());
    column.description = new.description.filter(|s| !s.is_empty());
    column.is_hidden = new.is_hidden;
    column.lineage_tag = session.new_lineage_tag();
    column.summarize_by = Some(if numeric {
        SummarizeBy::Sum
    } else {
        SummarizeBy::None
    });
    column
        .annotations
        .push(Annotation::new("SummarizationSetBy", "Automatic"));
    Ok((
        CreateColumn {
            table,
            column,
            index: None,
        },
        was_inferred,
    ))
}

/// Result of `model.column.create`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ColumnCreateOut {
    pub changed: bool,
    pub revision: u64,
    pub dirty: bool,
    pub changed_object_ids: Vec<u64>,
    pub column_id: u64,
    pub table: String,
    pub name: String,
    /// `string`, `int64`, `double`, `decimal`, `dateTime` or `boolean`.
    pub data_type: String,
    /// Whether the data type was inferred from the values (no `dataType`
    /// given).
    pub data_type_inferred: bool,
}

/// `model.column.create`.
#[derive(Debug)]
pub struct ModelColumnCreate;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateColumnParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// The column's table.
    pub table: String,
    /// The column name: not a column or measure of `table`.
    pub name: String,
    /// The DAX expression, evaluated per row of `table`.
    pub expression: String,
    /// The data type; inferred from the values when omitted (after the
    /// automatic first load; required when the table has no rows loaded or
    /// every value is blank).
    #[serde(default)]
    pub data_type: Option<DataTypeParam>,
    #[serde(default)]
    pub format_string: Option<String>,
    #[serde(default)]
    pub display_folder: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub is_hidden: bool,
}

impl Command for ModelColumnCreate {
    type Params = CreateColumnParams;
    type Output = ColumnCreateOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "model.column.create",
            title: "Create a calculated column",
            summary: "Adds a DAX calculated column to a table as one undoable edit, computes it over the loaded data and infers its data type unless dataType is given.",
            category: Category::Edit,
            mutates_document: true,
            examples: &[(
                "Add a calculated column",
                r##"{"docId": "d1", "table": "Orders", "name": "Margin", "expression": "Orders[Line Amount] - Orders[Line Cost]", "formatString": "#,0.00"}"##,
            )],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, params: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        // Inferring the type needs the data: the automatic first load.
        let entry = if params.data_type.is_none() {
            loaded_doc(ws, &params.doc_id, ctx).await?
        } else {
            doc(ws, &params.doc_id)?
        };
        resolve::require_model(&entry)?;
        let session = entry.session();
        let (command, data_type_inferred) = create_column(
            session,
            NewColumn {
                table: params.table,
                name: params.name,
                expression: params.expression,
                data_type: params.data_type,
                format_string: params.format_string,
                display_folder: params.display_folder,
                description: params.description,
                is_hidden: params.is_hidden,
            },
        )?;
        let data_type = command.column.data_type.as_str().to_owned();
        let result = session.execute_document_command(command).await?;
        let id = result
            .events
            .iter()
            .find_map(|e| match e {
                ChangeEvent::ObjectAdded { id } => Some(*id),
                _ => None,
            })
            .ok_or_else(|| OpError::internal("the new column has no id"))?;
        let (table, name) = {
            let document = session.document();
            match document.locate(id) {
                Some(ObjectPath::Column { table, column }) => document
                    .model
                    .as_ref()
                    .and_then(|m| m.tables.get(table))
                    .and_then(|t| Some((t.name.clone(), t.columns.get(column)?.name.clone())))
                    .unwrap_or_default(),
                _ => Default::default(),
            }
        };
        let edit = DocumentEditOut::committed(ctx, &entry, &result);
        Ok(ColumnCreateOut {
            changed: edit.changed,
            revision: edit.revision,
            dirty: edit.dirty,
            changed_object_ids: edit.changed_object_ids,
            column_id: id.get(),
            table,
            name,
            data_type,
            data_type_inferred,
        })
    }
}

/// Result of `model.column.delete` and `model.table.delete`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ObjectDeleteOut {
    pub changed: bool,
    pub revision: u64,
    pub dirty: bool,
    pub changed_object_ids: Vec<u64>,
    pub object_id: u64,
    /// `column` or `table`.
    pub kind: String,
    /// The table, or the column's table.
    pub table: String,
    pub name: String,
    pub dry_run: bool,
    /// What refers to the object and breaks once it is deleted (DAX,
    /// report fields). A table's own measures, columns and expressions go
    /// with it and are not listed.
    pub dependents: Vec<DependentOut>,
    /// `model.table.delete`: what is deleted with the table (relationships
    /// that join it, role row filters on it).
    pub removed: Vec<String>,
    /// References that cannot be confirmed (DAX that does not parse,
    /// ambiguous unqualified names, M queries naming a table).
    pub warnings: Vec<String>,
}

/// Deletes `id` (a column or table) with `command`, or with `dryRun` checks
/// the delete on a copy of the document; lists what breaks.
async fn delete_object(
    entry: &DocEntry,
    ctx: &Ctx,
    id: ObjectId,
    command: impl pbi_model::Command + Clone + 'static,
    dry_run: bool,
) -> Result<ObjectDeleteOut, OpError> {
    let session = entry.session();
    let (kind, table, name, removed) = {
        let document = session.document();
        let model = document
            .model
            .as_ref()
            .ok_or_else(|| OpError::no_document("the document has no semantic model"))?;
        let path = document
            .locate(id)
            .ok_or_else(|| OpError::not_found(format!("no object with id {}", id.get())))?;
        match path {
            ObjectPath::Column { table, column } => {
                let t = model
                    .tables
                    .get(table)
                    .ok_or_else(|| OpError::not_found(format!("no column with id {}", id.get())))?;
                let c = t
                    .columns
                    .get(column)
                    .ok_or_else(|| OpError::not_found(format!("no column with id {}", id.get())))?;
                ("column", t.name.clone(), c.name.clone(), Vec::new())
            }
            ObjectPath::Table { table } => {
                let t = model
                    .tables
                    .get(table)
                    .ok_or_else(|| OpError::not_found(format!("no table with id {}", id.get())))?;
                let mut removed: Vec<String> = model
                    .relationships
                    .iter()
                    .filter(|r| names_eq(&r.from.table, &t.name) || names_eq(&r.to.table, &t.name))
                    .map(|r| {
                        format!(
                            "relationship '{}'[{}] → '{}'[{}]",
                            r.from.table, r.from.column, r.to.table, r.to.column
                        )
                    })
                    .collect();
                for role in &model.roles {
                    if role
                        .table_permissions
                        .iter()
                        .any(|p| names_eq(&p.table, &t.name))
                    {
                        removed.push(format!("row filter of role '{}'", role.name));
                    }
                }
                ("table", t.name.clone(), t.name.clone(), removed)
            }
            _ => {
                return Err(OpError::invalid_params(format!(
                    "object {} is not a column or table",
                    id.get()
                )));
            }
        }
    };
    let found = session.delete_dependents(id)?;
    let edit = if dry_run {
        let mut copy = (*session.document()).clone();
        pbi_model::Command::apply(&command, &mut copy)?;
        let history = session.document_history();
        DocumentEditOut {
            changed: false,
            revision: history.revision,
            dirty: history.dirty,
            changed_object_ids: Vec::new(),
        }
    } else {
        let result = session.execute_document_command(command).await?;
        DocumentEditOut::committed(ctx, entry, &result)
    };
    Ok(ObjectDeleteOut {
        changed: edit.changed,
        revision: edit.revision,
        dirty: edit.dirty,
        changed_object_ids: edit.changed_object_ids,
        object_id: id.get(),
        kind: kind.to_owned(),
        table,
        name,
        dry_run,
        dependents: found
            .dependents
            .into_iter()
            .map(|d| DependentOut {
                location: d.location,
                property: d.property,
            })
            .collect(),
        removed,
        warnings: found.warnings,
    })
}

/// A column: `columnId`, or `table` and `column`.
fn column_target(
    session: &Session,
    column_id: Option<u64>,
    table: Option<&str>,
    column: Option<&str>,
) -> Result<ObjectId, OpError> {
    match (column_id, table, column) {
        (Some(id), None, None) => Ok(ObjectId(id)),
        (None, Some(table), Some(column)) => {
            model_object(session, None, Some(table), Some(column), None)
        }
        _ => Err(OpError::invalid_params(
            "name the column: columnId, or table and column",
        )),
    }
}

/// A table: `tableId` or `table`.
fn table_target(
    session: &Session,
    table_id: Option<u64>,
    table: Option<&str>,
) -> Result<ObjectId, OpError> {
    match (table_id, table) {
        (Some(id), None) => Ok(ObjectId(id)),
        (None, Some(table)) => model_object(session, None, Some(table), None, None),
        _ => Err(OpError::invalid_params("name the table: tableId or table")),
    }
}

/// `model.column.delete`.
#[derive(Debug)]
pub struct ModelColumnDelete;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteColumnParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// The column's id (instead of `table` and `column`).
    #[serde(default)]
    pub column_id: Option<u64>,
    /// The column's table.
    #[serde(default)]
    pub table: Option<String>,
    /// The column's name.
    #[serde(default)]
    pub column: Option<String>,
    /// Only check the delete and list what refers to the column; the
    /// document is unchanged.
    #[serde(default)]
    pub dry_run: bool,
}

impl Command for ModelColumnDelete {
    type Params = DeleteColumnParams;
    type Output = ObjectDeleteOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "model.column.delete",
            title: "Delete a calculated column",
            summary: "Deletes a calculated column as one undoable edit and lists what referred to it; refuses data columns and columns used by relationships, hierarchies or sort orders. dryRun only checks.",
            category: Category::Edit,
            mutates_document: true,
            examples: &[
                (
                    "What uses a calculated column",
                    r#"{"docId": "d1", "table": "Orders", "column": "Line Cost", "dryRun": true}"#,
                ),
                ("Delete by id", r#"{"docId": "d1", "columnId": 42}"#),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, params: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &params.doc_id)?;
        resolve::require_model(&entry)?;
        let id = column_target(
            entry.session(),
            params.column_id,
            params.table.as_deref(),
            params.column.as_deref(),
        )?;
        if !matches!(
            entry.session().document().locate(id),
            Some(ObjectPath::Column { .. }) | None
        ) {
            return Err(OpError::invalid_params(format!(
                "object {} is not a column",
                id.get()
            )));
        }
        delete_object(&entry, ctx, id, DeleteColumn { column: id }, params.dry_run).await
    }
}

/// `model.table.delete`.
#[derive(Debug)]
pub struct ModelTableDelete;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteTableParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// The table's id (instead of `table`).
    #[serde(default)]
    pub table_id: Option<u64>,
    /// The table's name.
    #[serde(default)]
    pub table: Option<String>,
    /// Only list what refers to the table and what goes with it; the
    /// document is unchanged.
    #[serde(default)]
    pub dry_run: bool,
}

impl Command for ModelTableDelete {
    type Params = DeleteTableParams;
    type Output = ObjectDeleteOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "model.table.delete",
            title: "Delete a table",
            summary: "Deletes a table with its columns, measures, relationships and role row filters as one undoable edit and lists what referred to it (DAX, report fields); dryRun only lists them.",
            category: Category::Edit,
            mutates_document: true,
            examples: &[
                (
                    "What a table delete breaks",
                    r#"{"docId": "d1", "table": "Stores", "dryRun": true}"#,
                ),
                ("Delete by id", r#"{"docId": "d1", "tableId": 3}"#),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, params: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &params.doc_id)?;
        resolve::require_model(&entry)?;
        let id = table_target(entry.session(), params.table_id, params.table.as_deref())?;
        if !matches!(
            entry.session().document().locate(id),
            Some(ObjectPath::Table { .. }) | None
        ) {
            return Err(OpError::invalid_params(format!(
                "object {} is not a table",
                id.get()
            )));
        }
        delete_object(&entry, ctx, id, DeleteTable { table: id }, params.dry_run).await
    }
}
/// `model.dependencies`.
#[derive(Debug)]
pub struct ModelDependencies;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DependenciesParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// The object's id (instead of `table`): also a relationship's id from
    /// `model.relationships`.
    #[serde(default)]
    pub object_id: Option<u64>,
    /// The table, or the table of `column` / `measure`.
    #[serde(default)]
    pub table: Option<String>,
    /// A column of `table`.
    #[serde(default)]
    pub column: Option<String>,
    /// A measure of `table`.
    #[serde(default)]
    pub measure: Option<String>,
}

/// Result of `model.dependencies`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DependenciesOut {
    pub object_id: u64,
    /// `table`, `column`, `measure` or `relationship`.
    pub kind: String,
    /// The table, or the table of the column or measure (the from table of
    /// a relationship).
    pub table: String,
    pub name: String,
    /// The document revision the list was computed on.
    pub revision: u64,
    /// What refers to the object: DAX expressions, relationships, hierarchy
    /// levels, sort-by columns, role permissions, report, page and visual
    /// fields and filters.
    pub dependents: Vec<DependentOut>,
    /// References that cannot be confirmed (DAX that does not parse,
    /// ambiguous unqualified names, M queries naming a table).
    pub warnings: Vec<String>,
}

impl Command for ModelDependencies {
    type Params = DependenciesParams;
    type Output = DependenciesOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "model.dependencies",
            title: "Object dependents",
            summary: "Lists what refers to a table, column, measure or relationship: DAX, relationships, hierarchies, sort-by columns, roles and report fields.",
            category: Category::Inspect,
            examples: &[
                (
                    "What uses a measure",
                    r#"{"docId": "d1", "table": "Orders", "measure": "Total Sales"}"#,
                ),
                (
                    "What uses a column",
                    r#"{"docId": "d1", "table": "Orders", "column": "Line Amount"}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(
        ws: &Workspace,
        params: Self::Params,
        _ctx: &Ctx,
    ) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &params.doc_id)?;
        resolve::require_model(&entry)?;
        let session = entry.session();
        let id = model_object(
            session,
            params.object_id,
            params.table.as_deref(),
            params.column.as_deref(),
            params.measure.as_deref(),
        )?;
        let (kind, table, name) = {
            let document = session.document();
            let model = document
                .model
                .as_ref()
                .ok_or_else(|| OpError::no_document("the document has no semantic model"))?;
            let path = document
                .locate(id)
                .ok_or_else(|| OpError::not_found(format!("no object with id {}", id.get())))?;
            let names = match path {
                ObjectPath::Table { table } => model
                    .tables
                    .get(table)
                    .map(|t| ("table", t.name.clone(), t.name.clone())),
                ObjectPath::Column { table, column } => model.tables.get(table).and_then(|t| {
                    let c = t.columns.get(column)?;
                    Some(("column", t.name.clone(), c.name.clone()))
                }),
                ObjectPath::Measure { table, measure } => model.tables.get(table).and_then(|t| {
                    let m = t.measures.get(measure)?;
                    Some(("measure", t.name.clone(), m.name.clone()))
                }),
                ObjectPath::Relationship { relationship } => model
                    .relationships
                    .get(relationship)
                    .map(|r| ("relationship", r.from.table.clone(), r.name.clone())),
                _ => None,
            };
            names.ok_or_else(|| {
                OpError::invalid_params(format!(
                    "object {} is not a table, column, measure or relationship",
                    id.get()
                ))
            })?
        };
        let revision = session.document_history().revision;
        let found = session.object_dependents(id)?;
        Ok(DependenciesOut {
            object_id: id.get(),
            kind: kind.to_owned(),
            table,
            name,
            revision,
            dependents: found
                .dependents
                .into_iter()
                .map(|d| DependentOut {
                    location: d.location,
                    property: d.property,
                })
                .collect(),
            warnings: found.warnings,
        })
    }
}

/// A column named by table and column (case-insensitive).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColumnParam {
    pub table: String,
    pub column: String,
}

/// One end of a relationship.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum CardinalityParam {
    One,
    Many,
}

impl From<CardinalityParam> for Cardinality {
    fn from(c: CardinalityParam) -> Self {
        match c {
            CardinalityParam::One => Cardinality::One,
            CardinalityParam::Many => Cardinality::Many,
        }
    }
}

/// Which way filters cross a relationship.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum CrossFilterParam {
    OneDirection,
    BothDirections,
}

impl From<CrossFilterParam> for CrossFilter {
    fn from(c: CrossFilterParam) -> Self {
        match c {
            CrossFilterParam::OneDirection => CrossFilter::OneDirection,
            CrossFilterParam::BothDirections => CrossFilter::BothDirections,
        }
    }
}

/// Which way row-level security filters cross a relationship.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SecurityFilterParam {
    OneDirection,
    /// Needs `crossFilteringBehavior: bothDirections`.
    BothDirections,
    None,
}

impl From<SecurityFilterParam> for SecurityFilteringBehavior {
    fn from(c: SecurityFilterParam) -> Self {
        match c {
            SecurityFilterParam::OneDirection => SecurityFilteringBehavior::OneDirection,
            SecurityFilterParam::BothDirections => SecurityFilteringBehavior::BothDirections,
            SecurityFilterParam::None => SecurityFilteringBehavior::None,
        }
    }
}

/// The properties of a new relationship.
#[derive(Debug, Clone)]
struct NewRelationship {
    from: ColumnParam,
    to: ColumnParam,
    from_cardinality: Option<CardinalityParam>,
    to_cardinality: Option<CardinalityParam>,
    cross_filtering_behavior: Option<CrossFilterParam>,
    is_active: Option<bool>,
    name: Option<String>,
}

/// The model's spelling of `c` (as given when there is no such column: the
/// model command rejects it).
fn column_ref(session: &Session, c: &ColumnParam) -> ColumnRef {
    let document = session.document();
    document
        .model
        .as_ref()
        .and_then(|m| {
            let t = m.table(&c.table)?;
            Some(ColumnRef::new(&t.name, &t.column(&c.column)?.name))
        })
        .unwrap_or_else(|| ColumnRef::new(&c.table, &c.column))
}

fn create_relationship(session: &Session, new: NewRelationship) -> CreateRelationship {
    let name = new
        .name
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| session.new_relationship_name());
    let mut relationship = Relationship::new(
        name,
        column_ref(session, &new.from),
        column_ref(session, &new.to),
    );
    relationship.from_cardinality = new
        .from_cardinality
        .unwrap_or(CardinalityParam::Many)
        .into();
    relationship.to_cardinality = new.to_cardinality.unwrap_or(CardinalityParam::One).into();
    relationship.cross_filtering_behavior = new
        .cross_filtering_behavior
        .unwrap_or(CrossFilterParam::OneDirection)
        .into();
    relationship.is_active = new.is_active.unwrap_or(true);
    CreateRelationship {
        relationship,
        index: None,
    }
}

/// The relationship named by `relationshipId` or by `name`.
fn relationship_target(
    session: &Session,
    relationship_id: Option<u64>,
    name: Option<&str>,
) -> Result<ObjectId, OpError> {
    let document = session.document();
    let model = document
        .model
        .as_ref()
        .ok_or_else(|| OpError::no_document("the document has no semantic model"))?;
    match (relationship_id, name) {
        (Some(id), None) => match document.locate(ObjectId(id)) {
            Some(ObjectPath::Relationship { .. }) => Ok(ObjectId(id)),
            Some(_) => Err(OpError::invalid_params(format!(
                "object {id} is not a relationship"
            ))),
            None => Err(OpError::not_found(format!("no object with id {id}"))),
        },
        (None, Some(name)) => model
            .relationships
            .iter()
            .find(|r| names_eq(&r.name, name))
            .map(|r| r.id)
            .ok_or_else(|| {
                OpError::not_found(format!("no relationship named '{name}'")).with_data(json!({
                    "relationships": model.relationships.iter().map(|r| r.name.as_str()).collect::<Vec<_>>()
                }))
            }),
        _ => Err(OpError::invalid_params(
            "name the relationship: relationshipId or name",
        )),
    }
}

/// Result of `model.relationship.create`, `model.relationship.update` and
/// `model.relationship.delete`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelationshipEditOut {
    pub changed: bool,
    pub revision: u64,
    pub dirty: bool,
    pub changed_object_ids: Vec<u64>,
    pub relationship_id: u64,
    /// The relationship's name (a GUID unless one was given).
    pub name: String,
    /// The from column (usually the many side).
    pub from: ColumnParam,
    /// The to column (usually the one side).
    pub to: ColumnParam,
    /// `many` or `one` (after the edit; before it for a delete).
    pub from_cardinality: String,
    /// `many` or `one`.
    pub to_cardinality: String,
    /// `oneDirection`, `bothDirections` or `automatic`.
    pub cross_filtering_behavior: String,
    /// `oneDirection`, `bothDirections` or `none`.
    pub security_filtering_behavior: String,
    pub is_active: bool,
    /// `model.relationship.delete` only: the DAX whose `USERELATIONSHIP`
    /// or `CROSSFILTER` calls name the relationship.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependents: Option<Vec<DependentOut>>,
    /// `model.relationship.delete` only: whether this was a dry run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    /// Problems that do not stop the edit: what the engine finds in the
    /// loaded data (duplicate keys on the one side, differing key types,
    /// ambiguous filter paths), references that cannot be confirmed.
    pub warnings: Vec<String>,
}

impl RelationshipEditOut {
    fn new(edit: DocumentEditOut, r: &Relationship, warnings: Vec<String>) -> Self {
        let end = |c: &ColumnRef| ColumnParam {
            table: c.table.clone(),
            column: c.column.clone(),
        };
        Self {
            changed: edit.changed,
            revision: edit.revision,
            dirty: edit.dirty,
            changed_object_ids: edit.changed_object_ids,
            relationship_id: r.id.get(),
            name: r.name.clone(),
            from: end(&r.from),
            to: end(&r.to),
            from_cardinality: r.from_cardinality.as_str().to_owned(),
            to_cardinality: r.to_cardinality.as_str().to_owned(),
            cross_filtering_behavior: r.cross_filtering_behavior.as_str().to_owned(),
            security_filtering_behavior: r.security_filtering_behavior.as_str().to_owned(),
            is_active: r.is_active,
            dependents: None,
            dry_run: None,
            warnings,
        }
    }
}

/// Relationship `id` in the session's document.
fn find_relationship(session: &Session, id: ObjectId) -> Option<Relationship> {
    let document = session.document();
    let Some(ObjectPath::Relationship { relationship }) = document.locate(id) else {
        return None;
    };
    document
        .model
        .as_ref()?
        .relationships
        .get(relationship)
        .cloned()
}

/// `model.relationship.create`.
#[derive(Debug)]
pub struct ModelRelationshipCreate;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateRelationshipParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// The from column (TOM `fromColumn`, usually the many side).
    pub from: ColumnParam,
    /// The to column (TOM `toColumn`, usually the one side).
    pub to: ColumnParam,
    /// Default `many`.
    #[serde(default)]
    pub from_cardinality: Option<CardinalityParam>,
    /// Default `one`.
    #[serde(default)]
    pub to_cardinality: Option<CardinalityParam>,
    /// Default `oneDirection`.
    #[serde(default)]
    pub cross_filtering_behavior: Option<CrossFilterParam>,
    /// Default `true`. Two tables have at most one active relationship.
    #[serde(default)]
    pub is_active: Option<bool>,
    /// Unique among relationships; a new GUID by default, as Power BI
    /// Desktop names them.
    #[serde(default)]
    pub name: Option<String>,
}

impl CreateRelationshipParams {
    fn new_relationship(self) -> NewRelationship {
        NewRelationship {
            from: self.from,
            to: self.to,
            from_cardinality: self.from_cardinality,
            to_cardinality: self.to_cardinality,
            cross_filtering_behavior: self.cross_filtering_behavior,
            is_active: self.is_active,
            name: self.name,
        }
    }
}

impl Command for ModelRelationshipCreate {
    type Params = CreateRelationshipParams;
    type Output = RelationshipEditOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "model.relationship.create",
            title: "Create a relationship",
            summary: "Adds a relationship between two columns as one undoable edit; filters follow it at once and warnings report what the engine finds in the loaded data.",
            category: Category::Edit,
            mutates_document: true,
            examples: &[
                (
                    "Many-to-one, single direction",
                    r#"{"docId": "d1", "from": {"table": "Orders", "column": "Store ID"}, "to": {"table": "Stores", "column": "Store ID"}}"#,
                ),
                (
                    "Inactive, both directions",
                    r#"{"docId": "d1", "from": {"table": "Orders", "column": "Ship Date"}, "to": {"table": "Date", "column": "Date"}, "crossFilteringBehavior": "bothDirections", "isActive": false}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, params: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &params.doc_id)?;
        resolve::require_model(&entry)?;
        let session = entry.session();
        let command = create_relationship(session, params.new_relationship());
        let result = session.execute_document_command(command).await?;
        let id = result
            .events
            .iter()
            .find_map(|e| match e {
                ChangeEvent::ObjectAdded { id } => Some(*id),
                _ => None,
            })
            .ok_or_else(|| OpError::internal("the new relationship has no id"))?;
        let relationship = find_relationship(session, id)
            .ok_or_else(|| OpError::internal("the new relationship is missing"))?;
        Ok(RelationshipEditOut::new(
            DocumentEditOut::committed(ctx, &entry, &result),
            &relationship,
            relationship_warnings(session, id),
        ))
    }
}

/// What the engine reports about relationship `id` in the loaded data.
fn relationship_warnings(session: &Session, id: ObjectId) -> Vec<String> {
    session
        .diagnostics()
        .into_iter()
        .filter(|d| d.object == Some(id))
        .map(|d| format!("{}: {}", d.code, d.message))
        .collect()
}

/// The property changes of `model.relationship.update` and
/// `updateRelationship`.
#[derive(Debug, Clone, Default)]
struct RelationshipChanges {
    from_cardinality: Option<CardinalityParam>,
    to_cardinality: Option<CardinalityParam>,
    cross_filtering_behavior: Option<CrossFilterParam>,
    security_filtering_behavior: Option<SecurityFilterParam>,
    is_active: Option<bool>,
}

fn update_relationship(
    relationship: ObjectId,
    changes: RelationshipChanges,
) -> Result<SetRelationshipProperties, OpError> {
    let command = SetRelationshipProperties {
        relationship,
        from_cardinality: changes.from_cardinality.map(Into::into),
        to_cardinality: changes.to_cardinality.map(Into::into),
        cross_filtering_behavior: changes.cross_filtering_behavior.map(Into::into),
        security_filtering_behavior: changes.security_filtering_behavior.map(Into::into),
        is_active: changes.is_active,
    };
    if command
        == (SetRelationshipProperties {
            relationship,
            ..SetRelationshipProperties::default()
        })
    {
        return Err(OpError::invalid_params(
            "give at least one of fromCardinality, toCardinality, crossFilteringBehavior, securityFilteringBehavior, isActive",
        ));
    }
    Ok(command)
}

/// `model.relationship.delete`.
#[derive(Debug)]
pub struct ModelRelationshipDelete;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteRelationshipParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// The relationship's id from `model.relationships` (instead of `name`).
    #[serde(default)]
    pub relationship_id: Option<u64>,
    /// The relationship's name (case-insensitive).
    #[serde(default)]
    pub name: Option<String>,
    /// Only list what uses the relationship; the document is unchanged.
    #[serde(default)]
    pub dry_run: bool,
}

impl Command for ModelRelationshipDelete {
    type Params = DeleteRelationshipParams;
    type Output = RelationshipEditOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "model.relationship.delete",
            title: "Delete a relationship",
            summary: "Deletes a relationship as one undoable edit and lists the DAX that names it (USERELATIONSHIP, CROSSFILTER); dryRun only lists it.",
            category: Category::Edit,
            mutates_document: true,
            examples: &[
                (
                    "What uses a relationship",
                    r#"{"docId": "d1", "relationshipId": 7, "dryRun": true}"#,
                ),
                (
                    "Delete by name",
                    r#"{"docId": "d1", "name": "d0c1e2f3-a4b5-4c6d-8e7f-000000000001"}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, params: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &params.doc_id)?;
        resolve::require_model(&entry)?;
        let session = entry.session();
        let id = relationship_target(session, params.relationship_id, params.name.as_deref())?;
        let relationship = find_relationship(session, id)
            .ok_or_else(|| OpError::not_found(format!("no relationship with id {}", id.get())))?;
        let found = session.object_dependents(id)?;
        let edit = if params.dry_run {
            let history = session.document_history();
            DocumentEditOut {
                changed: false,
                revision: history.revision,
                dirty: history.dirty,
                changed_object_ids: Vec::new(),
            }
        } else {
            let result = session
                .execute_document_command(DeleteRelationship { relationship: id })
                .await?;
            DocumentEditOut::committed(ctx, &entry, &result)
        };
        let mut out = RelationshipEditOut::new(edit, &relationship, found.warnings);
        out.dependents = Some(
            found
                .dependents
                .into_iter()
                .map(|d| DependentOut {
                    location: d.location,
                    property: d.property,
                })
                .collect(),
        );
        out.dry_run = Some(params.dry_run);
        Ok(out)
    }
}

/// `model.relationship.update`.
#[derive(Debug)]
pub struct ModelRelationshipUpdate;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateRelationshipParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// The relationship's id from `model.relationships` (instead of `name`).
    #[serde(default)]
    pub relationship_id: Option<u64>,
    /// The relationship's name (case-insensitive).
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub from_cardinality: Option<CardinalityParam>,
    #[serde(default)]
    pub to_cardinality: Option<CardinalityParam>,
    #[serde(default)]
    pub cross_filtering_behavior: Option<CrossFilterParam>,
    /// How row-level security filters cross it; `bothDirections` needs
    /// `crossFilteringBehavior: bothDirections`.
    #[serde(default)]
    pub security_filtering_behavior: Option<SecurityFilterParam>,
    /// Two tables have at most one active relationship: deactivate the
    /// other one first (both in one `doc.applyEdits` for one undo step).
    #[serde(default)]
    pub is_active: Option<bool>,
}

impl UpdateRelationshipParams {
    fn changes(&self) -> RelationshipChanges {
        RelationshipChanges {
            from_cardinality: self.from_cardinality,
            to_cardinality: self.to_cardinality,
            cross_filtering_behavior: self.cross_filtering_behavior,
            security_filtering_behavior: self.security_filtering_behavior,
            is_active: self.is_active,
        }
    }
}

impl Command for ModelRelationshipUpdate {
    type Params = UpdateRelationshipParams;
    type Output = RelationshipEditOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "model.relationship.update",
            title: "Update a relationship",
            summary: "Changes a relationship's cardinality, cross-filter direction, security filtering or active state as one undoable edit; filters follow it at once.",
            category: Category::Edit,
            mutates_document: true,
            examples: &[
                (
                    "Filter both ways",
                    r#"{"docId": "d1", "relationshipId": 7, "crossFilteringBehavior": "bothDirections"}"#,
                ),
                (
                    "Deactivate",
                    r#"{"docId": "d1", "name": "d0c1e2f3-a4b5-4c6d-8e7f-000000000001", "isActive": false}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, params: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &params.doc_id)?;
        resolve::require_model(&entry)?;
        let session = entry.session();
        let id = relationship_target(session, params.relationship_id, params.name.as_deref())?;
        let command = update_relationship(id, params.changes())?;
        let result = session.execute_document_command(command).await?;
        let relationship = find_relationship(session, id)
            .ok_or_else(|| OpError::internal("the relationship is missing"))?;
        Ok(RelationshipEditOut::new(
            DocumentEditOut::committed(ctx, &entry, &result),
            &relationship,
            relationship_warnings(session, id),
        ))
    }
}

/// One reversible edit accepted by `doc.applyEdits`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type",
    deny_unknown_fields
)]
pub enum EditOperation {
    SetMeasureExpression {
        measure_id: u64,
        expression: String,
    },
    SetDescription {
        object_id: u64,
        /// `null` clears the description.
        text: Option<String>,
    },
    /// Like `model.object.rename` (without `dryRun`).
    RenameObject {
        #[serde(default)]
        object_id: Option<u64>,
        #[serde(default)]
        table: Option<String>,
        #[serde(default)]
        column: Option<String>,
        #[serde(default)]
        measure: Option<String>,
        new_name: String,
    },
    /// Moves and/or resizes a visual (`report.visual.move`,
    /// `report.visual.resize`).
    SetVisualPosition {
        #[serde(default)]
        report: Option<String>,
        #[serde(default)]
        page: Option<String>,
        visual: String,
        #[serde(default)]
        x: Option<f64>,
        #[serde(default)]
        y: Option<f64>,
        #[serde(default)]
        width: Option<f64>,
        #[serde(default)]
        height: Option<f64>,
    },
    /// Like `model.measure.create`.
    CreateMeasure {
        table: String,
        name: String,
        expression: String,
        #[serde(default)]
        format_string: Option<String>,
        #[serde(default)]
        display_folder: Option<String>,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        is_hidden: bool,
    },
    /// Like `model.measure.update`.
    UpdateMeasure {
        #[serde(default)]
        measure_id: Option<u64>,
        #[serde(default)]
        table: Option<String>,
        #[serde(default)]
        measure: Option<String>,
        #[serde(default)]
        expression: Option<String>,
        #[serde(default)]
        format_string: Option<String>,
        #[serde(default)]
        display_folder: Option<String>,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        is_hidden: Option<bool>,
    },
    /// Like `model.measure.delete` (without `dryRun`).
    DeleteMeasure {
        #[serde(default)]
        measure_id: Option<u64>,
        #[serde(default)]
        table: Option<String>,
        #[serde(default)]
        measure: Option<String>,
    },
    /// Like `model.relationship.create`.
    CreateRelationship {
        from: ColumnParam,
        to: ColumnParam,
        #[serde(default)]
        from_cardinality: Option<CardinalityParam>,
        #[serde(default)]
        to_cardinality: Option<CardinalityParam>,
        #[serde(default)]
        cross_filtering_behavior: Option<CrossFilterParam>,
        #[serde(default)]
        is_active: Option<bool>,
        #[serde(default)]
        name: Option<String>,
    },
    /// Like `model.relationship.update`.
    UpdateRelationship {
        #[serde(default)]
        relationship_id: Option<u64>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        from_cardinality: Option<CardinalityParam>,
        #[serde(default)]
        to_cardinality: Option<CardinalityParam>,
        #[serde(default)]
        cross_filtering_behavior: Option<CrossFilterParam>,
        #[serde(default)]
        security_filtering_behavior: Option<SecurityFilterParam>,
        #[serde(default)]
        is_active: Option<bool>,
    },
    /// Like `model.relationship.delete` (without `dryRun`).
    DeleteRelationship {
        #[serde(default)]
        relationship_id: Option<u64>,
        #[serde(default)]
        name: Option<String>,
    },
    /// Like `model.column.create`. Without `dataType` the type is inferred
    /// against the document before the group, so the expression cannot use
    /// objects created earlier in the group.
    CreateColumn {
        table: String,
        name: String,
        expression: String,
        #[serde(default)]
        data_type: Option<DataTypeParam>,
        #[serde(default)]
        format_string: Option<String>,
        #[serde(default)]
        display_folder: Option<String>,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        is_hidden: bool,
    },
    /// Like `model.column.delete` (without `dryRun`).
    DeleteColumn {
        #[serde(default)]
        column_id: Option<u64>,
        #[serde(default)]
        table: Option<String>,
        #[serde(default)]
        column: Option<String>,
    },
    /// Like `model.table.delete` (without `dryRun`).
    DeleteTable {
        #[serde(default)]
        table_id: Option<u64>,
        #[serde(default)]
        table: Option<String>,
    },
}

/// `doc.applyEdits`.
#[derive(Debug)]
pub struct DocumentApplyEdits;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyEditsParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Edits applied atomically and undone as one history step. Names are
    /// resolved against the document before the first edit.
    pub edits: Vec<EditOperation>,
}

impl Command for DocumentApplyEdits {
    type Params = ApplyEditsParams;
    type Output = DocumentEditOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "doc.applyEdits",
            title: "Apply document edits",
            summary: "Applies a group of document edits atomically as one undoable history step.",
            category: Category::Edit,
            mutates_document: true,
            examples: &[(
                "Update a measure and its description",
                r#"{"docId": "d1", "edits": [{"type": "setMeasureExpression", "measureId": 42, "expression": "SUM(Orders[Amount])"}, {"type": "setDescription", "objectId": 42, "text": "Total amount"}]}"#,
            )],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, params: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &params.doc_id)?;
        if params.edits.iter().any(|e| {
            matches!(
                e,
                EditOperation::CreateColumn {
                    data_type: None,
                    ..
                }
            )
        }) {
            entry.ensure_loaded(ctx).await?;
        }
        let session = entry.session();
        let mut group = CommandGroup::new("Apply document edits");
        for edit in params.edits {
            group = match edit {
                EditOperation::SetMeasureExpression {
                    measure_id,
                    expression,
                } => group.with(SetMeasureExpression {
                    measure: ObjectId(measure_id),
                    expression,
                }),
                EditOperation::SetDescription { object_id, text } => group.with(SetDescription {
                    id: ObjectId(object_id),
                    text,
                }),
                EditOperation::RenameObject {
                    object_id,
                    table,
                    column,
                    measure,
                    new_name,
                } => group.with(RenameObject {
                    object: model_object(
                        session,
                        object_id,
                        table.as_deref(),
                        column.as_deref(),
                        measure.as_deref(),
                    )?,
                    new_name,
                }),
                EditOperation::SetVisualPosition {
                    report,
                    page,
                    visual,
                    x,
                    y,
                    width,
                    height,
                } => {
                    let visual =
                        visual_target(session, report.as_deref(), page.as_deref(), &visual)?;
                    group.with(SetVisualPosition {
                        visual: visual.id,
                        position: moved(&visual, x, y, width, height)?,
                    })
                }
                EditOperation::CreateMeasure {
                    table,
                    name,
                    expression,
                    format_string,
                    display_folder,
                    description,
                    is_hidden,
                } => {
                    let new = NewMeasure {
                        table,
                        name,
                        expression,
                        format_string,
                        display_folder,
                        description,
                        is_hidden,
                    };
                    group.with(create_measure(session, new, &mut Vec::new())?)
                }
                EditOperation::UpdateMeasure {
                    measure_id,
                    table,
                    measure,
                    expression,
                    format_string,
                    display_folder,
                    description,
                    is_hidden,
                } => {
                    let id =
                        measure_target(session, measure_id, table.as_deref(), measure.as_deref())?;
                    let changes = MeasureChanges {
                        expression,
                        format_string,
                        display_folder,
                        description,
                        is_hidden,
                    };
                    group.with(update_measure(id, changes, &mut Vec::new())?)
                }
                EditOperation::DeleteMeasure {
                    measure_id,
                    table,
                    measure,
                } => group.with(DeleteMeasure {
                    measure: measure_target(
                        session,
                        measure_id,
                        table.as_deref(),
                        measure.as_deref(),
                    )?,
                }),
                EditOperation::CreateRelationship {
                    from,
                    to,
                    from_cardinality,
                    to_cardinality,
                    cross_filtering_behavior,
                    is_active,
                    name,
                } => group.with(create_relationship(
                    session,
                    NewRelationship {
                        from,
                        to,
                        from_cardinality,
                        to_cardinality,
                        cross_filtering_behavior,
                        is_active,
                        name,
                    },
                )),
                EditOperation::UpdateRelationship {
                    relationship_id,
                    name,
                    from_cardinality,
                    to_cardinality,
                    cross_filtering_behavior,
                    security_filtering_behavior,
                    is_active,
                } => {
                    let id = relationship_target(session, relationship_id, name.as_deref())?;
                    let changes = RelationshipChanges {
                        from_cardinality,
                        to_cardinality,
                        cross_filtering_behavior,
                        security_filtering_behavior,
                        is_active,
                    };
                    group.with(update_relationship(id, changes)?)
                }
                EditOperation::DeleteRelationship {
                    relationship_id,
                    name,
                } => group.with(DeleteRelationship {
                    relationship: relationship_target(session, relationship_id, name.as_deref())?,
                }),
                EditOperation::CreateColumn {
                    table,
                    name,
                    expression,
                    data_type,
                    format_string,
                    display_folder,
                    description,
                    is_hidden,
                } => {
                    let new = NewColumn {
                        table,
                        name,
                        expression,
                        data_type,
                        format_string,
                        display_folder,
                        description,
                        is_hidden,
                    };
                    group.with(create_column(session, new)?.0)
                }
                EditOperation::DeleteColumn {
                    column_id,
                    table,
                    column,
                } => group.with(DeleteColumn {
                    column: column_target(session, column_id, table.as_deref(), column.as_deref())?,
                }),
                EditOperation::DeleteTable { table_id, table } => group.with(DeleteTable {
                    table: table_target(session, table_id, table.as_deref())?,
                }),
            };
        }
        let result = session.execute_document_command(group).await?;
        Ok(DocumentEditOut::committed(ctx, &entry, &result))
    }
}

/// `doc.undo`.
#[derive(Debug)]
pub struct DocumentUndo;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
}

impl Command for DocumentUndo {
    type Params = DocumentParams;
    type Output = DocumentEditOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "doc.undo",
            title: "Undo document edit",
            summary: "Undoes the latest document edit.",
            category: Category::Edit,
            mutates_document: true,
            examples: &[("Undo the latest edit", r#"{"docId": "d1"}"#)],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, params: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &params.doc_id)?;
        let result = entry.session().undo_document_command().await?;
        Ok(DocumentEditOut::committed(ctx, &entry, &result))
    }
}

/// `doc.redo`.
#[derive(Debug)]
pub struct DocumentRedo;

impl Command for DocumentRedo {
    type Params = DocumentParams;
    type Output = DocumentEditOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "doc.redo",
            title: "Redo document edit",
            summary: "Reapplies the latest undone document edit.",
            category: Category::Edit,
            mutates_document: true,
            examples: &[("Redo the latest undone edit", r#"{"docId": "d1"}"#)],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, params: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &params.doc_id)?;
        let result = entry.session().redo_document_command().await?;
        Ok(DocumentEditOut::committed(ctx, &entry, &result))
    }
}

/// `doc.history`.
#[derive(Debug)]
pub struct DocumentHistory;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocumentHistoryOut {
    pub revision: u64,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redo_label: Option<String>,
}

impl Command for DocumentHistory {
    type Params = DocumentParams;
    type Output = DocumentHistoryOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "doc.history",
            title: "Document edit history",
            summary: "Returns the document revision, dirty state and undo/redo availability.",
            category: Category::Edit,
            examples: &[("Current edit history", r#"{"docId": "d1"}"#)],
            ..BASE
        }
    }

    async fn run(
        ws: &Workspace,
        params: Self::Params,
        _ctx: &Ctx,
    ) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &params.doc_id)?;
        let session = entry.session();
        let history = session.document_history();
        Ok(DocumentHistoryOut {
            revision: history.revision,
            dirty: history.dirty,
            can_undo: history.can_undo,
            can_redo: history.can_redo,
            undo_label: history.undo_label,
            redo_label: history.redo_label,
        })
    }
}

/// `doc.save`.
#[derive(Debug)]
pub struct DocumentSave;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentSaveParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Save even if project files changed on disk since the open or last
    /// save: files DashCraft writes get the document's version, other
    /// files are kept. Without it such a save fails with `conflict`.
    #[serde(default)]
    pub overwrite: bool,
}

/// How a project file changed on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum FileChangeKind {
    Added,
    Modified,
    Removed,
}

/// A project file changed on disk by another program.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FileChangeOut {
    /// Path relative to the project folder.
    pub path: String,
    pub change: FileChangeKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSaveOut {
    /// The project folder.
    pub project_dir: String,
    /// Files written, relative to the project folder.
    pub written: Vec<String>,
    /// Files deleted (e.g. the `.tmdl` file of a renamed table).
    pub deleted: Vec<String>,
    /// Files changed on disk since the open or last save (only with
    /// `overwrite`).
    pub overwritten: Vec<FileChangeOut>,
    /// The revision saved.
    pub revision: u64,
    /// Whether the document has edits after the save (always `false`).
    pub dirty: bool,
}

impl Command for DocumentSave {
    type Params = DocumentSaveParams;
    type Output = DocumentSaveOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "doc.save",
            title: "Save",
            summary: "Saves the edits into the opened PBIP project, writing only changed files; files changed on disk since the open are a conflict unless overwrite.",
            category: Category::Edit,
            mutates_document: true,
            writes_files: true,
            examples: &[
                ("Save the edits", r#"{"docId": "d1"}"#),
                (
                    "Save over external changes",
                    r#"{"docId": "d1", "overwrite": true}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, params: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &params.doc_id)?;
        let session = entry.session();
        let opened = session.opened();
        let root = opened
            .container
            .root_dir()
            .filter(|_| opened.document.source.format == pbi_model::SourceFormat::Pbip);
        let Some(root) = root else {
            return Err(OpError::unsupported(
                "cannot save: only a PBIP project opened from a folder can be saved in place; use doc.saveAs to save it as a new PBIP project",
            ));
        };
        // Every file written lies under the project folder (the save refuses
        // other paths and symbolic links).
        let project_dir = ws.policy().check_write(root)?;
        let was_dirty = session.is_document_dirty();
        let report = session.save(params.overwrite).await.map_err(|e| {
            let mut e = OpError::from(e);
            if e.message.ends_with("save it as a new PBIP project instead") {
                e.message.push_str(" with doc.saveAs");
            }
            e
        })?;
        let dirty = session.is_document_dirty();
        if was_dirty && !dirty {
            ctx.doc_changed(DocChangedEvent {
                doc_id: entry.id().to_owned(),
                command: ctx.command().unwrap_or_default().to_owned(),
                revision: report.revision,
                dirty,
                changes: Vec::new(),
                path: None,
            });
        }
        Ok(DocumentSaveOut {
            project_dir: shown_dir(&project_dir),
            written: report.written,
            deleted: report.deleted,
            overwritten: report
                .overwritten
                .into_iter()
                .map(|c| FileChangeOut {
                    path: c.path,
                    change: match c.change {
                        pbi_core::ChangeKind::Added => FileChangeKind::Added,
                        pbi_core::ChangeKind::Modified => FileChangeKind::Modified,
                        pbi_core::ChangeKind::Removed => FileChangeKind::Removed,
                    },
                })
                .collect(),
            revision: report.revision,
            dirty,
        })
    }
}

/// `doc.saveAs`.
#[derive(Debug)]
pub struct DocumentSaveAs;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentSaveAsParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// The new project folder (created if missing; must be empty unless
    /// `overwrite`).
    pub out_dir: std::path::PathBuf,
    /// Write into a non-empty folder (files with the same paths are
    /// replaced, others kept).
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSaveAsOut {
    /// The new project folder.
    pub project_dir: String,
    /// The project's `.pbip` file, relative to `projectDir`.
    pub pbip_file: String,
    /// Files written, relative to `projectDir`.
    pub files: Vec<String>,
    pub bytes: u64,
    /// The format the document had before (`pbix`, `pbit`, `pbip`, …).
    pub previous_format: String,
    /// The revision saved.
    pub revision: u64,
    /// Whether the document has edits after the save (always `false`).
    pub dirty: bool,
    /// What the new project does not carry over (imported data, a local
    /// data cache).
    pub warnings: Vec<String>,
}

impl Command for DocumentSaveAs {
    type Params = DocumentSaveAsParams;
    type Output = DocumentSaveAsOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "doc.saveAs",
            title: "Save As",
            summary: "Saves the document, edits included, as a new PBIP project from any format and continues editing there: later doc.save writes to it; imported data is not carried over.",
            category: Category::Edit,
            mutates_document: true,
            writes_files: true,
            long_running: true,
            examples: &[
                (
                    "Save as a new project",
                    r#"{"docId": "d1", "outDir": "target/tmp/acme-copy"}"#,
                ),
                (
                    "Into an existing folder",
                    r#"{"docId": "d1", "outDir": "target/tmp/acme-copy", "overwrite": true}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, params: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let out_dir = super::export::check_out_dir(ws, &params.out_dir, params.overwrite)?;
        let entry = doc(ws, &params.doc_id)?;
        let session = entry.session();
        let report = session.save_as(&out_dir, params.overwrite).await?;
        let mut warnings = Vec::new();
        if report.data_not_carried {
            warnings.push(
                "the imported data of the source is not carried over: refresh the project from its data sources".to_owned(),
            );
        }
        if report.cache_not_copied {
            warnings.push(
                "the local data cache (.pbi/cache.abf) of the source is not copied: refresh the project from its data sources".to_owned(),
            );
        }
        // The reread project's folder (absolute), as doc.save reports it.
        let project_dir = session
            .opened()
            .container
            .root_dir()
            .map_or_else(|| out_dir.clone(), std::path::Path::to_path_buf);
        entry.relocate(
            project_dir.join(&report.pbip_file),
            params
                .out_dir
                .join(&report.pbip_file)
                .to_string_lossy()
                .into_owned(),
        );
        let dirty = session.is_document_dirty();
        ctx.doc_changed(DocChangedEvent {
            doc_id: entry.id().to_owned(),
            command: ctx.command().unwrap_or_default().to_owned(),
            revision: report.revision,
            dirty,
            changes: Vec::new(),
            path: Some(entry.shown_path()),
        });
        Ok(DocumentSaveAsOut {
            project_dir: shown_dir(&project_dir),
            pbip_file: report.pbip_file,
            files: report.written,
            bytes: report.bytes,
            previous_format: report.previous_format.as_str().to_owned(),
            revision: report.revision,
            dirty,
            warnings,
        })
    }
}

/// An absolute folder as people write it: without the `\\?\` prefix that
/// canonicalizing adds on Windows.
fn shown_dir(path: &std::path::Path) -> String {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        rest.to_owned()
    } else {
        text.into_owned()
    }
}
