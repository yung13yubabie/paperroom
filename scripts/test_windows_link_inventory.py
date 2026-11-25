import importlib.util
import json
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
from unittest import TestCase, main, mock

SCRIPT = Path(__file__).with_name("windows-link-inventory.py")
SPEC = importlib.util.spec_from_file_location("inventory", SCRIPT)
inventory = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(inventory)
BASE = 0x140000000
STAMP = 0x7ABC1234
COMMIT = "af" * 20
PRIVATE_MAP = r"""
 paperroom
 Timestamp is 7abc1234 (synthetic)
 Preferred load address is 0000000140000000

   Start  Length  Name    Class
   0001:00000000 00000240H .text CODE

   Address     Publics by Value              Rva+Base       Lib:Object
   0001:00000010 _private_symbol_user       0000000140001010 f   LIBCMT:exe_main.obj
   0001:00000015 _private_symbol_user_two   0000000140001015 f i LIBCMT:exe_main.obj
   0001:00000022 sqlite_not_an_approved_row 0000000140001022 f   LOCAL:private-candidate.obj
   0001:00000032 rust_private_symbol        0000000140001032 f libstd-c837ab243cfe2402.rlib:std.cgu.0.o
   0001:00000040 unanchored_row             00000000      f i LIBUCRT:wrong.obj

   entry point at 0001:00000010

   Static symbols
   0001:00000019 _implementation_detail    0000000140001019 f libucrt.lib:memmove.obj
   0001:00000022 _known_import_stub        0000000140001022 f KERNEL32:KERNEL32.dll
"""
PRIVATE_TRACE = r"""
Searching libraries
  Searching C:\private-user\VC Tools\lib\LibCMT.lib:
    Found sensitive_resolved_symbol
    Referenced in C:\private-user\Paperroom\private-candidate.obj
    Loaded LIBCMT.lib(exe_main.obj)
  Searching "C:\private-user\Kits (x86)\lib\libucrt.lib":
    Loaded LIBUCRT.lib(memmove.obj)
    Loaded LIBUCRT.lib(dead-after-optimization.obj)
  Searching C:\private-user\Rust\lib\libstd-c837ab243cfe2402.rlib:
    Loaded libstd-c837ab243cfe2402.rlib(std.cgu.0.o)
  Searching C:\private-user\VC Tools\lib\kernel32.lib:
    Loaded KERNEL32.lib(KERNEL32.dll)
    Loaded unclassified-username.lib(private-name.obj)
    Loaded gibberish
Finished searching libraries
"""
ROOTS = [r"msvc=C:\private-user\VC Tools", r"sdk=C:\private-user\Kits (x86)",
         r"rust=C:\private-user\Rust", r"target=C:\private-user\Paperroom"]


def fake_pe():
    pe = bytearray(512)
    pe[:2] = b"MZ"
    struct.pack_into("<I", pe, 0x3C, 0x80)
    pe[0x80:0x84] = b"PE\0\0"
    struct.pack_into("<HHI", pe, 0x84, 0x8664, 1, STAMP)
    struct.pack_into("<H", pe, 0x94, 0xF0)
    opt = 0x98
    struct.pack_into("<H", pe, opt, 0x20B)
    struct.pack_into("<Q", pe, opt + 24, BASE)
    struct.pack_into("<I", pe, opt + 56, 0x5000)
    return bytes(pe)


