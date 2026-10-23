//! Theme resolution: base + custom report themes, colors (including
//! `ThemeDataColor{ColorId, Percent}`), text classes and formatting-property
//! lookup with Power BI's precedence.
//!
//! # Layers
//!
//! [`Theme`] deep-merges three theme JSON documents, later ones winning:
//! the built-in fallback ([`defaults::builtin_theme`]: default palette,
//! structural colors, primary text classes) ← the report's **base** theme ←
//! its **custom** theme. Objects merge recursively; arrays of objects merge
//! element-wise (matching `$id` when present); other arrays and scalars are
//! replaced.
//!
//! # Property precedence
//!
//! [`ThemeResolver::visual`] gives a [`VisualProps`] view of one visual. A
//! property `object.prop` resolves from the first source that has it (typed
//! getters such as [`PropLookup::number`] skip values that do not decode,
//! e.g. `fontSize: 'abc'`, and fall through to the next source):
//!
//! 1. the visual's `objects` ([`VisualProps::objects`]) or
//!    `visualContainerObjects` ([`VisualProps::container`]), default
//!    selector only (for schema ≤ 2.2 files without
//!    `visualContainerObjects`, container lookups also read `objects`);
//! 2. theme `visualStyles[visualType][stylePreset]`, then `["*"]`
//!    (`object` entry, then the `"*"` wildcard object), default `$id`
//!    only;
//! 3. theme `visualStyles["*"]["*"]` (same order);
//! 4. built-in defaults ([`defaults::builtin_property`], plus text-class
//!    derived `title.fontColor`/`fontSize`/`fontFamily`).
//!
//! # Text classes
//!
//! Primary classes (`callout`, `title`, `header`, `label`) come from the
//! merged theme. Secondary classes are derived as in Power BI's theme
//! documentation unless the theme defines them: `largeTitle` (title, 14pt),
//! `semiboldLabel` (label, Segoe UI Semibold), `boldLabel` (label, Segoe UI
//! Bold), `largeLabel` (12pt), `smallLabel` (9pt), `lightLabel` (`#605E5C`
//! = `foregroundNeutralSecondary`), `largeLightLabel`, `smallLightLabel`.
//! Unknown names fall back to `label`.

pub mod defaults;
pub mod value;

use std::borrow::Cow;
use std::sync::OnceLock;

use pbi_model::{Page, Visual};
use pbi_pbir::{PbirReport, ThemeJson};
use serde_json::Value;

use crate::scene::{Color, Insets};
use crate::text::TextStyle;

pub use defaults::{builtin_property, title_shown_by_default};
pub use value::{as_bool, as_number, as_text};

/// A merged report theme (built-in ← base ← custom).
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    json: ThemeJson,
    /// `visualStyles` of the merged theme (`Null` if none).
    visual_styles: Value,
    /// Parsed `dataColors` (invalid entries skipped).
    palette: Vec<Color>,
}

impl Default for Theme {
    fn default() -> Self {
        Self::builtin()
    }
}

impl Theme {
    /// Power BI's defaults only (no report theme).
    pub fn builtin() -> Self {
        Self::from_values(None, None)
    }

    /// Merges optional base and custom theme JSON documents over the
    /// built-in defaults. Invalid documents are skipped (with a warning).
    pub fn from_values(base: Option<&Value>, custom: Option<&Value>) -> Self {
        let builtin = defaults::builtin_theme();
        let layers: Vec<&Value> = [base, custom].into_iter().flatten().collect();
        // Try all layers; on a typed-field error drop the top layer and retry.
        for n in (0..=layers.len()).rev() {
            let mut merged = builtin.clone();
            for layer in layers.iter().take(n) {
                if layer.is_object() {
                    merge_json(&mut merged, layer);
                }
            }
            match serde_json::from_value::<ThemeJson>(merged.clone()) {
                Ok(json) => {
                    let visual_styles = merged.get("visualStyles").cloned().unwrap_or(Value::Null);
                    let palette = json
                        .data_colors
                        .iter()
                        .flatten()
                        .filter_map(|c| Color::parse(c))
                        .collect();
                    return Self {
                        json,
                        visual_styles,
                        palette,
                    };
                }
                Err(e) => tracing::warn!("ignoring invalid theme layer: {e}"),
            }
        }
        Self {
            json: ThemeJson::default(),
            visual_styles: Value::Null,
            palette: Vec::new(),
        }
    }

