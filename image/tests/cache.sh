#!/usr/bin/env bash
# Fast filesystem/decision tests; no OS image, apt operation, mounts or host devices.
set -euo pipefail
HERE=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
source "$HERE/test-cache.sh"
TEST_DIR=$(mktemp -d /tmp/openstreamer-cache-unit.XXXXXX)
trap 'rm -rf -- "$TEST_DIR"' EXIT
fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }

# Two independent opens must contend; a successful wrapper keeps its lock
# until its descriptor closes, while an interrupted process releases it.
test_cache_take_lock "$TEST_DIR/locking"
if bash -c 'source "$1"; test_cache_take_lock "$2"' _ "$HERE/test-cache.sh" "$TEST_DIR/locking" 2>/dev/null; then
    fail 'concurrent preparations must not share a checkout'
fi
exec 9>&-
bash -c 'source "$1"; test_cache_take_lock "$2"' _ "$HERE/test-cache.sh" "$TEST_DIR/locking" || fail 'an exited build must release its checkout lock'

mkdir -p "$TEST_DIR/tree"
printf 'one' > "$TEST_DIR/tree/file"
before=$(test_cache_tree_digest "$TEST_DIR/tree")
touch -d '2000-01-01 UTC' "$TEST_DIR/tree/file"
[ "$before" = "$(test_cache_tree_digest "$TEST_DIR/tree")" ] || fail 'timestamps must not invalidate content keys'
mv "$TEST_DIR/tree/file" "$TEST_DIR/tree/renamed"
[ "$before" != "$(test_cache_tree_digest "$TEST_DIR/tree")" ] || fail 'renamed input must invalidate content keys'
before=$(test_cache_tree_digest "$TEST_DIR/tree")
printf 'two' > "$TEST_DIR/tree/renamed"
[ "$before" != "$(test_cache_tree_digest "$TEST_DIR/tree")" ] || fail 'changed input bytes must invalidate content keys'
ln -s first "$TEST_DIR/tree/link"
before=$(test_cache_tree_digest "$TEST_DIR/tree")
ln -sfn second "$TEST_DIR/tree/link"
[ "$before" != "$(test_cache_tree_digest "$TEST_DIR/tree")" ] || fail 'changed symlink targets must invalidate content keys'

WORK=$TEST_DIR/work
BASE=$WORK/stage2/rootfs
RUNTIME=$WORK/stage-openstreamer-runtime/rootfs
APP=$WORK/stage-openstreamer/rootfs
mkdir -p "$BASE/etc" "$BASE/var/lib/dpkg" "$BASE/usr/bin"
printf 'PRETTY_NAME=fixture\n' > "$BASE/etc/os-release"
printf 'Package: fixture\nStatus: install ok installed\n' > "$BASE/var/lib/dpkg/status"
printf 'original' > "$BASE/etc/shared"
test_cache_restore_pristine "$BASE" "$RUNTIME" "$WORK"
test_cache_write_key "$WORK/stage-openstreamer-runtime" runtime-one
test_cache_stage_matches "$WORK/stage-openstreamer-runtime" runtime-one || fail 'complete matching runtime must hit'
if test_cache_stage_matches "$WORK/stage-openstreamer-runtime" runtime-two; then fail 'different runtime key must miss'; fi
test_cache_restore_pristine "$RUNTIME" "$APP" "$WORK"
test_cache_write_key "$WORK/stage-openstreamer" complete-app
printf 'modified' > "$APP/etc/shared"
touch -r "$RUNTIME/etc/shared" "$APP/etc/shared"
printf 'removed package asset' > "$APP/usr/bin/old-app"
mkdir -p "$APP/etc/openstreamer"
printf 'stale settings' > "$APP/etc/openstreamer/settings.json"
test_cache_restore_pristine "$RUNTIME" "$APP" "$WORK"
[ ! -f "$WORK/stage-openstreamer/.openstreamer-test-cache-key" ] || fail 'restore must invalidate the old success marker before modification'
cmp "$RUNTIME/etc/shared" "$APP/etc/shared" || fail 'same-size/same-time mutation must be restored'
[ ! -e "$APP/usr/bin/old-app" ] && [ ! -e "$APP/etc/openstreamer/settings.json" ] || fail 'removed app assets/settings must disappear'
if test_cache_restore_pristine "$RUNTIME" "$TEST_DIR/unrelated" "$WORK" 2>/dev/null; then fail 'foreign sync target must be rejected'; fi
mv "$APP" "$WORK/app-preserved"
ln -s "$WORK/app-preserved" "$APP"
if test_cache_restore_pristine "$RUNTIME" "$APP" "$WORK" 2>/dev/null; then fail 'symlinked sync target must be rejected'; fi
rm "$APP"
mv "$WORK/app-preserved" "$APP"
findmnt() { printf '%s/proc\n' "$APP"; }
if test_cache_restore_pristine "$RUNTIME" "$APP" "$WORK" 2>/dev/null; then fail 'nested mount must be rejected'; fi
unset -f findmnt

printf 'test image' | xz > "$TEST_DIR/image.img.xz"
(cd "$TEST_DIR" && sha256sum image.img.xz) > "$TEST_DIR/image.sha256"
test_cache_image_matches "$TEST_DIR/image.img.xz" "$TEST_DIR/image.sha256" || fail 'matching image SHA256 must hit'
sed 's/image.img.xz/elsewhere.img.xz/' "$TEST_DIR/image.sha256" > "$TEST_DIR/wrong.sha256"
if test_cache_image_matches "$TEST_DIR/image.img.xz" "$TEST_DIR/wrong.sha256"; then fail 'checksum metadata for another file must miss'; fi
printf 'corrupt' >> "$TEST_DIR/image.img.xz"
if test_cache_image_matches "$TEST_DIR/image.img.xz" "$TEST_DIR/image.sha256"; then fail 'corrupt image must miss'; fi
printf 'PASS: content hashes, pristine restore, safe targets, stage keys, artifact integrity\n'
