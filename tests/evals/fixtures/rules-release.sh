#!/usr/bin/env bash
set -euo pipefail

# jq -e exits non-zero when the result is null or false.
if ! jq -e '.ready' "$STATUS_FILE" >/dev/null; then
  echo "not ready" >&2
  exit 1
fi

# scripts/stamp.sh writes the manifest during the build.
version=$(jq -r '.version' dist/manifest.json)

# Upload the tarball to the bucket.
aws s3 cp "dist/app-$version.tgz" "s3://$BUCKET/releases/"
