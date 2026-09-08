// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

self.onmessage = function () {
  fetch("/worker-gate?case=worker")
    .then(function (response) { return response.text(); })
    .then(function (text) {
      if (text !== "WORKER_GATE_OK") throw new Error("unexpected worker gate body");
      postMessage("WORKER_OK");
    });
};
