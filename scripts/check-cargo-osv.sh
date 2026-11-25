#!/usr/bin/env bash
# Linux x86_64 only. Fetch public inputs separately; matching is offline.
set -eu
umask 077
version=v2.7.1
digest=fb7dab19a8bd9bbe5c3189f09d4b7ee1820c9553a23bac202648a56efd2f6ee8
fail() { printf 'OSV verification blocked: %s\n' "$*" >&2; exit 127; }
if [[ $# -lt 2 || "$(uname -s)-$(uname -m)" != Linux-x86_64 ]]; then
  fail 'usage on Linux x86_64: check-cargo-osv.sh prepare CACHE | scan CACHE Cargo.lock'
fi
mode=$1; cache=$2
db="$cache/db/osv-scalibr/crates.io"
remote() {
  curl --disable --proto '=https' --proto-redir '=https' --fail --silent --show-error \
    --location --retry 2 --connect-timeout 10 --max-time 90 "$@"
}
validate_db() {
  python3 - "$cache/osv-db.headers" "$db/all.zip" <<'PY'
import datetime, email.utils, hashlib, sys, zipfile
headers, db = sys.argv[1:]
try:
    with open(headers, encoding="iso-8859-1") as f:
        modified = [line.split(":", 1)[1].strip() for line in f
                    if line.lower().startswith("last-modified:")][-1]
    timestamp = email.utils.parsedate_to_datetime(modified)
    if timestamp.utcoffset() is None:
        raise ValueError("missing timezone")
    age = (datetime.datetime.now(datetime.timezone.utc) - timestamp).total_seconds()
    if not -600 <= age <= 72 * 3600:
        raise ValueError("public database timestamp is stale or unexpectedly in the future")
    with zipfile.ZipFile(db) as archive:
        if not archive.namelist() or archive.testzip() is not None:
            raise ValueError("public database is empty or corrupt")
    with open(db, "rb") as file:
        checksum = hashlib.file_digest(file, "sha256").hexdigest()
except (ValueError, IndexError, OSError, RuntimeError, zipfile.BadZipFile) as error:
    print(f"Cannot validate OSV database: {error}", file=sys.stderr)
    sys.exit(1)
print(f"OSV crates.io DB last modified {timestamp.isoformat()} | sha256 {checksum}")
PY
}
verify_binary() {
  printf '%s  %s\n' "$digest" "$cache/osv-scanner" | sha256sum --check --status
}
case "$mode" in
  prepare)
    [[ $# -eq 2 && ! -e "$cache" ]] || fail 'prepare requires a fresh dedicated cache path'
    mkdir -p "$db" || fail 'cannot create cache'
    remote --max-filesize 100000000 --output "$cache/osv-scanner" \
      "https://github.com/google/osv-scanner/releases/download/$version/osv-scanner_linux_amd64" ||
      fail 'scanner download unavailable'
    verify_binary || fail 'scanner SHA-256 mismatch'
    chmod 700 "$cache/osv-scanner"
    remote --max-filesize 20000000 --dump-header "$cache/osv-db.headers" --output "$db/all.zip" \
      'https://osv-vulnerabilities.storage.googleapis.com/crates.io/all.zip' ||
      fail 'public database download unavailable'
    validate_db || fail 'database is unavailable or unfit'
    ;;
  scan)
    [[ $# -eq 3 ]] || fail 'scan requires cache path and one exact lockfile'
    [[ $(basename -- "$3") == Cargo.lock && -f "$3" && ! -L "$3" ]] ||
      fail 'no regular Cargo.lock input'
    verify_binary || fail 'scanner missing or SHA-256 mismatch'
    validate_db || fail 'database is unavailable or unfit'
    scratch="$(mktemp -d)" || fail 'cannot create isolated scan folder'
    trap 'rm -rf -- "$scratch"' EXIT
    cp -- "$3" "$scratch/Cargo.lock" || fail 'cannot copy exact lockfile bytes'
    cmp -s -- "$3" "$scratch/Cargo.lock" || fail 'input changed while being copied'
    mkdir -p "$scratch/home"
    # Explicit empty config; do not discover ignore/execution settings from a PR.
    : > "$scratch/osv-scanner.toml"
    scanner="$(realpath "$cache/osv-scanner")"
    db_root="$(realpath "$cache/db")"
    code=0
    (cd "$scratch" &&
      env -i PATH=/usr/bin:/bin HOME="$scratch/home" OSV_SCANNER_LOCAL_DB_CACHE_DIRECTORY="$db_root" \
        "$scanner" scan source --offline --no-resolve --all-packages \
        --config "$scratch/osv-scanner.toml" --lockfile "$scratch/Cargo.lock" --format json \
        > "$scratch/report.json" 2> "$scratch/scanner.stderr") || code=$?
    # Don't print raw scanner output to a CI runner command channel.
    case "$code" in
      0|1)
        python3 - "$scratch/report.json" "$code" "$scratch/Cargo.lock" <<'PY' || fail 'unexpected scanner JSON or status'
import json, re, sys, tomllib
path, exit_code, lock = sys.argv[1], int(sys.argv[2]), sys.argv[3]
with open(path, encoding="utf-8") as handle:
    report = json.load(handle)
pkgs = [p for source in report["results"] for p in source["packages"]]
matches = [item for p in pkgs for item in p.get("vulnerabilities", [])]
assert pkgs and bool(matches) == bool(exit_code)
assert all(p["package"]["ecosystem"] == "crates.io" for p in pkgs)
with open(lock, "rb") as handle:
    locked = tomllib.load(handle)["package"]
registry = "registry+https://github.com/rust-lang/crates.io-index"
assert all(p.get("source") in {None, registry} for p in locked), "unsupported Cargo source"
expected = {(p["name"], p["version"]) for p in locked if p.get("source") == registry}
seen = {(p["package"]["name"], p["package"]["version"]) for p in pkgs}
assert expected and expected <= seen, "not all registry entries were scanned"
print(f"OSV offline: {len(expected)} crates.io dependencies checked, {len(pkgs)} total entries reported, {len(matches)} advisory records.")
for finding in matches:
    advisory = finding.get("id", "")
    if re.fullmatch(r"[A-Za-z0-9._-]{1,100}", advisory):
        print("Advisory ID: " + advisory)
PY
        exit "$code"
        ;;
      128) printf 'OSV verification blocked: zero packages found\n' >&2; exit 128 ;;
      *) fail "scanner did not complete (status $code)" ;;
    esac
    ;;
  *) fail 'unsupported mode' ;;
esac
