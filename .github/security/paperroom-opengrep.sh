#!/usr/bin/env bash
# GitHub-hosted Linux static audit only. No app, test input, external rule or build executes.
set -euo pipefail
umask 077
[[ "${RUNNER_ENVIRONMENT:-}" == github-hosted && "${RUNNER_OS:-}" == Linux && "${RUNNER_ARCH:-}" == X64 ]]
[[ "${GITHUB_EVENT_NAME:-}" == push && "${GITHUB_REPOSITORY:-}" == yung13yubabie/paperroom ]]
[[ "${GITHUB_ACTOR:-}" == yung13yubabie && "${GITHUB_REF:-}" == refs/heads/dev/offline-workspace-handoff ]]
[[ "$GITHUB_SHA" =~ ^[0-9a-f]{40}$ && "$(git rev-parse HEAD)" == "$GITHUB_SHA" ]]
[[ "$(id -u)" != 0 ]]
work="$(mktemp -d "${RUNNER_TEMP:?}/paperroom-opengrep.XXXXXXXX")"
trap 'rm -rf -- "$work"' EXIT
mkdir -m 700 "$work/home" "$work/canary" "$work/source"
# Export tracked source only. No .git, user documents, workflow context or credentials.
git archive "$GITHUB_SHA" | tar -x -C "$work/source"
cd "$work"
version=2.0.0
digest=37e93f4da5fd04a18c5c1febbdd12f7e98dfccd4d0c9cf2593bcb25eeb462da0
# The SHA-256 is published in upstream's official release assets listing.
# https://github.com/opengrep/opengrep/releases/expanded_assets/v2.0.0
curl --disable --fail --silent --show-error --location \
  --proto '=https' --proto-redir '=https' --connect-timeout 15 --max-time 100 \
  --output opengrep "https://github.com/opengrep/opengrep/releases/download/v${version}/opengrep_linux_x86"
printf '%s  opengrep\n' "$digest" | sha256sum --check --strict
chmod 500 opengrep
rules="$work/source/.github/security/paperroom-opengrep.yml"
# Rules are in-repo and explicitly selected. No registry, no autofix, no local builds.
# GitHub's disposable Linux runner may block unprivileged namespaces. sudo only
# creates the fresh network namespace; setpriv drops all privileges before execution.
audit_uid="$(id -u)" audit_gid="$(id -g)"
sandbox() {
  sudo -n /usr/bin/unshare --net -- /usr/bin/setpriv \
    --reuid "$audit_uid" --regid "$audit_gid" --clear-groups --no-new-privs -- \
    /usr/bin/env -i HOME="$work/home" TMPDIR="$work/home" PATH=/usr/bin:/bin LC_ALL=C.UTF-8 \
    /bin/bash -c '
      set -euo pipefail
      test "$(id -u)" -ne 0
      grep -Eq "^NoNewPrivs:[[:space:]]+1$" /proc/self/status
      awk "NR>2 {split(\$1,p,\":\"); if (p[1]!=\"lo\") exit 1; n++} END {if (n!=1) exit 1}" /proc/net/dev
      exec "$@"
    ' audit "$@"
}
tool_version="$(sandbox "$work/opengrep" --version)"
[[ "$tool_version" == "$version" || "$tool_version" == "Opengrep $version" || "$tool_version" == "opengrep $version" ]]
cat > canary/positive.rs <<'RS'
use std::process::Command;
use std::net::TcpStream;
fn audit_canary() {
    let _ = Command::new("never-run-canary");
    let _ = TcpStream::connect("127.0.0.1:1");
    ShellExecuteW(0, 0, 0, 0, 0, 0);
}
RS
cat > canary/positive.py <<'PY'
import subprocess
def audit_canary():
    return eval("1 + 1")
def shell_canary():
    return subprocess.run("never-run-canary", shell=True)
