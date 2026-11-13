//! `install`, `uninstall`, `doctor`: system setup done by the binary itself.
//! Root steps run through `sudo`; the binary never runs as root.
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};

const UDEV_RULES: &str = include_str!("../assets/61-seele-rgb.rules");
const OPENRGB_UNIT: &str = include_str!("../assets/openrgb-server.service");
const SEELE_UNIT: &str = include_str!("../assets/seele-rgb.service");
const MODULES_CONF: &str = "i2c-dev\ni2c-piix4\n";

const UNIT_DIR: &str = "/etc/systemd/system";
const MODULES_PATH: &str = "/etc/modules-load.d/i2c.conf";
const UDEV_PATH: &str = "/etc/udev/rules.d/61-seele-rgb.rules";
const SERVICES: [&str; 2] = ["openrgb-server", "seele-rgb"];
const STATE_ADDR: &str = "127.0.0.1:6743";

struct Ctx {
    dry: bool,
    root: bool,
}

fn out(cmd: &str, args: &[&str]) -> Option<String> {
    let o = Command::new(cmd).args(args).stderr(Stdio::null()).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn ok(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd).args(args).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

fn user() -> Result<String> {
    out("id", &["-un"]).context("cannot determine current user")
}

fn which(bin: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?.to_str()?.split(':').map(|d| Path::new(d).join(bin)).find(|p| p.is_file())
}

fn group_exists(g: &str) -> bool {
    ok("getent", &["group", g])
}

fn in_group(user: &str, g: &str) -> bool {
    out("id", &["-nG", user]).map(|s| s.split_whitespace().any(|x| x == g)).unwrap_or(false)
        || out("getent", &["group", g]).map(|l| l.rsplit(':').next().unwrap_or("").split(',').any(|x| x == user)).unwrap_or(false)
}

impl Ctx {
    /// Run a command, via sudo when `root` and we are not already root. `tolerate` swallows failure.
    fn run(&self, root: bool, args: &[&str], tolerate: bool) -> Result<()> {
        let mut full: Vec<&str> = vec![];
        if root && !self.root {
            full.push("sudo");
        }
        full.extend_from_slice(args);
        if self.dry {
            println!("  [dry-run] {}", full.join(" "));
            return Ok(());
        }
        println!("  $ {}", full.join(" "));
        let st = Command::new(full[0]).args(&full[1..]).status().with_context(|| format!("running {}", full[0]))?;
        if !st.success() && !tolerate {
            bail!("`{}` failed ({st})", full.join(" "));
        }
        Ok(())
    }

    /// Write a root-owned file unless it is already identical. Returns true if changed.
    fn write_root(&self, path: &str, content: &str) -> Result<bool> {
        if std::fs::read_to_string(path).map(|c| c == content).unwrap_or(false) {
            println!("  = {path} unchanged");
            return Ok(false);
        }
        if self.dry {
            println!("  [dry-run] write {path} ({} bytes)", content.len());
            return Ok(true);
        }
        let tmp = std::env::temp_dir().join(format!("seele-rgb-{}-{}", std::process::id(), path.replace('/', "_")));
        std::fs::File::create(&tmp)?.write_all(content.as_bytes())?;
        let r = self.run(true, &["install", "-m", "644", tmp.to_str().unwrap(), path], false);
        let _ = std::fs::remove_file(&tmp);
        r?;
        Ok(true)
    }
}

fn unit_text(template: &str, user: &str, exe: &str, openrgb: &str) -> String {
    let groups = if group_exists("docker") { "SupplementaryGroups=docker\n" } else { "" };
    template.replace("{USER}", user).replace("{EXE}", exe).replace("{OPENRGB}", openrgb).replace("{GROUPS_LINE}", groups)
}

fn units(user: &str) -> Result<Vec<(String, String)>> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let openrgb = which("openrgb").unwrap_or_else(|| PathBuf::from("/usr/bin/openrgb"));
    let (exe, openrgb) = (exe.to_string_lossy().into_owned(), openrgb.to_string_lossy().into_owned());
    Ok(vec![
        (format!("{UNIT_DIR}/openrgb-server.service"), unit_text(OPENRGB_UNIT, user, &exe, &openrgb)),
        (format!("{UNIT_DIR}/seele-rgb.service"), unit_text(SEELE_UNIT, user, &exe, &openrgb)),
    ])
}

fn hidraw_nodes() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("/dev")
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("hidraw"))
        .map(|n| format!("/dev/{n}"))
        .collect();
    v.sort();
    v
}

