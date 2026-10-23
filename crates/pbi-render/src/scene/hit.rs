//! Hit-test identities attached to scene groups (emitted as `data-hit` in
//! SVG) and hit regions for non-DOM hit testing.

use serde::{Deserialize, Deserializer, Serialize};

use super::geom::Rect;

/// What a click on a scene group refers to. JSON:
/// `{"visual": "…", "kind": "dataPoint", "series": 0, "index": 3}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HitId {
    /// Visual name (PBIR visual id).
    pub visual: String,
    #[serde(flatten)]
    pub kind: HitKind,
}

/// The element of a visual that was hit.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HitKind {
    /// The visual container as a whole.
    Visual,
    /// A data point (bar, slice, marker, table cell row, …).
    DataPoint { series: usize, index: usize },
    /// A slicer item (list row, dropdown entry, tile).
    SlicerItem { index: usize },
    /// A button (`actionButton`, page navigator entry, …).
    Button,
}

// Deserialized through `serde_json::Value` (flattened, internally tagged
// enums cannot read numbers under serde_json's `arbitrary_precision`).
impl<'de> Deserialize<'de> for HitId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let value = serde_json::Value::deserialize(deserializer)?;
        let text = |key: &'static str| {
            value
                .get(key)
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| D::Error::missing_field(key))
        };
        let index = |key: &'static str| {
            value
                .get(key)
                .and_then(serde_json::Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| D::Error::missing_field(key))
        };
        let kind = match text("kind")? {
            "visual" => HitKind::Visual,
            "dataPoint" => HitKind::DataPoint {
                series: index("series")?,
                index: index("index")?,
            },
            "slicerItem" => HitKind::SlicerItem {
                index: index("index")?,
            },
            "button" => HitKind::Button,
            other => {
                return Err(D::Error::unknown_variant(
                    other,
                    &["visual", "dataPoint", "slicerItem", "button"],
                ));
            }
        };
        Ok(Self {
            visual: text("visual")?.to_owned(),
            kind,
        })
    }
}

impl HitId {
    pub fn visual(visual: impl Into<String>) -> Self {
        Self {
            visual: visual.into(),
            kind: HitKind::Visual,
        }
    }

    pub fn data_point(visual: impl Into<String>, series: usize, index: usize) -> Self {
        Self {
            visual: visual.into(),
            kind: HitKind::DataPoint { series, index },
        }
    }

    pub fn slicer_item(visual: impl Into<String>, index: usize) -> Self {
        Self {
            visual: visual.into(),
            kind: HitKind::SlicerItem { index },
        }
    }

    pub fn button(visual: impl Into<String>) -> Self {
        Self {
            visual: visual.into(),
            kind: HitKind::Button,
        }
    }

    /// Compact form used for the SVG `data-hit` attribute:
    /// `<visual>|visual`, `<visual>|dataPoint|<series>|<index>`,
    /// `<visual>|slicerItem|<index>`, `<visual>|button`. `%` and `|` in the
    /// visual name are percent-encoded.
    pub fn encode(&self) -> String {
        let visual = self.visual.replace('%', "%25").replace('|', "%7C");
        match &self.kind {
            HitKind::Visual => format!("{visual}|visual"),
            HitKind::DataPoint { series, index } => format!("{visual}|dataPoint|{series}|{index}"),
            HitKind::SlicerItem { index } => format!("{visual}|slicerItem|{index}"),
            HitKind::Button => format!("{visual}|button"),
        }
    }

    /// Inverse of [`HitId::encode`]; `None` if malformed.
    pub fn decode(text: &str) -> Option<Self> {
        let mut parts = text.split('|');
        let visual = parts
            .next()?
            .replace("%7C", "|")
            .replace("%7c", "|")
            .replace("%25", "%");
        let kind = match parts.next()? {
            "visual" => HitKind::Visual,
            "dataPoint" => HitKind::DataPoint {
                series: parts.next()?.parse().ok()?,
                index: parts.next()?.parse().ok()?,
            },
            "slicerItem" => HitKind::SlicerItem {
                index: parts.next()?.parse().ok()?,
            },
            "button" => HitKind::Button,
            _ => return None,
        };
        if parts.next().is_some() {
            return None;
        }
        Some(Self { visual, kind })
    }
}

/// A hit-testable area in scene (canvas) coordinates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HitRegion {
    pub id: HitId,
    /// Bounding box after all transforms and clips.
    pub rect: Rect,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_ids_encode_and_decode() {
        let ids = [
            HitId::visual("abc"),
            HitId::data_point("a|b%c", 2, 7),
            HitId::slicer_item("s", 3),
            HitId::button("btn"),
        ];
        for id in ids {
            let enc = id.encode();
            assert_eq!(HitId::decode(&enc), Some(id.clone()), "{enc}");
        }
        assert_eq!(HitId::data_point("v", 1, 2).encode(), "v|dataPoint|1|2");
        assert_eq!(HitId::decode("v|nope"), None);
        assert_eq!(HitId::decode("v|dataPoint|x|1"), None);
        assert_eq!(HitId::decode("v|button|extra"), None);
        let json = serde_json::to_string(&HitId::data_point("v", 1, 2)).unwrap();
        assert_eq!(
            json,
            r#"{"visual":"v","kind":"dataPoint","series":1,"index":2}"#
        );
        let back: HitId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, HitId::data_point("v", 1, 2));
    }
}
