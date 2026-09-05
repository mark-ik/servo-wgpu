#!/usr/bin/env python3

# Copyright 2026 Mark Alan Boykin
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
# SPDX-License-Identifier: MPL-2.0

"""Dependency-cone witnesses for Genet profile boundaries."""

from __future__ import annotations

import json
import pathlib
import re
import subprocess
import sys
import tomllib


ROOT = pathlib.Path(__file__).resolve().parents[2]


def fail(message: str) -> None:
    print(f"dependency-cone witness failed: {message}", file=sys.stderr)
    raise SystemExit(1)


def load_toml(path: pathlib.Path) -> dict:
    with path.open("rb") as fh:
        return tomllib.load(fh)


def dependency_names(table: dict) -> set[str]:
    return set(table.keys())


def is_beneath(path: pathlib.Path, parent: pathlib.Path) -> bool:
    try:
        path.resolve().relative_to(parent.resolve())
    except ValueError:
        return False
    return True


def assert_fleece_cone() -> None:
    manifest = load_toml(ROOT / "components" / "fleece" / "Cargo.toml")
    deps = dependency_names(manifest.get("dependencies", {}))
    expected = {"layout_dom_api", "unicode-segmentation"}
    if deps != expected:
        fail(f"fleece dependencies are {sorted(deps)}, expected {sorted(expected)}")
    build_deps = dependency_names(manifest.get("build-dependencies", {}))
    if build_deps:
        fail(f"fleece build-dependencies must stay empty, found {sorted(build_deps)}")
    dev_deps = dependency_names(manifest.get("dev-dependencies", {}))
    allowed_dev_deps = {"genet-static-dom"}
    if dev_deps - allowed_dev_deps:
        fail(
            "fleece dev-dependencies contain non-fixture deps: "
            f"{sorted(dev_deps - allowed_dev_deps)}"
        )

    forbidden = {
        "genet-layout",
        "genet-render",
        "paint",
        "paint_list_render",
        "netrender",
        "wgpu",
    }
    if deps & forbidden:
        fail(f"fleece pulled render deps directly: {sorted(deps & forbidden)}")


def cargo_metadata() -> dict:
    result = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"],
        cwd=ROOT,
        text=True,
        encoding="utf-8",
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        fail(f"cargo metadata failed:\n{result.stderr}")
    return json.loads(result.stdout)


def assert_cargo_metadata_sees_fleece(metadata: dict) -> None:
    names = {package["name"] for package in metadata["packages"]}
    missing = {"fleece"} - names
    if missing:
        fail(f"cargo metadata did not report fleece package {sorted(missing)}")


def assert_ports_depend_inward(metadata: dict) -> None:
    components = (ROOT / "components").resolve()
    ports = (ROOT / "ports").resolve()
    host_api = []
    ortet_packages = {}

    for package in metadata["packages"]:
        manifest = pathlib.Path(package["manifest_path"]).resolve()
        if package["name"] == "genet-host-api":
            host_api.append(manifest)
        if package["name"] == "ortet":
            ortet_packages[package["name"]] = manifest
        if not is_beneath(manifest, components):
            continue
        for dependency in package["dependencies"]:
            path = dependency.get("path")
            if path is not None and is_beneath(pathlib.Path(path), ports):
                fail(
                    f"{manifest.relative_to(ROOT)} depends on port package "
                    f"{dependency['name']} at {pathlib.Path(path).relative_to(ROOT)}"
                )

    expected_api = (components / "genet-host-api" / "Cargo.toml").resolve()
    if host_api != [expected_api]:
        rendered = [str(path.relative_to(ROOT)) for path in host_api]
        fail(
            f"genet-host-api manifests are {rendered}, "
            "expected components/genet-host-api/Cargo.toml"
        )

    # Pelt left for mere on 2026-09-03 (platform boundary plan, consumers-first
    # order); ortet is genet's headed host and takes the same assertion
    # (design_docs/2026-09-03_ortet_founding_plan.md, O4).
    expected_ortet = {"ortet": (ports / "ortet" / "Cargo.toml").resolve()}
    if ortet_packages != expected_ortet:
        rendered = {
            name: str(path.relative_to(ROOT)) for name, path in ortet_packages.items()
        }
        fail(f"Ortet package manifests are {rendered}, expected {expected_ortet}")


