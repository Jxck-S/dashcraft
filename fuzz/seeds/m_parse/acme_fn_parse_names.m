(raw as text) as text =>
let
    Trimmed = Text.Trim(raw),
    Inner = if Text.Start(Trimmed, 1) = "[" then Text.Range(Trimmed, 1, Text.Length(Trimmed) - 2) else Trimmed,
    Names = List.Transform(Text.Split(Inner, ","), each Text.Trim(Text.Trim(_), {"'", """"})),
    NonEmpty = List.Select(Names, each _ <> ""),
    Result = if List.IsEmpty(NonEmpty) then "" else Text.Combine(NonEmpty, "; ")
in
    Result
