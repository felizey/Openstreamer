#!/usr/bin/env bash
# Verify the delivered archive against its own file-backed image and .deb.
# No service is started and no physical disk is mounted.
set -euo pipefail

if [ "${1:-}" = --inside ]; then
    OUTPUT="/artifacts/$2"
    source /pi-gen/scripts/common
    RAW=$(find /pi-gen/work/Openstreamer/export-image -maxdepth 1 -name '*.img' \
        -type f -printf '%T@ %p\n' | sort -nr | head -1 | cut -d' ' -f2-)
    test -n "$RAW"
    echo "Checking compressed image integrity and its raw image SHA-256..."
    ARCHIVE_RAW_HASH=$(xz --decompress --stdout "$OUTPUT" | sha256sum | cut -d' ' -f1)
    RAW_HASH=$(sha256sum "$RAW" | cut -d' ' -f1)
    [ "$ARCHIVE_RAW_HASH" = "$RAW_HASH" ]
    sfdisk --dump "$RAW"

    LOOP=
    ROOT=$(mktemp -d)
    BOOT=$(mktemp -d)
    cleanup() {
        mountpoint -q "$BOOT" && umount "$BOOT" || true
        mountpoint -q "$ROOT" && umount "$ROOT" || true
        [ -z "$LOOP" ] || losetup --detach "$LOOP"
        rmdir "$BOOT" "$ROOT"
    }
    trap cleanup EXIT
    for ATTEMPT in 1 2 3 4 5; do
        ensure_next_loopdev
        if LOOP=$(losetup --find --show --read-only --partscan "$RAW"); then
            break
        fi
        sleep 1
    done
    test -n "$LOOP"
    ensure_loopdev_partitions "$LOOP"
    mount -o ro,noload "${LOOP}p2" "$ROOT"
    mount -o ro "${LOOP}p1" "$BOOT"

    grep -qx openstreamer "$ROOT/etc/hostname"
    grep -q '^openstreamer:' "$ROOT/etc/passwd"
    [ "$(awk -F: '$3 == 1000 { print $1 }' "$ROOT/etc/passwd")" = openstreamer ]
    grep -q '^openstreamer:' "$ROOT/etc/group"
    test -d "$ROOT/home/openstreamer"
    grep -q '^    name: openstreamer$' "$ROOT/etc/cloud/cloud.cfg.d/99-openstreamer-user.cfg"
    # Verify the cached user's password was replaced without printing its hash
    # or invoking login/PAM, which can create runtime files.
    chroot "$ROOT" /usr/bin/python3 - <<'PASSWORD_CHECK'
import ctypes
from pathlib import Path

stored = next(line.split(":")[1] for line in Path("/etc/shadow").read_text().splitlines()
              if line.startswith("openstreamer:"))
