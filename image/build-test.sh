#!/usr/bin/env bash
# Build an isolated Raspberry Pi OS ARM64 microSD test image.
# Run from Linux/WSL: bash image/build-test.sh path/to/openstreamer_arm64.deb
# --prepare prepares the Docker image. --base builds just Raspberry Pi OS Lite;
# --reuse package.deb completes an image from that isolated cached base.
set -euo pipefail

PI_GEN_REF=4d8ee447dd3d37e8b0ef8752e460d9082d9d435d
EATMYDATA_VERSION=131-2+b1
EATMYDATA_SHA256=cff65f8a922df16382dc43ce305607e9dd8ebbb7d6f80a46aa1b5f8e953462ed
HERE=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
source "$HERE/test-cache.sh"
PI_GEN="$HERE/.pi-gen-openstreamer-test"
PREFIX=openstreamer-test
DOCKER_IMAGE="$PREFIX-pigen:$PI_GEN_REF"
TOOLING_IMAGE=${OPENSTREAMER_TEST_TOOLING_IMAGE:-}
CONTAINER="$PREFIX-pigen-build"
WORK_VOLUME="$PREFIX-pigen-work"
DEPLOY_VOLUME="$PREFIX-pigen-deploy"
PREPARE=0
BASE_ONLY=0
REUSE=0

case "${1:-}" in
    --prepare) PREPARE=1; shift ;;
    --base) BASE_ONLY=1; shift ;;
    --reuse) REUSE=1; shift ;;
esac
if [ "$PREPARE" = 0 ] && [ "$BASE_ONLY" = 0 ] && [ "$#" != 1 ]; then
    echo "Usage: $0 [--prepare | --base | [--reuse] package_arm64.deb]" >&2
    exit 2
fi
if [ "$PREPARE" = 0 ] && [ "$BASE_ONLY" = 0 ]; then
    DEB=$(realpath -- "$1")
    test -f "$DEB"
    [ "$(dpkg-deb -f "$DEB" Architecture)" = arm64 ] || {
        echo "The test image requires an arm64 .deb: $DEB" >&2
        exit 1
    }
    [ "$(dpkg-deb -f "$DEB" Package)" = openstreamer ] || {
        echo "The test image requires an Openstreamer package: $DEB" >&2
        exit 1
    }
    VERSION=$(dpkg-deb -f "$DEB" Version)
    sh "$HERE/check-package.sh" "$DEB"
    VERSION=${VERSION%-*}
fi
# Serialize all writes to the shared checkout, including package staging.
# The container-running check alone cannot stop two simultaneous preparations.
test_cache_take_lock "$PI_GEN"
if [ "$(docker container inspect -f '{{.State.Running}}' "$CONTAINER" 2>/dev/null || true)" = true ]; then
    echo "The Openstreamer test image build is already running." >&2
    exit 1
fi

if [ ! -d "$PI_GEN/.git" ]; then
    git init -q "$PI_GEN"
    git -C "$PI_GEN" remote add origin https://github.com/RPi-Distro/pi-gen
fi
git -C "$PI_GEN" config core.autocrlf false
if ! git -C "$PI_GEN" cat-file -e "$PI_GEN_REF^{commit}" 2>/dev/null; then
    git -C "$PI_GEN" fetch --depth 1 origin "$PI_GEN_REF"
fi
git -C "$PI_GEN" checkout -q --detach "$PI_GEN_REF"
# Only this script's dedicated checkout is reset. Other pi-gen builds are untouched.
git -C "$PI_GEN" checkout -q --force HEAD -- .
mkdir -p "$PI_GEN/test-tools"
cp "$HERE/test-cache.sh" "$HERE/test-build-container.sh" \
    "$HERE/cache-prerun-app.sh" "$HERE/cache-complete-app.sh" "$PI_GEN/test-tools/"
