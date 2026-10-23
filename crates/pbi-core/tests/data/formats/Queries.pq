// Synthetic Power Query section document for pbi-core tests.
section Section1;

shared BaseUrl = "https://example.test/api" meta [IsParameterQuery = true, Type = "Text", IsParameterQueryRequired = true];

shared Sales = let
    Source = Csv.Document(Web.Contents(BaseUrl & "/sales.csv"), [Delimiter = ","]),
    Promoted = Table.PromoteHeaders(Source, [PromoteAllScalars = true])
in
    Promoted;

shared #"Sales by Region" = let
    Grouped = Table.Group(Sales, {"Region"}, {{"Amount", each List.Sum([Amount]), type number}})
in
    Grouped;

Helper = 1;