PY
printf 'fn audit_control() -> i32 { 1 + 1 }\n' > canary/negative.rs
printf 'def audit_control():\n    return 1 + 1\n' > canary/negative.py
scan_flags=(scan --strict --json --no-progress-bar --no-rewrite-rule-ids --config "$rules")
sandbox "$work/opengrep" "${scan_flags[@]}" canary/{positive,negative}.{rs,py} > canary.json
python3 -I - "$work" <<'PY'
import collections, json, pathlib, sys
work = pathlib.Path(sys.argv[1])
data = json.loads((work / 'canary.json').read_text())
expected = collections.Counter({
    ('positive.rs', 'paperroom.rust.shell-execute-review'): 1,
    ('positive.rs', 'paperroom.rust.child-process-review'): 1,
    ('positive.rs', 'paperroom.rust.tcp-connect-review'): 1,
    ('positive.py', 'paperroom.python.dynamic-evaluation'): 1,
    ('positive.py', 'paperroom.python.shell-execution'): 1,
})
actual = collections.Counter((pathlib.Path(row['path']).name, row['check_id']) for row in data['results'])
assert data['errors'] == [], data['errors']
assert actual == expected, (actual, expected)
assert {pathlib.Path(p).name for p in data['paths']['scanned']} == {
    'positive.py', 'positive.rs', 'negative.py', 'negative.rs',
}
print('Canaries: 5 expected detections, zero control detections, 4 of 4 files scanned')
PY
git -C "$GITHUB_WORKSPACE" ls-tree -r -z --name-only "$GITHUB_SHA" -- src scripts build.rs > target-list.nul
mapfile -d '' names < target-list.nul
targets=()
for name in "${names[@]}"; do
  case "$name" in
    src/*.rs|build.rs|scripts/*.py)
      [[ -f "source/$name" && ! -L "source/$name" ]]
      targets+=("$name") ;;
  esac
done
(( ${#targets[@]} > 0 ))
printf '%s\0' "${targets[@]}" > requested.nul
# Scope includes only a scratch archive, not the checkout. Compare tracked
# input digests after scanning. Actual scanner starts with networking disabled.
( cd source; sha256sum -- "$rules" "${targets[@]}" ) > inputs.sha256
(
  cd source
  sandbox "$work/opengrep" "${scan_flags[@]}" "${targets[@]}"
) > findings.json
( cd source; sha256sum --check --strict "$work/inputs.sha256" ) > /dev/null
python3 -I - "$work" "$GITHUB_SHA" "$tool_version" "$digest" <<'PY'
import hashlib, json, pathlib, sys
work, source_sha, version, scanner_sha = pathlib.Path(sys.argv[1]), *sys.argv[2:]
config = work / 'source/.github/security/paperroom-opengrep.yml'
data = json.loads((work / 'findings.json').read_text())
assert data['errors'] == [], data['errors']
targets = set((work / 'requested.nul').read_bytes().decode('utf-8').split('\0')[:-1])
root = work / 'source'
def normalize(value):
    path = pathlib.Path(value)
    return (path.relative_to(root) if path.is_absolute() else path).as_posix()
scanned = {normalize(path) for path in data['paths']['scanned']}
assert scanned == targets, {'not_scanned': sorted(targets - scanned), 'unexpected': sorted(scanned - targets)}
assert all(normalize(item['path']) in targets for item in data['results'])
print(f'OpenGrep {version} SHA-256 {scanner_sha}')
print(f'Source {source_sha}; rules SHA-256 {hashlib.sha256(config.read_bytes()).hexdigest()}')
print(f'5 local rules; scanned {len(scanned)} of {len(targets)} tracked Rust and Python inputs; parse errors 0')
print(f'{len(data["results"])} review signal(s); not confirmed findings')
for item in data['results']:
    print(f'{normalize(item["path"])}:{item["start"]["line"]} {item["check_id"]}')
PY
git -C "$GITHUB_WORKSPACE" diff --exit-code "$GITHUB_SHA" --
