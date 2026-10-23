//! Scene graph → SVG (deterministic).
//!
//! - Numbers are rounded to 3 decimals without trailing zeros
//!   ([`format_number`]); attributes are sorted by name; clip paths get
//!   sequential ids (`c0`, `c1`, …) in document order. The same scene always
//!   produces the same bytes.
//! - Text and attribute values are XML-escaped; characters XML 1.0 forbids
//!   (most C0 controls, U+FFFE/U+FFFF) are dropped. Text keeps its spaces
//!   (`xml:space="preserve"`).
//! - Groups with a [`HitId`](crate::scene::HitId) get `data-hit` (see
//!   [`HitId::encode`](crate::scene::HitId::encode)) and their `class`.
//! - Text uses `font-family="Selawik, 'Segoe UI', sans-serif"`
//!   ([`FONT_FAMILY_ATTR`]), or `font-family="Carlito, Calibri, sans-serif"`
//!   ([`CARLITO_FONT_FAMILY_ATTR`]) for Calibri, with the CSS weight and
//!   style of the bundled face ([`crate::text::resolve_face_with`]); with
//!   [`SvgOptions::font_face_css`] a `<style>` block declares `@font-face`
//!   rules for the faces used, embedding the TTFs as base64 when
//!   [`SvgOptions::embed_fonts`] is set (so a webview renders exactly what
//!   was measured) or pointing at installed fonts (`local()`) otherwise.
//! - Images are emitted only for `data:image/…` URIs.
//! - A fill with opacity 0 is written as `fill-opacity="0"` (invisible but
//!   hit-testable), not `fill="none"`.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use crate::scene::{
    Clip, Group, ImageFit, Node, Paint, PathData, Rect, Scene, Stroke, TextAnchor, TextNode,
    Transform, format_number,
};
use crate::text::BundledFace;

/// `font-family` attribute value for text drawn with a Selawik face.
pub const FONT_FAMILY_ATTR: &str = "Selawik, 'Segoe UI', sans-serif";
/// `font-family` attribute value for text drawn with a Carlito face
/// (Calibri).
pub const CARLITO_FONT_FAMILY_ATTR: &str = "Carlito, Calibri, sans-serif";

/// `font-family` attribute value for text drawn with `face`.
pub fn font_family_attr(face: BundledFace) -> &'static str {
    if face.is_carlito() {
        CARLITO_FONT_FAMILY_ATTR
    } else {
        FONT_FAMILY_ATTR
    }
}

/// Options for [`to_svg`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SvgOptions {
    /// Embed the used bundled fonts as base64 `@font-face` sources (implies
    /// [`SvgOptions::font_face_css`]).
    pub embed_fonts: bool,
    /// Emit a `<style>` block with `@font-face` rules for the used faces.
    pub font_face_css: bool,
}

/// Serializes a scene to an SVG document.
pub fn to_svg(scene: &Scene, opts: &SvgOptions) -> String {
    let mut w = Writer::default();
    w.group_children(&scene.root);
    let width = format_number(scene.width.max(0.0));
    let height = format_number(scene.height.max(0.0));
    let mut out = String::with_capacity(w.body.len() + 256);
    let mut root = Attrs::default();
    root.push("xmlns", "http://www.w3.org/2000/svg");
    root.push("width", &width);
    root.push("height", &height);
    root.push("viewBox", format!("0 0 {width} {height}"));
    out.push_str("<svg");
    root.write(&mut out);
    out.push_str(">\n");
    let css = (opts.font_face_css || opts.embed_fonts)
        .then(|| font_face_css(&w.faces, opts.embed_fonts))
        .filter(|css| !css.is_empty());
    if css.is_some() || !w.defs.is_empty() {
        out.push_str("<defs>\n");
        if let Some(css) = css {
            out.push_str("<style>");
            out.push_str(&css);
            out.push_str("</style>\n");
        }
        out.push_str(&w.defs);
        out.push_str("</defs>\n");
    }
    out.push_str(&w.body);
    out.push_str("</svg>\n");
    out
}

