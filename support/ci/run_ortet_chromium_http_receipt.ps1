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
    [int]$HttpPort = 18753,
    [ValidateRange(1, 65535)]
    [int]$DebugPort = 18754
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$artifact = [IO.Path]::GetFullPath($ArtifactDir)
$target = Join-Path $artifact 'target'
if (-not (Test-Path $ChromePath)) { throw "Chrome was not found at $ChromePath." }
New-Item -ItemType Directory -Path $artifact -Force | Out-Null
Copy-Item (Join-Path $repo 'ports\ortet\tests\web\http_receipt.html') (Join-Path $artifact 'http_receipt.html') -Force
Copy-Item (Join-Path $repo 'tests\wpt\tests\fonts\Ahem.ttf') (Join-Path $artifact 'Ahem.ttf') -Force

Push-Location $repo
try {
    $env:CARGO_TARGET_DIR = $target
    # This repository deliberately does not track Cargo.lock.  Offline keeps
    # the receipt reproducible against the caller's prepared dependency cache.
    cargo build -p ortet --lib --target wasm32-unknown-unknown --offline
    if ($LASTEXITCODE -ne 0) { throw 'wasm build failed.' }
    wasm-bindgen (Join-Path $target 'wasm32-unknown-unknown\debug\ortet.wasm') --target web --out-dir $artifact --no-typescript
    if ($LASTEXITCODE -ne 0) { throw 'wasm-bindgen failed.' }

    $server = Start-Process -FilePath 'node' -ArgumentList @(
        (Join-Path $repo 'support\ci\ortet_http_receipt_server.mjs'), '--artifact', $artifact, '--port', $HttpPort
    ) -PassThru -WindowStyle Hidden
    try {
        Start-Sleep -Milliseconds 400
        $stderr = Join-Path $artifact 'chromium-http-receipt.stderr.log'
        $profile = Join-Path $artifact "chrome-http-profile-$PID"
        $chrome = Start-Process -FilePath $ChromePath -ArgumentList @(
            '--headless=new', '--disable-gpu-sandbox', "--user-data-dir=$profile",
            "--remote-debugging-port=$DebugPort", 'about:blank'
        ) -RedirectStandardError $stderr -PassThru -WindowStyle Hidden
        try {
            & node (Join-Path $PSScriptRoot 'ortet_chromium_readback.mjs') --debug-port $DebugPort --url "http://127.0.0.1:$HttpPort/http_receipt.html" --artifact $artifact
            if ($LASTEXITCODE -ne 0) { throw "Chromium readback failed; see $stderr." }
        }
        finally {
            if ($chrome) { Stop-Process -Id $chrome.Id -ErrorAction SilentlyContinue }
        }
    }
    finally {
        if ($server) { Stop-Process -Id $server.Id -ErrorAction SilentlyContinue }
    }
}
finally {
    Pop-Location
}
