//! Connections to data sources. Each runs on its own thread, reconnects with a
//! back-off when it drops, and reports through a channel; nothing here touches
//! the UI.

use super::filter::{FilterMode, LineFilter};
use super::nmea::Nmea;
use super::signalk;
use super::sim::Simulator;
use crate::geo::LatLon;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Read, Write};
use std::net::{TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Kind {
    Tcp { host: String, port: u16 },
    Udp { port: u16 },
    Serial { path: String, baud: u32 },
    Gpsd { host: String, port: u16 },
    /// Polls the Signal K REST API at `url`, e.g. `http://localhost:3000`.
    SignalK { url: String },
    /// Replays a recorded NMEA file at `rate_hz` lines per second (0 means 20).
    File { path: String, #[serde(default)] rate_hz: f64 },
    Simulator { lat: f64, lon: f64, cog: f64, sog: f64 },
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// "off", "allow" or "deny".
    #[serde(default)]
    pub filter_mode: String,
    /// Comma-separated sentence types the filter applies to.
    #[serde(default)]
    pub filter_items: String,
}

impl ConnectionConfig {
    pub fn new(id: &str, name: &str, kind: Kind) -> ConnectionConfig {
        ConnectionConfig { id: id.into(), name: name.into(), kind, enabled: true, filter_mode: "off".into(), filter_items: String::new() }
    }

    pub fn filter(&self) -> LineFilter {
        let mode = match self.filter_mode.as_str() {
            "allow" => FilterMode::Allow,
            "deny" => FilterMode::Deny,
            _ => FilterMode::Off,
        };
        LineFilter::from_text(mode, &self.filter_items)
    }

    /// A one-line description for lists: `tcp 192.168.1.5:10110`.
    pub fn describe(&self) -> String {
        match &self.kind {
            Kind::Tcp { host, port } => format!("TCP {host}:{port}"),
            Kind::Udp { port } => format!("UDP :{port}"),
            Kind::Serial { path, baud } => format!("{path} @ {baud}"),
            Kind::Gpsd { host, port } => format!("gpsd {host}:{port}"),
            Kind::SignalK { url } => format!("Signal K {url}"),
            Kind::File { path, .. } => format!("File {path}"),
            Kind::Simulator { .. } => "Simulator".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Connecting,
    Connected,
    Disconnected(String),
    /// A finite source (a file) ran out.
    Finished,
}

#[derive(Clone, Debug)]
pub enum Payload {
    /// A raw line, for NMEA parsing and the data monitor.
    Line(String),
    /// Already-decoded updates from a JSON feed.
    Updates(Vec<Nmea>),
    Status(Status),
}

#[derive(Clone, Debug)]
pub struct Event {
    pub source: String,
    pub payload: Payload,
}

/// Assembles lines from arbitrary chunks of bytes.
#[derive(Default)]
pub struct LineBuf {
    buf: Vec<u8>,
}

const MAX_LINE: usize = 4096;

impl LineBuf {
    pub fn push(&mut self, data: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(data);
        let mut lines = Vec::new();
        while let Some(i) = self.buf.iter().position(|b| *b == b'\n') {
            let raw: Vec<u8> = self.buf.drain(..=i).collect();
            let s = String::from_utf8_lossy(&raw).trim().to_string();
            if !s.is_empty() {
                lines.push(s);
            }
        }
        // A "line" that never ends is binary junk; don't grow without bound.
        if self.buf.len() > MAX_LINE {
            self.buf.clear();
        }
        lines
    }

    /// Whatever is left, treating the end of a datagram as the end of a line.
    pub fn flush(&mut self) -> Option<String> {
        let s = String::from_utf8_lossy(&std::mem::take(&mut self.buf)).trim().to_string();
        (!s.is_empty()).then_some(s)
    }
}

/// Serial devices that look like instruments or GPS receivers, as paths.
pub fn serial_ports_in(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            ["ttyUSB", "ttyACM", "ttyAMA", "rfcomm", "ttyS"].iter().any(|p| n.starts_with(p)).then(|| format!("{}/{n}", dir.display()))
        })
        .collect();
    v.sort();
    v
}

pub fn list_serial_ports() -> Vec<String> {
    serial_ports_in(std::path::Path::new("/dev"))
}

type Emit<'a> = &'a dyn Fn(Payload) -> bool;

/// Runs a connection until `stop` is set. Blocking; call from a thread.
pub fn run(cfg: ConnectionConfig, tx: Sender<Event>, stop: &AtomicBool) {
    let emit = |p: Payload| tx.send(Event { source: cfg.id.clone(), payload: p }).is_ok();
    let filter = cfg.filter();
    let mut backoff = 1.0f64;
    while !stop.load(Ordering::Relaxed) {
        if !emit(Payload::Status(Status::Connecting)) {
            return;
        }
        let result = match &cfg.kind {
            Kind::Tcp { host, port } => run_tcp(host, *port, None, &filter, &emit, stop),
            Kind::Gpsd { host, port } => run_tcp(host, *port, Some(b"?WATCH={\"enable\":true,\"json\":true};\n"), &filter, &emit, stop),
            Kind::Udp { port } => run_udp(*port, &filter, &emit, stop),
            Kind::Serial { path, baud } => run_serial(path, *baud, &filter, &emit, stop),
            Kind::SignalK { url } => run_signalk(url, &emit, stop),
            Kind::File { path, rate_hz } => run_file(path, *rate_hz, &filter, &emit, stop),
            Kind::Simulator { lat, lon, cog, sog } => run_sim(LatLon::new(*lat, *lon), *cog, *sog, &emit, stop),
        };
        match result {
            Ok(()) => {
                // Only finite sources return Ok.
                emit(Payload::Status(Status::Finished));
                return;
            }
            Err(e) => {
                if stop.load(Ordering::Relaxed) || !emit(Payload::Status(Status::Disconnected(e))) {
                    return;
                }
                sleep_unless_stopped(backoff, stop);
                backoff = (backoff * 2.0).min(30.0);
            }
        }
    }
}

fn sleep_unless_stopped(secs: f64, stop: &AtomicBool) {
    let mut left = secs;
    while left > 0.0 && !stop.load(Ordering::Relaxed) {
        let d = left.min(0.1);
        std::thread::sleep(Duration::from_secs_f64(d));
        left -= d;
    }
}

fn emit_lines(lines: Vec<String>, filter: &LineFilter, emit: Emit) -> bool {
    for l in lines {
        if filter.accepts(&l) && !emit(Payload::Line(l)) {
            return false;
        }
    }
    true
}

fn run_tcp(host: &str, port: u16, hello: Option<&[u8]>, filter: &LineFilter, emit: Emit, stop: &AtomicBool) -> Result<(), String> {
    let addrs: Vec<_> = (host, port).to_socket_addrs().map_err(|e| format!("cannot resolve {host}: {e}"))?.collect();
    let mut stream = None;
    let mut last_err = String::from("no addresses");
    for a in addrs {
        match TcpStream::connect_timeout(&a, Duration::from_secs(5)) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(e) => last_err = e.to_string(),
        }
    }
    let mut stream = stream.ok_or(last_err)?;
    stream.set_read_timeout(Some(Duration::from_millis(300))).map_err(|e| e.to_string())?;
    if let Some(h) = hello {
        stream.write_all(h).map_err(|e| e.to_string())?;
    }
    emit(Payload::Status(Status::Connected));
    let json = hello.is_some();
    let mut lb = LineBuf::default();
    let mut buf = [0u8; 4096];
    while !stop.load(Ordering::Relaxed) {
        match stream.read(&mut buf) {
            Ok(0) => return Err("connection closed".into()),
            Ok(n) => {
                for l in lb.push(&buf[..n]) {
                    if json {
                        let ups = signalk::gpsd_to_nmea(&l);
                        if !ups.is_empty() && !emit(Payload::Updates(ups)) {
                            return Err("stopped".into());
                        }
                    } else if !emit_lines(vec![l], filter, emit) {
                        return Err("stopped".into());
                    }
                }
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("stopped".into())
}

fn run_udp(port: u16, filter: &LineFilter, emit: Emit, stop: &AtomicBool) -> Result<(), String> {
    let sock = UdpSocket::bind(("0.0.0.0", port)).map_err(|e| format!("cannot listen on UDP {port}: {e}"))?;
    sock.set_read_timeout(Some(Duration::from_millis(300))).map_err(|e| e.to_string())?;
    emit(Payload::Status(Status::Connected));
    let mut buf = [0u8; 2048];
    while !stop.load(Ordering::Relaxed) {
        match sock.recv_from(&mut buf) {
            Ok((n, _)) => {
                let mut lb = LineBuf::default();
                let mut lines = lb.push(&buf[..n]);
                lines.extend(lb.flush());
                if !emit_lines(lines, filter, emit) {
                    return Err("stopped".into());
                }
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("stopped".into())
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn run_serial(path: &str, baud: u32, filter: &LineFilter, emit: Emit, stop: &AtomicBool) -> Result<(), String> {
    let mut port = serialport::new(path, baud)
        .timeout(Duration::from_millis(300))
        .open()
        .map_err(|e| format!("cannot open {path}: {e}"))?;
    emit(Payload::Status(Status::Connected));
    let mut lb = LineBuf::default();
    let mut buf = [0u8; 1024];
    while !stop.load(Ordering::Relaxed) {
        match port.read(&mut buf) {
            Ok(0) => {}
            Ok(n) => {
                if !emit_lines(lb.push(&buf[..n]), filter, emit) {
                    return Err("stopped".into());
                }
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("stopped".into())
}

fn run_signalk(url: &str, emit: Emit, stop: &AtomicBool) -> Result<(), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(5))).build().into();
    let endpoint = format!("{}/signalk/v1/api/vessels/self", url.trim_end_matches('/'));
    let mut first = true;
    while !stop.load(Ordering::Relaxed) {
        let mut resp = agent.get(&endpoint).call().map_err(|e| e.to_string())?;
        let body = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
        let doc: serde_json::Value = serde_json::from_str(&body).map_err(|e| format!("bad Signal K document: {e}"))?;
        if first {
            emit(Payload::Status(Status::Connected));
            first = false;
        }
        let ups = signalk::updates_from_pairs(&signalk::flatten_document(&doc));
        if !ups.is_empty() && !emit(Payload::Updates(ups)) {
            return Err("stopped".into());
        }
        sleep_unless_stopped(1.0, stop);
    }
    Err("stopped".into())
}

fn run_file(path: &str, rate_hz: f64, filter: &LineFilter, emit: Emit, stop: &AtomicBool) -> Result<(), String> {
    let f = std::fs::File::open(path).map_err(|e| format!("cannot open {path}: {e}"))?;
    emit(Payload::Status(Status::Connected));
    let delay = 1.0 / if rate_hz > 0.0 { rate_hz } else { 20.0 };
    for line in std::io::BufReader::new(f).lines() {
        if stop.load(Ordering::Relaxed) {
            return Err("stopped".into());
        }
        let line = line.map_err(|e| e.to_string())?;
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        if filter.accepts(&line) && !emit(Payload::Line(line)) {
            return Err("stopped".into());
        }
        sleep_unless_stopped(delay, stop);
    }
    Ok(())
}

fn run_sim(start: LatLon, cog: f64, sog: f64, emit: Emit, stop: &AtomicBool) -> Result<(), String> {
    // The `ISOMAR_DEMO` boat (screenshots, the simulators) keeps its traffic clear unless asked for the
    // collision; a demo boat the user adds shows the alert, as the demo is there to show what it does.
    // The `ISOMAR_DEMO` boat moored in a harbour (no speed) has no traffic: the demo's boats are placed
    // round a boat under way, and round a quay they would be on land.
    let demo = std::env::var_os("ISOMAR_DEMO").is_some();
    let clear = demo && std::env::var_os("ISOMAR_DEMO_COLLISION").is_none();
    let now = Utc::now();
    let mut sim = if demo && sog <= 0.0 {
        Simulator::new(start, cog, 0.0, now)
    } else if clear {
        Simulator::demo_clear(start, cog, sog, now)
    } else {
        Simulator::demo(start, cog, sog, now)
    };
    emit(Payload::Status(Status::Connected));
    while !stop.load(Ordering::Relaxed) {
        for l in sim.step(1.0) {
            if !emit(Payload::Line(l)) {
                return Err("stopped".into());
            }
        }
        sleep_unless_stopped(1.0, stop);
    }
    Err("stopped".into())
}

#[cfg(any(target_os = "ios", target_os = "android"))]
fn run_serial(path: &str, _baud: u32, _filter: &LineFilter, _emit: Emit, _stop: &AtomicBool) -> Result<(), String> {
    Err(format!("cannot open {path}: serial ports are not available on this device"))
}

/// The serial ports the system lists, such as `/dev/cu.usbserial-1420` or `/dev/ttyUSB0`.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub fn serial_ports() -> Vec<String> {
    serialport::available_ports().map(|v| v.into_iter().map(|p| p.port_name).collect()).unwrap_or_default()
}

/// None on a phone or tablet.
#[cfg(any(target_os = "ios", target_os = "android"))]
pub fn serial_ports() -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_buffer_assembles_split_chunks_and_handles_crlf() {
        let mut lb = LineBuf::default();
        assert!(lb.push(b"$GPRMC,1").is_empty());
        assert_eq!(lb.push(b",2*00\r\n$GPGGA"), vec!["$GPRMC,1,2*00"]);
        assert_eq!(lb.push(b",3\n\n\r\n"), vec!["$GPGGA,3"]);
        assert_eq!(lb.flush(), None);
        lb.push(b"tail");
        assert_eq!(lb.flush().as_deref(), Some("tail"));
    }

    #[test]
    fn line_buffer_survives_binary_junk() {
        let mut lb = LineBuf::default();
        for _ in 0..10 {
            assert!(lb.push(&[0xff; 1000]).is_empty());
        }
        assert!(lb.buf.len() <= MAX_LINE + 1000);
        let lines = lb.push(b"\n$ok\n");
        assert_eq!(lines.last().map(|s| s.as_str()), Some("$ok"));
    }

    #[test]
    fn config_roundtrips_through_json_and_defaults() {
        let c = ConnectionConfig::new("a", "Chartplotter", Kind::Tcp { host: "192.168.1.5".into(), port: 10110 });
        let s = serde_json::to_string(&c).unwrap();
        assert!(s.contains("\"type\":\"tcp\""));
        let back: ConnectionConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back, c);
        let minimal: ConnectionConfig = serde_json::from_str(r#"{"id":"x","name":"n","kind":{"type":"udp","port":2000}}"#).unwrap();
        assert!(minimal.enabled && minimal.filter_mode.is_empty());
        let f: ConnectionConfig = serde_json::from_str(r#"{"id":"x","name":"n","kind":{"type":"file","path":"/tmp/x"}}"#).unwrap();
        assert!(matches!(f.kind, Kind::File { rate_hz, .. } if rate_hz == 0.0));
        assert!(serde_json::from_str::<ConnectionConfig>(r#"{"id":"x","name":"n","kind":{"type":"bogus"}}"#).is_err());
    }

    #[test]
    fn filters_come_from_the_config() {
        let mut c = ConnectionConfig::new("a", "n", Kind::Udp { port: 1 });
        c.filter_mode = "deny".into();
        c.filter_items = "GGA, GSV".into();
        let f = c.filter();
        assert_eq!(f.mode, FilterMode::Deny);
        assert_eq!(f.items, ["GGA", "GSV"]);
        assert_eq!(ConnectionConfig::new("a", "n", Kind::Udp { port: 1 }).filter().mode, FilterMode::Off);
    }

    #[test]
    fn serial_port_listing_picks_instrument_like_devices() {
        let d = tempfile::tempdir().unwrap();
        for n in ["ttyUSB1", "ttyUSB0", "ttyACM0", "null", "tty0", "rfcomm0"] {
            std::fs::write(d.path().join(n), "").unwrap();
        }
        let v = serial_ports_in(d.path());
        let names: Vec<_> = v.iter().map(|p| p.rsplit('/').next().unwrap()).collect();
        assert_eq!(names, ["rfcomm0", "ttyACM0", "ttyUSB0", "ttyUSB1"]);
        assert!(serial_ports_in(std::path::Path::new("/nonexistent")).is_empty());
    }

    #[test]
    fn descriptions() {
        assert_eq!(ConnectionConfig::new("a", "n", Kind::Serial { path: "/dev/ttyUSB0".into(), baud: 4800 }).describe(), "/dev/ttyUSB0 @ 4800");
        assert_eq!(ConnectionConfig::new("a", "n", Kind::Udp { port: 2000 }).describe(), "UDP :2000");
    }
}
