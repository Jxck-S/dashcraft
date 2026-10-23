//! XPress9, ABF and VertiPaq decoding to Arrow (clean-room).
//!
//! * [`xpress9`]: decompression of PBIX `DataModel` streams into ABF backups.
//! * [`xpress8`]: decompression of files stored compressed inside a backup
//!   (`ApplyCompression`, Power Pivot `xl/model/item.data`).
//! * [`abf`]: the Analysis Services backup container (header, virtual directory, backup
//!   log) with zero-copy file access ([`AbfArchive`]).
//! * [`metadata`]: `metadata.sqlitedb` → [`pbi_model::Model`] plus the per-column
//!   [`StorageMap`] used by the VertiPaq decoder; legacy XML metadata (compatibility
//!   level 1100/1103, Power Pivot) → model and storage map (`.tbl.xml`).
//! * [`vertipaq`]: column data (`.idfmeta`, `.idf`, `.dictionary`) → Arrow arrays and
//!   record batches.
//!
//! Typical use for a PBIX `DataModel` entry:
//!
//! ```no_run
//! # fn demo(datamodel: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
//! use pbi_vertipaq::{Limits, ModelOptions, load_model, open_pbix_datamodel};
//! let archive = open_pbix_datamodel(datamodel, &Limits::default())?;
//! let loaded = load_model(&archive, &ModelOptions::default())?;
//! for table in &loaded.storage.tables {
//!     for column in &table.columns {
//!         for file in column.files() {
//!             let _bytes = archive.read_file(file)?;
//!         }
//!     }
//! }
//! # Ok(()) }
//! ```
//!
//! Or, in one step, model plus decoded tables:
//!
//! ```no_run
//! # fn demo(datamodel: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
//! use pbi_vertipaq::{DecodeLimits, open_pbix_data};
//! let (model, tables, diagnostics) = open_pbix_data(datamodel, &DecodeLimits::default())?;
//! for (name, batch) in &tables {
//!     println!("{name}: {} rows", batch.num_rows());
//! }
//! # let _ = (model, diagnostics);
//! # Ok(()) }
//! ```

pub mod abf;
pub mod metadata;
pub mod vertipaq;
pub mod xpress8;
pub mod xpress9;

/// Synthetic backups and test vectors for tests of this and downstream crates
/// (feature `test-support`; never enabled in release builds).
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub mod test_support {
    pub use crate::abf::test_support::*;
    pub use crate::xpress8::test_support::*;
}

pub use abf::{AbfArchive, AbfError, AbfFile, FileRef, fuzz_abf, open_pbix_datamodel};
pub use metadata::{
    LoadedModel, MetadataDb, MetadataError, MetadataFormat, ModelOptions, StorageMap, load_model,
};
pub use vertipaq::{
    DecodeLimits, DecodedColumn, DecodedTables, InvalidIdPolicy, PbixData, VertipaqError,
    decode_all, decode_all_with_diagnostics, decode_column, decode_column_storage,
    decode_column_with_diagnostics, decode_table, decode_table_with_diagnostics, fuzz_vertipaq,
    open_pbix_data,
};
pub use xpress8::Xpress8Error;
pub use xpress9::{Limits, Xpress9Error, decompress_datamodel};
