let
    Source = AzureStorage.Blobs(StorageContainerUrl),
    #"Source CSV" = Csv.Document(Source{[Name = "sales/customers.csv"]}[Content], [Delimiter = ",", Columns = 10, Encoding = 65001, QuoteStyle = QuoteStyle.Csv]),
    #"Promoted Headers" = Table.PromoteHeaders(#"Source CSV", [PromoteAllScalars = true]),
    #"Cleaned Text" = Table.TransformColumns(#"Promoted Headers", {{"customer_id", fnCleanKey, type text}, {"email", each Text.Lower(Text.Trim(_)), type text}, {"segment", Text.Trim, type text}, {"loyalty_tier", each let t = Text.Trim(_ ?? "") in if t = "" then "Unassigned" else t, type text}, {"account_managers", each let names = fnParseNames(_ ?? "") in if names = "" then null else names, type nullable text}}),
    #"Removed Duplicates" = Table.Distinct(#"Cleaned Text"),
    #"Added Name" = Table.AddColumn(#"Removed Duplicates", "customer_name", each Text.Combine({Text.Trim([first_name]), Text.Trim([last_name])}, " "), type text),
    #"Added Domain" = Table.AddColumn(#"Added Name", "email_domain", each Text.AfterDelimiter([email], "@"), type text),
    #"Added Manager Count" = Table.AddColumn(#"Added Domain", "manager_count", each List.Count(List.RemoveNulls(List.Transform(Text.Split([account_managers] ?? "", ";"), each if Text.Trim(_) = "" then null else _))), Int64.Type),
    #"Changed Type" = Table.TransformColumnTypes(#"Added Manager Count", {{"join_date", type date}}),
    #"Selected Columns" = Table.SelectColumns(#"Changed Type", {"customer_id", "customer_name", "email", "email_domain", "segment", "city", "state", "join_date", "loyalty_tier", "account_managers", "manager_count"}),
    #"Renamed Columns" = Table.RenameColumns(#"Selected Columns", {{"customer_id", "Customer ID"}, {"customer_name", "Customer Name"}, {"email", "Email"}, {"email_domain", "Email Domain"}, {"segment", "Segment"}, {"city", "City"}, {"state", "State"}, {"join_date", "Join Date"}, {"loyalty_tier", "Loyalty Tier"}, {"account_managers", "Account Managers"}, {"manager_count", "Account Manager Count"}})
in
    #"Renamed Columns"
