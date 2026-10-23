//! Synthetic ABF backups for unit tests (no customer data).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::PAGE_SIZE;
use crate::xpress9::STREAM_STORAGE_SIGNATURE;

pub const SERVER_ROOT: &str = r"\\?\C:\Users\tester\AnalysisServicesWorkspaces\ws\Data";
pub const DB_FOLDER: &str = "testdb.1.db";

fn utf16le(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

pub fn esc(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn pad_to_page(buf: &mut Vec<u8>) {
    let rem = buf.len() % PAGE_SIZE;
    if rem != 0 {
        buf.resize(buf.len() + PAGE_SIZE - rem, 0);
    }
}

/// Byte offsets of the sections of a built backup (for corruption tests).
#[derive(Debug, Clone)]
pub struct Layout {
    pub directory_offset: usize,
    pub directory_size: usize,
    pub log_offset: usize,
}

/// Builds an uncompressed ABF backup laid out like the ones Power BI writes.
#[derive(Debug, Clone, Default)]
pub struct AbfBuilder {
    /// (path relative to the database folder, content, stored bytes if different).
    files: Vec<(String, Vec<u8>, Option<Vec<u8>>)>,
    error_code: bool,
    apply_compression: bool,
    encryption: bool,
    header_override: Option<String>,
    directory_override: Option<Vec<u8>>,
}

impl AbfBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a file under the database folder (`\`-separated relative path).
    pub fn file(mut self, relative: &str, content: impl Into<Vec<u8>>) -> Self {
        self.files.push((relative.to_owned(), content.into(), None));
        self
    }

    /// Adds a file whose stored bytes differ from its content (e.g. hand-made
    /// compressed chunks for [`AbfBuilder::apply_compression`]); the backup log records
    /// the content size.
    pub fn file_stored_as(
        mut self,
        relative: &str,
        content: impl Into<Vec<u8>>,
        stored: impl Into<Vec<u8>>,
    ) -> Self {
        self.files
            .push((relative.to_owned(), content.into(), Some(stored.into())));
        self
    }

    /// Stores 4-byte CRC markers after each file (`ErrorCode` = true).
    pub fn crc_markers(mut self) -> Self {
        self.error_code = true;
        self
    }

    /// Sets `ApplyCompression`: files without explicit stored bytes are framed as
    /// stored Xpress chunks, as Power Pivot writes incompressible data.
    pub fn apply_compression(mut self) -> Self {
        self.apply_compression = true;
        self
    }

    pub fn encrypted(mut self) -> Self {
        self.encryption = true;
        self
    }

    /// Replaces the header XML (after computing the layout).
    pub fn header_xml(mut self, xml: &str) -> Self {
        self.header_override = Some(xml.to_owned());
        self
    }

    /// Replaces the virtual directory bytes.
    pub fn directory_bytes(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.directory_override = Some(bytes.into());
        self
    }

    pub fn build(&self) -> Vec<u8> {
        self.build_with_layout().0
    }

    pub fn build_with_layout(&self) -> (Vec<u8>, Layout) {
        let mut out = vec![0u8; PAGE_SIZE];
        // (storage name, offset, stored size)
        let mut entries: Vec<(String, usize, usize)> = Vec::new();
        let marker = if self.error_code { 4 } else { 0 };
        let mut push = |out: &mut Vec<u8>, name: String, content: &[u8]| {
            let offset = out.len();
            out.extend_from_slice(content);
            out.extend(std::iter::repeat_n(0xA5, marker));
            entries.push((name, offset, content.len() + marker));
        };
        push(&mut out, "ADDITIONAL_LOG".into(), b"additional-log");
        push(&mut out, "PARTITIONS".into(), &[0u8; 28]);

        let persist = format!(r"{SERVER_ROOT}\{DB_FOLDER}");
        let mut file_list = String::new();
        let mut db_file_list = String::new();
        for (index, (relative, content, stored)) in self.files.iter().enumerate() {
            let storage = format!("{:020X}", index + 1);
            match stored {
                Some(stored) => push(&mut out, storage.clone(), stored),
                None if self.apply_compression => push(
                    &mut out,
                    storage.clone(),
                    &crate::xpress8::test_support::stored_chunks(content, 1000),
                ),
                None => push(&mut out, storage.clone(), content),
            }
            let item = format!(
                "<BackupFile><Path>{}</Path><StoragePath>{storage}</StoragePath><LastWriteTime>1</LastWriteTime><Size>{}</Size></BackupFile>",
                esc(&if relative.ends_with(".db.xml") {
                    format!(r"{SERVER_ROOT}\{relative}")
                } else {
                    format!(r"{persist}\{relative}")
                }),
                content.len()
            );
            if relative.ends_with(".db.xml") || relative.ends_with("CryptKey.bin") {
                db_file_list.push_str(&item);
            } else {
                file_list.push_str(&item);
            }
        }

        let log_xml = format!(
            "<BackupLog><BackupRestoreSyncVersion>11.53</BackupRestoreSyncVersion><ServerRoot>{root}</ServerRoot><CompressionFlag>false</CompressionFlag><EncryptionFlag>false</EncryptionFlag><ObjectName>testdb</ObjectName><ObjectId>testdb</ObjectId><IsTabular>true</IsTabular><Collations/><Languages><Language>1033</Language></Languages><FileGroups><FileGroup><Class>100002</Class><ID>testdb</ID><Name>testdb</Name><ObjectVersion>1</ObjectVersion><PersistLocation>1</PersistLocation><PersistLocationPath>{persist}</PersistLocationPath><StorageLocationPath/><FileList>{db_file_list}</FileList></FileGroup><FileGroup><Class>100069</Class><ID>testdb</ID><Name>testdb</Name><ObjectVersion>-1</ObjectVersion><PersistLocation>-1</PersistLocation><PersistLocationPath>{persist}</PersistLocationPath><StorageLocationPath/><FileList>{file_list}</FileList></FileGroup></FileGroups></BackupLog>",
            root = esc(SERVER_ROOT),
            persist = esc(&persist),
        );
        let mut log = vec![0xFF, 0xFE];
        log.extend(utf16le(&log_xml));
        let log_offset = out.len();
        push(&mut out, "LOG".into(), &log);
        pad_to_page(&mut out);

        let directory = match &self.directory_override {
            Some(bytes) => bytes.clone(),
            None => {
                let mut xml = String::from("<VirtualDirectory>");
                for (name, offset, size) in &entries {
                    xml.push_str(&format!(
                        "<BackupFile><Path>{name}</Path><Size>{size}</Size><m_cbOffsetHeader>{offset}</m_cbOffsetHeader><Delete>false</Delete><CreatedTimestamp>1</CreatedTimestamp><Access>1</Access><LastWriteTime>1</LastWriteTime></BackupFile>"
                    ));
                }
                xml.push_str("</VirtualDirectory>");
                xml.into_bytes()
            }
        };
        let directory_offset = out.len();
        out.extend_from_slice(&directory);
        pad_to_page(&mut out);

        let header_xml = self.header_override.clone().unwrap_or_else(|| {
            format!(
                "<BackupLog><BackupRestoreSyncVersion>140</BackupRestoreSyncVersion><Fault>false</Fault><faultcode>0</faultcode><ErrorCode>{}</ErrorCode><EncryptionFlag>{}</EncryptionFlag><EncryptionKey>0</EncryptionKey><ApplyCompression>{}</ApplyCompression><m_cbOffsetHeader>{directory_offset}</m_cbOffsetHeader><DataSize>{}</DataSize><Files>{}</Files><ObjectID>00000000-0000-0000-0000-000000000000</ObjectID><m_cbOffsetData>4096</m_cbOffsetData></BackupLog>",
                self.error_code,
                self.encryption,
                self.apply_compression,
                directory.len(),
                entries.len()
            )
        });
        let mut page = vec![0xFF, 0xFE];
        page.extend(utf16le(STREAM_STORAGE_SIGNATURE));
        page.extend(utf16le(&header_xml));
        page.truncate(PAGE_SIZE);
        out[..page.len()].copy_from_slice(&page);
        (
            out,
            Layout {
                directory_offset,
                directory_size: directory.len(),
                log_offset,
            },
        )
    }
}

const ASSL_NS: &str = r#"xmlns="http://schemas.microsoft.com/analysisservices/2003/engine" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:ddl200="http://schemas.microsoft.com/analysisservices/2010/engine/200" xmlns:ddl200_200="http://schemas.microsoft.com/analysisservices/2010/engine/200/200" xmlns:ddl300_300="http://schemas.microsoft.com/analysisservices/2011/engine/300/300""#;

fn attribute(id: &str, name: &str, data_type: &str, source: &str, visible: bool) -> String {
    format!(
        "<Attribute><Name>{name}</Name><ID>{id}</ID><Type>Regular</Type><KeyColumns><KeyColumn><DataType>{data_type}</DataType>{source}</KeyColumn></KeyColumns><AttributeHierarchyVisible>{visible}</AttributeHierarchyVisible></Attribute>"
    )
}

fn column_binding(table: &str, column: &str) -> String {
    format!(
        r#"<Source xsi:type="ColumnBinding"><TableID>{table}</TableID><ColumnID>{column}</ColumnID></Source>"#
    )
}

/// One hybrid-RLE segment and its `.idf` bytes. `runs` are `(data id, repeat)`; a
/// `packed` list adds a bit-packing entry whose values (data ids) come from a 1-bit
/// subsegment based at `min`.
fn rle_segment(runs: &[(u32, u32)], packed: &[u32]) -> (String, Vec<u8>) {
    let rows: u32 = runs.iter().map(|r| r.1).sum::<u32>() + packed.len() as u32;
    let ids: Vec<u32> = runs
        .iter()
        .map(|r| r.0)
        .chain(packed.iter().copied())
        .collect();
    let min = ids.iter().copied().min().unwrap_or(3);
    let max = ids.iter().copied().max().unwrap_or(3);
    let mut entries: Vec<(u32, u32)> = runs.to_vec();
    if !packed.is_empty() {
        entries.push((u32::MAX, packed.len() as u32));
    }
    let mut idf = (entries.len() as u64).to_le_bytes().to_vec();
    for (value, repeat) in &entries {
        idf.extend_from_slice(&value.to_le_bytes());
        idf.extend_from_slice(&repeat.to_le_bytes());
    }
    let word = packed
        .iter()
        .enumerate()
        .fold(0u64, |w, (i, id)| w | (u64::from(id - min) << i));
    let words: Vec<u64> = if packed.is_empty() {
        Vec::new()
    } else {
        vec![word]
    };
    idf.extend_from_slice(&(words.len() as u64).to_le_bytes());
    for w in words {
        idf.extend_from_slice(&w.to_le_bytes());
    }
    let xml = format!(
        "<XMObject class=\"XMColumnSegment\"><Properties><Records>{rows}</Records></Properties><Members><Member><Name>SubSegment</Name><XMObject class=\"XMColumnSegment\"><Properties><Records>{}</Records></Properties></XMObject></Member><Member><Name>CompressionInfo</Name><XMObject class=\"XMHybridRLECompressionInfo&lt;class XMRENoSplitCompressionInfo&lt;1&gt;&gt;\"><Members><Member><Name>RLECompression</Name><XMObject class=\"XMRLECompressionInfo\"><Properties><BookmarkBits>0</BookmarkBits><StorageAllocSize>{n}</StorageAllocSize><StorageUsedSize>{n}</StorageUsedSize><SegmentNeedsResizing>false</SegmentNeedsResizing></Properties></XMObject></Member><Member><Name>SubCompression</Name><XMObject class=\"XMRENoSplitCompressionInfo&lt;1&gt;\"><Properties><Min>{min}</Min></Properties></XMObject></Member></Members></XMObject></Member><Member><Name>ColumnSegmentStats</Name><XMObject class=\"XMColumnSegmentStats\"><Properties><MinDataID>{min}</MinDataID><MaxDataID>{max}</MaxDataID><RowCount>{rows}</RowCount><HasNulls>false</HasNulls></Properties></XMObject></Member></Members></XMObject>",
        packed.len(),
        n = entries.len(),
    );
    (xml, idf)
}

/// `XMRawColumn` XML for column `id` with one segment and its data objects.
fn raw_column(id: &str, segment: &str, dictionary: &str, idf: &str) -> String {
    format!(
        "<XMObject class=\"XMRawColumn\" name=\"{id}\"><Properties><Settings>1025</Settings></Properties><Members><Member><Name>ColumnStats</Name><XMObject class=\"XMColumnStats\"><Properties><DistinctStates>2</DistinctStates><HasNulls>false</HasNulls></Properties></XMObject></Member></Members><Collections><Collection><Name>Segments</Name>{segment}</Collection></Collections><DataObjects><DataObject>{dictionary}</DataObject><DataObject><XMObject class=\"XMRawColumnPartitionDataObject\" name=\"{}\"><Properties><Partition>0</Partition><SegmentCount>1</SegmentCount></Properties></XMObject></DataObject></DataObjects></XMObject>",
        esc(idf)
    )
}

fn value_dictionary(kind: &str, magnitude: f64) -> String {
    format!(
        "<XMObject class=\"XMValueDataDictionary&lt;{kind}&gt;\"><Properties><DataVersion>0</DataVersion><BaseId>0</BaseId><Magnitude>{magnitude:?}</Magnitude></Properties></XMObject>"
    )
}

fn hash_dictionary(kind: &str, file: &str) -> String {
    format!(
        "<XMObject class=\"XMHashDataDictionary&lt;{kind}&gt;\" name=\"{}\"><Properties><DataVersion>0</DataVersion><LastId>4</LastId><Nullable>false</Nullable><Unique>true</Unique><DictionaryFlags>3</DictionaryFlags></Properties></XMObject>",
        esc(file)
    )
}

fn simple_table(dim: &str, rows: u64, columns: &str) -> Vec<u8> {
    format!(
        "<XMObject class=\"XMSimpleTable\" name=\"{dim}\"><Members><Member><Name>SegmentMap</Name><XMObject class=\"XMMultiPartSegmentMap\"><Collections><Collection><Name>Partitions</Name><XMObject class=\"XMSegment1Map\"><Properties><Records>{rows}</Records></Properties></XMObject></Collection></Collections></XMObject></Member></Members><Collections><Collection><Name>Columns</Name>{columns}</Collection></Collections></XMObject>"
    )
    .into_bytes()
}

/// A Power Pivot style backup (`xl/model/item.data`): legacy XML metadata for two
/// tables (`Sales` with a calculated column and a measure, `Product`), one relationship,
/// `.tbl.xml` storage with value- and hash-encoded columns, `ApplyCompression` with one
/// genuinely compressed file, and CRC end markers.
///
/// Decoded data: `Sales` = ProductKey `[10, 10, 20]`, Net Amount `[12.5, 12.5, 7.25]`
/// (decimal), Date `[45000, 45001, 45000]` (OLE days), Year `[2023; 3]`; `Product` =
/// ProductKey `[10, 20]`, Product Name `["Widget", "Gadget"]`.
pub fn power_pivot_sample() -> Vec<u8> {
    use crate::vertipaq::test_support::{PageSpec, real_dictionary, string_dictionary};

    let sales = "Sales_0a1b";
    let product = "Product_9f8e";
    let db = format!(
        "<Load {ASSL_NS}><ObjectDefinition><Database><Name>PowerPivotDb</Name><ID>pp1</ID><ddl200:CompatibilityLevel>1103</ddl200:CompatibilityLevel><DataSourceImpersonationInfo><Password>not-a-secret</Password></DataSourceImpersonationInfo></Database></ObjectDefinition></Load>"
    );
    let cube = format!(
        "<Load {ASSL_NS}><ParentObject><DatabaseID>pp1</DatabaseID></ParentObject><ObjectDefinition><Cube><Name>Model</Name><ID>Model</ID><Dimensions><Dimension><ID>{sales}</ID><Name>Sales</Name><DimensionID>{sales}</DimensionID><Visible>true</Visible></Dimension><Dimension><ID>{product}</ID><Name>Product</Name><DimensionID>{product}</DimensionID><Visible>false</Visible></Dimension></Dimensions></Cube></ObjectDefinition></Load>"
    );
    let row_number = r#"<Attribute><Name>RowNumber</Name><ID>RowNumber</ID><Type>RowNumber</Type><KeyColumns><KeyColumn><DataType>Integer</DataType><Source xsi:type="ddl200_200:RowNumberBinding"/></KeyColumn></KeyColumns><AttributeHierarchyVisible>false</AttributeHierarchyVisible></Attribute>"#;
    let sales_dim = format!(
        "<Load {ASSL_NS}><ParentObject><DatabaseID>pp1</DatabaseID></ParentObject><ObjectDefinition><Dimension><Name>Sales</Name><ID>{sales}</ID><Description>Fact &amp; detail</Description><Attributes>{row_number}{}{}{}{}</Attributes><ddl300_300:Relationships><ddl300_300:Relationship><ID>rel-1</ID><Visible>true</Visible><ddl300_300:FromRelationshipEnd><ddl300_300:Multiplicity>Many</ddl300_300:Multiplicity><DimensionID>{sales}</DimensionID><Attributes><Attribute><AttributeID>ProductKey</AttributeID></Attribute></Attributes></ddl300_300:FromRelationshipEnd><ddl300_300:ToRelationshipEnd><ddl300_300:Multiplicity>One</ddl300_300:Multiplicity><DimensionID>{product}</DimensionID><Attributes><Attribute><AttributeID>ProductKey</AttributeID></Attribute></Attributes></ddl300_300:ToRelationshipEnd></ddl300_300:Relationship></ddl300_300:Relationships></Dimension></ObjectDefinition></Load>",
        attribute(
            "ProductKey",
            "ProductKey",
            "BigInt",
            &column_binding(sales, "ProductKey"),
            true
        ),
        attribute(
            "Amount",
            "Net Amount",
            "Currency",
            &column_binding(sales, "Net_x0020_Amount"),
            true
        ),
        attribute(
            "Date",
            "Date",
            "Date",
            &column_binding(sales, "Date"),
            false
        ),
        attribute(
            "CalculatedColumn1",
            "Year",
            "Empty",
            r#"<Source xsi:type="ddl200_200:ExpressionBinding"><Expression>YEAR([Date])</Expression></Source>"#,
            true
        ),
    );
    let product_dim = format!(
        "<Load {ASSL_NS}><ParentObject><DatabaseID>pp1</DatabaseID></ParentObject><ObjectDefinition><Dimension><Name>Product</Name><ID>{product}</ID><Attributes>{row_number}{}{}</Attributes><Hierarchies><Hierarchy><Name>Catalog</Name><Levels><Level><Name>Product</Name><SourceAttributeID>Name</SourceAttributeID></Level></Levels></Hierarchy></Hierarchies></Dimension></ObjectDefinition></Load>",
        attribute(
            "ProductKey",
            "ProductKey",
            "BigInt",
            &column_binding(product, "ProductKey"),
            true
        ),
        attribute(
            "Name",
            "Product Name",
            "WChar",
            &column_binding(product, "Name"),
            true
        ),
    );
    let script = format!(
        "<Load {ASSL_NS}><ParentObject><DatabaseID>pp1</DatabaseID><CubeID>Model</CubeID></ParentObject><ObjectDefinition><MdxScript><Name>MdxScript</Name><ID>MdxScript</ID><Commands><Command><Text>CALCULATE;\nCREATE MEMBER CURRENTCUBE.Measures.[__XL_Count of Models] AS 1, VISIBLE = 0;</Text></Command><Command><Text>-- PowerPivot measures command (do not modify manually) --\n\nCREATE MEASURE 'Sales'[Total Sales]=SUM('Sales'[Net Amount]);\n</Text><Annotations><Annotation><Name>FullName</Name><Value>Total Sales</Value></Annotation><Annotation><Name>Table</Name><Value>Sales</Value></Annotation></Annotations></Command></Commands></MdxScript></ObjectDefinition></Load>"
    );
    let partition = |table: &str, name: &str| {
        format!(
            r#"<Load {ASSL_NS}><ParentObject><DatabaseID>pp1</DatabaseID><CubeID>Model</CubeID><MeasureGroupID>{table}</MeasureGroupID></ParentObject><ObjectDefinition><Partition><Name>{name}</Name><ID>{table}</ID><Source xsi:type="QueryBinding"><DataSourceID>ds-1</DataSourceID><QueryDefinition>SELECT [{name}$].* FROM [{name}$]</QueryDefinition></Source></Partition></ObjectDefinition></Load>"#
        )
    };
    let data_source = format!(
        r#"<Load {ASSL_NS}><ObjectDefinition><DataSource xsi:type="RelationalDataSource"><ID>ds-1</ID><Name>Workbook</Name><ConnectionString>Provider=Microsoft.ACE.OLEDB.12.0;Password=hunter2</ConnectionString></DataSource></ObjectDefinition></Load>"#
    );
    // The MDX script is stored as genuinely compressed chunks: a leading XML comment
    // `<!--abcabcabcabc-->` whose repeat is one LZ77 match, then stored chunks.
    let mut script_content = b"<!--abcabcabcabc-->".to_vec();
    script_content.extend_from_slice(script.as_bytes());
    let mut packed = crate::xpress8::test_support::chunk(
        16,
        &crate::xpress8::test_support::literal_then_match(b"<!--abc", 3, 9),
    );
    packed.extend(crate::xpress8::test_support::stored_chunks(
        &script_content[16..],
        500,
    ));

    // Storage.
    let idf = |dim: &str, column: &str| format!("0.{dim}.{column}.0.idf");
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut sales_columns = String::new();
    for (id, runs, dictionary) in [
        (
            "ProductKey",
            vec![(10, 2), (20, 1)],
            value_dictionary("XM_Long", 1.0),
        ),
        (
            "Amount",
            vec![(125_000, 2), (72_500, 1)],
            value_dictionary("XM_Long", 1.0),
        ),
        (
            "Date",
            vec![(3, 1), (4, 1), (3, 1)],
            hash_dictionary("XM_Real", &format!("0.{sales}.Date.dictionary")),
        ),
        (
            "CalculatedColumn1",
            vec![(2023, 3)],
            value_dictionary("XM_Long", 1.0),
        ),
    ] {
        let (segment, bytes) = rle_segment(&runs, &[]);
        sales_columns.push_str(&raw_column(id, &segment, &dictionary, &idf(sales, id)));
        files.push((format!(r"{sales}.0.dim\{}", idf(sales, id)), bytes));
    }
    files.push((
        format!(r"{sales}.0.dim\0.{sales}.Date.dictionary"),
        real_dictionary(&[45000.0, 45001.0]),
    ));
    let mut product_columns = String::new();
    let (segment, bytes) = rle_segment(&[(10, 1), (20, 1)], &[]);
    product_columns.push_str(&raw_column(
        "ProductKey",
        &segment,
        &value_dictionary("XM_Long", 1.0),
        &idf(product, "ProductKey"),
    ));
    files.push((
        format!(r"{product}.0.dim\{}", idf(product, "ProductKey")),
        bytes,
    ));
    let (segment, bytes) = rle_segment(&[], &[3, 4]);
    product_columns.push_str(&raw_column(
        "Name",
        &segment,
        &hash_dictionary("XM_String", &format!("0.{product}.Name.dictionary")),
        &idf(product, "Name"),
    ));
    files.push((format!(r"{product}.0.dim\{}", idf(product, "Name")), bytes));
    files.push((
        format!(r"{product}.0.dim\0.{product}.Name.dictionary"),
        string_dictionary(&[PageSpec::Plain(vec!["Widget".into(), "Gadget".into()])]),
    ));

    let mut builder = AbfBuilder::new()
        .file("pp1.1.db.xml", db.into_bytes())
        .file("Model.0.cub.xml", cube.into_bytes())
        .file(&format!("{sales}.1.dim.xml"), sales_dim.into_bytes())
        .file(&format!("{product}.1.dim.xml"), product_dim.into_bytes())
        .file_stored_as(r"Model.0.cub\MdxScript.0.scr.xml", script_content, packed)
        .file(
            &format!(r"Model.0.cub\{sales}.0.det\{sales}.1.prt.xml"),
            partition(sales, "Sales").into_bytes(),
        )
        .file(
            &format!(r"Model.0.cub\{product}.0.det\{product}.1.prt.xml"),
            partition(product, "Product").into_bytes(),
        )
        .file("ds-1.0.ds.xml", data_source.into_bytes())
        .file(
            &format!(r"{sales}.0.dim\{sales}.0.tbl.xml"),
            simple_table(sales, 3, &sales_columns),
        )
        .file(
            &format!(r"{product}.0.dim\{product}.0.tbl.xml"),
            simple_table(product, 2, &product_columns),
        );
    for (path, bytes) in files {
        builder = builder.file(&path, bytes);
    }
    builder.apply_compression().crc_markers().build()
}
