# Ground-truth recorder helpers: black-box queries against the local Analysis Services engine
# that Power BI Desktop runs for an open file (ADR-0011). Windows only; PowerShell 7.
#
# The ADOMD.NET client library (NuGet Microsoft.AnalysisServices.AdomdClient) is downloaded
# into target/groundtruth/tools by Initialize-GtTools; it is never committed or shipped.

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$script:RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$script:ToolsDir = Join-Path $script:RepoRoot 'target\groundtruth\tools'
$script:AdomdVersion = '19.118.3'
$script:Invariant = [System.Globalization.CultureInfo]::InvariantCulture

function Initialize-GtTools {
    <# Downloads ADOMD.NET into target/groundtruth/tools (once) and loads it. #>
    $dll = Join-Path $script:ToolsDir 'adomd\lib\net8.0\Microsoft.AnalysisServices.AdomdClient.dll'
    if (-not (Test-Path $dll)) {
        New-Item -ItemType Directory -Force $script:ToolsDir | Out-Null
        $v = $script:AdomdVersion
        $zip = Join-Path $script:ToolsDir 'adomd.zip'
        Invoke-WebRequest "https://api.nuget.org/v3-flatcontainer/microsoft.analysisservices.adomdclient/$v/microsoft.analysisservices.adomdclient.$v.nupkg" -OutFile $zip
        Expand-Archive -Force $zip (Join-Path $script:ToolsDir 'adomd')
    }
    if (-not ('Microsoft.AnalysisServices.AdomdClient.AdomdConnection' -as [type])) {
        Add-Type -Path $dll
    }
}

function Get-GtDesktopVersion {
    $pkg = Get-AppxPackage -Name Microsoft.MicrosoftPowerBIDesktop -ErrorAction SilentlyContinue
    if ($pkg) { return [string]$pkg.Version }
    $exe = Get-Process PBIDesktop -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($exe) { return [string]$exe.MainModule.FileVersionInfo.ProductVersion }
    return 'unknown'
}

function Get-GtInstances {
    <# Ports of the running Desktop engines (msmdsrv.port.txt of live workspaces). #>
    $roots = @(
        (Join-Path $env:USERPROFILE 'Microsoft\Power BI Desktop Store App\AnalysisServicesWorkspaces'),
        (Join-Path $env:LOCALAPPDATA 'Microsoft\Power BI Desktop\AnalysisServicesWorkspaces')
    ) | Where-Object { Test-Path $_ }
    $listening = @{}
    foreach ($p in Get-Process msmdsrv -ErrorAction SilentlyContinue) {
        foreach ($c in Get-NetTCPConnection -OwningProcess $p.Id -State Listen -ErrorAction SilentlyContinue) {
            $listening[[int]$c.LocalPort] = $p.Id
        }
    }
    foreach ($root in $roots) {
        foreach ($f in Get-ChildItem $root -Recurse -Filter msmdsrv.port.txt -ErrorAction SilentlyContinue) {
            $port = [int](([IO.File]::ReadAllText($f.FullName)) -replace '[^0-9]', '')
            if ($listening.ContainsKey($port)) {
                [pscustomobject]@{ Port = $port; Pid = $listening[$port]; Workspace = $f.Directory.Parent.Name }
            }
        }
    }
}

function Open-GtConnection {
    param([int]$Port, [string[]]$Roles, [string]$EffectiveUserName)
    Initialize-GtTools
    $cs = "Data Source=localhost:$Port"
    if ($Roles) { $cs += ";Roles=" + ($Roles -join ',') }
    if ($EffectiveUserName) { $cs += ";EffectiveUserName=$EffectiveUserName" }
    $c = [Microsoft.AnalysisServices.AdomdClient.AdomdConnection]::new($cs)
    $c.Open()
    return $c
}

function Get-GtModelTables {
    param([int]$Port)
    $c = Open-GtConnection -Port $Port
    try {
        $cmd = $c.CreateCommand()
        $cmd.CommandText = 'SELECT [Name] FROM $SYSTEM.TMSCHEMA_TABLES'
        $r = $cmd.ExecuteReader()
        $names = while ($r.Read()) { [string]$r.GetValue(0) }
        $r.Close()
        return @($names)
    } finally { $c.Close() }
}

