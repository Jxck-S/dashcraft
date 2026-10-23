//! Bundled fonts (Selawik and Carlito) and the mapping from Power BI font
//! families.
//!
//! Only these embedded faces are ever loaded, for measurement
//! ([`super::FontBook`]) and for PNG rendering ([`crate::png`]), so output is
//! identical on every OS. The Selawik faces share the family name [`FAMILY`]
//! and differ by weight; the Carlito faces (Calibri's metrics) share
//! [`CARLITO_FAMILY`] and differ by weight and style:
//!
//! | Power BI family (case-insensitive, first entry of a CSS list) | Face |
//! |---|---|
//! | `Segoe UI Light`, `wf_segoe-ui_light`, `* Light` | Selawik Light (300) |
//! | `Segoe UI Semilight`, `wf_segoe-ui_semilight` | Selawik Semilight (350) |
//! | `Segoe UI`, `wf_segoe-ui_normal`, `DIN`, `Arial`, `Helvetica`, unknown | Selawik Regular (400) |
//! | `Segoe UI Semibold`, `wf_segoe-ui_semibold`, `* Semibold`/`Demibold` | Selawik Semibold (600) |
//! | `Segoe UI Bold`, `wf_segoe-ui_bold`, `* Bold`/`Black`/`Heavy` | Selawik Bold (700) |
//! | `Calibri`, `Calibri Light`, `Carlito` (weight below 600) | Carlito Regular / Italic (400) |
//! | `Calibri Bold`, bold `Calibri` (weight 600 and above) | Carlito Bold / Bold Italic (700) |
//!
//! A [`TextStyle::weight`](super::TextStyle::weight) other than 400 combines
//! with the family's implied weight (bold on `Segoe UI` → Bold, bold on
//! `Segoe UI Semibold` → Bold), then snaps to the nearest bundled face of the
//! family. Italic selects Carlito's italic faces; Selawik has none (the
//! upright face is used).

use std::sync::Arc;

/// Family name every bundled Selawik face is registered under.
pub const FAMILY: &str = "Selawik";
/// Family name every bundled Carlito face is registered under.
pub const CARLITO_FAMILY: &str = "Carlito";

static LIGHT: &[u8] = include_bytes!("../../assets/fonts/selawik/Selawik-Light.ttf");
static SEMILIGHT: &[u8] = include_bytes!("../../assets/fonts/selawik/Selawik-Semilight.ttf");
static REGULAR: &[u8] = include_bytes!("../../assets/fonts/selawik/Selawik-Regular.ttf");
static SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/selawik/Selawik-Semibold.ttf");
static BOLD: &[u8] = include_bytes!("../../assets/fonts/selawik/Selawik-Bold.ttf");
static CARLITO_REGULAR: &[u8] = include_bytes!("../../assets/fonts/carlito/Carlito-Regular.ttf");
static CARLITO_BOLD: &[u8] = include_bytes!("../../assets/fonts/carlito/Carlito-Bold.ttf");
static CARLITO_ITALIC: &[u8] = include_bytes!("../../assets/fonts/carlito/Carlito-Italic.ttf");
static CARLITO_BOLD_ITALIC: &[u8] =
    include_bytes!("../../assets/fonts/carlito/Carlito-BoldItalic.ttf");

/// One of the bundled faces: Selawik (Segoe UI metrics) or Carlito
/// (Calibri metrics).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BundledFace {
    Light,
    Semilight,
    Regular,
    Semibold,
    Bold,
    CarlitoRegular,
    CarlitoBold,
    CarlitoItalic,
    CarlitoBoldItalic,
}

impl BundledFace {
    pub const ALL: [BundledFace; 9] = [
        BundledFace::Light,
        BundledFace::Semilight,
        BundledFace::Regular,
        BundledFace::Semibold,
        BundledFace::Bold,
        BundledFace::CarlitoRegular,
        BundledFace::CarlitoBold,
        BundledFace::CarlitoItalic,
        BundledFace::CarlitoBoldItalic,
    ];

    /// CSS weight this face is registered with.
    pub const fn weight(self) -> u16 {
        match self {
            BundledFace::Light => 300,
            BundledFace::Semilight => 350,
            BundledFace::Regular | BundledFace::CarlitoRegular | BundledFace::CarlitoItalic => 400,
            BundledFace::Semibold => 600,
            BundledFace::Bold | BundledFace::CarlitoBold | BundledFace::CarlitoBoldItalic => 700,
        }
    }

