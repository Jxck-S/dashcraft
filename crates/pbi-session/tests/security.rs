#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Row-level security ("View as") and calculation groups through the
//! session API, on an in-memory model (no fixtures needed).
//!
//! ```text
//! Region (1) ──< Sales >── (1) Customer      (Sales → Customer: cross-filter both)
//!
//! Sales row Region Customer Amount     Customer Segment   Email
//! 0         East   C1       10         C1       Retail    ann@contoso.com
//! 1         West   C1       20         C2       Corporate bob@contoso.com
//! 2         East   C2        5         C3       Retail    cat@contoso.com
//! 3         North  C3        7
//! 4         West   C3        3         Region: East, West, North
//! 5         South  C2      100         (South has no Region row: blank row)
//! ```

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::TimeZone;
use pbi_session::{
    CancellationToken, ColumnFilterSpec, FilterSpec, FixedClock, RefreshOptions, Session,
    SessionError, SessionOptions, TableStatus, ViewAs,
};

fn bim(security: &str) -> String {
    let security = match security {
        "bothDirections" => r#", "securityFilteringBehavior": "bothDirections""#,
        _ => "",
    };
    format!(
        r##"{{"compatibilityLevel": 1567, "model": {{"culture": "en-US",
  "tables": [
    {{"name": "Region",
      "columns": [{{"name": "Region", "dataType": "string", "sourceColumn": "Region"}}],
      "partitions": [{{"name": "Region", "mode": "import", "source": {{"type": "m",
        "expression": "#table(type table [Region = text], {{{{\"East\"}}, {{\"West\"}}, {{\"North\"}}}})"}}}}]}},
    {{"name": "Customer",
      "columns": [
        {{"name": "Customer", "dataType": "string", "sourceColumn": "Customer"}},
        {{"name": "Segment", "dataType": "string", "sourceColumn": "Segment"}},
        {{"name": "Email", "dataType": "string", "sourceColumn": "Email"}}],
      "partitions": [{{"name": "Customer", "mode": "import", "source": {{"type": "m",
        "expression": "#table(type table [Customer = text, Segment = text, Email = text], {{{{\"C1\", \"Retail\", \"ann@contoso.com\"}}, {{\"C2\", \"Corporate\", \"bob@contoso.com\"}}, {{\"C3\", \"Retail\", \"cat@contoso.com\"}}}})"}}}}]}},
    {{"name": "Sales",
      "columns": [
        {{"name": "Region", "dataType": "string", "sourceColumn": "Region"}},
        {{"name": "Customer", "dataType": "string", "sourceColumn": "Customer"}},
        {{"name": "Amount", "dataType": "int64", "sourceColumn": "Amount"}}],
      "partitions": [{{"name": "Sales", "mode": "import", "source": {{"type": "m",
        "expression": "#table(type table [Region = text, Customer = text, Amount = Int64.Type], {{{{\"East\", \"C1\", 10}}, {{\"West\", \"C1\", 20}}, {{\"East\", \"C2\", 5}}, {{\"North\", \"C3\", 7}}, {{\"West\", \"C3\", 3}}, {{\"South\", \"C2\", 100}}}})"}}}}],
      "measures": [
        {{"name": "Total", "expression": "SUM(Sales[Amount])", "formatString": "#,0"}},
        {{"name": "Rows", "expression": "COUNTROWS(Sales)"}},
        {{"name": "Customers", "expression": "COUNTROWS(Customer)"}},
        {{"name": "Regions", "expression": "COUNTROWS(Region)"}},
        {{"name": "All Total", "expression": "CALCULATE([Total], ALL(Sales))"}},
        {{"name": "Me", "expression": "USERPRINCIPALNAME()"}}]}},
    {{"name": "Scale",
      "calculationGroup": {{"precedence": 1, "calculationItems": [
        {{"name": "Same", "expression": "SELECTEDMEASURE()", "ordinal": 0}},
        {{"name": "Double", "expression": "SELECTEDMEASURE() * 2", "ordinal": 1,
          "formatStringDefinition": {{"expression": "\"0.00\""}}}}]}},
      "columns": [
        {{"name": "Name", "dataType": "string", "sourceColumn": "Name", "sortByColumn": "Ordinal"}},
        {{"name": "Ordinal", "dataType": "int64", "isHidden": true, "sourceColumn": "Ordinal"}}],
      "partitions": [{{"name": "Scale", "mode": "import", "source": {{"type": "calculationGroup"}}}}]}}
  ],
  "relationships": [
    {{"name": "r1", "fromTable": "Sales", "fromColumn": "Region", "toTable": "Region", "toColumn": "Region"}},
    {{"name": "r2", "fromTable": "Sales", "fromColumn": "Customer", "toTable": "Customer", "toColumn": "Customer",
      "crossFilteringBehavior": "bothDirections"{security}}}
  ],
  "roles": [
    {{"name": "East", "modelPermission": "read",
      "tablePermissions": [{{"name": "Region", "filterExpression": "[Region] = \"East\""}}]}},
    {{"name": "Retail", "modelPermission": "read",
      "tablePermissions": [{{"name": "Customer", "filterExpression": "Customer[Segment] = \"Retail\""}}]}},
    {{"name": "Self", "modelPermission": "read",
      "tablePermissions": [{{"name": "Customer", "filterExpression": "[Email] = USERPRINCIPALNAME()"}}]}},
    {{"name": "Broken", "modelPermission": "read",
      "tablePermissions": [{{"name": "Customer", "filterExpression": "Customer[Nope] = 1"}}]}}
  ]
}}}}"##
    )
}