function Find-GtInstance {
    <# The running engine whose model has every table in $RequiredTables. #>
    param([string[]]$RequiredTables)
    $found = @()
    foreach ($i in Get-GtInstances) {
        # An engine without a model (an "Untitled" or still-loading window) has no catalog.
        try { $tables = Get-GtModelTables -Port $i.Port } catch { continue }
        if (@($RequiredTables | Where-Object { $tables -notcontains $_ }).Count -eq 0) { $found += $i }
    }
    if ($found.Count -eq 0) { throw "No running Power BI Desktop model has the tables: $($RequiredTables -join ', '). Open the fixture in Desktop first." }
    if ($found.Count -gt 1) { throw "Several open Desktop models match ($($found.Port -join ', ')); close the others or pass -Port." }
    return $found[0]
}

function Invoke-GtRefresh {
    <# Full refresh of the open model through TMSL (no UI). #>
    param([int]$Port)
    $c = Open-GtConnection -Port $Port
    try {
        $cmd = $c.CreateCommand()
        $cmd.CommandText = '{"refresh":{"type":"full","objects":[{"database":"' + $c.Database + '"}]}}'
        $null = $cmd.ExecuteNonQuery()
    } finally { $c.Close() }
}

function ConvertTo-GtJsonString([string]$s) {
    return [System.Text.Json.JsonSerializer]::Serialize($s, [System.Text.Json.JsonSerializerOptions]@{
            Encoder = [System.Text.Encodings.Web.JavaScriptEncoder]::UnsafeRelaxedJsonEscaping })
}

function ConvertTo-GtJsonValue($v) {
    <# Typed value encoding of reference/groundtruth/README.md. #>
    if ($null -eq $v -or $v -is [System.DBNull]) { return 'null' }
    switch ($v.GetType().FullName) {
        'System.Boolean' { return $(if ($v) { 'true' } else { 'false' }) }
        { $_ -in 'System.Int64', 'System.Int32', 'System.Int16', 'System.Byte', 'System.UInt64', 'System.UInt32', 'System.UInt16', 'System.SByte' } {
            return ([long]$v).ToString($script:Invariant)
        }
        { $_ -in 'System.Double', 'System.Single' } {
            $d = [double]$v
            if ([double]::IsNaN($d)) { return '{"double":"NaN"}' }
            if ([double]::IsPositiveInfinity($d)) { return '{"double":"Infinity"}' }
            if ([double]::IsNegativeInfinity($d)) { return '{"double":"-Infinity"}' }
            $s = $d.ToString('R', $script:Invariant)
            if ($s -notmatch '[\.Ee]') { $s += '.0' }
            return $s
        }
        'System.Decimal' { return '{"decimal":"' + ([decimal]$v).ToString($script:Invariant) + '"}' }
        'System.DateTime' { return '{"dateTime":"' + ([datetime]$v).ToString('yyyy-MM-ddTHH:mm:ss.fff', $script:Invariant) + '"}' }
        'System.String' { return ConvertTo-GtJsonString $v }
        default { return ConvertTo-GtJsonString ([string]$v) }
    }
}

function Invoke-GtQuery {
    <# Runs one query; returns @{ Columns; Rows (JSON text per row) } or @{ Error }. #>
    param($Connection, [string]$Query, [int]$TimeoutSeconds = 300)
    $cmd = $Connection.CreateCommand()
    $cmd.CommandText = $Query
    $cmd.CommandTimeout = $TimeoutSeconds
    try {
        $r = $cmd.ExecuteReader()
    } catch {
        return @{ Error = ($_.Exception.InnerException ?? $_.Exception).Message }
    }
    try {
        $cols = for ($i = 0; $i -lt $r.FieldCount; $i++) { $r.GetName($i) }
        $rows = [System.Collections.Generic.List[string]]::new()
        while ($r.Read()) {
            $cells = for ($i = 0; $i -lt $r.FieldCount; $i++) { ConvertTo-GtJsonValue $r.GetValue($i) }
            $rows.Add('[' + ($cells -join ', ') + ']')
        }
        return @{ Columns = @($cols); Rows = $rows }
    } catch {
        return @{ Error = ($_.Exception.InnerException ?? $_.Exception).Message }
    } finally {
        $r.Close()
    }
}

