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

const fixtureRoot = join(process.cwd(), "ports", "ortet", "tests", "native", "live");
const pages = new Map([
  ["/fetch.html", "fetch.html"],
  ["/worker.html", "worker.html"],
  ["/worker.js", "worker.js"],
  ["/stale.html", "stale.html"],
  ["/replacement.html", "replacement.html"],
]);
const events = new Map();
let sequence = 0;
const receiptStart = Date.now();

function caseName(requestUrl) {
  const value = requestUrl.searchParams.get("case");
  if (!value || !/^(fetch|worker|stale)$/.test(value)) return null;
  return value;
}

function writeEvents(name) {
  writeFileSync(join(artifact, `scripted-native-${name}-events.json`), JSON.stringify(events.get(name) ?? [], null, 2));
}

function record(name, event) {
  const record = { sequence: ++sequence, event, elapsed_ms: Date.now() - receiptStart };
  const rows = events.get(name) ?? [];
  rows.push(record);
  events.set(name, rows);
  writeEvents(name);
}

function text(response, status, body, contentType) {
  response.writeHead(status, { "content-type": contentType, "cache-control": "no-store" });
  response.end(body);
}

function delayed(response, name, requested, released, body) {
  record(name, requested);
  setTimeout(() => {
    record(name, released);
    text(response, 200, body, "text/plain; charset=utf-8");
  }, 250);
}

const server = createServer((request, response) => {
  const requestUrl = new URL(request.url ?? "/", "http://127.0.0.1");
  const path = requestUrl.pathname;
  const name = caseName(requestUrl);

  if (path === "/__reset") {
    if (!name) return text(response, 400, "case is required", "text/plain; charset=utf-8");
    events.set(name, []);
    writeEvents(name);
    return text(response, 200, "reset", "text/plain; charset=utf-8");
  }
  if (path === "/__events") {
    if (!name) return text(response, 400, "case is required", "text/plain; charset=utf-8");
    return text(response, 200, JSON.stringify(events.get(name) ?? [], null, 2), "application/json; charset=utf-8");
  }
  if (pages.has(path)) {
    if (!name) return text(response, 400, "case is required", "text/plain; charset=utf-8");
    record(name, `${path.slice(1)} served`);
    const contentType = path.endsWith(".js") ? "text/javascript; charset=utf-8" : "text/html; charset=utf-8";
    return text(response, 200, readFileSync(join(fixtureRoot, pages.get(path))), contentType);
  }
  if (path === "/delayed-fetch" && name === "fetch") {
    return delayed(response, name, "delayed-fetch requested", "delayed-fetch released", "FETCH_OK");
  }
  if (path === "/worker-gate" && name === "worker") {
    return delayed(response, name, "worker-gate requested", "worker-gate released", "WORKER_GATE_OK");
  }
  if (path === "/late-stale" && name === "stale") {
    return delayed(response, name, "late-stale requested", "late-stale released", "STALE_OK");
  }
  if (name) record(name, `${path.slice(1) || "root"} 404`);
  return text(response, 404, "not found", "text/plain; charset=utf-8");
});

server.listen(port, "127.0.0.1");