async fn session(security: &str) -> Session {
    let opened = pbi_core::open_bytes(
        "model.bim",
        bim(security).into_bytes(),
        &pbi_core::OpenOptions::default(),
    )
    .unwrap();
    assert!(
        opened.document.model.is_some(),
        "{:?}",
        opened.document.diagnostics
    );
    let options = SessionOptions {
        clock: Some(Arc::new(FixedClock::utc(
            chrono::Utc
                .with_ymd_and_hms(2026, 10, 21, 14, 5, 0)
                .single()
                .unwrap(),
        ))),
        ..SessionOptions::default()
    };
    let session = Session::from_opened(opened, options).unwrap();
    let report = session.refresh(RefreshOptions::all()).await.unwrap();
    for t in &report.tables {
        assert_eq!(t.status, TableStatus::Ok, "{}: {:?}", t.name, t.status);
    }
    session
}

/// Measure name → displayed value.
async fn values(session: &Session, filter: &FilterSpec) -> BTreeMap<String, String> {
    session
        .measures(filter, CancellationToken::new())
        .await
        .unwrap()
        .into_iter()
        .map(|m| (m.name.clone(), m.display()))
        .collect()
}

/// `(Total, Rows, Customers, Regions, All Total)`.
async fn kpis(session: &Session) -> [String; 5] {
    let v = values(session, &FilterSpec::new()).await;
    ["Total", "Rows", "Customers", "Regions", "All Total"].map(|k| v[k].clone())
}

fn kpi(values: [&str; 5]) -> [String; 5] {
    values.map(str::to_owned)
}

