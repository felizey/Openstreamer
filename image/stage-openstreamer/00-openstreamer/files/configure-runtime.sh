#!/bin/sh
set -eu

release=$1
# Tailscale is distributed through its own repository.
curl -fsSL "https://pkgs.tailscale.com/stable/debian/${release}.noarmor.gpg" \
    -o /usr/share/keyrings/tailscale-archive-keyring.gpg
curl -fsSL "https://pkgs.tailscale.com/stable/debian/${release}.tailscale-keyring.list" \
    -o /etc/apt/sources.list.d/tailscale.list
apt-get update
apt-get install -y tailscale
