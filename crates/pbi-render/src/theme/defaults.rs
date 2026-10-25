//! Built-in Power BI defaults: the fallback theme (used under any report
//! theme) and the last-resort formatting property table.
//!
//! # Property defaults
//!
//! Used when neither the visual's objects nor the theme's `visualStyles`
//! set a property ([`builtin_property`]). Values were chosen to match how
//! Power BI draws visuals (e.g. a shown border is 1px black with square
//! corners; content starts 6px inside a 1px border, i.e. 5px padding).
//!
//! | Object | Property | Default |
//! |---|---|---|
//! | `title` | `show` | `true` for charts and unknown/custom visuals; `false` for `textbox`, `shape`, `basicShape`, `image`, `actionButton`, slicers, cards, `multiRowCard`, `tableEx`, `pivotTable`, navigators, groups |
//! | `title` | `text` | (auto title from projections, see `layout`) |
//! | `title` | `fontColor` / `fontSize` / `fontFamily` | `title` text class (theme; built-in `#252423`, 12pt, `DIN`) |
//! | `title` | `alignment` | `left` |
//! | `title` | `titleWrap` | `false` |
//! | `title` | `bold` / `italic` / `underline` | `false` |
//! | `title` | `background` | none |
//! | `subTitle` | `show` | `false` |
//! | `divider` | `show` | `false` |
//! | `background` | `show` | `true`; `false` for `shape`, `basicShape`, `actionButton`, `group`, `pageNavigator`, `bookmarkNavigator` |
//! | `background` | `color` | theme `background` (`#FFFFFF`) |
//! | `background` | `transparency` | `0` |
//! | `border` | `show` | `false` |
//! | `border` | `color` | `#000000` |
//! | `border` | `width` | `1` |
//! | `border` | `radius` | `0` |
//! | `padding` | `top` / `right` / `bottom` / `left` | `5` |
//! | `dropShadow` | `show` | `false` |
//! | `dropShadow` | `color` | `#000000` |
//! | `dropShadow` | `transparency` | `70` |
//! | `dropShadow` | `position` / `preset` | `Outer` / `BottomRight` |
//! | `dropShadow` | `angle` / `shadowDistance` / `shadowBlur` / `shadowSpread` | `45` / `10` / `10` / `3` |
//! | `visualHeader` | `show` | `true`; `false` for `textbox`, `shape`, `basicShape`, `actionButton`, `image` (not drawn in static renders) |
//! | page `background` | `color` / `transparency` | `#FFFFFF` / `100` |
//! | page `outspace` | `color` / `transparency` | `#FFFFFF` / `0` |

use serde_json::{Value, json};

/// Visual types whose container title is hidden by default.
const NO_TITLE_TYPES: &[&str] = &[
    "textbox",
    "shape",
    "basicShape",
    "image",
    "actionButton",
    "slicer",
    "advancedSlicerVisual",
    "listSlicer",
    "textSlicer",
    "card",
    "cardVisual",
    "multiRowCard",
    "tableEx",
    "pivotTable",
    "pageNavigator",
    "bookmarkNavigator",
    "group",
    "page",
];

/// Visual types whose container background is off by default.
const NO_BACKGROUND_TYPES: &[&str] = &[
    "shape",
    "basicShape",
    "actionButton",
    "group",
    "pageNavigator",
    "bookmarkNavigator",
];

/// Visual types without a visual header by default.
const NO_HEADER_TYPES: &[&str] = &["textbox", "shape", "basicShape", "actionButton", "image"];

/// `true` if `visual_type` shows its container title unless told otherwise.
pub fn title_shown_by_default(visual_type: &str) -> bool {
    !NO_TITLE_TYPES.contains(&visual_type)
}

/// Last-resort default of a formatting property, in theme (bare JSON) form.
pub fn builtin_property(visual_type: &str, object: &str, prop: &str) -> Option<Value> {
    let is_page = visual_type == "page";
    let v = match (object, prop) {
        ("background", "color") if is_page => json!("#FFFFFF"),
        ("background", "transparency") if is_page => json!(100),
        ("outspace", "color") => json!("#FFFFFF"),
        ("outspace", "transparency") => json!(0),
        ("title", "show") => json!(title_shown_by_default(visual_type)),
        ("title", "alignment") => json!("left"),
        ("title", "titleWrap") => json!(false),
        ("title" | "subTitle", "bold" | "italic" | "underline") => json!(false),
        ("subTitle", "show") | ("divider", "show") => json!(false),
        ("background", "show") => json!(!NO_BACKGROUND_TYPES.contains(&visual_type)),
        ("background", "color") => json!("background"),
        ("background", "transparency") => json!(0),
        ("border", "show") => json!(false),
        ("border", "color") => json!("#000000"),
        ("border", "width") => json!(1),
        ("border", "radius") => json!(0),
        ("padding", "top" | "right" | "bottom" | "left") => json!(5),
        ("dropShadow", "show") => json!(false),
        ("dropShadow", "color") => json!("#000000"),
        ("dropShadow", "transparency") => json!(70),
        ("dropShadow", "position") => json!("Outer"),
        ("dropShadow", "preset") => json!("BottomRight"),
        ("dropShadow", "angle") => json!(45),
        ("dropShadow", "shadowDistance") => json!(10),
        ("dropShadow", "shadowBlur") => json!(10),
        ("dropShadow", "shadowSpread") => json!(3),
        ("visualHeader", "show") => json!(!NO_HEADER_TYPES.contains(&visual_type)),
        _ => return None,
    };
    Some(v)
}

/// The fallback theme layered under every report theme: Power BI's default
/// palette, structural colors and primary text classes.
pub fn builtin_theme() -> Value {
    json!({
        "name": "dashcraft-builtin",
        "dataColors": [
            "#118DFF", "#12239E", "#E66C37", "#6B007B", "#E044A7", "#744EC2", "#D9B300",
            "#D64550", "#197278", "#1AAB40", "#15C6F4", "#4092FF", "#FFA058", "#BE5DC9",
            "#F472D0", "#B5A1FF", "#C4A200", "#FF8080", "#00DBBC", "#5BD667", "#0091D5",
            "#4668C5", "#FF6300", "#99008A", "#EC008C", "#533285", "#99700A", "#FF4141",
            "#1F9A85", "#25891C", "#0057A2", "#002050", "#C94F0F", "#450F54", "#B60064",
            "#34124F", "#6A5A29", "#1AAB40", "#BA141A", "#0C3D37", "#0B511F"
        ],
        "foreground": "#252423",
        "foregroundNeutralSecondary": "#605E5C",
        "foregroundNeutralTertiary": "#B3B0AD",
        "background": "#FFFFFF",
        "backgroundLight": "#F3F2F1",
        "backgroundNeutral": "#C8C6C4",
        "tableAccent": "#118DFF",
        "good": "#1AAB40",
        "neutral": "#D9B300",
        "bad": "#D64554",
        "maximum": "#118DFF",
        "center": "#D9B300",
        "minimum": "#DEEFFF",
        "null": "#FF7F48",
        "hyperlink": "#0078D4",
        "visitedHyperlink": "#0078D4",
        "textClasses": {
            "callout": {"fontSize": 45, "fontFace": "DIN", "color": "#252423"},
            "title": {"fontSize": 12, "fontFace": "DIN", "color": "#252423"},
            "header": {"fontSize": 12, "fontFace": "Segoe UI Semibold", "color": "#252423"},
            "label": {"fontSize": 10, "fontFace": "Segoe UI", "color": "#252423"}
        }
    })
}
