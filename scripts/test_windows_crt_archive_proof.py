import importlib.util
from io import BytesIO
import json
from pathlib import Path
import struct
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("windows-crt-archive-proof.py")
spec = importlib.util.spec_from_file_location("crt_archive_proof", SCRIPT)
proof = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proof)
VC = "14.44.35207"
SDK = "10.0.26100.0"
VC_DIR = r"C:\private-install\VC\Tools\MSVC" + "\\" + VC
SDK_DIR = r"C:\private-install\Windows Kits\10"
COMMIT = "ab" * 20


def member(name, body):
    assert len(name) <= 16
    header = (name.ljust(16) + b"0".ljust(12) + b"".ljust(6) + b"".ljust(6)
              + b"100644".ljust(8) + str(len(body)).encode().ljust(10) + b"`\n")
    return header + body + (b"\n" if len(body) & 1 else b"")


def coff(short=(), long=()):
    source = proof.COFF_SIGNATURE + member(b"/", bytes(4)) + member(b"/", bytes(8))
    if long:
        names, offsets = bytearray(), []
        for name in long:
            offsets.append(len(names))
            names.extend(name + b"\0")
        source += member(b"//", bytes(names))
        source += b"".join(member(f"/{offset}".encode(), b"object") for offset in offsets)
    return source + b"".join(member(name + b"/", b"obj") for name in short)


def candidate():
    data = bytearray(512)
    header = 0x80
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 0x3C, header)
    data[header:header+4] = b"PE\0\0"
    struct.pack_into("<HHI", data, header+4, 0x8664, 1, 0x1234)
    struct.pack_into("<H", data, header+20, 0xF0)
    struct.pack_into("<H", data, header+24, 0x020B)
    struct.pack_into("<Q", data, header+48, 0x140000000)
    struct.pack_into("<I", data, header+80, 0x2000)
    return bytes(data)


def map_row(family, member_name, count=1, table="public"):
    return {"family": family, "kind": "msvc-crt", "member": member_name, "symbol_rows": count, "table": table}


def inventory(exe, stdout, stderr, rows):
    return json.dumps({
        "schema_version": 1, "source_commit": COMMIT, "exe_sha256": proof.link.sha(exe),
        "map_header_matches_exe": True, "map_sha256": "1a" * 32,
        "verbose_stdout_sha256": proof.link.sha(stdout),
        "verbose_stderr_sha256": proof.link.sha(stderr) if stderr else None,
        "toolchain": {"vc_toolset_version": VC, "sdk_version": SDK},
        "map_observations": {"rows": rows},
    }).encode()


def run(exe, evidence, out, err, reader):
    return proof.build_report(exe, proof.link.sha(exe), COMMIT, evidence, proof.link.sha(evidence), out, err,
                              VC_DIR, SDK_DIR, VC, SDK, reader)