rm -rf "$PI_GEN/test-tools/stage-openstreamer-runtime"
cp -R "$HERE/test-runtime" "$PI_GEN/test-tools/stage-openstreamer-runtime"
RUNTIME_FILES="$PI_GEN/test-tools/stage-openstreamer-runtime/00-runtime/files"
mkdir -p "$RUNTIME_FILES"
cp "$HERE/stage-openstreamer/00-openstreamer/00-packages-nr" "$RUNTIME_FILES/runtime-packages"
cp "$HERE/stage-openstreamer/00-openstreamer/files/configure-runtime.sh" "$RUNTIME_FILES/"
: > "$RUNTIME_FILES/runtime-depends"
if [ "$PREPARE" = 0 ] && [ "$BASE_ONLY" = 0 ]; then
    # apt-get satisfy understands Debian alternatives and version constraints.
    # Include recommendations because the regular local .deb install does too.
    DEPENDENCIES=()
    for FIELD in Pre-Depends Depends Recommends; do
        DEPENDENCY_FIELD=$(dpkg-deb -f "$DEB" "$FIELD")
        [ -z "$DEPENDENCY_FIELD" ] || DEPENDENCIES+=("$DEPENDENCY_FIELD")
    done
    if [ "${#DEPENDENCIES[@]}" != 0 ]; then
        (IFS=,; printf '%s\n' "${DEPENDENCIES[*]}") > "$RUNTIME_FILES/runtime-depends"
    fi
fi
EATMYDATA_DEB="$PI_GEN/test-tools/libeatmydata1_${EATMYDATA_VERSION}_arm64.deb"
if [ ! -f "$EATMYDATA_DEB" ]; then
    curl --fail --location --retry 3 \
        "https://deb.debian.org/debian/pool/main/libe/libeatmydata/libeatmydata1_${EATMYDATA_VERSION}_arm64.deb" \
        --output "$EATMYDATA_DEB"
fi
printf '%s  %s\n' "$EATMYDATA_SHA256" "$EATMYDATA_DEB" | sha256sum --check
mkdir -p "$PI_GEN/test-tools/eatmydata-arm64"
dpkg-deb --extract "$EATMYDATA_DEB" "$PI_GEN/test-tools/eatmydata-arm64"
cp "$HERE/test.config" "$PI_GEN/config"
rm -rf "$PI_GEN/stage-openstreamer"
mkdir -p "$PI_GEN/stage-openstreamer"
cp -R "$HERE/stage-openstreamer/." "$PI_GEN/stage-openstreamer/"
mkdir -p "$PI_GEN/stage-openstreamer/99-test-build-cleanup"
cat > "$PI_GEN/stage-openstreamer/99-test-build-cleanup/00-run.sh" <<'CLEANUP'
#!/bin/bash -e
# A build-only dpkg option must never affect package management on the Pi.
rm -f "${ROOTFS_DIR}/etc/dpkg/dpkg.cfg.d/00-openstreamer-test-build-io"
if [ -f "${ROOTFS_DIR}/etc/ld.so.preload" ]; then
    sed -i '\|^/usr/local/lib/libopenstreamer-test-eatmydata.so$|d' "${ROOTFS_DIR}/etc/ld.so.preload"
    [ -s "${ROOTFS_DIR}/etc/ld.so.preload" ] || rm -f "${ROOTFS_DIR}/etc/ld.so.preload"
fi
rm -f "${ROOTFS_DIR}/usr/local/lib/libopenstreamer-test-eatmydata.so"
# ldconfig may have added a SONAME alias while configuring libc packages.
IO_LINK="${ROOTFS_DIR}/usr/local/lib/libeatmydata.so.1"
if [ -L "$IO_LINK" ] && [ "$(readlink "$IO_LINK")" = libopenstreamer-test-eatmydata.so ]; then
    rm -f "$IO_LINK"
fi
on_chroot <<'REBUILD_CACHE'
ldconfig
REBUILD_CACHE
CLEANUP
touch "$PI_GEN/stage2/SKIP_IMAGES"
rm -f "$PI_GEN/stage0/SKIP" "$PI_GEN/stage1/SKIP" "$PI_GEN/stage2/SKIP"
if [ "$REUSE" = 1 ]; then
    touch "$PI_GEN/stage0/SKIP" "$PI_GEN/stage1/SKIP" "$PI_GEN/stage2/SKIP"
