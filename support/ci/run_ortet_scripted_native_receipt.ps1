# Copyright 2026 Mark Alan Boykin
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
# SPDX-License-Identifier: MPL-2.0

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$ArtifactDir,
    [string]$TargetDir,
    [ValidateRange(1, 65535)]
    [int]$HttpPort = 18755,
    [ValidateRange(1, 120000)]
    [int]$ReceiptTimeoutMs = 10000
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$artifact = [IO.Path]::GetFullPath($ArtifactDir)
$target = if ($TargetDir) { [IO.Path]::GetFullPath($TargetDir) } else { Join-Path $artifact 'target' }
$fixture = Join-Path $repo 'ports\ortet\tests\native\scripted_receipt.html'
$completion = 'Ortet O5 native receipt complete'
$serverScript = Join-Path $repo 'support\ci\ortet_scripted_native_receipt_server.mjs'
New-Item -ItemType Directory -Path $artifact -Force | Out-Null

function Assert-ReceiptLog {
    param(
        [string]$Log,
        [string]$Engine,
        [string]$Heading,
        [string]$SourceRevision,
        [string]$ExpectedAddress
    )

    $output = Get-Content $Log -Raw
    $engineId = if ($Engine -eq 'nova') { 'genet.scripted.nova' } else { 'genet.scripted' }
    $identity = "engine $engineId backend $Engine presented"
    if ($output -notmatch [regex]::Escape($identity)) { throw "Ortet $Engine log did not report exact engine/backend identity '$identity'." }
    if ($output -notmatch [regex]::Escape("semantic heading `"$Heading`"")) { throw "Ortet $Engine did not report semantic completion $Heading." }
    if ($output -notmatch [regex]::Escape("source $SourceRevision")) { throw "Ortet $Engine receipt lacks the exact source revision." }
    if ($output -notmatch [regex]::Escape("settled at $ExpectedAddress")) { throw "Ortet $Engine did not settle at $ExpectedAddress." }
    return $output
}

function Assert-CompletionPixels {
    param(
        [string]$Png,
        [string]$ExpectedColor,
        [string]$RecordPath
    )

    Add-Type -AssemblyName System.Drawing
    $bitmap = [System.Drawing.Bitmap]::FromFile($Png)
    try {
        $point = [Math]::Max(0, $bitmap.Width - 8), [Math]::Max(0, $bitmap.Height - 8)
        $actual = $bitmap.GetPixel($point[0], $point[1])
        $expected = [System.Drawing.ColorTranslator]::FromHtml($ExpectedColor)
        [pscustomobject]@{
            x = $point[0]
            y = $point[1]
            expected = ('#{0:X2}{1:X2}{2:X2}' -f $expected.R, $expected.G, $expected.B)
            actual = ('#{0:X2}{1:X2}{2:X2}{3:X2}' -f $actual.R, $actual.G, $actual.B, $actual.A)
            width = $bitmap.Width
            height = $bitmap.Height
        } | ConvertTo-Json | Set-Content $RecordPath
        if ($actual.R -ne $expected.R -or $actual.G -ne $expected.G -or $actual.B -ne $expected.B) {
            throw "receipt pixel at ($($point[0]),$($point[1])) was $($actual.R),$($actual.G),$($actual.B), expected $($expected.R),$($expected.G),$($expected.B)"
        }
    }
    finally {
        $bitmap.Dispose()
    }
}

function Assert-EventOrder {
    param(
        [object[]]$Events,
        [string[]]$Expected
    )

    $previous = -1
    foreach ($event in $Expected) {
        $index = [Array]::FindIndex($Events, [Predicate[object]]{ param($row) $row.event -eq $event })
        if ($index -lt 0) { throw "server event '$event' was absent." }
        if ($index -le $previous) { throw "server event '$event' did not follow the preceding expected event." }
        $previous = $index
    }
}

function Reset-ReceiptCase {
    param([string]$Origin, [string]$Case)
    Invoke-WebRequest -UseBasicParsing "$Origin/__reset?case=$Case" | Out-Null
}

function Copy-ReceiptEvents {
    param([string]$Origin, [string]$Case, [string]$RecordPath)
    $response = Invoke-WebRequest -UseBasicParsing "$Origin/__events?case=$Case"
    Set-Content -Path $RecordPath -Value $response.Content
    return @($response.Content | ConvertFrom-Json)
}

Push-Location $repo
try {
    $env:CARGO_TARGET_DIR = $target
    $env:GENET_SOURCE_REVISION = (git rev-parse HEAD).Trim()
    cargo build -p ortet --features scripted-nova --offline
    if ($LASTEXITCODE -ne 0) { throw 'native scripted Ortet build failed.' }
    $exe = Join-Path $target 'debug\ortet.exe'
    if (-not (Test-Path $exe)) { throw "Ortet binary was not produced at $exe." }

    $server = Start-Process -FilePath 'node' -ArgumentList @(
        $serverScript, '--artifact', $artifact, '--port', $HttpPort
    ) -PassThru -WindowStyle Hidden
    try {
        Start-Sleep -Milliseconds 400
        $origin = "http://127.0.0.1:$HttpPort"
        $cases = @(
            [pscustomobject]@{
                Name = 'fetch'
                Url = "$origin/fetch.html?case=fetch"
                Heading = 'Ortet O5 fetch idle wake complete'
                Color = '#185d8c'
                Wake = 'fetch'
                WakeResource = '/delayed-fetch?case=fetch'
                Actions = $null
                Events = @('fetch.html served', 'delayed-fetch requested', 'delayed-fetch released')
            },
            [pscustomobject]@{
                Name = 'worker'
                Url = "$origin/worker.html?case=worker"
                Heading = 'Ortet O5 worker idle wake complete'
                Color = '#5b3b78'
                Wake = 'worker'
                WakeResource = '/worker-gate?case=worker'
                Actions = $null
                Events = @('worker.html served', 'worker.js served', 'worker-gate requested', 'worker-gate released')
            },
            [pscustomobject]@{
                Name = 'stale'
                Url = "$origin/stale.html?case=stale"
                Heading = 'Ortet O5 stale completion rejected'
                Color = '#17324d'
                Wake = $null
                WakeResource = $null
                Actions = 'click:100,120'
                Events = @('stale.html served', 'replacement.html served', 'late-stale released')
            }
        )

        foreach ($engine in @('boa', 'nova')) {
            $engineArtifact = Join-Path $artifact $engine
            New-Item -ItemType Directory -Path $engineArtifact -Force | Out-Null
            $png = Join-Path $engineArtifact 'receipt.png'
            $log = Join-Path $engineArtifact 'receipt.log'
            & $exe --url $fixture --engine $engine --size 640x400 --frames 6 --artifact $png --actions 'click:160,74' --expect-heading $completion 2>&1 | Tee-Object -FilePath $log
            if ($LASTEXITCODE -ne 0) { throw "Ortet $engine static receipt failed; see $log." }
            $staticAddress = ([Uri]::new($fixture)).AbsoluteUri
            $staticOutput = Assert-ReceiptLog -Log $log -Engine $engine -Heading $completion -SourceRevision $env:GENET_SOURCE_REVISION -ExpectedAddress $staticAddress
            if (-not (Test-Path $png)) { throw "Ortet $engine static receipt did not write its PNG." }
            (Get-FileHash -Algorithm SHA256 $png).Hash.ToLowerInvariant() + '  receipt.png' | Set-Content (Join-Path $engineArtifact 'receipt.sha256')
            Assert-CompletionPixels -Png $png -ExpectedColor '#d09b5b' -RecordPath (Join-Path $engineArtifact 'receipt-pixels.json')

            $timeoutArtifact = Join-Path $engineArtifact 'timeout'
            New-Item -ItemType Directory -Path $timeoutArtifact -Force | Out-Null
            $timeoutLog = Join-Path $timeoutArtifact 'receipt.log'
            $timeoutPng = Join-Path $timeoutArtifact 'receipt.png'
            & $exe --url $fixture --engine $engine --size 640x400 --artifact $timeoutPng --expect-heading 'Ortet impossible timeout heading' --timeout-ms 25 2>&1 | Tee-Object -FilePath $timeoutLog
            if ($LASTEXITCODE -eq 0) { throw "Ortet $engine timeout receipt unexpectedly succeeded." }
            $timeoutOutput = Get-Content $timeoutLog -Raw
            if ($timeoutOutput -notmatch [regex]::Escape('was absent before the 25ms deadline')) {
                throw "Ortet $engine timeout receipt did not report its bounded deadline."
            }

            foreach ($case in $cases) {
                $caseArtifact = Join-Path $engineArtifact $case.Name
                New-Item -ItemType Directory -Path $caseArtifact -Force | Out-Null
                $png = Join-Path $caseArtifact 'receipt.png'
                $log = Join-Path $caseArtifact 'receipt.log'
                Reset-ReceiptCase -Origin $origin -Case $case.Name
                $arguments = @('--url', $case.Url, '--engine', $engine, '--size', '640x400', '--artifact', $png, '--expect-heading', $case.Heading, '--timeout-ms', $ReceiptTimeoutMs)
                if ($case.Actions) { $arguments += @('--actions', $case.Actions) }
                & $exe @arguments 2>&1 | Tee-Object -FilePath $log
                if ($LASTEXITCODE -ne 0) { throw "Ortet $engine $($case.Name) receipt failed; see $log." }
                $expectedAddress = if ($case.Name -eq 'stale') { "$origin/replacement.html?case=stale" } else { $case.Url }
                $output = Assert-ReceiptLog -Log $log -Engine $engine -Heading $case.Heading -SourceRevision $env:GENET_SOURCE_REVISION -ExpectedAddress $expectedAddress
                if (-not (Test-Path $png)) { throw "Ortet $engine $($case.Name) receipt did not write its PNG." }
                (Get-FileHash -Algorithm SHA256 $png).Hash.ToLowerInvariant() + '  receipt.png' | Set-Content (Join-Path $caseArtifact 'receipt.sha256')
                Assert-CompletionPixels -Png $png -ExpectedColor $case.Color -RecordPath (Join-Path $caseArtifact 'receipt-pixels.json')
                $events = Copy-ReceiptEvents -Origin $origin -Case $case.Name -RecordPath (Join-Path $caseArtifact 'server-events.json')
                Assert-EventOrder -Events $events -Expected $case.Events

                if ($case.Wake) {
                    $idle = [regex]::Match($output, 'ortet: receipt idle generation=(?<generation>\d+) timer=none microtasks=false external=true')
                    $wakePattern = "ortet: receipt wake source=$($case.Wake) generation=(?<generation>\d+) resource=\S*" + [regex]::Escape($case.WakeResource)
                    $wake = [regex]::Match($output, $wakePattern)
                    if (-not $idle.Success -or -not $wake.Success) { throw "Ortet $engine $($case.Name) did not prove an external idle wake." }
                    if ($idle.Index -ge $wake.Index) { throw "Ortet $engine $($case.Name) woke before the host became externally idle." }
                    if ($idle.Groups['generation'].Value -ne $wake.Groups['generation'].Value) { throw "Ortet $engine $($case.Name) wake belonged to a different session generation." }
                }
                if ($case.Name -eq 'stale') {
                    if (-not ($events | Where-Object event -eq 'late-stale requested')) {
                        throw "Ortet $engine stale receipt did not reach the delayed old-session resource."
                    }
                    $drop = [regex]::Match($output, 'ortet: receipt stale-completion dropped source=fetch generation=(?<old>\d+) active=(?<active>\d+)')
                    if (-not $drop.Success) { throw "Ortet $engine stale receipt did not report a rejected stale completion." }
                    if ($drop.Groups['old'].Value -eq $drop.Groups['active'].Value) { throw "Ortet $engine stale receipt did not advance the active generation." }
                }
            }
        }
    }
    finally {
        if ($server) { Stop-Process -Id $server.Id -ErrorAction SilentlyContinue }
    }
}
finally {
    Pop-Location
}
