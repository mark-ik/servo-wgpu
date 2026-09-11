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
    [ValidateRange(1, 32)]
    [int]$BuildJobs = 2,
    [ValidateRange(1, 120000)]
    [int]$ReceiptTimeoutMs = 8000
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$artifact = [IO.Path]::GetFullPath($ArtifactDir)
$target = if ($TargetDir) { [IO.Path]::GetFullPath($TargetDir) } else { Join-Path $artifact 'target' }
$fixture = Join-Path $repo 'ports\ortet\tests\native\webgl_receipt.html'
$completion = 'Ortet WebGL receipt complete'
New-Item -ItemType Directory -Path $artifact -Force | Out-Null

function Get-SourceIdentity {
    $metadataText = cargo metadata --format-version 1 --locked --offline
    if ($LASTEXITCODE -ne 0) { throw 'Cannot resolve the locked receipt dependency graph.' }
    $metadataText | Set-Content (Join-Path $artifact 'cargo-metadata.json')
    $metadata = ($metadataText -join [Environment]::NewLine) | ConvertFrom-Json
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
        if ($LASTEXITCODE -ne 0 -or $dirty.Count) { throw "Receipt requires a clean source checkout: $root" }
        [pscustomobject]@{ path = $root; revision = (git -C $root rev-parse HEAD).Trim() }
    }
    [pscustomobject]@{
        repositories = @($identities)
        lock_sha256 = (Get-FileHash (Join-Path $repo 'Cargo.lock') -Algorithm SHA256).Hash
        config_sha256 = if (Test-Path (Join-Path $repo '.cargo/config.toml')) {
            (Get-FileHash (Join-Path $repo '.cargo/config.toml') -Algorithm SHA256).Hash
        } else { $null }
        rustc = (rustc -Vv) -join [Environment]::NewLine
    } | ConvertTo-Json -Depth 5 -Compress
}

function Assert-ReceiptLog {
    param([string]$Log, [string]$Engine, [string]$SourceRevision, [string]$ExpectedAddress)
    $output = Get-Content $Log -Raw
    $engineId = if ($Engine -eq 'nova') { 'genet.scripted.nova' } else { 'genet.scripted' }
    foreach ($needle in @(
        "engine $engineId backend $Engine presented",
        ('semantic heading "' + $completion + '"'),
        "source $SourceRevision",
        "settled at $ExpectedAddress"
    )) {
        if ($output -notmatch [regex]::Escape($needle)) { throw "Ortet $Engine log lacks '$needle'." }
    }
    return $output
}

function Assert-CompletionPixels {
    param([string]$Png, [string]$Log, [string]$RecordPath)
    Add-Type -AssemblyName System.Drawing
    $bitmap = [System.Drawing.Bitmap]::FromFile($Png)
    try {
        $scaleMatch = [regex]::Match((Get-Content $Log -Raw), 'ortet: display scale=(?<scale>[0-9]+(?:\.[0-9]+)?) physical=(?<width>[0-9]+)x(?<height>[0-9]+) logical=(?<logicalWidth>[0-9]+)x(?<logicalHeight>[0-9]+)')
        if (-not $scaleMatch.Success) { throw 'Receipt log has no native display scale telemetry.' }
        $scale = [double]::Parse($scaleMatch.Groups['scale'].Value, [Globalization.CultureInfo]::InvariantCulture)
        if ($scale -le 0 -or $bitmap.Width -ne [int]$scaleMatch.Groups['width'].Value -or $bitmap.Height -ne [int]$scaleMatch.Groups['height'].Value) {
            throw 'Receipt capture dimensions disagree with native surface telemetry.'
        }
        $samples = @(
            [pscustomobject]@{ name = 'body'; logical = @(300, 180); color = '#C43D3D' },
            [pscustomobject]@{ name = 'webgl'; logical = @(40, 130); color = '#2F6B3C' },
            [pscustomobject]@{ name = 'overlay'; logical = @(100, 100); color = '#2B6CB0' }
        )
        $records = foreach ($sample in $samples) {
            $x = [int][Math]::Round($sample.logical[0] * $scale)
            $y = [int][Math]::Round($sample.logical[1] * $scale)
            if ($x -ge $bitmap.Width -or $y -ge $bitmap.Height) { throw 'Display scale leaves receipt probes outside the window; increase the receipt window size.' }
            $actual = $bitmap.GetPixel($x, $y)
            $expected = [System.Drawing.ColorTranslator]::FromHtml($sample.color)
            if ($actual.R -ne $expected.R -or $actual.G -ne $expected.G -or $actual.B -ne $expected.B -or $actual.A -ne 255) {
                throw "receipt $($sample.name) pixel at ($x,$y) was $($actual.R),$($actual.G),$($actual.B),$($actual.A), expected $($sample.color),FF"
            }
            [pscustomobject]@{ name = $sample.name; logical_x = $sample.logical[0]; logical_y = $sample.logical[1]; x = $x; y = $y; expected = $sample.color; actual = ('#{0:X2}{1:X2}{2:X2}{3:X2}' -f $actual.R, $actual.G, $actual.B, $actual.A) }
        }
        [pscustomobject]@{ logical_width = 320; logical_height = 200; scale_factor = $scale; width = $bitmap.Width; height = $bitmap.Height; samples = @($records) } | ConvertTo-Json -Depth 5 | Set-Content $RecordPath
    } finally { $bitmap.Dispose() }
}

