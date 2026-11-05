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

Export-ModuleMember -Function Initialize-GtTools, Get-GtDesktopVersion, Get-GtInstances, Open-GtConnection,
Get-GtModelTables, Find-GtInstance, Invoke-GtRefresh, ConvertTo-GtJsonString, ConvertTo-GtJsonValue, Invoke-GtQuery