# Genet resolves without Mere (platform boundary plan, mere
# design_docs/mere_docs/implementation_strategy/
# 2026-09-02_platform_boundary_and_repository_topology_plan.md, invariant 1).
# Three ways a Mere source can enter the graph, each checked: a git source at
# Mere's repository, a path dependency that leaves this repository for a `mere`
# checkout, and a registry crate that Mere publishes. The name list is the
# unprefixed publishable members of Mere's workspace on 2026-09-02; prefixed
# families are matched by prefix. Extend it when Mere publishes a new family.
MERE_GIT_SOURCE = re.compile(r"merely-made/mere(?:\.git)?(?:[?#/]|$)")
MERE_CRATE_PREFIXES = ("mere-", "graphshell", "sceno", "register-")
MERE_CRATES = {
    "mere", "scenograph", "armillary", "chartulary", "codicil", "muniment",
    "scholia", "tulpa", "personae", "dramatis", "gaz", "gazette", "servitor",
    "vates", "sibylla", "conatus", "numen", "quint", "seiche", "murm",
    "moothold", "mooting", "gemot", "castellan", "chatelaine", "chirograph",
    "distillery", "djinn", "esp", "graphlets", "incipit", "insigne", "luggage",
    "mien", "nisus", "notochord", "pandect", "pictograph", "platen",
    "stickleback", "titulus", "ux-events", "uxtree", "script-rhai",
    # Explicitly moved or Mere-published names whose registry packages are
    # unprefixed. Keep this list to the platform-boundary inventory.
    "inker", "workbench", "nematic", "meristem", "sprigging",
    "document-canvas", "scrying-engine", "graft-engine", "weld-engine",
    "verso-tile", "illume", "errand", "tinct", "tabard", "knot-editor-host",
    "mere-surface-api",
}

# Names and families moved to Mere by the 2026-09-03 platform-boundary
# landing. This is shared by the registry-source and Ortet-cone witnesses so
# the two guards cannot drift. The older MERE_CRATES/MERE_CRATE_PREFIXES above
# remain the pre-existing global Mere catalog.
MOVED_TO_MERE_NAMES = frozenset({
    "pelt", "pelt-core", "pelt-desktop", "tabard", "knot-editor-host",
    "cambium", "cambium-rootstock", "cambium-winit", "cambium-winit-a11y",
    "cambium-genet-winit-host", "cambium-genet-web-host", "cambium-nematic",
    "meristem", "sprigging", "workbench", "mere-surface-api", "inker",
    "document-canvas", "scrying-engine", "graft-engine", "weld-engine",
    "verso-tile", "nematic", "illume", "errand", "tinct",
})
MOVED_TO_MERE_PREFIXES = ("cambium", "pelt")


def is_mere_crate(name: str) -> bool:
    return (
        name in MERE_CRATES
        or name in MOVED_TO_MERE_NAMES
        or name.startswith(MERE_CRATE_PREFIXES + MOVED_TO_MERE_PREFIXES)
    )


