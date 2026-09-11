// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

const canvas = document.getElementById("surface");
const state = document.getElementById("state");

function fail(message) {
  state.textContent = "Ortet WebGL receipt failed: " + message;
}

function run() {
  if (document.body.dataset.inlineReady !== "yes") {
    fail("inline DOM ordering");
    return;
  }
  if (!canvas || !state || typeof canvas.getContext !== "function") {
    fail("canvas missing");
    return;
  }

  const gl = canvas.getContext("webgl");
  if (!gl) {
    fail("WebGL context missing");
    return;
  }
  if (gl !== canvas.getContext("webgl")) {
    fail("context identity changed");
    return;
  }
  if (gl.drawingBufferWidth !== 320 || gl.drawingBufferHeight !== 200) {
    fail("initial drawing buffer size");
    return;
  }

  // Exercise the actual producer before resizing. The final green clear is
  // visible in the captured canvas only if the WebGL producer is connected.
  gl.clearColor(0.77, 0.24, 0.24, 1);
  gl.clear(gl.COLOR_BUFFER_BIT);
  if (gl.getError() !== gl.NO_ERROR) {
    fail("initial clear error");
    return;
  }

  canvas.width = 160;
  canvas.height = 100;
  if (gl !== canvas.getContext("webgl") ||
      gl.drawingBufferWidth !== 160 || gl.drawingBufferHeight !== 100) {
    fail("resize changed context identity or dimensions");
    return;
  }

  // This color is the receipt pixel. It is issued through gl.clear so a
  // missing or disconnected canvas cannot satisfy the visual gate with CSS.
  gl.clearColor(0.184, 0.42, 0.235, 1);
  gl.clear(gl.COLOR_BUFFER_BIT);
  if (gl.getError() !== gl.NO_ERROR) {
    fail("resized clear error");
    return;
  }

  // Publish semantic completion only after every DOM ordering, producer,
  // identity, resize and GL error check has passed.
  state.textContent = "Ortet WebGL receipt complete";
  document.documentElement.dataset.webglComplete = "yes";
}

run();
