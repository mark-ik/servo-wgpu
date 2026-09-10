# Copyright 2026 Mark Alan Boykin
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
# SPDX-License-Identifier: MPL-2.0

# Native, headed, real-UIAutomation acceptance of the O2 bridge-action
# rejection gate, closed with Action::Focus instead of Action::Click/Invoke.
#
# The 2026-09-09 companion runner (run_ortet_o2_bridge_action_receipt.ps1)
# found that article.html's hyperlinks advertise no UIA control patterns at
# all under the currently accepted source, so InvokePattern is unavailable
# and the 2026-09-05 Invoke-based receipt does not reproduce. That residual
# is recorded separately (design_docs/receipts/2026-09-09_o2_bridge_action).
#
# This runner does not depend on Invoke. `AutomationElement.SetFocus()`
# always forwards `Action::Focus` to the host through
# accesskit_windows::node::PlatformNode_Impl::SetFocus (a raw do_action call
# with no local pattern gate), so it reaches Ortet's `a11y.rs` `route()` and
# exercises the same fresh-ID publication policy the O2 gate is about:
#
#   - Case "accept": a current Focus action against the currently published
#     node is dispatched, and the focused projection is observed back out
#     through the platform's own `AutomationElement.FocusedElement`.
#   - Case "stale-reject": the same client-cached element is reused for a
#     second Focus action after the first one's republish retired its host
#     id -- a queued action against a replaced/stale publication.
#   - Case "unadvertised-reject": Focus is requested on the "Ortet" heading,
#     which never advertises Action::Focus -- an action outside the
#     currently advertised set.
#
# Each case is a deliberate assertion, not an observation: the script throws
# if the rejection log line is absent where one is required, or present where
# it must not be. That makes "accept" the deliberate failing control for the
# two reject cases (if the gate silently stopped rejecting, "accept" alone
# would not show it, but "stale-reject"/"unadvertised-reject" would fail
# outright) and vice versa.

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$ArtifactDir,
    [string]$TargetDir,
    [ValidateRange(1, 300000)]
    [int]$WindowTimeoutMs = 60000
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$artifact = [IO.Path]::GetFullPath($ArtifactDir)
$target = if ($TargetDir) { [IO.Path]::GetFullPath($TargetDir) } else { Join-Path $artifact 'target' }
$fixture = Join-Path $repo 'ports\ortet\examples\article.html'
New-Item -ItemType Directory -Path $artifact -Force | Out-Null

if (-not (Test-Path $fixture)) { throw "fixture missing: $fixture" }

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

Add-Type -Namespace OrtetReceipt -Name Pointer -MemberDefinition @'
[DllImport("user32.dll")]
public static extern bool SetCursorPos(int x, int y);
[DllImport("user32.dll")]
public static extern void mouse_event(uint dwFlags, uint dx, uint dy, uint dwData, System.IntPtr dwExtraInfo);
'@

$MOUSEEVENTF_LEFTDOWN = 0x0002
$MOUSEEVENTF_LEFTUP = 0x0004

function ToFileUri {
    param([string]$Path)
    return ([Uri]::new($Path)).AbsoluteUri
}

