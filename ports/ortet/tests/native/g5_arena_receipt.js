// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

// G5 arena semantic-contract proof sequence, driven by one native click:
// retain a node from JS, detach it, force collection pressure, adopt/reattach
// it into a new parent in the same document, mutate it, let the host render
// the mutation, then release the only remaining reference. Every stage is a
// JS-observable assertion; a failed assertion sets a distinct, non-matching
// heading rather than approximating success. Collection itself is confirmed
// host-side, through the same production `Runtime::collect_garbage`
// unpinned/collected accounting the S1/S2/S5 arena receipts already treat as
// authoritative (`ortet: receipt collection unpinned=... collected=...`),
// not by an engine-specific script-visible probe.

const activate = document.getElementById("activate");
const state = document.getElementById("state");

activate.addEventListener("click", function () {
  if (document.body.dataset.inlineReady !== "yes") {
    state.textContent = "Ortet G5 arena sequence inline failure";
    return;
  }
  runSequence();
});

function runSequence() {
  // 1. Retain a node and a descendant through JS references.
  let victim = document.getElementById("victim");
  let victimChild = document.getElementById("victim-child");
  if (!victim || !victimChild || victimChild.parentNode !== victim) {
    state.textContent = "Ortet G5 retain failed: node or descendant missing";
    return;
  }
  const retainedId = victim.id;

  // A native pointer release enters this listener; every following stage
  // runs across its own timer turn so the host's frame-cadence GC tick
  // (`Runtime::pump` -> `collect_garbage`) actually runs between stages
  // instead of coalescing into one timer-service batch.
  setTimeout(function () {
    // 2. Detach via explicit removal. The retained descendant must stay
    // readable and reinsertable while only the JS reference holds it.
    victim.remove();
    if (document.getElementById(retainedId)) {
      state.textContent = "Ortet G5 detach failed: node still findable in tree";
      return;
    }
    if (victim.parentNode !== null || victimChild.parentNode !== victim) {
      state.textContent = "Ortet G5 detach failed: retained descendant unreadable";
      return;
    }

    setTimeout(function () {
      // 3. Collection pressure: the two preceding timer turns each ended in
      // a GC tick while `victim` was detached but still JS-reachable. It
      // must have survived — the point of this stage is that a live root
      // is spared, not that detachment alone frees the node.
      if (victim.textContent !== "victim payloadchild") {
        state.textContent = "Ortet G5 collection-pressure failed: retained node did not survive";
        return;
      }

      // 4. Adopt/reattach into the active document under a new parent,
      // proving stable identity rather than a copy against a different
      // store.
      const destination = document.getElementById("destination");
      destination.appendChild(victim);
      if (victim.parentNode !== destination || document.getElementById(retainedId) !== victim) {
        state.textContent = "Ortet G5 adopt failed: identity not preserved on reattachment";
        return;
      }

      // 5. Mutate the reattached node.
      victim.textContent = "victim reattached and mutated";
      victim.className = "reattached";

      // 6. Render: the next composed frame shows this mutation. No further
      // JS action is needed for the frame itself; the host paints on its
      // normal per-frame cadence.
      setTimeout(function () {
        // 7. Release: drop the tree link and every remaining strong JS
        // reference, including the retained descendant (a dangling child
        // whose `parentNode` still pointed at `victim` would keep the
        // retained root reachable through that link alone).
        victim.remove();
        victim = null;
        victimChild = null;

        // 8. Give the host several more frame-cadence GC ticks to actually
        // reap the released subtree before declaring completion, so the
        // collection counts the host reports are correlated with this same
        // heading and frame rather than a race against the capture.
        setTimeout(function () {
          document.documentElement.className = "complete";
          state.textContent = "Ortet G5 arena sequence complete";
        }, 240);
      }, 40);
    }, 40);
  }, 40);
}
