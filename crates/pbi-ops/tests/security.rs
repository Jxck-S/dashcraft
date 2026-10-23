//! `security.roles` and `security.viewAs` ("View as" row-level security) on
//! the synthetic PBIP of `common::mini_pbip` plus a `Regions` table and four
//! roles (no fixture files needed).
//!
//! ```text
//! Regions (1) ──< Sales      Sales: 60 rows, Region East / West / North,
//! Region  Manager                   Amount 1…60 (Total 1830)
//! East    ann@contoso.com    roles: East    Sales[Region] = "East"
//! West    bob@contoso.com           West    Sales[Region] = "West"
//! North   cat@contoso.com           Manager Regions[Manager] = USERPRINCIPALNAME()
//!                                   Broken  Sales[Nope] = 1
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{
    BLOB_MAP, MINI_TOTAL, MS_PBIX, PBIP, Runner, expected, fixture, mini_pbip, s, tmp_dir,
};
use pbi_ops::{CallerKind, Ctx, OpErrorCode, Registry, Workspace};
use serde_json::{Value, json};

fn sum(region: usize) -> i64 {
    (1..=60i64).filter(|i| (*i % 3) as usize == region).sum()
}

const EAST: usize = 0;
const WEST: usize = 1;

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// [`mini_pbip`] with a `Regions` table related to `Sales` and four roles.
fn secured_pbip(root: &Path) -> PathBuf {
    let pbip = mini_pbip(root);
    let def = "Mini.SemanticModel/definition";
    write(
        root,
        &format!("{def}/model.tmdl"),
        "model Model\n\tculture: en-US\n\nref table Sales\nref table Regions\n\n\
         ref role East\nref role West\nref role Manager\nref role Broken\n",
    );
    write(
        root,
        &format!("{def}/tables/Regions.tmdl"),
        "table Regions\n\
         \n\
         \tcolumn Region\n\
         \t\tdataType: string\n\
         \t\tsummarizeBy: none\n\
         \t\tsourceColumn: Region\n\
         \n\
         \tcolumn Manager\n\
         \t\tdataType: string\n\
         \t\tsummarizeBy: none\n\
         \t\tsourceColumn: Manager\n\
         \n\
         \tpartition Regions = m\n\
         \t\tmode: import\n\
         \t\tsource =\n\
         \t\t\t\tlet\n\
         \t\t\t\t    Source = #table(type table [Region = text, Manager = text], {{\"East\", \"ann@contoso.com\"}, {\"West\", \"bob@contoso.com\"}, {\"North\", \"cat@contoso.com\"}})\n\
         \t\t\t\tin\n\
         \t\t\t\t    Source\n",
    );
    write(
        root,
        &format!("{def}/relationships.tmdl"),
        "relationship r1\n\tfromColumn: Sales.Region\n\ttoColumn: Regions.Region\n",
    );
    write(
        root,
        &format!("{def}/roles/East.tmdl"),
        "/// Sees the East region\n\
         role East\n\
         \tmodelPermission: read\n\
         \n\
         \ttablePermission Sales = [Region] = \"East\"\n\
         \n\
         \tmember 'ann@contoso.com'\n\
         \t\tmemberType: user\n\
         \n\
         \tmember 'dan@contoso.com'\n\
         \t\tmemberType: user\n",
    );
    write(
        root,
        &format!("{def}/roles/West.tmdl"),
        "role West\n\tmodelPermission: read\n\n\ttablePermission Sales = Sales[Region] = \"West\"\n",
    );
    write(
        root,
        &format!("{def}/roles/Manager.tmdl"),
        "role Manager\n\tmodelPermission: read\n\n\ttablePermission Regions = [Manager] = USERPRINCIPALNAME()\n",
    );
    write(
        root,
        &format!("{def}/roles/Broken.tmdl"),
        "role Broken\n\tmodelPermission: read\n\n\ttablePermission Sales = Sales[Nope] = 1\n",
    );
    pbip
}

fn role<'a>(roles: &'a Value, name: &str) -> &'a Value {
    roles["roles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == name)
        .unwrap_or_else(|| panic!("no role {name} in {roles}"))
}

fn table<'a>(out: &'a Value, name: &str) -> &'a Value {
    out["tables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == name)
        .unwrap_or_else(|| panic!("no table {name} in {out}"))
}