    /// Family name this face is registered under ([`FAMILY`] or
    /// [`CARLITO_FAMILY`]).
    pub const fn family(self) -> &'static str {
        if self.is_carlito() {
            CARLITO_FAMILY
        } else {
            FAMILY
        }
    }

    /// A Carlito face (Calibri metrics).
    pub const fn is_carlito(self) -> bool {
        matches!(
            self,
            BundledFace::CarlitoRegular
                | BundledFace::CarlitoBold
                | BundledFace::CarlitoItalic
                | BundledFace::CarlitoBoldItalic
        )
    }

    /// An italic face.
    pub const fn is_italic(self) -> bool {
        matches!(
            self,
            BundledFace::CarlitoItalic | BundledFace::CarlitoBoldItalic
        )
    }

    /// The TrueType file.
    pub fn data(self) -> &'static [u8] {
        match self {
            BundledFace::Light => LIGHT,
            BundledFace::Semilight => SEMILIGHT,
            BundledFace::Regular => REGULAR,
            BundledFace::Semibold => SEMIBOLD,
            BundledFace::Bold => BOLD,
            BundledFace::CarlitoRegular => CARLITO_REGULAR,
            BundledFace::CarlitoBold => CARLITO_BOLD,
            BundledFace::CarlitoItalic => CARLITO_ITALIC,
            BundledFace::CarlitoBoldItalic => CARLITO_BOLD_ITALIC,
        }
    }

    /// File name under `assets/fonts/selawik/` or `assets/fonts/carlito/`.
    pub const fn file_name(self) -> &'static str {
        match self {
            BundledFace::Light => "Selawik-Light.ttf",
            BundledFace::Semilight => "Selawik-Semilight.ttf",
            BundledFace::Regular => "Selawik-Regular.ttf",
            BundledFace::Semibold => "Selawik-Semibold.ttf",
            BundledFace::Bold => "Selawik-Bold.ttf",
            BundledFace::CarlitoRegular => "Carlito-Regular.ttf",
            BundledFace::CarlitoBold => "Carlito-Bold.ttf",
            BundledFace::CarlitoItalic => "Carlito-Italic.ttf",
            BundledFace::CarlitoBoldItalic => "Carlito-BoldItalic.ttf",
        }
    }

    /// Full font name, as `local()` in `@font-face` refers to the
    /// installed font (`Selawik Semibold`, `Carlito Bold Italic`).
    pub fn full_name(self) -> String {
        match self {
            BundledFace::Regular => FAMILY.to_owned(),
            BundledFace::Light | BundledFace::Semilight | BundledFace::Semibold => {
                format!("{FAMILY} {self:?}")
            }
            BundledFace::Bold => format!("{FAMILY} Bold"),
            BundledFace::CarlitoRegular => format!("{CARLITO_FAMILY} Regular"),
            BundledFace::CarlitoBold => format!("{CARLITO_FAMILY} Bold"),
            BundledFace::CarlitoItalic => format!("{CARLITO_FAMILY} Italic"),
            BundledFace::CarlitoBoldItalic => format!("{CARLITO_FAMILY} Bold Italic"),
        }
    }

    /// Nearest bundled Selawik face for a CSS weight (100–900).
    pub fn from_weight(weight: u16) -> Self {
        match weight {
            0..=325 => BundledFace::Light,
            326..=375 => BundledFace::Semilight,
            376..=500 => BundledFace::Regular,
            501..=650 => BundledFace::Semibold,
            _ => BundledFace::Bold,
        }
    }

    /// Carlito face for a CSS weight and style: Bold from 600 (CSS font
    /// matching prefers the heavier face above 500), else Regular.
    pub fn carlito(weight: u16, italic: bool) -> Self {
        match (weight >= 600, italic) {
            (false, false) => BundledFace::CarlitoRegular,
            (true, false) => BundledFace::CarlitoBold,
            (false, true) => BundledFace::CarlitoItalic,
            (true, true) => BundledFace::CarlitoBoldItalic,
        }
    }
}

