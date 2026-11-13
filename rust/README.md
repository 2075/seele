# seele-rgb

Machine load and service health as OpenRGB lighting (Rust). See `docs/adr/0002` and `0003` in the repo.

## Install

```sh
cargo install --path rust      # or: cargo install --git <repo> seele-rgb
seele-rgb install --dry-run    # print every action
seele-rgb install              # modules, i2c group, udev rules, systemd units, verify
seele-rgb doctor               # checks only
seele-rgb uninstall            # remove units and udev rules
```

`install` calls `sudo` per root step (one `sudo -v` up front; the binary never runs as root), skips files that are already identical, migrates the old `seele.service`, and prints `ALL OK` when OpenRGB is bound to 127.0.0.1:6742 and `http://127.0.0.1:6743/state` answers. The unit's `ExecStart` is the canonical path of the running binary, so reinstall after moving it.

Needs the `openrgb` package. Config lives in `~/.config/seele/rgb.toml` (honours `XDG_CONFIG_HOME`; `--config` overrides) and is written from a built-in default on first run. Zones are machine-specific: use `seele-rgb identify`, then edit the file (hot-reloads every 2 s).