/// The `Total` measure as displayed (`""` for BLANK).
async fn total(r: &Runner, doc: &str) -> String {
    let v = r.run("measures.values", json!({"docId": doc})).await;
    v["measures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "Total")
        .unwrap()["display"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn view_as(r: &Runner, doc: &str, params: Value) -> Value {
    let mut params = params;
    params["docId"] = json!(doc);
    r.run("security.viewAs", params).await
}

#[tokio::test(flavor = "multi_thread")]
async fn roles_are_listed_with_filters_members_and_state() {
    let dir = tmp_dir("pbi-ops-security-roles-");
    let pbip = secured_pbip(dir.path());
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip})).await;

    let roles = r.run("security.roles", json!({"docId": doc})).await;
    assert_eq!(roles["roles"].as_array().unwrap().len(), 4, "{roles}");
    let east = role(&roles, "East");
    assert_eq!(east["description"], "Sees the East region");
    assert_eq!(east["modelPermission"], "read");
    assert_eq!(
        east["tablePermissions"],
        json!([{"table": "Sales", "filterExpression": "[Region] = \"East\""}])
    );
    assert_eq!(east["members"], 2);
    assert_eq!(east["dynamic"], false);
    let manager = role(&roles, "Manager");
    assert_eq!(manager["dynamic"], true);
    assert_eq!(manager["tablePermissions"][0]["table"], "Regions");
    assert!(manager.get("members").is_none(), "{manager}");
    assert_eq!(roles["viewAs"], json!({"active": false, "roles": []}));
    // Listing roles loads no data.
    let tables = r.run("data.tables", json!({"docId": doc})).await;
    assert!(
        tables["tables"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["loaded"] == false)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn view_as_restricts_every_query_and_clears() {
    let dir = tmp_dir("pbi-ops-security-view-");
    let pbip = secured_pbip(dir.path());
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip})).await;
    assert_eq!(total(&r, &doc).await, MINI_TOTAL.to_string());
    let g0 = r.run("data.tables", json!({"docId": doc})).await["generation"]
        .as_u64()
        .unwrap();

    // One role (case-insensitive name; the model spelling comes back).
    let out = view_as(&r, &doc, json!({"roles": ["east"]})).await;
    assert_eq!(
        out["viewAs"],
        json!({"active": true, "roles": ["East"]}),
        "{out}"
    );
    assert_eq!(table(&out, "Sales")["rows"], 20);
    assert_eq!(table(&out, "Sales")["totalRows"], 60);
    assert_eq!(table(&out, "Regions")["rows"], 3);
    let g1 = out["generation"].as_u64().unwrap();
    assert!(g1 > g0, "the generation is bumped: {g0} → {g1}");
    assert_eq!(total(&r, &doc).await, sum(EAST).to_string());
    let page = r
        .run(
            "data.page",
            json!({"docId": doc, "table": "Sales", "limit": 100}),
        )
        .await;
    assert_eq!(page["totalRows"], 20);
    assert!(
        page["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row[1] == "East")
    );
    let q = r
        .run(
            "dax.query",
            json!({"docId": doc, "query": "EVALUATE ROW(\"n\", COUNTROWS(ALL(Sales)))"}),
        )
        .await;
    assert_eq!(
        q["tables"][0]["rows"][0][0], 20,
        "ALL cannot escape the role"
    );
    let state = r.run("security.roles", json!({"docId": doc})).await;
    assert_eq!(state["viewAs"]["roles"], json!(["East"]));

    // Several roles: the union of their rows.
    let out = view_as(&r, &doc, json!({"roles": ["East", "West", "EAST"]})).await;
    assert_eq!(out["viewAs"]["roles"], json!(["East", "West"]));
    assert_eq!(table(&out, "Sales")["rows"], 40);
    assert_eq!(total(&r, &doc).await, (sum(EAST) + sum(WEST)).to_string());

    // Empty roles and no user: stop viewing as.
    let out = view_as(&r, &doc, json!({"roles": [], "userName": "  "})).await;
    assert_eq!(out["viewAs"], json!({"active": false, "roles": []}));
    assert_eq!(table(&out, "Sales")["rows"], 60);
    assert!(out["generation"].as_u64().unwrap() > g1);
    assert_eq!(total(&r, &doc).await, MINI_TOTAL.to_string());
    // Clearing again changes nothing.
    let again = view_as(&r, &doc, json!({"roles": []})).await;
    assert_eq!(again["generation"], out["generation"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_roles_use_the_user_name() {
    let dir = tmp_dir("pbi-ops-security-dynamic-");
    let pbip = secured_pbip(dir.path());
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip})).await;

    let out = view_as(
        &r,
        &doc,
        json!({"roles": ["Manager"], "userName": "bob@contoso.com", "customData": "tag"}),
    )
    .await;
    assert_eq!(
        out["viewAs"],
        json!({"active": true, "roles": ["Manager"], "userName": "bob@contoso.com", "customData": "tag"})
    );
    assert_eq!(table(&out, "Regions")["rows"], 1);
    assert_eq!(table(&out, "Sales")["rows"], 20, "Regions filters Sales");
    assert_eq!(total(&r, &doc).await, sum(WEST).to_string());
    let q = r
        .run(
            "dax.query",
            json!({"docId": doc, "query": "EVALUATE ROW(\"u\", USERPRINCIPALNAME(), \"c\", CUSTOMDATA())"}),
        )
        .await;
    assert_eq!(q["tables"][0]["rows"][0], json!(["bob@contoso.com", "tag"]));

    // Without a user the dynamic role matches nobody.
    let out = view_as(&r, &doc, json!({"roles": ["Manager"]})).await;
    assert_eq!(table(&out, "Sales")["rows"], 0);
    assert_eq!(total(&r, &doc).await, "");

    // A user without roles only sets the identity.
    let out = view_as(
        &r,
        &doc,
        json!({"roles": [], "userName": "ann@contoso.com"}),
    )
    .await;
    assert_eq!(
        out["viewAs"],
        json!({"active": true, "roles": [], "userName": "ann@contoso.com"})
    );
    assert_eq!(table(&out, "Sales")["rows"], 60);
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_roles_are_refused_and_keep_the_view() {
    let dir = tmp_dir("pbi-ops-security-errors-");
    let pbip = secured_pbip(dir.path());
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip})).await;
    let applied = view_as(&r, &doc, json!({"roles": ["West"]})).await;

    let e = r
        .try_run(
            "security.viewAs",
            json!({"docId": doc, "roles": ["West", "Nope"]}),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::NotFound, "{e:?}");
    assert!(e.message.contains("'Nope'"), "{}", e.message);
    assert_eq!(
        e.data,
        Some(json!({"roles": ["East", "West", "Manager", "Broken"]}))
    );

    let e = r
        .try_run(
            "security.viewAs",
            json!({"docId": doc, "roles": ["Broken"]}),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::Query, "{e:?}");
    assert!(
        e.message.contains("Broken") && e.message.contains("Nope"),
        "the DAX error: {}",
        e.message
    );
    assert_eq!(e.data, Some(json!({"role": "Broken", "table": "Sales"})));

    // Neither failure changed the view or the data generation.
    let state = r.run("security.roles", json!({"docId": doc})).await;
    assert_eq!(state["viewAs"]["roles"], json!(["West"]));
    let tables = r.run("data.tables", json!({"docId": doc})).await;
    assert_eq!(tables["generation"], applied["generation"]);
    assert_eq!(total(&r, &doc).await, sum(WEST).to_string());

    let e = r
        .try_run("security.viewAs", json!({"docId": doc}))
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::InvalidParams, "roles is required");
    let long = "x".repeat(2000);
    let e = r
        .try_run(
            "security.viewAs",
            json!({"docId": doc, "roles": [], "userName": long}),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::InvalidParams);
}

