//! Applying a bookmark: the page it shows, the slicer selections and
//! filter cards it captures and the visuals it hides or sorts, as
//! [`InteractionState`]s.
//!
//! # What a PBIR bookmark holds
//!
//! `explorationState.activeSection` is the page to show;
//! `explorationState.sections.<page>.visualContainers.<visual>.singleVisual`
//! holds each captured visual's state as a diff against the saved visual:
//! `objects.remove` (`[{object, property, selector}]`) and `objects.merge`
//! (`{object: [{properties, selector}]}`). A slicer's selection is its
//! `general.filter` property (see [`super::interaction`]), so applying
//! the diff to the saved visual and reading the selection back gives the
//! bookmarked selection.
//!
//! Visibility: a captured visual is hidden when its
//! `singleVisual.display.mode` is `hidden` and shown otherwise;
//! `sections.<page>.visualContainerGroups.<group>.isHidden` (with nested
//! `children`) does the same for groups. Visuals and groups the bookmark
//! does not capture keep their saved `isHidden`
//! ([`InteractionState::visibility`] holds the differences).
//!
//! Filters: `explorationState.filters` (report),
//! `sections.<page>.filters` (page) and
//! `visualContainers.<visual>.filters` (visual) are `FiltersState`s; each
//! container in `byName` (by filter name) or `byExpr` (by name, else by
//! field) replaces its card's condition with its `filter` (none: the card
//! does not filter; [`InteractionState::filters`]). Report filters are
//! set on each page the bookmark captures; "Selected visuals" bookmarks
//! set only their targets' visual filters.
//!
//! Sort: a captured visual's `singleVisual.orderBy` (semantic-query sort
//! clauses) replaces its `sortDefinition`; without one it keeps its saved
//! order ([`InteractionState::sort`]).
//!
//! `options`:
//! - `applyOnlyToTargetVisuals` + `targetVisualNames` ("Selected
//!   visuals"): only those visuals change; the rest of the page keeps its
//!   current state. Otherwise ("All visuals") every page in `sections`
//!   starts from its saved state and the cross-filter is cleared.
//! - `suppressData` ("Data" unchecked): selections and filters are not
//!   applied.
//! - `suppressDisplay` ("Display" unchecked): visibility, sort, drill and
//!   formatting are not applied (the page keeps its current visibility).
//! - `suppressActiveSection` ("Current page" unchecked): the page does not
//!   change.
//!
//! Not applied yet (reported in [`BookmarkEffect::not_applied`]): changed
//! Top N filters, `byType` and transient filters, filter containers not
//! in the report, the spotlight, focus and elevation display modes, drill
//! and expansion state, projection changes, cross-highlights and
//! formatting of other visuals.

use indexmap::IndexMap;
use pbi_model::{Bookmark, Filter, FilterKind, Page, Report};
use serde::Serialize;
use serde_json::{Map, Value};

use super::interaction::{FilterLevel, InteractionState, is_slicer_type, persisted_selection};
use super::plan::same_field;
use super::semantic::{Aliases, field, sort_clauses, where_condition};

/// The result of applying a bookmark.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkEffect {
    /// Name of the page to show; `None` keeps the current page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    /// Page name → its new interaction state (pages the bookmark changes).
    pub states: IndexMap<String, InteractionState>,
    /// Parts of the bookmark that are not applied yet, one line each.
    pub not_applied: Vec<String>,
}

