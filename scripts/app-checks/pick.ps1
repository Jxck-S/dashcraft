param([int]$ProcessId, [string]$Title, [string]$Path)
# Answers the app's native open dialog titled $Title with $Path (Windows): finds it with UI
# Automation, writes the path into its file name / folder box and clicks Open / Select Folder
# (Win32 messages: those classic controls expose no UI Automation patterns here).
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type -Namespace DashCraftChecks -Name Win32 -MemberDefinition @'
[DllImport("user32.dll", CharSet = CharSet.Unicode)]
public static extern System.IntPtr SendMessage(System.IntPtr hWnd, uint msg, System.IntPtr wParam, string lParam);
[DllImport("user32.dll")]
public static extern bool PostMessage(System.IntPtr hWnd, uint msg, System.IntPtr wParam, System.IntPtr lParam);
'@
$WM_SETTEXT = 0x000C
$BM_CLICK = 0x00F5
$A = [System.Windows.Automation.AutomationElement]
$scope = [System.Windows.Automation.TreeScope]
$byProcess = New-Object System.Windows.Automation.PropertyCondition($A::ProcessIdProperty, $ProcessId)
$byTitle = New-Object System.Windows.Automation.PropertyCondition($A::NameProperty, $Title)
function Find-Dialog {
  # A top-level window of the process, or a window it owns (a child in the UI Automation tree).
  foreach ($window in $A::RootElement.FindAll($scope::Children, $byProcess)) {
    if ($window.Current.Name -eq $Title) { return $window }
    $owned = $window.FindFirst($scope::Children, $byTitle)
    if ($owned) { return $owned }
  }
  $null
}
function Find-Control([string]$id) {
  # The raw view: the classic edit box and buttons are not in the control view.
  $walker = [System.Windows.Automation.TreeWalker]::RawViewWalker
  $queue = [System.Collections.Generic.Queue[object]]::new()
  $queue.Enqueue($dialog)
  while ($queue.Count -gt 0) {
    $child = $walker.GetFirstChild($queue.Dequeue())
    while ($child) {
      if ($child.Current.AutomationId -eq $id -and $child.Current.ClassName -in 'Edit', 'Button') {
        return [System.IntPtr]$child.Current.NativeWindowHandle
      }
      $queue.Enqueue($child)
      $child = $walker.GetNextSibling($child)
    }
  }
  [System.IntPtr]::Zero
}
$deadline = (Get-Date).AddSeconds(15)
$dialog = $null
while (-not $dialog -and (Get-Date) -lt $deadline) {
  $dialog = Find-Dialog
  if (-not $dialog) { Start-Sleep -Milliseconds 200 }
}
if (-not $dialog) { 'NO DIALOG'; exit 1 }
# 1152: the file name / folder edit box; 1: Open / Select Folder.
$edit = Find-Control '1152'
$ok = Find-Control '1'
if ($edit -eq [System.IntPtr]::Zero -or $ok -eq [System.IntPtr]::Zero) { 'NO CONTROLS'; exit 1 }
[void][DashCraftChecks.Win32]::SendMessage($edit, $WM_SETTEXT, [System.IntPtr]::Zero, $Path)
for ($attempt = 0; $attempt -lt 3; $attempt++) {
  # A typed folder may first be navigated into; the next click then selects it.
  [void][DashCraftChecks.Win32]::PostMessage($ok, $BM_CLICK, [System.IntPtr]::Zero, [System.IntPtr]::Zero)
  Start-Sleep -Milliseconds 700
  if (-not (Find-Dialog)) { 'PICKED'; exit 0 }
}
'STILL OPEN'
exit 1
