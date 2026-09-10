# Copyright 2026 Mark Alan Boykin
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
# SPDX-License-Identifier: MPL-2.0

# Standalone diagnostic: a synthetic OS-level pointer click at the "Field
# notes" hyperlink's ClientToScreen-space bounding-rect center, to check
# whether Ortet's pointer pipeline still navigates article.html ->
# notes.html even though AccessKit advertises no UIA patterns (and so no
# InvokePattern) for that link. This is evidence for the O2 bridge-action
# Invoke residual, not a pass/fail gate case.

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$ArtifactDir,
    [Parameter(Mandatory)]
    [string]$Exe,
    [ValidateRange(1, 300000)]
    [int]$TimeoutMs = 40000
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$artifact = [IO.Path]::GetFullPath($ArtifactDir)
$fixture = Join-Path $repo 'ports\ortet\examples\article.html'
New-Item -ItemType Directory -Path $artifact -Force | Out-Null

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -Namespace OrtetReceipt -Name Pointer -MemberDefinition @'
[DllImport("user32.dll")]
public static extern bool SetCursorPos(int x, int y);
[DllImport("user32.dll")]
public static extern void mouse_event(uint dwFlags, uint dx, uint dy, uint dwData, System.IntPtr dwExtraInfo);
[DllImport("user32.dll")]
public static extern bool SetForegroundWindow(System.IntPtr hWnd);
'@
$MOUSEEVENTF_LEFTDOWN = 0x0002
$MOUSEEVENTF_LEFTUP = 0x0004

function ToFileUri { param([string]$Path) return ([Uri]::new($Path)).AbsoluteUri }

function Wait-OrtetWindow {
    param([System.Diagnostics.Process]$Proc, [int]$TimeoutMs = 60000)
    $desktop = [System.Windows.Automation.AutomationElement]::RootElement
    $deadline = (Get-Date).AddMilliseconds($TimeoutMs)
    while ((Get-Date) -lt $deadline) {
        $Proc.Refresh()
        if ($Proc.HasExited) { throw "ortet.exe exited before presenting a window (exit $($Proc.ExitCode))." }
        $kids = $desktop.FindAll([System.Windows.Automation.TreeScope]::Children, [System.Windows.Automation.Condition]::TrueCondition)
        foreach ($candidate in $kids) {
            try {
                if ($candidate.Current.ProcessId -eq $Proc.Id -and $candidate.Current.Name -like 'ortet *') {
                    return $candidate
                }
            }
            catch {}
        }
        Start-Sleep -Milliseconds 300
    }
    throw 'ortet.exe never presented its titled window within the timeout.'
}

function Find-ByName {
    param([System.Windows.Automation.AutomationElement]$Root, [string]$Name, [int]$TimeoutMs = 10000)
    $condition = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::NameProperty, $Name)
    $deadline = (Get-Date).AddMilliseconds($TimeoutMs)
    while ((Get-Date) -lt $deadline) {
        $found = $Root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
        if ($found) { return $found }
        Start-Sleep -Milliseconds 150
    }
    throw "UIAutomation element named '$Name' was never observed."
}

function Stop-IfAlive {
    param([System.Diagnostics.Process]$Proc)
    $Proc.Refresh()
    if (-not $Proc.HasExited) {
        try { $Proc.CloseMainWindow() | Out-Null } catch {}
        Start-Sleep -Milliseconds 300
        $Proc.Refresh()
        if (-not $Proc.HasExited) { $Proc.Kill() }
        $Proc.WaitForExit(5000) | Out-Null
    }
}

$articleUri = ToFileUri $fixture
$log = Join-Path $artifact 'receipt.log'
$err = Join-Path $artifact 'receipt.log.err'
$png = Join-Path $artifact 'receipt.png'
$cmdLine = "--url $articleUri --size 640x400 --expect-heading `"Field notes`" --timeout-ms $TimeoutMs --artifact `"$png`""
Set-Content -Path (Join-Path $artifact 'cmdline.txt') -Value $cmdLine
$proc = Start-Process -FilePath $Exe -ArgumentList $cmdLine -PassThru `
    -RedirectStandardOutput $log -RedirectStandardError $err
$driver = New-Object System.Collections.Generic.List[string]
try {
    $win = Wait-OrtetWindow -Proc $proc
    $driver.Add("titled window acquired: '$($win.Current.Name)' NativeWindowHandle=$($win.Current.NativeWindowHandle)")
    $fgOk = [OrtetReceipt.Pointer]::SetForegroundWindow($win.Current.NativeWindowHandle)
    $driver.Add("SetForegroundWindow returned $fgOk")
    Start-Sleep -Milliseconds 300

    # First click activates the window (Windows commonly swallows the first
    # click on an inactive window as pure activation rather than delivering
    # it to the control under the cursor), so an initial neutral click inside
    # the window establishes foreground/input focus before the real click.
    $winRect = $win.Current.BoundingRectangle
    $neutralX = [int]($winRect.X + 10)
    $neutralY = [int]($winRect.Y + 10)
    [OrtetReceipt.Pointer]::SetCursorPos($neutralX, $neutralY) | Out-Null
    Start-Sleep -Milliseconds 150
    [OrtetReceipt.Pointer]::mouse_event($MOUSEEVENTF_LEFTDOWN, 0, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 40
    [OrtetReceipt.Pointer]::mouse_event($MOUSEEVENTF_LEFTUP, 0, 0, 0, [IntPtr]::Zero)
    $driver.Add("neutral activation click at screen ($neutralX, $neutralY)")
    Start-Sleep -Milliseconds 300

    $link = Find-ByName -Root $win -Name 'Field notes'
    $rect = $link.Current.BoundingRectangle
    $driver.Add("found hyperlink 'Field notes' at $($rect.ToString())")
    $cx = [int]($rect.X + $rect.Width / 2)
    $cy = [int]($rect.Y + $rect.Height / 2)
    [OrtetReceipt.Pointer]::SetCursorPos($cx, $cy) | Out-Null
    Start-Sleep -Milliseconds 200
    [OrtetReceipt.Pointer]::mouse_event($MOUSEEVENTF_LEFTDOWN, 0, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 60
    [OrtetReceipt.Pointer]::mouse_event($MOUSEEVENTF_LEFTUP, 0, 0, 0, [IntPtr]::Zero)
    $driver.Add("synthetic pointer click at screen ($cx, $cy) [bounding-rect center of 'Field notes']")
    if (-not $proc.WaitForExit($TimeoutMs + 10000)) {
        throw 'ortet.exe did not exit within the expected window.'
    }
}
finally {
    Stop-IfAlive -Proc $proc
    $driver | Set-Content (Join-Path $artifact 'driver.log')
}
$output = Get-Content $log -Raw -ErrorAction SilentlyContinue
$navigated = ($output -match [regex]::Escape('settled at')) -and ($output -match 'notes\.html')
[pscustomobject]@{
    exitCode = $proc.ExitCode
    navigatedToNotesHtml = [bool]$navigated
} | ConvertTo-Json | Set-Content (Join-Path $artifact 'result.json')
Write-Host "pointer diagnostic: navigatedToNotesHtml=$navigated exitCode=$($proc.ExitCode)"