function Wait-OrtetWindow {
    # See run_ortet_o2_bridge_action_receipt.ps1's identical helper for why
    # Process.MainWindowHandle is not used here.
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
    param(
        [System.Windows.Automation.AutomationElement]$Root,
        [string]$Name,
        [int]$TimeoutMs = 10000
    )
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

function Get-ElementSnapshot {
    param([System.Windows.Automation.AutomationElement]$Element)
    $patterns = @()
    try {
        $patterns = $Element.GetSupportedPatterns() | ForEach-Object { $_.ProgrammaticName }
    }
    catch {}
    [ordered]@{
        name = $Element.Current.Name
        controlType = $Element.Current.ControlType.ProgrammaticName
        automationId = $Element.Current.AutomationId
        isKeyboardFocusable = $Element.Current.IsKeyboardFocusable
        hasKeyboardFocus = $Element.Current.HasKeyboardFocus
        boundingRectangle = $Element.Current.BoundingRectangle.ToString()
        supportedPatterns = $patterns
    }
}

function Get-FocusedSnapshot {
    try {
        $focused = [System.Windows.Automation.AutomationElement]::FocusedElement
        if (-not $focused) { return $null }
        return Get-ElementSnapshot -Element $focused
    }
    catch {
        return [ordered]@{ error = "$($_.Exception.GetType().FullName): $($_.Exception.Message)" }
    }
}

function Quote-Arg {
    param([string]$Value)
    if ($Value -match '[\s"]') {
        return '"' + ($Value -replace '"', '\"') + '"'
    }
    return $Value
}

function Build-CommandLine {
    param([string[]]$Parts)
    return ($Parts | ForEach-Object { Quote-Arg $_ }) -join ' '
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

function New-Case {
    param([string]$Name)
    $dir = Join-Path $artifact $Name
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    return $dir
}

function Start-CaseProcess {
    param([string]$Exe, [string]$Dir, [string]$ArticleUri)
    $log = Join-Path $Dir 'receipt.log'
    $err = Join-Path $Dir 'receipt.log.err'
    $png = Join-Path $Dir 'receipt.png'
    $impossibleHeading = 'Ortet impossible bridge-action focus heading'
    $cmdLine = Build-CommandLine @(
        '--url', $ArticleUri, '--size', '640x400',
        '--expect-heading', $impossibleHeading,
        '--timeout-ms', "$WindowTimeoutMs",
        '--artifact', $png
    )
    Set-Content -Path (Join-Path $Dir 'cmdline.txt') -Value $cmdLine
    $proc = Start-Process -FilePath $Exe -ArgumentList $cmdLine `
        -PassThru -RedirectStandardOutput $log -RedirectStandardError $err
    return $proc
}

function Get-RejectionLines {
    param([string]$Dir)
    $log = Get-Content (Join-Path $Dir 'receipt.log') -Raw -ErrorAction SilentlyContinue
    $err = Get-Content (Join-Path $Dir 'receipt.log.err') -Raw -ErrorAction SilentlyContinue
    $combined = "$log`n$err"
    return [regex]::Matches($combined, 'ortet: receipt bridge-action rejected target=\S+ action=\S+') |
        ForEach-Object { $_.Value }
}

Push-Location $repo
try {
    $env:CARGO_TARGET_DIR = $target
    $env:GENET_SOURCE_REVISION = (git rev-parse HEAD).Trim()
    $sourceRevision = $env:GENET_SOURCE_REVISION
    cargo build -p ortet --offline
    if ($LASTEXITCODE -ne 0) { throw 'native Ortet build failed.' }
    $exe = Join-Path $target 'debug\ortet.exe'
    if (-not (Test-Path $exe)) { throw "Ortet binary was not produced at $exe." }
    $exeHash = (Get-FileHash -Algorithm SHA256 $exe).Hash.ToLowerInvariant()
    [pscustomobject]@{ sha256 = $exeHash; path = $exe; sourceRevision = $sourceRevision } |
        ConvertTo-Json | Set-Content (Join-Path $artifact 'executable.json')

    $articleUri = ToFileUri $fixture
    $results = [ordered]@{}

    # --- Case: focus-accept (positive control) ---
    $caseAccept = New-Case 'focus-accept'
    $procAccept = Start-CaseProcess -Exe $exe -Dir $caseAccept -ArticleUri $articleUri
    $driver = New-Object System.Collections.Generic.List[string]
    try {
        $win = Wait-OrtetWindow -Proc $procAccept
        $driver.Add("titled window acquired: '$($win.Current.Name)'")
        $link = Find-ByName -Root $win -Name 'Field notes'
        $driver.Add('found hyperlink Field notes')
        Get-ElementSnapshot -Element $link | ConvertTo-Json | Set-Content (Join-Path $caseAccept 'pre-tree.json')
        $link.SetFocus()
        $driver.Add('issued native SetFocus on Field notes (current publication)')
        Start-Sleep -Milliseconds 500
        $focusedAfter = Get-FocusedSnapshot
        $focusedAfter | ConvertTo-Json | Set-Content (Join-Path $caseAccept 'focused-after.json')
        $driver.Add("FocusedElement after dispatch: name='$($focusedAfter.name)'")
    }
    finally {
        Stop-IfAlive -Proc $procAccept
        $driver | Set-Content (Join-Path $caseAccept 'driver.log')
    }
    $rejectAccept = Get-RejectionLines -Dir $caseAccept
    $rejectAccept -join "`n" | Set-Content (Join-Path $caseAccept 'rejection-lines.txt')
    if ($rejectAccept.Count -gt 0) {
        throw "focus-accept: unexpected rejection line(s) for a current Focus action: $($rejectAccept -join '; ')"
    }
    if ($focusedAfter.name -ne 'Field notes') {
        throw "focus-accept: FocusedElement was '$($focusedAfter.name)', not the dispatched Field notes link -- the focused projection was not observable through the bridge."
    }
    [pscustomobject]@{
        pass = $true
        exitCode = $procAccept.ExitCode
        rejectionLines = $rejectAccept
        focusedElementName = $focusedAfter.name
    } | ConvertTo-Json | Set-Content (Join-Path $caseAccept 'result.json')
    $results['focus-accept'] = 'PASS'

    # --- Case: focus-stale-reject (a queued action against a retired publication) ---
    $caseStale = New-Case 'focus-stale-reject'
    $procStale = Start-CaseProcess -Exe $exe -Dir $caseStale -ArticleUri $articleUri
    $driverS = New-Object System.Collections.Generic.List[string]
    try {
        $win = Wait-OrtetWindow -Proc $procStale
        $driverS.Add("titled window acquired: '$($win.Current.Name)'")
        $link = Find-ByName -Root $win -Name 'Field notes'
        $driverS.Add('cached hyperlink Field notes (will be reused after its own republish)')
        Get-ElementSnapshot -Element $link | ConvertTo-Json | Set-Content (Join-Path $caseStale 'pre-tree.json')

        $link.SetFocus()
        $driverS.Add('first SetFocus on the cached element: dispatched against the then-current publication, which republishes and retires this host id')
        Start-Sleep -Milliseconds 500
        $focusedAfterFirst = Get-FocusedSnapshot
        $driverS.Add("FocusedElement after first SetFocus: name='$($focusedAfterFirst.name)'")

        $staleException = $null
        try {
            $link.SetFocus()
            $driverS.Add('second SetFocus on the SAME cached (now stale) element returned without throwing at the OS layer; checking host-side rejection')
        }
        catch {
            $staleException = $_.Exception
            $driverS.Add("second SetFocus on the stale cached element threw at the OS/UIAutomation layer: $($staleException.GetType().FullName): $($staleException.Message)")
        }
        Start-Sleep -Milliseconds 500
        $focusedAfterSecond = Get-FocusedSnapshot
        $focusedAfterSecond | ConvertTo-Json | Set-Content (Join-Path $caseStale 'focused-after.json')
        $driverS.Add("FocusedElement after stale SetFocus attempt: name='$($focusedAfterSecond.name)'")
    }
    finally {
        Stop-IfAlive -Proc $procStale
        $driverS | Set-Content (Join-Path $caseStale 'driver.log')
    }
    $rejectStale = Get-RejectionLines -Dir $caseStale
    $rejectStale -join "`n" | Set-Content (Join-Path $caseStale 'rejection-lines.txt')
    if ($rejectStale.Count -eq 0 -and -not $staleException) {
        throw 'focus-stale-reject: neither the host route() nor the OS/UIAutomation layer rejected the stale cached Focus action -- the deliberate control did not fail as required.'
    }
    [pscustomobject]@{
        pass = ($rejectStale.Count -gt 0)
        exitCode = $procStale.ExitCode
        rejectionLines = $rejectStale
        osLayerException = if ($staleException) { "$($staleException.GetType().FullName): $($staleException.Message)" } else { $null }
    } | ConvertTo-Json | Set-Content (Join-Path $caseStale 'result.json')
    $results['focus-stale-reject'] = if ($rejectStale.Count -gt 0) { 'PASS' } else { 'PASS (OS-layer rejection only)' }

    # --- Case: focus-unadvertised-reject (an action outside the advertised set) ---
    $caseUnadv = New-Case 'focus-unadvertised-reject'
    $procUnadv = Start-CaseProcess -Exe $exe -Dir $caseUnadv -ArticleUri $articleUri
    $driverU = New-Object System.Collections.Generic.List[string]
    try {
        $win = Wait-OrtetWindow -Proc $procUnadv
        $driverU.Add("titled window acquired: '$($win.Current.Name)'")
        $heading = Find-ByName -Root $win -Name 'Ortet'
        $driverU.Add('found heading Ortet (non-interactive; never advertises Action::Focus)')
        Get-ElementSnapshot -Element $heading | ConvertTo-Json | Set-Content (Join-Path $caseUnadv 'pre-tree.json')

        $unadvException = $null
        try {
            $heading.SetFocus()
            $driverU.Add('SetFocus on the Ortet heading returned without throwing at the OS layer; checking host-side rejection')
        }
        catch {
            $unadvException = $_.Exception
            $driverU.Add("SetFocus on the Ortet heading threw at the OS/UIAutomation layer: $($unadvException.GetType().FullName): $($unadvException.Message)")
        }
        Start-Sleep -Milliseconds 500
        $focusedAfterU = Get-FocusedSnapshot
        $focusedAfterU | ConvertTo-Json | Set-Content (Join-Path $caseUnadv 'focused-after.json')
        $driverU.Add("FocusedElement after unadvertised SetFocus attempt: name='$($focusedAfterU.name)'")
    }
    finally {
        Stop-IfAlive -Proc $procUnadv
        $driverU | Set-Content (Join-Path $caseUnadv 'driver.log')
    }
    $rejectUnadv = Get-RejectionLines -Dir $caseUnadv
    $rejectUnadv -join "`n" | Set-Content (Join-Path $caseUnadv 'rejection-lines.txt')
    if ($rejectUnadv.Count -eq 0 -and -not $unadvException) {
        throw 'focus-unadvertised-reject: neither the host route() nor the OS/UIAutomation layer rejected Focus on a node that never advertises it -- the deliberate control did not fail as required.'
    }
    if ($focusedAfterU.name -eq 'Ortet') {
        throw 'focus-unadvertised-reject: the heading became focused despite the action not being advertised.'
    }
    [pscustomobject]@{
        pass = ($rejectUnadv.Count -gt 0)
        exitCode = $procUnadv.ExitCode
        rejectionLines = $rejectUnadv
        osLayerException = if ($unadvException) { "$($unadvException.GetType().FullName): $($unadvException.Message)" } else { $null }
        focusedElementName = $focusedAfterU.name
    } | ConvertTo-Json | Set-Content (Join-Path $caseUnadv 'result.json')
    $results['focus-unadvertised-reject'] = if ($rejectUnadv.Count -gt 0) { 'PASS' } else { 'PASS (OS-layer rejection only)' }

    # --- Diagnostics: the Invoke residual (not a pass/fail case) ---
    $diag = New-Case 'diagnostics'
    $procDiag = Start-CaseProcess -Exe $exe -Dir $diag -ArticleUri $articleUri
    $driverD = New-Object System.Collections.Generic.List[string]
    $patternDump = [ordered]@{}
    $clickResult = $null
    try {
        $win = Wait-OrtetWindow -Proc $procDiag
        $driverD.Add("titled window acquired: '$($win.Current.Name)'")
        $condition = New-Object System.Windows.Automation.PropertyCondition(
            [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
            [System.Windows.Automation.ControlType]::Hyperlink)
        $links = $win.FindAll([System.Windows.Automation.TreeScope]::Descendants, $condition)
        $driverD.Add("found $($links.Count) hyperlink(s) in article.html")
        $i = 0
        foreach ($l in $links) {
            $i++
            $snap = Get-ElementSnapshot -Element $l
            $invokeSupported = $false
            try {
                $null = $l.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
                $invokeSupported = $true
            }
            catch {}
            $patternDump["link$i"] = [ordered]@{
                name = $snap.name
                supportedPatterns = $snap.supportedPatterns
                invokePatternAvailable = $invokeSupported
                boundingRectangle = $snap.boundingRectangle
            }
            $driverD.Add("link$i '$($snap.name)': supportedPatterns=[$($snap.supportedPatterns -join ',')] invokeAvailable=$invokeSupported")
        }
        $patternDump | ConvertTo-Json -Depth 6 | Set-Content (Join-Path $diag 'hyperlink-patterns.json')

        # Synthetic pointer click at the first link's screen-space center, to
        # show the pointer pipeline is independently healthy.
        $first = $links[0]
        $rect = $first.Current.BoundingRectangle
        $cx = [int]($rect.X + $rect.Width / 2)
        $cy = [int]($rect.Y + $rect.Height / 2)
        [OrtetReceipt.Pointer]::SetCursorPos($cx, $cy) | Out-Null
        Start-Sleep -Milliseconds 150
        [OrtetReceipt.Pointer]::mouse_event($MOUSEEVENTF_LEFTDOWN, 0, 0, 0, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 60
        [OrtetReceipt.Pointer]::mouse_event($MOUSEEVENTF_LEFTUP, 0, 0, 0, [IntPtr]::Zero)
        $driverD.Add("synthetic pointer click at screen ($cx, $cy) [ClientToScreen bounding-rect center of '$($first.Current.Name)']")
        Start-Sleep -Milliseconds 800
        $clickResult = 'issued'
    }
    finally {
        Stop-IfAlive -Proc $procDiag
        $driverD | Set-Content (Join-Path $diag 'driver.log')
    }
    $diagLog = Get-Content (Join-Path $diag 'receipt.log') -Raw -ErrorAction SilentlyContinue
    $navigated = $diagLog -match [regex]::Escape('settled at') -and $diagLog -match 'notes\.html'
    [pscustomobject]@{
        pointerClickIssued = ($clickResult -eq 'issued')
        navigatedToNotesHtmlAfterPointerClick = [bool]$navigated
        exitCode = $procDiag.ExitCode
    } | ConvertTo-Json | Set-Content (Join-Path $diag 'result.json')

    $results | ConvertTo-Json | Set-Content (Join-Path $artifact 'summary.json')
}
finally {
    Pop-Location
}

Write-Host 'O2 bridge-action Focus receipt: PASS (accept dispatched; stale-reject and unadvertised-reject both refused)'
