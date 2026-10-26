#!/bin/bash -e

# Keep the image small by skipping package recommendations and documentation.
install -m 644 /dev/stdin "${ROOTFS_DIR}/etc/apt/apt.conf.d/90-openstreamer" <<- EOF
	APT::Install-Recommends "false";
	APT::Install-Suggests "false";
	APT::AutoRemove::RecommendsImportant "false";
	APT::AutoRemove::SuggestsImportant "false";
EOF
install -m 644 /dev/stdin "${ROOTFS_DIR}/etc/dpkg/dpkg.cfg.d/90-openstreamer" <<- EOF
	path-exclude=/usr/share/doc/*
	path-include=/usr/share/doc/*/copyright
	path-exclude=/usr/share/man/*
	path-exclude=/usr/share/info/*
	path-exclude=/usr/share/lintian/*
	path-exclude=/usr/share/locale/*
	path-include=/usr/share/locale/en*
	path-include=/usr/share/locale/locale.alias
EOF

# Applies current image credentials even when stage2 came from the cached base.
on_chroot -s -- "$FIRST_USER_NAME" "$FIRST_USER_PASS" < files/configure-account.sh

# Where the updater keeps the running version's package, to go back to if a
# later one doesn't start. Not /tmp: the chroot mounts its own over it
install -d "${ROOTFS_DIR}/var/lib/openstreamer/update"
install -m 644 files/openstreamer.deb "${ROOTFS_DIR}/var/lib/openstreamer/update/current.deb"
on_chroot <<- EOF
	apt-get install -y /var/lib/openstreamer/update/current.deb
EOF
cmp files/update-key.pub "${ROOTFS_DIR}/usr/share/openstreamer/update-key.pub"

# Tells Openstreamer the whole device is its own
install -d "${ROOTFS_DIR}/etc/openstreamer"
touch "${ROOTFS_DIR}/etc/openstreamer/image"

# The test image builder can provide a pristine cached runtime with Tailscale
# already installed. Normal production pi-gen builds configure it here.
if [ "${OPENSTREAMER_TEST_RUNTIME_READY:-0}" != 1 ]; then
    on_chroot -s -- "$RELEASE" < files/configure-runtime.sh
fi

# Raspberry Pi OS Lite includes compilers, Pi 5 support, and unused firmware.
on_chroot <<- 'EOF'
	set -e
	# The export-image stage refreshes APT sources, so retain the Tailscale
	# archive keyring even though no installed package depends on it directly.
	if [ "$(dpkg-query -W -f='${db:Status-Status}' tailscale-archive-keyring 2>/dev/null || true)" = installed ]; then
		apt-mark manual tailscale-archive-keyring
	fi
	purge=$(dpkg-query -W -f='${db:Status-Abbrev} ${Package}\n' \
		build-essential gcc g++ cpp gcc-14 g++-14 cpp-14 \
		gcc-aarch64-linux-gnu g++-aarch64-linux-gnu gcc-14-aarch64-linux-gnu \
		g++-14-aarch64-linux-gnu cpp-14-aarch64-linux-gnu libstdc++-14-dev libgcc-14-dev \
		binutils binutils-common gdb manpages-dev pkg-config python3-venv \
		'linux-headers-*' 'linux-image-*rpi-2712' \
		cloud-init rpi-cloud-init-mods rpi-connect-lite mkvtoolnix \
		firmware-atheros firmware-mediatek firmware-libertas 2>/dev/null |
		awk '/^ii/ { print $2 }')
	apt-get purge -y $purge
	apt-get autoremove --purge -y
	apt-get clean
EOF
rm -f "${ROOTFS_DIR}/boot/firmware/kernel_2712.img" "${ROOTFS_DIR}/boot/firmware/initramfs_2712"
find "${ROOTFS_DIR}/usr/share/doc" -mindepth 1 ! -type d ! -name copyright -delete
find "${ROOTFS_DIR}/usr/share/doc" -mindepth 1 -type d -empty -delete
rm -rf "${ROOTFS_DIR}"/usr/share/man/* "${ROOTFS_DIR}"/usr/share/info/* "${ROOTFS_DIR}"/usr/share/lintian/*
find "${ROOTFS_DIR}/usr/share/locale" -mindepth 1 -maxdepth 1 ! -name 'en*' ! -name locale.alias -exec rm -rf {} +

# An image without these would boot into nothing useful
grep -q '^dtoverlay=dwc2,dr_mode=peripheral' "${ROOTFS_DIR}/boot/firmware/config.txt"
test -L "${ROOTFS_DIR}/etc/systemd/system/multi-user.target.wants/openstreamer.service"
test -x "${ROOTFS_DIR}/usr/bin/wg"
test -x "${ROOTFS_DIR}/usr/bin/tailscale"
test -L "${ROOTFS_DIR}/etc/systemd/system/multi-user.target.wants/tailscaled.service"
for pkg in openstreamer network-manager openssh-server gstreamer1.0-plugins-bad gstreamer1.0-libav linux-image-rpi-v8 firmware-brcm80211; do
	dpkg-query --admindir="${ROOTFS_DIR}/var/lib/dpkg" -W -f='${db:Status-Abbrev}' "$pkg" | grep -q '^ii'
done
test ! -e "${ROOTFS_DIR}/usr/bin/gcc"
test ! -e "${ROOTFS_DIR}/boot/firmware/kernel_2712.img"