pub fn install(dry: bool, config_path: &str) -> Result<()> {
    crate::banner::print(crate::banner::color_ok());
    let ctx = Ctx { dry, root: out("id", &["-u"]).as_deref() == Some("0") };
    if ctx.root {
        bail!("run install as your normal user; it calls sudo for the root steps itself");
    }
    let user = user()?;
    if which("openrgb").is_none() {
        println!("warning: `openrgb` not found in PATH; the unit will point at /usr/bin/openrgb (install the OpenRGB package)");
    }
    println!("==> sudo credentials");
    ctx.run(true, &["-v"], false)?;

    println!("==> kernel modules (i2c-dev, i2c-piix4)");
    ctx.write_root(MODULES_PATH, MODULES_CONF)?;
    ctx.run(true, &["modprobe", "i2c-dev"], false)?;
    ctx.run(true, &["modprobe", "i2c-piix4"], true)?;

    println!("==> i2c group");
    if !group_exists("i2c") {
        ctx.run(true, &["groupadd", "--system", "i2c"], false)?;
    }
    if in_group(&user, "i2c") {
        println!("  = {user} already in i2c");
    } else {
        ctx.run(true, &["usermod", "-aG", "i2c", &user], false)?;
    }

    println!("==> udev rules + plugdev ACLs");
    ctx.write_root(UDEV_PATH, UDEV_RULES)?;
    ctx.run(true, &["udevadm", "control", "--reload"], false)?;
    ctx.run(true, &["udevadm", "trigger", "--subsystem-match=hidraw", "--subsystem-match=usb"], false)?;
    for d in hidraw_nodes() {
        ctx.run(true, &["setfacl", "-m", "g:plugdev:rw", &d], true)?;
    }

    println!("==> default config");
    // Carry over the zone mapping from the old seele.service working directory, if any.
    let legacy = std::fs::read_to_string(format!("{UNIT_DIR}/seele.service"))
        .ok()
        .and_then(|u| u.lines().find_map(|l| l.strip_prefix("WorkingDirectory=").map(|d| format!("{}/config.toml", d.trim()))))
        .filter(|p| Path::new(p).exists());
    if let (false, Some(src)) = (Path::new(config_path).exists(), legacy) {
        if dry {
            println!("  [dry-run] copy {src} -> {config_path}");
        } else {
            if let Some(dir) = Path::new(config_path).parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::copy(&src, config_path)?;
            println!("  + migrated {src} -> {config_path}");
        }
    } else if !dry {
        crate::config::ensure_default(config_path)?;
    } else if !Path::new(config_path).exists() {
        println!("  [dry-run] write default config to {config_path}");
    } else {
        println!("  = {config_path} exists");
    }

    println!("==> migrating old seele.service");
    let old = format!("{UNIT_DIR}/seele.service");
    if Path::new(&old).exists() || ok("systemctl", &["cat", "seele.service"]) {
        ctx.run(true, &["systemctl", "stop", "seele.service"], true)?;
        ctx.run(true, &["systemctl", "disable", "seele.service"], true)?;
        ctx.run(true, &["rm", "-f", &old], false)?;
    } else {
        println!("  = no old seele.service");
    }

    println!("==> systemd units");
    let mut changed = false;
    for (path, text) in units(&user)? {
        changed |= ctx.write_root(&path, &text)?;
    }
    ctx.run(true, &["systemctl", "daemon-reload"], false)?;
    ctx.run(true, &["systemctl", "reset-failed", "openrgb-server.service", "seele-rgb.service"], true)?;
    ctx.run(true, &["systemctl", "enable", "openrgb-server.service", "seele-rgb.service"], false)?;
    let verb = if changed { "restart" } else { "start" };
    ctx.run(true, &["systemctl", verb, "openrgb-server.service", "seele-rgb.service"], false)?;

    println!("==> verifying");
    if dry {
        println!("  [dry-run] would check: openrgb-server + seele-rgb active, 127.0.0.1:6742 bound, GET {STATE_ADDR}/state");
        println!("  [dry-run] would print ALL OK");
        return Ok(());
    }
    let mut results = vec![];
    for _ in 0..15 {
        std::thread::sleep(Duration::from_secs(1));
        results = service_checks();
        if results.iter().all(|c| c.ok) {
            break;
        }
    }
    print_checks(&results);
    if results.iter().all(|c| c.ok) {
        println!("note: log out and back in for new group memberships to apply to your own shell (the services already have them)");
        println!("ALL OK");
        Ok(())
    } else {
        bail!("SOMETHING FAILED (see above)")
    }
}

pub fn uninstall(dry: bool) -> Result<()> {
    let ctx = Ctx { dry, root: out("id", &["-u"]).as_deref() == Some("0") };
    if ctx.root {
        bail!("run uninstall as your normal user; it calls sudo itself");
    }
    ctx.run(true, &["-v"], false)?;
    for n in ["seele-rgb.service", "seele.service", "openrgb-server.service"] {
        let path = format!("{UNIT_DIR}/{n}");
        if Path::new(&path).exists() {
            ctx.run(true, &["systemctl", "disable", "--now", n], true)?;
            ctx.run(true, &["rm", "-f", &path], false)?;
        } else {
            println!("  = {n} not installed");
        }
    }
    ctx.run(true, &["systemctl", "daemon-reload"], false)?;
    if Path::new(UDEV_PATH).exists() {
        ctx.run(true, &["rm", "-f", UDEV_PATH], false)?;
        ctx.run(true, &["udevadm", "control", "--reload"], false)?;
    }
    println!("kept: {MODULES_CONF:?} in {MODULES_PATH}, the i2c group and ~/.config/seele (remove by hand if unwanted)");
    println!("{}", if dry { "dry run only; nothing changed" } else { "uninstalled" });
    Ok(())
}