#[tokio::test(flavor = "multi_thread")]
async fn canvas_renders_after_view_as_show_restricted_values() {
    let dir = tmp_dir("pbi-ops-security-canvas-");
    let pbip = secured_pbip(dir.path());
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip})).await;
    let svg = |page: &Value| page["svg"].as_str().unwrap().to_owned();

    let before = r.run("canvas.render", json!({"docId": doc})).await;
    assert!(svg(&before).contains(&format!(">{MINI_TOTAL}<")));
    // Rendered twice: the page's query results are cached for this data.
    let cached = r.run("canvas.render", json!({"docId": doc})).await;
    assert!(svg(&cached).contains(&format!(">{MINI_TOTAL}<")));

    view_as(&r, &doc, json!({"roles": ["East"]})).await;
    let east = r.run("canvas.render", json!({"docId": doc})).await;
    assert!(
        svg(&east).contains(&format!(">{}<", sum(EAST))),
        "the card shows the East total after View as"
    );
    assert!(!svg(&east).contains(&format!(">{MINI_TOTAL}<")));

    view_as(&r, &doc, json!({"roles": []})).await;
    let all = r.run("canvas.render", json!({"docId": doc})).await;
    assert!(svg(&all).contains(&format!(">{MINI_TOTAL}<")));
}

