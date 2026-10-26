#!/usr/bin/env bash
# Portable Ubuntu/WSL amd64 build; no pre-existing project images or volumes.
set -euo pipefail
ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
VERSION=$(awk '/^version[[:space:]]*=/{gsub(/"/, "", $3); print $3; exit}' "$ROOT/device/backend/Cargo.toml")
[ -n "$VERSION" ] || { echo 'Missing application version in Cargo.toml.' >&2; exit 1; }
MODE=${1:-package}
case "$MODE" in package|update|image|image-reuse) ;; *) echo "Usage: bash build.sh [package|update|image|image-reuse]" >&2; exit 2 ;; esac
[ "$#" -le 1 ] || { echo 'Too many arguments.' >&2; exit 2; }
SIGNING_KEY=${OPENSTREAMER_SIGNING_KEY:-$HOME/openstreamer-signing-key.pem}
if [ "$MODE" = update ] || [ -n "${OPENSTREAMER_SIGNING_KEY:-}" ] || [ -r "$SIGNING_KEY" ]; then
    [ -r "$SIGNING_KEY" ] || { echo 'Set OPENSTREAMER_SIGNING_KEY to the private Openstreamer update key before building an update.' >&2; exit 1; }
    for tool in openssl python3 dpkg-deb; do command -v "$tool" >/dev/null || { echo "Missing $tool" >&2; exit 1; }; done
    export OPENSTREAMER_SIGNING_KEY="$SIGNING_KEY"
fi
for tool in docker flock sha256sum; do command -v "$tool" >/dev/null || { echo "Missing $tool" >&2; exit 1; }; done
docker info >/dev/null
exec 9>"$ROOT/.build-package.lock"
flock -n 9 || { echo 'A package build is already running in this directory.' >&2; exit 1; }
IMAGE=openstreamer-portable-builder:0.1.9
# Each checkout owns its compiler/registry caches. Empty Docker is supported.
KEY=$(printf '%s' "$ROOT" | sha256sum | cut -c1-12)
WORK=openstreamer-portable-$KEY-work
REGISTRY=openstreamer-portable-$KEY-registry
OWNER=org.openstreamer.portable-builder
for volume in "$WORK" "$REGISTRY"; do
    if docker volume inspect "$volume" >/dev/null 2>&1; then
        [ "$(docker volume inspect -f '{{index .Labels "org.openstreamer.portable-builder"}}' "$volume")" = 1 ] || {
            echo "Refusing to use a foreign cache volume: $volume" >&2; exit 1;
        }
    else
        docker volume create --label "$OWNER=1" "$volume" >/dev/null
    fi
done
mkdir -p "$ROOT/device/backend/target/debian"
docker build --platform linux/amd64 --file "$ROOT/device/packaging/test/Dockerfile" --tag "$IMAGE" "$ROOT"
if ! docker run --rm --platform linux/amd64 --network none --label "$OWNER=1" "$IMAGE" /usr/local/bin/openstreamer-arm64-probe; then
    echo 'ARM64 execution is unavailable. Enable ARM64 binfmt/QEMU on the Linux Docker host, then retry. See BUILD.md.' >&2
    exit 1
fi
docker run --rm --platform linux/amd64 --label "$OWNER=1" \
    --mount "type=bind,source=$ROOT,target=/source,readonly" \
    --mount "type=bind,source=$ROOT/device/backend/target/debian,target=/out" \
    --mount "type=volume,source=$WORK,target=/build" \
    --mount "type=volume,source=$REGISTRY,target=/cargo-cache" \
    --env CARGO_HOME=/cargo-cache \
    "$IMAGE" sh /source/device/packaging/test/build-package.sh
PACKAGE="$ROOT/device/backend/target/debian/openstreamer_${VERSION}-1_arm64.deb"
test -f "$PACKAGE"
(cd "$(dirname "$PACKAGE")" && sha256sum "$(basename "$PACKAGE")" >"$(basename "$PACKAGE").sha256")
printf 'Package ready: %s\n' "$PACKAGE"
if [ -n "${OPENSTREAMER_SIGNING_KEY:-}" ]; then
    sh "$ROOT/device/packaging/sign.sh" "$PACKAGE"
    python3 "$ROOT/device/packaging/make-update-bundle.py" "$PACKAGE" --output "$ROOT/device/backend/target/debian"
fi
if [ "$MODE" = image ] || [ "$MODE" = image-reuse ]; then
    # Build pi-gen tools from the supplied Dockerfile, never from a caller's
    # exported local image ID. Image caches are created on this Docker host.
    unset OPENSTREAMER_TEST_TOOLING_IMAGE
    if [ "$MODE" = image-reuse ]; then
        bash "$ROOT/image/build-test.sh" --reuse "$PACKAGE"
    else
        bash "$ROOT/image/build-test.sh" "$PACKAGE"
    fi
fi
