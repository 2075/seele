//! OpenRGB SDK client (blocking TCP) and Sink. Port of seele/sinks/openrgb_sink.py.
//!
//! Wire format: 16-byte header `ORGB`, device id u32, packet type u32, payload size u32 (all LE),
//! followed by the payload.
use super::{Frame, Sink};
use anyhow::{anyhow, bail, Context, Result};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

const MAX_VERSION: u32 = 4;
const PKT_CONTROLLER_COUNT: u32 = 0;
const PKT_CONTROLLER_DATA: u32 = 1;
const PKT_PROTOCOL_VERSION: u32 = 40;
const PKT_SET_CLIENT_NAME: u32 = 50;
const PKT_DEVICE_LIST_UPDATED: u32 = 100;
const PKT_UPDATE_ZONE_LEDS: u32 = 1051;
const PKT_UPDATE_MODE: u32 = 1101;

#[derive(Debug, Clone, PartialEq)]
pub struct Zone {
    pub name: String,
    pub num_leds: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mode {
    pub index: i32,
    pub name: String,
    /// Raw mode body (name .. colours) exactly as sent by the server, echoed back on update.
    raw: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Controller {
    pub name: String,
    pub zones: Vec<Zone>,
    pub modes: Vec<Mode>,
}

// ---------- decoding ----------

struct Rd<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Rd<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.p.checked_add(n).filter(|&e| e <= self.b.len()).ok_or_else(|| anyhow!("truncated packet"))?;
        let s = &self.b[self.p..end];
        self.p = end;
        Ok(s)
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into()?))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }
    fn string(&mut self) -> Result<String> {
        let n = self.u16()? as usize;
        let s = self.take(n)?;
        Ok(String::from_utf8_lossy(s).trim_end_matches('\0').to_string())
    }
}

fn parse_mode(r: &mut Rd, version: u32, index: i32) -> Result<Mode> {
    let start = r.p;
    let name = r.string()?;
    r.take(4 + 4 + 4 + 4)?; // value, flags, speed min/max
    if version >= 3 {
        r.take(8)?; // brightness min/max
    }
    r.take(8 + 4)?; // colors min/max, speed
    if version >= 3 {
        r.take(4)?; // brightness
    }
    r.take(8)?; // direction, colour mode
    let n = r.u16()? as usize;
    r.take(4 * n)?;
    Ok(Mode { index, name, raw: r.b[start..r.p].to_vec() })
}

fn parse_zone(r: &mut Rd, version: u32) -> Result<Zone> {
    let name = r.string()?;
    r.take(4 + 4 + 4)?; // type, leds min, max
    let num_leds = r.u32()? as usize;
    if r.u16()? > 0 {
        let h = r.u32()? as usize;
        let w = r.u32()? as usize;
        r.take(4 * h * w)?;
    }
    if version >= 4 {
        for _ in 0..r.u16()? {
            r.string()?;
            r.take(12)?;
        }
    }
    Ok(Zone { name, num_leds })
}

/// Decode a REQUEST_CONTROLLER_DATA payload.
pub fn parse_controller(data: &[u8], version: u32) -> Result<Controller> {
    let mut r = Rd { b: data, p: 0 };
    r.u32()?; // total size
    r.i32()?; // device type
    let name = r.string()?;
    if version >= 1 {
        r.string()?; // vendor
    }
    for _ in 0..4 {
        r.string()?; // description, version, serial, location
    }
    let nmodes = r.u16()? as usize;
    r.i32()?; // active mode
    let modes = (0..nmodes).map(|i| parse_mode(&mut r, version, i as i32)).collect::<Result<Vec<_>>>()?;
    let nzones = r.u16()? as usize;
    let zones = (0..nzones).map(|_| parse_zone(&mut r, version)).collect::<Result<Vec<_>>>()?;
    Ok(Controller { name, zones, modes })
}

// ---------- encoding ----------

pub fn header(device: u32, ty: u32, size: u32) -> [u8; 16] {
    let mut h = [0u8; 16];
    h[..4].copy_from_slice(b"ORGB");
    h[4..8].copy_from_slice(&device.to_le_bytes());
    h[8..12].copy_from_slice(&ty.to_le_bytes());
    h[12..].copy_from_slice(&size.to_le_bytes());
    h
}