#[tokio::test(flavor = "multi_thread")]
async fn agents_can_list_roles_and_view_as() {
    let registry = Registry::default_catalog();
    let agent = registry.agent_catalog();
    for id in ["security.roles", "security.viewAs"] {
        let spec = agent.describe(id).unwrap_or_else(|| panic!("{id}"));
        assert!(spec.meta.agent_safe && !spec.meta.writes_files, "{id}");
    }
    assert!(
        registry
            .describe("security.viewAs")
            .unwrap()
            .meta
            .mutates_document,
        "ordered with refreshes and renders of its document"
    );

    let dir = tmp_dir("pbi-ops-security-agent-");
    let pbip = secured_pbip(dir.path());
    let r = Runner {
        registry,
        ws: Workspace::new(),
        ctx: Ctx::new(CallerKind::Agent),
    };
    let doc = r.open(json!({"path": pbip})).await;
    let roles = r.run("security.roles", json!({"docId": doc})).await;
    assert_eq!(roles["roles"].as_array().unwrap().len(), 4);
    let out = view_as(&r, &doc, json!({"roles": ["West"]})).await;
    assert_eq!(table(&out, "Sales")["rows"], 20);
    assert_eq!(total(&r, &doc).await, sum(WEST).to_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn models_without_roles() {
    let dir = tmp_dir("pbi-ops-security-none-");
    let pbip = mini_pbip(dir.path());
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip})).await;
    let roles = r.run("security.roles", json!({"docId": doc})).await;
    assert_eq!(
        roles,
        json!({"roles": [], "viewAs": {"active": false, "roles": []}})
    );
    let e = r
        .try_run("security.viewAs", json!({"docId": doc, "roles": ["Any"]}))
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::NotFound);
    assert!(e.message.contains("no roles"), "{}", e.message);

    // The committed Microsoft sample has no roles either.
    let Some(path) = fixture(MS_PBIX) else { return };
    let doc = r.open(json!({"path": s(&path)})).await;
    let roles = r.run("security.roles", json!({"docId": doc})).await;
    assert_eq!(roles["roles"], json!([]));
}

/// The Acme Outfitters roles: a static `Region West` and a dynamic `Store Manager`
/// (`USERPRINCIPALNAME()`), with the visible rows and measures of its `expected.json`.
#[tokio::test(flavor = "multi_thread")]
async fn example_roles_match_the_expected_values() {
    let (Some(pbip), Some(map), Some(exp)) = (fixture(PBIP), fixture(BLOB_MAP), expected()) else {
        return;
    };
    let r = Runner::new();
    let doc = r
        .open(json!({"path": s(&pbip), "blobMap": s(&map), "source": "query"}))
        .await;
    let roles = r.run("security.roles", json!({"docId": doc})).await;
    let listed: Vec<(&str, bool)> = roles["roles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["name"].as_str().unwrap(), r["dynamic"].as_bool().unwrap()))
        .collect();
    assert_eq!(listed, [("Region West", false), ("Store Manager", true)]);

    for case in exp.0["rls"].as_array().unwrap() {
        let mut params = json!({"docId": doc, "roles": case["roles"]});
        if let Some(user) = case["userName"].as_str() {
            params["userName"] = json!(user);
        }
        let out = r.run("security.viewAs", params).await;
        assert_eq!(out["viewAs"]["active"], true);
        for (name, rows) in case["visibleRows"].as_object().unwrap() {
            assert_eq!(&table(&out, name)["rows"], rows, "{case}: {name}");
        }
        let values = r.run("measures.values", json!({"docId": doc})).await;
        for (name, expected) in case["measures"].as_object().unwrap() {
            let got = values["measures"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["name"] == name.as_str())
                .unwrap();
            // `display` is empty for BLANK, which expected.json writes as "(Blank)".
            let want = if expected["value"].is_null() {
                json!("")
            } else {
                expected["formatted"].clone()
            };
            assert_eq!(got["display"], want, "{case}: {name}");
        }
    }
    let off = r
        .run("security.viewAs", json!({"docId": doc, "roles": []}))
        .await;
    assert_eq!(off["viewAs"]["active"], false);
    assert_eq!(table(&off, "Orders")["rows"], exp.rows("Orders"));
}
