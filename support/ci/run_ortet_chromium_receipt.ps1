# Copyright 2026 Mark Alan Boykin
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
# SPDX-License-Identifier: MPL-2.0

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$ArtifactDir,
    [string]$ChromePath = 'C:\Program Files\Google\Chrome\Application\chrome.exe',
    [ValidateRange(1, 65535)]
    [int]$HttpPort = 18743,
    [ValidateRange(1, 65535)]
    [int]$DebugPort = 18744
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$artifact = [IO.Path]::GetFullPath($ArtifactDir)
$target = Join-Path $artifact 'target'

if (-not (Test-Path $ChromePath)) { throw "Chrome was not found at $ChromePath." }
New-Item -ItemType Directory -Path $artifact -Force | Out-Null
Copy-Item (Join-Path $repo 'ports\ortet\tests\web\receipt.html') (Join-Path $artifact 'receipt.html') -Force
Copy-Item (Join-Path $repo 'tests\wpt\tests\fonts\Ahem.ttf') (Join-Path $artifact 'Ahem.ttf') -Force

Push-Location $repo
try {
    $env:CARGO_TARGET_DIR = $target
    cargo build -p ortet --lib --target wasm32-unknown-unknown --locked --offline
    wasm-bindgen (Join-Path $target 'wasm32-unknown-unknown\debug\ortet.wasm') --target web --out-dir $artifact --no-typescript

    $python = (& python -c 'import sys; print(sys.executable)').Trim()
    if (-not $python) { throw 'Python could not report its executable path.' }
    $server = Start-Process -FilePath $python -ArgumentList @('-m', 'http.server', $HttpPort, '--directory', $artifact) -PassThru -WindowStyle Hidden
    try {
        Start-Sleep -Milliseconds 750
        $stderr = Join-Path $artifact 'chromium-receipt.stderr.log'
        $profile = Join-Path $artifact "chrome-profile-$PID"
        $chrome = Start-Process -FilePath $ChromePath -ArgumentList @(
            '--headless=new', '--disable-gpu-sandbox', "--user-data-dir=$profile",
            "--remote-debugging-port=$DebugPort", 'about:blank'
        ) -RedirectStandardError $stderr -PassThru -WindowStyle Hidden
        & node (Join-Path $PSScriptRoot 'ortet_chromium_readback.mjs') --debug-port $DebugPort --url "http://127.0.0.1:$HttpPort/receipt.html" --artifact $artifact
        if ($LASTEXITCODE -ne 0) { throw "Chromium readback failed; see $stderr." }
    }
    finally {
        if ($chrome) { Stop-Process -Id $chrome.Id -ErrorAction SilentlyContinue }
        Stop-Process -Id $server.Id -ErrorAction SilentlyContinue
    }
}
finally {
    Pop-Location
}