    /// From parsed theme files.
    pub fn from_json(base: Option<&ThemeJson>, custom: Option<&ThemeJson>) -> Self {
        let to_value = |t: &ThemeJson| serde_json::to_value(t).ok();
        let base = base.and_then(to_value);
        let custom = custom.and_then(to_value);
        Self::from_values(base.as_ref(), custom.as_ref())
    }

    /// The base and custom themes referenced by a PBIR report's
    /// `themeCollection`.
    pub fn from_pbir(report: &PbirReport) -> Self {
        Self::from_json(report.base_theme(), report.custom_theme())
    }

    /// The merged theme.
    pub fn json(&self) -> &ThemeJson {
        &self.json
    }

    /// Merged `visualStyles`, verbatim.
    pub fn visual_styles(&self) -> &Value {
        &self.visual_styles
    }

    /// Theme style entry `visualStyles[visual_type][style][object][default].prop`
    /// (falling back to the `"*"` wildcard object).
    pub fn style_property(
        &self,
        visual_type: &str,
        style: &str,
        object: &str,
        prop: &str,
    ) -> Option<&Value> {
        let objects = self.visual_styles.get(visual_type)?.get(style)?;
        [object, "*"].into_iter().find_map(|o| {
            objects
                .get(o)?
                .as_array()?
                .iter()
                .filter(|entry| {
                    entry
                        .get("$id")
                        .is_none_or(|id| id.as_str() == Some("default"))
                })
                .find_map(|entry| entry.get(prop))
        })
    }
}

/// Deep merge of `top` into `base` (see the module docs).
fn merge_json(base: &mut Value, top: &Value) {
    match (base, top) {
        (Value::Object(b), Value::Object(t)) => {
            for (k, tv) in t {
                match b.get_mut(k) {
                    Some(bv) => merge_json(bv, tv),
                    None => {
                        b.insert(k.clone(), tv.clone());
                    }
                }
            }
        }
        (Value::Array(b), Value::Array(t))
            if b.iter().all(Value::is_object) && t.iter().all(Value::is_object) =>
        {
            for (i, tv) in t.iter().enumerate() {
                let id = tv.get("$id");
                let slot = match id {
                    Some(id) => b.iter().position(|bv| bv.get("$id") == Some(id)),
                    None => b.get(i).filter(|bv| bv.get("$id").is_none()).map(|_| i),
                };
                match slot.and_then(|s| b.get_mut(s)) {
                    Some(bv) => merge_json(bv, tv),
                    None => b.push(tv.clone()),
                }
            }
        }
        (b, t) => *b = t.clone(),
    }
}

/// Where a resolved property came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropSource {
    /// The visual's `objects`/`visualContainerObjects`.
    Visual,
    /// Theme `visualStyles[visualType]`.
    ThemeVisualType,
    /// Theme `visualStyles["*"]`.
    ThemeWildcard,
    /// [`defaults::builtin_property`] or a text-class default.
    Builtin,
}

/// A resolved property value (report literal or theme JSON form; decode
/// with [`as_bool`]/[`as_number`]/[`as_text`] or
/// [`ThemeResolver::color`]).
#[derive(Debug, Clone, PartialEq)]
pub struct PropRef<'a> {
    pub value: Cow<'a, Value>,
    pub source: PropSource,
}

/// Which formatting object bag a lookup reads first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectScope {
    /// `visual.objects` (visual-specific formatting).
    Objects,
    /// `visual.visualContainerObjects` (title, background, border, padding,
    /// dropShadow, visualHeader, …).
    Container,
}

/// Resolves colors, text classes and formatting properties against a
/// [`Theme`].
#[derive(Debug, Clone, Default)]
pub struct ThemeResolver {
    theme: Theme,
    /// Content hash of `theme` (render cache key), computed on first use;
    /// the theme never changes after construction.
    fingerprint: OnceLock<Option<u64>>,
}

impl ThemeResolver {
    pub fn new(theme: Theme) -> Self {
        Self {
            theme,
            fingerprint: OnceLock::new(),
        }
    }

    /// Content hash of the merged theme (`None` if it cannot be hashed).
    pub(crate) fn fingerprint(&self) -> Option<u64> {
        *self.fingerprint.get_or_init(|| {
            let Theme {
                json,
                visual_styles,
                palette,
            } = &self.theme;
            let mut fp = crate::fingerprint::Fingerprint::new();
            fp.add(json).add(visual_styles).add(palette);
            fp.finish()
        })
    }

