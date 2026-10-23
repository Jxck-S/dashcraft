//! Report themes (`StaticResources/*/BaseThemes/*.json`, custom themes) and
//! color resolution.
//!
//! # `ThemeDataColor{ColorId, Percent}`
//!
//! `ColorId` indexes the **theme row of Power BI's color picker**, not
//! `dataColors` directly:
//!
//! | `ColorId` | Color |
//! |---|---|
//! | 0 | white `#FFFFFF` ("White") |
//! | 1 | black `#000000` ("Black") |
//! | 2..=9 | `dataColors[0..=7]` ("Theme color 1–8") |
//!
//! Ids 0 and 1 are fixed white/black, independent of the theme's
//! `background`/`foreground`. Ids past the eight picker colors (or past the
//! end of `dataColors`) do not resolve.
//!
//! `Percent` shades the color per RGB channel: `p < 0` darkens toward black
//! (`c·(1+p)`), `p > 0` lightens toward white (`c + (255−c)·p`); each
//! channel rounds half up. E.g. white at −0.1 is `#E6E6E6`, black at +0.1
//! is `#1A1A1A`, `#E81123` at −0.1 is `#D10F20`.
//!
//! Source: the Microsoft schema only says "the theme color to select"
//! (`semanticQuery` `QueryThemeDataColorExpression`). The mapping and the
//! shading rule follow the measurements of Power BI Desktop documented in
//! the MIT-licensed `d0nk3yhm/pbix-mcp` test
//! `tests/test_issue119_theme_data_color.py` (no code copied). Our fixtures
//! agree: cards use `ColorId 0, Percent -0.6` for a dark-grey value text
//! (`#666666`) and `ColorId 0, Percent 0` for a white background.

use indexmap::IndexMap;
use serde_json::{Number, Value};

use crate::json::{json_object, num_f64};

json_object! {
    /// A report theme JSON file.
    pub struct ThemeJson {
        "name" => name: String,
        "dataColors" => data_colors: Vec<String>,
        "foreground" => foreground: String,
        "foregroundNeutralSecondary" => foreground_neutral_secondary: String,
        "foregroundNeutralTertiary" => foreground_neutral_tertiary: String,
        "background" => background: String,
        "backgroundLight" => background_light: String,
        "backgroundNeutral" => background_neutral: String,
        "tableAccent" => table_accent: String,
        "good" => good: String,
        "neutral" => neutral: String,
        "bad" => bad: String,
        "maximum" => maximum: String,
        "center" => center: String,
        "minimum" => minimum: String,
        "null" => null_color: String,
        "hyperlink" => hyperlink: String,
        "visitedHyperlink" => visited_hyperlink: String,
        "textClasses" => text_classes: IndexMap<String, TextClassJson>,
        /// `visualStyles[visualType][styleName][objectName][]`, verbatim.
        "visualStyles" => visual_styles: Value,
    }
}

json_object! {
    /// A `textClasses` entry (`callout`, `title`, `header`, `label`, …).
    pub struct TextClassJson {
        "fontSize" => font_size: Number,
        "fontFace" => font_face: String,
        "color" => color: String,
    }
}

/// Number of theme colors in the color picker's theme row.
const PICKER_DATA_COLORS: i64 = 8;

/// An RGB color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// Parses `#RGB`, `#RRGGBB` or `#RRGGBBAA` (alpha ignored); the `#` is
    /// optional.
    pub fn parse_hex(text: &str) -> Option<Self> {
        let hex = text.trim();
        let hex = hex.strip_prefix('#').unwrap_or(hex);
        if !hex.is_ascii() {
            return None;
        }
        let channel = |s: &str| u8::from_str_radix(s, 16).ok();
        match hex.len() {
            3 => {
                let mut c = hex.chars().map(|c| channel(&c.to_string()).map(|v| v * 17));
                Some(Rgb(c.next()??, c.next()??, c.next()??))
            }
            6 | 8 => Some(Rgb(
                channel(&hex[0..2])?,
                channel(&hex[2..4])?,
                channel(&hex[4..6])?,
            )),
            _ => None,
        }
    }

    /// `#RRGGBB` (uppercase).
    pub fn to_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.0, self.1, self.2)
    }

    /// Lightens (`percent > 0`) toward white or darkens (`percent < 0`)
    /// toward black; `percent` is clamped to `[-1, 1]`.
    pub fn shade(self, percent: f64) -> Self {
        let p = if percent.is_finite() {
            percent.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        let f = |c: u8| -> u8 {
            let c = f64::from(c);
            let v = if p < 0.0 {
                c * (1.0 + p)
            } else {
                c + (255.0 - c) * p
            };
            // Round half up; the clamp keeps the cast in range.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let out = (v + 0.5).floor().clamp(0.0, 255.0) as u8;
            out
        };
        Rgb(f(self.0), f(self.1), f(self.2))
    }
}

