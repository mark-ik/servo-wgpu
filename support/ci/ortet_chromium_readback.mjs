// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

import { createHash } from "node:crypto";
import { mkdir, writeFile } from "node:fs/promises";

const values = new Map();
for (let index = 2; index < process.argv.length; index += 2) values.set(process.argv[index], process.argv[index + 1]);
const port = values.get("--debug-port");
const url = values.get("--url");
const artifact = values.get("--artifact");
if (!port || !url || !artifact) throw new Error("usage: --debug-port PORT --url URL --artifact DIR");

const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
let target;
for (let attempt = 0; attempt < 80; attempt += 1) {
  try {
    const response = await fetch(`http://127.0.0.1:${port}/json/new?${encodeURIComponent(url)}`, { method: "PUT" });
    if (response.ok) {
      target = await response.json();
      break;
    }
  } catch {}
  await delay(250);
}
if (!target) throw new Error("Chrome DevTools endpoint did not open a receipt tab");

const socket = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  socket.addEventListener("open", resolve, { once: true });
  socket.addEventListener("error", () => reject(new Error("Chrome DevTools WebSocket failed")), { once: true });
});
let sequence = 0;
const pending = new Map();
socket.addEventListener("message", event => {
  const message = JSON.parse(event.data);
  const request = pending.get(message.id);
  if (!request) return;
  pending.delete(message.id);
  message.error ? request.reject(new Error(message.error.message)) : request.resolve(message.result);
});
function cdp(method, params = {}) {
  const id = ++sequence;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    socket.send(JSON.stringify({ id, method, params }));
  });
}

let receipt;
let png;
for (let attempt = 0; attempt < 240; attempt += 1) {
  const result = await cdp("Runtime.evaluate", {
    expression: "(() => { const receipt = document.querySelector('#receipt'); return { text: receipt?.textContent ?? '', png: receipt?.dataset.png ?? '' }; })()",
    returnByValue: true,
  });
  ({ text: receipt, png } = result.result.value);
  if (/^ORTET_CANVAS_RECEIPT width=\d+ height=\d+ non_white=\d+ opaque_non_white=[1-9]\d* page_tone=[1-9]\d* card_tone=[1-9]\d* card_border=[1-9]\d* glyph_tone=[1-9]\d*/.test(receipt)) break;
  if (receipt.startsWith("ORTET_CANVAS_RECEIPT error=")) throw new Error(receipt);
  await delay(250);
}
socket.close();
if (!/^ORTET_CANVAS_RECEIPT width=\d+ height=\d+ non_white=\d+ opaque_non_white=[1-9]\d* page_tone=[1-9]\d* card_tone=[1-9]\d* card_border=[1-9]\d* glyph_tone=[1-9]\d*/.test(receipt ?? "")) {
  throw new Error(`receipt did not settle: ${receipt || "missing #receipt"}`);
}
const prefix = "data:image/png;base64,";
if (!png?.startsWith(prefix)) throw new Error("receipt did not return a canvas PNG");
const bytes = Buffer.from(png.slice(prefix.length), "base64");
await mkdir(artifact, { recursive: true });
await writeFile(`${artifact}/chromium-receipt.log`, `${receipt}\n`);
await writeFile(`${artifact}/chromium-canvas.png`, bytes);
await writeFile(`${artifact}/chromium-canvas.sha256`, `${createHash("sha256").update(bytes).digest("hex")}  chromium-canvas.png\n`);
console.log(receipt);
