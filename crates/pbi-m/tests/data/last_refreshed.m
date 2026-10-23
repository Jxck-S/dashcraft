let
    Source = #table(type table [#"Last Refreshed (UTC)" = datetime], {{DateTimeZone.RemoveZone(DateTimeZone.UtcNow())}})
in
    Source
