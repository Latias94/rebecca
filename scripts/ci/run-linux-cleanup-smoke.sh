#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "Linux cleanup smoke is only supported on Linux hosts." >&2
  exit 1
fi

cargo build -p rebecca --locked
rebecca_bin="$repo_root/target/debug/rebecca"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

export HOME="$tmp/home"
export XDG_CACHE_HOME="$HOME/.cache"
export XDG_CONFIG_HOME="$HOME/.config"
export XDG_DATA_HOME="$HOME/.local/share"
export XDG_STATE_HOME="$HOME/.local/state"
export TMPDIR="$tmp/tmp"
export TEMP="$tmp/tmp"
export REBECCA_CONFIG_DIR="$tmp/rebecca-config"
export REBECCA_STATE_DIR="$tmp/rebecca-state"
export REBECCA_CACHE_DIR="$tmp/rebecca-cache"
export REBECCA_HISTORY_FILE="$tmp/rebecca-state/history.jsonl"
export REBECCA_STEAM_DISCOVERY="none"
export REBECCA_TEST_DISABLE_LIVE_NTFS_MFT="1"

mkdir -p \
  "$XDG_CACHE_HOME/pip" \
  "$XDG_CACHE_HOME/google-chrome/Default/Cache" \
  "$XDG_CACHE_HOME/mozilla/firefox/profile.default/cache2" \
  "$TMPDIR" \
  "$REBECCA_CONFIG_DIR" \
  "$REBECCA_STATE_DIR" \
  "$REBECCA_CACHE_DIR"

printf 'pip-cache\n' > "$XDG_CACHE_HOME/pip/http-cache.bin"
printf 'chrome-cache\n' > "$XDG_CACHE_HOME/google-chrome/Default/Cache/data.bin"
printf 'firefox-cache\n' > "$XDG_CACHE_HOME/mozilla/firefox/profile.default/cache2/data.bin"
printf 'temp-cache\n' > "$TMPDIR/rebecca-linux-smoke.tmp"

"$rebecca_bin" catalog --kind cleanup-rule --platform linux --format json > "$tmp/catalog.json"
python3 - "$tmp/catalog.json" <<'PY'
import json
import sys

payload = json.load(open(sys.argv[1], encoding="utf-8"))
rules = payload["data"]
ids = {rule["id"] for rule in rules}
assert payload["command"] == "catalog"
assert payload["payload_kind"] == "catalog"
assert rules, "Linux catalog should not be empty"
assert all(rule["platform"] == "linux" for rule in rules), ids
for required in {
    "linux.user-temp",
    "linux.pip-cache",
    "linux.chrome-cache",
    "linux.firefox-profile-cache",
    "linux.apt-cache",
}:
    assert required in ids, f"missing {required}"
PY

"$rebecca_bin" clean --dry-run --format json --no-scan-cache \
  --rule linux.pip-cache --allow-moderate > "$tmp/pip-clean.json"
python3 - "$tmp/pip-clean.json" <<'PY'
import json
import sys

payload = json.load(open(sys.argv[1], encoding="utf-8"))
data = payload["data"]
assert data["request"]["platform"] == "linux"
assert data["summary"]["allowed_targets"] >= 1, data["summary"]
assert any(target["rule_id"] == "linux.pip-cache" for target in data["targets"])
PY

"$rebecca_bin" clean --dry-run --format ndjson --no-scan-cache \
  --rule linux.chrome-cache --allow-warning active-process > "$tmp/chrome-clean.ndjson"
python3 - "$tmp/chrome-clean.ndjson" <<'PY'
import json
import sys

events = [json.loads(line) for line in open(sys.argv[1], encoding="utf-8") if line.strip()]
assert events[0]["event_kind"] == "started"
assert events[-1]["event_kind"] == "completed"
assert any(event.get("event_kind") == "target-finished" for event in events), events
completed = events[-1]["data"]
assert completed["request"]["platform"] == "linux"
assert completed["summary"]["allowed_targets"] >= 1, completed["summary"]
PY

"$rebecca_bin" clean --dry-run --format json \
  --rule linux.apt-cache --allow-moderate --allow-warning permission-sensitive > "$tmp/apt-clean.json"
python3 - "$tmp/apt-clean.json" <<'PY'
import json
import sys

payload = json.load(open(sys.argv[1], encoding="utf-8"))
data = payload["data"]
assert data["request"]["platform"] == "linux"
assert data["request"]["selected_rule_ids"] == ["linux.apt-cache"]
assert data["summary"]["failed_targets"] == 0
PY

"$rebecca_bin" doctor permissions --format json > "$tmp/permissions.json"
python3 - "$tmp/permissions.json" <<'PY'
import json
import sys

data = json.load(open(sys.argv[1], encoding="utf-8"))["data"]
assert data["platform"] == "linux"
assert data["platform_supported"] is True
assert data["cleanup_execution_supported"] is True
assert data["privilege_level"] in {"standard-user", "elevated", "unknown"}
PY

REBECCA_ACTIVE_PROCESSES="firefox:4242" \
  "$rebecca_bin" doctor active-processes --format json > "$tmp/processes.json"
python3 - "$tmp/processes.json" <<'PY'
import json
import sys

data = json.load(open(sys.argv[1], encoding="utf-8"))["data"]
rule_ids = {rule_id for match in data["matches"] for rule_id in match["rule_ids"]}
assert data["platform"] == "linux"
assert data["platform_supported"] is True
assert data["process_inspection_available"] is True
assert "linux.firefox-profile-cache" in rule_ids, data
assert not any(rule_id.startswith("windows.") for rule_id in rule_ids), rule_ids
PY

echo "Linux cleanup smoke passed."
