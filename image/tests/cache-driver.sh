#!/usr/bin/env bash
# Execute in a disposable container with /pi-gen backed only by tmpfs.
set -euo pipefail
HERE=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
source "$HERE/test-cache.sh"
fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
[ "${OPENSTREAMER_CACHE_TEST_TMPFS:-}" = 1 ] || fail 'this test requires its disposable tmpfs container'
[ "$(findmnt -nr -o FSTYPE /pi-gen)" = tmpfs ] || fail '/pi-gen must be tmpfs, not a real builder workspace'
mkdir -p /pi-gen/test-tools /pi-gen/mock-bin /pi-gen/work/Openstreamer/stage2/rootfs/{etc,var/lib/dpkg,usr/bin}
cp "$HERE/test-cache.sh" "$HERE/test-build-container.sh" "$HERE/cache-prerun-app.sh" "$HERE/cache-complete-app.sh" /pi-gen/test-tools/
cp -R "$HERE/test-runtime" /pi-gen/test-tools/stage-openstreamer-runtime
cp -R "$HERE/stage-openstreamer" /pi-gen/stage-openstreamer
printf 'IMG_NAME=Openstreamer\nCLEAN=1\nRELEASE=trixie\n' > /pi-gen/config
printf 'fixture' > /pi-gen/work/Openstreamer/stage2/rootfs/etc/os-release
printf 'fixture' > /pi-gen/work/Openstreamer/stage2/rootfs/var/lib/dpkg/status
printf 'baseline' > /pi-gen/work/Openstreamer/stage2/rootfs/etc/shared
touch /pi-gen/work/Openstreamer/.openstreamer-test-base-ready
printf 'base-input-one\n' > /pi-gen/work/Openstreamer/.openstreamer-test-base-inputs
printf 'generation-one\n' > /pi-gen/work/Openstreamer/.openstreamer-test-base-generation
for command in mount arch-test; do
    printf '#!/bin/sh\nexit 0\n' > "/pi-gen/mock-bin/$command"
    chmod +x "/pi-gen/mock-bin/$command"
done
export PATH=/pi-gen/mock-bin:$PATH
export OPENSTREAMER_TEST_REUSE=1 OPENSTREAMER_TEST_BASE_ONLY=0
export OPENSTREAMER_TEST_WORK_VOLUME_CREATED=fixture-created
export OPENSTREAMER_TEST_BASE_INPUT_KEY=base-input-one
export OPENSTREAMER_TEST_RUNTIME_INPUT_KEY=runtime-input-one
export OPENSTREAMER_TEST_APP_INPUT_KEY=app-input-one
export OPENSTREAMER_TEST_EXPORT_INPUT_KEY=export-input-one
STAGED_PACKAGE=/pi-gen/stage-openstreamer/00-openstreamer/files/openstreamer.deb
set_package() {
    printf '%s\n' "$1" > "$STAGED_PACKAGE"
    export OPENSTREAMER_TEST_PACKAGE_SHA256
    OPENSTREAMER_TEST_PACKAGE_SHA256=$(sha256sum "$STAGED_PACKAGE" | cut -d ' ' -f 1)
}
set_package package-one
export MOCK_APP_ASSET=first-app
cat > /pi-gen/build.sh <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
source "$2"
export WORK_DIR=/pi-gen/work/Openstreamer
printf 'build\n' >> /pi-gen/mock-build-count
for STAGE_DIR in $STAGE_LIST; do
    export STAGE_DIR STAGE_WORK_DIR=$WORK_DIR/$(basename "$STAGE_DIR")
    export ROOTFS_DIR=$STAGE_WORK_DIR/rootfs
    [ ! -f "$STAGE_DIR/SKIP" ] || continue
    printf '%s\n' "$(basename "$STAGE_DIR")" >> /pi-gen/mock-stage-count
    bash "$STAGE_DIR/prerun.sh"
    if [ "$(basename "$STAGE_DIR")" = stage-openstreamer-runtime ]; then
        printf 'runtime only' > "$ROOTFS_DIR/usr/bin/runtime-fixture"
        bash "$STAGE_DIR/99-test-cache-complete/00-run.sh"
    else
        printf 'app' > "$ROOTFS_DIR/usr/bin/$MOCK_APP_ASSET"
        [ "${MOCK_FAIL_APP_AFTER_WRITE:-0}" != 1 ] || exit 57
        bash "$STAGE_DIR/99-zz-test-cache-complete/00-run.sh"
    fi
done
printf 'mock image %s' "$IMG_FILENAME" | xz > "/pi-gen/deploy/$ARCHIVE_FILENAME.img.xz"
printf 'mock log' > "$WORK_DIR/build.log"
MOCK
chmod +x /pi-gen/build.sh

bash /pi-gen/test-tools/test-build-container.sh > /tmp/cache-first.log
[ "$(wc -l < /pi-gen/mock-build-count)" = 1 ] || fail 'initial miss must build once'
[ "$(wc -l < /pi-gen/mock-stage-count)" = 2 ] || fail 'initial miss must execute runtime and app'
bash /pi-gen/test-tools/test-build-container.sh > /tmp/cache-identical.log
grep -q 'image HIT' /tmp/cache-identical.log || fail 'identical run must hit whole image'
[ "$(wc -l < /pi-gen/mock-build-count)" = 1 ] || fail 'whole image hit must not run pi-gen'