/// Resolves `ThemeDataColor{ColorId, Percent}` against `theme` (see the
/// [module docs](self)). Returns `#RRGGBB`.
pub fn resolve_theme_data_color(theme: &ThemeJson, color_id: i64, percent: f64) -> Option<String> {
    let base = match color_id {
        0 => Rgb(255, 255, 255),
        1 => Rgb(0, 0, 0),
        id if (2..2 + PICKER_DATA_COLORS).contains(&id) => {
            let index = usize::try_from(id - 2).ok()?;
            Rgb::parse_hex(theme.data_colors.as_ref()?.get(index)?)?
        }
        _ => return None,
    };
    Some(base.shade(percent).to_hex())
}

impl ThemeJson {
    /// A named theme color: `foreground`, `background`, `tableAccent`,
    /// `good`, `bad`, … (also the `minColor`/`midColor`/`maxColor`
    /// aliases). Case-insensitive.
    pub fn named_color(&self, name: &str) -> Option<&str> {
        let n = name.trim().to_ascii_lowercase();
        let value = match n.as_str() {
            "foreground" => &self.foreground,
            "foregroundneutralsecondary" => &self.foreground_neutral_secondary,
            "foregroundneutraltertiary" => &self.foreground_neutral_tertiary,
            "background" => &self.background,
            "backgroundlight" => &self.background_light,
            "backgroundneutral" => &self.background_neutral,
            "tableaccent" => &self.table_accent,
            "good" => &self.good,
            "neutral" => &self.neutral,
            "bad" => &self.bad,
            "maximum" | "maxcolor" => &self.maximum,
            "center" | "midcolor" => &self.center,
            "minimum" | "mincolor" => &self.minimum,
            "null" => &self.null_color,
            "hyperlink" => &self.hyperlink,
            "visitedhyperlink" => &self.visited_hyperlink,
            _ => return self.extra.get(name).and_then(Value::as_str),
        };
        value.as_deref()
    }

    /// A `textClasses` entry.
    pub fn text_class(&self, name: &str) -> Option<&TextClassJson> {
        self.text_classes.as_ref()?.get(name)
    }
}

impl TextClassJson {
    pub fn font_size_f64(&self) -> Option<f64> {
        self.font_size.as_ref().and_then(num_f64)
    }
}

/// Resolves a formatting color value to `#RRGGBB`.
///
/// Accepts the shapes found in `objects` and themes:
/// - `{"solid": {"color": …}}` (fill/font colors);
/// - `{"expr": …}` wrappers;
/// - `{"Literal": {"Value": "'#C00000'"}}` (a quoted hex or a named theme
///   color such as `'good'`);
/// - `{"ThemeDataColor": {"ColorId": 0, "Percent": -0.6}}` (needs `theme`);
/// - a bare string (`"#C00000"`, `"'#C00000'"` or a theme color name).
///
/// Anything else (conditional `FillRule`s, measures, …) returns `None`.
pub fn resolve_color(value: &Value, theme: Option<&ThemeJson>) -> Option<String> {
    resolve_color_depth(value, theme, 0)
}

fn resolve_color_depth(value: &Value, theme: Option<&ThemeJson>, depth: u8) -> Option<String> {
    if depth > 8 {
        return None;
    }
    match value {
        Value::String(text) => color_text(text, theme),
        Value::Object(map) => {
            if let Some(inner) = map.get("solid").and_then(|s| s.get("color")) {
                return resolve_color_depth(inner, theme, depth + 1);
            }
            if let Some(inner) = map.get("expr").or_else(|| map.get("color")) {
                return resolve_color_depth(inner, theme, depth + 1);
            }
            if let Some(text) = map
                .get("Literal")
                .and_then(|l| l.get("Value"))
                .and_then(Value::as_str)
            {
                return color_text(text, theme);
            }
            if let Some(tdc) = map.get("ThemeDataColor") {
                let id = tdc.get("ColorId").and_then(Value::as_number)?;
                let percent = tdc
                    .get("Percent")
                    .and_then(Value::as_number)
                    .and_then(num_f64)
                    .unwrap_or(0.0);
                return resolve_theme_data_color(theme?, crate::json::num_i64(id)?, percent);
            }
            None
        }
        _ => None,
    }
}

