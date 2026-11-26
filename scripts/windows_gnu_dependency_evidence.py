"""Project an exact Windows GNU package's dependency and notice evidence."""

import json
from hashlib import sha256
from pathlib import Path
import re
import tomllib

INDEX = "evidence/gnu-dependency-redistribution.json"
CARGO_PROVENANCE = "evidence/cargo-provenance.json"
REGISTRY_NOTICES = "evidence/registry-notices.json"
RUNTIME_INPUTS = "evidence/gnu-runtime-members.json"
MAP_INPUT = re.compile(r"\b0x([0-9a-f]+)\s+0x([0-9a-f]+)\s+(/[^\r\n]+)$", re.I)
RUST_ARCHIVE = re.compile(r"lib([A-Za-z0-9_]+)-[0-9a-f]+\.rlib", re.ASCII)
OBJECT_NAME = re.compile(r"[A-Za-z0-9_.+-]{1,200}", re.ASCII)
EXPLICIT_NOTICE = {
    ("alloc-stdlib", "0.2.4"): {
        "path": "licenses/pdf-assets/alloc-stdlib.txt",
        "sha256": "c0c56f26d9c051cac4d200c34c84e7ae9aaa853e01a982a1df08b09931e518ae",
        "source": "https://github.com/dropbox/rust-alloc-no-stdlib/blob/"
                  "ae42d22078b98549e987d2f03d12df7b984fde47/LICENSE",
    },
}


def digest(data):
    return sha256(data).hexdigest()


def identity(row):
    return row["name"], row["version"]


def target_archive(archive, target_release):
    if not RUST_ARCHIVE.fullmatch(archive.name) or not archive.is_relative_to(target_release):
        return False
    parts = archive.relative_to(target_release).parts
    return ((len(parts) == 2 and parts[0] == "deps")
            or (len(parts) == 5 and parts[0] == "build" and parts[3] == "out"
                and re.fullmatch(r"[A-Za-z0-9_][A-Za-z0-9_.-]*", parts[1])
                and re.fullmatch(r"[a-f0-9]+", parts[2])))


def live_rust_members(map_text, target_release):
    """Observe nonempty final-map `.rlib` members within this target's output."""
    _, marker, memory = map_text.partition("Linker script and memory map")
    if not marker:
        raise ValueError("GNU ld did not emit the memory map")
    observed = set()
    for line in memory.splitlines():
        match = MAP_INPUT.search(line)
        if not match or int(match[1], 16) == 0 or int(match[2], 16) == 0:
            continue
        filename, separator, raw_member = match[3].strip().partition("(")
        archive = Path(filename)
        if not target_archive(archive, target_release):
            continue
        member = raw_member.removesuffix(")") if separator else ""
        if member and not OBJECT_NAME.fullmatch(member):
            raise ValueError("Unfamiliar member name in target live-input evidence")
        observed.add((archive, member))
    return sorted(observed)


def attribute_members(cargo_metadata, target_packages, observations, artifact_messages):
    """Join live archives to the exact filename and Cargo package ID seen at build."""
    packages_by_id = {row["id"]: identity(row) for row in cargo_metadata["packages"]}
    observed_paths = {archive: set() for archive, _ in observations}
    for line in artifact_messages.splitlines():
        message = json.loads(line)
        if message.get("reason") != "compiler-artifact":
            continue
        for filename in message["filenames"]:
            archive = Path(filename)
            if archive in observed_paths:
                observed_paths[archive].add(message["package_id"])
    linked, unassigned = {}, {}
    for archive, member in observations:
        owners = observed_paths[archive]
        package = packages_by_id.get(next(iter(owners))) if len(owners) == 1 else None
        if package not in target_packages:
            unassigned.setdefault(archive.name, set()).add(member)
            continue
        linked.setdefault(package, {}).setdefault(archive.name, set()).add(member)
    return linked, unassigned


def archives_for_report(archives):
    return [
        {"archive": archive, "live_member_names": sorted(m for m in members if m)}
        for archive, members in sorted(archives.items())
    ]


def package_notices(row, entries):
    if row["notices"]:
        notices = row["notices"]
        source = "locked-registry-archive"
    elif fallback := EXPLICIT_NOTICE.get(identity(row)):
        notices = [fallback]
        source = "version-pinned-upstream-notice"
    else:
        return "unresolved", []
    for notice in notices:
        data = entries.get(notice["path"])
        if data is None or digest(data) != notice["sha256"]:
            raise ValueError(f"Notice bytes differ or are absent: {notice['path']}")
    return source, notices


