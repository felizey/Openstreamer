#!/usr/bin/env bash
# Runs only inside this project's Docker builder, with its labeled named volumes.
set -euo pipefail
cd /pi-gen
source /pi-gen/test-tools/test-cache.sh
source /pi-gen/config

if [ "$OPENSTREAMER_TEST_BASE_ONLY" != 1 ]; then
    STAGED_PACKAGE=/pi-gen/stage-openstreamer/00-openstreamer/files/openstreamer.deb
    if [[ ! "$OPENSTREAMER_TEST_PACKAGE_SHA256" =~ ^[0-9a-f]{64}$ ]] ||
        [ ! -f "$STAGED_PACKAGE" ] ||
        [ "$(sha256sum "$STAGED_PACKAGE" | cut -d ' ' -f 1)" != "$OPENSTREAMER_TEST_PACKAGE_SHA256" ]; then
        echo 'The staged Openstreamer package does not match the requested SHA256. Retry after package preparation finishes.' >&2
        exit 1
    fi
fi

WORK_DIR=/pi-gen/work/Openstreamer
DEPLOY_DIR=/pi-gen/deploy
CACHE_DIR=$DEPLOY_DIR/.openstreamer-test-cache
BASE_MARKER=$WORK_DIR/.openstreamer-test-base-ready
BASE_INPUTS=$WORK_DIR/.openstreamer-test-base-inputs
BASE_GENERATION=$WORK_DIR/.openstreamer-test-base-generation
mkdir -p "$WORK_DIR" "$CACHE_DIR"

prepare_emulation() {
    mount -t binfmt_misc binfmt_misc /proc/sys/fs/binfmt_misc || true
    # Keep Docker Desktop's shared ARM64 registration intact.
    local probe_dir
    probe_dir=$(mktemp -d)
    arch-test -c "$probe_dir" arm64
    rmdir "$probe_dir" 2>/dev/null || true
}

start_io_helper() {
    (
        while :; do
            local_root=$WORK_DIR/stage0/rootfs
            local_config=$local_root/etc/dpkg/dpkg.cfg.d/00-openstreamer-test-build-io
            if [ -d "$(dirname "$local_config")" ] && [ -f "$local_root/usr/bin/dpkg" ] &&
                { [ -f "$local_root/debootstrap/debootstrap" ] || [ -f /pi-gen/stage0/SKIP ]; }; then
                [ -f "$local_config" ] || printf 'force-unsafe-io\n' > "$local_config"
                local_library=/usr/local/lib/libopenstreamer-test-eatmydata.so
                if [ ! -f "$local_root$local_library" ]; then
                    install -D -m 644 /pi-gen/test-tools/eatmydata-arm64/usr/lib/aarch64-linux-gnu/libeatmydata.so.1.3.0 "$local_root$local_library"
                fi
                if ! grep -qxF "$local_library" "$local_root/etc/ld.so.preload" 2>/dev/null; then
                    printf '%s\n' "$local_library" >> "$local_root/etc/ld.so.preload"
                fi
            fi
            sleep 2
        done
    ) &
    IO_HELPER_PID=$!
    trap 'kill "${IO_HELPER_PID:-}" 2>/dev/null || true' EXIT
}

if [ "$OPENSTREAMER_TEST_REUSE" != 1 ]; then
    prepare_emulation
    # A failed replacement must not leave the old base marked complete.
    rm -f "$BASE_MARKER" "$BASE_INPUTS" "$BASE_GENERATION"
    rm -f "$WORK_DIR/stage0/rootfs/debootstrap/debootstrap"
    cp /pi-gen/config /tmp/openstreamer-test-base-config
    printf '\nSTAGE_LIST="stage0 stage1 stage2"\nCLEAN=1\n' >> /tmp/openstreamer-test-base-config
    start_io_helper
    ./build.sh -c /tmp/openstreamer-test-base-config
    test -f "$WORK_DIR/stage2/rootfs/etc/os-release"
    printf '%s\n' "$OPENSTREAMER_TEST_BASE_INPUT_KEY" > "$BASE_INPUTS"
    cat /proc/sys/kernel/random/uuid > "$BASE_GENERATION"
    touch "$BASE_MARKER"
    if [ "$OPENSTREAMER_TEST_BASE_ONLY" = 1 ]; then
        rsync -a "$WORK_DIR/build.log" "$DEPLOY_DIR/"
        exit 0
    fi
else
    test -f "$BASE_MARKER"
    if [ -f "$BASE_INPUTS" ] && [ "$(cat "$BASE_INPUTS")" != "$OPENSTREAMER_TEST_BASE_INPUT_KEY" ]; then
        echo '[cache] base MISS: base configuration or pinned pi-gen revision changed. Run --base first.' >&2
        exit 1
    fi
    if [ ! -f "$BASE_INPUTS" ]; then
        # Adopt the project's earlier completed base without an OS rebuild.
        # Its marker and volume creation time identify this legacy generation.
        printf '%s\n' "$OPENSTREAMER_TEST_BASE_INPUT_KEY" > "$BASE_INPUTS"
        echo '[cache] base HIT: adopted the existing completed isolated base.'
    else
        echo '[cache] base HIT: configuration and completed generation match.'
    fi
