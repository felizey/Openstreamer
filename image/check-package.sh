#!/bin/sh
# Fail before touching image caches if the package carries an old update key.
set -eu
[ "$#" = 1 ] || { echo "Usage: $0 package_arm64.deb" >&2; exit 2; }
HERE=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
DEB=$1
[ "$(dpkg-deb -f "$DEB" Package)" = openstreamer ]
[ "$(dpkg-deb -f "$DEB" Architecture)" = arm64 ]
AUDIT=$(mktemp -d)
trap 'rm -rf -- "$AUDIT"' EXIT
dpkg-deb --extract "$DEB" "$AUDIT"
if ! cmp -s "$HERE/../device/packaging/update-key.pub" "$AUDIT/usr/share/openstreamer/update-key.pub" ||
   ! grep -aFq "$(tr -d '\r\n' <"$HERE/../device/packaging/update-key.pub")" "$AUDIT/usr/bin/openstreamer"; then
    echo 'This package does not carry the current Openstreamer update key. Run bash build.sh package first.' >&2
    exit 1
fi
printf 'Image package: Openstreamer %s, ARM64; current update key verified.\n' "$(dpkg-deb -f "$DEB" Version)"
