// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

const activate = document.getElementById("activate");
const state = document.getElementById("state");
activate.addEventListener("click", function () {
  if (document.body.dataset.inlineReady !== "yes") {
    state.textContent = "Ortet O5 native receipt inline failure";
    return;
  }
  // A native pointer release enters this listener. The timer then queues a
  // microtask, so completion needs the production session's input, timer and
  // microtask route before its changed DOM can be painted and inspected.
  setTimeout(function () {
    Promise.resolve().then(function () {
      document.documentElement.className = "complete";
      state.textContent = "Ortet O5 native receipt complete";
    });
  }, 0);
});
