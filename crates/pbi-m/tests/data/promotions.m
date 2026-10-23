let
    Source = AzureStorage.Blobs(StorageContainerUrl),
    #"Source CSV" = Csv.Document(Source{[Name = "marketing/promotions.csv"]}[Content], [Delimiter = ",", Encoding = 65001, QuoteStyle = QuoteStyle.Csv]),
    #"Promoted Headers" = Table.PromoteHeaders(#"Source CSV", [PromoteAllScalars = true]),
    // The campaign export writes "\" into empty channel cells.
    #"Running Campaigns" = Table.SelectRows(#"Promoted Headers", each Text.Trim([Campaign Status] ?? "") = "Running" and Text.Trim([Promo Code] ?? "") <> ""),
    SalePrice = (price as number, percent as number) as number => price * (1 - percent / 100),
    CleanChannel = (raw as nullable text) as nullable text =>
        let
            t = Text.Trim(raw ?? "")
        in
            if t = "" or t = "\" then null else Text.Proper(t),
    #"Cleaned Channel" = Table.TransformColumns(#"Running Campaigns", {{"Sales Channel", CleanChannel, type nullable text}}),
    #"Added Sale Price" = Table.AddColumn(#"Cleaned Channel", "Sale Price", each SalePrice(Number.From([List Price]), Number.From([#"Discount (%)"])), type number),
    #"Selected Columns" = Table.SelectColumns(#"Added Sale Price", {"Promo Code", "Sales Channel", "List Price", "Discount (%)", "Sale Price"})
in
    #"Selected Columns"