/// A literal color text: `#hex`, `'#hex'` or a (quoted) theme color name.
fn color_text(text: &str, theme: Option<&ThemeJson>) -> Option<String> {
    let t = text.trim();
    let t = t
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .unwrap_or(t);
    if let Some(rgb) = t.starts_with('#').then(|| Rgb::parse_hex(t)).flatten() {
        return Some(rgb.to_hex());
    }
    let named = theme?.named_color(t)?;
    Rgb::parse_hex(named).map(Rgb::to_hex)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn theme(colors: &[&str]) -> ThemeJson {
        ThemeJson {
            data_colors: Some(colors.iter().map(|s| (*s).to_owned()).collect()),
            foreground: Some("#252423".into()),
            background: Some("#FFFFFF".into()),
            good: Some("#1AAB40".into()),
            ..ThemeJson::default()
        }
    }

    #[test]
    fn color_ids_index_the_picker_row() {
        let pal = [
            "#118DFF", "#12239E", "#E66C37", "#6B007B", "#E044A7", "#744EC2", "#D9B300", "#D64550",
            "#197278", "#1AAB40",
        ];
        let t = theme(&pal);
        assert_eq!(
            resolve_theme_data_color(&t, 0, 0.0).as_deref(),
            Some("#FFFFFF")
        );
        assert_eq!(
            resolve_theme_data_color(&t, 1, 0.0).as_deref(),
            Some("#000000")
        );
        for (i, c) in pal.iter().take(8).enumerate() {
            let id = i64::try_from(i).unwrap() + 2;
            assert_eq!(resolve_theme_data_color(&t, id, 0.0).as_deref(), Some(*c));
        }
        assert_eq!(resolve_theme_data_color(&t, 10, 0.0), None);
        assert_eq!(resolve_theme_data_color(&t, -1, 0.0), None);
        assert_eq!(resolve_theme_data_color(&theme(&pal[..3]), 5, 0.0), None);
    }

    #[test]
    fn percent_shades_toward_white_or_black() {
        let t = theme(&["#E81123", "#00B294"]);
        let cases = [
            (0, -0.1, "#E6E6E6"),
            (0, -0.6, "#666666"),
            (1, 0.1, "#1A1A1A"),
            (2, -0.1, "#D10F20"),
            (3, 0.0, "#00B294"),
            (2, 1.0, "#FFFFFF"),
            (2, -1.0, "#000000"),
            (2, -7.0, "#000000"),
            (2, f64::NAN, "#E81123"),
        ];
        for (id, p, want) in cases {
            assert_eq!(
                resolve_theme_data_color(&t, id, p).as_deref(),
                Some(want),
                "{id} {p}"
            );
        }
    }

    #[test]
    fn formatting_colors_resolve() {
        let t = theme(&["#118DFF"]);
        let solid_tdc = json!({"solid": {"color": {"expr": {"ThemeDataColor": {"ColorId": 0, "Percent": -0.6}}}}});
        assert_eq!(
            resolve_color(&solid_tdc, Some(&t)).as_deref(),
            Some("#666666")
        );
        assert_eq!(resolve_color(&solid_tdc, None), None);
        let literal = json!({"solid": {"color": {"expr": {"Literal": {"Value": "'#c00000'"}}}}});
        assert_eq!(resolve_color(&literal, None).as_deref(), Some("#C00000"));
        assert_eq!(
            resolve_color(&json!("#7030A0"), None).as_deref(),
            Some("#7030A0")
        );
        assert_eq!(
            resolve_color(&json!("'#FFF'"), None).as_deref(),
            Some("#FFFFFF")
        );
        assert_eq!(
            resolve_color(&json!("'good'"), Some(&t)).as_deref(),
            Some("#1AAB40")
        );
        assert_eq!(
            resolve_color(&json!({"expr": {"Measure": {}}}), Some(&t)),
            None
        );
        assert_eq!(resolve_color(&json!("#GGGGGG"), None), None);
        assert_eq!(resolve_color(&json!("#ÿÿÿ"), None), None);
    }
}
