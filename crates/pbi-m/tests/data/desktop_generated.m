// Hand-written in the style of Power BI Desktop generated queries (not from a fixture).
let
    Source = Excel.Workbook(File.Contents("C:\Users\me\Book1.xlsx"), null, true),
    Sheet1_Sheet = Source{[Item="Sheet1",Kind="Sheet"]}[Data],
    #"Promoted Headers" = Table.PromoteHeaders(Sheet1_Sheet, [PromoteAllScalars=true]),
    #"Changed Type" = Table.TransformColumnTypes(#"Promoted Headers",{{"Column1", type text}, {"Date", type date}, {"Amount", Currency.Type}, {"Count", Int64.Type}, {"Pct", Percentage.Type}}),
    #"Added Custom" = Table.AddColumn(#"Changed Type", "Custom", each if [Amount] = null then 0 else [Amount] * 1.1),
    #"Filtered Rows" = Table.SelectRows(#"Added Custom", each ([Column1] <> null and [Column1] <> "") and not Text.StartsWith([Column1], "#")),
    #"Split Column by Delimiter" = Table.SplitColumn(#"Filtered Rows", "Column1", Splitter.SplitTextByDelimiter("-", QuoteStyle.Csv), {"Column1.1", "Column1.2"}),
    #"Renamed Columns" = Table.RenameColumns(#"Split Column by Delimiter",{{"Column1.1", "Key"}}),
    Generated = List.Generate(() => 0, each _ < 10, each _ + 1),
    Api = Json.Document(Web.Contents("https://example.invalid/api", [Headers=[#"Content-Type"="application/json", Accept="*/*"], Content=Text.ToBinary("{}")])),
    Typed = Value.ReplaceType((x as number, optional y as nullable number) as number => x + (y ?? 0), type function (x as number, optional y as nullable number) as number meta [Documentation.Name = "Add"]),
    Recursive = let l = {0, @l} in l,
    Cyclic = [A = {B}, B = {A}],
    Grouped = Table.Group(#"Renamed Columns", {"Key"}, {{"Rows", each Table.RowCount(_), Int64.Type}, {"All", each _, type table [Key = nullable text, Amount = nullable number]}}),
    Final = try Grouped otherwise #table(type table [Key = text, Rows = Int64.Type], {})
in
    Final
