# Copyright 2026 Mark Alan Boykin
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
# SPDX-License-Identifier: MPL-2.0

# Native, headed, real-UIAutomation acceptance of the O2 bridge-action gate's
# rejection conditions. The 2026-09-05 receipt drove the accepted positive
# path (find, focus, re-acquire, invoke) natively; the fresh-ID publication
# policy's *rejections* (a queued action against a retired publication, or an
# action no longer advertised) had only ever been exercised by a11y.rs's
# in-process unit tests. This runner reuses the accepted host and fixture and
# adds a native negative case: a real Windows UIAutomation client that caches
# a bridge element, forces a republish (which retires that element's node
# id, since a fresh id is allocated on every publication), and then invokes
# the stale cached element through the real OS accessibility stack.

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$ArtifactDir,
    [string]$TargetDir,
    [ValidateRange(1, 300000)]
    [int]$InvokeTimeoutMs = 75000,
    [ValidateRange(1, 300000)]
    [int]$StaleTimeoutMs = 40000
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$artifact = [IO.Path]::GetFullPath($ArtifactDir)
$target = if ($TargetDir) { [IO.Path]::GetFullPath($TargetDir) } else { Join-Path $artifact 'target' }
$fixture = Join-Path $repo 'ports\ortet\examples\article.html'
$notesFixture = Join-Path $repo 'ports\ortet\examples\notes.html'
New-Item -ItemType Directory -Path $artifact -Force | Out-Null

if (-not (Test-Path $fixture)) { throw "fixture missing: $fixture" }
if (-not (Test-Path $notesFixture)) { throw "fixture missing: $notesFixture" }

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

function ToFileUri {
    param([string]$Path)
    return ([Uri]::new($Path)).AbsoluteUri
}