fi
if [ "$PREPARE" = 0 ] && [ "$BASE_ONLY" = 0 ]; then
    mkdir -p "$PI_GEN/stage-openstreamer/00-openstreamer/files"
    cp "$DEB" "$PI_GEN/stage-openstreamer/00-openstreamer/files/openstreamer.deb"
    cp "$HERE/../device/packaging/update-key.pub" "$PI_GEN/stage-openstreamer/00-openstreamer/files/update-key.pub"
fi
# Docker build contexts created on Windows may lose executable mode bits.
find "$PI_GEN" -type f \( -name '*.sh' -o -name 'on_chroot' \) -exec chmod +x {} +

mkdir -p "$PI_GEN/work" "$PI_GEN/deploy"
RUN_MOUNTS=(--mount "type=bind,source=$PI_GEN,target=/pi-gen,readonly")
if [ -n "$TOOLING_IMAGE" ]; then
    case "$TOOLING_IMAGE" in
        sha256:*) ;;
        *) echo "OPENSTREAMER_TEST_TOOLING_IMAGE must be a pinned local image ID (sha256:...)." >&2; exit 2 ;;
    esac
    docker image inspect "$TOOLING_IMAGE" >/dev/null
    DOCKER_IMAGE="$TOOLING_IMAGE"
else
    docker build --build-arg BASE_IMAGE=docker.io/debian:trixie \
        --file "$HERE/Dockerfile.test" --tag "$DOCKER_IMAGE" "$HERE"
fi
if [ "$PREPARE" = 1 ]; then
    printf 'Prepared ARM64 pi-gen %s in %s\nDocker image: %s\n' \
        "$PI_GEN_REF" "$PI_GEN" "$DOCKER_IMAGE"
    exit 0
fi

TOOLING_IMAGE_ID=$(docker image inspect --format '{{.Id}}' "$DOCKER_IMAGE")
BASE_INPUT_KEY=$(test_cache_key base-v1 "$PI_GEN_REF" \
    "$(sha256sum "$HERE/test.config" | cut -d ' ' -f 1)")
RUNTIME_INPUT_KEY=$(test_cache_key runtime-v1 "$TOOLING_IMAGE_ID" \
    "$(test_cache_tree_digest "$PI_GEN/test-tools/stage-openstreamer-runtime")" \
    "$(sha256sum "$HERE/test-cache.sh" | cut -d ' ' -f 1)")
APP_INPUT_KEY=$(test_cache_key application-v1 "$(test_cache_tree_digest "$HERE/stage-openstreamer")" \
    "$(sha256sum "$HERE/cache-prerun-app.sh" "$HERE/cache-complete-app.sh" "$HERE/test-cache.sh" "$HERE/build-test.sh" | test_cache_digest)" \
    "$(sha256sum "$HERE/test.config" | cut -d ' ' -f 1)")
EXPORT_INPUT_KEY=$(test_cache_key export-v1 "$PI_GEN_REF" "$TOOLING_IMAGE_ID" \
    "$(test_cache_tree_digest "$PI_GEN/export-image")" \
    "$(sha256sum "$HERE/test.config" "$HERE/test-build-container.sh" "$HERE/build-test.sh" | test_cache_digest)")
PACKAGE_SHA256=''
if [ "$BASE_ONLY" = 0 ]; then
    PACKAGE_SHA256=$(sha256sum "$DEB" | cut -d ' ' -f 1)
fi