/// First family of a CSS `font-family` list, without quotes (`'Segoe UI
/// Semibold', wf_segoe-ui_semibold, helvetica` → `Segoe UI Semibold`).
pub fn primary_family(list: &str) -> String {
    let mut out = String::new();
    let mut quote: Option<char> = None;
    for c in list.trim().chars() {
        match (quote, c) {
            (None, '\'' | '"') if out.trim().is_empty() => {
                out.clear();
                quote = Some(c);
            }
            (Some(q), c) if c == q => break,
            (None, ',') => break,
            _ => out.push(c),
        }
    }
    out.trim().to_owned()
}

/// Weight implied by a family name (`Segoe UI Semibold` → 600).
pub fn family_weight(family: &str) -> u16 {
    let f = primary_family(family)
        .to_ascii_lowercase()
        .replace(['-', '_'], " ");
    let has = |w: &str| f.split_whitespace().any(|t| t == w) || f.contains(w);
    if has("semilight") || has("semi light") {
        350
    } else if has("semibold") || has("semi bold") || has("demibold") || has("demi bold") {
        600
    } else if has("light") || has("thin") {
        300
    } else if has("bold") || has("black") || has("heavy") {
        700
    } else {
        400
    }
}

/// `true` if `family` (a name or CSS family list) is Calibri: its first
/// entry is `Calibri`, `Calibri Light`, `Calibri Bold`, … or `Carlito`.
pub fn is_calibri(family: &str) -> bool {
    let f = primary_family(family)
        .to_ascii_lowercase()
        .replace(['-', '_'], " ");
    let f = f.trim_start_matches("wf ");
    f.starts_with("calibri") || f.starts_with("carlito")
}

/// The bundled face for a Power BI family (or CSS family list) and CSS
/// weight (400 normal, 700 bold), upright.
pub fn resolve_face(family: &str, weight: u16) -> BundledFace {
    resolve_face_with(family, weight, false)
}

/// The bundled face for a Power BI family (or CSS family list), CSS weight
/// and style: Carlito for Calibri ([`is_calibri`]), else Selawik (which has
/// no italic faces).
pub fn resolve_face_with(family: &str, weight: u16, italic: bool) -> BundledFace {
    let implied = family_weight(family);
    let w = match (implied, weight) {
        (i, 400) => i,
        (400, w) => w,
        (i, w) if w > 400 => i.max(w),
        (i, w) => i.min(w),
    };
    if is_calibri(family) {
        BundledFace::carlito(w, italic)
    } else {
        BundledFace::from_weight(w)
    }
}

/// Builds a font database holding only the bundled faces, each under its
/// [`BundledFace::family`] with its [`BundledFace::weight`] (the Selawik
/// Semilight file declares 300; it is registered as 350 so it does not
/// collide with Light) and the file's style (Carlito's italic faces).
/// Generic families (`sans-serif`, …) all map to [`FAMILY`]. Works for both
/// fontdb versions in the tree (cosmic-text's and usvg's).
macro_rules! bundled_database {
    ($fontdb:ident) => {{
        let mut db = $fontdb::Database::new();
        for face in BundledFace::ALL {
            let ids = db.load_font_source($fontdb::Source::Binary(Arc::new(face.data())));
            for id in ids {
                if let Some(mut info) = db.face(id).cloned() {
                    db.remove_face(id);
                    info.families = vec![(
                        face.family().to_owned(),
                        $fontdb::Language::English_UnitedStates,
                    )];
                    info.weight = $fontdb::Weight(face.weight());
                    info.style = if face.is_italic() {
                        $fontdb::Style::Italic
                    } else {
                        $fontdb::Style::Normal
                    };
                    db.push_face_info(info);
                }
            }
        }
        db.set_sans_serif_family(FAMILY);
        db.set_serif_family(FAMILY);
        db.set_monospace_family(FAMILY);
        db.set_cursive_family(FAMILY);
        db.set_fantasy_family(FAMILY);
        db
    }};
}

/// Bundled-only database for cosmic-text (measurement).
pub(crate) fn cosmic_database() -> cosmic_text::fontdb::Database {
    use cosmic_text::fontdb;
    bundled_database!(fontdb)
}

