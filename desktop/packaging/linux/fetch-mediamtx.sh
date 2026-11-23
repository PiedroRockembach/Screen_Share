#!/bin/sh
# Downloads the MediaMTX server for Linux x86_64 and checks it against the
# release's published checksums.
# Usage: fetch-mediamtx.sh <version, e.g. v1.21.4> <output file>
set -eu
version=$1
out=$2
file="mediamtx_${version}_linux_amd64.tar.gz"
base="https://github.com/bluenviron/mediamtx/releases/download/$version"
work=$(mktemp -d)
curl -fsSL -o "$work/$file" "$base/$file"
curl -fsSL -o "$work/checksums.sha256" "$base/checksums.sha256"
# Lines are "<hash> *<file>" (binary mode) or "<hash>  <file>".
expected=$(awk -v f="$file" '{ n = $2; sub(/^\*/, "", n); if (n == f) print $1 }' "$work/checksums.sha256")
actual=$(sha256sum "$work/$file" | cut -d' ' -f1)
if [ -z "$expected" ] || [ "$expected" != "$actual" ]; then
    echo "checksum do MediaMTX não confere" >&2
    exit 1
fi
tar -xzf "$work/$file" -C "$work" mediamtx
install -m755 "$work/mediamtx" "$out"
rm -rf "$work"