#[tokio::test(flavor = "multi_thread")]
async fn roles_restrict_every_query_and_union() {
    let s = session("oneDirection").await;
    // No view: 6 sales (145), 3 customers, 3 regions (the blank row of the
    // unmatched "South" is not a row of COUNTROWS(Region)).
    assert_eq!(kpis(&s).await, kpi(["145", "6", "3", "3", "145"]));

    // East: Region keeps East; Sales keeps its East rows (0, 2 → 15); the
    // unmatched South sale relates to Region's blank row, which the filter
    // rejects. Security flows Sales → Customer only with
    // securityFilteringBehavior bothDirections: all 3 customers stay.
    // ALL(Sales) cannot remove security.
    s.set_effective_roles(&["east"], None).await.unwrap();
    assert_eq!(s.view_as().unwrap().roles, ["East"]);
    assert_eq!(kpis(&s).await, kpi(["15", "2", "3", "1", "15"]));

    // Retail: customers C1, C3 → their sales 0, 1, 3, 4 (10 + 20 + 7 + 3);
    // Region is not restricted (no Sales → Region propagation).
    s.set_effective_roles(&["Retail"], None).await.unwrap();
    assert_eq!(kpis(&s).await, kpi(["40", "4", "2", "3", "40"]));

    // East + Retail: the union per table. Sales {0, 2} ∪ {0, 1, 3, 4};
    // the South sale (C2, no region) is in neither. Customers: East keeps
    // all three. Regions: Retail keeps all three.
    s.set_effective_roles(&["East", "Retail"], None)
        .await
        .unwrap();
    assert_eq!(kpis(&s).await, kpi(["45", "5", "3", "3", "45"]));

    // Every query path sees the restricted data: DAX queries, the grid,
    // SQL, the snapshot visuals read.
    s.set_effective_roles(&["East"], None).await.unwrap();
    let batches = s
        .dax_query(
            "EVALUATE SUMMARIZECOLUMNS(Region[Region], \"T\", [Total])",
            &FilterSpec::new(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(batches[0].num_rows(), 1);
    assert_eq!(
        s.row_count("Sales", None, CancellationToken::new())
            .await
            .unwrap(),
        2
    );
    assert_eq!(s.snapshot().row_count("Sales").unwrap(), 2);
    assert_eq!(
        s.tables().iter().find(|t| t.name == "Sales").unwrap().rows,
        2
    );

    // A refresh works on all rows and re-applies the view.
    s.refresh(RefreshOptions::all()).await.unwrap();
    assert_eq!(kpis(&s).await, kpi(["15", "2", "3", "1", "15"]));

    s.clear_view_as();
    assert!(s.view_as().is_none());
    assert_eq!(kpis(&s).await, kpi(["145", "6", "3", "3", "145"]));
}

#[tokio::test(flavor = "multi_thread")]
async fn security_filtering_behavior_controls_the_reverse_direction() {
    // With bothDirections, East's Sales rows (C1, C2) restrict Customer.
    let s = session("bothDirections").await;
    s.set_effective_roles(&["East"], None).await.unwrap();
    assert_eq!(kpis(&s).await, kpi(["15", "2", "2", "1", "15"]));
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_rls_uses_the_effective_user() {
    let s = session("oneDirection").await;
    // [Email] = USERPRINCIPALNAME(): cat sees C3 and its sales 3, 4.
    s.set_effective_roles(&["Self"], Some("cat@contoso.com".into()))
        .await
        .unwrap();
    let v = values(&s, &FilterSpec::new()).await;
    assert_eq!(
        (v["Total"].as_str(), v["Rows"].as_str(), v["Me"].as_str()),
        ("10", "2", "cat@contoso.com")
    );
    // An unknown user sees nothing.
    s.set_effective_roles(&["Self"], Some("eve@contoso.com".into()))
        .await
        .unwrap();
    let v = values(&s, &FilterSpec::new()).await;
    assert_eq!((v["Total"].as_str(), v["Customers"].as_str()), ("", ""));
    // A user without roles: identity only, no row-level security.
    s.set_view_as(
        ViewAs {
            user_name: Some("ann@contoso.com".into()),
            custom_data: Some("x".into()),
            ..ViewAs::default()
        },
        CancellationToken::new(),
    )
    .await
    .unwrap();
    let v = values(&s, &FilterSpec::new()).await;
    assert_eq!(
        (v["Total"].as_str(), v["Me"].as_str()),
        ("145", "ann@contoso.com")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_roles_and_failing_filters_are_errors() {
    let s = session("oneDirection").await;
    s.set_effective_roles(&["East"], None).await.unwrap();
    let err = s.set_effective_roles(&["Nope"], None).await.unwrap_err();
    assert!(
        matches!(&err, SessionError::UnknownRole(r) if r == "Nope"),
        "{err}"
    );
    let err = s.set_effective_roles(&["Broken"], None).await.unwrap_err();
    assert!(
        matches!(&err, SessionError::RoleFilter { role, table, .. } if role == "Broken" && table == "Customer"),
        "{err}"
    );
    // The previous view stays.
    assert_eq!(s.view_as().unwrap().roles, ["East"]);
    assert_eq!(kpis(&s).await[0], "15");
    assert!(s.view_as_error().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn calculation_group_tables_load_and_apply_with_format_strings() {
    let s = session("oneDirection").await;
    let scale = s.tables().into_iter().find(|t| t.name == "Scale").unwrap();
    assert_eq!(scale.rows, 2);
    let double = FilterSpec::new().with(ColumnFilterSpec::include("Scale", "Name", ["Double"]));
    let total = s
        .evaluate_measure("Total", &double, CancellationToken::new())
        .await
        .unwrap();
    // 145 × 2 with the item's format string "0.00".
    assert_eq!(total.display(), "290.00");
    assert_eq!(total.format_string.as_deref(), Some("0.00"));
    let same = FilterSpec::new().with(ColumnFilterSpec::include("Scale", "Name", ["Same"]));
    let total = s
        .evaluate_measure("Total", &same, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(
        (total.display().as_str(), total.format_string.as_deref()),
        ("145", Some("#,0"))
    );
    // With "View as" too: (East 15) × 2.
    s.set_effective_roles(&["East"], None).await.unwrap();
    let total = s
        .evaluate_measure("Total", &double, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(total.display(), "30.00");
}
