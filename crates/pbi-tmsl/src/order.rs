//! Canonical TOM serializer key order per object type ([`KeyOrder::Tom`]).
//!
//! Derived from Analysis Services / Tabular Editor / PBIT output. `*` marks
//! where unknown keys without an anchor go (see [`crate::obj::ObjWriter`]).
//! Every key the writer emits from a typed field must be listed.
//!
//! [`KeyOrder::Tom`]: crate::KeyOrder::Tom

pub(crate) const ROOT: &[&str] = &[
    "name",
    "description",
    "compatibilityLevel",
    "compatibilityMode",
    "readWriteMode",
    "model",
    "*",
    "annotations",
    "extendedProperties",
];

pub(crate) const MODEL: &[&str] = &[
    "name",
    "description",
    "defaultMode",
    "culture",
    "collation",
    "discourageImplicitMeasures",
    "dataAccessOptions",
    "defaultDataView",
    "defaultPowerBIDataSourceVersion",
    "forceUniqueNames",
    "sourceQueryCulture",
    "disableAutoExists",
    "maxParallelismPerRefresh",
    "maxParallelismPerQuery",
    "directLakeBehavior",
    "valueFilterBehavior",
    "mAttributes",
    "dataSourceVariablesOverrideBehavior",
    "dataSourceDefaultMaxConnections",
    "automaticAggregationOptions",
    "dataSources",
    "tables",
    "relationships",
    "perspectives",
    "cultures",
    "roles",
    "expressions",
    "queryGroups",
    "analyticsAIMetadata",
    "functions",
    "*",
    "annotations",
    "extendedProperties",
];

pub(crate) const TABLE: &[&str] = &[
    "name",
    "description",
    "isHidden",
    "isPrivate",
    "showAsVariationsOnly",
    "dataCategory",
    "lineageTag",
    "sourceLineageTag",
    "systemManaged",
    "excludeFromModelRefresh",
    "alternateSourcePrecedence",
    "excludeFromAutomaticAggregations",
    "defaultDetailRowsDefinition",
    "calculationGroup",
    "columns",
    "partitions",
    "measures",
    "hierarchies",
    "refreshPolicy",
    "*",
    "changedProperties",
    "annotations",
    "extendedProperties",
];

pub(crate) const COLUMN: &[&str] = &[
    "type",
    "name",
    "description",
    "dataType",
    "isNameInferred",
    "isDataTypeInferred",
    "isHidden",
    "isUnique",
    "isKey",
    "isNullable",
    "alignment",
    "tableDetailPosition",
    "isDefaultLabel",
    "isDefaultImage",
    "expression",
    "sourceColumn",
    "formatString",
    "isAvailableInMdx",
    "keepUniqueRows",
    "displayOrdinal",
    "sourceProviderType",
    "displayFolder",
    "sortByColumn",
    "columnOriginTable",
    "columnOriginColumn",
    "lineageTag",
    "sourceLineageTag",
    "dataCategory",
    "summarizeBy",
    "encodingHint",
    "alternateOf",
    "attributeHierarchy",
    "variations",
    "relatedColumnDetails",
    "*",
    "changedProperties",
    "annotations",
    "extendedProperties",
];

pub(crate) const MEASURE: &[&str] = &[
    "name",
    "description",
    "expression",
    "formatString",
    "isHidden",
    "isSimpleMeasure",
    "displayFolder",
    "lineageTag",
    "sourceLineageTag",
    "dataCategory",
    "detailRowsDefinition",
    "formatStringDefinition",
    "kpi",
    "*",
    "changedProperties",
    "annotations",
    "extendedProperties",
];

pub(crate) const PARTITION: &[&str] = &[
    "name",
    "description",
    "mode",
    "dataView",
    "queryGroup",
    "source",
    "*",
    "changedProperties",
    "annotations",
    "extendedProperties",
];

pub(crate) const SOURCE: &[&str] = &[
    "type",
    "expression",
    "query",
    "dataSource",
    "entityName",
    "expressionSource",
    "schemaName",
    "*",
];

pub(crate) const HIERARCHY: &[&str] = &[
    "name",
    "description",
    "isHidden",
    "displayFolder",
    "hideMembers",
    "lineageTag",
    "sourceLineageTag",
    "levels",
    "*",
    "changedProperties",
    "annotations",
    "extendedProperties",
];

pub(crate) const LEVEL: &[&str] = &[
    "name",
    "description",
    "ordinal",
    "column",
    "lineageTag",
    "sourceLineageTag",
    "*",
    "changedProperties",
    "annotations",
    "extendedProperties",
];

pub(crate) const RELATIONSHIP: &[&str] = &[
    "name",
    "type",
    "fromCardinality",
    "fromTable",
    "fromColumn",
    "toTable",
    "toColumn",
    "isActive",
    "crossFilteringBehavior",
    "toCardinality",
    "joinOnDateBehavior",
    "relyOnReferentialIntegrity",
    "securityFilteringBehavior",
    "state",
    "*",
    "changedProperties",
    "annotations",
    "extendedProperties",
];

pub(crate) const EXPRESSION: &[&str] = &[
    "name",
    "description",
    "kind",
    "expression",
    "lineageTag",
    "sourceLineageTag",
    "queryGroup",
    "expressionSource",
    "mAttributes",
    "*",
    "annotations",
    "extendedProperties",
];

pub(crate) const ROLE: &[&str] = &[
    "name",
    "description",
    "modelPermission",
    "members",
    "tablePermissions",
    "*",
    "annotations",
    "extendedProperties",
];

pub(crate) const TABLE_PERMISSION: &[&str] = &[
    "name",
    "filterExpression",
    "metadataPermission",
    "columnPermissions",
    "*",
    "annotations",
    "extendedProperties",
];

pub(crate) const CULTURE: &[&str] = &[
    "name",
    "linguisticMetadata",
    "translations",
    "*",
    "annotations",
    "extendedProperties",
];

pub(crate) const PERSPECTIVE: &[&str] = &[
    "name",
    "description",
    "tables",
    "*",
    "annotations",
    "extendedProperties",
];

pub(crate) const QUERY_GROUP: &[&str] = &["folder", "description", "*", "annotations"];

pub(crate) const CALCULATION_GROUP: &[&str] = &[
    "description",
    "precedence",
    "calculationItems",
    "multipleOrEmptySelectionExpression",
    "noSelectionExpression",
    "*",
    "annotations",
    "extendedProperties",
];

pub(crate) const CALCULATION_ITEM: &[&str] = &[
    "name",
    "description",
    "expression",
    "ordinal",
    "state",
    "errorMessage",
    "formatStringDefinition",
    "*",
    "annotations",
    "extendedProperties",
];

pub(crate) const SELECTION_EXPRESSION: &[&str] = &[
    "description",
    "expression",
    "formatStringDefinition",
    "*",
    "annotations",
    "extendedProperties",
];