    /// Resolver for a PBIR report's themes.
    pub fn from_pbir(report: &PbirReport) -> Self {
        Self::new(Theme::from_pbir(report))
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// Resolves any PBIR color form: `{"solid":{"color":…}}`, `{"expr":…}`,
    /// literals (`'#C00000'`), `ThemeDataColor{ColorId, Percent}`, bare hex
    /// strings and theme color names (`background`, `good`, …).
    pub fn color(&self, value: &Value) -> Option<Color> {
        pbi_pbir::resolve_color(value, Some(&self.theme.json)).and_then(|hex| Color::parse(&hex))
    }

    /// A named theme color (`foreground`, `background`, `tableAccent`, …).
    pub fn named_color(&self, name: &str) -> Option<Color> {
        self.theme.json.named_color(name).and_then(Color::parse)
    }

    /// `dataColors[i]`, cycling through the palette.
    pub fn data_color(&self, i: usize) -> Color {
        let palette = &self.theme.palette;
        if palette.is_empty() {
            return Color::rgb(0x11, 0x8D, 0xFF);
        }
        palette[i % palette.len()]
    }

    /// Theme `foreground` (`#252423` by default).
    pub fn foreground(&self) -> Color {
        self.named_color("foreground")
            .unwrap_or(Color::rgb(0x25, 0x24, 0x23))
    }

    /// Theme `background` (`#FFFFFF` by default).
    pub fn background(&self) -> Color {
        self.named_color("background").unwrap_or(Color::WHITE)
    }

    /// A text class as a [`TextStyle`] (see the module docs).
    pub fn text_class(&self, name: &str) -> TextStyle {
        let secondary = self
            .named_color("foregroundNeutralSecondary")
            .unwrap_or(Color::rgb(0x60, 0x5E, 0x5C));
        let (primary, size, family, color): (&str, Option<f64>, Option<&str>, Option<Color>) =
            match name {
                "callout" | "title" | "header" | "label" => (name, None, None, None),
                "largeTitle" => ("title", Some(14.0), None, None),
                "semiboldLabel" => ("label", None, Some("Segoe UI Semibold"), None),
                "boldLabel" => ("label", None, Some("Segoe UI Bold"), None),
                "largeLabel" => ("label", Some(12.0), None, None),
                "smallLabel" => ("label", Some(9.0), None, None),
                "lightLabel" => ("label", None, None, Some(secondary)),
                "largeLightLabel" => ("label", Some(12.0), None, Some(secondary)),
                "smallLightLabel" => ("label", Some(9.0), None, Some(secondary)),
                _ => ("label", None, None, None),
            };
        let mut style = self.primary_class(primary);
        if let Some(s) = size {
            style.size_pt = s;
        }
        if let Some(f) = family {
            style.family = f.to_owned();
        }
        if let Some(c) = color {
            style.color = c;
        }
        if primary != name {
            self.apply_class(&mut style, name);
        }
        style
    }

    fn primary_class(&self, name: &str) -> TextStyle {
        let mut style = TextStyle {
            color: self.foreground(),
            ..TextStyle::default()
        };
        self.apply_class(&mut style, name);
        style
    }

    /// Overlays the theme's `textClasses[name]` entries onto `style`.
    fn apply_class(&self, style: &mut TextStyle, name: &str) {
        let Some(class) = self.theme.json.text_class(name) else {
            return;
        };
        if let Some(size) = class.font_size_f64().filter(|s| s.is_finite() && *s > 0.0) {
            style.size_pt = size;
        }
        if let Some(face) = class.font_face.as_deref().filter(|f| !f.trim().is_empty()) {
            style.family = face.to_owned();
        }
        if let Some(color) = class.color.as_deref().and_then(Color::parse) {
            style.color = color;
        }
        if let Some(bold) = class.extra.get("bold").and_then(as_bool) {
            style.weight = if bold { 700 } else { 400 };
        }
        if let Some(italic) = class.extra.get("italic").and_then(as_bool) {
            style.italic = italic;
        }
    }

    /// Property view of a visual (`visualType`, `objects`,
    /// `visualContainerObjects`).
    pub fn visual<'a>(&'a self, visual: &'a Visual) -> VisualProps<'a> {
        let visual_type = if visual.is_group() {
            "group"
        } else {
            visual.visual_type.as_str()
        };
        self.props(visual_type, &visual.objects, &visual.container_objects)
    }

    /// Property view of a page (`visualType` `page`; `background`,
    /// `outspace`, …).
    pub fn page<'a>(&'a self, page: &'a Page) -> VisualProps<'a> {
        self.props("page", &page.objects, &Value::Null)
    }

    /// Property view from raw parts.
    pub fn props<'a>(
        &'a self,
        visual_type: &'a str,
        objects: &'a Value,
        container_objects: &'a Value,
    ) -> VisualProps<'a> {
        VisualProps {
            resolver: self,
            visual_type,
            objects,
            container_objects,
        }
    }
}

