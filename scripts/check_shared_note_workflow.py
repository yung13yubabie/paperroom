"""Read the Phase 29 synthetic database and compare its saved rules, paths and immutable identities."""
import json
import sqlite3
import sys
from contextlib import closing
from pathlib import Path

from check_unified_workspace import check, exactly_one, require


def snapshot(db, manifest):
    identity = check(db, manifest)
    notes = identity["notes"]
    edge_id = identity["field"][0]
    def value(role, key):
        return exactly_one(db, "SELECT value FROM note_properties WHERE note_id=? AND key=?",
                           (notes[role][0], key), f"{role}.{key}")[0]
    require(value("exact", "Duration") == "12" and value("decoy", "Duration") == "1", "Wrong values on same-title notes")
    require(value("canonical", "Batch") == "A" and value("other", "Batch") == "A", "Unexpected saved review fields")
    view, raw = exactly_one(db, "SELECT id,rules_json FROM collection_views WHERE name='SharedReview' AND collection_id=?",
                            (identity["field"][1],), "SharedReview")
    mode, filters, sorts = json.loads(raw)
    require(mode is False and len(filters) == 2, "Expected two filters joined with AND")
    require(filters[0][:3] == ["Batch", "text", 0] and filters[0][3].lower() == "a", "Approval filter changed")
    require(filters[1] == ["DurationTotal", "formula-number", 4, "0"], "Rollup filter changed")
    require(sorts == [["DurationTotal", "formula-number", True]], "Numeric descending sort changed")
    expression, = exactly_one(db, "SELECT expression FROM collection_formulas WHERE name='DurationTotal' AND collection_id=?",
                              (identity["field"][1],), "DurationTotal")
    require(expression == f'field_sum("{edge_id}","out","Duration")', "Unexpected rollup binding")
    for role, title in [("exact", "ExactSource"), ("decoy", "DecoySource")]:
        placements = db.execute("SELECT c.board_id,c.x,c.y FROM board_cards c JOIN boards b ON b.id=c.board_id WHERE c.note_id=? AND b.title=?",
                                (notes[role][0], title)).fetchall()
        require(len(placements) == 1, f"Wrong {role} placement")
        identity[role + "_placement"] = placements[0]
    identity["view_id"] = view
    return identity


def main():
    database, manifest, *prior = sys.argv[1:]
    with closing(sqlite3.connect(Path(database).resolve(strict=True).as_uri() + "?mode=ro", uri=True)) as db:
        db.execute("PRAGMA query_only=ON")
        db.execute("BEGIN")
        identity = snapshot(db, json.loads(Path(manifest).read_text(encoding="utf-8-sig")))
    result = json.dumps(identity, ensure_ascii=True, sort_keys=True)
    if prior:
        require(json.loads(result) == json.loads(Path(prior[0]).read_text(encoding="utf-8-sig")), "Stable IDs, placements or attachments changed")
    print(result)


if __name__ == "__main__":
    main()