# Evaluates each M case (the text in `c{1}`) against the standard library alone (the dotted names
# of #shared, without the model's own queries) and encodes its value as JSON text: null, logical,
# number and text as JSON; lists as arrays; records as objects; anything else as a one-key object
# tagged with `#`: {"#date": "2024-01-05"}, {"#datetime": ...}, {"#datetimezone": ...},
# {"#time": ...}, {"#duration": "1.02:03:04"}, {"#binary": base64}, {"#number": "NaN"},
# {"#table": {"columns": [...], "rows": [[...]]}}, {"#type": "nullable number"},
# {"#function": null}, and a record whose only field starts with `#` as {"#record": {...}}.
$script:MProbe = @'
let
    Library = Record.SelectFields(#shared, List.Select(Record.FieldNames(#shared), each Text.Contains(_, "."))),
    Json = (t as text) as text => Text.FromBinary(Json.FromValue(t), TextEncoding.Utf8),
    Tag = (k as text, body as text) as text => "{""#" & k & """:" & body & "}",
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
    Num = (n as number) as text =>
        if Number.IsNaN(n) then Tag("number", """NaN""")
        else if n = Number.PositiveInfinity then Tag("number", """Infinity""")
        else if n = Number.NegativeInfinity then Tag("number", """-Infinity""")
        else Number.ToText(n, "R", "en-US"),
    Encode = (v as any) as text =>
        if v = null then "null"
        else if v is logical then (if v then "true" else "false")
        else if v is number then Num(v)
        else if v is text then Json(v)
        else if v is date then Tag("date", Json(Date.ToText(v, [Format = "yyyy-MM-dd", Culture = "en-US"])))
        else if v is datetimezone then Tag("datetimezone", Json(DateTimeZone.ToText(v, [Format = "yyyy-MM-ddTHH:mm:ss.FFFFFFFzzz", Culture = "en-US"])))
        else if v is datetime then Tag("datetime", Json(DateTime.ToText(v, [Format = "yyyy-MM-ddTHH:mm:ss.FFFFFFF", Culture = "en-US"])))
        else if v is time then Tag("time", Json(Time.ToText(v, [Format = "HH:mm:ss.FFFFFFF", Culture = "en-US"])))
        else if v is duration then Tag("duration", Json(Duration.ToText(v)))
        else if v is binary then Tag("binary", Json(Binary.ToText(v, BinaryEncoding.Base64)))
        else if v is list then "[" & Text.Combine(List.Transform(v, @Encode), ",") & "]"
        else if v is table then Tag("table", "{""columns"":" & @Encode(Table.ColumnNames(v)) & ",""rows"":" & @Encode(Table.ToRows(v)) & "}")
        else if v is record then
            let
                names = Record.FieldNames(v),
                body = "{" & Text.Combine(List.Transform(names, (n) => Json(n) & ":" & @Encode(Record.Field(v, n))), ",") & "}"
            in
                if List.Count(names) = 1 and Text.StartsWith(names{0}, "#") then Tag("record", body) else body
        else if v is type then Tag("type", Json(TypeName(v)))
        else if v is function then Tag("function", "null")
        else Tag("unknown", "null"),
    // Desktop truncates text cells at 32,766 characters: long results are split into parts.
    Parts = (t as text) as list =>
        List.Transform({0 .. Number.RoundUp(Text.Length(t) / 16000) - 1}, each Text.Middle(t, _ * 16000, 16000)),
    Run = (c as list) as list =>
        let
            r = try Encode(Expression.Evaluate(c{1}, Library)),
            ok = not r[HasError],
            result = if ok then r[Value] else r[Error][Reason] & ": " & r[Error][Message],
            parts = if result = "" then {""} else Parts(result)
        in
            List.Transform({0 .. List.Count(parts) - 1}, (i) => [Id = c{0}, Ok = ok, Part = i, Result = parts{i}])
in
    Table.FromRecords(List.Combine(List.Transform(Cases, Run)), type table [Id = text, Ok = logical, Part = Int64.Type, Result = text])
'@

function ConvertTo-GtMString([string]$s) {
    return '"' + $s.Replace('"', '""') + '"'
}

function Invoke-GtMCases {
    <# Evaluates M expressions with Desktop's mashup engine: a temporary hidden import table whose
       M partition evaluates every case (TMSL create and refresh, read back with DAX, then
       deleted). Returns a hashtable id -> @{ Ok; Result } (Result: JSON text, or
       "Reason: message"). #>
    param([int]$Port, [object[]]$Cases)
    $table = 'DashCraft GT M Probe'
    $list = ($Cases | ForEach-Object { '{' + (ConvertTo-GtMString $_.id) + ', ' + (ConvertTo-GtMString $_.query) + '}' }) -join ",`n"
    $m = "let`nCases = {`n$list`n},`nProbe = $($script:MProbe)`nin`nProbe"
    $c = Open-GtConnection -Port $Port
    $run = {
        param([string]$Tmsl, [bool]$IgnoreErrors)
        $cmd = $c.CreateCommand()
        $cmd.CommandText = $Tmsl
        try { $null = $cmd.ExecuteNonQuery() } catch { if (-not $IgnoreErrors) { throw } }
    }
    try {
        $db = $c.Database
        $delete = @{ delete = @{ object = @{ database = $db; table = $table } } } | ConvertTo-Json -Depth 5 -Compress
        & $run $delete $true
        $columns = @(
            @{ name = 'Id'; dataType = 'string'; sourceColumn = 'Id' },
            @{ name = 'Ok'; dataType = 'boolean'; sourceColumn = 'Ok' },
            @{ name = 'Part'; dataType = 'int64'; sourceColumn = 'Part' },
            @{ name = 'Result'; dataType = 'string'; sourceColumn = 'Result' })
        $partition = @{ name = 'Probe'; mode = 'import'; source = @{ type = 'm'; expression = $m } }
        $create = @{ create = @{ parentObject = @{ database = $db }; table = @{ name = $table; isHidden = $true; columns = $columns; partitions = @($partition) } } }
        & $run ($create | ConvertTo-Json -Depth 10 -Compress) $false
        try {
            & $run ('{"refresh":{"type":"full","objects":[{"database":"' + $db + '","table":"' + $table + '"}]}}') $false
            $r = Invoke-GtQuery -Connection $c -Query "EVALUATE '$table' ORDER BY [Id], [Part]" -TimeoutSeconds 1800
            if ($r.ContainsKey('Error')) { throw "reading the M probe failed: $($r.Error)" }
            $parts = @{}
            foreach ($row in $r.Rows) {
                $cells = $row | ConvertFrom-Json -NoEnumerate
                $id = [string]$cells[0]
                if (-not $parts.ContainsKey($id)) { $parts[$id] = @{ Ok = [bool]$cells[1]; Text = [System.Text.StringBuilder]::new() } }
                [void]$parts[$id].Text.Append([string]$cells[3])
            }
            $out = @{}
            foreach ($id in $parts.Keys) { $out[$id] = @{ Ok = $parts[$id].Ok; Result = $parts[$id].Text.ToString() } }
            return $out
        } finally {
            & $run $delete $true
        }
    } finally { $c.Close() }
}

function Format-GtMValue([string]$Json) {
    <# A recorded M value: compact JSON without escaped non-ASCII text; a non-empty top-level
       array gets one element per line so the recordings diff well. #>
    $opts = [System.Text.Json.JsonSerializerOptions]@{ Encoder = [System.Text.Encodings.Web.JavaScriptEncoder]::UnsafeRelaxedJsonEscaping }
    $node = [System.Text.Json.Nodes.JsonNode]::Parse($Json)
    if ($null -eq $node) { return 'null' }
    if ($node -is [System.Text.Json.Nodes.JsonArray] -and $node.Count -gt 0) {
        $items = foreach ($item in $node) { if ($null -eq $item) { 'null' } else { $item.ToJsonString($opts) } }
        return "[`n        " + ($items -join ",`n        ") + "`n      ]"
    }
    return $node.ToJsonString($opts)
}

Export-ModuleMember -Function Initialize-GtTools, Get-GtDesktopVersion, Get-GtInstances, Open-GtConnection,
Get-GtModelTables, Find-GtInstance, Invoke-GtRefresh, ConvertTo-GtJsonString, ConvertTo-GtJsonValue, Invoke-GtQuery,
Invoke-GtMCases, Format-GtMValue