/// Full packet (header + payload) for RGBCONTROLLER_UPDATEZONELEDS.
pub fn encode_zone_leds(device: u32, zone: u32, colors: &[(u8, u8, u8)]) -> Vec<u8> {
    let payload_len = 4 + 4 + 2 + 4 * colors.len();
    let mut p = header(device, PKT_UPDATE_ZONE_LEDS, payload_len as u32).to_vec();
    p.extend((payload_len as u32).to_le_bytes());
    p.extend(zone.to_le_bytes());
    p.extend((colors.len() as u16).to_le_bytes());
    for &(r, g, b) in colors {
        p.extend([r, g, b, 0]);
    }
    p
}

/// Full packet for RGBCONTROLLER_UPDATEMODE echoing the server's own mode description.
pub fn encode_update_mode(device: u32, mode: &Mode) -> Vec<u8> {
    let payload_len = 4 + 4 + mode.raw.len();
    let mut p = header(device, PKT_UPDATE_MODE, payload_len as u32).to_vec();
    p.extend((payload_len as u32).to_le_bytes());
    p.extend(mode.index.to_le_bytes());
    p.extend(&mode.raw);
    p
}

fn encode_name(name: &str) -> Vec<u8> {
    let mut v = name.as_bytes().to_vec();
    v.push(0);
    v
}

// ---------- client ----------

pub struct Client {
    sock: TcpStream,
    pub version: u32,
}

impl Client {
    pub fn connect(host: &str, port: u16, name: &str) -> Result<Client> {
        let sock = TcpStream::connect((host, port)).with_context(|| format!("connecting to OpenRGB at {host}:{port}"))?;
        sock.set_nodelay(true).ok();
        let mut c = Client { sock, version: 0 };
        c.sock.set_read_timeout(Some(Duration::from_secs(1)))?;
        c.send(0, PKT_PROTOCOL_VERSION, &MAX_VERSION.to_le_bytes())?;
        // Old servers never answer: timeout means protocol version 0.
        c.version = match c.read_until(PKT_PROTOCOL_VERSION) {
            Ok(p) => u32::from_le_bytes(p.get(..4).ok_or_else(|| anyhow!("short version reply"))?.try_into()?).min(MAX_VERSION),
            Err(e) if is_timeout(&e) => 0,
            Err(e) => return Err(e),
        };
        c.sock.set_read_timeout(Some(Duration::from_secs(10)))?;
        c.send(0, PKT_SET_CLIENT_NAME, &encode_name(name))?;
        Ok(c)
    }

    fn send(&mut self, device: u32, ty: u32, payload: &[u8]) -> Result<()> {
        let mut p = header(device, ty, payload.len() as u32).to_vec();
        p.extend(payload);
        self.send_raw(&p)
    }

    fn send_raw(&mut self, packet: &[u8]) -> Result<()> {
        self.sock.write_all(packet).context("OpenRGB write")
    }

    /// Read packets until one of type `want` arrives (unsolicited ones are dropped).
    fn read_until(&mut self, want: u32) -> Result<Vec<u8>> {
        loop {
            let mut h = [0u8; 16];
            self.sock.read_exact(&mut h)?;
            if &h[..4] != b"ORGB" {
                bail!("bad OpenRGB packet signature");
            }
            let ty = u32::from_le_bytes(h[8..12].try_into()?);
            let size = u32::from_le_bytes(h[12..16].try_into()?) as usize;
            let mut payload = vec![0u8; size];
            self.sock.read_exact(&mut payload)?;
            if ty == want {
                return Ok(payload);
            }
            if ty != PKT_DEVICE_LIST_UPDATED {
                tracing::debug!("ignoring OpenRGB packet type {ty}");
            }
        }
    }

    /// Request the current controller list.
    pub fn controllers(&mut self) -> Result<Vec<Controller>> {
        self.send(0, PKT_CONTROLLER_COUNT, &[])?;
        let p = self.read_until(PKT_CONTROLLER_COUNT)?;
        let n = u32::from_le_bytes(p.get(..4).ok_or_else(|| anyhow!("short count reply"))?.try_into()?);
        (0..n)
            .map(|i| {
                self.send(i, PKT_CONTROLLER_DATA, &self.version.to_le_bytes())?;
                let d = self.read_until(PKT_CONTROLLER_DATA)?;
                parse_controller(&d, self.version).with_context(|| format!("parsing controller {i}"))
            })
            .collect()
    }

    pub fn set_mode(&mut self, device: u32, mode: &Mode) -> Result<()> {
        self.send_raw(&encode_update_mode(device, mode))
    }

    pub fn set_zone_leds(&mut self, device: u32, zone: u32, colors: &[(u8, u8, u8)]) -> Result<()> {
        self.send_raw(&encode_zone_leds(device, zone, colors))
    }
}

fn is_timeout(e: &anyhow::Error) -> bool {
    e.downcast_ref::<std::io::Error>()
        .is_some_and(|e| matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut))
}