crypt = ctypes.CDLL("libcrypt.so.1").crypt
crypt.argtypes = [ctypes.c_char_p, ctypes.c_char_p]
crypt.restype = ctypes.c_char_p
assert crypt(b"openstreamer", stored.encode()) == stored.encode()
print("Verified: openstreamer account password")
PASSWORD_CHECK
    test ! -e "$ROOT/etc/dpkg/dpkg.cfg.d/00-openstreamer-test-build-io"
    test ! -e "$ROOT/usr/local/lib/libopenstreamer-test-eatmydata.so"
    test ! -L "$ROOT/usr/local/lib/libeatmydata.so.1"
    ! grep -q 'openstreamer-test-eatmydata' "$ROOT/etc/ld.so.preload" 2>/dev/null
    ! grep -aq 'libopenstreamer-test-eatmydata' "$ROOT/etc/ld.so.cache"
    ! grep -aq '/usr/local/lib/libeatmydata.so.1' "$ROOT/etc/ld.so.cache"
    test -f "$BOOT/bcm2711-rpi-4-b.dtb"
    test -f "$BOOT/kernel8.img"
    grep -qx 'dtoverlay=dwc2,dr_mode=peripheral' "$BOOT/config.txt"
    grep -qx 'gpu_mem=256' "$BOOT/config.txt"
    ! grep -qw resize "$BOOT/cmdline.txt"
    test -L "$ROOT/etc/systemd/system/multi-user.target.wants/openstreamer.service"
    test -L "$ROOT/etc/systemd/system/multi-user.target.wants/openstreamer-data.service"
    test -L "$ROOT/etc/systemd/system/multi-user.target.wants/tailscaled.service"
    test ! -L "$ROOT/etc/systemd/system/multi-user.target.wants/ssh.service"
    test ! -L "$ROOT/etc/systemd/system/sockets.target.wants/ssh.socket"
    test ! -e "$BOOT/ssh"
    test -f "$ROOT/etc/openstreamer/image"
    test -x "$ROOT/usr/bin/wg"
    test -x "$ROOT/usr/bin/tailscale"
    test -x "$ROOT/usr/bin/ffmpeg"
    test -x "$ROOT/usr/bin/ffprobe"
    test -s "$ROOT/usr/share/keyrings/tailscale-archive-keyring.gpg"
    test -x "$ROOT/usr/bin/vcgencmd"
    test -x "$ROOT/usr/bin/udisksctl"
    test -x "$ROOT/usr/bin/wg"
    for pkg in openstreamer network-manager openssh-server avahi-daemon \
        gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
        gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly \
        gstreamer1.0-libav ffmpeg udisks2 wireguard-tools tailscale; do
        [ "$(dpkg-query --admindir="$ROOT/var/lib/dpkg" -W \
            -f='${db:Status-Status}' "$pkg" 2>/dev/null || true)" = installed ]
    done
    test -x "$ROOT/usr/lib/openstreamer/openstreamer-data"
    test -x "$ROOT/usr/bin/openstreamer"
    test ! -e "$ROOT/usr/bin/ystreamer"
    test ! -e "$ROOT/etc/ystreamer"
    test ! -e "$ROOT/var/lib/ystreamer"
    test ! -e "$ROOT/home/ystreamer"
    test ! -e "$ROOT/lib/systemd/system/ystreamer.service"
    test ! -e "$ROOT/usr/lib/systemd/system/ystreamer.service"
    # ELF e_machine 183 is AArch64; inspect without executing the binary.
    [ "$(od -An -tu2 -j18 -N2 "$ROOT/usr/bin/openstreamer" | tr -d ' ')" = 183 ]
    file "$ROOT/usr/bin/openstreamer"

    PACKAGE_ROOT=$(mktemp -d)
    dpkg-deb --extract /package.deb "$PACKAGE_ROOT"
    cmp "$PACKAGE_ROOT/usr/bin/openstreamer" "$ROOT/usr/bin/openstreamer"
    cmp /package.deb "$ROOT/var/lib/openstreamer/update/current.deb"
    cmp "$PACKAGE_ROOT/usr/share/openstreamer/update-key.pub" "$ROOT/usr/share/openstreamer/update-key.pub"
    grep -aFq "$(tr -d '\r\n' <"$ROOT/usr/share/openstreamer/update-key.pub")" "$ROOT/usr/bin/openstreamer"
    EXPECTED_PACKAGE=$(dpkg-deb -f /package.deb Package)
    EXPECTED_VERSION=$(dpkg-deb -f /package.deb Version)
    COPYRIGHT=usr/share/doc/$EXPECTED_PACKAGE/copyright
    cmp "$PACKAGE_ROOT/$COPYRIGHT" "$ROOT/$COPYRIGHT"
    grep -q 'GNU GENERAL PUBLIC LICENSE' "$ROOT/$COPYRIGHT"
    ! grep -q 'Copyright information missing' "$ROOT/$COPYRIGHT"
    INSTALLED_VERSION=$(dpkg-query --admindir="$ROOT/var/lib/dpkg" \
        --show --showformat='${Version}' "$EXPECTED_PACKAGE")
    [ "$EXPECTED_VERSION" = "$INSTALLED_VERSION" ]
    [ "$(dpkg-query --admindir="$ROOT/var/lib/dpkg" --show --showformat='${Architecture}' "$EXPECTED_PACKAGE")" = arm64 ]
    [ "$(dpkg-query --admindir="$ROOT/var/lib/dpkg" --show --showformat='${Status}' "$EXPECTED_PACKAGE")" = 'install ok installed' ]
    [ "$(dpkg-query --admindir="$ROOT/var/lib/dpkg" --show --showformat='${Status}' ystreamer 2>/dev/null || true)" != 'install ok installed' ]
    # This CLI branch prints only CARGO_PKG_VERSION. It does not start the
    # service, access hardware, write settings, or open network connections.
    RUNTIME_VERSION=$(chroot "$ROOT" /usr/bin/openstreamer version)
    [ "$RUNTIME_VERSION" = "${EXPECTED_VERSION%-*}" ]
    FFMPEG_VERSION=$(chroot "$ROOT" /usr/bin/ffmpeg -version)
    FFPROBE_VERSION=$(chroot "$ROOT" /usr/bin/ffprobe -version)
    printf '%s\n%s\n' "${FFMPEG_VERSION%%$'\n'*}" "${FFPROBE_VERSION%%$'\n'*}"
    printf 'Verified: Openstreamer %s, ARM64, Pi 4B boot files, hostname, enabled services, SSH off, package/binary match and version CLI runs with image libraries.\n' \
        "$INSTALLED_VERSION"
    exit 0
fi

if [ "$#" != 2 ]; then
    echo "Usage: $0 image.img.xz package_arm64.deb" >&2
    exit 2
fi
HERE=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
OUTPUT=$(realpath -- "$1")
DEB=$(realpath -- "$2")
PI_GEN_REF=4d8ee447dd3d37e8b0ef8752e460d9082d9d435d
WORK_VOLUME=openstreamer-test-pigen-work
DOCKER_IMAGE=${OPENSTREAMER_TEST_TOOLING_IMAGE:-openstreamer-test-pigen:$PI_GEN_REF}
mkdir -p "$HERE/.pi-gen-openstreamer-test/work" "$HERE/.pi-gen-openstreamer-test/deploy"
[ "$(docker volume inspect -f '{{index .Labels "org.openstreamer.test-builder"}}' "$WORK_VOLUME")" = 1 ]
docker run --rm --privileged --name openstreamer-test-verify-v011 \
    --label org.openstreamer.test-builder=1 \
    --mount "type=bind,source=$HERE/.pi-gen-openstreamer-test,target=/pi-gen,readonly" \
    --mount "type=volume,source=$WORK_VOLUME,target=/pi-gen/work,readonly" \
    --mount "type=bind,source=$HERE,target=/test-build,readonly" \
    --mount "type=bind,source=$(dirname "$OUTPUT"),target=/artifacts,readonly" \
    --mount "type=bind,source=$DEB,target=/package.deb,readonly" \
    "$DOCKER_IMAGE" bash /test-build/verify-test.sh --inside "$(basename "$OUTPUT")" \
    | tee "$HERE/deploy/openstreamer-test-verify.log"
