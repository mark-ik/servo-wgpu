# Copyright 2026 Mark Alan Boykin
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
# SPDX-License-Identifier: MPL-2.0

[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$ArtifactDir,
    [string]$TargetDir,
    [ValidateRange(1, 32)][int]$BuildJobs = 2,
    [ValidateRange(1, 120000)][int]$ReceiptTimeoutMs = 8000
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$artifact = [IO.Path]::GetFullPath($ArtifactDir)
$target = if ($TargetDir) { [IO.Path]::GetFullPath($TargetDir) } else { Join-Path $artifact 'target' }
$fixture = Join-Path $repo 'ports\ortet\tests\native\standards_compositing_receipt.html'
$completion = 'Ortet standards compositing receipt complete'
New-Item -ItemType Directory -Path $artifact -Force | Out-Null

function Source-Identity {
    $metadataText = cargo metadata --format-version 1 --locked --offline
    if ($LASTEXITCODE -ne 0) { throw 'Cannot resolve locked graph.' }
    $metadataText | Set-Content (Join-Path $artifact 'cargo-metadata.json')
    $metadata = ($metadataText -join [Environment]::NewLine) | ConvertFrom-Json
    $roots = @($repo)
    foreach ($package in $metadata.packages) {
        if ($null -ne $package.source) { continue }
        $directory = Split-Path $package.manifest_path -Parent
        $root = (git -C $directory rev-parse --show-toplevel 2>$null).Trim()
        if ($LASTEXITCODE -ne 0) { throw "Local package has no source revision: $directory" }
        $roots += $root
    }
    $identities = foreach ($root in ($roots | Sort-Object -Unique)) {
        $dirty = @(git -C $root status --porcelain --untracked-files=normal)
        if ($LASTEXITCODE -ne 0 -or $dirty.Count) { throw "Receipt requires clean source: $root" }
        [pscustomobject]@{ path = $root; revision = (git -C $root rev-parse HEAD).Trim() }
    }
    [pscustomobject]@{
        repositories = @($identities)
        lock_sha256 = (Get-FileHash (Join-Path $repo 'Cargo.lock') -Algorithm SHA256).Hash
        config_sha256 = if (Test-Path (Join-Path $repo '.cargo/config.toml')) { (Get-FileHash (Join-Path $repo '.cargo/config.toml') -Algorithm SHA256).Hash } else { $null }
        rustc = (rustc -Vv) -join [Environment]::NewLine
    } | ConvertTo-Json -Depth 5 -Compress
}

function Assert-Log([string]$log, [string]$engine, [string]$revision, [string]$address) {
    $text = Get-Content $log -Raw
    $id = if ($engine -eq 'nova') { 'genet.scripted.nova' } else { 'genet.scripted' }
    foreach ($needle in @("engine $id backend $engine presented", "semantic heading `"$completion`"", "source $revision", "settled at $address")) {
        if ($text -notmatch [regex]::Escape($needle)) { throw "$engine log lacks '$needle'" }
    }
    if ($text -notmatch 'ortet: display scale=(?<scale>[0-9]+(?:\.[0-9]+)?) physical=(?<width>[0-9]+)x(?<height>[0-9]+) logical=(?<lw>[0-9]+)x(?<lh>[0-9]+)') { throw "$engine log lacks display telemetry" }
    return $text
}

function Assert-Pixels([string]$png, [string]$log, [string]$record) {
    Add-Type -AssemblyName System.Drawing
    $bitmap = [Drawing.Bitmap]::FromFile($png)
    try {
        $m = [regex]::Match((Get-Content $log -Raw), 'ortet: display scale=(?<scale>[0-9]+(?:\.[0-9]+)?) physical=(?<width>[0-9]+)x(?<height>[0-9]+) logical=(?<lw>[0-9]+)x(?<lh>[0-9]+)')
        $scale = [double]::Parse($m.Groups['scale'].Value, [Globalization.CultureInfo]::InvariantCulture)
        if ($bitmap.Width -ne [int]$m.Groups['width'].Value -or $bitmap.Height -ne [int]$m.Groups['height'].Value) { throw 'Capture differs from native telemetry' }
        if ([Math]::Abs((300.5 * $scale) - [Math]::Round(300.5 * $scale)) -gt 0.01) { throw 'Sharp edge is not aligned for measured native scale' }
        $samples = @(
            @{ name='source-over'; logical=@(12,12); color=@(128,255,128) },
            @{ name='dom-over-canvas'; logical=@(20,20); color=@(64,128,192) },
            @{ name='canvas-opacity'; logical=@(20,60); color=@(192,128,192) },
            @{ name='group-red'; logical=@(75,13); color=@(255,128,128) },
            @{ name='group-overlap'; logical=@(100,25); color=@(128,128,255) },
            @{ name='clip-inside'; logical=@(220,30); color=@(255,255,0) },
            @{ name='clip-outside'; logical=@(240,30); color=@(255,255,255) },
            @{ name='nested-inside'; logical=@(230,155); color=@(255,0,255) },
            @{ name='nested-inner-clip'; logical=@(260,155); color=@(255,255,255) },
            @{ name='nested-outer-clip'; logical=@(295,155); color=@(255,255,255) },
            @{ name='canvas-border'; logical=@(10,115); color=@(20,20,20) },
            @{ name='canvas-padding'; logical=@(14,120); color=@(255,255,255) },
            @{ name='canvas-content'; logical=@(25,130); color=@(255,0,255) },
            @{ name='sharp-edge-before'; logical=@(300.25,125); color=@(255,255,255) },
            @{ name='sharp-edge-after'; logical=@(300.75,125); color=@(15,15,15) }
        )
        $rows = foreach ($s in $samples) {
            $x=[int][Math]::Floor($s.logical[0]*$scale); $y=[int][Math]::Floor($s.logical[1]*$scale)
            $p=$bitmap.GetPixel($x,$y); $channels=@($p.R,$p.G,$p.B); $d=0..2 | ForEach-Object { [Math]::Abs($channels[$_]-$s.color[$_]) } | Measure-Object -Maximum | Select-Object -ExpandProperty Maximum
            if ($d -gt 2 -or $p.A -ne 255) { throw "$($s.name) at ($x,$y) was $($p.R),$($p.G),$($p.B),$($p.A), expected $($s.color -join ',')" }
            [pscustomobject]@{ name=$s.name; x=$x; y=$y; expected=($s.color -join ','); actual="$($p.R),$($p.G),$($p.B),$($p.A)" }
        }
        [pscustomobject]@{ scale=$scale; physical="$($bitmap.Width)x$($bitmap.Height)"; samples=@($rows) } | ConvertTo-Json -Depth 5 | Set-Content $record
    } finally { $bitmap.Dispose() }
}

Push-Location $repo
try {
    $env:CARGO_TARGET_DIR=$target
    $before=Source-Identity; $before | Set-Content (Join-Path $artifact 'source-identity.json')
    $env:GENET_SOURCE_REVISION=(git rev-parse HEAD).Trim(); $address=([Uri]::new($fixture)).AbsoluteUri
    cargo build -p ortet --features scripted-nova --locked --offline -j $BuildJobs
    if ($LASTEXITCODE -ne 0) { throw 'native Ortet build failed' }
    $exe=Join-Path $target 'debug\ortet.exe'; if (!(Test-Path $exe)) { throw 'ortet.exe missing' }
    foreach($engine in @('boa','nova')) {
        $dir=Join-Path $artifact $engine; New-Item -ItemType Directory -Force $dir | Out-Null
        $log=Join-Path $dir 'receipt.log'; $png=Join-Path $dir 'receipt.png'
        & $exe --url $fixture --engine $engine --size 640x400 --artifact $png --expect-heading $completion --timeout-ms $ReceiptTimeoutMs 2>&1 | Tee-Object $log
        if($LASTEXITCODE -ne 0){throw "$engine receipt failed"}; Assert-Log $log $engine $env:GENET_SOURCE_REVISION $address | Out-Null
        Assert-Pixels $png $log (Join-Path $dir 'receipt-pixels.json')
    }
    $after=Source-Identity; $after | Set-Content (Join-Path $artifact 'source-identity-after.json')
    if ((($before|ConvertFrom-Json)|ConvertTo-Json -Depth 5 -Compress) -ne (($after|ConvertFrom-Json)|ConvertTo-Json -Depth 5 -Compress)){throw 'source identity changed'}
} finally { Pop-Location }