fi

if [ -f "$BASE_GENERATION" ]; then
    BASE_ID=$(cat "$BASE_GENERATION")
else
    BASE_ID=$(test_cache_key legacy "$OPENSTREAMER_TEST_WORK_VOLUME_CREATED" "$(stat -c '%y:%s:%i' "$BASE_MARKER")")
fi
export OPENSTREAMER_TEST_RUNTIME_KEY
export OPENSTREAMER_TEST_APP_KEY
OPENSTREAMER_TEST_RUNTIME_KEY=$(test_cache_key runtime-v1 "$BASE_ID" "$OPENSTREAMER_TEST_RUNTIME_INPUT_KEY")
OPENSTREAMER_TEST_APP_KEY=$(test_cache_key app-v1 "$OPENSTREAMER_TEST_RUNTIME_KEY" "$OPENSTREAMER_TEST_APP_INPUT_KEY" "$OPENSTREAMER_TEST_PACKAGE_SHA256")
IMAGE_KEY=$(test_cache_key image-v1 "$OPENSTREAMER_TEST_APP_KEY" "$OPENSTREAMER_TEST_EXPORT_INPUT_KEY")
IMAGE_NAME=Openstreamer-${IMAGE_KEY}.img.xz
IMAGE_HASH=$CACHE_DIR/$IMAGE_KEY.sha256

if test_cache_image_matches "$DEPLOY_DIR/$IMAGE_NAME" "$IMAGE_HASH"; then
    echo '[cache] image HIT: all inputs and the compressed image SHA256 match; export and compression skipped.'
    printf '%s\n' "$IMAGE_NAME" > "$DEPLOY_DIR/.openstreamer-test-result"
    exit 0
fi
echo '[cache] image MISS: inputs changed, cached image absent, or its SHA256 does not match.'

if [ "$OPENSTREAMER_TEST_REUSE" = 1 ]; then
    prepare_emulation
    start_io_helper
fi
STAGES=$(mktemp -d /tmp/openstreamer-test-stages.XXXXXX)
cp -R /pi-gen/test-tools/stage-openstreamer-runtime "$STAGES/"
cp -R /pi-gen/stage-openstreamer "$STAGES/"
RUNTIME_STAGE=$STAGES/stage-openstreamer-runtime
APP_STAGE=$STAGES/stage-openstreamer
# Runtime installs dependencies once; the application stage installs only the
# local package and always reapplies the image's current configuration.
rm -f "$APP_STAGE/00-openstreamer/00-packages"
cp /pi-gen/test-tools/cache-prerun-app.sh "$APP_STAGE/prerun.sh"
mkdir -p "$APP_STAGE/99-zz-test-cache-complete"
cp /pi-gen/test-tools/cache-complete-app.sh "$APP_STAGE/99-zz-test-cache-complete/00-run.sh"
if test_cache_stage_matches "$WORK_DIR/stage-openstreamer-runtime" "$OPENSTREAMER_TEST_RUNTIME_KEY"; then
    echo '[cache] runtime HIT: base generation, runtime scripts and package dependencies match.'
    touch "$RUNTIME_STAGE/SKIP"
else
    echo '[cache] runtime MISS: build a pristine dependency rootfs from the completed base.'
fi
if [ -f "$RUNTIME_STAGE/SKIP" ] && test_cache_stage_matches "$WORK_DIR/stage-openstreamer" "$OPENSTREAMER_TEST_APP_KEY"; then
    echo '[cache] application HIT: package bytes and image configuration match.'
    touch "$APP_STAGE/SKIP"
else
    echo '[cache] application MISS: restore the runtime baseline and install the current package.'
fi
find "$STAGES" -type f -name '*.sh' -exec chmod +x {} +
cp /pi-gen/config /tmp/openstreamer-test-config
{
    printf '\nSTAGE_LIST="%s %s"\n' "$RUNTIME_STAGE" "$APP_STAGE"
    printf 'CLEAN=0\n'
    printf 'IMG_FILENAME="Openstreamer-%s"\nARCHIVE_FILENAME="Openstreamer-%s"\n' "$IMAGE_KEY" "$IMAGE_KEY"
} >> /tmp/openstreamer-test-config
export OPENSTREAMER_TEST_RUNTIME_READY=1
./build.sh -c /tmp/openstreamer-test-config
test -f "$DEPLOY_DIR/$IMAGE_NAME"
xz --test "$DEPLOY_DIR/$IMAGE_NAME"
(cd "$DEPLOY_DIR" && sha256sum "$IMAGE_NAME") > "$IMAGE_HASH.tmp"
mv -f "$IMAGE_HASH.tmp" "$IMAGE_HASH"
printf '%s\n' "$IMAGE_NAME" > "$DEPLOY_DIR/.openstreamer-test-result"
rsync -a "$WORK_DIR/build.log" "$DEPLOY_DIR/"
