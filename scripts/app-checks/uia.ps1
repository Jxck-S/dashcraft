param([int]$ProcessId, [Parameter(ValueFromRemainingArguments)][string[]]$Names)
# Lists the UI Automation elements (what Narrator and NVDA read) of a process's window by name.
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$A = [System.Windows.Automation.AutomationElement]
$cond = New-Object System.Windows.Automation.PropertyCondition($A::ProcessIdProperty, $ProcessId)
$win = $A::RootElement.FindFirst([System.Windows.Automation.TreeScope]::Children, $cond)
if (-not $win) { 'NO WINDOW'; exit 1 }
$all = $win.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
foreach ($n in $Names) {
  $hits = @($all | Where-Object { $_.Current.Name -eq $n })
  if ($hits.Count -eq 0) { "MISSING`t$n"; continue }
  foreach ($e in $hits) {
    $c = $e.Current
    $value = ''
    try { $vp = $e.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern); $value = $vp.Current.Value } catch {}
    if ($value.Length -gt 40) { $value = $value.Substring(0, 40) + '...' }
    "FOUND`t$n`t$($c.ControlType.ProgrammaticName)`tenabled=$($c.IsEnabled)`tfocusable=$($c.IsKeyboardFocusable)`tvalue=$value"
  }
}
