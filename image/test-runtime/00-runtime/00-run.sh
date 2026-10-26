#!/bin/bash -e

# Install the dependency closure without installing Openstreamer itself. A new
# app binary with identical control fields can reuse this pristine OS rootfs.
on_chroot -s -- "$(cat files/runtime-depends)" "$(sed -f "$SCRIPT_DIR/remove-comments.sed" files/runtime-packages)" <<'INSTALL'
set -eu
set -f
apt-get update
if [ -n "$2" ]; then
    apt-get install -y $2
fi
if [ -n "$1" ]; then
    apt-get satisfy -y "$1"
fi
INSTALL
on_chroot -s -- "$RELEASE" < files/configure-runtime.sh
! chroot "$ROOTFS_DIR" dpkg-query -W -f='${db:Status-Status}' openstreamer 2>/dev/null | grep -qx installed