/// `@font-face` rules for `faces` (each with its family, CSS weight and
/// style). With `embed`, sources are base64 `data:` URIs; otherwise
/// `local()` names of the installed fonts.
pub fn font_face_css(faces: &BTreeSet<BundledFace>, embed: bool) -> String {
    let mut css = String::new();
    for face in faces {
        let src = if embed {
            format!(
                "url(data:font/ttf;base64,{}) format(\"truetype\")",
                base64(face.data())
            )
        } else {
            format!("local(\"{}\")", face.full_name())
        };
        let _ = write!(
            css,
            "@font-face{{font-family:\"{}\";font-style:{};font-weight:{};src:{src};}}",
            face.family(),
            if face.is_italic() { "italic" } else { "normal" },
            face.weight()
        );
    }
    css
}

/// `@font-face` rules for every bundled face, for the app to inject once.
pub fn all_font_faces_css(embed: bool) -> String {
    font_face_css(&BundledFace::ALL.into_iter().collect(), embed)
}

/// Sorted attribute list.
#[derive(Default)]
struct Attrs(Vec<(&'static str, String)>);

impl Attrs {
    fn push(&mut self, name: &'static str, value: impl Into<String>) {
        self.0.push((name, value.into()));
    }

    fn num(&mut self, name: &'static str, value: f64) {
        self.push(name, format_number(value));
    }

    fn write(mut self, out: &mut String) {
        self.0.sort_by(|a, b| a.0.cmp(b.0));
        for (k, v) in self.0 {
            out.push(' ');
            out.push_str(k);
            out.push_str("=\"");
            escape_into(out, &v, true);
            out.push('"');
        }
    }
}

#[derive(Default)]
struct Writer {
    body: String,
    defs: String,
    next_clip: usize,
    faces: BTreeSet<BundledFace>,
}

impl Writer {
    fn clip_id(&mut self, clip: &Clip) -> String {
        let id = format!("c{}", self.next_clip);
        self.next_clip += 1;
        let mut a = Attrs::default();
        rect_attrs(&mut a, &clip.rect, clip.radius);
        let mut rect = String::from("<rect");
        a.write(&mut rect);
        let _ = writeln!(self.defs, "<clipPath id=\"{id}\">{rect}/></clipPath>");
        id
    }

    fn group_children(&mut self, group: &Group) {
        for child in &group.children {
            self.node(child);
        }
    }

    fn node(&mut self, node: &Node) {
        match node {
            Node::Group(g) => self.group(g),
            Node::Rect(r) => {
                if r.rect.is_empty() {
                    return;
                }
                let mut a = Attrs::default();
                rect_attrs(&mut a, &r.rect, r.radius);
                paint_attrs(&mut a, r.fill.as_ref(), r.stroke.as_ref());
                self.leaf("rect", a);
            }
            Node::Path(p) => {
                if p.d.is_empty() {
                    return;
                }
                let mut a = Attrs::default();
                a.push("d", PathData::to_d(&p.d));
                paint_attrs(&mut a, p.fill.as_ref(), p.stroke.as_ref());
                self.leaf("path", a);
            }
            Node::Line(l) => {
                let mut a = Attrs::default();
                a.num("x1", l.x1);
                a.num("y1", l.y1);
                a.num("x2", l.x2);
                a.num("y2", l.y2);
                paint_attrs(&mut a, None, Some(&l.stroke));
                self.leaf("line", a);
            }
            Node::Text(t) => self.text(t),
            Node::Image(i) => {
                let href = i.href.trim();
                let is_data_image = href
                    .get(..11)
                    .is_some_and(|p| p.eq_ignore_ascii_case("data:image/"));
                if !is_data_image || i.rect.is_empty() {
                    return;
                }
                let mut a = Attrs::default();
                a.num("x", i.rect.x);
                a.num("y", i.rect.y);
                a.num("width", i.rect.width);
                a.num("height", i.rect.height);
                a.push("href", href);
                a.push(
                    "preserveAspectRatio",
                    match i.fit {
                        ImageFit::Fill => "none",
                        ImageFit::Contain => "xMidYMid meet",
                        ImageFit::Cover => "xMidYMid slice",
                    },
                );
                self.leaf("image", a);
            }
        }
    }

    fn leaf(&mut self, tag: &str, attrs: Attrs) {
        self.body.push('<');
        self.body.push_str(tag);
        attrs.write(&mut self.body);
        self.body.push_str("/>\n");
    }

    fn group(&mut self, g: &Group) {
        if g.children.is_empty() {
            return;
        }
        let mut a = Attrs::default();
        if let Some(t) = g.transform.filter(|t| !t.is_identity()) {
            a.push("transform", transform_attr(&t));
        }
        if let Some(clip) = &g.clip {
            let id = self.clip_id(clip);
            a.push("clip-path", format!("url(#{id})"));
        }
        if g.opacity < 1.0 {
            a.num("opacity", g.opacity.max(0.0));
        }
        if let Some(id) = &g.id {
            a.push("data-hit", id.encode());
        }
        if let Some(class) = g.class.as_deref().filter(|c| !c.is_empty()) {
            a.push("class", class);
        }
        self.body.push_str("<g");
        a.write(&mut self.body);
        self.body.push_str(">\n");
        self.group_children(g);
        self.body.push_str("</g>\n");
    }

    fn text(&mut self, t: &TextNode) {
        if t.text.is_empty() {
            return;
        }
        let face = t.style.face();
        self.faces.insert(face);
        let mut a = Attrs::default();
        a.num("x", t.x);
        a.num("y", t.y);
        a.push("font-family", font_family_attr(face));
        a.num("font-size", t.style.size_px());
        if face.weight() != 400 {
            a.push("font-weight", face.weight().to_string());
        }
        if t.style.italic {
            a.push("font-style", "italic");
        }
        if t.style.underline {
            a.push("text-decoration", "underline");
        }
        a.push("fill", t.style.color.to_hex());
        match t.anchor {
            TextAnchor::Start => {}
            TextAnchor::Middle => a.push("text-anchor", "middle"),
            TextAnchor::End => a.push("text-anchor", "end"),
        }
        a.push("xml:space", "preserve");
        if let Some(clip) = &t.clip {
            let id = self.clip_id(&Clip::rect(*clip));
            a.push("clip-path", format!("url(#{id})"));
        }
        self.body.push_str("<text");
        a.write(&mut self.body);
        self.body.push('>');
        escape_into(&mut self.body, &t.text, false);
        self.body.push_str("</text>\n");
    }
}

fn rect_attrs(a: &mut Attrs, r: &Rect, radius: f64) {
    a.num("x", r.x);
    a.num("y", r.y);
    a.num("width", r.width.max(0.0));
    a.num("height", r.height.max(0.0));
    let radius = radius.min(r.width / 2.0).min(r.height / 2.0);
    if radius > 0.0 && radius.is_finite() {
        a.num("rx", radius);
        a.num("ry", radius);
    }
}

fn paint_attrs(a: &mut Attrs, fill: Option<&Paint>, stroke: Option<&Stroke>) {
    // A zero-opacity fill stays a fill (`fill-opacity="0"`), so the shape
    // still receives pointer events in a webview.
    match fill {
        Some(p) => {
            a.push("fill", p.color.to_hex());
            if p.opacity < 1.0 {
                a.num("fill-opacity", p.opacity.max(0.0));
            }
        }
        None => a.push("fill", "none"),
    }
    if let Some(s) = stroke.filter(|s| s.is_visible()) {
        a.push("stroke", s.color.to_hex());
        a.num("stroke-width", s.width);
        if s.opacity < 1.0 {
            a.num("stroke-opacity", s.opacity);
        }
        let dash: Vec<String> = s
            .dash
            .iter()
            .filter(|d| d.is_finite() && **d >= 0.0)
            .map(|d| format_number(*d))
            .collect();
        if !dash.is_empty() && s.dash.iter().any(|d| *d > 0.0) {
            a.push("stroke-dasharray", dash.join(" "));
        }
    }
}

fn transform_attr(t: &Transform) -> String {
    let n = format_number;
    if t.is_translate() {
        format!("translate({} {})", n(t.e), n(t.f))
    } else if t.b == 0.0 && t.c == 0.0 {
        if t.e == 0.0 && t.f == 0.0 {
            format!("scale({} {})", n(t.a), n(t.d))
        } else {
            format!(
                "translate({} {}) scale({} {})",
                n(t.e),
                n(t.f),
                n(t.a),
                n(t.d)
            )
        }
    } else {
        format!(
            "matrix({} {} {} {} {} {})",
            n(t.a),
            n(t.b),
            n(t.c),
            n(t.d),
            n(t.e),
            n(t.f)
        )
    }
}

/// `true` for characters allowed in XML 1.0 documents.
fn xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r') || (c >= ' ' && !matches!(c, '\u{FFFE}' | '\u{FFFF}'))
}

fn escape_into(out: &mut String, text: &str, attr: bool) {
    for c in text.chars().filter(|c| xml_char(*c)) {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attr => out.push_str("&quot;"),
            '\'' if attr => out.push_str("&apos;"),
            '\n' if attr => out.push_str("&#10;"),
            '\r' if attr => out.push_str("&#13;"),
            '\t' if attr => out.push_str("&#9;"),
            c => out.push(c),
        }
    }
}

