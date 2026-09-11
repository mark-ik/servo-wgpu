# Copyright 2026 Mark Alan Boykin
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
# SPDX-License-Identifier: MPL-2.0

# The G5 arena semantic-contract headed receipt: retain a node from JS,
# detach it, force collection pressure, adopt/reattach it, mutate it, render
# it, then release it and confirm the engine's own JS wrapper for it is
# collected. Reuses the exact-source Boa/Nova build and the receipt
# conventions of run_ortet_scripted_native_receipt.ps1 (log/PNG/SHA-256/
# completion-pixel), against the dedicated fixture
# ports/ortet/tests/native/g5_arena_receipt.html instead of a new harness.

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$ArtifactDir,
    [string]$TargetDir,
    [ValidateRange(1, 32)]
    [int]$BuildJobs = 2,
    [ValidateRange(1, 120000)]
    [int]$ReceiptTimeoutMs = 8000,
    [ValidateRange(1, 5000)]
    [int]$FailureTimeoutMs = 25
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$artifact = [IO.Path]::GetFullPath($ArtifactDir)
$target = if ($TargetDir) { [IO.Path]::GetFullPath($TargetDir) } else { Join-Path $artifact 'target' }
$fixture = Join-Path $repo 'ports\ortet\tests\native\g5_arena_receipt.html'
$completion = 'Ortet G5 arena sequence complete'
$impossible = 'Ortet impossible G5 heading'
New-Item -ItemType Directory -Path $artifact -Force | Out-Null

# The original G5 run reported HEAD while building an uncommitted tree and
# a dirty Boa fork. Require clean source repositories and record local path
# dependency revisions as well as Cargo's resolved graph for every replay.
function Get-SourceIdentity {
    $metadataText = cargo metadata --format-version 1 --locked --offline
    if ($LASTEXITCODE -ne 0) { throw 'Cannot resolve the locked receipt dependency graph.' }
    $metadata = ($metadataText -join "`n") | ConvertFrom-Json
    $metadataText | Set-Content (Join-Path $artifact 'cargo-metadata.json')
    $roots = @($repo)
    foreach ($package in $metadata.packages) {
        if ($null -ne $package.source) { continue }
        $directory = Split-Path $package.manifest_path -Parent
        $root = git -C $directory rev-parse --show-toplevel 2>$null
        if ($LASTEXITCODE -ne 0) { throw "Local package has no source revision: $directory" }
        $roots += ($root -join '').Trim()
    }
    $identities = foreach ($root in ($roots | Sort-Object -Unique)) {
        $dirty = @(git -C $root status --porcelain --untracked-files=normal)
        if ($LASTEXITCODE -ne 0 -or $dirty.Count) {
            throw "Receipt requires a clean source checkout: $root"
        }
        [pscustomobject]@{
            path = $root
            revision = (git -C $root rev-parse HEAD).Trim()
        }
    }
    $config = Join-Path $repo '.cargo/config.toml'
    [pscustomobject]@{
        repositories = @($identities)
        lock_sha256 = (Get-FileHash (Join-Path $repo 'Cargo.lock') -Algorithm SHA256).Hash
        config_sha256 = if (Test-Path $config) { (Get-FileHash $config -Algorithm SHA256).Hash } else { $null }
        rustc = (rustc -Vv) -join "`n"
    } | ConvertTo-Json -Depth 5 -Compress
}

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

