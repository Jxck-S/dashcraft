let
    Source = AzureStorage.Blobs(StorageContainerUrl),
    #"Source CSV" = Csv.Document(Source{[Name = "sales/orders.csv"]}[Content], [Delimiter = ",", Columns = 10, Encoding = 65001, QuoteStyle = QuoteStyle.Csv]),
    #"Promoted Headers" = Table.PromoteHeaders(#"Source CSV", [PromoteAllScalars = true]),
    #"Removed Rows Without Id" = Table.SelectRows(#"Promoted Headers", each Text.Trim([order_id] ?? "") <> ""),
    #"Cleaned Text" = Table.TransformColumns(#"Removed Rows Without Id", {{"order_id", Text.Trim, type text}, {"customer_id", fnCleanKey, type text}, {"store_id", fnCleanKey, type text}, {"product_id", fnCleanKey, type text}, {"status", each Text.Proper(Text.Trim(_)), type text}}),
    #"Removed Duplicates" = Table.Distinct(#"Cleaned Text"),
    #"Changed Type" = Table.TransformColumnTypes(#"Removed Duplicates", {{"order_date", type date}, {"ship_date", type date}, {"quantity", Int64.Type}, {"unit_price", type number}, {"discount", type number}}),
    #"Default Discount" = Table.TransformColumns(#"Changed Type", {{"discount", each _ ?? 0, type number}}),
    #"Added Line Amount" = Table.AddColumn(#"Default Discount", "line_amount", each [quantity] * [unit_price] * (1 - [discount]), type number),
    #"Renamed Columns" = Table.RenameColumns(#"Added Line Amount", {{"order_id", "Order ID"}, {"order_date", "Order Date"}, {"ship_date", "Ship Date"}, {"customer_id", "Customer ID"}, {"store_id", "Store ID"}, {"product_id", "Product ID"}, {"quantity", "Quantity"}, {"unit_price", "Unit Price"}, {"discount", "Discount"}, {"status", "Status"}, {"line_amount", "Line Amount"}})
in
    #"Renamed Columns"
