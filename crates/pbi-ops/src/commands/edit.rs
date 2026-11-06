//! Document edit and history commands.
//!
//! Model objects are named by `objectId` (`model.measures` lists measure
//! ids; edits return the ids they change) or by `table` with an optional
//! `column` or `measure` (case-insensitive). Visuals are named like
//! `render.visual`: `report`, `page` and the visual's name.

use pbi_model::{
    ChangeEvent, CommandGroup, CreateMeasure, DeleteMeasure, HistoryResult, Measure, ObjectId,
    ObjectPath, Position, SetDescription, SetMeasureExpression, SetMeasureProperties,
    SetVisualPosition, Visual,
};
use pbi_session::{
    RenameKind, RenameObject, RenamePreview, Session, dax_syntax_error, preview_rename,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{BASE, doc};
use crate::command::{Category, Command, CommandMeta};
use crate::ctx::Ctx;
use crate::error::OpError;
use crate::inspect_model::PositionOut;
use crate::resolve;
use crate::workspace::Workspace;

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
    fn new(session: &Session, result: &HistoryResult) -> Self {
        Self {
            changed: result.changed,
            revision: result.revision,
            dirty: session.is_document_dirty(),
            changed_object_ids: result.events.iter().map(|event| event.id().get()).collect(),
        }
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

    async fn run(
        ws: &Workspace,
        params: Self::Params,
        _ctx: &Ctx,
    ) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &params.doc_id)?;
        resolve::require_model(&entry)?;
        let result = entry
            .session()
            .execute_document_command(SetMeasureExpression {
                measure: ObjectId(params.measure_id),
                expression: params.expression,
            })
            .await?;
        Ok(DocumentEditOut::new(entry.session(), &result))
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

    async fn run(
        ws: &Workspace,
        params: Self::Params,
        _ctx: &Ctx,
    ) -> Result<Self::Output, OpError> {
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
            DocumentEditOut::new(session, &result),
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
        let text = params.text.filter(|t| !t.is_empty());
        let result = session
            .execute_document_command(SetDescription { id, text })
            .await?;
        Ok(DocumentEditOut::new(session, &result))
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
    doc_id: &str,
    target: (Option<&str>, Option<&str>, &str),
    x: Option<f64>,
    y: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
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
    let edit = DocumentEditOut::new(session, &result);
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

    async fn run(
        ws: &Workspace,
        params: Self::Params,
        _ctx: &Ctx,
    ) -> Result<Self::Output, OpError> {
        set_visual_position(
            ws,
            &params.doc_id,
            (
                params.report.as_deref(),
                params.page.as_deref(),
                &params.visual,
            ),
            params.x,
            params.y,
            None,
            None,
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

    async fn run(
        ws: &Workspace,
        params: Self::Params,
        _ctx: &Ctx,
    ) -> Result<Self::Output, OpError> {
        set_visual_position(
            ws,
            &params.doc_id,
            (
                params.report.as_deref(),
                params.page.as_deref(),
                &params.visual,
            ),
            None,
            None,
            params.width,
            params.height,
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

    async fn run(
        ws: &Workspace,
        params: Self::Params,
        _ctx: &Ctx,
    ) -> Result<Self::Output, OpError> {
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
            DocumentEditOut::new(session, &result),
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

    async fn run(
        ws: &Workspace,
        params: Self::Params,
        _ctx: &Ctx,
    ) -> Result<Self::Output, OpError> {
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
            DocumentEditOut::new(session, &result),
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

    async fn run(
        ws: &Workspace,
        params: Self::Params,
        _ctx: &Ctx,
    ) -> Result<Self::Output, OpError> {
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
            DocumentEditOut::new(session, &result)
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

/// `model.dependencies`.
#[derive(Debug)]
pub struct ModelDependencies;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DependenciesParams {
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
}

/// Result of `model.dependencies`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DependenciesOut {
    pub object_id: u64,
    /// `table`, `column` or `measure`.
    pub kind: String,
    /// The table, or the table of the column or measure.
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
            summary: "Lists what refers to a table, column or measure: DAX, relationships, hierarchies, sort-by columns, roles and report fields.",
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
                _ => None,
            };
            names.ok_or_else(|| {
                OpError::invalid_params(format!(
                    "object {} is not a table, column or measure",
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

    async fn run(
        ws: &Workspace,
        params: Self::Params,
        _ctx: &Ctx,
    ) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &params.doc_id)?;
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
            };
        }
        let result = session.execute_document_command(group).await?;
        Ok(DocumentEditOut::new(session, &result))
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

    async fn run(
        ws: &Workspace,
        params: Self::Params,
        _ctx: &Ctx,
    ) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &params.doc_id)?;
        let result = entry.session().undo_document_command().await?;
        Ok(DocumentEditOut::new(entry.session(), &result))
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

    async fn run(
        ws: &Workspace,
        params: Self::Params,
        _ctx: &Ctx,
    ) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &params.doc_id)?;
        let result = entry.session().redo_document_command().await?;
        Ok(DocumentEditOut::new(entry.session(), &result))
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

    async fn run(
        ws: &Workspace,
        params: Self::Params,
        _ctx: &Ctx,
    ) -> Result<Self::Output, OpError> {
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
        let report = session.save(params.overwrite).await.map_err(|e| {
            let mut e = OpError::from(e);
            if e.message.ends_with("save it as a new PBIP project instead") {
                e.message.push_str(" with doc.saveAs");
            }
            e
        })?;
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
            dirty: session.is_document_dirty(),
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

    async fn run(
        ws: &Workspace,
        params: Self::Params,
        _ctx: &Ctx,
    ) -> Result<Self::Output, OpError> {
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
        Ok(DocumentSaveAsOut {
            project_dir: shown_dir(&project_dir),
            pbip_file: report.pbip_file,
            files: report.written,
            bytes: report.bytes,
            previous_format: report.previous_format.as_str().to_owned(),
            revision: report.revision,
            dirty: session.is_document_dirty(),
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
