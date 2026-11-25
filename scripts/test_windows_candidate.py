"""Regression coverage for the local release check. Run with Python 3."""

import argparse
import contextlib
import hashlib
import io
from pathlib import Path, PurePosixPath
import runpy
import struct
import subprocess
import tempfile
import unittest
from zipfile import ZipFile


ROOT = Path(__file__).resolve().parent.parent
CANDIDATE = runpy.run_path(str(ROOT / "scripts" / "windows-candidate.py"))


def sha256(contents):
    return hashlib.sha256(contents).hexdigest()


class PreflightTest(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix="paperroom-preflight-test-")
        self.addCleanup(temp.cleanup)
        self.directory = Path(temp.name)
        self.exe = self.directory / "Paperroom.exe"
        self.windows = self.directory / "Paperroom-Windows-x64.zip"
        self.source = self.directory / "Paperroom-source.zip"
        self.reference = self.directory / "reviewed-notices.zip"
        self.manifest = self.directory / "SHA256SUMS.txt"
        self.commit = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
        ).strip()
        self.source.write_bytes(subprocess.check_output(
            ["git", "-c", "core.autocrlf=false", "-c", "core.eol=lf",
             "archive", "--format=zip", "--prefix=paperroom-0.1.2/", self.commit], cwd=ROOT,
        ))
        with ZipFile(self.reference, "w") as archive:
            archive.writestr("licenses/TEST.txt", "Synthetic notice for the preflight test.")
        pe = bytearray(128)
        pe[:2] = b"MZ"
        struct.pack_into("<I", pe, 0x3C, 64)
        pe[64:68] = b"PE\0\0"
        struct.pack_into("<H", pe, 68, 0x8664)
        struct.pack_into("<H", pe, 88, 0x20B)
        self.exe.write_bytes(pe)
        self.write_windows_zip()
        self.write_manifest()

    def write_windows_zip(self, bundled=None, extra=None):
        exe = self.exe.read_bytes()
        with ZipFile(self.windows, "w") as archive, ZipFile(self.reference) as notices:
            archive.writestr("Paperroom.exe", exe if bundled is None else bundled)
            archive.writestr("README.zh-Hant.md", f"Windows test candidate {sha256(exe)}")
            archive.writestr("licenses/TEST.txt", notices.read("licenses/TEST.txt"))
            if extra is not None:
                archive.writestr(extra, "synthetic metadata")

    def write_manifest(self):
        self.manifest.write_text(
            "".join(f"{sha256(asset.read_bytes())}  {asset.name}\n"
                    for asset in (self.exe, self.windows, self.source)),
            encoding="utf-8",
        )

    def run_preflight(self, tested=None):
        args = argparse.Namespace(
            exe=self.exe, windows=self.windows, source=self.source, reference_notices=self.reference,
            manifest=self.manifest, commit=self.commit, source_prefix="paperroom-0.1.2", prefix=[],
            tested_exe_sha256=sha256(self.exe.read_bytes()) if tested is None else tested,
        )
        with contextlib.redirect_stdout(io.StringIO()):
            CANDIDATE["preflight"](args)

    def test_exact_assets_pass(self):
        self.run_preflight()

    def test_source_prefixes_keep_specific_paths_not_generic_parents(self):
        prefixes = CANDIDATE["source_private_prefixes"]
        self.assertEqual(prefixes(PurePosixPath("/tmp/checkout")), ["/tmp/checkout"])
        self.assertEqual(
            prefixes(PurePosixPath("/tmp/specific/checkout")),
            ["/tmp/specific/checkout", "/tmp/specific"],
        )
        self.assertEqual(prefixes(PurePosixPath("/checkout")), ["/checkout"])
        self.assertTrue(CANDIDATE["hits"](b"build from /tmp/checkout/src/main.rs", prefixes(PurePosixPath("/tmp/checkout"))))
        self.assertFalse(CANDIDATE["hits"](b"generic example /tmp/file", prefixes(PurePosixPath("/tmp/checkout"))))

    def test_source_zip_missing_static_crt_config_fails_with_matching_manifest(self):
        path = "paperroom-0.1.2/.cargo/config.toml"
        with ZipFile(io.BytesIO(self.source.read_bytes())) as canonical:
            self.assertEqual(canonical.namelist().count(path), 1)
            with ZipFile(self.source, "w") as omitted:
                omitted.comment = canonical.comment
                for entry in canonical.infolist():
                    if entry.filename != path:
                        omitted.writestr(entry, canonical.read(entry))
        self.write_manifest()
        with self.assertRaisesRegex(ValueError, "Source ZIP does not match the specified commit"):
            self.run_preflight()

    def test_unrelated_acceptance_hash_fails(self):
        with self.assertRaisesRegex(ValueError, "acceptance testing"):
            self.run_preflight(tested="0" * 64)

    def test_wrong_checksum_and_missing_entry_fail(self):
        manifest = self.manifest.read_text(encoding="utf-8")
        lines = manifest.splitlines()
        self.manifest.write_text("\n".join([*lines[:2], f"{'0' * 64}  {self.source.name}"]))
        with self.assertRaisesRegex(ValueError, "does not match the SHA-256"):
            self.run_preflight()
        self.manifest.write_text("\n".join(lines[:2]), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "all three assets"):
            self.run_preflight()
        self.manifest.write_text(manifest + lines[0] + "\n", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "repeated filename"):
            self.run_preflight()

    def test_zip_executable_must_equal_standalone(self):
        self.write_windows_zip(bundled=self.exe.read_bytes() + b"changed")
        self.write_manifest()
        with self.assertRaisesRegex(ValueError, "executable differs"):
            self.run_preflight()

    def test_unsafe_member_fails_even_with_a_matching_manifest(self):
        self.write_windows_zip(extra="../user-data.txt")
        self.write_manifest()
        with self.assertRaisesRegex(ValueError, "unsafe"):
            self.run_preflight()

    def test_host_path_in_wide_string_fails(self):
        host_path = "C:\\Users\\synthetic-release-fixture\\data.txt".encode("utf-16le")
        self.exe.write_bytes(self.exe.read_bytes() + host_path)
        self.write_windows_zip()
        self.write_manifest()
        with self.assertRaisesRegex(ValueError, "contains forbidden data"):
            self.run_preflight()


if __name__ == "__main__":
    unittest.main()
