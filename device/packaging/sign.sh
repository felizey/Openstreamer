#!/bin/sh
# Signs a package for the web interface's updater: sign.sh <file.deb>
# writes <file.deb>.sig. The key is the private half of update-key.pub
set -eu
KEY=${OPENSTREAMER_SIGNING_KEY:-$HOME/openstreamer-signing-key.pem}
[ "$#" = 1 ] && [ -f "$1" ] || { echo "Usage: $0 package.deb" >&2; exit 2; }
HERE=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
WORK=$(mktemp -d)
trap 'rm -rf -- "$WORK"' EXIT
openssl pkey -in "$KEY" -pubout -outform DER -out "$WORK/public.der"
actual=$(tail -c 32 "$WORK/public.der" | base64 -w0)
expected=$(tr -d '\r\n' <"$HERE/update-key.pub")
[ "$actual" = "$expected" ] || { echo 'Signing key does not match this source release public key.' >&2; exit 1; }
openssl pkeyutl -sign -inkey "$KEY" -rawin -in "$1" -out "$WORK/signature"
openssl pkeyutl -verify -pubin -keyform DER -inkey "$WORK/public.der" -rawin -in "$1" -sigfile "$WORK/signature" >/dev/null
base64 -w0 "$WORK/signature" >"$1.sig.tmp"
mv -f -- "$1.sig.tmp" "$1.sig"
echo "$1.sig"
