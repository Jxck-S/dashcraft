 let
    Source = Orders,
    #"Changed Type" = Table.TransformColumnTypes(Source, {{"a", Int64.Type}}),
    Added = Table.AddColumn(#"Changed Type", "b", each Text.Upper([a]), 