if [ "$REUSE" = 1 ]; then
    BASE_VOLUME_OWNER=$(docker volume inspect -f '{{index .Labels "org.openstreamer.test-builder"}}' "$WORK_VOLUME") || {
        echo "Could not inspect the isolated base volume $WORK_VOLUME. Check the Docker error above." >&2
        exit 1
    }
    [ "$BASE_VOLUME_OWNER" = 1 ] || {
        echo "Refusing to reuse a base volume without this builder's owner label: $WORK_VOLUME" >&2
        exit 1
    }
    BASE_PROBE_STATUS=0
    # Let Docker choose a unique name. An interrupted probe must not block the
    # next build, and the tooling image's deploy VOLUME needs no persistent data.
    docker run --rm --network none --workdir / --entrypoint sh \
        --label org.openstreamer.test-builder=1 \
        --mount "type=volume,source=$WORK_VOLUME,target=/pi-gen/work,readonly,volume-nocopy" \
        --mount type=tmpfs,target=/pi-gen/deploy,tmpfs-size=1048576 \
        "$DOCKER_IMAGE" -c \
        'test -f /pi-gen/work/Openstreamer/.openstreamer-test-base-ready || exit 42' \
        || BASE_PROBE_STATUS=$?
    case "$BASE_PROBE_STATUS" in
        0) ;;
        42)
            echo "No completed isolated base image is available. Run --base first." >&2
            exit 1
            ;;
        *)
            echo "Could not check the isolated base image (Docker exit $BASE_PROBE_STATUS). Fix the Docker error above and retry --reuse; the cached base has not been rebuilt." >&2
            exit "$BASE_PROBE_STATUS"
            ;;
    esac
fi

if docker container inspect "$CONTAINER" >/dev/null 2>&1; then
    [ "$(docker inspect -f '{{index .Config.Labels "org.openstreamer.test-builder"}}' "$CONTAINER")" = 1 ] || {
        echo "Refusing to replace a container without this builder's owner label." >&2
        exit 1
    }
    [ "$(docker inspect -f '{{.State.Running}}' "$CONTAINER")" = false ] || {
        echo "The Openstreamer test image build is already running." >&2
        exit 1
    }
    docker rm "$CONTAINER"
fi
for VOLUME in "$WORK_VOLUME" "$DEPLOY_VOLUME"; do
    if docker volume inspect "$VOLUME" >/dev/null 2>&1; then
        [ "$(docker volume inspect -f '{{index .Labels "org.openstreamer.test-builder"}}' "$VOLUME")" = 1 ] || {
            echo "Refusing to use a volume without this builder's owner label: $VOLUME" >&2
            exit 1
        }
    else
        docker volume create --label org.openstreamer.test-builder=1 "$VOLUME" >/dev/null
    fi
done
mkdir -p "$HERE/deploy"
WORK_VOLUME_CREATED=$(docker volume inspect -f '{{.CreatedAt}}' "$WORK_VOLUME")

# There are no host device mounts. All generated partitions live in the builder's
# own named volumes. Preserve the container and logs for inspection on failure.
BUILD_STATUS=0
docker run --name "$CONTAINER" --privileged --workdir / --entrypoint bash \
    --label org.openstreamer.test-builder=1 \
    "${RUN_MOUNTS[@]}" \
    --mount "type=volume,source=$WORK_VOLUME,target=/pi-gen/work,volume-nocopy" \
    --mount "type=volume,source=$DEPLOY_VOLUME,target=/pi-gen/deploy,volume-nocopy" \
    --env "GIT_HASH=$PI_GEN_REF" \
    --env "OPENSTREAMER_TEST_BASE_ONLY=$BASE_ONLY" \
    --env "OPENSTREAMER_TEST_REUSE=$REUSE" \
    --env "OPENSTREAMER_TEST_WORK_VOLUME_CREATED=$WORK_VOLUME_CREATED" \
    --env "OPENSTREAMER_TEST_BASE_INPUT_KEY=$BASE_INPUT_KEY" \
    --env "OPENSTREAMER_TEST_RUNTIME_INPUT_KEY=$RUNTIME_INPUT_KEY" \
    --env "OPENSTREAMER_TEST_APP_INPUT_KEY=$APP_INPUT_KEY" \
    --env "OPENSTREAMER_TEST_EXPORT_INPUT_KEY=$EXPORT_INPUT_KEY" \
    --env "OPENSTREAMER_TEST_PACKAGE_SHA256=$PACKAGE_SHA256" \
    "$DOCKER_IMAGE" /pi-gen/test-tools/test-build-container.sh || BUILD_STATUS=$?
