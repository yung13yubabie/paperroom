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
    return {"name": name, "version": version, "targets": [{"name": target, "kind": ["lib"]}]}


def lockfile(packages):
    return "\n".join(
        f'[[package]]\nname = "{name}"\nversion = "{version}"\n'
        f'source = "registry+https://github.com/rust-lang/crates.io-index"\nchecksum = "{CHECKSUM}"'
        for name, version in packages
    ).encode()


class GnuDependencyEvidence(unittest.TestCase):
    def test_only_live_target_archive_members_are_observed(self):
        root = Path("/build/proof_a/out") / TARGET / "release/deps"
        map_text = f"""
0x101 0x16 {root}/libscoped-1234.rlib(nope.o)
Linker script and memory map
 .rdata 0x120 0x08 {root}/libscoped-1234.rlib(c-member.o)
 .rdata 0x120 0x08 {root}/libscoped-1234.rlib(c-member.o)
 .rdata 0x000 0x18 {root}/libscoped-1234.rlib(discarded.o)
 .rdata 0x120 0x00 {root}/libscoped-1234.rlib(empty.o)
 .rdata 0x120 0x08 /build/host/deps/libscoped-1234.rlib(host.o)
"""
        self.assertEqual(evidence.live_rust_members(map_text, root),
                         [("libscoped-1234.rlib", "c-member.o")])
        with self.assertRaisesRegex(ValueError, "memory map"):
            evidence.live_rust_members("discarded-only", root)

    def test_same_crate_target_name_across_versions_is_not_attributed(self):
        packages = {("target-package", "1.2.0"), ("target-package", "2.0.0")}
        metadata = {"packages": [crate(name, version, "rust_name") for name, version in packages]}
        observed = [("librust_name-1234.rlib", "object.o")]
        linked, unknown = evidence.attribute_members(metadata, packages, observed)
        self.assertEqual(linked, {})
        self.assertEqual(unknown, {"librust_name-1234.rlib": {"object.o"}})

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
        metadata = {"packages": [crate("alloc-stdlib", "0.2.4", "alloc_stdlib")]}
        observations = [("liballoc_stdlib-125abc.rlib", "live.o")]
        result = evidence.make_evidence(entries, metadata, lock, observations, SOURCE_COMMIT, TARGET,
                                        EXE_SHA, ["msvcrt.dll"])
        alloc, host, other = result["packages"]
        self.assertEqual(alloc["notice_evidence"], "version-pinned-upstream-notice")
        self.assertEqual(alloc["incorporation"], "nonempty-link-input-observed")
        self.assertEqual(host["registry_archive_locked_sha256"], CHECKSUM)
        self.assertFalse(host["registry_archive_verified_for_notices"])
        self.assertEqual(host["incorporation"], "host-build-only")
        self.assertEqual(other["notice_evidence"], "locked-registry-archive")
        self.assertEqual(other["incorporation"], "target-graph-only")
        entries["licenses/pdf-assets/alloc-stdlib.txt"] = b"not the upstream notice"
        with self.assertRaisesRegex(ValueError, "Notice bytes differ"):
            evidence.make_evidence(entries, metadata, lock, observations, SOURCE_COMMIT, TARGET,
                                   EXE_SHA, ["msvcrt.dll"])
