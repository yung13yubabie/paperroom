import hashlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("windows-build-provenance.py")
spec = importlib.util.spec_from_file_location("provenance", SCRIPT)
provenance = importlib.util.module_from_spec(spec)
spec.loader.exec_module(provenance)

DIGEST = "3210" * 16
REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"
PRIVATE = "registry+https://github.com/rust-lang/crates.io-index@private.invalid/runner/User"
ROOT = "path+file:///D:/Users/private-user/Paperroom#paperroom@0.1.2"


def item(key, name, source, kind="lib", notice=None):
    return {
        "id": key, "name": name, "version": "0.1.2", "source": source,
        "license": "MIT OR Apache-2.0", "license_file": notice,
        "manifest_path": r"C:\Users\private-user\Cargo.toml",
        "targets": [{"kind": [kind], "src_path": "/private-user/src/lib.rs"}],
    }


def edge(key, kind=None):
    return {"pkg": key, "dep_kinds": [{"kind": kind, "target": None}]}


def sample():
    deps = {
        ROOT: [edge("registry"), edge("alternative"), edge("cc", "build"), edge("derive"), edge("dev", "dev")],
        "registry": [edge("shared")],
        "alternative": [],
        "cc": [edge("host-child"), edge("shared")],
        "derive": [edge("host-child")],
        "shared": [],
        "host-child": [],
        "dev": [],
        "orphan": [],
    }
    packages = [
        item(ROOT, "paperroom", None, "bin"),
        item("registry", "same", REGISTRY),
        item("alternative", "same", PRIVATE, notice=r"D:\private-user\private-repo\LICENSE.custom"),
        item("cc", "cc", REGISTRY),
        item("derive", "derive", REGISTRY, "proc-macro"),
        item("shared", "shared", REGISTRY),
        item("host-child", "host-child", REGISTRY),
        item("dev", "dev", REGISTRY),
        item("orphan", "orphan", REGISTRY),
    ]
    return {
        "version": 1, "workspace_members": [ROOT], "packages": packages,
        "workspace_root": "/Users/private-user", "target_directory": "/private-user/target",
        "resolve": {"root": ROOT, "nodes": [{"id": key, "deps": nested} for key, nested in deps.items()]},
    }


class ProvenanceTests(unittest.TestCase):
    def test_scope_preserves_identity_and_omits_nonreachable_and_private_data(self):
        output = provenance.dependency_graph(sample(), DIGEST.upper())
        self.assertEqual(output["exe_sha256"], DIGEST)
        self.assertEqual(output["packages"][output["root"]]["name"], "paperroom")
        names = {record["name"]: record for record in output["packages"]}
        self.assertNotIn("dev", names)
        self.assertNotIn("orphan", names)
        self.assertEqual(names["derive"]["roles"], ["host-build"])
        self.assertEqual(names["host-child"]["roles"], ["host-build"])
        self.assertEqual(names["cc"]["roles"], ["host-build"])
        self.assertEqual(names["shared"]["roles"], ["host-build", "target-normal"])
        identical_names = [record for record in output["packages"] if record["name"] == "same"]
        self.assertEqual({r["source"] for r in identical_names}, {"crates.io", "other-registry"})
        other = next(r for r in identical_names if r["source"] == "other-registry")
        self.assertEqual(other["license_file_basename"], "LICENSE.custom")
        edges = [
            (output["packages"][a]["name"], output["packages"][b]["name"], arole, brole, kind)
            for a, b, arole, brole, kind in output["edges"]
        ]
        self.assertIn(("paperroom", "cc", "target-normal", "host-build", "build"), edges)
        self.assertIn(("cc", "shared", "host-build", "host-build", "normal"), edges)
        self.assertIn(("same", "shared", "target-normal", "target-normal", "normal"), edges)
        encoded = json.dumps(output)
        for excluded in [PRIVATE, "private-user", "registry+", "manifest_path", "target_directory"]:
            self.assertNotIn(excluded, encoded)

    def test_reports_are_deterministic_and_fail_for_ambiguous_or_scoped_root(self):
        data = sample()
        expected = provenance.dependency_graph(data, DIGEST)
        data["packages"].reverse()
        data["resolve"]["nodes"].reverse()
        self.assertEqual(provenance.dependency_graph(data, DIGEST), expected)
        original = data["resolve"]["root"]
        data["resolve"]["root"] = "registry"
        with self.assertRaises(ValueError):
            provenance.dependency_graph(data, DIGEST)
        data["resolve"]["root"] = original
        bad = next(record for record in data["packages"] if record["id"] == original)
        bad["source"] = REGISTRY
        with self.assertRaises(ValueError):
            provenance.dependency_graph(data, DIGEST)
        with self.assertRaises(ValueError):
            provenance.dependency_graph(sample(), "not-a-digest")

    def test_executable_bound_cli_and_rust_notice_hashes(self):
        with tempfile.TemporaryDirectory(prefix="private-user-") as path:
            root = Path(path)
            notices = root / "share" / "doc" / "rust"
            notices.mkdir(parents=True)
            text = b"synthetic notice\n"
            (notices / "COPYRIGHT-library.html").write_bytes(text)
            (notices / "COPYRIGHT.html").write_bytes(b"synthetic full notice\n")
            toolchain = root / "rustc.txt"
            toolchain.write_text(
                "rustc 1.100.0\nrelease: 1.100.0\n"
                "host: x86_64-pc-windows-msvc\ncommit-hash: " + "8" * 40
                + "\ncommit-date: 2026-11-24\nLLVM version: 22.1.0\n"
                + "binary: /private-user/rustc\n", encoding="utf-8",
            )
            run = subprocess.run(
                [sys.executable, "-B", str(SCRIPT), "--metadata", "-", "--exe-sha256", DIGEST,
                 "--rustc-info", str(toolchain), "--rust-sysroot", str(root)],
                check=True, input=json.dumps(sample()).encode(), capture_output=True,
            )
            self.assertNotIn(b"private-user", run.stdout)
            output = json.loads(run.stdout)
            self.assertEqual(output["exe_sha256"], DIGEST)
            self.assertEqual(output["rustc"]["host"], provenance.TARGET)
            self.assertEqual(output["rustc"]["release"], "1.100.0")
            notices = output["rust_toolchain_notices"]
            self.assertTrue(notices["copyright_library_observed"])
            self.assertEqual(notices["available"][0]["sha256"], hashlib.sha256(text).hexdigest())

    def test_missing_and_overlarge_notice_evidence(self):
        with tempfile.TemporaryDirectory() as root:
            rows = provenance.rust_notices(root)
            self.assertFalse(rows["copyright_library_observed"])
            self.assertEqual(rows["available"], [])
        with self.assertRaises(ValueError):
            provenance.bounded(io.BytesIO(b"abc"), 2)


if __name__ == "__main__":
    unittest.main()