function Wait-OrtetWindow {
    # `Process.MainWindowHandle` is unreliable for this binary: under this
    # machine's build/GPU contention, ortet.exe's console pseudo-window (an
    # empty, nameless Pane) is what .NET reports as the "main" window for
    # several seconds before the real winit surface finishes booting (wgpu
    # adapter negotiation and first paint), so the desktop's own top-level
    # children are searched directly by process id and title instead.
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

function Invoke-Element {
    param([System.Windows.Automation.AutomationElement]$Element)
    $pattern = $Element.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
    $pattern.Invoke()
}

function Quote-Arg {
    param([string]$Value)
    # Start-Process -ArgumentList does not reliably quote array elements
    # containing spaces across PowerShell hosts, so the full command line is
    # built as one pre-quoted string instead.
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
    }
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
    $notesUri = ToFileUri $notesFixture

    # --- Case A: invoke (reconfirms the accepted positive O2 native gate) ---
    $caseA = Join-Path $artifact 'invoke'
    New-Item -ItemType Directory -Path $caseA -Force | Out-Null
    $logA = Join-Path $caseA 'receipt.log'
    $pngA = Join-Path $caseA 'receipt.png'
    $driverA = Join-Path $caseA 'driver.log'
    $driverLines = New-Object System.Collections.Generic.List[string]
    function Log-Driver { param([string]$Line) $driverLines.Add("$(Get-Date -Format o) $Line") }

    $cmdLineA = Build-CommandLine @(
        '--url', $articleUri, '--size', '640x400',
        '--expect-heading', 'Field notes',
        '--timeout-ms', "$InvokeTimeoutMs",
        '--artifact', $pngA
    )
    Set-Content -Path (Join-Path $caseA 'cmdline.txt') -Value $cmdLineA
    $procA = Start-Process -FilePath $exe -ArgumentList $cmdLineA `
        -PassThru -RedirectStandardOutput $logA -RedirectStandardError "$logA.err"
    try {
        $winElem = Wait-OrtetWindow -Proc $procA
        Log-Driver "titled window acquired: '$($winElem.Current.Name)'"
        $heading = Find-ByName -Root $winElem -Name 'Ortet'
        Log-Driver "found heading 'Ortet' (ControlType=$($heading.Current.ControlType.ProgrammaticName))"
        $link = Find-ByName -Root $winElem -Name 'Field notes'
        Log-Driver "found hyperlink 'Field notes' (ControlType=$($link.Current.ControlType.ProgrammaticName))"

        $link.SetFocus()
        Log-Driver 'issued native SetFocus on Field notes (Action::Focus over the installed bridge)'
        Start-Sleep -Milliseconds 400

        # Re-acquire: the fresh-ID policy retires every node id on republish,
        # so the accepted path re-finds the element after the focus-driven
        # republish rather than reusing the one found above.
        $linkFresh = Find-ByName -Root $winElem -Name 'Field notes'
        Log-Driver 're-acquired Field notes after the focus republish'
        Invoke-Element -Element $linkFresh
        Log-Driver 'issued native Invoke on the re-acquired Field notes hyperlink'

        if (-not $procA.WaitForExit($InvokeTimeoutMs + 10000)) {
            throw 'ortet.exe (invoke case) did not exit within the expected window.'
        }
        if ($procA.ExitCode -ne 0) {
            throw "ortet.exe (invoke case) exited $($procA.ExitCode); see $logA"
        }
    }
    finally {
        Stop-IfAlive -Proc $procA
        $driverLines | Set-Content $driverA
    }

    $outputA = Get-Content $logA -Raw
    if ($outputA -notmatch [regex]::Escape('accessibility bridge installed')) { throw 'invoke case: bridge was not installed.' }
    if ($outputA -notmatch [regex]::Escape("source $sourceRevision")) { throw 'invoke case: log lacks exact source revision.' }
    if ($outputA -notmatch [regex]::Escape('semantic heading "Field notes"')) { throw 'invoke case: did not report the Field notes heading.' }
    if ($outputA -notmatch [regex]::Escape("settled at $notesUri")) { throw 'invoke case: did not settle at notes.html.' }
    if (-not (Test-Path $pngA)) { throw 'invoke case: no receipt PNG was written.' }
    (Get-FileHash -Algorithm SHA256 $pngA).Hash.ToLowerInvariant() + '  receipt.png' | Set-Content (Join-Path $caseA 'receipt.sha256')

    # --- Case B: stale-reject (the deliberate failing control) ---
    $caseB = Join-Path $artifact 'stale-reject'
    New-Item -ItemType Directory -Path $caseB -Force | Out-Null
    $logB = Join-Path $caseB 'receipt.log'
    $pngB = Join-Path $caseB 'receipt.png'
    $driverB = Join-Path $caseB 'driver.log'
    $driverLinesB = New-Object System.Collections.Generic.List[string]
    function Log-DriverB { param([string]$Line) $driverLinesB.Add("$(Get-Date -Format o) $Line") }
    $impossibleHeading = 'Ortet impossible bridge-action heading'

    $cmdLineB = Build-CommandLine @(
        '--url', $articleUri, '--size', '640x400',
        '--expect-heading', $impossibleHeading,
        '--timeout-ms', "$StaleTimeoutMs",
        '--artifact', $pngB
    )
    $procB = Start-Process -FilePath $exe -ArgumentList $cmdLineB `
        -PassThru -RedirectStandardOutput $logB -RedirectStandardError "$logB.err"
    $invokeException = $null
    try {
        $winElemB = Wait-OrtetWindow -Proc $procB
        Log-DriverB "titled window acquired: '$($winElemB.Current.Name)'"
        $headingBefore = Find-ByName -Root $winElemB -Name 'Ortet'
        Log-DriverB "found heading 'Ortet' before any action"
        $staleLink = Find-ByName -Root $winElemB -Name 'Field notes'
        Log-DriverB "cached hyperlink 'Field notes' (stale reference to be reused after republish)"

        $staleLink.SetFocus()
        Log-DriverB 'issued native SetFocus on Field notes to force a republish (fresh host ids, same generation)'
        Start-Sleep -Milliseconds 400

        try {
            Invoke-Element -Element $staleLink
            Log-DriverB 'native Invoke on the STALE cached element returned without throwing; checking host-side rejection.'
        }
        catch {
            $invokeException = $_.Exception
            Log-DriverB "native Invoke on the STALE cached element threw at the OS/UIAutomation layer: $($invokeException.GetType().FullName): $($invokeException.Message)"
        }

        Start-Sleep -Milliseconds 400
        $headingAfter = Find-ByName -Root $winElemB -Name 'Ortet'
        Log-DriverB "heading 'Ortet' is still present after the stale invoke attempt (no navigation occurred)"

        if (-not $procB.WaitForExit($StaleTimeoutMs + 10000)) {
            throw 'ortet.exe (stale-reject case) did not exit within the expected window.'
        }
        if ($procB.ExitCode -eq 0) {
            throw 'ortet.exe (stale-reject case) unexpectedly exited 0 — the deliberate control must fail.'
        }
    }
    finally {
        Stop-IfAlive -Proc $procB
        $driverLinesB | Set-Content $driverB
    }

    $outputB = Get-Content $logB -Raw
    $errOutputB = if (Test-Path "$logB.err") { Get-Content "$logB.err" -Raw } else { '' }
    if ($outputB -notmatch [regex]::Escape('accessibility bridge installed')) { throw 'stale-reject case: bridge was not installed.' }
    $timeoutMessage = "was absent before the ${StaleTimeoutMs}ms deadline"
    if (($errOutputB + $outputB) -notmatch [regex]::Escape($timeoutMessage)) {
        throw "stale-reject case: did not report the expected bounded-deadline failure ($timeoutMessage)."
    }
    $rejectPattern = 'ortet: receipt bridge-action rejected target=\S+ action=Click'
    $hostRejected = [regex]::IsMatch($outputB, $rejectPattern)
    if (-not $hostRejected -and -not $invokeException) {
        throw 'stale-reject case: neither the OS/UIAutomation layer nor the host route() rejected the stale invoke — the deliberate control did not fail as required.'
    }
    [pscustomobject]@{
        hostSideRejectionLogged = $hostRejected
        osLayerException = if ($invokeException) { "$($invokeException.GetType().FullName): $($invokeException.Message)" } else { $null }
        exitCode = $procB.ExitCode
    } | ConvertTo-Json | Set-Content (Join-Path $caseB 'rejection-evidence.json')
}
finally {
    Pop-Location
}

Write-Host 'O2 bridge-action receipt: PASS (invoke accepted; stale-reject refused)'
