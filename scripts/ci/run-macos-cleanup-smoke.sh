#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "macOS cleanup smoke is only supported on macOS hosts." >&2
  exit 1
fi

cargo build -p rebecca --locked
rebecca_bin="$repo_root/target/debug/rebecca"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

export HOME="$tmp/home"
export TMPDIR="$tmp/tmp"
export TEMP="$tmp/tmp"
export REBECCA_CONFIG_DIR="$tmp/rebecca-config"
export REBECCA_STATE_DIR="$tmp/rebecca-state"
export REBECCA_CACHE_DIR="$tmp/rebecca-cache"
export REBECCA_HISTORY_FILE="$tmp/rebecca-state/history.jsonl"
export REBECCA_STEAM_DISCOVERY="none"
export REBECCA_TEST_DISABLE_LIVE_NTFS_MFT="1"

macos_cache_home="$HOME/Library/Caches"
macos_application_support_home="$HOME/Library/Application Support"

mkdir -p \
  "$macos_cache_home/pip" \
  "$macos_cache_home/Firefox/Profiles/profile.default/cache2" \
  "$macos_application_support_home/Google/Chrome/Default/Cache" \
  "$macos_application_support_home/Slack/Cache" \
  "$TMPDIR" \
  "$REBECCA_CONFIG_DIR" \
  "$REBECCA_STATE_DIR" \
  "$REBECCA_CACHE_DIR"

printf 'pip-cache\n' > "$macos_cache_home/pip/http-cache.bin"
printf 'chrome-cache\n' > "$macos_application_support_home/Google/Chrome/Default/Cache/data.bin"
printf 'firefox-cache\n' > "$macos_cache_home/Firefox/Profiles/profile.default/cache2/data.bin"
printf 'slack-cache\n' > "$macos_application_support_home/Slack/Cache/data.bin"
printf 'temp-cache\n' > "$TMPDIR/rebecca-macos-smoke.tmp"

"$rebecca_bin" catalog --kind cleanup-rule --platform macos --format json > "$tmp/catalog.json"
python3 - "$tmp/catalog.json" <<'PY'
import json
import sys

payload = json.load(open(sys.argv[1], encoding="utf-8"))
rules = payload["data"]
ids = {rule["id"] for rule in rules}
assert payload["command"] == "catalog"
assert payload["payload_kind"] == "catalog"
assert rules, "macOS catalog should not be empty"
assert all(rule["platform"] == "macos" for rule in rules), ids
for required in {
    "macos.user-temp",
    "macos.pip-cache",
    "macos.chrome-cache",
    "macos.firefox-profile-cache",
    "macos.slack-cache",
}:
    assert required in ids, f"missing {required}"
PY

"$rebecca_bin" clean --dry-run --format json --no-scan-cache \
  --rule macos.pip-cache --allow-moderate > "$tmp/pip-clean.json"
python3 - "$tmp/pip-clean.json" <<'PY'
import json
import sys

payload = json.load(open(sys.argv[1], encoding="utf-8"))
data = payload["data"]
assert data["request"]["platform"] == "macos"
assert data["summary"]["allowed_targets"] >= 1, data["summary"]
assert any(target["rule_id"] == "macos.pip-cache" for target in data["targets"])
PY

"$rebecca_bin" clean --dry-run --format ndjson --no-scan-cache \
  --rule macos.chrome-cache --allow-warning active-process > "$tmp/chrome-clean.ndjson"
python3 - "$tmp/chrome-clean.ndjson" <<'PY'
import json
import sys

events = [json.loads(line) for line in open(sys.argv[1], encoding="utf-8") if line.strip()]
assert events[0]["event_kind"] == "started"
assert events[-1]["event_kind"] == "completed"
assert any(event.get("event_kind") == "target-finished" for event in events), events
completed = events[-1]["data"]
assert completed["request"]["platform"] == "macos"
assert completed["summary"]["allowed_targets"] >= 1, completed["summary"]
PY

"$rebecca_bin" clean --dry-run --format json --no-scan-cache \
  --rule macos.slack-cache --allow-warning active-process > "$tmp/slack-clean.json"
python3 - "$tmp/slack-clean.json" <<'PY'
import json
import sys

payload = json.load(open(sys.argv[1], encoding="utf-8"))
data = payload["data"]
assert data["request"]["platform"] == "macos"
assert data["summary"]["allowed_targets"] >= 1, data["summary"]
assert any(target["rule_id"] == "macos.slack-cache" for target in data["targets"])
PY

"$rebecca_bin" doctor permissions --format json > "$tmp/permissions.json"
python3 - "$tmp/permissions.json" <<'PY'
import json
import sys

data = json.load(open(sys.argv[1], encoding="utf-8"))["data"]
assert data["platform"] == "macos"
assert data["platform_supported"] is True
assert data["cleanup_execution_supported"] is True
assert data["privilege_level"] in {"standard-user", "elevated", "unknown"}
PY

REBECCA_ACTIVE_PROCESSES="firefox:4242;Google Chrome:4243;Slack:4244;zoom.us:4245" \
  "$rebecca_bin" doctor active-processes --format json > "$tmp/processes.json"
python3 - "$tmp/processes.json" <<'PY'
import json
import sys

data = json.load(open(sys.argv[1], encoding="utf-8"))["data"]
rule_ids = {rule_id for match in data["matches"] for rule_id in match["rule_ids"]}
assert data["platform"] == "macos"
assert data["platform_supported"] is True
assert data["process_inspection_available"] is True
for required in {
    "macos.firefox-profile-cache",
    "macos.chrome-cache",
    "macos.slack-cache",
    "macos.zoom-logs",
}:
    assert required in rule_ids, f"missing {required}: {data}"
assert not any(rule_id.startswith(("windows.", "linux.")) for rule_id in rule_ids), rule_ids
PY

echo "macOS cleanup smoke passed."
