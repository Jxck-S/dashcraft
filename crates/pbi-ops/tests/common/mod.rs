//! Helpers of the `pbi-ops` integration tests: fixture paths, the expected
//! values of the synthetic Acme Outfitters example, a runner and a small
//! synthetic PBIP project with inline M data.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use pbi_ops::{CallerKind, Ctx, OpError, Registry, Workspace};
use serde_json::{Value, json};

/// `$DASHCRAFT_REFERENCE`, else `<workspace>/reference`.
pub fn reference_root() -> PathBuf {
    std::env::var_os("DASHCRAFT_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference"))
}

/// A fixture under the reference root, or `None` (after printing `SKIP:`)
/// when it is missing.
pub fn fixture(rel: &str) -> Option<PathBuf> {
    let path = reference_root().join(rel);
    if path.exists() {
        Some(path)
    } else {
        println!("SKIP: {} missing", path.display());
        None
    }
}

/// The Acme Outfitters example (`reference/examples/acme-outfitters/`).
pub const ACME: &str = "examples/acme-outfitters";
pub const PBIP: &str = "examples/acme-outfitters/AcmeOutfitters.pbip";
pub const BLOB_MAP: &str = "examples/acme-outfitters/blob-map.json";
pub const EXPECTED: &str = "examples/acme-outfitters/expected.json";
/// The raw orders CSV (before the M clean-up steps).
pub const ORDERS_CSV: &str = "examples/acme-outfitters/data/orders.csv";
/// A committed Microsoft sample (MIT) with a VertiPaq data model.
pub const MS_PBIX: &str = "samples/microsoft/Revenue Opportunities.pbix";
pub const MS_EXPECTED: &str = "samples/microsoft/expected.json";

pub fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

/// The example's `expected.json` (values computed by its generator,
/// independently of DashCraft), or `None` (after `SKIP:`).
pub fn expected() -> Option<Expected> {
    fixture(EXPECTED).map(|p| Expected(read_json(&p)))
}

