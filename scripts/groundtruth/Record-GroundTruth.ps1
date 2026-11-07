<#
.SYNOPSIS
Records Power BI Desktop's results for the ground-truth corpora (ADR-0011).

.DESCRIPTION
For each corpus (reference/groundtruth/corpus/*.json) this script finds the running Desktop
model that has the corpus's tables, optionally refreshes it, runs every case through ADOMD.NET
and writes reference/groundtruth/recordings/<suite>.json. Windows only, PowerShell 7; open the
fixture in Desktop first (reference/groundtruth/README.md).

Cases marked "volatile" record their columns and row count only, never their values (they
may contain the clock, random numbers or the signed-in Windows user); so does every case of a
corpus with "recordValues": false (dax-arity, which only needs to know whether Desktop accepted
the query).

A corpus with "language": "m" holds M expressions instead of DAX queries. They are evaluated by
Desktop's mashup engine against the standard library alone (no model query is in scope) in a
temporary hidden table of any open model (or the one on -Port), which is deleted afterwards; the
recording keeps each case's value as typed JSON ("value") or its error ("error").

Corpora with "language": "tmdl" are skipped: Record-Tmdl.ps1 records them with Microsoft's TOM
library instead of Desktop.

.EXAMPLE
pwsh scripts/groundtruth/Record-GroundTruth.ps1 -Refresh
pwsh scripts/groundtruth/Record-GroundTruth.ps1 -Corpus reference/groundtruth/corpus/dax-functions.json
#>
[CmdletBinding()]
param(
    [string[]]$Corpus,
    [int]$Port,
    [switch]$Refresh,
    [string]$OutDir
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'GroundTruth.psm1') -Force

$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
if (-not $Corpus) { $Corpus = Get-ChildItem (Join-Path $root 'reference\groundtruth\corpus') -Filter *.json | ForEach-Object FullName }
# `pwsh -File` passes a comma list as one string.
$Corpus = @($Corpus | ForEach-Object { $_ -split ',' } | Where-Object { $_ } | ForEach-Object { (Resolve-Path $_).Path })
if (-not $OutDir) { $OutDir = Join-Path $root 'reference\groundtruth\recordings' }
New-Item -ItemType Directory -Force $OutDir | Out-Null

$desktopVersion = Get-GtDesktopVersion
$refreshed = @{}

foreach ($file in $Corpus) {
    $suite = Get-Content -Raw $file | ConvertFrom-Json -NoEnumerate
    if ($suite.PSObject.Properties['language'] -and $suite.language -eq 'tmdl') { continue } # Record-Tmdl.ps1
    $isM = [bool]($suite.PSObject.Properties['language'] -and $suite.language -eq 'm')
    $instancePort = if ($Port) { $Port }
    elseif ($isM) {
        $any = Get-GtInstances | Select-Object -First 1
        if (-not $any) { throw 'No running Power BI Desktop model; open any file in Desktop first.' }
        $any.Port
    } else { (Find-GtInstance -RequiredTables $suite.requiresTables).Port }
    if ($Refresh -and -not $isM -and -not $refreshed.ContainsKey($instancePort)) {
        Write-Host "refreshing the model on port $instancePort"
        Invoke-GtRefresh -Port $instancePort
        $refreshed[$instancePort] = $true
    }

    $probe = Open-GtConnection -Port $instancePort
    try {
        $engine = [string]$probe.ServerVersion
        $culture = (Invoke-GtQuery -Connection $probe -Query 'EVALUATE ROW("c", USERCULTURE())').Rows[0] | ConvertFrom-Json
    } finally { $probe.Close() }
    if ($culture -ne 'en-US') { throw "Desktop's culture is $culture; record with en-US (reference/groundtruth/README.md)." }
    $mResults = @{}
    if ($isM) {
        $mCulture = (Invoke-GtMCases -Port $instancePort -Cases @(@{ id = 'culture'; query = 'Culture.Current' }))['culture'].Result
        if ($mCulture -ne '"en-US"') { throw "Desktop's M culture is $mCulture; record with en-US (reference/groundtruth/README.md)." }
        $batch = 200
        for ($i = 0; $i -lt $suite.cases.Count; $i += $batch) {
            $part = @($suite.cases)[$i..([Math]::Min($i + $batch, $suite.cases.Count) - 1)]
            $r = Invoke-GtMCases -Port $instancePort -Cases $part
            foreach ($k in $r.Keys) { $mResults[$k] = $r[$k] }
        }
    }

    $connections = @{}
    $out = [System.Text.StringBuilder]::new()
    [void]$out.Append("{`n")
    [void]$out.Append('  "schemaVersion": 1,' + "`n")
    [void]$out.Append('  "suite": ' + (ConvertTo-GtJsonString $suite.suite) + ",`n")
    [void]$out.Append('  "desktop": { "version": ' + (ConvertTo-GtJsonString $desktopVersion) + ', "engine": ' + (ConvertTo-GtJsonString $engine) + ', "culture": "en-US" },' + "`n")
    [void]$out.Append('  "recordedOn": "' + (Get-Date -Format 'yyyy-MM-dd') + '",' + "`n")
    [void]$out.Append('  "cases": [')
    $first = $true
    $errors = 0
    try {
        foreach ($case in $suite.cases) {
            $volatile = [bool]$case.PSObject.Properties['volatile'] -or
                ($suite.PSObject.Properties['recordValues'] -and -not $suite.recordValues)
            if ($isM) {
                $m = $mResults[[string]$case.id]
                if (-not $m) { throw "no M result for case $($case.id)" }
                [void]$out.Append($(if ($first) { "`n" } else { ",`n" }))
                $first = $false
                [void]$out.Append('    {' + "`n")
                [void]$out.Append('      "id": ' + (ConvertTo-GtJsonString $case.id) + ",`n")
                [void]$out.Append('      "query": ' + (ConvertTo-GtJsonString $case.query) + ",`n")
                if (-not $m.Ok) {
                    $errors++
                    [void]$out.Append('      "error": ' + (ConvertTo-GtJsonString $m.Result) + "`n")
                } elseif ($volatile) {
                    [void]$out.Append('      "ok": true' + "`n")
                } else {
                    [void]$out.Append('      "value": ' + (Format-GtMValue $m.Result) + "`n")
                }
                [void]$out.Append('    }')
                continue
            }
            $roles = @()
            if ($case.PSObject.Properties['desktopRoles']) { $roles = @($case.desktopRoles) }
            elseif ($case.PSObject.Properties['viewAs']) { $roles = @($case.viewAs.roles) }
            $key = $roles -join '|'
            if (-not $connections.ContainsKey($key)) {
                $connections[$key] = if ($roles) { Open-GtConnection -Port $instancePort -Roles $roles } else { Open-GtConnection -Port $instancePort }
            }
            $r = Invoke-GtQuery -Connection $connections[$key] -Query $case.query

            [void]$out.Append($(if ($first) { "`n" } else { ",`n" }))
            $first = $false
            [void]$out.Append('    {' + "`n")
            [void]$out.Append('      "id": ' + (ConvertTo-GtJsonString $case.id) + ",`n")
            [void]$out.Append('      "query": ' + (ConvertTo-GtJsonString $case.query) + ",`n")
            if ($r.ContainsKey('Error')) {
                $errors++
                # The message is kept for people; the comparison only checks that both sides fail.
                [void]$out.Append('      "error": ' + (ConvertTo-GtJsonString $r.Error) + "`n")
            } else {
                $cols = ($r.Columns | ForEach-Object { ConvertTo-GtJsonString $_ }) -join ', '
                [void]$out.Append('      "columns": [' + $cols + '],' + "`n")
                if ($volatile) {
                    [void]$out.Append('      "rowCount": ' + $r.Rows.Count + "`n")
                } elseif ($r.Rows.Count -eq 0) {
                    [void]$out.Append('      "rows": []' + "`n")
                } else {
                    [void]$out.Append('      "rows": [' + "`n        " + ($r.Rows -join ",`n        ") + "`n      ]`n")
                }
            }
            [void]$out.Append('    }')
        }
    } finally {
        foreach ($c in $connections.Values) { $c.Close() }
    }
    [void]$out.Append("`n  ]`n}`n")
    $path = Join-Path $OutDir "$($suite.suite).json"
    [IO.File]::WriteAllText($path, $out.ToString(), [Text.UTF8Encoding]::new($false))
    Write-Host ("{0}: {1} cases ({2} errors) -> {3}" -f $suite.suite, $suite.cases.Count, $errors, $path)
}
