//! Retained scene graph: the target of every visual and the single source for
//! SVG ([`crate::svg`]) and PNG ([`crate::png`]) output.
//!
//! A [`Scene`] is a canvas size plus a root [`Group`]. Groups carry an
//! optional transform, clip, opacity, CSS class and [`HitId`]; leaves are
//! rectangles, paths, lines, text runs and (data-URI) images. Coordinates
//! are CSS pixels; text sizes are points ([`TextStyle::size_pt`]) converted
//! with `px = pt · 96/72`.
//!
//! Text nodes hold one already-laid-out line each (`y` is the baseline):
//! visuals measure and wrap with [`crate::text::FontBook`] first, then emit
//! one [`TextNode`] per line, so the SVG and PNG show exactly the measured
//! layout.
//!
//! Everything is `serde`-serializable (camelCase, `type`-tagged nodes) so
//! the app can receive scenes or [`HitRegion`]s over IPC.

mod geom;
mod hit;
mod path;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub use geom::{Insets, Rect, Transform};
pub use hit::{HitId, HitKind, HitRegion};
pub use path::{PathBuilder, PathData, PathSeg, format_number};

use crate::text::TextStyle;

/// An opaque sRGB color. Serializes as `#RRGGBB`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const WHITE: Color = Color::rgb(255, 255, 255);
    pub const BLACK: Color = Color::rgb(0, 0, 0);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Parses `#RGB`, `#RRGGBB` or `#RRGGBBAA` (alpha ignored); `#` optional.
    pub fn parse(text: &str) -> Option<Self> {
        pbi_pbir::theme::Rgb::parse_hex(text).map(|c| Self::rgb(c.0, c.1, c.2))
    }

    /// `#RRGGBB` (uppercase).
    pub fn to_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }

    /// Power BI shading: `percent < 0` darkens toward black, `> 0` lightens
    /// toward white (see [`pbi_pbir::theme`]).
    pub fn shade(self, percent: f64) -> Self {
        let c = pbi_pbir::theme::Rgb(self.r, self.g, self.b).shade(percent);
        Self::rgb(c.0, c.1, c.2)
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Color::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("invalid color {s:?}")))
    }
}

fn one() -> f64 {
    1.0
}

fn is_one(v: &f64) -> bool {
    *v == 1.0
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

/// Clamps to `[0, 1]`; NaN becomes 1.
pub(crate) fn clamp_unit(v: f64) -> f64 {
    if v.is_nan() { 1.0 } else { v.clamp(0.0, 1.0) }
}

/// A solid fill. A fill with opacity 0 is invisible but still hit-testable
/// in the SVG (use `None` for no fill).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Paint {
    pub color: Color,
    /// 0 (invisible) to 1 (opaque).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub opacity: f64,
}

impl Paint {
    pub const fn solid(color: Color) -> Self {
        Self {
            color,
            opacity: 1.0,
        }
    }

    pub fn new(color: Color, opacity: f64) -> Self {
        Self {
            color,
            opacity: clamp_unit(opacity),
        }
    }

    /// From a Power BI `transparency` percentage (0 opaque … 100 invisible).
    pub fn with_transparency(color: Color, transparency_pct: f64) -> Self {
        Self::new(color, 1.0 - transparency_pct / 100.0)
    }

    pub fn is_visible(&self) -> bool {
        self.opacity > 0.0
    }
}

/// A stroke (outline).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stroke {
    pub color: Color,
    pub width: f64,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub opacity: f64,
    /// Dash pattern (`stroke-dasharray`); empty for a solid line.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dash: Vec<f64>,
}

impl Stroke {
    pub fn new(color: Color, width: f64) -> Self {
        Self {
            color,
            width,
            opacity: 1.0,
            dash: Vec::new(),
        }
    }

    pub fn with_opacity(mut self, opacity: f64) -> Self {
        self.opacity = clamp_unit(opacity);
        self
    }

    pub fn with_dash(mut self, dash: Vec<f64>) -> Self {
        self.dash = dash;
        self
    }

    pub fn is_visible(&self) -> bool {
        self.width > 0.0 && self.width.is_finite() && self.opacity > 0.0
    }
}

/// Horizontal anchor of a text run relative to its `x`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextAnchor {
    #[default]
    Start,
    Middle,
    End,
}

