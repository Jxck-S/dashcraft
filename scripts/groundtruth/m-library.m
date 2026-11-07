// The m-library ground-truth case (scripts/groundtruth/gen_corpus.py): every name of the standard
// library that Power BI Desktop's mashup engine has (evaluated with the library as #shared): its
// kind, and for functions the required parameter count, parameter names and types and the return
// type; for types their primitive kind; for number, text and logical constants their value.
// Names, kinds, counts and values only: no documentation text is recorded.

let
    Prims = {
        {"logical", type logical}, {"number", type number}, {"time", type time}, {"date", type date},
        {"datetime", type datetime}, {"datetimezone", type datetimezone}, {"duration", type duration},
        {"text", type text}, {"binary", type binary}, {"type", type type}, {"list", type list},
        {"record", type record}, {"table", type table}, {"function", type function}, {"action", type action},
        {"anynonnull", type anynonnull}
    },
    Same = (a as type, b as type) as logical => Type.Is(a, b) and Type.Is(b, a),
    // The primitive kind of a type (structured record, table, list and function types give
    // their kind), with "nullable " when it admits null.
    TypeName = (t as type) as text =>
        if Same(t, type any) then "any"
        else if Same(t, type null) then "null"
        else if Same(t, type none) then "none"
        else
            let
                n = Type.NonNullable(t),
                hit = List.First(List.Select(Prims, each Type.Is(n, _{1})), null),
                base = if hit = null then "unknown" else hit{0}
            in
                (if Type.IsNullable(t) then "nullable " else "") & base,
    Kind = (v as any) as text =>
        if v = null then "null"
        else if v is function then "function"
        else if v is type then "type"
        else if v is logical then "logical"
        else if v is number then "number"
        else if v is text then "text"
        else if v is date then "date"
        else if v is datetimezone then "datetimezone"
        else if v is datetime then "datetime"
        else if v is time then "time"
        else if v is duration then "duration"
        else if v is binary then "binary"
        else if v is list then "list"
        else if v is record then "record"
        else if v is table then "table"
        else "other",
    MachineDependent = {"Culture.Current", "TimeZone.Current"},
    Describe = (n as text) as record =>
        let
            r = try Record.Field(#shared, n),
            v = r[Value],
            k = if r[HasError] then "error" else Kind(v)
        in
            if k = "function" then
                let
                    t = Value.Type(v),
                    ps = Type.FunctionParameters(t)
                in
                    [
                        name = n,
                        kind = k,
                        required = Type.FunctionRequiredParameters(t),
                        params = List.Transform(Record.FieldNames(ps), (p) => {p, TypeName(Record.Field(ps, p))}),
                        returns = TypeName(Type.FunctionReturn(t))
                    ]
            else if k = "type" then [name = n, kind = k, type = TypeName(v)]
            else if List.Contains({"number", "text", "logical"}, k) and not List.Contains(MachineDependent, n) then
                [name = n, kind = k, value = v]
            else [name = n, kind = k]
in
    List.Transform(List.Sort(Record.FieldNames(#shared), Comparer.Ordinal), Describe)
