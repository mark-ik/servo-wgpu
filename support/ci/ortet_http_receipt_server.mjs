// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

import { readFileSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { join } from "node:path";

const values = new Map();
for (let index = 2; index < process.argv.length; index += 2) values.set(process.argv[index], process.argv[index + 1]);
const artifact = values.get("--artifact");
const port = Number(values.get("--port"));
if (!artifact || !Number.isInteger(port)) throw new Error("usage: --artifact DIR --port PORT");

const requests = [];
const image = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAFUlEQVR4nGN87mb0n4GBgYEJRIAwACTNAmJ9gJg8AAAAAElFTkSuQmCC",
  "base64",
);
const ahem = () => readFileSync(join(artifact, "Ahem.ttf"));
const record = (path, status) => {
  requests.push({ path, status });
  writeFileSync(join(artifact, "http-receipt-requests.json"), JSON.stringify(requests, null, 2));
};
const text = (response, status, body, contentType) => {
  response.writeHead(status, { "content-type": contentType, "cache-control": "no-store" });
  response.end(body);
};
const staticTypes = {
  "/http_receipt.html": "text/html; charset=utf-8",
  "/ortet.js": "text/javascript; charset=utf-8",
  "/ortet_bg.wasm": "application/wasm",
};

const server = createServer((request, response) => {
  const path = new URL(request.url ?? "/", "http://127.0.0.1").pathname;
  if (staticTypes[path]) {
    const bytes = readFileSync(join(artifact, path.slice(1)));
    record(path, 200);
    response.writeHead(200, { "content-type": staticTypes[path], "content-length": bytes.length, "cache-control": "no-store" });
    response.end(bytes);
    return;
  }
  if (path === "/start") {
    record(path, 302);
    response.writeHead(302, { location: "/site/page/index.html", "cache-control": "no-store" });
    response.end();
    return;
  }
  if (path === "/__stats") {
    record(path, 200);
    text(response, 200, JSON.stringify({ requests }), "application/json");
    return;
  }
  if (path === "/site/page/index.html") {
    record(path, 200);
    text(response, 200, `<!doctype html>
<html><head><meta charset="utf-8"><title>Ortet HTTP resource receipt</title>
<link rel="stylesheet" href="styles/base.css"></head>
<body><main class="page"><h1>HTTP resource receipt</h1>
<p class="accent">The imported stylesheet and authored font are live.</p>
<img class="proof-image" src="images/proof.png" alt="served image">
<img class="denied-image" src="images/denied.png" alt="unavailable image">
<div class="font-proof">MMMMM</div></main></body></html>`, "text/html; charset=utf-8");
    return;
  }
  if (path === "/site/page/styles/base.css") {
    record(path, 200);
    text(response, 200, `@import "theme/colors.css";
@font-face { font-family: ReceiptAhem; src: url("../fonts/Ahem.ttf") format("truetype"); }
body { margin: 0; background: #f9f4e8; color: #17324d; font: 18px ReceiptAhem; }
.page { margin: 30px; padding: 18px; background: #d8e6dc; border: 5px solid #5b3b78; }
.proof-image { display: block; width: 72px; height: 52px; margin: 12px 0; }
.denied-image { width: 20px; height: 20px; }`, "text/css; charset=utf-8");
    return;
  }
  if (path === "/site/page/styles/theme/colors.css") {
    record(path, 200);
    text(response, 200, `@import "../shared/accent.css";
.page { background: #d8e6dc; }`, "text/css; charset=utf-8");
    return;
  }
  if (path === "/site/page/styles/shared/accent.css") {
    record(path, 200);
    text(response, 200, ".accent { color: #185d8c; }", "text/css; charset=utf-8");
    return;
  }
  if (path === "/site/page/images/proof.png") {
    record(path, 200);
    response.writeHead(200, { "content-type": "image/png", "content-length": image.length, "cache-control": "no-store" });
    response.end(image);
    return;
  }
  if (path === "/site/page/images/denied.png") {
    record(path, 403);
    text(response, 403, "denied by receipt fixture", "text/plain; charset=utf-8");
    return;
  }
  if (path === "/site/page/fonts/Ahem.ttf") {
    const bytes = ahem();
    record(path, 200);
    response.writeHead(200, { "content-type": "font/ttf", "content-length": bytes.length, "cache-control": "no-store" });
    response.end(bytes);
    return;
  }
  record(path, 404);
  text(response, 404, "not found", "text/plain; charset=utf-8");
});

server.listen(port, "127.0.0.1");