/// How an image fills its rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageFit {
    /// Stretch to the rectangle.
    Fill,
    /// Scale to fit, centred (`xMidYMid meet`).
    #[default]
    Contain,
    /// Scale to cover, centred and clipped (`xMidYMid slice`).
    Cover,
}

/// A (rounded) rectangle clip.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub rect: Rect,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub radius: f64,
}

impl Clip {
    pub fn rect(rect: Rect) -> Self {
        Self { rect, radius: 0.0 }
    }

    pub fn rounded(rect: Rect, radius: f64) -> Self {
        Self { rect, radius }
    }
}

/// A (rounded) rectangle.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RectNode {
    pub rect: Rect,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub radius: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Paint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Stroke>,
}

impl RectNode {
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            ..Self::default()
        }
    }

    pub fn fill(mut self, paint: Paint) -> Self {
        self.fill = Some(paint);
        self
    }

    pub fn stroke(mut self, stroke: Stroke) -> Self {
        self.stroke = Some(stroke);
        self
    }

    pub fn radius(mut self, radius: f64) -> Self {
        self.radius = radius;
        self
    }
}

/// An arbitrary path.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathNode {
    pub d: PathData,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Paint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Stroke>,
}

impl PathNode {
    pub fn new(d: PathData) -> Self {
        Self {
            d,
            ..Self::default()
        }
    }

    pub fn fill(mut self, paint: Paint) -> Self {
        self.fill = Some(paint);
        self
    }

    pub fn stroke(mut self, stroke: Stroke) -> Self {
        self.stroke = Some(stroke);
        self
    }
}

/// A straight line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineNode {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    pub stroke: Stroke,
}

/// One line of text. `y` is the baseline; `x` is interpreted per `anchor`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextNode {
    pub x: f64,
    pub y: f64,
    pub text: String,
    pub style: TextStyle,
    #[serde(default)]
    pub anchor: TextAnchor,
    /// Clip rectangle (e.g. the cell or content box).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<Rect>,
}

impl TextNode {
    pub fn new(x: f64, y: f64, text: impl Into<String>, style: TextStyle) -> Self {
        Self {
            x,
            y,
            text: text.into(),
            style,
            anchor: TextAnchor::Start,
            clip: None,
        }
    }

    pub fn anchor(mut self, anchor: TextAnchor) -> Self {
        self.anchor = anchor;
        self
    }

    pub fn clip(mut self, clip: Rect) -> Self {
        self.clip = Some(clip);
        self
    }
}

/// A raster or SVG image. Only `data:` URIs are emitted (no external
/// fetches); other hrefs are dropped by the SVG writer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageNode {
    pub rect: Rect,
    pub href: String,
    #[serde(default)]
    pub fit: ImageFit,
}

/// A container node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<Transform>,
    /// Clip in the group's own (transformed) coordinates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<Clip>,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub opacity: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<HitId>,
    /// CSS class for the app (e.g. `pbi-visual`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
}

impl Default for Group {
    fn default() -> Self {
        Self {
            transform: None,
            clip: None,
            opacity: 1.0,
            id: None,
            class: None,
            children: Vec::new(),
        }
    }
}

impl Group {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_transform(mut self, transform: Transform) -> Self {
        self.transform = (!transform.is_identity()).then_some(transform);
        self
    }

    pub fn with_clip(mut self, clip: Clip) -> Self {
        self.clip = Some(clip);
        self
    }

    pub fn with_opacity(mut self, opacity: f64) -> Self {
        self.opacity = clamp_unit(opacity);
        self
    }

    pub fn with_id(mut self, id: HitId) -> Self {
        self.id = Some(id);
        self
    }

    pub fn with_class(mut self, class: impl Into<String>) -> Self {
        self.class = Some(class.into());
        self
    }

    /// Appends a child node.
    pub fn push(&mut self, node: impl Into<Node>) -> &mut Self {
        self.children.push(node.into());
        self
    }

    /// Appends a rectangle.
    pub fn rect(&mut self, rect: Rect, fill: Option<Paint>, stroke: Option<Stroke>) -> &mut Self {
        self.push(RectNode {
            rect,
            radius: 0.0,
            fill,
            stroke,
        })
    }