// ---------- sink ----------

pub struct OpenRgbSink {
    client: Client,
    /// Per configured zone: (device id, zone index within device), or None if not found.
    targets: Vec<Option<(u32, u32)>>,
    sizes: Vec<usize>,
    last: Vec<Option<Vec<(u8, u8, u8)>>>,
}

// Assumed config fields: `host: String`, `port: u16`, `zones: Vec<ZoneBinding { device, zone, role }>`.
pub fn connect(cfg: &crate::config::Config) -> Result<OpenRgbSink> {
    let b: Vec<(String, String)> = cfg.zones.iter().map(|z| (z.device.clone(), z.zone.clone())).collect();
    connect_with(&cfg.host, cfg.port, &b)
}

pub fn connect_with(host: &str, port: u16, bindings: &[(String, String)]) -> Result<OpenRgbSink> {
    let deadline = Instant::now() + Duration::from_secs(30);
    // The server may not be listening yet at boot; retry the connection within the same budget.
    let mut client = loop {
        match Client::connect(host, port, "seele") {
            Ok(c) => break c,
            Err(e) if Instant::now() < deadline => {
                tracing::info!("OpenRGB not ready ({e:#}), retrying");
                std::thread::sleep(Duration::from_secs(1));
            }
            Err(e) => return Err(e),
        }
    };
    let mut devs = client.controllers()?;
    // The server detects devices asynchronously at startup, so poll until all are present.
    loop {
        let missing: Vec<&str> = bindings
            .iter()
            .map(|(d, _)| d.as_str())
            .filter(|d| !devs.iter().any(|c| c.name.to_lowercase().contains(&d.to_lowercase())))
            .collect();
        if missing.is_empty() {
            break;
        }
        if Instant::now() >= deadline {
            tracing::warn!("devices still missing after 30s: {missing:?}");
            break;
        }
        std::thread::sleep(Duration::from_secs(1));
        devs = client.controllers()?;
    }

    let (mut targets, mut sizes, mut direct) = (vec![], vec![], std::collections::HashSet::new());
    for (dev, zone) in bindings {
        let found = devs.iter().enumerate().find(|(_, c)| c.name.to_lowercase().contains(&dev.to_lowercase())).and_then(|(di, c)| {
            c.zones.iter().position(|z| &z.name == zone).map(|zi| (di, c, zi))
        });
        match found {
            None => {
                tracing::warn!("not found: {dev} / {zone}; skipping");
                targets.push(None);
                sizes.push(0);
            }
            Some((di, c, zi)) => {
                if direct.insert(di) {
                    match c.modes.iter().find(|m| m.name.eq_ignore_ascii_case("direct")) {
                        Some(m) => client.set_mode(di as u32, m)?,
                        None => tracing::warn!("{} has no Direct mode", c.name),
                    }
                }
                targets.push(Some((di as u32, zi as u32)));
                sizes.push(c.zones[zi].num_leds);
            }
        }
    }
    let last = vec![None; targets.len()];
    Ok(OpenRgbSink { client, targets, sizes, last })
}

impl Sink for OpenRgbSink {
    fn sizes(&self) -> &[usize] {
        &self.sizes
    }

    fn show(&mut self, frame: &Frame, _status: &str) -> Result<()> {
        for (&i, colors) in frame {
            let Some(Some((dev, zone))) = self.targets.get(i) else { continue };
            if self.last[i].as_deref() == Some(colors.as_slice()) {
                continue; // unchanged
            }
            self.client.set_zone_leds(*dev, *zone, colors)?;
            self.last[i] = Some(colors.clone());
        }
        Ok(())
    }

