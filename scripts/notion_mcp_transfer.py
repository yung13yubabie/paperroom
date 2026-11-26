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
REFERENCE_SCHEMA = "paperroom-notion-mcp-transfer/2"
RELATION_SCHEMA = "paperroom-notion-mcp-transfer/3"
REFERENCE_LIMIT = 2048
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


def add_reference_notes(archive, references, approved, manifest):
    """Project only confirmed direct paragraph page mentions; never inspect Markdown for IDs."""
    if not isinstance(references, list) or len(references) > REFERENCE_LIMIT:
        raise ValueError("expected up to 2048 explicit reference records")
    records = []
    by_source = {}
    block_owners = {}
    for ref in references:
        require_keys(ref, ["source_page_id", "block_id", "block_parent_page_id", "block_type",
                           "rich_text_type", "mention_type", "target_page_id", "code"])
        source = page_id(ref["source_page_id"])
        block = page_id(ref["block_id"])
        parent = page_id(ref["block_parent_page_id"])
        if source not in approved or source != parent:
            raise ValueError("reference must come from a confirmed page's direct child block")
        if block_owners.setdefault(block, parent) != parent:
            raise ValueError("conflicting parent IDs for the same source block")
        if type(ref["code"]) is not bool or any(not isinstance(ref[k], str)
                for k in ("block_type", "rich_text_type", "mention_type")):
            raise ValueError("invalid reference type evidence")
        receipt = {"source_page_id": source, "block_id": block}
        records.append(receipt)
        if (ref["block_type"] != "paragraph" or ref["rich_text_type"] != "mention"
                or ref["mention_type"] != "page" or ref["code"]):
            receipt["result"] = "unsupported_type_or_code"
            continue
        target = page_id(ref["target_page_id"])
        if target not in approved:
            receipt["result"] = "target_outside_confirmed_batch"
            continue
        receipt["target_page_id"] = target
        targets = by_source.setdefault(source, set())
        receipt["result"] = "already_listed" if target in targets else "listed"
        targets.add(target)

    members = []
    titles = {p["source_page_id"]: p["title"] for p in manifest["pages"]}
    for source, targets in sorted(by_source.items()):
        name = f"Page {source}/Local page references.md"
        lines = ["# 本機頁面引用", "", "這是紙間依已確認的 Notion 頁面提及產生的導覽。", ""]
        for target in sorted(targets):
            # Escape Markdown punctuation so a supplied title cannot add markup.
            label = re.sub(r"([!-/:-@\[-`{-~])", r"\\\1", titles[target])
            lines.append(f"- [{label} ({target})](../Page%20{target}.md)")
        data = ("\n".join(lines) + "\n").encode("utf-8")
        if len(data) > PAGE_LIMIT:
            raise ValueError("reference companion exceeds the Markdown page limit")
        add_member(archive, name, data)
        members.append({"source_page_id": source, "member": name,
                        "member_sha256": hashlib.sha256(data).hexdigest()})
    manifest["reference_records"] = records
    manifest["reference_companions"] = members


def add_typed_relations(archive, source, approved):
    members = source["confirmed_page_data_sources"]
    relations = source["confirmed_database_relations"]
    if not isinstance(members, dict) or len(members) != len(approved):
        raise ValueError("each confirmed page needs one data-source ID")
    memberships = {page_id(k): page_id(v) for k, v in members.items()}
    if set(memberships) != approved:
        raise ValueError("data-source membership must match the confirmed pages")
    if not isinstance(relations, list) or len(relations) > REFERENCE_LIMIT:
        raise ValueError("expected up to 2048 confirmed typed relations")
    declarations = {}
    edges = set()
    for item in relations:
        require_keys(item, ["source_page_id", "property_name", "property_type",
                            "target_data_source_id", "target_page_id"])
        start, end = page_id(item["source_page_id"]), page_id(item["target_page_id"])
        destination = page_id(item["target_data_source_id"])
        name = item["property_name"]
        if (start not in approved or start == end or memberships.get(end) != destination
                or item["property_type"] != "relation" or not isinstance(name, str)
                or not name or name != name.strip() or len(name.encode()) > 256
                or any(ord(c) < 32 or 127 <= ord(c) <= 159 for c in name)):
            raise ValueError("relation identity, type, name or target data-source membership mismatch")
        if declarations.setdefault((memberships[start], name), destination) != destination:
            raise ValueError("same source property has inconsistent target data sources")
        edge = (start, name, "relation", destination, end)
        if edge in edges:
            raise ValueError("duplicate typed relation")
        edges.add(edge)
    plan = ["paperroom-notion-typed-relations/1", sorted(memberships.items()), sorted(edges)]
    data = json.dumps(plan, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    if len(data) > 1024 * 1024:
        raise ValueError("typed relation plan exceeds 1 MiB")
    add_member(archive, "paperroom-transfer/typed-relations-v1.json", data)


def build(raw):
    if len(raw) > INPUT_LIMIT:
        raise ValueError("input exceeds 8 MiB")
    source = json.loads(raw.decode("utf-8"), object_pairs_hook=unique_object)
    schema = source.get("schema") if isinstance(source, dict) else None
    keys = ["schema", "confirmed_page_ids", "pages"]
    if schema == REFERENCE_SCHEMA:
        keys.append("confirmed_page_references")
    if schema == RELATION_SCHEMA:
        keys.extend(["confirmed_page_data_sources", "confirmed_database_relations"])
    require_keys(source, keys)
    if schema not in (SCHEMA, REFERENCE_SCHEMA, RELATION_SCHEMA):
        raise ValueError("unknown transfer schema")
    approved = source["confirmed_page_ids"]
    pages = source["pages"]
    if not isinstance(approved, list) or not 1 <= len(approved) <= 128:
        raise ValueError("explicitly confirm 1–128 pages")
    ids = {page_id(item) for item in approved}
    if len(ids) != len(approved) or not isinstance(pages, list) or len(pages) != len(ids):
        raise ValueError("expected one record per unique confirmed page")
    manifest = {"schema": schema, "original_sha256": hashlib.sha256(raw).hexdigest(),
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
            if schema == REFERENCE_SCHEMA:
                manifest["pages"][-1]["title"] = title
        if schema == REFERENCE_SCHEMA:
            add_reference_notes(archive, source["confirmed_page_references"],
                                {p["source_page_id"] for p in manifest["pages"]}, manifest)
        if schema == RELATION_SCHEMA:
            add_typed_relations(archive, source, {p["source_page_id"] for p in manifest["pages"]})
            manifest["limitations"] = [x for x in LIMITATIONS if "relations," not in x] + [
                "Only explicit directed database relation values are projected; views, rollups and remote resources are not restored."]
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
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        for limit in json.loads(archive.read("paperroom-transfer/manifest.json"))["limitations"]:
            print(limit)


if __name__ == "__main__":
    main()