    /// Appends a line.
    pub fn line(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, stroke: Stroke) -> &mut Self {
        self.push(LineNode {
            x1,
            y1,
            x2,
            y2,
            stroke,
        })
    }

    /// Appends a single text line (`y` = baseline).
    pub fn text(
        &mut self,
        x: f64,
        y: f64,
        text: impl Into<String>,
        style: TextStyle,
        anchor: TextAnchor,
    ) -> &mut Self {
        self.push(TextNode::new(x, y, text, style).anchor(anchor))
    }

    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }
}

/// A scene node. JSON: the node's fields plus `"type"` (`rect`, `path`,
/// `line`, `text`, `image`, `group`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Node {
    Rect(RectNode),
    Path(PathNode),
    Line(LineNode),
    Text(TextNode),
    Image(ImageNode),
    Group(Group),
}

macro_rules! node_from {
    ($($ty:ident => $variant:ident),*) => {
        $(impl From<$ty> for Node {
            fn from(n: $ty) -> Self {
                Node::$variant(n)
            }
        })*
    };
}

node_from!(
    RectNode => Rect,
    PathNode => Path,
    LineNode => Line,
    TextNode => Text,
    ImageNode => Image,
    Group => Group
);

// Deserialized through `serde_json::Value`: serde's buffered internally
// tagged enums cannot read numbers under serde_json's `arbitrary_precision`.
impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        const VARIANTS: &[&str] = &["rect", "path", "line", "text", "image", "group"];
        let mut value = serde_json::Value::deserialize(deserializer)?;
        let kind = value
            .as_object_mut()
            .and_then(|m| m.remove("type"))
            .and_then(|t| t.as_str().map(str::to_owned))
            .ok_or_else(|| D::Error::missing_field("type"))?;
        let node = match kind.as_str() {
            "rect" => serde_json::from_value(value).map(Node::Rect),
            "path" => serde_json::from_value(value).map(Node::Path),
            "line" => serde_json::from_value(value).map(Node::Line),
            "text" => serde_json::from_value(value).map(Node::Text),
            "image" => serde_json::from_value(value).map(Node::Image),
            "group" => serde_json::from_value(value).map(Node::Group),
            other => return Err(D::Error::unknown_variant(other, VARIANTS)),
        };
        node.map_err(D::Error::custom)
    }
}

/// A canvas of `width` × `height` CSS pixels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scene {
    pub width: f64,
    pub height: f64,
    pub root: Group,
}

impl Scene {
    pub fn new(width: f64, height: f64) -> Self {
        Self {
            width,
            height,
            root: Group::new(),
        }
    }

    /// Every group with a [`HitId`], in paint order (hit-test from the end),
    /// with its canvas-space bounding box (clips applied). Text without a
    /// clip does not contribute to a group's bounds.
    pub fn hit_regions(&self) -> Vec<HitRegion> {
        let mut out = Vec::new();
        collect_hits(&self.root, &Transform::IDENTITY, None, &mut out);
        out
    }
}

fn own_transform(group: &Group, parent: &Transform) -> Transform {
    group.transform.map_or(*parent, |own| parent.then(&own))
}

/// The group's effective clip in canvas space (its own clip intersected with
/// the inherited one).
fn effective_clip(group: &Group, t: &Transform, parent_clip: Option<Rect>) -> Option<Rect> {
    let own = group.clip.map(|c| t.apply_rect(&c.rect));
    match (own, parent_clip) {
        (Some(a), Some(b)) => Some(a.intersect(&b).unwrap_or(Rect::new(a.x, a.y, 0.0, 0.0))),
        (a, b) => a.or(b),
    }
}

fn collect_hits(
    group: &Group,
    parent: &Transform,
    parent_clip: Option<Rect>,
    out: &mut Vec<HitRegion>,
) {
    let t = own_transform(group, parent);
    let clip = effective_clip(group, &t, parent_clip);
    if let Some(id) = &group.id
        && let Some(rect) = group_bounds(group, parent, parent_clip)
    {
        out.push(HitRegion {
            id: id.clone(),
            rect,
        });
    }
    for child in &group.children {
        if let Node::Group(g) = child {
            collect_hits(g, &t, clip, out);
        }
    }
}

