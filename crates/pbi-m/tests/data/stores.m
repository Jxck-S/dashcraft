let
    Source = AzureStorage.Blobs(StorageContainerUrl),
    #"Source CSV" = Csv.Document(Source{[Name = "stores/stores.csv"]}[Content], [Delimiter = ",", Columns = 9, Encoding = 65001, QuoteStyle = QuoteStyle.Csv]),
    #"Promoted Headers" = Table.PromoteHeaders(#"Source CSV", [PromoteAllScalars = true]),
    #"Cleaned Text" = Table.TransformColumns(#"Promoted Headers", {{"store_id", fnCleanKey, type text}, {"region", Text.Trim, type text}, {"manager_email", each Text.Lower(Text.Trim(_)), type text}}),
    #"Removed Duplicates" = Table.Distinct(#"Cleaned Text"),
    #"Blanks To Null" = Table.ReplaceValue(#"Removed Duplicates", "", null, Replacer.ReplaceValue, {"city", "state", "square_feet"}),
    #"Changed Type" = Table.TransformColumnTypes(#"Blanks To Null", {{"open_date", type date}, {"square_feet", Int64.Type}}),
    #"Added Manager" = Table.AddColumn(#"Changed Type", "store_manager", each Text.Proper(Text.Replace(Text.BeforeDelimiter([manager_email], "@"), ".", " ")), type text),
    #"Added Region Code" = Table.AddColumn(#"Added Manager", "region_code", each Text.Upper(Text.Start([region], 3)), type text),
    #"Renamed Columns" = Table.RenameColumns(#"Added Region Code", {{"store_id", "Store ID"}, {"store_name", "Store Name"}, {"region", "Region"}, {"channel", "Channel"}, {"city", "City"}, {"state", "State"}, {"manager_email", "Manager Email"}, {"open_date", "Open Date"}, {"square_feet", "Square Feet"}, {"store_manager", "Store Manager"}, {"region_code", "Region Code"}})
in
    #"Renamed Columns"