/// The `files[]` entry of `samples/microsoft/expected.json` for `file`.
pub fn ms_expected(file: &str) -> Option<Value> {
    let all = read_json(&fixture(MS_EXPECTED)?);
    all["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["file"] == file)
        .cloned()
}

/// Accessors over the Acme `expected.json`.
pub struct Expected(pub Value);

impl Expected {
    /// The full report name ending with `short` (`Stores` → `Acme Outfitters - Stores`).
    pub fn report_name(&self, short: &str) -> String {
        self.report(short)["report"].as_str().unwrap().to_owned()
    }

    pub fn report(&self, short: &str) -> &Value {
        self.0["reports"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["report"].as_str().unwrap().ends_with(short))
            .unwrap_or_else(|| panic!("no report {short} in expected.json"))
    }

    pub fn page(&self, report: &str, page: &str) -> &Value {
        self.report(report)["pages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["page"] == page)
            .unwrap_or_else(|| panic!("no page {report}/{page} in expected.json"))
    }

    /// The formatted value of `measure` on the card `visual_key` of a page.
    pub fn card(&self, report: &str, page: &str, visual_key: &str, measure: &str) -> &str {
        self.page(report, page)["cards"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["visualKey"] == visual_key && c["measure"] == measure)
            .unwrap_or_else(|| panic!("no card {visual_key}/{measure} on {report}/{page}"))
            ["formatted"]
            .as_str()
            .unwrap()
    }

    /// A non-card visual of a page (`visualKey`).
    pub fn visual(&self, report: &str, page: &str, visual_key: &str) -> &Value {
        self.page(report, page)["visuals"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["visualKey"] == visual_key)
            .unwrap_or_else(|| panic!("no visual {visual_key} on {report}/{page}"))
    }

    /// The visual name (folder) of the card or visual `visual_key` of a page.
    pub fn visual_id(&self, report: &str, page: &str, visual_key: &str) -> String {
        let page = self.page(report, page);
        ["cards", "visuals", "slicers"]
            .iter()
            .flat_map(|key| page[*key].as_array().into_iter().flatten())
            .find(|v| v["visualKey"] == visual_key)
            .unwrap_or_else(|| panic!("no visual {visual_key} on {}", page["page"]))["visual"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// Every measure without filters: `(name, value, formatted)`.
    pub fn measures(&self) -> Vec<(&str, &Value, &str)> {
        Self::entries(&self.0["measures"]["unfiltered"])
    }

    /// Every measure under the report-level filters of `report`.
    pub fn report_measures(&self, report: &str) -> Vec<(&str, &Value, &str)> {
        let name = self.report_name(report);
        Self::entries(&self.0["measures"]["byReport"][name.as_str()]["values"])
    }

    /// The formatted value of `measure` (unfiltered, or under a report's filters).
    pub fn measure(&self, report: Option<&str>, measure: &str) -> &str {
        let list = match report {
            Some(r) => self.report_measures(r),
            None => self.measures(),
        };
        list.into_iter()
            .find(|(n, ..)| *n == measure)
            .unwrap_or_else(|| panic!("no measure {measure} in expected.json"))
            .2
    }

    fn entries(values: &Value) -> Vec<(&str, &Value, &str)> {
        values
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.as_str(), &v["value"], v["formatted"].as_str().unwrap()))
            .collect()
    }

    /// Rows per table after the M steps and calculated tables.
    pub fn row_counts(&self) -> Vec<(&str, u64)> {
        self.0["rowCounts"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_u64().unwrap()))
            .collect()
    }

    pub fn rows(&self, table: &str) -> u64 {
        self.0["rowCounts"][table].as_u64().unwrap()
    }
}

/// Copies a folder recursively.
pub fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// A fresh folder under the test target dir.
pub fn tmp_dir(prefix: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap()
}

/// Runs commands on one workspace with a CLI caller.
pub struct Runner {
    pub registry: Registry,
    pub ws: Workspace,
    pub ctx: Ctx,
}

impl Runner {
    pub fn new() -> Self {
        Self::with(Registry::default_catalog(), Workspace::new())
    }

    pub fn with(registry: Registry, ws: Workspace) -> Self {
        Self {
            registry,
            ws,
            ctx: Ctx::new(CallerKind::Cli),
        }
    }

    pub async fn try_run(&self, id: &str, params: Value) -> Result<Value, OpError> {
        self.registry.execute(&self.ws, id, params, &self.ctx).await
    }

    pub async fn run(&self, id: &str, params: Value) -> Value {
        match self.try_run(id, params.clone()).await {
            Ok(v) => v,
            Err(e) => panic!("{id} {params} failed: {e:?}"),
        }
    }

    /// Opens a document and returns its doc id.
    pub async fn open(&self, params: Value) -> String {
        let v = self.run("doc.open", params).await;
        v["docId"].as_str().unwrap().to_owned()
    }
}

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn write_json(root: &Path, rel: &str, value: &Value) {
    write(root, rel, &serde_json::to_string_pretty(value).unwrap());
}

fn column(table: &str, column: &str) -> Value {
    json!({"Column": {"Expression": {"SourceRef": {"Entity": table}}, "Property": column}})
}

fn measure(table: &str, measure: &str) -> Value {
    json!({"Measure": {"Expression": {"SourceRef": {"Entity": table}}, "Property": measure}})
}

fn projection(field: Value, query_ref: &str, native: &str) -> Value {
    json!({"field": field, "queryRef": query_ref, "nativeQueryRef": native, "active": true})
}

fn visual(page: &str, name: &str, pos: (f64, f64, f64, f64), body: Value) -> (String, Value) {
    (
        format!("Mini.Report/definition/pages/{page}/visuals/{name}/visual.json"),
        json!({
            "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/visualContainer/2.4.0/schema.json",
            "name": name,
            "position": {"x": pos.0, "y": pos.1, "z": 0, "width": pos.2, "height": pos.3, "tabOrder": 0},
            "visual": body
        }),
    )
}

/// Total of `Sales[Amount]` in [`mini_pbip`] (1 + 2 + … + 60).
pub const MINI_TOTAL: i64 = 1830;

/// Writes a small PBIP project: a `Sales` table (inline M, 60 rows: `Item
/// NN`, Region East/West/North, Amount 1…60) with a `Total` measure, and a
/// report with an "Overview" page (card `total`, Region slicer `region`,
/// bar chart `bars`) and a "Details" page (a long table `table`). Returns
/// the `.pbip` path.
pub fn mini_pbip(root: &Path) -> PathBuf {
    write_json(
        root,
        "Mini.pbip",
        &json!({"version": "1.0", "artifacts": [{"report": {"path": "Mini.Report"}}]}),
    );
    write_json(
        root,
        "Mini.SemanticModel/definition.pbism",
        &json!({"version": "4.2", "settings": {}}),
    );
    write(
        root,
        "Mini.SemanticModel/definition/database.tmdl",
        "database\n\tcompatibilityLevel: 1567\n",
    );
    write(
        root,
        "Mini.SemanticModel/definition/model.tmdl",
        "model Model\n\tculture: en-US\n\nref table Sales\n",
    );
    let rows: Vec<String> = (1..=60)
        .map(|i| {
            format!(
                "{{\"Item {i:02}\", \"{}\", {i}}}",
                ["East", "West", "North"][i % 3]
            )
        })
        .collect();
    let sales = format!(
        "table Sales\n\
         \n\
         \tmeasure Total = SUM(Sales[Amount])\n\
         \t\tformatString: 0\n\
         \n\
         \tcolumn Item\n\
         \t\tdataType: string\n\
         \t\tsummarizeBy: none\n\
         \t\tsourceColumn: Item\n\
         \n\
         \tcolumn Region\n\
         \t\tdataType: string\n\
         \t\tsummarizeBy: none\n\
         \t\tsourceColumn: Region\n\
         \n\
         \tcolumn Amount\n\
         \t\tdataType: int64\n\
         \t\tformatString: 0\n\
         \t\tsummarizeBy: sum\n\
         \t\tsourceColumn: Amount\n\
         \n\
         \tpartition Sales = m\n\
         \t\tmode: import\n\
         \t\tsource =\n\
         \t\t\t\tlet\n\
         \t\t\t\t    Source = #table(type table [Item = text, Region = text, Amount = number], {{{}}})\n\
         \t\t\t\tin\n\
         \t\t\t\t    Source\n",
        rows.join(", ")
    );
    write(
        root,
        "Mini.SemanticModel/definition/tables/Sales.tmdl",
        &sales,
    );
    write_json(
        root,
        "Mini.Report/definition.pbir",
        &json!({"version": "4.0", "datasetReference": {"byPath": {"path": "../Mini.SemanticModel"}}}),
    );
    write_json(
        root,
        "Mini.Report/definition/version.json",
        &json!({"$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/versionMetadata/1.0.0/schema.json", "version": "2.0.0"}),
    );
    write_json(
        root,
        "Mini.Report/definition/report.json",
        &json!({"$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/report/3.0.0/schema.json"}),
    );
    write_json(
        root,
        "Mini.Report/definition/pages/pages.json",
        &json!({"pageOrder": ["overview", "details"], "activePageName": "overview"}),
    );
    for (name, display) in [("overview", "Overview"), ("details", "Details")] {
        write_json(
            root,
            &format!("Mini.Report/definition/pages/{name}/page.json"),
            &json!({"name": name, "displayName": display, "displayOption": "FitToPage", "width": 640, "height": 360}),
        );
    }
    let region = || projection(column("Sales", "Region"), "Sales.Region", "Region");
    let item = || projection(column("Sales", "Item"), "Sales.Item", "Item");
    let amount = || {
        projection(
            json!({"Aggregation": {"Expression": column("Sales", "Amount"), "Function": 0}}),
            "Sum(Sales.Amount)",
            "Sum of Amount",
        )
    };
    let visuals = [
        visual(
            "overview",
            "total",
            (10.0, 60.0, 180.0, 90.0),
            json!({"visualType": "cardVisual", "query": {"queryState": {"Data": {"projections": [
                projection(measure("Sales", "Total"), "Sales.Total", "Total")
            ]}}}}),
        ),
        visual(
            "overview",
            "region",
            (440.0, 60.0, 190.0, 90.0),
            json!({"visualType": "slicer", "query": {"queryState": {"Values": {"projections": [region()]}}}}),
        ),
        visual(
            "overview",
            "bars",
            (10.0, 160.0, 620.0, 190.0),
            json!({"visualType": "clusteredBarChart", "query": {"queryState": {
                "Category": {"projections": [region()]},
                "Y": {"projections": [amount()]}
            }}}),
        ),
        visual(
            "details",
            "table",
            (10.0, 10.0, 300.0, 200.0),
            json!({"visualType": "tableEx", "query": {"queryState": {"Values": {"projections": [item(), region(), amount()]}}}}),
        ),
    ];
    for (rel, v) in &visuals {
        write_json(root, rel, v);
    }
    root.join("Mini.pbip")
}