struct Check {
    ok: bool,
    what: String,
    hint: String,
}

fn chk(ok: bool, what: impl Into<String>, hint: &str) -> Check {
    Check { ok, what: what.into(), hint: hint.into() }
}

fn bound_localhost() -> bool {
    out("ss", &["-ltn", "sport = :6742"]).map(|s| s.contains("127.0.0.1:6742")).unwrap_or(false)
}

fn state_ok() -> bool {
    let Some(addr) = STATE_ADDR.to_socket_addrs().ok().and_then(|mut a| a.next()) else { return false };
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_secs(3)) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
    if s.write_all(b"GET /state HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n").is_err() {
        return false;
    }
    let mut buf = String::new();
    let _ = s.read_to_string(&mut buf);
    buf.starts_with("HTTP/1.") && buf.contains(" 200")
}

fn service_checks() -> Vec<Check> {
    let mut v = vec![];
    for s in SERVICES {
        let st = out("systemctl", &["is-active", &format!("{s}.service")]).unwrap_or_else(|| "inactive".into());
        v.push(chk(st == "active", format!("{s}.service is active ({st})"), &format!("journalctl -u {s} -e; or run `seele-rgb install`")));
    }
    v.push(chk(bound_localhost(), "OpenRGB bound to 127.0.0.1:6742", "check openrgb-server.service; unit must use --server-host 127.0.0.1"));
    v.push(chk(state_ok(), format!("GET {STATE_ADDR}/state"), "seele-rgb not serving; check `journalctl -u seele-rgb -e`"));
    v
}

fn print_checks(cs: &[Check]) {
    for c in cs {
        if c.ok {
            println!("  ✓ {}", c.what);
        } else {
            println!("  ✗ {}  -> {}", c.what, c.hint);
        }
    }
}

pub fn doctor(config_path: &str) -> Result<()> {
    let user = user()?;
    let mut cs = vec![];
    cs.push(chk(which("openrgb").is_some(), "openrgb binary in PATH", "install the OpenRGB package"));
    let loaded = Path::new("/sys/module/i2c_dev").exists() || std::fs::read_dir("/dev").map(|d| d.flatten().any(|e| e.file_name().to_string_lossy().starts_with("i2c-"))).unwrap_or(false);
    cs.push(chk(loaded, "i2c-dev module loaded", "sudo modprobe i2c-dev (or `seele-rgb install`)"));
    cs.push(chk(Path::new(MODULES_PATH).exists(), format!("{MODULES_PATH} present"), "run `seele-rgb install`"));
    cs.push(chk(group_exists("i2c"), "group i2c exists", "run `seele-rgb install`"));
    cs.push(chk(in_group(&user, "i2c"), format!("{user} in group i2c"), "run `seele-rgb install`, then log out and in"));
    cs.push(chk(std::fs::read_to_string(UDEV_PATH).map(|c| c == UDEV_RULES).unwrap_or(false), format!("{UDEV_PATH} up to date"), "run `seele-rgb install`"));
    let nodes = hidraw_nodes();
    let acl_ok = !nodes.is_empty() && nodes.iter().all(|n| out("getfacl", &["-p", n]).map(|s| s.contains("group:plugdev:rw")).unwrap_or(false));
    cs.push(chk(acl_ok || nodes.is_empty(), "hidraw nodes have plugdev ACL", "run `seele-rgb install` (needs setfacl from the acl package)"));
    if let Ok(us) = units(&user) {
        for (p, text) in us {
            cs.push(chk(std::fs::read_to_string(&p).map(|c| c == text).unwrap_or(false), format!("{p} installed and current"), "run `seele-rgb install`"));
        }
    }
    cs.push(chk(!Path::new(&format!("{UNIT_DIR}/seele.service")).exists(), "old seele.service migrated away", "run `seele-rgb install`"));
    cs.push(chk(Path::new(config_path).exists(), format!("config {config_path} present"), "run `seele-rgb run` once or `seele-rgb install` to write the default"));
    cs.extend(service_checks());
    print_checks(&cs);
    let bad = cs.iter().filter(|c| !c.ok).count();
    if bad == 0 {
        println!("ALL OK");
        Ok(())
    } else {
        bail!("{bad} check(s) failed")
    }
}
