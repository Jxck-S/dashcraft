<#
.SYNOPSIS
Records Microsoft's TOM TMDL reader's verdicts for the TMDL ground-truth corpora (ADR-0029).

.DESCRIPTION
For each corpus with "language": "tmdl" (reference/groundtruth/corpus/tmdl-*.json) this script
copies the corpus's fixture folder (its "fixture" path) into target/groundtruth/tmdl-work, applies
each case to it and deserializes the folder with the Tabular Object Model's TMDL reader
(TmdlSerializer.DeserializeDatabaseFromFolder; NuGet Microsoft.AnalysisServices, downloaded once
into target/groundtruth/tools/tom, never committed). It writes
reference/groundtruth/recordings/<suite>.json. The reader is used as a black box: only its
documented API, its exceptions and their messages. Needs PowerShell 7 and .NET 8 (any OS that
runs them; Windows is what we record on). Power BI Desktop is not needed.

Each verdict is normalized to a short token: ok, unknown (neither a property nor an object),
unsupported (a known keyword that is not valid here), type:<T> (the value does not convert to
TOM type T), novalue, indent, nodefault (no `= value` form), notnamed, named (the form has or lacks
a name the object needs), unresolved (a reference to a missing object), compat (needs a higher
compatibility level), or <ParsingErrorType>:<detail> / <Exception>:<message> for anything else.

Case kinds (see scripts/groundtruth/gen_corpus.py):
  keywords: every keyword probed as `kw abc`, `kw`, `kw: abc`, `kw = 1`, `kw abc = 1`; keywords
    rejected in all five forms as unknown or unsupported are left out. For every accepted form the lowest
    compatibility level (of $Levels) that accepts it is recorded when it is above the lowest one,
    and so is the level the context itself needs (baselineLevel).
    When `kw: abc` fails to convert to a type, `kw: <sample>` of that type (true, 1, or 0 for an
    enum, which accepts any integer) is probed too and appended to the row.
  values: `keyword: value` for each candidate value.
  edit: one find/replace on one file (an empty find writes the file), then the whole folder.

