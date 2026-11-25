import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from zipfile import ZipFile

SCRIPT = Path(__file__).with_name("verify-windows-notice-bundle.py")
SPEC = importlib.util.spec_from_file_location("notices", SCRIPT)
verify = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(verify)
COMMIT = "a0" * 20


def raw_json(value):
    return json.dumps(value, sort_keys=True).encode("utf-8")


class NoticeBundleTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.folder = Path(self.temp.name)
        self.notices = {
            name: (verify.ROOT / "docs" / "third-party" / "pdf-assets" / name).read_bytes()
            for name in verify.EMBEDDED
        }
        self.exe = b"MZ\x00synthetic-test\x00" + b"\x00".join(self.notices.values())
        self.exe_sha = verify.sha(self.exe)
        self.rust = b"synthetic crate copyright"
        receipt = raw_json({
            "schema_version": 1, "exe_sha256": self.exe_sha, "target": verify.TARGET,
            "rustc": {"host": verify.TARGET},
            "packages": [{"name": "paperroom", "source": "workspace"}], "root": 0,
            "rust_toolchain_notices": {
                "available": [{"filename": "COPYRIGHT-library.html", "sha256": verify.sha(self.rust)}],
            },
        })
        self.receipt = receipt
        self.receipt_sha = verify.sha(receipt)
        self.index = {
            "schema_version": 1, "source_commit": COMMIT, "exe_sha256": self.exe_sha,
            "provenance_sha256": self.receipt_sha, "notices": [],
        }
        self.members = {
            verify.EXE: self.exe,
            verify.GUIDE: f"Candidate SHA-256 {self.exe_sha}".encode(),
        }
        for name, body in self.notices.items():
            self.add_notice("licenses/pdf-assets/" + name, body, "embedded-pdf-notice")
        self.add_notice(verify.RUST_NOTICE, self.rust, "rust-toolchain-notice")

    def add_notice(self, name, data, role):
        self.index["notices"].append({"path": name, "sha256": verify.sha(data), "role": role})
        self.members[name] = data

    def write_zip(self):
        path = self.folder / "windows.zip"
        raw = raw_json(self.index)
        with ZipFile(path, "w") as archive:
            archive.writestr(verify.INDEX, raw)
            for p, data in self.members.items():
                archive.writestr(p, data)
        return path, verify.sha(raw)

    def validate(self):
        path, index_sha = self.write_zip()
        return verify.verify_windows_zip(path, index_sha, COMMIT, self.exe_sha,
                                         self.receipt_sha, verify.sha(self.rust))

    def test_exact_embedded_and_shipped_evidence(self):
        found = verify.verify_embedded(self.exe, verify.ROOT / "docs" / "third-party" / "pdf-assets")
        self.assertEqual(set(found), set(verify.EMBEDDED))
        normalized = self.exe.replace(b"\r\n", b"\n").replace(b"\n", b"\r\n")
        self.assertEqual(set(verify.verify_embedded(normalized, verify.ROOT / "docs" / "third-party" / "pdf-assets")),
                         set(verify.EMBEDDED))
        self.assertEqual(verify.verify_receipt(self.receipt, self.receipt_sha, self.exe_sha), (1, verify.sha(self.rust)))
        self.index["link_markers"] = {
            "exe_sha256": self.exe_sha, "map_sha256": "ab" * 32,
            "symbol_rows": {"libcmt": 7, "libucrt": 0, "libvcruntime": 6},
        }
        verified = self.validate()
        self.assertEqual(verified["listed_notice_count"], 5)
        self.assertTrue(verified["rust_copyright_library_match"])
        self.assertIn("private MAP not examined", verified["link_markers"]["status"])

    def test_omitted_or_tampered_notice_and_wrong_exe_fail(self):
        name = next(iter(self.notices))
        root = verify.ROOT / "docs" / "third-party" / "pdf-assets"
        with self.assertRaisesRegex(ValueError, "lacks the complete"):
            verify.verify_embedded(self.exe.replace(self.notices[name], b"not present"), root)
        path = "licenses/pdf-assets/" + name
        original = self.members[path]
        self.members[path] = b"same index, wrong file"
        with self.assertRaisesRegex(ValueError, "shipped notice bytes"):
            self.validate()
        self.members[path] = original
        self.members[verify.EXE] = b"MZother executable"
        with self.assertRaisesRegex(ValueError, "different candidate EXE"):
            self.validate()
        self.members[verify.EXE] = self.exe
        del self.members[path]
        self.index["notices"] = [row for row in self.index["notices"] if row["path"] != path]
        with self.assertRaisesRegex(ValueError, "omits or misidentifies"):
            self.validate()

    def test_extra_path_and_receipt_or_index_commit_mismatches_fail(self):
        with self.assertRaisesRegex(ValueError, "recorded digest"):
            verify.verify_receipt(self.receipt, "42" * 32, self.exe_sha)
        with self.assertRaisesRegex(ValueError, "another candidate"):
            verify.verify_receipt(self.receipt, self.receipt_sha, "42" * 32)
        self.index["source_commit"] = "42" * 20
        with self.assertRaisesRegex(ValueError, "not bound"):
            self.validate()
        self.index["source_commit"] = COMMIT
        for unsafe in ("licenses/../outside.txt", "licenses/AUX.txt", "licenses/NOTICE-index.json"):
            self.members[unsafe] = b"unexpected"
            with self.assertRaisesRegex(ValueError, "unsafe, duplicate"):
                self.validate()
            del self.members[unsafe]
        self.members["licenses/extra.txt"] = b"not reviewed"
        with self.assertRaisesRegex(ValueError, "differs from the reviewed index"):
            self.validate()

    def test_receipt_only_cli_reports_absence_and_does_not_execute_candidate(self):
        exe = self.folder / "synthetic.exe"
        exe.write_bytes(self.exe)
        receipt = self.folder / "provenance.json"
        receipt.write_bytes(self.receipt)
        run = subprocess.run(
            [sys.executable, "-B", str(SCRIPT), "--exe", str(exe),
             "--exe-sha256", self.exe_sha, "--provenance", str(receipt),
             "--provenance-sha256", self.receipt_sha, "--commit", COMMIT],
            check=True, capture_output=True,
        )
        report = json.loads(run.stdout)
        self.assertEqual(report["source_commit"], COMMIT)
        self.assertEqual(report["shipped_windows_zip"]["status"], "not supplied")
        self.assertFalse(report["cargo_resolution"]["linked_inventory"])
        self.assertNotIn(str(self.folder), run.stdout.decode())

    def test_qa_stage_checks_before_publish_and_refuses_overwrite(self):
        root = verify.ROOT / "docs" / "third-party" / "pdf-assets"
        out = self.folder / "staged-only.zip"
        result = verify.stage_qa_zip(out, self.exe, COMMIT, self.exe_sha, self.receipt_sha,
                                     verify.sha(self.rust), root)
        data = out.read_bytes()
        self.assertEqual(result["windows_zip_sha256"], verify.sha(data))
        self.assertTrue(result["qa_only"])
        self.assertFalse(result["notice_scope_complete"])
        self.assertFalse(result["rust_copyright_library_match"])
        with ZipFile(out) as z:
            self.assertIn(b"Do not publish", z.read(verify.GUIDE))
        with self.assertRaisesRegex(ValueError, "already exists"):
            verify.stage_qa_zip(out, self.exe, COMMIT, self.exe_sha, self.receipt_sha, None, root)
        self.assertEqual(out.read_bytes(), data)
        wrong = self.folder / "invalid.zip"
        with self.assertRaisesRegex(ValueError, "different candidate EXE"):
            verify.stage_qa_zip(wrong, b"wrong", COMMIT, self.exe_sha, self.receipt_sha, None, root)
        self.assertFalse(wrong.exists())
        self.assertFalse(list(self.folder.glob(".paperroom-qa-*")))

    def test_qa_stage_checks_the_exact_rust_library_notice_before_writing(self):
        root = verify.ROOT / "docs" / "third-party" / "pdf-assets"
        notice = self.folder / "COPYRIGHT-library.html"
        notice.write_bytes(self.rust)
        out = self.folder / "with-rust.zip"
        result = verify.stage_qa_zip(out, self.exe, COMMIT, self.exe_sha, self.receipt_sha,
                                     verify.sha(self.rust), root, notice)
        self.assertTrue(result["rust_copyright_library_match"])
        self.assertFalse(result["notice_scope_complete"])
        self.assertEqual(result["listed_notice_count"], 5)
        with ZipFile(out) as z:
            self.assertEqual(z.read(verify.RUST_NOTICE), self.rust)
            index = json.loads(z.read(verify.INDEX))
            self.assertIn({"path": verify.RUST_NOTICE, "role": "rust-toolchain-notice",
                           "sha256": verify.sha(self.rust)}, index["notices"])

        missing = self.folder / "missing-receipt-hash.zip"
        with self.assertRaisesRegex(ValueError, "does not record"):
            verify.stage_qa_zip(missing, self.exe, COMMIT, self.exe_sha, self.receipt_sha,
                                None, root, notice)
        self.assertFalse(missing.exists())
        notice.write_bytes(self.rust + bytes([10]))
        wrong = self.folder / "wrong-rust.zip"
        with self.assertRaisesRegex(ValueError, "does not match the pinned receipt"):
            verify.stage_qa_zip(wrong, self.exe, COMMIT, self.exe_sha, self.receipt_sha,
                                verify.sha(self.rust), root, notice)
        self.assertFalse(wrong.exists())


if __name__ == "__main__":
    unittest.main()
