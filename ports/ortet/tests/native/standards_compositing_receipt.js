// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

const state = document.getElementById("state");
const ids = ["source", "canvas-opacity", "group-canvas", "clip-canvas", "boxed", "nested-canvas"];
const canvases = ids.map((id) => document.getElementById(id));
const initial = canvases.map((c) => [c.width, c.height]);

function fail(message) {
  state.textContent = "standards receipt failed: " + message;
}

function clear(canvas, color) {
  const gl = canvas.getContext("webgl", { premultipliedAlpha: true });
  if (!gl || gl !== canvas.getContext("webgl")) throw new Error("WebGL context");
  gl.clearColor(color[0], color[1], color[2], color[3]);
  gl.clear(gl.COLOR_BUFFER_BIT);
  if (gl.getError() !== gl.NO_ERROR) throw new Error("WebGL clear");
}

function run() {
  try {
    if (canvases.some((c, i) => !c || c.width !== initial[i][0] || c.height !== initial[i][1]))
      throw new Error("canvas attributes before draw");
    clear(canvases[0], [0, 0.5, 0, 0.5]); // premultiplied half-green source-over oracle
    clear(canvases[1], [0.5, 0, 0.5, 1]);
    clear(canvases[2], [1, 0, 0, 1]);
    clear(canvases[3], [1, 1, 0, 1]);
    clear(canvases[4], [1, 0, 1, 1]);
    clear(canvases[5], [1, 0, 1, 1]);
    for (const canvas of canvases) {
      const gl = canvas.getContext("webgl");
      if (gl.drawingBufferWidth !== canvas.width || gl.drawingBufferHeight !== canvas.height)
        throw new Error("drawing buffer differs from authored attributes");
    }
    const sourceWidth = canvases[0].width;
    const sourceHeight = canvases[0].height;
    canvases[0].style.width = "50px";
    canvases[0].style.height = "35px";
    const resized = canvases[0].getContext("webgl");
    if (canvases[0].width !== sourceWidth || canvases[0].height !== sourceHeight ||
        resized.drawingBufferWidth !== sourceWidth || resized.drawingBufferHeight !== sourceHeight)
      throw new Error("CSS-only resize changed bitmap");
    canvases[0].style.width = "40px";
    canvases[0].style.height = "30px";
    const group = document.getElementById("group");
    const domOverlay = document.getElementById("dom-over-source");
    if (getComputedStyle(group).opacity !== "0.5") throw new Error("group opacity");
    if (getComputedStyle(canvases[1]).opacity !== "0.5") throw new Error("canvas opacity");
    if (!getComputedStyle(domOverlay).backgroundColor) throw new Error("DOM overlay color");
    if (canvases.some((c, i) => c.width !== initial[i][0] || c.height !== initial[i][1]))
      throw new Error("canvas attributes changed");
    state.textContent = "Ortet standards compositing receipt complete";
    document.documentElement.dataset.standardsComplete = "yes";
  } catch (error) { fail(error.message); }
}

run();