/// Bundled-only database for usvg/resvg (PNG rendering). System fonts are
/// never loaded.
pub fn usvg_database() -> usvg::fontdb::Database {
    use usvg::fontdb;
    bundled_database!(fontdb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_map_to_bundled_faces() {
        let cases = [
            ("Segoe UI", 400, BundledFace::Regular),
            ("Segoe UI", 700, BundledFace::Bold),
            ("Segoe UI Semibold", 400, BundledFace::Semibold),
            ("Segoe UI Semibold", 700, BundledFace::Bold),
            ("Segoe UI Light", 400, BundledFace::Light),
            ("Segoe UI Semilight", 400, BundledFace::Semilight),
            ("Segoe UI Bold", 400, BundledFace::Bold),
            ("wf_segoe-ui_semibold", 400, BundledFace::Semibold),
            ("wf_segoe-ui_light", 400, BundledFace::Light),
            ("wf_segoe-ui_normal", 400, BundledFace::Regular),
            ("DIN", 400, BundledFace::Regular),
            ("Arial", 400, BundledFace::Regular),
            ("Helvetica", 700, BundledFace::Bold),
            ("Comic Sans MS", 400, BundledFace::Regular),
            ("", 400, BundledFace::Regular),
            (
                "'Segoe UI Semibold', wf_segoe-ui_semibold, helvetica, arial, sans-serif",
                400,
                BundledFace::Semibold,
            ),
            ("\"Segoe UI Light\", sans-serif", 400, BundledFace::Light),
        ];
        for (family, weight, want) in cases {
            assert_eq!(resolve_face(family, weight), want, "{family} {weight}");
        }
        assert_eq!(primary_family("  'A, B' , C"), "A, B");
        assert_eq!(primary_family("Segoe UI, x"), "Segoe UI");
    }

    #[test]
    fn calibri_maps_to_carlito() {
        let cases = [
            ("Calibri", 400, false, BundledFace::CarlitoRegular),
            ("calibri", 700, false, BundledFace::CarlitoBold),
            ("Calibri Light", 400, false, BundledFace::CarlitoRegular),
            ("Calibri Bold", 400, false, BundledFace::CarlitoBold),
            ("Calibri", 400, true, BundledFace::CarlitoItalic),
            ("Calibri", 700, true, BundledFace::CarlitoBoldItalic),
            (
                "'Calibri', sans-serif",
                400,
                false,
                BundledFace::CarlitoRegular,
            ),
            (
                "\"Calibri Light\", Calibri, sans-serif",
                600,
                false,
                BundledFace::CarlitoBold,
            ),
            ("Carlito", 400, false, BundledFace::CarlitoRegular),
            ("wf_calibri", 400, false, BundledFace::CarlitoRegular),
            // Selawik has no italic face; Calibri later in a list does not apply.
            ("Segoe UI", 400, true, BundledFace::Regular),
            ("'Segoe UI', Calibri", 400, false, BundledFace::Regular),
        ];
        for (family, weight, italic, want) in cases {
            assert_eq!(
                resolve_face_with(family, weight, italic),
                want,
                "{family} {weight} {italic}"
            );
        }
        assert_eq!(resolve_face("Calibri", 400), BundledFace::CarlitoRegular);
        assert_eq!(
            BundledFace::CarlitoBoldItalic.full_name(),
            "Carlito Bold Italic"
        );
        assert_eq!(BundledFace::Semibold.full_name(), "Selawik Semibold");
    }

    #[test]
    fn databases_hold_only_bundled_faces() {
        let db = usvg_database();
        assert_eq!(db.len(), BundledFace::ALL.len());
        let mut selawik: Vec<u16> = db
            .faces()
            .filter(|f| f.families[0].0 == FAMILY)
            .map(|f| f.weight.0)
            .collect();
        selawik.sort_unstable();
        assert_eq!(selawik, [300, 350, 400, 600, 700]);
        let mut carlito: Vec<(u16, bool)> = db
            .faces()
            .filter(|f| f.families[0].0 == CARLITO_FAMILY)
            .map(|f| (f.weight.0, f.style == usvg::fontdb::Style::Italic))
            .collect();
        carlito.sort_unstable();
        assert_eq!(
            carlito,
            [(400, false), (400, true), (700, false), (700, true)]
        );
        assert!(db.faces().all(|f| f.families.len() == 1));
        assert_eq!(cosmic_database().len(), BundledFace::ALL.len());
        let total: usize = BundledFace::ALL.iter().map(|f| f.data().len()).sum();
        assert!(total < 6 * 1024 * 1024);
    }
}
