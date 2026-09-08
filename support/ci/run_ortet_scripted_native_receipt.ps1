# Copyright 2026 Mark Alan Boykin
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
# SPDX-License-Identifier: MPL-2.0

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$ArtifactDir
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$artifact = [IO.Path]::GetFullPath($ArtifactDir)
$target = Join-Path $artifact 'target'
$fixture = Join-Path $repo 'ports\ortet\tests\native\scripted_receipt.html'
$completion = 'Ortet O5 native receipt complete'
New-Item -ItemType Directory -Path $artifact -Force | Out-Null

Push-Location $repo
try {
    $env:CARGO_TARGET_DIR = $target
    $env:GENET_SOURCE_REVISION = (git rev-parse HEAD).Trim()
    cargo build -p ortet --features scripted-nova --offline
    if ($LASTEXITCODE -ne 0) { throw 'native scripted Ortet build failed.' }
    $exe = Join-Path $target 'debug\ortet.exe'
    if (-not (Test-Path $exe)) { throw "Ortet binary was not produced at $exe." }

    foreach ($engine in @('boa', 'nova')) {
        $engineArtifact = Join-Path $artifact $engine
        New-Item -ItemType Directory -Path $engineArtifact -Force | Out-Null
        $png = Join-Path $engineArtifact 'receipt.png'
        $log = Join-Path $engineArtifact 'receipt.log'
        & $exe --url $fixture --engine $engine --size 640x400 --frames 6 --artifact $png --actions 'click:160,74' --expect-heading $completion 2>&1 | Tee-Object -FilePath $log
        if ($LASTEXITCODE -ne 0) { throw "Ortet $engine receipt failed; see $log." }
        $output = Get-Content $log -Raw
        if ($output -notmatch "engine genet\.scripted.*backend $engine") { throw "Ortet $engine log did not report the selected backend." }
        if ($output -notmatch [regex]::Escape("semantic heading `"$completion`"")) { throw "Ortet $engine did not report its semantic completion." }
        if ($output -notmatch [regex]::Escape("source $env:GENET_SOURCE_REVISION")) { throw "Ortet $engine receipt lacks the exact source revision." }
        if (-not (Test-Path $png)) { throw "Ortet $engine receipt did not write its PNG." }
        (Get-FileHash -Algorithm SHA256 $png).Hash.ToLowerInvariant() + '  receipt.png' | Set-Content (Join-Path $engineArtifact 'receipt.sha256')
    }
}
finally {
    Pop-Location
}
