#!/usr/bin/env bash
# Exercise only newly created file-backed loop devices in an isolated volume.
# No host /dev bind, physical disks, or other pi-gen work volumes are used.
set -euo pipefail

if [ "${1:-}" = --inside ]; then
    source /pi-gen/scripts/common
    export OPENSTREAMER_TEST_LOOP_LOG=/work/owned-loops
    : > "$OPENSTREAMER_TEST_LOOP_LOG"
    cleanup() {
        local device image backing target
        while IFS=$'\t' read -r device image; do
            case "$device:$image" in /dev/loop[0-9]*:/work/*.img) ;; *) continue ;; esac
            backing=$(losetup -n -O BACK-FILE "$device" 2>/dev/null || true)
            [ "$backing" = "$image" ] || continue
            # Only mounts backed by this run's verified image are eligible.
            while IFS= read -r target; do
                [ -z "$target" ] || umount "$target" || true
            done < <(findmnt -rn -o TARGET --source "$device" | sort -r)
            while IFS= read -r target; do
                [ -z "$target" ] || umount "$target" || true
            done < <(findmnt -rn -o SOURCE,TARGET | awk -v dev="$device" '$1 ~ "^" dev "p[0-9]+$" {print $2}' | sort -r)
            losetup --detach "$device" || true
        done < "$OPENSTREAMER_TEST_LOOP_LOG"
    }
    trap cleanup EXIT
    cp /test-source/stage-openstreamer/01-data-partition/files/openstreamer-data /work/openstreamer-data
    # There is no host udev/dev bind. After partition-table changes, create
    # nodes only for a loop whose backing file is recorded by this test run.
    export OPENSTREAMER_TEST_PARTX
    OPENSTREAMER_TEST_PARTX=$(command -v partx)
    mkdir -p /work/test-bin
    cat > /work/test-bin/refresh-loop <<'REFRESH'
#!/usr/bin/env bash
set -euo pipefail
DEVICE=$1
BACKING=$(losetup -n -O BACK-FILE "$DEVICE" 2>/dev/null || true)
case "$DEVICE:$BACKING" in /dev/loop[0-9]*:/work/*.img) ;; *) exit 1 ;; esac
grep -qxF "$DEVICE"$'\t'"$BACKING" "$OPENSTREAMER_TEST_LOOP_LOG"
# Removing container-local nodes cannot affect mounts. Recreate them from
# current sysfs so partition minors reused after a reflash cannot stay stale.
for NODE in "${DEVICE}"p[0-9]*; do
    [ ! -b "$NODE" ] || rm -- "$NODE"
done
source /pi-gen/scripts/common
ensure_loopdev_partitions "$DEVICE"
REFRESH
    cat > /work/test-bin/partx <<'PARTX'
#!/usr/bin/env bash
set -euo pipefail
"$OPENSTREAMER_TEST_PARTX" "$@"
DEVICE=${!#}
bash /work/test-bin/refresh-loop "$DEVICE"
PARTX
    chmod +x /work/test-bin/partx /work/test-bin/refresh-loop
    ensure_loopdev_partitions() { bash /work/test-bin/refresh-loop "$@"; }
    export PATH="/work/test-bin:$PATH"
    # The cases intentionally run failed operations to check recovery paths.
    set +e
    source /test-source/tests/data-partition-cases.sh
    exit
fi

HERE=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
SOURCE=$(dirname "$HERE")
IMAGE=${OPENSTREAMER_TEST_TOOLING_IMAGE:-openstreamer-test-pigen:4d8ee447dd3d37e8b0ef8752e460d9082d9d435d}
docker image inspect "$IMAGE" >/dev/null || { echo "Run bash image/build-test.sh --prepare first." >&2; exit 1; }
CONTAINER=openstreamer-test-data-partition-v011
VOLUME=openstreamer-test-data-partition-v011
OWNER=org.openstreamer.test-builder
if docker container inspect "$CONTAINER" >/dev/null 2>&1; then
    [ "$(docker inspect -f '{{index .Config.Labels "org.openstreamer.test-builder"}}' "$CONTAINER")" = 1 ]
    [ "$(docker inspect -f '{{.State.Running}}' "$CONTAINER")" = false ]
    docker rm "$CONTAINER" >/dev/null
fi
if docker volume inspect "$VOLUME" >/dev/null 2>&1; then
    [ "$(docker volume inspect -f '{{index .Labels "org.openstreamer.test-builder"}}' "$VOLUME")" = 1 ]
else
    docker volume create --label "$OWNER=1" "$VOLUME" >/dev/null
fi
mkdir -p "$SOURCE/deploy"
docker run --name "$CONTAINER" --privileged --label "$OWNER=1" \
    --mount "type=bind,source=$SOURCE/.pi-gen-openstreamer-test,target=/pi-gen,readonly" \
    --mount "type=bind,source=$SOURCE,target=/test-source,readonly" \
    --mount "type=volume,source=$VOLUME,target=/work" \
    "$IMAGE" bash /test-source/tests/data-partition-isolated.sh --inside \
    | tee "$SOURCE/deploy/openstreamer-test-data-partition-v011.log"
