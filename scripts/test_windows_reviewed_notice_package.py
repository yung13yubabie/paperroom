import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from zipfile import ZipFile

SCRIPT = Path(__file__).with_name("windows-reviewed-notice-package.py")
spec = importlib.util.spec_from_file_location("reviewed_package", SCRIPT)
pack = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pack)
COMMIT = "a0" * 20
VERSION = "0.1.2"


class ReviewedNoticePackage(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.doc = self.root / "share/doc/rust"
        (self.doc / "licenses").mkdir(parents=True)
        self.html = b"""<a href="#section">Section</a>
<a href="https://example.invalid">Remote text</a>
<a href="licenses/MIT.txt">Local MIT</a>
<a href="./licenses/Apache-2.0.txt">Local Apache</a>
<a href="licenses/MIT.txt">Same link</a>"""
        (self.doc / "COPYRIGHT-library.html").write_bytes(self.html)
        (self.doc / "licenses/MIT.txt").write_bytes(b"original MIT fixture")
        (self.doc / "licenses/Apache-2.0.txt").write_bytes(b"original Apache fixture")
        (self.doc / "licenses/unreferenced.txt").write_bytes(b"not in the notice")
        pe = bytearray(100)
        pe[:2] = b"MZ"
        pe[60:64] = (64).to_bytes(4, "little")
        pe[64:70] = b"PE\0\0d\x86"
        pe[88:90] = b"\x0b\x02"
        pdf_root = pack.notice.ROOT / "docs/third-party/pdf-assets"
        pe.extend(b"\0".join((pdf_root / name).read_bytes() for name in pack.notice.EMBEDDED))
        self.exe = self.root / "Paperroom.exe"
        self.exe.write_bytes(pe)
        self.exe_sha = pack.notice.sha(pe)
        self.receipt = self.root / "provenance.json"
        self.receipt.write_bytes(pack.json_bytes({
            "schema_version": 1, "exe_sha256": self.exe_sha, "target": pack.notice.TARGET,
            "rustc": {"host": pack.notice.TARGET, "release": pack.RUST_RELEASE},
            "root": 0,
            "packages": [{"name": "paperroom", "source": "workspace", "version": VERSION}],
            "rust_toolchain_notices": {"available": [{
                "filename": "COPYRIGHT-library.html", "sha256": pack.notice.sha(self.html),
            }]},
        }))
        self.dependency = self.root / "dependency-inventory.tsv"
        self.dependency.write_bytes(pack.DEPENDENCY_HEADER + b"\n")
        self.link = self.root / "sanitized-link.json"
        self.link.write_bytes(pack.json_bytes({
            "schema_version": 1, "exe_sha256": self.exe_sha, "source_commit": COMMIT,
            "map_sha256": "01" * 32, "verbose_stdout_sha256": "02" * 32,
            "map_header_matches_exe": True, "recognized_containers": [],
            "map_observations": {"rows": []}, "verbose_selections": {"rows": []},
        }))
        self.zip = self.root / "notice.zip"

    def expected(self, include_link=True):
        return pack.identity(COMMIT, VERSION, self.exe_sha, pack.notice.sha(self.receipt.read_bytes()),
                             pack.notice.sha(self.dependency.read_bytes()),
                             pack.notice.sha(self.link.read_bytes()) if include_link else None)

    def build(self, include_link=True):
        return pack.build_package(self.zip, self.exe, self.receipt, self.dependency, self.root, self.expected(include_link),
                                  self.link if include_link else None)

    def test_round_trip_binds_binary_toolchain_and_local_links(self):
        report = self.build()
        self.assertEqual(report["rust_relative_license_count"], 2)
        self.assertTrue(report["link_inventory_included"])
        self.assertFalse(report["notice_scope_complete"])
        with ZipFile(self.zip) as archive:
            self.assertEqual(archive.read(pack.RUST_HTML), self.html)
            self.assertEqual(archive.read(pack.RUST_LINK_ROOT + "licenses/MIT.txt"), b"original MIT fixture")
            self.assertEqual(archive.read(pack.RUST_LINK_ROOT + "licenses/Apache-2.0.txt"), b"original Apache fixture")
            self.assertNotIn(pack.RUST_LINK_ROOT + "licenses/unreferenced.txt", archive.namelist())
            self.assertEqual(archive.read(pack.PROVENANCE), self.receipt.read_bytes())
            self.assertEqual(archive.read(pack.DEPENDENCY_INVENTORY), self.dependency.read_bytes())
            self.assertEqual(archive.read(pack.LINK_INVENTORY), self.link.read_bytes())
        self.assertEqual(pack.verify_package(self.zip, report["package_sha256"], report["notice_index_sha256"],
                                             self.expected()), report)
        with self.assertRaisesRegex(ValueError, "output already"):
            self.build()
        self.assertEqual(list(self.root.glob(".paperroom-notices-*")), [])

    def test_download_digest_missing_link_changed_bytes_and_unsafe_members_fail(self):
        report = self.build(False)
        with self.assertRaisesRegex(ValueError, "expected download identity"):
            pack.verify_package(self.zip, "aa" * 32, report["notice_index_sha256"], self.expected(False))
        with ZipFile(self.zip) as archive:
            members = {entry.filename: archive.read(entry) for entry in archive.infolist()}
        mit = pack.RUST_LINK_ROOT + "licenses/MIT.txt"
        for changed, error in [
            ({name: raw for name, raw in members.items() if name != mit}, "unresolved Rust"),
            ({**members, mit: b"changed bytes"}, "file bytes and identities"),
            ({**members, "licenses/Rust-Toolchain/licenses/mit.txt": b"extra"}, "unsafe"),
            ({**members, "licenses/msvc/vcruntime.dll": b"extra"}, "unsafe"),
        ]:
            with ZipFile(self.zip, "w") as archive:
                for name, raw in changed.items():
                    archive.writestr(name, raw)
            with self.assertRaisesRegex(ValueError, error):
                pack.verify_package(self.zip, pack.notice.sha(self.zip.read_bytes()),
                                    report["notice_index_sha256"], self.expected(False))

    def test_build_requires_real_receipt_matched_html_and_installed_targets(self):
        (self.doc / "licenses/MIT.txt").unlink()
        with self.assertRaises(FileNotFoundError):
            self.build()
        self.assertFalse(self.zip.exists())
        (self.doc / "licenses/MIT.txt").write_bytes(b"original MIT fixture")
        (self.doc / "COPYRIGHT-library.html").write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "Rust library copyright"):
            self.build()
        self.assertFalse(self.zip.exists())
        (self.doc / "COPYRIGHT-library.html").write_bytes(self.html)
        self.link.write_bytes(pack.json_bytes({"schema_version": 1, "source_commit": "ae" * 20,
                                               "exe_sha256": self.exe_sha}))
        with self.assertRaisesRegex(ValueError, "another candidate or source"):
            self.build()

    def test_rejects_unsafe_relative_link_spelling(self):
        for relative in ("../x.txt", "licenses/%2E%2E/x.txt", "licenses/NUL.txt", "licenses/example.txt?other",
                         "file:///C:/Windows/x.txt", "//arbitrary.example/license.txt"):
            with self.subTest(relative=relative), self.assertRaisesRegex(ValueError, "unsupported"):
                pack.relative_links(f'<a href="{relative}">text</a>'.encode())
        with self.assertRaisesRegex(ValueError, "base URL"):
            pack.relative_links(b'<base href="https://example.invalid/"><a href="licenses/MIT.txt">MIT</a>')
        with self.assertRaisesRegex(ValueError, "case-conflicting"):
            pack.relative_links(b'<a href="licenses/MIT.txt"></a><a href="licenses/Mit.txt"></a>')

    def test_registry_lane_checks_exact_reachable_scope_and_payload_bytes(self):
        receipt = pack.notice.parse(self.receipt.read_bytes())
        record = {
            "name": "crate_qa", "version": "1.2.3", "roles": ["host-build", "target-normal"],
            "license_expression": "MIT", "license_file_declared": True, "license_file_basename": "LICENSE.txt",
        }
        receipt["packages"].append({"source": "crates.io", **record})
        self.receipt.write_bytes(pack.json_bytes(receipt))
        root = self.root / "registry-notices"
        member_name = "licenses/crates-io/crate_qa/1.2.3/legal/LICENSE.txt"
        path = root / member_name
        path.parent.mkdir(parents=True)
        path.write_bytes(b"original legal fixture")
        registry_file = root / pack.REGISTRY_INDEX
        registry_file.parent.mkdir()
        manifest = {
            "schema_version": 1, "exe_sha256": self.exe_sha,
            "provenance_sha256": pack.notice.sha(self.receipt.read_bytes()),
            "cargo_lock_sha256": "aa" * 32, "target": pack.notice.TARGET, "scope": "Synthetic third-party scope",
            "packages": [{
                **record, "archive_sha256": "bb" * 32, "status": "collected",
                "notices": [{"path": member_name, "sha256": pack.notice.sha(path.read_bytes()), "bytes": path.stat().st_size}],
            }],
        }
        registry_file.write_bytes(pack.json_bytes(manifest))
        bound = {**self.expected(), "registry_notice_index_sha256": pack.notice.sha(registry_file.read_bytes())}
        found = pack.build_package(self.zip, self.exe, self.receipt, self.dependency, self.root, bound,
                                   self.link, root)
        self.assertEqual(found["registry_notice_file_count"], 1)
        self.assertEqual(pack.verify_package(self.zip, found["package_sha256"], found["notice_index_sha256"], bound), found)
        with ZipFile(self.zip) as archive:
            self.assertEqual(archive.read(member_name), b"original legal fixture")
        with self.assertRaisesRegex(ValueError, "unexpected members"):
            pack.verify_package(self.zip, found["package_sha256"], found["notice_index_sha256"], self.expected())
        self.zip.unlink()
        path.write_bytes(b"modified legal fixture")
        with self.assertRaisesRegex(ValueError, "Registry notice bytes"):
            pack.build_package(self.zip, self.exe, self.receipt, self.dependency, self.root, bound, self.link, root)
        self.assertFalse(self.zip.exists())
        manifest["packages"] = []
        registry_file.write_bytes(pack.json_bytes(manifest))
        bound["registry_notice_index_sha256"] = pack.notice.sha(registry_file.read_bytes())
        with self.assertRaisesRegex(ValueError, "does not account for every resolved"):
            pack.build_package(self.zip, self.exe, self.receipt, self.dependency, self.root, bound, self.link, root)

    def test_cli_paths_and_component_digests(self):
        bound = self.expected(False)
        same = ["--commit", COMMIT, "--version", VERSION, "--exe-sha256", self.exe_sha, "--provenance-sha256",
                bound["provenance_sha256"], "--dependency-inventory-sha256", bound["dependency_inventory_sha256"]]
        def cli(*args):
            return subprocess.run([sys.executable, "-B", str(SCRIPT), *args], capture_output=True, check=True, text=True)
        result = cli("build", *same, "--output", str(self.zip), "--exe", str(self.exe),
                     "--provenance", str(self.receipt), "--dependency-inventory", str(self.dependency),
                     "--rust-sysroot", str(self.root))
        report = pack.notice.parse(result.stdout)
        verified = cli("verify", *same, "--package", str(self.zip), "--package-sha256", report["package_sha256"],
                       "--notice-index-sha256", report["notice_index_sha256"])
        self.assertEqual(pack.notice.parse(verified.stdout), report)


if __name__ == "__main__":
    unittest.main()