/// Canvas-space bounds of a group drawn under the `parent` transform.
fn group_bounds(group: &Group, parent: &Transform, parent_clip: Option<Rect>) -> Option<Rect> {
    let t = own_transform(group, parent);
    let clip = effective_clip(group, &t, parent_clip);
    let mut acc: Option<Rect> = None;
    for child in &group.children {
        if let Some(b) = node_bounds(child, &t, clip) {
            acc = Some(acc.map_or(b, |a| a.union(&b)));
        }
    }
    let acc = acc.or(group.clip.map(|c| t.apply_rect(&c.rect)))?;
    match clip {
        Some(c) => acc.intersect(&c),
        None => Some(acc),
    }
}

fn node_bounds(node: &Node, t: &Transform, clip: Option<Rect>) -> Option<Rect> {
    let local = match node {
        Node::Rect(r) => {
            let half = r
                .stroke
                .as_ref()
                .filter(|s| s.is_visible())
                .map_or(0.0, |s| s.width / 2.0);
            Some(r.rect.inset_uniform(-half))
        }
        Node::Path(p) => p.d.bounds(),
        Node::Line(l) => Some(Rect::from_points(l.x1, l.y1, l.x2, l.y2)),
        Node::Text(tx) => tx.clip,
        Node::Image(i) => Some(i.rect),
        Node::Group(g) => return group_bounds(g, t, clip),
    }?;
    Some(t.apply_rect(&local))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_serialize_as_hex() {
        let c = Color::parse("c00000").unwrap();
        assert_eq!(c, Color::rgb(0xC0, 0, 0));
        assert_eq!(serde_json::to_string(&c).unwrap(), "\"#C00000\"");
        assert_eq!(Color::WHITE.shade(-0.6).to_hex(), "#666666");
        assert!(serde_json::from_str::<Color>("\"nope\"").is_err());
        assert_eq!(Paint::with_transparency(c, 25.0).opacity, 0.75);
        assert_eq!(Paint::new(c, f64::NAN).opacity, 1.0);
    }

    #[test]
    fn scenes_round_trip_through_json() {
        let mut scene = Scene::new(100.0, 50.0);
        let mut g = Group::new()
            .with_transform(Transform::translate(10.0, 10.0))
            .with_id(HitId::visual("v1"))
            .with_class("pbi-visual");
        g.rect(
            Rect::new(0.0, 0.0, 20.0, 10.0),
            Some(Paint::solid(Color::BLACK)),
            None,
        );
        g.push(PathNode::new(PathData::pie_slice(
            5.0, 5.0, 5.0, 0.0, 0.0, 90.0,
        )));
        g.text(0.0, 8.0, "Hi", TextStyle::default(), TextAnchor::Middle);
        scene.root.push(g);
        let json = serde_json::to_string(&scene).unwrap();
        assert!(json.contains(r#""type":"rect""#), "{json}");
        let back: Scene = serde_json::from_str(&json).unwrap();
        assert_eq!(back, scene);
        assert!(serde_json::from_str::<Node>(r#"{"type":"blob"}"#).is_err());
        assert!(serde_json::from_str::<Node>(r#"{"x":1}"#).is_err());
        assert!(serde_json::from_str::<Node>(r#"{"type":"path","d":"M1"}"#).is_err());
    }

    #[test]
    fn hit_regions_follow_transforms_and_clips() {
        let mut scene = Scene::new(200.0, 200.0);
        let mut outer = Group::new()
            .with_transform(Transform::translate(100.0, 0.0))
            .with_id(HitId::visual("v"));
        outer.rect(Rect::new(0.0, 0.0, 50.0, 50.0), None, None);
        let mut inner = Group::new()
            .with_clip(Clip::rect(Rect::new(0.0, 0.0, 10.0, 10.0)))
            .with_id(HitId::data_point("v", 0, 1));
        inner.rect(Rect::new(0.0, 0.0, 40.0, 40.0), None, None);
        outer.push(inner);
        scene.root.push(outer);
        let hits = scene.hit_regions();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].rect, Rect::new(100.0, 0.0, 50.0, 50.0));
        assert_eq!(hits[1].rect, Rect::new(100.0, 0.0, 10.0, 10.0));
    }
}
