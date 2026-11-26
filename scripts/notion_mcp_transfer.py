"""Build a Paperroom transfer ZIP from explicitly confirmed Notion page bodies."""
import argparse
import hashlib
import io
import json
import re
import sys
import uuid
import zipfile
from pathlib import Path

SCHEMA = "paperroom-notion-mcp-transfer/1"
INPUT_LIMIT = 8 * 1024 * 1024
PAGE_LIMIT = 4 * 1024 * 1024
LIMITATIONS = [
    "Only the supplied page titles and text are projected into notes. The ZIP preserves the supplied JSON separately.",
    "Notion-specific blocks, block references, URLs, views, properties, relations, permissions, attachments and discussions are not reconstructed. No linked resource is fetched.",
    "The Paperroom reader normalizes CRLF to LF. Retain this ZIP if original line endings or source IDs matter.",
]


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON key")
        result[key] = value
    return result


def require_keys(value, keys):
    if not isinstance(value, dict) or set(value) != set(keys):
        raise ValueError("missing or unexpected fields; supply only the documented transfer fields")


def page_id(value):
    if not isinstance(value, str) or not re.fullmatch(
        r"[a-fA-F0-9]{32}|[a-fA-F0-9]{8}-(?:[a-fA-F0-9]{4}-){3}[a-fA-F0-9]{12}", value
    ):
        raise ValueError("expected a page UUID")
    return uuid.UUID(value).hex


def add_member(archive, name, data):
    entry = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
    entry.create_system = 3
    entry.external_attr = 0o100644 << 16
    archive.writestr(entry, data)


def build(raw):
    if len(raw) > INPUT_LIMIT:
        raise ValueError("input exceeds 8 MiB")
    source = json.loads(raw.decode("utf-8"), object_pairs_hook=unique_object)
    require_keys(source, ["schema", "confirmed_page_ids", "pages"])
    if source["schema"] != SCHEMA:
        raise ValueError("unknown transfer schema")
    approved = source["confirmed_page_ids"]
    pages = source["pages"]
    if not isinstance(approved, list) or not 1 <= len(approved) <= 128:
        raise ValueError("explicitly confirm 1–128 pages")
    ids = {page_id(item) for item in approved}
    if len(ids) != len(approved) or not isinstance(pages, list) or len(pages) != len(ids):
        raise ValueError("expected one record per unique confirmed page")
    manifest = {"schema": SCHEMA, "original_sha256": hashlib.sha256(raw).hexdigest(),
                "limitations": LIMITATIONS, "pages": []}
    output = io.BytesIO()
    with zipfile.ZipFile(output, "w") as archive:
        for page in pages:
            require_keys(page, ["id", "title", "markdown", "truncated", "unknown_block_count", "unknown_block_ids"])
            identity = page_id(page["id"])
            if identity not in ids:
                raise ValueError("unconfirmed or duplicate page")
            ids.remove(identity)
            if (page["truncated"] is not False or type(page["unknown_block_count"]) is not int
                    or page["unknown_block_count"] != 0 or page["unknown_block_ids"] != []):
                raise ValueError("incomplete connector result; review the source before transfer")
            title, body = page["title"], page["markdown"]
            if (not isinstance(title, str) or not 1 <= len(title) <= 180 or title != title.strip()
                    or any(ord(c) < 32 or 127 <= ord(c) <= 159 or c in "\u2028\u2029" for c in title)):
                raise ValueError("title must be a single trimmed line of up to 180 characters")
            if not isinstance(body, str) or "\x00" in body:
                raise ValueError("invalid Markdown body")
            body_bytes = body.encode("utf-8")
            data = ("# " + title + "\n\n").encode("utf-8") + body_bytes
            if len(data) > PAGE_LIMIT:
                raise ValueError("projected Markdown exceeds 4 MiB")
            name = f"Page {identity}.md"
            add_member(archive, name, data)
            manifest["pages"].append({"source_page_id": identity, "member": name,
                                      "body_sha256": hashlib.sha256(body_bytes).hexdigest(),
                                      "member_sha256": hashlib.sha256(data).hexdigest()})
        add_member(archive, "paperroom-transfer/original.json", raw)
        add_member(archive, "paperroom-transfer/manifest.json",
                   json.dumps(manifest, ensure_ascii=False, indent=2).encode("utf-8"))
    return output.getvalue()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    try:
        data = build(sys.stdin.buffer.read(INPUT_LIMIT + 1))
        with args.output.open("xb") as stream:
            try:
                stream.write(data)
                stream.flush()
            except OSError:
                stream.close()
                args.output.unlink()
                raise
    except (ValueError, OSError, RecursionError) as error:
        parser.exit(1, f"Transfer refused: {error}\n")
    print(f"Created ZIP SHA-256 {hashlib.sha256(data).hexdigest()}")
    for limit in LIMITATIONS:
        print(limit)


if __name__ == "__main__":
    main()