# Validate the staged bytes even if the requested image already exists.
printf 'unexpected replacement' >> "$STAGED_PACKAGE"
if bash /pi-gen/test-tools/test-build-container.sh > /tmp/cache-wrong-package.log 2>&1; then fail 'mismatched staged package must be rejected'; fi
grep -q 'does not match the requested SHA256' /tmp/cache-wrong-package.log || fail 'staged package mismatch needs an actionable error'
[ "$(wc -l < /pi-gen/mock-build-count)" = 1 ] || fail 'package mismatch must not invoke pi-gen'
set_package package-two
export MOCK_APP_ASSET=second-app
bash /pi-gen/test-tools/test-build-container.sh > /tmp/cache-new-package.log
grep -q 'runtime HIT' /tmp/cache-new-package.log || fail 'new binary must reuse runtime'
[ "$(wc -l < /pi-gen/mock-stage-count)" = 3 ] || fail 'new binary must execute only app stage'
[ ! -e /pi-gen/work/Openstreamer/stage-openstreamer/rootfs/usr/bin/first-app ] || fail 'old package assets must be removed'
[ -f /pi-gen/work/Openstreamer/stage-openstreamer/rootfs/usr/bin/second-app ] || fail 'new package must be applied'

export OPENSTREAMER_TEST_RUNTIME_INPUT_KEY=runtime-input-two
bash /pi-gen/test-tools/test-build-container.sh > /tmp/cache-new-deps.log
grep -q 'runtime MISS' /tmp/cache-new-deps.log || fail 'dependency change must rebuild runtime'
[ "$(wc -l < /pi-gen/mock-stage-count)" = 5 ] || fail 'dependency change must execute both stages'

printf 'generation-two\n' > /pi-gen/work/Openstreamer/.openstreamer-test-base-generation
bash /pi-gen/test-tools/test-build-container.sh > /tmp/cache-new-base.log
grep -q 'runtime MISS' /tmp/cache-new-base.log || fail 'base generation change must rebuild runtime'
image=$(cat /pi-gen/deploy/.openstreamer-test-result)
printf 'corrupt' >> "/pi-gen/deploy/$image"
before=$(wc -l < /pi-gen/mock-stage-count)
bash /pi-gen/test-tools/test-build-container.sh > /tmp/cache-corrupt.log
grep -q 'image MISS' /tmp/cache-corrupt.log || fail 'corrupt whole image must miss'
grep -q 'application HIT' /tmp/cache-corrupt.log || fail 'corrupt image must still reuse completed app rootfs'
[ "$(wc -l < /pi-gen/mock-stage-count)" = "$before" ] || fail 'corrupt image should rerun only export'

# Complete A -> failed partial B -> request A again without its whole-image
# artifact. A stale stage marker must not allow exporting B's partial files.
old_image=$(cat /pi-gen/deploy/.openstreamer-test-result)
set_package broken-package
export MOCK_APP_ASSET=broken-app MOCK_FAIL_APP_AFTER_WRITE=1
if bash /pi-gen/test-tools/test-build-container.sh > /tmp/cache-failed-app.log 2>&1; then fail 'the mock partial application build must fail'; fi
[ ! -f /pi-gen/work/Openstreamer/stage-openstreamer/.openstreamer-test-cache-key ] || fail 'partial app build must lose its old success marker'
rm "/pi-gen/deploy/$old_image"
set_package package-two
export MOCK_APP_ASSET=second-app MOCK_FAIL_APP_AFTER_WRITE=0
bash /pi-gen/test-tools/test-build-container.sh > /tmp/cache-rollback.log
grep -q 'application MISS' /tmp/cache-rollback.log || fail 'rollback after partial app failure must rebuild app'
[ ! -e /pi-gen/work/Openstreamer/stage-openstreamer/rootfs/usr/bin/broken-app ] || fail 'partial app files must disappear after recovery'

export OPENSTREAMER_TEST_BASE_INPUT_KEY=changed-base-configuration
if bash /pi-gen/test-tools/test-build-container.sh > /tmp/cache-base-mismatch.log 2>&1; then fail 'changed base config must require --base'; fi
grep -q 'Run --base first' /tmp/cache-base-mismatch.log || fail 'changed base config needs actionable message'
export OPENSTREAMER_TEST_BASE_INPUT_KEY=base-input-one
rm /pi-gen/work/Openstreamer/.openstreamer-test-base-inputs /pi-gen/work/Openstreamer/.openstreamer-test-base-generation
bash /pi-gen/test-tools/test-build-container.sh > /tmp/cache-legacy-base.log
grep -q 'adopted the existing' /tmp/cache-legacy-base.log || fail 'legacy completed base must work without an OS rebuild'
printf 'PASS: warm cache, identical image, package/dependency/base changes, corruption recovery, failed app rollback, legacy adoption\n'