/// Standard base64 (RFC 4648, with padding).
fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk.first().copied().unwrap_or(0);
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let n = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        let sextet = |shift: u32| char::from(TABLE[((n >> shift) & 63) as usize]);
        out.push(sextet(18));
        out.push(sextet(12));
        out.push(if chunk.len() > 1 { sextet(6) } else { '=' });
        out.push(if chunk.len() > 2 { sextet(0) } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648_vectors() {
        let cases = [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ];
        for (input, want) in cases {
            assert_eq!(base64(input.as_bytes()), want);
        }
    }

    #[test]
    fn escaping_and_transforms() {
        let mut s = String::new();
        escape_into(&mut s, "a<b>&\"'\u{1}\n", true);
        assert_eq!(s, "a&lt;b&gt;&amp;&quot;&apos;&#10;");
        let mut t = String::new();
        escape_into(&mut t, "x & 'y'\u{FFFF}", false);
        assert_eq!(t, "x &amp; 'y'");
        assert_eq!(
            transform_attr(&Transform::translate(1.0, 2.0)),
            "translate(1 2)"
        );
        assert_eq!(
            transform_attr(&Transform::translate(1.0, 2.0).then(&Transform::scale(2.0, 2.0))),
            "translate(1 2) scale(2 2)"
        );
        assert!(transform_attr(&Transform::rotate(90.0)).starts_with("matrix("));
    }

    #[test]
    fn font_face_rules() {
        let css = font_face_css(&[BundledFace::Semibold].into_iter().collect(), false);
        assert_eq!(
            css,
            "@font-face{font-family:\"Selawik\";font-style:normal;font-weight:600;src:local(\"Selawik Semibold\");}"
        );
        let carlito = font_face_css(
            &[BundledFace::CarlitoBoldItalic].into_iter().collect(),
            false,
        );
        assert_eq!(
            carlito,
            "@font-face{font-family:\"Carlito\";font-style:italic;font-weight:700;src:local(\"Carlito Bold Italic\");}"
        );
        let embedded = all_font_faces_css(true);
        assert_eq!(
            embedded.matches("@font-face").count(),
            BundledFace::ALL.len()
        );
        assert!(embedded.contains("data:font/ttf;base64,AAEAAA"));
    }
}
