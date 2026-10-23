// Section document in the style of a Power Query SDK connector (.pq).
[Version = "1.0.0", Tags = {"a", 1, true, null, [Nested = 0x10]}]
section HelloWorld;

[DataSource.Kind = "HelloWorld", Publish = "HelloWorld.Publish"]
shared HelloWorld.Contents = (optional message as text) =>
    let
        _message = if (message <> null) then message else "(no message)",
        a = "Hello from HelloWorld: " & _message
    in
        a;

HelloWorld = [
    Authentication = [Anonymous = []],
    Label = Extension.LoadString("DataSourceLabel")
];

HelloWorld.Publish = [
    Beta = true,
    ButtonText = {Extension.LoadString("FormulaTitle"), Extension.LoadString("FormulaHelp")},
    SourceImage = HelloWorld.Icons
];

HelloWorldType = type function (
    message as (type text meta [
        Documentation.FieldCaption = "Message",
        Documentation.SampleValues = {"Hello"}
    ]))
    as table meta [Documentation.Name = "Hello"];

shared #"Quoted Member" = HelloWorld!HelloWorldType;