Push-Location $repo
try {
    $env:CARGO_TARGET_DIR = $target
    $before = Get-SourceIdentity
    $before | Set-Content (Join-Path $artifact 'source-identity.json')
    $env:GENET_SOURCE_REVISION = (git rev-parse HEAD).Trim()
    cargo build -p ortet --features scripted-nova --locked --offline -j $BuildJobs
    if ($LASTEXITCODE -ne 0) { throw 'native scripted Ortet build failed.' }
    $exe = Join-Path $target 'debug\ortet.exe'
    if (-not (Test-Path $exe)) { throw "Ortet binary was not produced at $exe." }
    $staticAddress = ([Uri]::new($fixture)).AbsoluteUri
    $anyRefused = $false

    foreach ($engine in @('boa', 'nova')) {
        $engineArtifact = Join-Path $artifact $engine
        New-Item -ItemType Directory -Path $engineArtifact -Force | Out-Null

        # Positive case: the full retain/detach/collect/adopt/mutate/render/
        # release/confirm sequence, driven by one native click.
        $png = Join-Path $engineArtifact 'receipt.png'
        $log = Join-Path $engineArtifact 'receipt.log'
        & $exe --url $fixture --engine $engine --size 640x400 --artifact $png --actions 'click:160,74' --expect-heading $completion --timeout-ms $ReceiptTimeoutMs 2>&1 | Tee-Object -FilePath $log
        $exitCode = $LASTEXITCODE
        $output = Get-Content $log -Raw
        if ($exitCode -ne 0) {
            # A per-engine refusal is a legitimate, recorded outcome for that
            # engine rather than a harness bug. It still fails the receipt for
            # that engine; it is not papered over.
            "exit $exitCode" | Set-Content (Join-Path $engineArtifact 'receipt.refused')
            Write-Warning "Ortet $engine G5 sequence did not complete; see $log and $(Join-Path $engineArtifact 'receipt.refused')."
            $anyRefused = $true
            continue
        }
        $null = Assert-ReceiptLog -Log $log -Engine $engine -Heading $completion -SourceRevision $env:GENET_SOURCE_REVISION -ExpectedAddress $staticAddress
        if (-not (Test-Path $png)) { throw "Ortet $engine G5 receipt did not write its PNG." }
        (Get-FileHash -Algorithm SHA256 $png).Hash.ToLowerInvariant() + '  receipt.png' | Set-Content (Join-Path $engineArtifact 'receipt.sha256')
        Assert-CompletionPixels -Png $png -ExpectedColor '#2f6b3c' -RecordPath (Join-Path $engineArtifact 'receipt-pixels.json')

        # Confirm collected: the production `Runtime::collect_garbage`
        # accounting (the same mechanism the S1/S2/S5 arena receipts already
        # treat as authoritative) must show the released subtree actually
        # reaped, not merely detached. `victim` and `victim-child` are the
        # floor; text nodes usually bring the true count higher.
        $stats = [regex]::Match($output, 'ortet: receipt collection unpinned=(?<unpinned>\d+) collected=(?<collected>\d+)')
        if (-not $stats.Success) { throw "Ortet $engine G5 receipt did not report collection stats." }
        $unpinned = [int]$stats.Groups['unpinned'].Value
        $collected = [int]$stats.Groups['collected'].Value
        [pscustomobject]@{ unpinned = $unpinned; collected = $collected } |
            ConvertTo-Json | Set-Content (Join-Path $engineArtifact 'receipt-collection.json')
        if ($unpinned -lt 2 -or $collected -lt 2) {
            throw "Ortet $engine G5 receipt did not confirm collection: unpinned=$unpinned collected=$collected (expected >= 2 each)."
        }

        # Deliberate failing control: an unmet completion condition under a
        # short deadline must fail the run rather than silently succeed.
        $failArtifact = Join-Path $engineArtifact 'timeout'
        New-Item -ItemType Directory -Path $failArtifact -Force | Out-Null
        $failLog = Join-Path $failArtifact 'receipt.log'
        $failPng = Join-Path $failArtifact 'receipt.png'
        & $exe --url $fixture --engine $engine --size 640x400 --artifact $failPng --expect-heading $impossible --timeout-ms $FailureTimeoutMs 2>&1 | Tee-Object -FilePath $failLog
        if ($LASTEXITCODE -eq 0) { throw "Ortet $engine G5 deliberate-failure control unexpectedly succeeded." }
        $failOutput = Get-Content $failLog -Raw
        if ($failOutput -notmatch [regex]::Escape("was absent before the ${FailureTimeoutMs}ms deadline")) {
            throw "Ortet $engine G5 failure control did not report its bounded deadline."
        }
        # The deliberate control's expected nonzero $LASTEXITCODE must not
        # leak out as this script's own exit code once it has been checked.
        $global:LASTEXITCODE = 0
    }
    if ($anyRefused) { throw 'At least one engine did not complete the G5 sequence; see its receipt.refused.' }
    $after = Get-SourceIdentity
    $after | Set-Content (Join-Path $artifact 'source-identity-after.json')
    if ($before -ne $after) { throw 'Source or dependency state changed during the G5 receipt.' }
}
finally {
    Pop-Location
}
exit 0