    fn close(&mut self) {
        let _ = self.client.sock.shutdown(std::net::Shutdown::Both);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(n: &str) -> Vec<u8> {
        let mut v = ((n.len() + 1) as u16).to_le_bytes().to_vec();
        v.extend(n.as_bytes());
        v.push(0);
        v
    }

    fn mode_bytes(name: &str, version: u32) -> Vec<u8> {
        let mut v = s(name);
        v.extend(1i32.to_le_bytes()); // value
        v.extend(0x20u32.to_le_bytes()); // flags
        v.extend([0u8; 8]); // speed min/max
        if version >= 3 {
            v.extend([0u8; 8]);
        }
        v.extend([0u8; 8]); // colors min/max
        v.extend([0u8; 4]); // speed
        if version >= 3 {
            v.extend([0u8; 4]);
        }
        v.extend([0u8; 4]); // direction
        v.extend(1u32.to_le_bytes()); // color mode per-led
        v.extend(1u16.to_le_bytes()); // one colour
        v.extend([1, 2, 3, 0]);
        v
    }

    fn zone_bytes(name: &str, leds: u32, matrix: bool) -> Vec<u8> {
        let mut v = s(name);
        v.extend(1i32.to_le_bytes());
        v.extend(0u32.to_le_bytes());
        v.extend(leds.to_le_bytes());
        v.extend(leds.to_le_bytes());
        if matrix {
            v.extend(4u16.to_le_bytes());
            v.extend(1u32.to_le_bytes());
            v.extend(2u32.to_le_bytes());
            v.extend([0u8; 8]);
        } else {
            v.extend(0u16.to_le_bytes());
        }
        v.extend(1u16.to_le_bytes()); // one segment
        v.extend(s("seg"));
        v.extend([0u8; 12]);
        v
    }

    fn controller_bytes() -> Vec<u8> {
        let mut v = vec![0u8; 4]; // size, ignored
        v.extend(0i32.to_le_bytes());
        v.extend(s("NZXT Smart Device V2"));
        for m in ["NZXT", "d", "v", "ser", "loc"] {
            v.extend(s(m));
        }
        v.extend(2u16.to_le_bytes());
        v.extend(0i32.to_le_bytes());
        v.extend(mode_bytes("Off", 4));
        v.extend(mode_bytes("Direct", 4));
        v.extend(2u16.to_le_bytes());
        v.extend(zone_bytes("Hue 2 Channel 1", 16, false));
        v.extend(zone_bytes("Hue 2 Channel 2", 10, true));
        v.extend(0u16.to_le_bytes()); // leds
        v.extend(0u16.to_le_bytes()); // colours
        v
    }

    #[test]
    fn decode_controller() {
        let c = parse_controller(&controller_bytes(), 4).unwrap();
        assert_eq!(c.name, "NZXT Smart Device V2");
        assert_eq!(c.zones, vec![Zone { name: "Hue 2 Channel 1".into(), num_leds: 16 }, Zone { name: "Hue 2 Channel 2".into(), num_leds: 10 }]);
        assert_eq!(c.modes[1].name, "Direct");
        assert_eq!(c.modes[1].index, 1);
        assert!(parse_controller(&controller_bytes()[..50], 4).is_err());
    }

    #[test]
    fn encode_header_and_zone() {
        let h = header(2, 1051, 7);
        assert_eq!(&h, b"ORGB\x02\0\0\0\x1b\x04\0\0\x07\0\0\0");
        let p = encode_zone_leds(1, 0, &[(1, 2, 3), (4, 5, 6)]);
        let expect: Vec<u8> = [
            &b"ORGB"[..], &[1, 0, 0, 0], &[0x1b, 4, 0, 0], &[18, 0, 0, 0], // header, payload size 18
            &[18, 0, 0, 0], &[0, 0, 0, 0], &[2, 0], &[1, 2, 3, 0, 4, 5, 6, 0],
        ]
        .concat();
        assert_eq!(p, expect);
    }

    #[test]
    fn encode_mode_echoes_server_bytes() {
        let c = parse_controller(&controller_bytes(), 4).unwrap();
        let p = encode_update_mode(1, &c.modes[1]);
        let body = mode_bytes("Direct", 4);
        assert_eq!(&p[8..12], &1101u32.to_le_bytes());
        assert_eq!(&p[12..16], &((8 + body.len()) as u32).to_le_bytes());
        assert_eq!(&p[16..20], &((8 + body.len()) as u32).to_le_bytes());
        assert_eq!(&p[20..24], &1i32.to_le_bytes());
        assert_eq!(&p[24..], &body[..]);
    }

    #[test]
    #[ignore = "needs live OpenRGB server at 127.0.0.1:6742"]
    fn live_list() {
        let mut c = Client::connect("127.0.0.1", 6742, "seele-test").unwrap();
        println!("protocol version {}", c.version);
        let devs = c.controllers().unwrap();
        for (i, d) in devs.iter().enumerate() {
            let z: Vec<String> = d.zones.iter().map(|z| format!("{} ({})", z.name, z.num_leds)).collect();
            println!("{i}: {} [{}]", d.name, z.join(", "));
        }
        assert_eq!(devs.len(), 5);
        // one brief write: repaint a zone with its own (black) colours is avoided; seele repaints within 50 ms
        let d = devs.iter().position(|d| d.name.contains("NZXT Smart")).unwrap();
        c.set_zone_leds(d as u32, 1, &[(0, 0, 0); 10]).unwrap();
    }
}
