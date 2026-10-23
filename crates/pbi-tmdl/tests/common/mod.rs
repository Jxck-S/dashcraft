//! Shared test data.

/// A Desktop-style TMDL folder in canonical form (see `desktop_sample.rs`).
pub const DESKTOP_SAMPLE: &[(&str, &str)] = &[
    ("database.tmdl", "database\n\tcompatibilityLevel: 1601\n"),
    (
        "model.tmdl",
        r#"model Model
	culture: en-US
	defaultPowerBIDataSourceVersion: powerBI_V3
	discourageImplicitMeasures
	sourceQueryCulture: en-US
	dataAccessOptions
		legacyRedirects
		returnErrorValuesAsNull

queryGroup Parameters

	annotation PBI_QueryGroupOrder = 0

annotation PBI_QueryOrder = ["Server","Sales"]

annotation __PBI_TimeIntelligenceEnabled = 0

ref table Sales
ref table Calendar
ref table 'Time Intelligence'

ref role Readers
ref role Admins
"#,
    ),
    (
        "expressions.tmdl",
        r#"expression Server = "localhost" meta [IsParameterQuery=true, Type="Text", IsParameterQueryRequired=true]
	lineageTag: 6b0b2c2e-0000-4000-8000-000000000001
	queryGroup: Parameters

	annotation PBI_ResultType = Text

expression Sales =
		let
		    Source = Sql.Database(Server, "db")
		in
		    Source
	lineageTag: 6b0b2c2e-0000-4000-8000-000000000002

	annotation PBI_ResultType = Table
"#,
    ),
    (
        "relationships.tmdl",
        "relationship a1\n\tisActive: false\n\tcrossFilteringBehavior: bothDirections\n\tjoinOnDateBehavior: datePartOnly\n\tfromColumn: Sales.OrderDate\n\ttoColumn: Calendar.Date\n\nrelationship a2\n\tfromColumn: Sales.'Customer Key'\n\ttoColumn: Customer.'Customer Key'\n",
    ),
    (
        "functions.tmdl",
        "function AddOne = (x: int64) => x + 1\n\tlineageTag: f1\n",
    ),
    (
        "tables/Sales.tmdl",
        r#"/// Sales facts.
table Sales
	lineageTag: t1

	measure 'Sales Amount' = SUMX(Sales, Sales[Quantity] * Sales[Net Price])
		formatString: \$#,0.00;(\$#,0.00);\$#,0.00
		displayFolder: Totals
		lineageTag: m1

	/// Year to date.
	/// Second line.
	measure YTD =
			VAR d = MAX('Calendar'[Date])

			RETURN
			    TOTALYTD([Sales Amount], 'Calendar'[Date])
		lineageTag: m2
		formatStringDefinition = SELECTEDMEASUREFORMATSTRING()

	measure Padded = ```
			  keep leading spaces  
			```
		isHidden

	column OrderDate
		dataType: dateTime
		formatString: Long Date
		lineageTag: c1
		summarizeBy: none
		sourceColumn: OrderDate
		isAvailableInMdx: false
		changedProperty = IsHidden

		variation Variation
			isDefault
			relationship: a1
			defaultHierarchy: LocalDateTable.'Date Hierarchy'

		annotation SummarizationSetBy = Automatic

	column 'Customer Key'
		dataType: int64
		isHidden
		isKey
		formatString: 0
		lineageTag: c2
		summarizeBy: none
		sourceColumn: CustomerKey
		sortByColumn: 'Order Date'

	hierarchy 'Date Hierarchy'
		lineageTag: h1

		level Year
			lineageTag: l1
			column: Year

	partition Sales = m
		mode: import
		queryGroup: Parameters
		source =
				let
				    Source = Sales
				in
				    Source

	annotation PBI_ResultType = Table
"#,
    ),
    (
        "tables/Calendar.tmdl",
        r#"table Calendar
	lineageTag: t2
	dataCategory: Time

	column Date
		dataType: dateTime
		isNameInferred
		lineageTag: c3
		summarizeBy: none
		sourceColumn: [Date]

	column Year = YEAR('Calendar'[Date])
		dataType: int64
		lineageTag: c4
		summarizeBy: none

	partition Calendar = calculated
		mode: import
		source = CALENDARAUTO()
"#,
    ),
    (
        "tables/Time Intelligence.tmdl",
        r#"table 'Time Intelligence'
	lineageTag: t3

	calculationGroup
		precedence: 1

		calculationItem Current = SELECTEDMEASURE()

		calculationItem YTD =
				CALCULATE(
				    SELECTEDMEASURE(),
				    DATESYTD('Calendar'[Date])
				)
			ordinal: 1

	column 'Time Calculation'
		dataType: string
		lineageTag: c5
		summarizeBy: none
		sourceColumn: Name
		sortByColumn: Ordinal

	partition 'Time Intelligence' = calculationGroup
		mode: import
"#,
    ),
    (
        "roles/Readers.tmdl",
        r#"role Readers
	modelPermission: read

	tablePermission Sales = Sales[Region] = "West"

	member 'user@contoso.com'
		memberType: user
"#,
    ),
    (
        "roles/Admins.tmdl",
        "role Admins\n\tmodelPermission: administrator\n",
    ),
    (
        "cultures/en-US.tmdl",
        r#"cultureInfo en-US

	linguisticMetadata =
			{
			  "Version": "1.0.0",
			  "Language": "en-US"
			}
		contentType: json
"#,
    ),
    (
        "perspectives/Sales View.tmdl",
        "perspective 'Sales View'\n\n\tperspectiveTable Sales\n\n\t\tperspectiveMeasure 'Sales Amount'\n",
    ),
];
