#!/bin/bash -e

install -d "${ROOTFS_DIR}/usr/lib/openstreamer"
install -m 755 files/openstreamer-data "${ROOTFS_DIR}/usr/lib/openstreamer/openstreamer-data"
install -m 644 files/openstreamer-data.service "${ROOTFS_DIR}/etc/systemd/system/openstreamer-data.service"
on_chroot <<- EOF
	systemctl enable openstreamer-data.service
EOF

# Raspberry Pi OS would stretch the system partition over the whole card on
# the first start, leaving no room for the data partition
sed -i 's/ resize\b//' "${ROOTFS_DIR}/boot/firmware/cmdline.txt"
! grep -qw resize "${ROOTFS_DIR}/boot/firmware/cmdline.txt"