.EXAMPLE
pwsh scripts/groundtruth/Record-Tmdl.ps1
pwsh scripts/groundtruth/Record-Tmdl.ps1 -Corpus reference/groundtruth/corpus/tmdl-validity.json
#>
[CmdletBinding()]
param(
    [string[]]$Corpus,
    [string]$OutDir
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$TomVersion = '19.118.3'
$Levels = 1550, 1600, 1601, 1603, 1604, 1605, 1606, 1700, 1701, 1702
$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$tools = Join-Path $root 'target\groundtruth\tools'
$lib = Join-Path $tools 'tom\lib\net8.0'
if (-not (Test-Path (Join-Path $lib 'Microsoft.AnalysisServices.Tabular.dll'))) {
    New-Item -ItemType Directory -Force $tools | Out-Null
    $zip = Join-Path $tools 'tom.zip'
    Invoke-WebRequest "https://api.nuget.org/v3-flatcontainer/microsoft.analysisservices/$TomVersion/microsoft.analysisservices.$TomVersion.nupkg" -OutFile $zip
    Expand-Archive -Force $zip (Join-Path $tools 'tom')
}
if (-not ('Microsoft.AnalysisServices.Tabular.TmdlSerializer' -as [type])) {
    foreach ($n in 'Runtime.Core', 'Runtime.Windows', 'Core', 'Tabular.Json', 'Tabular') {
        Add-Type -Path (Join-Path $lib "Microsoft.AnalysisServices.$n.dll") | Out-Null
    }
}

if (-not $Corpus) {
    $Corpus = Get-ChildItem (Join-Path $root 'reference\groundtruth\corpus') -Filter *.json | ForEach-Object FullName
}
$Corpus = @($Corpus | ForEach-Object { $_ -split ',' } | Where-Object { $_ } | ForEach-Object { (Resolve-Path $_).Path })
if (-not $OutDir) { $OutDir = Join-Path $root 'reference\groundtruth\recordings' }
New-Item -ItemType Directory -Force $OutDir | Out-Null
$work = Join-Path $root 'target\groundtruth\tmdl-work'

function Json([string]$s) { ConvertTo-Json -InputObject $s -Compress }

function Unescape([string]$s) {
    $sb = [Text.StringBuilder]::new()
    for ($i = 0; $i -lt $s.Length; $i++) {
        $c = $s[$i]
        if ($c -eq '\' -and $i + 1 -lt $s.Length) {
            $i++
            switch ($s[$i]) { 't' { [void]$sb.Append("`t") } 'n' { [void]$sb.Append("`n") } default { [void]$sb.Append($s[$i]) } }
        } else { [void]$sb.Append($c) }
    }
    $sb.ToString()
}

function Read-Spec([string]$query) {
    $spec = @{}
    foreach ($line in $query.Split("`n")) {
        $i = $line.IndexOf(': ')
        if ($i -gt 0) { $spec[$line.Substring(0, $i)] = Unescape $line.Substring($i + 2) }
        elseif ($line.EndsWith(':')) { $spec[$line.TrimEnd(':')] = '' }
    }
    $spec
}

function Invoke-Tom {
    try {
        $db = [Microsoft.AnalysisServices.Tabular.TmdlSerializer]::DeserializeDatabaseFromFolder($work)
        $errors = @($db.Model.Validate().Errors | ForEach-Object { [string]$_.Message })
        @{ Ok = $true; Validation = $errors }
    } catch {
        $e = $_.Exception
        while ($e -is [System.Management.Automation.MethodInvocationException] -and $e.InnerException) { $e = $e.InnerException }
        $r = @{ Ok = $false; Type = $e.GetType().Name; Message = [string]$e.Message }
        foreach ($p in 'Document', 'Line', 'LineText') {
            $prop = $e.GetType().GetProperty($p)
            if ($prop) { $r[$p] = [string]$prop.GetValue($e) }
        }
        $r
    }
}

function Get-Token($r) {
    if ($r.Ok) { if ($r.Validation.Count) { return 'invalid' } return 'ok' }
    $m = $r.Message
    if ($m -match 'neither a property nor an object') { return 'unknown' }
    if ($m -match 'Unsupported (property|object type) - ') { return 'unsupported' }
    if ($m -match 'expected type ([\w\.]+)!') { return "type:$($Matches[1])" }
    if ($m -match 'has no value specified') { return 'novalue' }
    if ($m -match 'Invalid indentation') { return 'indent' }
    if ($m -match 'does not support a default property') { return 'nodefault' }
    if ($m -match 'but \w+ is not a named object') { return 'notnamed' }
    if ($m -match 'but \w+ is a named object') { return 'named' }
    if ($m -match 'compatibility level of \d+ is below the minimal') { return 'compat' }
    if ($r.Type -eq 'TmdlDeserializationWithReferenceErrorsException') { return 'unresolved' }
    if ($m -match 'Parsing error type - (\w+)\r?\n\tDetailed error - (.*?)\r?\n') { return "$($Matches[1]):$($Matches[2])" }
    return "$($r.Type):$(($m -replace '\s+', ' ').Trim())"
}

function Reset-Work([string]$fixture) {
    if (Test-Path $work) { Remove-Item -Recurse -Force $work }
    Copy-Item -Recurse $fixture $work
}

# The level written into database.tmdl; Invoke-Probe keeps it when it probes that file.
$script:Level = 0
$LevelPattern = [regex]'compatibilityLevel: \d+'

function Set-Level([int]$level) {
    $script:Level = $level
    $p = Join-Path $work 'database.tmdl'
    [IO.File]::WriteAllText($p, $LevelPattern.Replace([IO.File]::ReadAllText($p), "compatibilityLevel: $level", 1))
}

# Inserts `lines` after the spec's anchor line, under its prefix lines; returns the verdict.
function Invoke-Probe($spec, [string]$fixture, [string[]]$lines) {
    $rel = $spec['file']
    $orig = [IO.File]::ReadAllText((Join-Path $fixture $rel)).Replace("`r`n", "`n")
    $anchor = $spec['after'] + "`n"
    $at = $orig.IndexOf($anchor)
    if ($at -lt 0 -or $orig.IndexOf($anchor, $at + 1) -ge 0) { throw "anchor not found once in ${rel}: $($spec['after'])" }
    $at += $anchor.Length
    $depth = [int]$spec['depth']
    $ins = [Text.StringBuilder]::new()
    if ($spec.ContainsKey('prefix')) {
        foreach ($p in $spec['prefix'].Split("`n")) { [void]$ins.Append(("`t" * $depth) + $p + "`n") }
        $depth++
    }
    foreach ($l in $lines) { [void]$ins.Append(("`t" * $depth) + $l + "`n") }
    $path = Join-Path $work $rel
    $cur = [IO.File]::ReadAllText($path)
    $text = $orig.Substring(0, $at) + $ins.ToString() + $orig.Substring($at)
    if ($rel -eq 'database.tmdl' -and $script:Level) { $text = $LevelPattern.Replace($text, "compatibilityLevel: $($script:Level)", 1) }
    [IO.File]::WriteAllText($path, $text)
    try { Invoke-Tom } finally { [IO.File]::WriteAllText($path, $cur) }
}

# The lowest level of $Levels that accepts the probe, or $null for the lowest one.
function Get-MinLevel($spec, [string]$fixture, [string[]]$lines, [int]$fixtureLevel) {
    try {
        foreach ($level in $Levels) {
            if ($level -ge $fixtureLevel) { return $fixtureLevel }
            Set-Level $level
            if ((Get-Token (Invoke-Probe $spec $fixture $lines)) -eq 'ok') {
                if ($level -eq $Levels[0]) { return $null }
                return $level
            }
        }
        $null
    } finally { Set-Level $fixtureLevel }
}

foreach ($file in $Corpus) {
    $suite = Get-Content -Raw $file | ConvertFrom-Json -NoEnumerate
    if (-not ($suite.PSObject.Properties['language'] -and $suite.language -eq 'tmdl')) { continue }
    $fixture = (Resolve-Path (Join-Path $root $suite.fixture)).Path
    Reset-Work $fixture
    $fixtureLevel = [int]([regex]::Match([IO.File]::ReadAllText((Join-Path $fixture 'database.tmdl')), 'compatibilityLevel: (\d+)').Groups[1].Value)
    $out = [Text.StringBuilder]::new()
    [void]$out.Append("{`n  `"schemaVersion`": 1,`n  `"suite`": $(Json $suite.suite),`n")
    [void]$out.Append("  `"oracle`": { `"library`": `"Microsoft.AnalysisServices (TOM)`", `"version`": $(Json $TomVersion), `"api`": `"TmdlSerializer.DeserializeDatabaseFromFolder`" },`n")
    [void]$out.Append("  `"recordedOn`": `"$(Get-Date -Format 'yyyy-MM-dd')`",`n  `"cases`": [")
    $first = $true
    $sw = [Diagnostics.Stopwatch]::StartNew()
    foreach ($case in $suite.cases) {
        $spec = Read-Spec $case.query
        [void]$out.Append($(if ($first) { "`n" } else { ",`n" }))
        $first = $false
        [void]$out.Append("    {`n      `"id`": $(Json $case.id),`n      `"query`": $(Json $case.query),`n")
        if ($spec.ContainsKey('keywords')) {
            $base = Get-Token (Invoke-Probe $spec $fixture @())
            # The level the context itself needs (its prefix object); its rows need at least that.
            $baseLevel = if ($base -eq 'ok') { Get-MinLevel $spec $fixture @() $fixtureLevel }
            $rows = foreach ($kw in $spec['keywords'].Split(' ', [StringSplitOptions]::RemoveEmptyEntries)) {
                $forms = "$kw abc", $kw, "${kw}: abc", "$kw = 1", "$kw abc = 1"
                $v = @($forms | ForEach-Object { Get-Token (Invoke-Probe $spec $fixture @($_)) })
                if (@($v | Where-Object { $_ -ne 'unknown' -and $_ -ne 'unsupported' }).Count -eq 0) { continue }
                $cells = @($kw) + $v
                if ($v[2] -match '^type:(.+)$') {
                    # A value of the right type: is the property allowed here at all?
                    $sample = switch ($Matches[1]) { 'Boolean' { 'true' } { $_ -in 'Int32', 'Int64', 'Double' } { '1' } default { '0' } }
                    $cells += $sample, (Get-Token (Invoke-Probe $spec $fixture @("${kw}: $sample")))
                }
                for ($i = 0; $i -lt $forms.Count; $i++) {
                    if ($v[$i] -eq 'ok') {
                        $min = Get-MinLevel $spec $fixture @($forms[$i]) $fixtureLevel
                        if ($min) { $cells[$i + 1] = "ok>=$min" }
                    }
                }
                '[' + (($cells | ForEach-Object { Json $_ }) -join ', ') + ']'
            }
            [void]$out.Append("      `"baseline`": $(Json $base),`n")
            if ($baseLevel) { [void]$out.Append("      `"baselineLevel`": $baseLevel,`n") }
            [void]$out.Append("      `"keywords`": [`n        " + (@($rows) -join ",`n        ") + "`n      ]`n")
        } elseif ($spec.ContainsKey('values')) {
            $kw = $spec['keyword']
            $rows = foreach ($value in $spec['values'].Split(' ', [StringSplitOptions]::RemoveEmptyEntries)) {
                '[' + (Json $value) + ', ' + (Json (Get-Token (Invoke-Probe $spec $fixture @("${kw}: $value")))) + ']'
            }
            [void]$out.Append("      `"values`": [" + (@($rows) -join ', ') + "]`n")
        } else {
            $rel = $spec['file']
            $path = Join-Path $work $rel
            $exists = Test-Path $path
            $orig = if ($exists) { [IO.File]::ReadAllText($path).Replace("`r`n", "`n") } else { '' }
            $find = $spec['find']
            if ($find -eq '') { $text = $spec['replace'] }
            else {
                $at = $orig.IndexOf($find)
                if ($at -lt 0 -or $orig.IndexOf($find, $at + 1) -ge 0) { throw "$($case.id): find text not found once in $rel" }
                $text = $orig.Substring(0, $at) + $spec['replace'] + $orig.Substring($at + $find.Length)
            }
            New-Item -ItemType Directory -Force (Split-Path $path) | Out-Null
            [IO.File]::WriteAllText($path, $text)
            try { $r = Invoke-Tom } finally { if ($exists) { [IO.File]::WriteAllText($path, $orig) } else { Remove-Item $path } }
            $token = Get-Token $r
            $fields = @("`"verdict`": $(Json $token)")
            if (-not $r.Ok -and $r.ContainsKey('Line') -and [int]$r.Line -gt 0) {
                $doc = ($r.Document -replace '^\./', '') + '.tmdl'
                $fields += "`"document`": $(Json $doc)", "`"line`": $([int]$r.Line)"
            }
            if ($r.Ok -and $r.Validation.Count) {
                $fields += "`"validation`": [" + (($r.Validation | ForEach-Object { Json $_ }) -join ', ') + ']'
            }
            if (-not $r.Ok -and $token -eq 'unresolved') {
                $refs = @(($r.Message -split "`n") | Where-Object { $_ -match '^\s*Property ' } | ForEach-Object { $_.Trim() })
                $fields += "`"references`": [" + (($refs | ForEach-Object { Json $_ }) -join ', ') + ']'
            }
            [void]$out.Append('      ' + ($fields -join ",`n      ") + "`n")
        }
        [void]$out.Append('    }')
    }
    [void]$out.Append("`n  ]`n}`n")
    $path = Join-Path $OutDir "$($suite.suite).json"
    [IO.File]::WriteAllText($path, $out.ToString(), [Text.UTF8Encoding]::new($false))
    Write-Host ("{0}: {1} cases in {2:n0} s -> {3}" -f $suite.suite, $suite.cases.Count, $sw.Elapsed.TotalSeconds, $path)
}
if (Test-Path $work) { Remove-Item -Recurse -Force $work }