/// Formatting-property view of one visual; see the module docs for the
/// precedence.
#[derive(Debug, Clone, Copy)]
pub struct VisualProps<'a> {
    resolver: &'a ThemeResolver,
    visual_type: &'a str,
    objects: &'a Value,
    container_objects: &'a Value,
}

impl<'a> VisualProps<'a> {
    pub fn visual_type(&self) -> &'a str {
        self.visual_type
    }

    pub fn resolver(&self) -> &'a ThemeResolver {
        self.resolver
    }

    /// Lookups that read `visual.objects` first.
    pub fn objects(&self) -> PropLookup<'a> {
        PropLookup {
            props: *self,
            scope: ObjectScope::Objects,
        }
    }

    /// Lookups that read `visual.visualContainerObjects` first.
    pub fn container(&self) -> PropLookup<'a> {
        PropLookup {
            props: *self,
            scope: ObjectScope::Container,
        }
    }

    /// Lookups in `scope`.
    pub fn scope(&self, scope: ObjectScope) -> PropLookup<'a> {
        PropLookup {
            props: *self,
            scope,
        }
    }

    /// `visualContainerObjects.stylePreset.name`, if any.
    fn style_preset(&self) -> Option<String> {
        visual_prop(self.container_objects, "stylePreset", "name")
            .or_else(|| visual_prop(self.objects, "stylePreset", "name"))
            .and_then(as_text)
    }
}

/// Default-selector property of a report object bag.
fn visual_prop<'v>(objects: &'v Value, object: &str, prop: &str) -> Option<&'v Value> {
    pbi_pbir::objects::property(objects, object, prop)
}

fn is_empty_bag(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::Object(m) => m.is_empty(),
        _ => true,
    }
}

/// Property lookups of one [`ObjectScope`].
#[derive(Debug, Clone, Copy)]
pub struct PropLookup<'a> {
    props: VisualProps<'a>,
    scope: ObjectScope,
}

