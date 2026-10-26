#!/bin/bash -e

# Raspberry Pi OS Lite includes useful general-purpose tools and hardware
# stacks which Openstreamer does not use. Keep this after the data-partition
# stage so cleanup runs against the complete runtime root filesystem.
on_chroot <<'TRIM_RUNTIME'
set -euo pipefail

packages=(
	strace ed ncdu bash-completion debconf-utils
	python-is-python3 v4l-utils
	gpiod python3-libgpiod python3-gpiozero python3-rpi-lgpio
	python3-spidev python3-smbus2 lua5.1 luajit
	apt-listchanges htop ssh-import-id ethtool pciutils rsync
	usb-modeswitch libmtp-runtime
	rpi-keyboard-config rpi-keyboard-fw-update
	unzip zip p7zip-full file kms++-utils
	bluez bluez-firmware cifs-utils rpicam-apps-lite
)

protected_packages=(
	openstreamer network-manager openssh-server avahi-daemon
	gstreamer1.0-plugins-base gstreamer1.0-plugins-good
	gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly
	gstreamer1.0-libav ffmpeg udisks2 wireguard-tools tailscale
	dnsmasq-base wpasupplicant iw iproute2 kmod util-linux passwd
	systemd curl ca-certificates tzdata rpi-usb-gadget
	raspberrypi-net-mods raspi-firmware raspi-utils linux-image-rpi-v8
	firmware-brcm80211 tailscale-archive-keyring
)

is_installed() {
	[ "$(dpkg-query -W -f='${db:Status-Status}' -- "$1" 2>/dev/null || true)" = installed ]
}

would_remove_protected() {
	local action candidate protected
	while read -r action candidate _; do
		case "$action" in
			Remv|Purg)
				candidate=${candidate%%:*}
				for protected in "${protected_packages[@]}"; do
					if [ "$candidate" = "$protected" ]; then
						echo "[trim] APT plan would remove protected package: $candidate"
						return 0
					fi
				done
				;;
		esac
	done < "$1"
	return 1
}

protected_installed=()
for package in "${protected_packages[@]}"; do
	if is_installed "$package"; then
		protected_installed+=("$package")
	fi
done
if [ "${#protected_installed[@]}" -gt 0 ]; then
	apt-mark manual "${protected_installed[@]}"
fi

# Query each allowlisted package separately. Before purging, simulate APT's
# full dependency resolution because removing a package can also remove the
# application or Raspberry Pi runtime packages that depend on it.
installed=()
for package in "${packages[@]}"; do
	if is_installed "$package"; then
		installed+=("$package")
	fi
done
if [ "${#installed[@]}" -gt 0 ]; then
	plan=$(mktemp)
	if ! apt-get -s purge "${installed[@]}" >"$plan"; then
		echo "[trim] skipping package cleanup: APT could not simulate the purge safely"
	elif would_remove_protected "$plan"; then
		echo "[trim] skipping package cleanup to preserve the Openstreamer runtime"
	else
		apt-get purge -y "${installed[@]}"
		autoremove_plan=$(mktemp)
		if ! apt-get -s autoremove --purge >"$autoremove_plan"; then
			echo "[trim] skipping autoremove: APT could not simulate it safely"
		elif would_remove_protected "$autoremove_plan"; then
			echo "[trim] skipping autoremove to preserve the Openstreamer runtime"
		else
			apt-get autoremove --purge -y
		fi
		rm -f "$autoremove_plan"
	fi
	rm -f "$plan"
fi
apt-get clean

for package in openstreamer network-manager openssh-server avahi-daemon \
		gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
		gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly \
		gstreamer1.0-libav ffmpeg udisks2 linux-image-rpi-v8 firmware-brcm80211 \
		wireguard-tools tailscale; do
	[ "$(dpkg-query -W -f='${db:Status-Status}' "$package")" = installed ]
done
command -v vcgencmd >/dev/null
command -v udisksctl >/dev/null
command -v wg >/dev/null
command -v tailscale >/dev/null
test -s /usr/share/keyrings/tailscale-archive-keyring.gpg
TRIM_RUNTIME