class InventoryTests(TestCase):
    def test_observations_use_known_families_and_do_not_emit_paths_symbols(self):
        exe = fake_pe()
        with mock.patch.object(inventory, "archive_hash", return_value="a1" * 32) as hash_call:
            report = inventory.build_report(
                exe, inventory.sha(exe), COMMIT, PRIVATE_MAP.encode("utf-16"), PRIVATE_TRACE.encode(), b"",
                inventory.parse_roots(ROOTS), {"cl_version": "19.44.35109",
                    "vs_product_id": "Microsoft.VisualStudio.Product.Enterprise"},
            )
        retained = {item["member"]: item for item in report["map_observations"]["rows"]}
        chosen = {item["member"]: item for item in report["verbose_selections"]["rows"]}
        self.assertEqual(retained["exe_main.obj"]["symbol_rows"], 2)
        self.assertEqual(retained["memmove.obj"]["table"], "static")
        self.assertEqual(retained["std.cgu.0.o"]["family"], "std")
        self.assertTrue(retained["kernel32.dll"]["also_observed_loaded"])
        self.assertNotIn("dead-after-optimization.obj", retained)
        self.assertEqual(chosen["dead-after-optimization.obj"]["origin"], "sdk")
        self.assertEqual(chosen["exe_main.obj"]["origin"], "msvc")
        self.assertEqual(chosen["std.cgu.0.o"]["origin"], "rust")
        self.assertEqual(report["map_observations"]["unattributed_symbol_rows"], 2)
        self.assertEqual(report["verbose_selections"]["unattributed_loaded_rows"], 2)
        self.assertEqual(len(report["recognized_containers"]), 4)
        self.assertEqual(hash_call.call_count, 4)
        self.assertEqual(report["toolchain"]["cl_version"], "19.44.35109")
        encoded = json.dumps(report)
        for secret in ["private-user", "private_symbol", "implementation_detail",
                       "unclassified-username", "resolved_symbol", "candidate.obj"]:
            self.assertNotIn(secret, encoded)

    def test_no_cross_stream_or_case_collision_origin_attribution(self):
        rows, *_ = inventory.parse_verbose([
            r"Searching C:\private-user\VC Tools\lib\libucrt.lib:",
            "Loaded LIBUCRT.lib(memmove.obj)",
            r"Searching C:\private-user\VC Tools\lib\libucrt.lib:" +
            "\nFinished searching libraries\nLoaded LIBUCRT.lib(stale.obj)",
            "Searching C:\\private-user\\VC Tools Other\\libucrt.lib:\nLoaded LIBUCRT.lib(other.obj)",
        ], inventory.parse_roots(ROOTS))
        self.assertEqual(len(rows), 3)
        self.assertTrue(all(row[-1] == "unresolved" for row in rows))
        for value in [r"\\server\share\kit", r"C:\safe\..\other", r"C:\safe\lib.exe:stream", "C:\\"]:
            with self.assertRaises(ValueError):
                inventory.parse_roots(["sdk=" + value])

    def test_map_mismatch_exe_mismatch_and_private_version_fail(self):
        exe = fake_pe()
        with self.assertRaisesRegex(ValueError, "pinned PE digest"):
            inventory.pe_identity(exe, "42" * 32)
        for changed in (PRIVATE_MAP.replace("7abc1234", "7abc1235"),
                        PRIVATE_MAP.replace("0000000140000000", "0000000150000000"),
                        PRIVATE_MAP + "\nTimestamp is 7abc1234"):
            with self.assertRaisesRegex(ValueError, "MAP timestamp"):
                inventory.parse_map(changed, (STAMP, BASE, 0x5000))
        for fields in [{"cl_version": "/private-user/cl.exe"}, {"vs_product_id": "user-data"}, {"sdk_version": "10."}]:
            with self.assertRaises(ValueError):
                inventory.check_toolchain(fields)

    def test_pinned_registry_candidates_and_sqlite_do_not_claim_exact_versions(self):
        exe = fake_pe()
        lock = b"""version=4
[[package]]
name="png-example"
version="0.18.0"
source="registry+https://github.com/rust-lang/crates.io-index"
[[package]]
name="png-example"
version="0.18.1"
source="sparse+https://index.crates.io/"
[[package]]
name="private-png"
version="1.0.0"
source="registry+https://github.com/rust-lang/crates.io-index@private.example"
"""
        png_row = b"\n0001:00000050 _private 0000000140001050 f libpng_example-124578abcd.rlib:png.obj"
        trace = (PRIVATE_TRACE + r"""
Loaded libpng_example-124578abcd.rlib(png.obj)
Loaded sqlite3.lib(sqlite3.obj)
Loaded libprivate_png-124578abcd.rlib(private.obj)
""").encode()
        report = inventory.build_report(exe, inventory.sha(exe), COMMIT,
            PRIVATE_MAP.encode() + png_row, trace, b"", {}, cargo_lock=lock)
        self.assertEqual(report["cargo_lock_sha256"], inventory.sha(lock))
        self.assertEqual(report["observed_cargo_registry_candidates"], [{
            "family": "png_example", "possible_packages": [
                {"name": "png-example", "version": "0.18.0"},
                {"name": "png-example", "version": "0.18.1"},
            ],
        }])
        self.assertTrue(any(row["kind"] == "cargo-registry-archive"
                            for row in report["map_observations"]["rows"]))
        self.assertTrue(any(row["kind"] == "sqlite-native-archive"
                            for row in report["verbose_selections"]["rows"]))
        self.assertNotIn("private.example", json.dumps(report))
        self.assertNotIn("private_png", json.dumps(report))
        candidates = {"memchr": {("memchr", "2.7.6")}}
        self.assertEqual(inventory.family("libmemchr-124578abcd.rlib", candidates),
                         ("memchr", "rust-or-cargo-archive"))
        self.assertEqual(inventory.family("libmemchr-124578abcd.rlib", candidates, "rust"),
                         ("memchr", "rust-toolchain-archive"))
        self.assertEqual(inventory.family("libmemchr-124578abcd.rlib", candidates, "target"),
                         ("memchr", "cargo-registry-archive"))


    def test_bomless_windows_encodings_and_trace_shapes_remain_safe(self):
        exe = fake_pe()
        trace = ("\x1b[01;32mSearching C:\\private-user\\VC Tools\\lib\\LIBCMT.lib:\x1b[0m\r\n"
                 " Found secret-symbol-identity\r\n"
                 " Referenced in private-object.obj\r\n"
                 " Loaded LIBCMT.lib    (exe_main.obj)\r\n"
                 " Loaded LIBCMT.lib : second.obj\r\n"
                 " Loaded unexpected shape\r\n"
                 "Finished searching libraries\r\n")
        for encoding in ("utf-16-le", "utf-16-be"):
            raw = trace.encode(encoding)
            report = inventory.build_report(exe, inventory.sha(exe), COMMIT,
                PRIVATE_MAP.encode(encoding), raw, b"", inventory.parse_roots(ROOTS))
            selections = report["verbose_selections"]
            self.assertEqual(selections["trace_streams"]["stdout"]["encoding"], encoding + "-without-bom")
            self.assertEqual(selections["trace_streams"]["stdout"]["byte_count"], len(raw))
            self.assertEqual(selections["trace_streams"]["stderr"]["encoding"], "empty")
            stats = selections["trace_marker_counts"]
            self.assertEqual(stats["ansi_color_sequences_removed"], 2)
            self.assertEqual(stats["loaded_literal_rows"], 3)
            self.assertEqual(stats["loaded_colon_format_rows"], 1)
            self.assertEqual(stats["loaded_parenthesized_format_rows"], 1)
            self.assertEqual(stats["referenced_in_marker_rows"], 1)
            self.assertEqual(selections["attributed_loaded_rows"], 2)
            self.assertEqual(selections["unattributed_loaded_rows"], 1)
            encoded = json.dumps(report)
            for private in ["private-user", "secret-symbol-identity", "private-object.obj"]:
                self.assertNotIn(private, encoded)
        self.assertEqual(inventory.decode_with_encoding("Searching libraries".encode())[1], "utf-8")

    def test_release_cli_refuses_bad_map_and_never_outputs_private_paths(self):
        exe = fake_pe()
        with tempfile.TemporaryDirectory(prefix="private-user-") as dir:
            files = Path(dir)
            (files / "app.exe").write_bytes(exe)
            (files / "real.map").write_text(PRIVATE_MAP)
            (files / "stdout.txt").write_text(PRIVATE_TRACE)
            cmd = [
                sys.executable, "-B", str(SCRIPT), "--exe", str(files / "app.exe"),
                "--exe-sha256", inventory.sha(exe), "--commit", COMMIT,
                "--map", str(files / "real.map"), "--verbose", str(files / "stdout.txt"),
                "--sdk-version", "10.0.26100.0",
            ]
            run = subprocess.run(cmd, capture_output=True, check=True)
            report = json.loads(run.stdout)
            self.assertEqual(report["exe_sha256"], inventory.sha(exe))
            self.assertEqual(report["source_commit"], COMMIT)
            self.assertEqual(report["map_observations"]["status"], "observed")
            self.assertNotIn(b"private-user", run.stdout)
            (files / "real.map").write_text("not a MAP with header")
            failed = subprocess.run(cmd, capture_output=True)
            self.assertEqual(failed.returncode, 2)
            self.assertFalse(failed.stdout)
            self.assertNotIn(b"private-user", failed.stderr)


if __name__ == "__main__":
    main()