impl<'a> PropLookup<'a> {
    /// The resolved raw value and its source (the first layer that has
    /// the property, decodable or not).
    pub fn get(&self, object: &str, prop: &str) -> Option<PropRef<'a>> {
        self.candidates(object, prop).into_iter().next()
    }

    /// Every layer's value for the property, in precedence order. Typed
    /// getters use the first one that decodes, so an invalid value falls
    /// back to the next layer.
    pub fn candidates(&self, object: &str, prop: &str) -> Vec<PropRef<'a>> {
        let p = &self.props;
        let mut out = Vec::new();
        let mut push = |value: Option<&'a Value>, source: PropSource| {
            if let Some(v) = value {
                out.push(PropRef {
                    value: Cow::Borrowed(v),
                    source,
                });
            }
        };
        match self.scope {
            ObjectScope::Objects => push(visual_prop(p.objects, object, prop), PropSource::Visual),
            ObjectScope::Container => {
                push(
                    visual_prop(p.container_objects, object, prop),
                    PropSource::Visual,
                );
                if is_empty_bag(p.container_objects) {
                    push(visual_prop(p.objects, object, prop), PropSource::Visual);
                }
            }
        }
        let theme = &p.resolver.theme;
        if let Some(preset) = p.style_preset() {
            push(
                theme.style_property(p.visual_type, &preset, object, prop),
                PropSource::ThemeVisualType,
            );
        }
        push(
            theme.style_property(p.visual_type, "*", object, prop),
            PropSource::ThemeVisualType,
        );
        push(
            theme.style_property("*", "*", object, prop),
            PropSource::ThemeWildcard,
        );
        if let Some(v) = self.builtin(object, prop) {
            out.push(PropRef {
                value: Cow::Owned(v),
                source: PropSource::Builtin,
            });
        }
        out
    }

    fn builtin(&self, object: &str, prop: &str) -> Option<Value> {
        let r = self.props.resolver;
        if object == "title" || object == "subTitle" {
            let class = r.text_class(if object == "title" { "title" } else { "label" });
            match prop {
                "fontColor" => return Some(Value::String(class.color.to_hex())),
                "fontSize" => {
                    return serde_json::Number::from_f64(class.size_pt).map(Value::Number);
                }
                "fontFamily" => return Some(Value::String(class.family)),
                _ => {}
            }
        }
        builtin_property(self.props.visual_type, object, prop)
    }

    /// The raw value, without the source.
    pub fn value(&self, object: &str, prop: &str) -> Option<Cow<'a, Value>> {
        self.get(object, prop).map(|r| r.value)
    }

    pub fn bool(&self, object: &str, prop: &str) -> Option<bool> {
        self.candidates(object, prop)
            .iter()
            .find_map(|r| as_bool(&r.value))
    }

    pub fn number(&self, object: &str, prop: &str) -> Option<f64> {
        self.candidates(object, prop)
            .iter()
            .find_map(|r| as_number(&r.value))
    }

    pub fn text(&self, object: &str, prop: &str) -> Option<String> {
        self.candidates(object, prop)
            .iter()
            .find_map(|r| as_text(&r.value))
    }

    pub fn color(&self, object: &str, prop: &str) -> Option<Color> {
        let resolver = self.props.resolver;
        self.candidates(object, prop)
            .iter()
            .find_map(|r| resolver.color(&r.value))
    }

    /// A text style from an object's font properties (`fontFamily`,
    /// `fontSize`, `fontColor` or `color`, `bold`, `italic`, `underline`)
    /// over the text class `class`.
    pub fn text_style(&self, object: &str, class: &str) -> TextStyle {
        let mut style = self.props.resolver.text_class(class);
        if let Some(family) = self
            .text(object, "fontFamily")
            .filter(|f| !f.trim().is_empty())
        {
            style.family = family;
        }
        if let Some(size) = self.number(object, "fontSize").filter(|s| *s > 0.0) {
            style.size_pt = size;
        }
        if let Some(color) = self
            .color(object, "fontColor")
            .or_else(|| self.color(object, "color"))
        {
            style.color = color;
        }
        if let Some(bold) = self.bool(object, "bold") {
            style.weight = if bold { 700 } else { 400 };
        }
        if let Some(italic) = self.bool(object, "italic") {
            style.italic = italic;
        }
        if let Some(underline) = self.bool(object, "underline") {
            style.underline = underline;
        }
        style
    }

    /// `top`/`right`/`bottom`/`left` of `object` (e.g. `padding`), each
    /// clamped to ≥ 0.
    pub fn insets(&self, object: &str) -> Insets {
        let side = |p: &str| self.number(object, p).unwrap_or(0.0).max(0.0);
        Insets {
            top: side("top"),
            right: side("right"),
            bottom: side("bottom"),
            left: side("left"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn lit(raw: &str) -> Value {
        json!({"expr": {"Literal": {"Value": raw}}})
    }

    #[test]
    fn custom_theme_overrides_base_and_builtin() {
        let base = json!({
            "name": "base",
            "dataColors": ["#111111", "#222222"],
            "textClasses": {"title": {"fontSize": 13}},
            "visualStyles": {"*": {"*": {"title": [{"titleWrap": true, "fontSize": 11}]}}}
        });
        let custom = json!({
            "name": "custom",
            "dataColors": ["#AA0000"],
            "visualStyles": {"*": {"*": {"title": [{"fontSize": 15}]}}}
        });
        let theme = Theme::from_values(Some(&base), Some(&custom));
        assert_eq!(theme.json().name.as_deref(), Some("custom"));
        let r = ThemeResolver::new(theme);
        assert_eq!(r.data_color(0), Color::rgb(0xAA, 0, 0));
        assert_eq!(r.data_color(5), Color::rgb(0xAA, 0, 0));
        let title = r.text_class("title");
        assert_eq!((title.size_pt, title.family.as_str()), (13.0, "DIN"));
        let props = r.props("lineChart", &Value::Null, &Value::Null);
        assert_eq!(props.container().number("title", "fontSize"), Some(15.0));
        assert_eq!(props.container().bool("title", "titleWrap"), Some(true));
    }

    #[test]
    fn invalid_layers_are_dropped() {
        let bad = json!({"dataColors": "not a list"});
        let theme = Theme::from_values(None, Some(&bad));
        assert_eq!(
            ThemeResolver::new(theme).data_color(0),
            Color::rgb(0x11, 0x8D, 0xFF)
        );
        let r = ThemeResolver::new(Theme::from_values(Some(&json!(42)), None));
        assert_eq!(r.foreground(), Color::rgb(0x25, 0x24, 0x23));
    }

    #[test]
    fn precedence_visual_then_type_then_wildcard_then_builtin() {
        let theme = json!({
            "visualStyles": {
                "*": {"*": {"border": [{"width": 1}], "*": [{"wordWrap": true}]}},
                "shape": {"*": {"background": [{"show": false}]}},
                "cardVisual": {"*": {"padding": [{"paddingUniform": 12}, {"$id": "x", "top": 99}]}}
            }
        });
        let r = ThemeResolver::new(Theme::from_values(Some(&theme), None));
        let objects = json!({"fill": [{"properties": {"fillColor": {"solid": {"color": lit("'#C00000'")}}}}]});
        let container =
            json!({"border": [{"properties": {"show": lit("true"), "width": lit("3D")}}]});
        let shape = r.props("shape", &objects, &container);
        let border_width = shape.container().get("border", "width").unwrap();
        assert_eq!(border_width.source, PropSource::Visual);
        assert_eq!(as_number(&border_width.value), Some(3.0));
        let bg = shape.container().get("background", "show").unwrap();
        assert_eq!(
            (as_bool(&bg.value), bg.source),
            (Some(false), PropSource::ThemeVisualType)
        );
        let card = r.props("cardVisual", &Value::Null, &Value::Null);
        assert_eq!(
            card.container().get("border", "width").map(|p| p.source),
            Some(PropSource::ThemeWildcard)
        );
        assert_eq!(card.container().bool("anything", "wordWrap"), Some(true));
        assert_eq!(card.objects().number("padding", "top"), Some(5.0));
        assert_eq!(
            card.container().get("border", "color").map(|p| p.source),
            Some(PropSource::Builtin)
        );
        assert_eq!(
            shape.objects().color("fill", "fillColor"),
            Some(Color::rgb(0xC0, 0, 0))
        );
        // Container scope does not read visual objects when the container bag exists.
        assert_eq!(shape.container().color("fill", "fillColor"), None);
        let legacy = r.props("shape", &objects, &Value::Null);
        assert_eq!(
            legacy.container().color("fill", "fillColor"),
            Some(Color::rgb(0xC0, 0, 0))
        );
    }

    #[test]
    fn text_classes_derive_secondary_styles() {
        let r = ThemeResolver::default();
        let label = r.text_class("label");
        assert_eq!((label.family.as_str(), label.size_pt), ("Segoe UI", 10.0));
        assert_eq!(r.text_class("largeTitle").size_pt, 14.0);
        assert_eq!(r.text_class("largeTitle").family, "DIN");
        assert_eq!(r.text_class("semiboldLabel").family, "Segoe UI Semibold");
        assert_eq!(r.text_class("smallLightLabel").size_pt, 9.0);
        assert_eq!(
            r.text_class("lightLabel").color,
            Color::rgb(0x60, 0x5E, 0x5C)
        );
        assert_eq!(r.text_class("callout").size_pt, 45.0);
        assert_eq!(r.text_class("nope"), label);
    }

    #[test]
    fn colors_resolve_in_all_forms() {
        let r = ThemeResolver::default();
        let tdc = json!({"solid": {"color": {"expr": {"ThemeDataColor": {"ColorId": 2, "Percent": 0.6}}}}});
        assert_eq!(r.color(&tdc), Some(Color::rgb(160, 209, 255)));
        assert_eq!(r.color(&json!("background")), Some(Color::WHITE));
        assert_eq!(
            r.color(&json!({"solid": {"color": "#ffffff"}})),
            Some(Color::WHITE)
        );
        assert_eq!(
            r.color(&lit("'#7030A0'")),
            Some(Color::rgb(0x70, 0x30, 0xA0))
        );
        assert_eq!(r.color(&json!({"weird": 1})), None);
    }
}
