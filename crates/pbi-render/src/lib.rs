//! Theme, layout, semantic query to DataView, visuals to SVG and PNG.
//!
//! Pipeline: report AST + session data → [`query`] builds a [`dataview`] per visual →
//! [`visuals`] draw into a [`scene`] laid out by [`layout`] with [`theme`] and [`text`] →
//! [`svg`] serializes, [`png`] rasterizes. [`conditional`] reads and evaluates the
//! conditional formatting of tables, matrices and charts.

pub mod conditional;
pub mod dataview;
pub(crate) mod fingerprint;
pub mod golden;
pub mod layout;
pub mod png;
pub mod query;
pub mod scene;
pub mod svg;
pub mod text;
pub mod theme;
pub mod visuals;