/// Applies `bookmark` to `report`. `current` gives a page's current
/// interaction state (used when the bookmark changes only target visuals).
pub fn apply_bookmark(
    report: &Report,
    bookmark: &Bookmark,
    current: &dyn Fn(&Page) -> InteractionState,
) -> BookmarkEffect {
    let mut effect = BookmarkEffect::default();
    let opts = &bookmark.options;
    let flag = |key: &str| opts.get(key).and_then(Value::as_bool).unwrap_or(false);
    let target_only = flag("applyOnlyToTargetVisuals");
    let targets: Vec<&str> = opts
        .get("targetVisualNames")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let applies_to = |visual: &str| !target_only || targets.contains(&visual);
    let state = &bookmark.exploration_state;

    if !flag("suppressActiveSection")
        && let Some(active) = state.get("activeSection").and_then(Value::as_str)
    {
        if report.pages.iter().any(|p| p.name == active) {
            effect.page = Some(active.to_owned());
        } else {
            note(&mut effect, format!("page {active} is not in the report"));
        }
    }

    let empty = Map::new();
    let sections = state
        .get("sections")
        .and_then(Value::as_object)
        .unwrap_or(&empty);

    for (section, captured) in sections {
        let Some(page) = report.pages.iter().find(|p| &p.name == section) else {
            note(&mut effect, format!("page {section} is not in the report"));
            continue;
        };
        let label = if page.display_name.is_empty() {
            &page.name
        } else {
            &page.display_name
        };
        let containers = captured
            .get("visualContainers")
            .and_then(Value::as_object)
            .unwrap_or(&empty);

        let data = !flag("suppressData");
        let display = !flag("suppressDisplay");
        if !data && !display {
            continue;
        }
        let mut new_state = if data && !target_only {
            let mut s = InteractionState::from_page(page).0;
            if !display {
                let now = current(page);
                s.visibility = now.visibility;
                s.sort = now.sort;
            }
            s
        } else {
            current(page)
        };

        if data {
            if target_only
                && new_state
                    .selected_points
                    .as_ref()
                    .is_some_and(|p| applies_to(&p.source))
            {
                new_state.selected_points = None;
            }
            for visual in page
                .visuals
                .iter()
                .filter(|v| is_slicer_type(&v.visual_type) && applies_to(&v.name))
            {
                let Some(changes) = containers
                    .get(&visual.name)
                    .and_then(|c| c.get("singleVisual"))
                    .and_then(|s| s.get("objects"))
                else {
                    continue;
                };
                let mut bookmarked = visual.clone();
                apply_object_changes(&mut bookmarked.objects, changes);
                match persisted_selection(&bookmarked) {
                    Ok(Some(selection)) => {
                        new_state.select(visual.name.clone(), selection);
                    }
                    Ok(None) => {
                        new_state.clear_slicer(&visual.name);
                    }
                    Err(reason) => note(
                        &mut effect,
                        format!("{label}: slicer {}: {reason}", visual.name),
                    ),
                }
            }
            if !target_only {
                let mut cx = FilterCx {
                    state: &mut new_state,
                    effect: &mut effect,
                    label,
                };
                cx.apply(
                    FilterLevel::Report,
                    &report.filters,
                    state.get("filters"),
                    "report",
                );
                cx.apply(
                    FilterLevel::Page,
                    &page.filters,
                    captured.get("filters"),
                    "page",
                );
            }
            let mut highlights = false;
            for (name, c) in containers.iter().filter(|(n, _)| applies_to(n)) {
                if let Some(visual) = page.visuals.iter().find(|v| &v.name == name) {
                    new_state.filters.visuals.shift_remove(name);
                    FilterCx {
                        state: &mut new_state,
                        effect: &mut effect,
                        label,
                    }
                    .apply(
                        FilterLevel::Visual(name),
                        &visual.filters,
                        c.get("filters"),
                        "visual",
                    );
                }
                highlights |= c
                    .get("singleVisual")
                    .is_some_and(|s| s.get("highlight").is_some())
                    || c.get("highlight").is_some();
            }
            if highlights {
                note(&mut effect, format!("{label}: cross-highlights"));
            }
        }

        if display {
            if !target_only {
                new_state.visibility.clear();
                new_state.sort.clear();
            }
            for (name, c) in containers.iter().filter(|(n, _)| applies_to(n)) {
                let single = c.get("singleVisual").unwrap_or(&Value::Null);
                let slicer = page
                    .visuals
                    .iter()
                    .find(|v| &v.name == name)
                    .is_some_and(|v| is_slicer_type(&v.visual_type));
                let mode = single
                    .get("display")
                    .and_then(|d| d.get("mode"))
                    .and_then(Value::as_str);
                new_state.set_hidden(page, name, mode == Some("hidden"));
                if let Some(mode) = mode.filter(|m| *m != "hidden") {
                    note(&mut effect, format!("{label}: {mode} display mode"));
                }
                match single.get("orderBy") {
                    None => {
                        new_state.set_sort(page, name, None);
                    }
                    Some(order) => match sort_clauses(order) {
                        Some(sort) => {
                            new_state.set_sort(page, name, Some(sort));
                        }
                        None => note(&mut effect, format!("{label}: sort")),
                    },
                }
                if single.get("expansionStates").is_some()
                    || single.get("drillFilterOtherVisuals").is_some()
                    || c.get("drilling").is_some()
                {
                    note(&mut effect, format!("{label}: drill and expansion state"));
                }
                if single.get("activeProjections").is_some() {
                    note(&mut effect, format!("{label}: projection changes"));
                }
                if !slicer && has_object_changes(single.get("objects")) {
                    note(&mut effect, format!("{label}: formatting"));
                }
            }
            if let Some(groups) = captured
                .get("visualContainerGroups")
                .and_then(Value::as_object)
            {
                set_group_visibility(page, &mut new_state, groups, &applies_to, 0);
            }
        }
        effect.states.insert(page.name.clone(), new_state);
    }
    effect
}

