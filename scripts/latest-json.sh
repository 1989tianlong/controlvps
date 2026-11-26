#!/usr/bin/env bash
# Builds the updater manifest attached to a GitHub release.
# Usage: scripts/latest-json.sh <version> [notes]
set -euo pipefail
cd "$(dirname "$0")/.."

version="${1:?usage: scripts/latest-json.sh <version> [notes]}"
notes="${2:-}"
repo="${CONTROLVPS_REPO:-1989tianlong/controlvps}"
bundle="src-tauri/target/release/bundle/macos/ControlVPS.app.tar.gz"

test -f "$bundle" || { echo "missing $bundle; build with TAURI_SIGNING_PRIVATE_KEY first" >&2; exit 1; }
test -f "$bundle.sig" || { echo "missing $bundle.sig" >&2; exit 1; }

bundle_version="$(tar -xOzf "$bundle" ControlVPS.app/Contents/Info.plist | plutil -extract CFBundleShortVersionString raw -o - - 2>/dev/null || true)"
if [ "$bundle_version" != "$version" ]; then
  echo "version mismatch: manifest $version, bundle ${bundle_version:-unknown}" >&2
  exit 1
fi

VERSION="$version" NOTES="$notes" REPO="$repo" SIG="$(cat "$bundle.sig")" python3 - <<'PY' > latest.json
import json, os
version = os.environ["VERSION"]
repo = os.environ["REPO"]
print(json.dumps({
    "version": version,
    "notes": os.environ.get("NOTES", ""),
    "pub_date": __import__("datetime").datetime.now(__import__("datetime").timezone.utc)
        .strftime("%Y-%m-%dT%H:%M:%SZ"),
    "platforms": {
        "darwin-aarch64": {
            "signature": os.environ["SIG"],
            "url": f"https://github.com/{repo}/releases/download/v{version}/ControlVPS.app.tar.gz",
        }
    },
}, ensure_ascii=False, indent=2))
PY
echo "wrote latest.json for v$version"