def make_evidence(entries, cargo_metadata, lock_raw, observations, artifact_messages,
                  commit, target, exe_sha, imports):
    provenance_raw = entries[CARGO_PROVENANCE]
    registry_raw = entries[REGISTRY_NOTICES]
    provenance, registry = json.loads(provenance_raw), json.loads(registry_raw)
    if any((doc["target"], doc["exe_sha256"]) != (target, exe_sha)
           for doc in (registry, provenance)) or registry["provenance_sha256"] != digest(provenance_raw):
        raise ValueError("Package dependency inputs are not for the exact EXE")
    if registry["cargo_lock_sha256"] != digest(lock_raw):
        raise ValueError("Notice inventory recorded a different Cargo.lock")
    locked = {identity(row): row["checksum"] for row in tomllib.loads(lock_raw.decode())["package"]
              if row.get("source") == "registry+https://github.com/rust-lang/crates.io-index"}
    target_packages = {identity(row) for row in provenance["packages"]
                       if "target-normal" in row["roles"]}
    target_registry = {identity(row) for row in provenance["packages"]
                       if row["source"] == "crates.io" and "target-normal" in row["roles"]}
    registry_rows = {identity(row): row for row in registry["packages"]}
    if registry_rows.keys() != target_registry or len(registry_rows) != len(registry["packages"]):
        raise ValueError("Package notice index does not cover the exact target registry graph")
    observed, unattributed = attribute_members(cargo_metadata, target_packages, observations, artifact_messages)
    if not observed:
        raise ValueError("No final link-map archive could be assigned to a target package via Cargo artifacts")
    rows = []
    for row in provenance["packages"]:
        package = identity(row)
        locked_archive = locked.get(package) if row["source"] == "crates.io" else None
        if row["source"] == "crates.io" and locked_archive is None:
            raise ValueError("Selected registry package is absent from locked checksums")
        verified_registry_row = registry_rows.get(package)
        if verified_registry_row:
            if any(verified_registry_row[field] != row[field] for field in ("roles", "license_expression")):
                raise ValueError("Package roles or declared license differ between inputs")
            status, notices = package_notices(verified_registry_row, entries)
            if locked_archive != verified_registry_row["archive_sha256"]:
                raise ValueError("Notice source archive checksum differs from Cargo.lock")
        else:
            status = ("workspace-owner" if row["source"] == "workspace" else
                      "host-build-only" if "target-normal" not in row["roles"] else "unresolved")
            notices = []
        linked = archives_for_report(observed.get(package, {}))
        rows.append({
            "name": row["name"], "version": row["version"], "source_kind": row["source"],
            "source": (f"https://crates.io/crates/{row['name']}/{row['version']}"
                       if row["source"] == "crates.io" else None),
            "cargo_roles": row["roles"], "declared_license": row["license_expression"],
            "registry_archive_locked_sha256": locked_archive,
            "registry_archive_verified_for_notices": verified_registry_row is not None,
            "notice_evidence": status, "notices": notices,
            "incorporation": ("nonempty-link-input-observed" if linked else
                              "target-graph-only" if "target-normal" in row["roles"] else "host-build-only"),
            "live_target_archives": linked,
        })
    return {
        "schema_version": 1, "source_commit": commit, "target": target, "exe_sha256": exe_sha,
        "cargo_provenance_sha256": digest(provenance_raw),
        "registry_notices_sha256": digest(registry_raw),
        "cargo_lock_sha256": registry["cargo_lock_sha256"],
        "runtime_link_inventory": {"path": RUNTIME_INPUTS, "sha256": digest(entries[RUNTIME_INPUTS])},
        "direct_system_imports": imports,
        "packages": rows,
        "unattributed_target_archives": archives_for_report(unattributed),
        "scope": ("Exact locked target resolution, shipped notice bytes and recognizable nonempty inputs "
                  "from the first build's final GNU link map, assigned by exact Cargo artifact path and package ID. "
                  "An unobserved crate is not evidence "
                  "of absence, and a mapped archive does not attribute every executable byte. "
                  "This inventory does not decide redistribution rights."),
    }