/// Applies a captured `filters` state (`FiltersState`) to filter cards.
struct FilterCx<'a> {
    state: &'a mut InteractionState,
    effect: &'a mut BookmarkEffect,
    label: &'a str,
}

impl FilterCx<'_> {
    /// Each captured container (`byName`, matched by name; `byExpr`, by
    /// name or field) sets its card's condition from its `filter`. Top N
    /// cards that differ, unmatched containers and `byType` /
    /// `byTransientState` are listed as not applied.
    fn apply(
        &mut self,
        level: FilterLevel<'_>,
        cards: &[Filter],
        captured: Option<&Value>,
        what: &str,
    ) {
        let Some(captured) = captured.and_then(Value::as_object) else {
            return;
        };
        let by_name = captured
            .get("byName")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(name, c)| {
                (
                    Some(c.get("name").and_then(Value::as_str).unwrap_or(name)),
                    c,
                )
            });
        let by_expr = captured
            .get("byExpr")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|c| (c.get("name").and_then(Value::as_str), c));
        let aliases = Aliases::default();
        for (name, c) in by_name.chain(by_expr) {
            let card = match name {
                Some(name) => cards.iter().find(|f| f.name == name),
                None => {
                    let expr = c.get("expression").map(|e| field(e, &aliases));
                    cards.iter().find(|f| {
                        f.field
                            .as_ref()
                            .zip(expr.as_ref())
                            .is_some_and(|(a, b)| same_field(a, b))
                    })
                }
            };
            let Some(card) = card else {
                if name.is_none() {
                    self.note(format!("{}: a {what} filter not in the report", self.label));
                }
                continue;
            };
            let def = c.get("filter");
            if card.kind == FilterKind::TopN {
                if def != card.raw.get("filter") {
                    self.note(format!("{}: Top N {what} filter {}", self.label, card.name));
                }
                continue;
            }
            self.state
                .set_filter(level, card, def.and_then(where_condition));
        }
        for key in ["byType", "byTransientState"] {
            if captured
                .get(key)
                .and_then(Value::as_array)
                .is_some_and(|a| !a.is_empty())
            {
                self.note(format!("{}: {what} filters {key}", self.label));
            }
        }
    }

    fn note(&mut self, message: String) {
        note(self.effect, message);
    }
}

/// Deepest `visualContainerGroups.*.children` nesting followed.
const MAX_GROUP_DEPTH: usize = 32;