def cargo_metadata_resolved() -> dict:
    """The full resolved graph: transitive sources are what the invariant is about."""
    result = subprocess.run(
        ["cargo", "metadata", "--format-version", "1"],
        cwd=ROOT,
        text=True,
        encoding="utf-8",
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        fail(f"cargo metadata (resolved) failed:\n{result.stderr}")
    return json.loads(result.stdout)


def mere_sources(metadata: dict) -> list[str]:
    """Every package in the resolved graph that comes from Mere, as messages."""
    found = []
    for package in metadata["packages"]:
        name = package["name"]
        source = package.get("source") or ""
        manifest = pathlib.Path(package["manifest_path"])
        if MERE_GIT_SOURCE.search(source):
            found.append(f"{name} resolves from Mere's repository: {source}")
        elif not source and not is_beneath(manifest, ROOT):
            if "mere" in {part.lower() for part in manifest.parts}:
                found.append(f"{name} is a path dependency into a mere checkout: {manifest}")
        elif source.startswith("registry+") and is_mere_crate(name):
            found.append(f"{name} is a Mere-published crate pulled from the registry ({package['version']})")
    return found


def assert_no_mere_source(metadata: dict) -> None:
    found = mere_sources(metadata)
    if found:
        fail("Genet's graph reaches Mere:\n  " + "\n  ".join(found))


def self_test_mere_witness() -> None:
    """Positive control: the witness must catch each of the three entry routes."""
    registry = "registry+https://github.com/rust-lang/crates.io-index"
    moved_registry_names = {
        "inker", "cambium", "workbench", "nematic", "sprigging", "meristem",
        "pelt-desktop",
    }
    fake = {"packages": [
        {"name": "inker", "version": "0.0.3", "source": registry,
         "manifest_path": str(ROOT / "registry" / "inker.toml")},
        {"name": "cambium", "version": "0.0.3", "source": registry,
         "manifest_path": str(ROOT / "registry" / "cambium.toml")},
        {"name": "workbench", "version": "0.0.3", "source": registry,
         "manifest_path": str(ROOT / "registry" / "workbench.toml")},
        {"name": "nematic", "version": "0.0.3", "source": registry,
         "manifest_path": str(ROOT / "registry" / "nematic.toml")},
        {"name": "sprigging", "version": "0.0.3", "source": registry,
         "manifest_path": str(ROOT / "registry" / "sprigging.toml")},
        {"name": "meristem", "version": "0.0.3", "source": registry,
         "manifest_path": str(ROOT / "registry" / "meristem.toml")},
        {"name": "pelt-desktop", "version": "0.0.3", "source": registry,
         "manifest_path": str(ROOT / "registry" / "pelt-desktop.toml")},
        {"name": "sceno", "version": "0.0.3", "source": registry,
         "manifest_path": str(ROOT / "x" / "Cargo.toml")},
        {"name": "anything", "version": "0.1.0",
         "source": "git+https://github.com/merely-made/mere.git?rev=abc#abc",
         "manifest_path": str(ROOT / "y" / "Cargo.toml")},
        {"name": "moothold", "version": "0.1.0", "source": None,
         "manifest_path": str(ROOT.parent / "mere" / "crates" / "moothold" / "Cargo.toml")},
        {"name": "netrender", "version": "0.1.0",
         "source": "git+https://github.com/merely-made/netrender.git?rev=abc#abc",
         "manifest_path": str(ROOT / "z" / "Cargo.toml")},
        {"name": "mer3ly-ish", "version": "0.1.0",
         "source": "git+https://github.com/merely-made/mer3ly.git?rev=abc#abc",
         "manifest_path": str(ROOT / "w" / "Cargo.toml")},
    ]}
    found = mere_sources(fake)
    if len(found) != 10 or not all(any(name in item for item in found) for name in moved_registry_names) \
            or not any("registry" in f for f in found) or not any("repository" in f for f in found) \
            or not any("checkout" in f for f in found):
        fail(f"mere witness self-test expected moved registry families plus git/path routes, got {found}")
    clean_names = {"fleece", "document-session-api", "netrender"}
    clean = {"packages": [
        {"name": name, "version": "0.1.0", "source": registry,
         "manifest_path": str(ROOT / "clean" / f"{name}.toml")}
        for name in sorted(clean_names)
    ]}
    if mere_sources(clean):
        fail(f"mere witness negative control classified independent crates as Mere: {mere_sources(clean)}")


# netfetcher's authority split (platform boundary plan P1): the Fetch semantics
# Genet owns must link no transport. With default features off the resolved
# graph may not contain any of these; the transport lanes bring them in only
# behind `hyper-transport`, `h3` and `websocket`.
NETFETCHER_TRANSPORT_CRATES = {
    "hyper", "hyper-util", "hyper-rustls", "rustls", "quinn", "h3", "h3-quinn",
    "tokio-tungstenite", "webpki-roots", "http", "http-body-util",
}


def assert_netfetcher_semantics_cone() -> None:
    result = subprocess.run(
        ["cargo", "tree", "-p", "netfetcher", "--no-default-features", "-e", "normal",
         "--prefix", "none"],
        cwd=ROOT,
        text=True,
        encoding="utf-8",
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        fail(f"cargo tree for netfetcher without default features failed:\n{result.stderr}")
    names = {line.split()[0] for line in result.stdout.splitlines() if line.strip()}
    if "netfetcher" not in names:
        fail("cargo tree for netfetcher did not list netfetcher itself")
    reached = sorted(names & NETFETCHER_TRANSPORT_CRATES)
    if reached:
        fail(f"netfetcher's semantics-only build reaches transport crates: {reached}")


# genet-host-api's authority split (platform boundary plan P1): the raw engine
# host half keeps the name and depends on nothing in the workspace. The
# application half, mere-surface-api, left for mere on 2026-09-03 with
# workbench and the Cambium family, so it is no longer a member to check here;
# its cone is mere's to witness.
def assert_host_api_cone(metadata: dict) -> None:
    members = {package["name"] for package in metadata["packages"]}
    by_name = {package["name"]: package for package in metadata["packages"]}
    for name, allowed in (("genet-host-api", set()),
                          # inker's engine-facing contract half (P1): a leaf.
                          ("document-session-api", set())):
        package = by_name.get(name)
        if package is None:
            fail(f"{name} is not a workspace member")
        deps = {d["name"] for d in package["dependencies"] if d["name"] in members}
        if deps - allowed:
            fail(f"{name} depends on workspace crates {sorted(deps - allowed)}; allowed {sorted(allowed)}")
    # Engine crates the plan keeps never depend on crates it moves. genet-render
    # reaches inker's contracts through the contract crate; genet-documents,
    # split by authority, keeps only the Livery and Scripted lanes and links no
    # controller, content lane or transport. Both subjects are members that
    # stay; the banned names on the right left genet in P3 on 2026-09-03 and
    # are kept, like ORTET_FORBIDDEN, as names the edge may never carry again.
    forbidden = {
        "genet-render": {"inker"},
        "genet-documents": {"inker", "nematic", "errand", "document-canvas", "netfetcher"},
    }
    for name, banned in forbidden.items():
        deps = {d["name"] for d in by_name[name]["dependencies"]}
        if deps & banned:
            fail(f"{name} depends on {sorted(deps & banned)}, which the boundary plan moves to Mere")


# Ortet's cone (2026-09-03 ortet founding plan, O0). Ortet is the one headed
# host that proves the engine runs without Mere, so its resolved cone must
# contain no crate the platform boundary plan moves to Mere, and no other port.
# Every name below is now a crate that has left genet: `tabard`,
# `knot-editor-host` and the `pelt` prefix on 2026-09-03; the `cambium` and
# `mere-` prefixes and `workbench` the same day; `inker`, `nematic`, `errand`
# and `document-canvas` with the engine-management layer in P3. They stay in
# the list: it is a set of names the cone may not reach, not a set of workspace
# members, so keeping them fails loudly if any of them ever arrives back from
# mere as a git or registry source.
ORTET_FORBIDDEN = MOVED_TO_MERE_NAMES
ORTET_FORBIDDEN_PREFIXES = ("mere-",) + MOVED_TO_MERE_PREFIXES
# `fleece` is named by the founding plan's forbidden list, but section 9.1 of
# the boundary plan reclasses it *independent* -- "it may stay in genet as a
# lower library or leave for its own repository, but it does not go to Mere" --
# and
# `genet-documents` reaches it unconditionally at `src/engines/clip.rs:114`
# (`fleece::extract_main_text`). Ortet cannot have the Livery session engine
# without it. It is carved out here rather than dropped silently, so the
# exception is visible on every CI run and fails loudly if it ever widens.
ORTET_RECLASSED = {"fleece"}


def is_ortet_forbidden(name: str) -> bool:
    return name in MOVED_TO_MERE_NAMES or name.startswith(ORTET_FORBIDDEN_PREFIXES)


def resolved_cone(metadata: dict, package: str) -> set[str]:
    """Package names reachable from `package` over normal (non-dev, non-build)
    dependency edges in cargo's resolve graph."""
    resolve = metadata.get("resolve")
    if resolve is None:
        fail("cargo metadata carried no resolve graph")
    ids = {node["id"]: node for node in resolve["nodes"]}
    name_of = {p["id"]: p["name"] for p in metadata["packages"]}
    roots = [pid for pid, name in name_of.items() if name == package]
    if len(roots) != 1:
        fail(f"expected exactly one {package} package, found {len(roots)}")

    # Cargo resolves package *ids*, not names.  Two versions of one crate may
    # have different outgoing edges, so de-duplicating by display name can hide
    # a forbidden dependency behind whichever version happened to be queued
    # first.  Keep the traversal identity-based; names are the witness's
    # reporting vocabulary only.
    seen_ids: set[str] = {roots[0]}
    seen_names: set[str] = set()
    queue = [roots[0]]
    while queue:
        current = queue.pop()
        node = ids.get(current)
        if node is None:
            continue
        for dependency in node["deps"]:
            kinds = dependency.get("dep_kinds") or [{"kind": None}]
            # A `kind` of None is a normal dependency; "dev" and "build" are not
            # part of what the binary links.
            if not any(entry.get("kind") is None for entry in kinds):
                continue
            name = name_of.get(dependency["pkg"])
            if name is None or dependency["pkg"] in seen_ids:
                continue
            seen_ids.add(dependency["pkg"])
            seen_names.add(name)
            queue.append(dependency["pkg"])
    return seen_names


def self_test_resolved_cone() -> None:
    """Exercise the resolve walk with Cargo-shaped package ids and edge kinds."""
    def package(identifier: str, name: str) -> dict:
        return {"id": identifier, "name": name}

    def dependency(identifier: str, kind: str | None = None) -> dict:
        return {"pkg": identifier, "dep_kinds": [{"kind": kind}]}

    # `relay@1` and `relay@2` deliberately share a name.  Each has a distinct
    # outgoing forbidden path; both must be followed.  The old name-deduping
    # walk visited only one of them.  Dev/build-only poison must remain absent.
    positive = {
        "packages": [
            package("root 0.1.0", "synthetic-root"),
            package("relay 1.0.0", "relay"),
            package("relay 2.0.0", "relay"),
            package("exact-bridge 0.1.0", "exact-bridge"),
            package("prefix-bridge 0.1.0", "prefix-bridge"),
            package("inker 0.1.0", "inker"),
            package("mere-test 0.1.0", "mere-test"),
            package("dev-poison 0.1.0", "pelt-dev-only"),
            package("build-poison 0.1.0", "cambium-build-only"),
        ],
        "resolve": {"nodes": [
            {"id": "root 0.1.0", "deps": [
                dependency("relay 1.0.0"), dependency("relay 2.0.0"),
                dependency("dev-poison 0.1.0", "dev"),
                dependency("build-poison 0.1.0", "build"),
            ]},
            {"id": "relay 1.0.0", "deps": [dependency("exact-bridge 0.1.0")]},
            {"id": "relay 2.0.0", "deps": [dependency("prefix-bridge 0.1.0")]},
            {"id": "exact-bridge 0.1.0", "deps": [dependency("inker 0.1.0")]},
            {"id": "prefix-bridge 0.1.0", "deps": [dependency("mere-test 0.1.0")]},
            {"id": "inker 0.1.0", "deps": []},
            {"id": "mere-test 0.1.0", "deps": []},
            {"id": "dev-poison 0.1.0", "deps": []},
            {"id": "build-poison 0.1.0", "deps": []},
        ]},
    }
    reached = resolved_cone(positive, "synthetic-root")
    required = {"relay", "exact-bridge", "prefix-bridge", "inker", "mere-test"}
    if not required <= reached:
        fail(f"resolved-cone positive control missed {sorted(required - reached)}")
    forbidden = sorted(name for name in reached if is_ortet_forbidden(name))
    if forbidden != ["inker", "mere-test"]:
        fail(f"resolved-cone positive control reported {forbidden}, expected inker and mere-test")
    ignored = {"pelt-dev-only", "cambium-build-only"}
    if reached & ignored:
        fail(f"resolved-cone included non-normal edges {sorted(reached & ignored)}")

    negative = {
        "packages": [
            package("safe-root 0.1.0", "safe-root"),
            package("safe-middle 0.1.0", "safe-middle"),
            package("genet-livery 0.1.0", "genet-livery"),
        ],
        "resolve": {"nodes": [
            {"id": "safe-root 0.1.0", "deps": [dependency("safe-middle 0.1.0")]},
            {"id": "safe-middle 0.1.0", "deps": [dependency("genet-livery 0.1.0")]},
            {"id": "genet-livery 0.1.0", "deps": []},
        ]},
    }
    safe = resolved_cone(negative, "safe-root")
    if safe != {"safe-middle", "genet-livery"}:
        fail(f"resolved-cone negative control reached {sorted(safe)}")
    if any(is_ortet_forbidden(name) for name in safe):
        fail("resolved-cone negative control reached a forbidden package")
    print(
        "resolved-cone synthetic controls: exact and prefix forbidden paths found; "
        "same-name package ids both traversed; dev/build edges excluded; allowed graph clean"
    )


def assert_ortet_cone(metadata: dict) -> None:
    cone = resolved_cone(metadata, "ortet")
    if "genet-documents" not in cone or "netrender" not in cone:
        fail(
            "ortet's resolved cone is missing the engine crates it is built on "
            f"(saw {len(cone)} packages); the cone walk is not reading ortet"
        )
    breached = sorted(name for name in cone if is_ortet_forbidden(name))
    if breached:
        fail(
            "ortet's cone reaches crates the platform boundary plan moves to "
            f"Mere: {breached}"
        )

    # Positive controls: the check must be able to SEE what it forbids. A walk
    # that reports nothing on a cone known to contain forbidden crates is a
    # broken instrument, not a clean cone.
    #
    # THE LIVE CONTROL RETIRED WITH P3. It was pelt-desktop, whose cone
    # exercised both halves of `is_ortet_forbidden` at once -- an exact name
    # (`inker`) and a prefix (`cambium`, `mere-`, `pelt`) -- until Pelt left
    # for mere on 2026-09-03. The prefix half then moved to
    # `cambium-genet-winit-host`, which left with the Cambium family the same
    # day, and the exact half to `document-canvas`, which left with the
    # engine-management layer in P3. No member remains whose cone reaches any
    # forbidden name, so there is no crate left to carry a live walk, and one
    # cannot be invented without reintroducing what the boundary removed.
    # `self_test_resolved_cone` is the current graph control: it runs this
    # actual traversal through a Cargo-shaped graph with exact and prefix
    # forbidden paths, same-name package ids, and non-normal edges. The
    # predicate assertions below remain a separate guard on the name rule; the
    # live Ortet walk must still reach genet-documents and netrender.
    reported = {}

    # The exact-name and prefix halves of the predicate, asserted directly.
    for forbidden_name in sorted(ORTET_FORBIDDEN):
        if not is_ortet_forbidden(forbidden_name):
            fail(
                "cone witness positive control failed: is_ortet_forbidden "
                f"does not forbid {forbidden_name}, which is in ORTET_FORBIDDEN"
            )
    for accepted in ("cambium-anything", "mere-anything", "pelt-anything"):
        if not is_ortet_forbidden(accepted):
            fail(
                "cone witness positive control failed: is_ortet_forbidden "
                f"does not forbid {accepted}, so the prefix half of the rule "
                "is not in force"
            )
    # ...and the matching negative, so the predicate is not simply always true.
    if is_ortet_forbidden("genet-livery"):
        fail(
            "cone witness control failed: is_ortet_forbidden forbids "
            "genet-livery, an engine crate ortet is built on"
        )

    rendered = "; ".join(f"{name} reports {hits}" for name, hits in reported.items())
    print(
        f"ortet cone: {len(cone)} packages, none forbidden; "
        f"historical live positive control: {rendered or 'none (retired with P3)'}; "
        f"predicate control: forbids all {len(ORTET_FORBIDDEN)} exact names, "
        "forbids cambium-/mere-/pelt-anything, admits genet-livery"
    )

    reclassed = sorted(name for name in cone if name in ORTET_RECLASSED)
    if reclassed:
        print(
            f"ortet cone note: {reclassed} present - reclassed independent by "
            "the boundary plan 9.1, reached through genet-documents' clip lane"
        )


def main() -> None:
    assert_fleece_cone()
    self_test_resolved_cone()
    metadata = cargo_metadata()
    assert_cargo_metadata_sees_fleece(metadata)
    assert_ports_depend_inward(metadata)
    assert_host_api_cone(metadata)
    self_test_mere_witness()
    resolved = cargo_metadata_resolved()
    assert_no_mere_source(resolved)
    assert_ortet_cone(resolved)
    assert_netfetcher_semantics_cone()
    print("dependency-cone witnesses passed")


if __name__ == "__main__":
    main()
