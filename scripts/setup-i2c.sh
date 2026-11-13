#!/usr/bin/env bash
# Phase 1: give OpenRGB access to SMBus/I2C devices (motherboard and RAM lighting).
set -euo pipefail
sudo modprobe i2c-dev
sudo modprobe i2c-piix4 || true
printf 'i2c-dev\ni2c-piix4\n' | sudo tee /etc/modules-load.d/i2c.conf >/dev/null
sudo usermod -aG i2c "$USER"
sudo udevadm control --reload
sudo udevadm trigger
echo "Done. Log out and back in (or use 'sg i2c') for the i2c group to apply."
