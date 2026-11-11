//! Owns the running connections and gathers their events in one place.

use super::source::{ConnectionConfig, Event, Payload, run};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

struct Conn {
    cfg: ConnectionConfig,
    stop: Arc<AtomicBool>,
}

pub struct Hub {
    tx: Sender<Event>,
    rx: Receiver<Event>,
    conns: Vec<Conn>,
}

impl Default for Hub {
    fn default() -> Self {
        Self::new()
    }
}

impl Hub {
    pub fn new() -> Hub {
        let (tx, rx) = channel();
        Hub { tx, rx, conns: Vec::new() }
    }

    /// Starts a connection (unless disabled), replacing any with the same id.
    pub fn add(&mut self, cfg: ConnectionConfig) {
        self.remove(&cfg.id);
        let stop = Arc::new(AtomicBool::new(false));
        if cfg.enabled {
            let (tx, stop2, cfg2) = (self.tx.clone(), stop.clone(), cfg.clone());
            // Not joined: it ends on its own once `stop` is set.
            std::thread::Builder::new()
                .name(format!("conn-{}", cfg.id))
                .spawn(move || run(cfg2, tx, &stop2))
                .expect("cannot spawn connection thread");
        }
        self.conns.push(Conn { cfg, stop });
    }

    /// Stops and forgets a connection. Its thread notices within a fraction
    /// of a second and exits on its own.
    pub fn remove(&mut self, id: &str) -> bool {
        match self.conns.iter().position(|c| c.cfg.id == id) {
            Some(i) => {
                let c = self.conns.remove(i);
                c.stop.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    pub fn configs(&self) -> Vec<ConnectionConfig> {
        self.conns.iter().map(|c| c.cfg.clone()).collect()
    }

    /// Passes on what the shell receives itself, under `source`: the phone's own GPS, or a USB or
    /// Bluetooth link only the platform can open. It arrives with the connections' events, in order.
    pub fn inject(&self, source: &str, payload: Payload) {
        let _ = self.tx.send(Event { source: source.into(), payload });
    }

    /// Events received so far, up to `max`. Never blocks.
    pub fn poll(&self, max: usize) -> Vec<Event> {
        self.rx.try_iter().take(max).collect()
    }

    pub fn stop_all(&mut self) {
        for c in self.conns.drain(..) {
            c.stop.store(true, Ordering::Relaxed);
        }
    }
}

impl Drop for Hub {
    fn drop(&mut self) {
        self.stop_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comm::nmea::frame;
    use crate::comm::source::{Kind, Payload, Status};
    use std::io::Write;
    use std::time::{Duration, Instant};

    /// Polls until `want` is satisfied by the events gathered so far.
    fn gather(hub: &Hub, want: impl Fn(&[Event]) -> bool) -> Vec<Event> {
        let mut all = Vec::new();
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            all.extend(hub.poll(1000));
            if want(&all) {
                return all;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        all
    }

    fn lines(ev: &[Event]) -> Vec<String> {
        ev.iter().filter_map(|e| if let Payload::Line(l) = &e.payload { Some(l.clone()) } else { None }).collect()
    }

    #[test]
    fn what_the_shell_passes_on_arrives_like_a_connections_lines() {
        let hub = Hub::new();
        hub.inject("device", Payload::Line(frame("GPRMC,1", '$')));
        let ev = hub.poll(10);
        assert_eq!(lines(&ev), vec![frame("GPRMC,1", '$')]);
        assert_eq!(ev[0].source, "device");
    }

    #[test]
    fn file_replay_delivers_lines_in_order_then_finishes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("log.nmea");
        let (a, b) = (frame("GPGGA,1", '$'), frame("GPRMC,2,A", '$'));
        std::fs::write(&p, format!("{a}\r\n\r\n{b}\n")).unwrap();
        let mut hub = Hub::new();
        hub.add(ConnectionConfig::new("f", "replay", Kind::File { path: p.to_string_lossy().into(), rate_hz: 1000.0 }));
        let ev = gather(&hub, |e| e.iter().any(|x| matches!(x.payload, Payload::Status(Status::Finished))));
        assert_eq!(lines(&ev), vec![a, b]);
        assert!(ev.iter().all(|e| e.source == "f"));
    }

    #[test]
    fn a_missing_file_reports_disconnected_and_keeps_retrying() {
        let mut hub = Hub::new();
        hub.add(ConnectionConfig::new("f", "x", Kind::File { path: "/definitely/not/here".into(), rate_hz: 0.0 }));
        let ev = gather(&hub, |e| e.iter().any(|x| matches!(x.payload, Payload::Status(Status::Disconnected(_)))));
        assert!(ev.iter().any(|x| matches!(&x.payload, Payload::Status(Status::Disconnected(m)) if m.contains("cannot open"))));
        assert!(hub.remove("f"));
        assert!(!hub.remove("f"));
    }

    #[test]
    fn tcp_client_reads_a_stream_and_applies_the_filter() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (keep, drop_) = (frame("GPRMC,3,A", '$'), frame("GPGSV,1", '$'));
        let (k2, d2) = (keep.clone(), drop_.clone());
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            // Deliver in awkward pieces.
            let all = format!("{d2}\r\n{k2}\r\n");
            let (x, y) = all.split_at(7);
            s.write_all(x.as_bytes()).unwrap();
            s.flush().unwrap();
            std::thread::sleep(Duration::from_millis(50));
            s.write_all(y.as_bytes()).unwrap();
            std::thread::sleep(Duration::from_millis(500));
        });
        let mut cfg = ConnectionConfig::new("t", "tcp", Kind::Tcp { host: "127.0.0.1".into(), port });
        cfg.filter_mode = "deny".into();
        cfg.filter_items = "GSV".into();
        let mut hub = Hub::new();
        hub.add(cfg);
        let ev = gather(&hub, |e| !lines(e).is_empty());
        assert!(ev.iter().any(|e| matches!(e.payload, Payload::Status(Status::Connected))));
        assert_eq!(lines(&ev), vec![keep]);
        server.join().unwrap();
    }

    #[test]
    fn tcp_reconnects_after_the_server_closes() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            // First connection is closed immediately; the second gets a line.
            let (s, _) = listener.accept().unwrap();
            drop(s);
            let (mut s, _) = listener.accept().unwrap();
            s.write_all(frame("GPRMC,9,A", '$').as_bytes()).unwrap();
            s.write_all(b"\r\n").unwrap();
            std::thread::sleep(Duration::from_millis(300));
        });
        let mut hub = Hub::new();
        hub.add(ConnectionConfig::new("t", "tcp", Kind::Tcp { host: "127.0.0.1".into(), port }));
        let ev = gather(&hub, |e| !lines(e).is_empty());
        assert!(ev.iter().any(|e| matches!(&e.payload, Payload::Status(Status::Disconnected(m)) if m.contains("closed"))));
        assert_eq!(lines(&ev).len(), 1);
        server.join().unwrap();
    }

    #[test]
    fn udp_listener_receives_datagrams() {
        // Find a free port, then release it for the listener.
        let port = {
            let s = UdpSocket::bind("127.0.0.1:0").unwrap();
            s.local_addr().unwrap().port()
        };
        let mut hub = Hub::new();
        hub.add(ConnectionConfig::new("u", "udp", Kind::Udp { port }));
        gather(&hub, |e| e.iter().any(|x| matches!(x.payload, Payload::Status(Status::Connected))));
        let tx = UdpSocket::bind("127.0.0.1:0").unwrap();
        let l = frame("GPGGA,5", '$');
        // One datagram holding two sentences, the last without a newline.
        tx.send_to(format!("{l}\r\n{l}").as_bytes(), ("127.0.0.1", port)).unwrap();
        let ev = gather(&hub, |e| lines(e).len() >= 2);
        assert_eq!(lines(&ev), vec![l.clone(), l]);
    }

    use std::net::UdpSocket;

    #[test]
    fn simulator_source_emits_a_full_set_of_sentences() {
        let mut hub = Hub::new();
        hub.add(ConnectionConfig::new("s", "sim", Kind::Simulator { lat: 48.0, lon: -4.0, cog: 90.0, sog: 5.0 }));
        let ev = gather(&hub, |e| lines(e).len() >= 10);
        let ls = lines(&ev);
        assert!(ls.iter().any(|l| l.contains("RMC")) && ls.iter().any(|l| l.starts_with("!AIVDM")));
    }

    #[test]
    fn disabled_connections_do_not_run_but_are_listed() {
        let mut hub = Hub::new();
        let mut c = ConnectionConfig::new("d", "off", Kind::Simulator { lat: 0.0, lon: 0.0, cog: 0.0, sog: 0.0 });
        c.enabled = false;
        hub.add(c);
        std::thread::sleep(Duration::from_millis(100));
        assert!(hub.poll(10).is_empty());
        assert_eq!(hub.configs().len(), 1);
    }

    #[test]
    fn adding_the_same_id_replaces_the_connection() {
        let mut hub = Hub::new();
        let mk = |p| ConnectionConfig::new("x", "n", Kind::Udp { port: p });
        hub.add(mk(1));
        hub.add(mk(2));
        let c = hub.configs();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].kind, Kind::Udp { port: 2 });
    }
}
