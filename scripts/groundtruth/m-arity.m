// The m-arity ground-truth case (scripts/groundtruth/gen_corpus.py): for every function of Power BI
// Desktop's M standard library (evaluated with the library as #shared), the error Desktop raises
// when the function is called with one argument fewer than it requires (when it requires any) and
// with one more than it accepts. Every argument is null: the count is checked before the function
// runs. Names and error messages only: no documentation text is recorded.

let
    Probe = (f as function, n as number) as text =>
        let
            r = try Function.Invoke(f, List.Repeat({null}, n))
        in
            if r[HasError] then r[Error][Reason] & ": " & r[Error][Message] else "ok",
    Describe = (n as text) as list =>
        let
            f = Record.Field(#shared, n),
            t = Value.Type(f),
            required = Type.FunctionRequiredParameters(t),
            accepted = Record.FieldCount(Type.FunctionParameters(t))
        in
            {n, if required = 0 then null else Probe(f, required - 1), Probe(f, accepted + 1)},
    Functions = List.Select(
        List.Sort(Record.FieldNames(#shared), Comparer.Ordinal),
        (n) => (try Record.Field(#shared, n) is function otherwise false)
    )
in
    List.Transform(Functions, Describe)
