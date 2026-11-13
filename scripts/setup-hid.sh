#!/usr/bin/env bash
# Phase 1b: grant plugdev access to USB RGB controllers (ASUS Aura, NZXT, Razer).
set -euo pipefail
sudo install -m 644 "$(dirname "$0")/61-seele-rgb.rules" /etc/udev/rules.d/61-seele-rgb.rules
sudo udevadm control --reload
sudo udevadm trigger --subsystem-match=hidraw --subsystem-match=usb
for d in /dev/hidraw*; do sudo setfacl -m g:plugdev:rw "$d"; done
getfacl -p /dev/hidraw* | grep -E "file|plugdev"
