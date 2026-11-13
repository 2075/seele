# OpenRGB devices on metaverse

Board: ASUS ROG STRIX X570-E GAMING · GPU: ASUS TUF RTX 3070 Gaming OC

| # | Device | Bus | Zones (LEDs) | Default mode |
|---|---|---|---|---|
| 0 | ASUS TUF RTX 3070 Gaming OC | SMBus `/dev/i2c-3` 0x67 | Unknown (4) | Breathing (Direct available) |
| 1 | NZXT Smart Device V2 `1e71:2006` | USB HID | Hue 2 Ch1 (16), Hue 2 Ch2 (10) | Direct |
| 2 | ASUS ROG STRIX X570-E (Aura `0b05:18f3`) | USB HID | Aura Mainboard (8, incl. 2 RGB headers), Aura Addressable 1 (0), Aura Addressable 2 (0) | Direct |
| 3 | NZXT Kraken X3 (`1e71:2007`) | USB HID | Hue 2 Ch1 (0), Ch2 (8), Ch3 (1) | Direct |
| 4 | Razer DeathAdder Essential V2 `1532:0098` | USB HID | Logo (1), Scroll Wheel (1) | Direct |

All detected as of 2026-11-12, **49 LEDs** in total. The Aura ARGB headers report 0 LEDs: either nothing is connected, or they need resizing in the OpenRGB zone editor. Kraken Ch2 (8) is probably the pump ring and Ch3 (1) the logo. Identified with `scripts/identify.sh` (2026-11-12): **NZXT Ch1 (16) = case fans** (role `fans`), **NZXT Ch2 (10) = case LED strip** (role `strip`, carries the request pulses).

- To reach the SMBus devices: `i2c-piix4` loaded, `/etc/modules-load.d/i2c.conf` written, user added to the `i2c` group (`scripts/setup-i2c.sh`).
- **Root cause of the missing USB devices:** `60-openrgb.rules` grants access only through `TAG+="uaccess"`, which applies only to an active local seat session. Over SSH, or while headless, `/dev/hidraw*` stays `root:root 0660`. Fix: `scripts/61-seele-rgb.rules` sets `GROUP=plugdev` (installed by `scripts/setup-hid.sh`).
- **Second cause:** setting `MODE`/`GROUP` alone isn't enough. logind's uaccess ACL (here granting `gdm-greeter`, since the machine sits at the login screen) leaves `group::---`, so access is still denied. The rule now also runs `setfacl -m g:plugdev:rw`.
- The daemon must run as a user in `plugdev` and `i2c`, or `openrgb --server` must run as a system service.
- RAM: no SMBus RGB DIMMs detected.