Push-Location $repo
try {
    $env:CARGO_TARGET_DIR = $target
    $before = Get-SourceIdentity
    $before | Set-Content (Join-Path $artifact 'source-identity.json')
    $env:GENET_SOURCE_REVISION = (git rev-parse HEAD).Trim()
    $expectedAddress = ([Uri]::new($fixture)).AbsoluteUri
    cargo build -p ortet --features scripted-nova --locked --offline -j $BuildJobs
    if ($LASTEXITCODE -ne 0) { throw 'native scripted Ortet build failed.' }
    $exe = Join-Path $target 'debug\ortet.exe'
    if (-not (Test-Path $exe)) { throw "Ortet binary was not produced at $exe." }
    foreach ($engine in @('boa', 'nova')) {
        $engineArtifact = Join-Path $artifact $engine
        New-Item -ItemType Directory -Path $engineArtifact -Force | Out-Null
        $png = Join-Path $engineArtifact 'receipt.png'
        $log = Join-Path $engineArtifact 'receipt.log'
        & $exe --url $fixture --engine $engine --size 640x400 --artifact $png --expect-heading $completion --timeout-ms $ReceiptTimeoutMs 2>&1 | Tee-Object -FilePath $log
        if ($LASTEXITCODE -ne 0) { throw "Ortet $engine WebGL receipt failed; see $log." }
        $null = Assert-ReceiptLog -Log $log -Engine $engine -SourceRevision $env:GENET_SOURCE_REVISION -ExpectedAddress $expectedAddress
        (Get-FileHash -Algorithm SHA256 $png).Hash.ToLowerInvariant() + '  receipt.png' | Set-Content (Join-Path $engineArtifact 'receipt.sha256')
        Assert-CompletionPixels -Png $png -Log $log -RecordPath (Join-Path $engineArtifact 'receipt-pixels.json')
        $timeoutArtifact = Join-Path $engineArtifact 'timeout'
        New-Item -ItemType Directory -Path $timeoutArtifact -Force | Out-Null
        $timeoutLog = Join-Path $timeoutArtifact 'receipt.log'
        $timeoutPng = Join-Path $timeoutArtifact 'receipt.png'
        & $exe --url $fixture --engine $engine --size 640x400 --artifact $timeoutPng --expect-heading 'Ortet impossible WebGL heading' --timeout-ms 25 2>&1 | Tee-Object -FilePath $timeoutLog
        if ($LASTEXITCODE -eq 0) { throw "Ortet $engine WebGL timeout unexpectedly succeeded." }
        if ((Get-Content $timeoutLog -Raw) -notmatch 'was absent before the 25ms deadline') {
            throw "Ortet $engine timeout did not report its bounded deadline."
        }
    }
    $after = Get-SourceIdentity
    $after | Set-Content (Join-Path $artifact 'source-identity-after.json')
    $beforeNormalized = (($before | ConvertFrom-Json) | ConvertTo-Json -Depth 5 -Compress)
    $afterNormalized = (($after | ConvertFrom-Json) | ConvertTo-Json -Depth 5 -Compress)
    if ($beforeNormalized -ne $afterNormalized) { throw 'Source identity changed during the WebGL receipt.' }
}
finally { Pop-Location }