/// Applies `isHidden` of each captured group (and its child groups).
fn set_group_visibility(
    page: &Page,
    state: &mut InteractionState,
    groups: &Map<String, Value>,
    applies_to: &dyn Fn(&str) -> bool,
    depth: usize,
) {
    if depth >= MAX_GROUP_DEPTH {
        return;
    }
    for (name, group) in groups {
        if applies_to(name) {
            let hidden = group
                .get("isHidden")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            state.set_hidden(page, name, hidden);
        }
        if let Some(children) = group.get("children").and_then(Value::as_object) {
            set_group_visibility(page, state, children, applies_to, depth + 1);
        }
    }
}

fn note(effect: &mut BookmarkEffect, message: String) {
    if !effect.not_applied.contains(&message) {
        effect.not_applied.push(message);
    }
}

fn has_object_changes(objects: Option<&Value>) -> bool {
    objects.is_some_and(|o| {
        o.get("merge")
            .and_then(Value::as_object)
            .is_some_and(|m| !m.is_empty())
            || o.get("remove")
                .and_then(Value::as_array)
                .is_some_and(|r| !r.is_empty())
    })
}

fn same_selector(a: Option<&Value>, b: Option<&Value>) -> bool {
    use pbi_pbir::objects::is_default_selector;
    (is_default_selector(a) && is_default_selector(b)) || a == b
}

/// Applies a bookmark's `remove` then `merge` to a visual's `objects`.
pub fn apply_object_changes(objects: &mut Value, changes: &Value) {
    for r in changes
        .get("remove")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(object), Some(property)) = (
            r.get("object").and_then(Value::as_str),
            r.get("property").and_then(Value::as_str),
        ) else {
            continue;
        };
        let selector = r.get("selector");
        for entry in objects
            .get_mut(object)
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            if same_selector(entry.get("selector"), selector)
                && let Some(props) = entry.get_mut("properties").and_then(Value::as_object_mut)
            {
                props.remove(property);
            }
        }
    }
    let Some(merge) = changes.get("merge").and_then(Value::as_object) else {
        return;
    };
    if !objects.is_object() {
        *objects = Value::Object(Map::new());
    }
    let Some(map) = objects.as_object_mut() else {
        return;
    };
    for (object, entries) in merge {
        let list = map
            .entry(object.clone())
            .or_insert_with(|| Value::Array(Vec::new()));
        if !list.is_array() {
            *list = Value::Array(Vec::new());
        }
        let Some(list) = list.as_array_mut() else {
            continue;
        };
        for entry in entries.as_array().into_iter().flatten() {
            let selector = entry.get("selector");
            let props = entry.get("properties").and_then(Value::as_object);
            match list
                .iter_mut()
                .find(|e| same_selector(e.get("selector"), selector))
            {
                Some(existing) => {
                    if !existing.get("properties").is_some_and(Value::is_object) {
                        existing["properties"] = Value::Object(Map::new());
                    }
                    if let (Some(target), Some(props)) = (
                        existing
                            .get_mut("properties")
                            .and_then(Value::as_object_mut),
                        props,
                    ) {
                        for (k, v) in props {
                            target.insert(k.clone(), v.clone());
                        }
                    }
                }
                None => list.push(entry.clone()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn remove_then_merge() {
        let mut objects = json!({"general": [{"properties": {"filter": 1, "keep": 2}}]});
        apply_object_changes(
            &mut objects,
            &json!({"remove": [{"object": "general", "property": "filter", "selector": {}}]}),
        );
        assert_eq!(objects, json!({"general": [{"properties": {"keep": 2}}]}));
        apply_object_changes(
            &mut objects,
            &json!({"merge": {
                "general": [{"properties": {"filter": 3}}],
                "data": [{"properties": {"mode": 4}, "selector": {"id": "x"}}]
            }}),
        );
        assert_eq!(
            objects,
            json!({
                "general": [{"properties": {"keep": 2, "filter": 3}}],
                "data": [{"properties": {"mode": 4}, "selector": {"id": "x"}}]
            })
        );
        let mut null = Value::Null;
        apply_object_changes(
            &mut null,
            &json!({"merge": {"general": [{"properties": {}}]}}),
        );
        assert_eq!(null, json!({"general": [{"properties": {}}]}));
    }
}