class CoffProof(unittest.TestCase):
    def test_microsoft_two_linkers_and_longname_offsets(self):
        raw = coff(short=(b"BOOT.OBJ", b"system.dll"),
                   long=(b"C:\\private-user\\path\\shared.obj", b"/private-trace/folder/SHARED.OBJ",
                         b"directory/large_object_member.obj"))
        wanted = {"boot.obj", "shared.obj", "large_object_member.obj", "system.dll"}
        names, count, skipped = proof.coff_members(BytesIO(raw), len(raw), wanted)
        self.assertEqual((count, skipped), (5, 1))
        self.assertEqual(names, {"boot.obj": 1, "shared.obj": 2, "large_object_member.obj": 1})
        with tempfile.TemporaryDirectory() as folder:
            actual = Path(folder) / "libcmt.lib"
            actual.write_bytes(raw)
            self.assertEqual(proof.archive_file(actual, wanted), (proof.link.sha(raw), names, count, skipped))

    def test_malformed_members_offsets_and_gnu_thin_formats_fail_closed(self):
        basic = coff(short=(b"MAIN.OBJ",))
        links = proof.COFF_SIGNATURE + member(b"/", bytes(4)) + member(b"/", bytes(8))
        for raw, message in [
            (b"!<thin>\n", "archive signature"),
            (proof.COFF_SIGNATURE + member(b"/", bytes(4)) + member(b"main.obj/", b"data"), "second"),
            (basic[:-2], "truncated"),
            (basic[:-1] + b"X", "alignment"),
            (links + member(b"//", b"some-long.obj") + member(b"/0", b"obj"), "unterminated"),
            (links + member(b"//", b"some-long.obj\0") + member(b"/1", b"obj"), "entry boundary"),
            (links + member(b"#1/7", b"bad GNU extended form"), "member-name form"),
        ]:
            with self.subTest(message=message), self.assertRaisesRegex(ValueError, message):
                proof.coff_members(BytesIO(raw), len(raw), set())

    def test_exact_verified_search_archive_correlates_only_positive_map_members(self):
        roots = proof.versioned_roots(VC_DIR, SDK_DIR, VC, SDK)
        _, crt = proof.approved_path("libcmt", roots, SDK)
        _, ucrt = proof.approved_path("libucrt", roots, SDK)
        _, vcruntime = proof.approved_path("libvcruntime", roots, SDK)
        exe = candidate()
        out = (f' Searching "{crt}":\n     Loaded libcmt.lib(ignore-loaded-only.obj)\n'
               f' Searching {ucrt}:\n Searching {vcruntime}:\n'
               f' Searching C:\\private-install\\unapproved\\kernel32.lib:\n').encode("utf-16")
        err = f" Searching {crt}:\n".encode()
        rows = [map_row("libcmt", "main.obj", 2), map_row("libcmt", "main.obj", 3, "static"),
                map_row("libcmt", "missing.obj"), map_row("libucrt", "duplicate.obj")]
        evidence = inventory(exe, out, err, rows)
        libraries = {
            crt: coff(short=(b"main.obj",), long=(b"C:\\private-binary\\ignore-loaded-only.obj",)),
            ucrt: coff(short=(b"duplicate.obj", b"DUPLICATE.OBJ")),
            vcruntime: coff(short=(b"present.obj",)),
        }
        wanted_reads = []

        def reader(path, root, wanted):
            wanted_reads.append(path)
            raw = libraries[path]
            return proof.link.sha(raw), *proof.coff_members(BytesIO(raw), len(raw), wanted)

        report = run(exe, evidence, out, err, reader)
        self.assertEqual(set(wanted_reads), {crt, ucrt, vcruntime})
        self.assertEqual(report["link_inventory_sha256"], proof.link.sha(evidence))
        families = {record["family"]: record for record in report["families"]}
        checked = {item["member"]: item for item in families["libcmt"]["map_members"]}
        self.assertEqual(checked["main.obj"], {"status": proof.MATCHED, "member": "main.obj", "map_symbol_rows": 5,
                         "observed_occurrences": 1, "archive_sha256": proof.link.sha(libraries[crt])})
        self.assertEqual(checked["missing.obj"]["status"], "not-found-in-approved-searched-archive")
        self.assertNotIn("ignore-loaded-only.obj", checked)
        self.assertEqual(families["libucrt"]["map_members"][0]["status"], "ambiguous-member-occurrences")
        self.assertEqual(families["libvcruntime"]["map_members"], [])
        self.assertEqual(families["libcmt"]["search"]["trace_search_rows"], 2)
        self.assertEqual(families["libcmt"]["search"]["absolute_paths"], 1)
        self.assertNotIn("private-", json.dumps(report))

    def test_ambiguous_or_unapproved_search_is_never_opened_or_proven(self):
        roots = proof.versioned_roots(VC_DIR, SDK_DIR, VC, SDK)
        _, crt = proof.approved_path("libcmt", roots, SDK)
        exe = candidate()
        out = (f" Searching {crt}:\n Searching libcmt.lib:\n"
               " Searching C:\\not-approved\\libcmt.lib:\n Searching C:\\not-approved\\libucrt.lib:\n").encode()
        evidence = inventory(exe, out, b"", [map_row("libcmt", "main.obj"), map_row("libucrt", "main.obj"),
                                             map_row("libvcruntime", "only-map.obj")])
        calls = []

        def reader(path, root, wanted):
            calls.append(path)
            return "21" * 32, {"main.obj": 1}, 1, 0

        found = run(exe, evidence, out, b"", reader)
        self.assertEqual(calls, [crt])
        for item, unresolved in zip(found["families"], (True, True, False)):
            self.assertEqual(item["map_members"][0]["status"],
                             "unresolved-searched-containers" if unresolved else "no-approved-archive-searched")
        self.assertEqual(found["families"][0]["search"]["unapproved_paths"], 1)
        self.assertEqual(found["families"][0]["search"]["unbound_rows"], 1)

    def test_trace_and_inventory_bind_same_exact_candidate_and_versions(self):
        exe = candidate()
        out = b"trace from candidate"
        record = inventory(exe, out, b"", [map_row("libcmt", "main.obj")])
        uncalled = lambda *args: self.fail("unverified evidence examined a native archive")
        with self.assertRaisesRegex(ValueError, "verbose trace"):
            run(exe, record, out + b"edited", b"", uncalled)
        altered = json.loads(record)
        altered["source_commit"] = "3a" * 20
        with self.assertRaisesRegex(ValueError, "another EXE"):
            run(exe, json.dumps(altered).encode(), out, b"", uncalled)
        with self.assertRaisesRegex(ValueError, "toolchain versions"):
            proof.versioned_roots(VC_DIR, SDK_DIR, "14.1.2", SDK)


if __name__ == "__main__":
    unittest.main()
