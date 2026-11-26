import json
from pathlib import Path
import unittest

import windows_gnu_dependency_evidence as evidence

EXE_SHA = "e" * 64
SOURCE_COMMIT = "a" * 40
TARGET = "x86_64-pc-windows-gnu"
CHECKSUM = "c" * 64
ASSET = Path(__file__).resolve().parents[1] / "docs/third-party/pdf-assets/alloc-stdlib.txt"


def receipt(value):
    return json.dumps(value).encode()


def crate(name, version, target):
    return {"name": name, "version": version, "id": f"pkg:{name}@{version}",
            "targets": [{"name": target, "kind": ["lib"]}]}


def artifact(package, path):
    return json.dumps({"reason": "compiler-artifact", "package_id": package["id"],
                       "filenames": [str(path)]})


def lockfile(packages):
    return "\n".join(
        f'[[package]]\nname = "{name}"\nversion = "{version}"\n'
        f'source = "registry+https://github.com/rust-lang/crates.io-index"\nchecksum = "{CHECKSUM}"'
        for name, version in packages
    ).encode()


class GnuDependencyEvidence(unittest.TestCase):
    def test_only_live_target_archive_members_are_observed(self):
        root = Path("/build/proof_a/out") / TARGET / "release"
        legacy = root / "deps/libscoped-1234.rlib"
        scoped = root / "build/scoped/aa22/out/libscoped-aa22.rlib"
        map_text = f"""
0x101 0x16 {legacy}(pre-map.o)
Linker script and memory map
 .rdata 0x120 0x08 {legacy}(c-member.o)
 .rdata 0x120 0x08 {scoped}(scoped-member.o)
 .rdata 0x120 0x08 {scoped}(scoped-member.o)
 .rdata 0x000 0x18 {scoped}(discarded.o)
 .rdata 0x120 0x00 {scoped}(empty.o)
 .rdata 0x120 0x08 {root}/build/scoped/z/out/libscoped-12.rlib(invalid-hash-dir.o)
 .rdata 0x120 0x08 /build/host/release/build/scoped/aa22/out/libscoped-aa22.rlib(host.o)
"""
        self.assertEqual(evidence.live_rust_members(map_text, root),
                         [(scoped, "scoped-member.o"), (legacy, "c-member.o")])
        with self.assertRaisesRegex(ValueError, "memory map"):
            evidence.live_rust_members("discarded-only", root)

    def test_exact_paths_assign_duplicate_versions_but_not_ambiguous_or_missing_claims(self):
        v1 = crate("target-package", "1.2.0", "rust_name")
        v2 = crate("target-package", "2.0.0", "rust_name")
        metadata = {"packages": [v1, v2]}
        packages = {evidence.identity(v1), evidence.identity(v2)}
        v1_path = Path("/build/left/out/librust_name-1234.rlib")
        v2_path = Path("/build/right/out/librust_name-4321.rlib")
        missing = Path("/build/missing/out/librust_name-1234.rlib")
        observed = [(v1_path, "old.o"), (v2_path, "new.o"), (missing, "unknown.o")]
        messages = "\n".join([artifact(v1, v1_path), artifact(v2, v2_path)])
        linked, unknown = evidence.attribute_members(metadata, packages, observed, messages)
        self.assertEqual(linked, {evidence.identity(v1): {v1_path.name: {"old.o"}},
                                  evidence.identity(v2): {v2_path.name: {"new.o"}}})
        self.assertEqual(unknown, {missing.name: {"unknown.o"}})
        linked, unknown = evidence.attribute_members(
            metadata, packages, observed, messages + "\n" + artifact(v2, v1_path))
        self.assertNotIn(evidence.identity(v1), linked)
        self.assertEqual(unknown, {missing.name: {"unknown.o", "old.o"}})

    def test_exact_notices_roles_and_locked_sources_including_upstream_exception(self):
        rows = [
            {"name": "alloc-stdlib", "version": "0.2.4", "source": "crates.io",
             "roles": ["target-normal"], "license_expression": "BSD-3-Clause"},
            {"name": "build-macro", "version": "1.0.0", "source": "crates.io",
             "roles": ["host-build"], "license_expression": "MIT"},
            {"name": "other", "version": "1.0.0", "source": "crates.io",
             "roles": ["target-normal"], "license_expression": "MIT"},
        ]
        lock = lockfile([evidence.identity(row) for row in rows])
        provenance = receipt({"target": TARGET, "exe_sha256": EXE_SHA, "packages": rows})
        license_data = b"verbatim example notice"
        registry = receipt({
            "target": TARGET, "exe_sha256": EXE_SHA, "provenance_sha256": evidence.digest(provenance),
            "cargo_lock_sha256": evidence.digest(lock),
            "packages": [dict(rows[0], archive_sha256=CHECKSUM, notices=[]),
                         dict(rows[2], archive_sha256=CHECKSUM,
                              notices=[{"path": "licenses/other.txt", "sha256": evidence.digest(license_data)}])],
        })
        entries = {evidence.CARGO_PROVENANCE: provenance, evidence.REGISTRY_NOTICES: registry,
                   evidence.RUNTIME_INPUTS: b"[]",
                   "licenses/pdf-assets/alloc-stdlib.txt": ASSET.read_bytes(), "licenses/other.txt": license_data}
        linked_crate = crate("alloc-stdlib", "0.2.4", "alloc_stdlib")
        metadata = {"packages": [linked_crate]}
        archive = Path("/source/release/deps/liballoc_stdlib-125abc.rlib")
        observations = [(archive, "live.o")]
        messages = artifact(linked_crate, archive)
        result = evidence.make_evidence(entries, metadata, lock, observations, messages, SOURCE_COMMIT,
                                        TARGET, EXE_SHA, ["msvcrt.dll"])
        alloc, host, other = result["packages"]
        self.assertEqual(alloc["notice_evidence"], "version-pinned-upstream-notice")
        self.assertEqual(alloc["incorporation"], "nonempty-link-input-observed")
        self.assertEqual(host["registry_archive_locked_sha256"], CHECKSUM)
        self.assertFalse(host["registry_archive_verified_for_notices"])
        self.assertEqual(host["incorporation"], "host-build-only")
        self.assertEqual(other["notice_evidence"], "locked-registry-archive")
        self.assertEqual(other["incorporation"], "target-graph-only")
        self.assertNotIn(str(archive.parent), json.dumps(result))
        with self.assertRaisesRegex(ValueError, "No final link-map archive"):
            evidence.make_evidence(entries, metadata, lock, observations, "", SOURCE_COMMIT, TARGET, EXE_SHA, [])
        entries["licenses/pdf-assets/alloc-stdlib.txt"] = b"not the upstream notice"
        with self.assertRaisesRegex(ValueError, "Notice bytes differ"):
            evidence.make_evidence(entries, metadata, lock, observations, messages, SOURCE_COMMIT, TARGET,
                                   EXE_SHA, ["msvcrt.dll"])