docker logs --timestamps "$CONTAINER" > "$HERE/deploy/openstreamer-test-build.log" 2>&1
if [ "$BUILD_STATUS" != 0 ]; then
    echo "Image build failed ($BUILD_STATUS); see image/deploy/openstreamer-test-build.log." >&2
    exit "$BUILD_STATUS"
fi
if [ "$BASE_ONLY" = 1 ]; then
    echo "Raspberry Pi OS ARM64 base is ready in $WORK_VOLUME. Complete with --reuse package.deb."
    exit 0
fi
TEMP_DEPLOY="$HERE/deploy/.openstreamer-test-results"
mkdir -p "$TEMP_DEPLOY"
docker cp "$CONTAINER:/pi-gen/deploy/.openstreamer-test-result" "$TEMP_DEPLOY/result.txt"
IMAGE=$(cat "$TEMP_DEPLOY/result.txt")
[[ "$IMAGE" =~ ^Openstreamer-[0-9a-f]{64}\.img\.xz$ ]] || {
    echo "The builder returned an invalid image filename." >&2
    exit 1
}
OUTPUT="$HERE/deploy/openstreamer-$VERSION-raspi4b-arm64.img.xz"
IMAGE_KEY=${IMAGE#Openstreamer-}
IMAGE_KEY=${IMAGE_KEY%.img.xz}
docker cp "$CONTAINER:/pi-gen/deploy/.openstreamer-test-cache/$IMAGE_KEY.sha256" "$TEMP_DEPLOY/image.sha256"
EXPECTED_IMAGE_SHA256=$(cut -d ' ' -f 1 "$TEMP_DEPLOY/image.sha256")
[[ "$EXPECTED_IMAGE_SHA256" =~ ^[0-9a-f]{64}$ ]] || { echo 'Invalid cached image checksum.' >&2; exit 1; }
ACTUAL_IMAGE_SHA256=''
if [ -f "$OUTPUT" ]; then
    ACTUAL_IMAGE_SHA256=$(sha256sum "$OUTPUT" | cut -d ' ' -f 1)
fi
if [ "$ACTUAL_IMAGE_SHA256" = "$EXPECTED_IMAGE_SHA256" ]; then
    echo '[cache] local output HIT: SHA256 matches; copying the image out of Docker skipped.'
else
    docker cp "$CONTAINER:/pi-gen/deploy/$IMAGE" "$OUTPUT.tmp"
    xz --test "$OUTPUT.tmp"
    [ "$(sha256sum "$OUTPUT.tmp" | cut -d ' ' -f 1)" = "$EXPECTED_IMAGE_SHA256" ] || {
        echo 'The copied image does not match the builder checksum.' >&2
        exit 1
    }
    mv -f "$OUTPUT.tmp" "$OUTPUT"
fi
printf '%s  %s\n' "$EXPECTED_IMAGE_SHA256" "$(basename "$OUTPUT")" > "$OUTPUT.sha256"
{
    printf 'Openstreamer Raspberry Pi 4B ARM64 image\n'
    printf 'pi-gen: %s\nPackage: %s\n' "$PI_GEN_REF" "$(basename "$DEB")"
    docker image inspect --format 'Tooling image: {{.Id}}' "$DOCKER_IMAGE"
    printf 'Temporary ARM64 eatmydata helper: %s SHA256 %s (custom preload copy removed before export)\n' \
        "$EATMYDATA_VERSION" "$EATMYDATA_SHA256"
    printf '%s  %s\n' "$PACKAGE_SHA256" "$DEB" "$EXPECTED_IMAGE_SHA256" "$OUTPUT"
} > "$HERE/deploy/openstreamer-test-manifest.txt"
ls -lh "$OUTPUT" "$OUTPUT.sha256"
