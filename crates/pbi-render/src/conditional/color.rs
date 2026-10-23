//! CSS color values returned by "field value" rules (CSS Color Module
//! Level 3: hex, `rgb()`/`rgba()`, `hsl()`/`hsla()`, the extended color
//! keywords and `transparent`), and gradient interpolation.

use crate::scene::{Color, Paint};

/// Parses a CSS color (case-insensitive, surrounding spaces allowed).
/// Alpha becomes the paint's opacity. `None` for anything else.
pub fn parse_css_color(text: &str) -> Option<Paint> {
    let t = text.trim();
    if t.is_empty() || t.len() > 64 {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    if let Some(hex) = lower.strip_prefix('#') {
        return parse_hex(hex);
    }
    if lower == "transparent" {
        return Some(Paint::new(Color::BLACK, 0.0));
    }
    if let Some((name, args)) = lower.split_once('(') {
        let args = args.strip_suffix(')')?;
        let parts: Vec<&str> = args
            .split([',', ' ', '/'])
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .collect();
        return match name.trim() {
            "rgb" | "rgba" => rgb_function(&parts),
            "hsl" | "hsla" => hsl_function(&parts),
            _ => None,
        };
    }
    named(&lower).map(Paint::solid)
}

fn parse_hex(hex: &str) -> Option<Paint> {
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let digit = |i: usize| u8::from_str_radix(hex.get(i..=i)?, 16).ok();
    let pair = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    let (r, g, b, a) = match hex.len() {
        3 | 4 => {
            let d = |i: usize| digit(i).map(|v| v * 17);
            let a = if hex.len() == 4 { d(3)? } else { 255 };
            (d(0)?, d(1)?, d(2)?, a)
        }
        6 | 8 => {
            let a = if hex.len() == 8 { pair(6)? } else { 255 };
            (pair(0)?, pair(2)?, pair(4)?, a)
        }
        _ => return None,
    };
    Some(Paint::new(Color::rgb(r, g, b), f64::from(a) / 255.0))
}

/// A number or percentage scaled to `0..=max` (`100%` = `max`).
fn channel(text: &str, max: f64) -> Option<f64> {
    let v = match text.strip_suffix('%') {
        Some(p) => p.parse::<f64>().ok()? / 100.0 * max,
        None => text.parse::<f64>().ok()?,
    };
    v.is_finite().then(|| v.clamp(0.0, max))
}

fn alpha(parts: &[&str], at: usize) -> Option<f64> {
    match parts.get(at) {
        Some(a) => channel(a, 1.0),
        None => Some(1.0),
    }
}

fn rgb_function(parts: &[&str]) -> Option<Paint> {
    if !(3..=4).contains(&parts.len()) {
        return None;
    }
    let c = |i: usize| -> Option<u8> { Some(channel(parts.get(i)?, 255.0)?.round() as u8) };
    Some(Paint::new(
        Color::rgb(c(0)?, c(1)?, c(2)?),
        alpha(parts, 3)?,
    ))
}

fn hsl_function(parts: &[&str]) -> Option<Paint> {
    if !(3..=4).contains(&parts.len()) {
        return None;
    }
    let h = parts.first()?.trim_end_matches("deg").parse::<f64>().ok()?;
    if !h.is_finite() {
        return None;
    }
    let s = channel(parts.get(1)?, 1.0)?;
    let l = channel(parts.get(2)?, 1.0)?;
    Some(Paint::new(hsl_to_rgb(h, s, l), alpha(parts, 3)?))
}

/// CSS `hsl()` to sRGB (CSS Color 3, §4.2.4).
fn hsl_to_rgb(h: f64, s: f64, l: f64) -> Color {
    let h = h.rem_euclid(360.0) / 360.0;
    let m2 = if l <= 0.5 {
        l * (s + 1.0)
    } else {
        l + s - l * s
    };
    let m1 = l * 2.0 - m2;
    let hue = |h: f64| {
        let h = h.rem_euclid(1.0);
        if h * 6.0 < 1.0 {
            m1 + (m2 - m1) * h * 6.0
        } else if h * 2.0 < 1.0 {
            m2
        } else if h * 3.0 < 2.0 {
            m1 + (m2 - m1) * (2.0 / 3.0 - h) * 6.0
        } else {
            m1
        }
    };
    let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color::rgb(
        byte(hue(h + 1.0 / 3.0)),
        byte(hue(h)),
        byte(hue(h - 1.0 / 3.0)),
    )
}

/// Linear interpolation between two colors in sRGB (`t` clamped to 0–1).
pub fn lerp(a: Color, b: Color, t: f64) -> Color {
    let t = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mix = |x: u8, y: u8| (f64::from(x) + (f64::from(y) - f64::from(x)) * t).round() as u8;
    Color::rgb(mix(a.r, b.r), mix(a.g, b.g), mix(a.b, b.b))
}

/// CSS extended color keywords (CSS Color 3 §4.3, plus `rebeccapurple`),
/// sorted by name.
const NAMED: [(&str, u32); 148] = [
    ("aliceblue", 0xF0F8FF),
    ("antiquewhite", 0xFAEBD7),
    ("aqua", 0x00FFFF),
    ("aquamarine", 0x7FFFD4),
    ("azure", 0xF0FFFF),
    ("beige", 0xF5F5DC),
    ("bisque", 0xFFE4C4),
    ("black", 0x000000),
    ("blanchedalmond", 0xFFEBCD),
    ("blue", 0x0000FF),
    ("blueviolet", 0x8A2BE2),
    ("brown", 0xA52A2A),
    ("burlywood", 0xDEB887),
    ("cadetblue", 0x5F9EA0),
    ("chartreuse", 0x7FFF00),
    ("chocolate", 0xD2691E),
    ("coral", 0xFF7F50),
    ("cornflowerblue", 0x6495ED),
    ("cornsilk", 0xFFF8DC),
    ("crimson", 0xDC143C),
    ("cyan", 0x00FFFF),
    ("darkblue", 0x00008B),
    ("darkcyan", 0x008B8B),
    ("darkgoldenrod", 0xB8860B),
    ("darkgray", 0xA9A9A9),
    ("darkgreen", 0x006400),
    ("darkgrey", 0xA9A9A9),
    ("darkkhaki", 0xBDB76B),
    ("darkmagenta", 0x8B008B),
    ("darkolivegreen", 0x556B2F),
    ("darkorange", 0xFF8C00),
    ("darkorchid", 0x9932CC),
    ("darkred", 0x8B0000),
    ("darksalmon", 0xE9967A),
    ("darkseagreen", 0x8FBC8F),
    ("darkslateblue", 0x483D8B),
    ("darkslategray", 0x2F4F4F),
    ("darkslategrey", 0x2F4F4F),
    ("darkturquoise", 0x00CED1),
    ("darkviolet", 0x9400D3),
    ("deeppink", 0xFF1493),
    ("deepskyblue", 0x00BFFF),
    ("dimgray", 0x696969),
    ("dimgrey", 0x696969),
    ("dodgerblue", 0x1E90FF),
    ("firebrick", 0xB22222),
    ("floralwhite", 0xFFFAF0),
    ("forestgreen", 0x228B22),
    ("fuchsia", 0xFF00FF),
    ("gainsboro", 0xDCDCDC),
    ("ghostwhite", 0xF8F8FF),
    ("gold", 0xFFD700),
    ("goldenrod", 0xDAA520),
    ("gray", 0x808080),
    ("green", 0x008000),
    ("greenyellow", 0xADFF2F),
    ("grey", 0x808080),
    ("honeydew", 0xF0FFF0),
    ("hotpink", 0xFF69B4),
    ("indianred", 0xCD5C5C),
    ("indigo", 0x4B0082),
    ("ivory", 0xFFFFF0),
    ("khaki", 0xF0E68C),
    ("lavender", 0xE6E6FA),
    ("lavenderblush", 0xFFF0F5),
    ("lawngreen", 0x7CFC00),
    ("lemonchiffon", 0xFFFACD),
    ("lightblue", 0xADD8E6),
    ("lightcoral", 0xF08080),
    ("lightcyan", 0xE0FFFF),
    ("lightgoldenrodyellow", 0xFAFAD2),
    ("lightgray", 0xD3D3D3),
    ("lightgreen", 0x90EE90),
    ("lightgrey", 0xD3D3D3),
    ("lightpink", 0xFFB6C1),
    ("lightsalmon", 0xFFA07A),
    ("lightseagreen", 0x20B2AA),
    ("lightskyblue", 0x87CEFA),
    ("lightslategray", 0x778899),
    ("lightslategrey", 0x778899),
    ("lightsteelblue", 0xB0C4DE),
    ("lightyellow", 0xFFFFE0),
    ("lime", 0x00FF00),
    ("limegreen", 0x32CD32),
    ("linen", 0xFAF0E6),
    ("magenta", 0xFF00FF),
    ("maroon", 0x800000),
    ("mediumaquamarine", 0x66CDAA),
    ("mediumblue", 0x0000CD),
    ("mediumorchid", 0xBA55D3),
    ("mediumpurple", 0x9370DB),
    ("mediumseagreen", 0x3CB371),
    ("mediumslateblue", 0x7B68EE),
    ("mediumspringgreen", 0x00FA9A),
    ("mediumturquoise", 0x48D1CC),
    ("mediumvioletred", 0xC71585),
    ("midnightblue", 0x191970),
    ("mintcream", 0xF5FFFA),
    ("mistyrose", 0xFFE4E1),
    ("moccasin", 0xFFE4B5),
    ("navajowhite", 0xFFDEAD),
    ("navy", 0x000080),
    ("oldlace", 0xFDF5E6),
    ("olive", 0x808000),
    ("olivedrab", 0x6B8E23),
    ("orange", 0xFFA500),
    ("orangered", 0xFF4500),
    ("orchid", 0xDA70D6),
    ("palegoldenrod", 0xEEE8AA),
    ("palegreen", 0x98FB98),
    ("paleturquoise", 0xAFEEEE),
    ("palevioletred", 0xDB7093),
    ("papayawhip", 0xFFEFD5),
    ("peachpuff", 0xFFDAB9),
    ("peru", 0xCD853F),
    ("pink", 0xFFC0CB),
    ("plum", 0xDDA0DD),
    ("powderblue", 0xB0E0E6),
    ("purple", 0x800080),
    ("rebeccapurple", 0x663399),
    ("red", 0xFF0000),
    ("rosybrown", 0xBC8F8F),
    ("royalblue", 0x4169E1),
    ("saddlebrown", 0x8B4513),
    ("salmon", 0xFA8072),
    ("sandybrown", 0xF4A460),
    ("seagreen", 0x2E8B57),
    ("seashell", 0xFFF5EE),
    ("sienna", 0xA0522D),
    ("silver", 0xC0C0C0),
    ("skyblue", 0x87CEEB),
    ("slateblue", 0x6A5ACD),
    ("slategray", 0x708090),
    ("slategrey", 0x708090),
    ("snow", 0xFFFAFA),
    ("springgreen", 0x00FF7F),
    ("steelblue", 0x4682B4),
    ("tan", 0xD2B48C),
    ("teal", 0x008080),
    ("thistle", 0xD8BFD8),
    ("tomato", 0xFF6347),
    ("turquoise", 0x40E0D0),
    ("violet", 0xEE82EE),
    ("wheat", 0xF5DEB3),
    ("white", 0xFFFFFF),
    ("whitesmoke", 0xF5F5F5),
    ("yellow", 0xFFFF00),
    ("yellowgreen", 0x9ACD32),
];

fn named(name: &str) -> Option<Color> {
    let i = NAMED.binary_search_by(|(n, _)| (*n).cmp(name)).ok()?;
    let (_, rgb) = NAMED.get(i)?;
    let [_, r, g, b] = rgb.to_be_bytes();
    Some(Color::rgb(r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_color_forms() {
        let rgb = |t: &str| parse_css_color(t).map(|p| (p.color.to_hex(), p.opacity));
        assert_eq!(rgb("#3E4AFF"), Some(("#3E4AFF".into(), 1.0)));
        assert_eq!(rgb("#f00"), Some(("#FF0000".into(), 1.0)));
        assert_eq!(rgb("#00ff0080").map(|c| c.0), Some("#00FF00".into()));
        assert_eq!(rgb(" SkyBlue "), Some(("#87CEEB".into(), 1.0)));
        assert_eq!(rgb("PeachPuff").map(|c| c.0), Some("#FFDAB9".into()));
        assert_eq!(
            rgb("RGBA(234, 234, 234, 0.5)"),
            Some(("#EAEAEA".into(), 0.5))
        );
        assert_eq!(
            rgb("rgb(100%, 0%, 0%)").map(|c| c.0),
            Some("#FF0000".into())
        );
        assert_eq!(
            rgb("hsl(120, 100%, 25%)").map(|c| c.0),
            Some("#008000".into())
        );
        assert_eq!(
            rgb("HSLA(0, 100%, 50%, 1)").map(|c| c.0),
            Some("#FF0000".into())
        );
        assert_eq!(rgb("transparent").map(|c| c.1), Some(0.0));
        for bad in [
            "",
            "#12",
            "#GGGGGG",
            "nocolor",
            "rgb(1,2)",
            "rgb(a,b,c)",
            "url(x)",
        ] {
            assert_eq!(rgb(bad), None, "{bad}");
        }
        assert!(NAMED.windows(2).all(|w| w[0].0 < w[1].0), "sorted");
    }

    #[test]
    fn interpolation() {
        let (w, b) = (Color::WHITE, Color::rgb(0x11, 0x8D, 0xFF));
        assert_eq!(lerp(w, b, 0.0), w);
        assert_eq!(lerp(w, b, 1.0), b);
        assert_eq!(lerp(w, b, 0.5), Color::rgb(0x88, 0xC6, 0xFF));
        assert_eq!(lerp(w, b, 7.0), b);
        assert_eq!(lerp(w, b, f64::NAN), w);
    }
}
