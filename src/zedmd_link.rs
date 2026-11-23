//! ZeDMD, the LED dot-matrix display many cabinets use in place of a DMD
//! screen: finding it (USB or the local network) and showing the selected
//! table's DMD clip on it while the launcher is up.
//!
//! The device protocol is the `zedmd` crate's. Finding is ours: the crate's
//! USB search writes its handshake to every USB serial port, so it is only
//! called once a port with a ZeDMD's USB id is seen, and the network search
//! needs a short timeout per address, which the crate's handshake does not
//! offer.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

/// USB ids of the serial bridges ZeDMD boards use (the list libzedmd checks).
const USB_IDS: [(u16, u16); 4] = [
    (0x303a, 0x1001), // ESP32-S3 native USB
    (0x1a86, 0x55d3), // CH343
    (0x1a86, 0x7523), // CH340
    (0x10c4, 0xea60), // CP210x
];

/// The name a ZeDMD announces on the network, and Visual Pinball's default.
pub const DEFAULT_WIFI_HOST: &str = "zedmd-wifi.local";

/// How a ZeDMD is reached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    Usb,
    Wifi(String),
}

impl Link {
    /// The form kept in PinReady's database.
    pub fn to_db(&self) -> String {
        match self {
            Link::Usb => "usb".to_string(),
            Link::Wifi(host) => format!("wifi:{host}"),
        }
    }

    pub fn from_db(s: &str) -> Option<Self> {
        match s {
            "usb" => Some(Link::Usb),
            _ => s
                .strip_prefix("wifi:")
                .filter(|h| !h.is_empty())
                .map(|h| Link::Wifi(h.to_string())),
        }
    }
}

/// A ZeDMD that answered. Size and firmware are known for a network one
/// only: a USB one is not opened while searching.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub link: Link,
    pub size: Option<(u32, u32)>,
    pub firmware: Option<String>,
}

/// Write Visual Pinball's DMDUtil plugin settings for this ZeDMD, so the
/// tables drive it without the user filling the plugin's page.
pub fn write_vpx_settings(config: &mut crate::config::VpxConfig, link: &Link) {
    const S: &str = "Plugin.DMDUtil";
    config.set_i32(S, "Enable", 1);
    config.set_i32(S, "ZeDMD", 1);
    match link {
        Link::Usb => {
            config.set_i32(S, "ZeDMDWiFiEnabled", 0);
            // empty device: DMDUtil searches the USB ports itself
            config.set(S, "ZeDMDDevice", "");
        }
        Link::Wifi(host) => {
            config.set_i32(S, "ZeDMDWiFiEnabled", 1);
            config.set(S, "ZeDMDWiFiAddr", host);
        }
    }
}

/// A serial port whose USB id is a ZeDMD's.
pub fn usb_port() -> Option<String> {
    serialport::available_ports()
        .ok()?
        .into_iter()
        .find(|p| match &p.port_type {
            serialport::SerialPortType::UsbPort(info) => USB_IDS.contains(&(info.vid, info.pid)),
            _ => false,
        })
        .map(|p| p.port_name)
}

/// Ask `host` for a ZeDMD handshake (`GET /handshake`, which answers
/// `width|height|firmware|…`).
pub fn probe_wifi(host: &str, timeout: Duration) -> Option<Found> {
    let addr = (host, 80)
        .to_socket_addrs()
        .ok()?
        .find(SocketAddr::is_ipv4)?;
    let mut stream = TcpStream::connect_timeout(&addr, timeout).ok()?;
    let io_timeout = Some(timeout.max(Duration::from_millis(800)));
    stream.set_read_timeout(io_timeout).ok()?;
    stream.set_write_timeout(io_timeout).ok()?;
    let request = format!(
        "GET /handshake HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nUser-Agent: PinReady\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).ok()?;
    let mut response = Vec::with_capacity(512);
    // a stranger may never close: read a bounded amount
    let _ = stream.take(4096).read_to_end(&mut response);
    let (size, firmware) = parse_handshake(&response)?;
    // keep a name the user can read when it resolved, the address otherwise
    let host = if host.parse::<IpAddr>().is_ok() || host == DEFAULT_WIFI_HOST {
        host.to_string()
    } else {
        addr.ip().to_string()
    };
    Some(Found {
        link: Link::Wifi(host),
        size: Some(size),
        firmware: Some(firmware),
    })
}

/// Width, height and firmware out of a handshake HTTP response.
fn parse_handshake(response: &[u8]) -> Option<((u32, u32), String)> {
    let sep = response.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&response[..sep]).ok()?;
    if !head.lines().next()?.contains(" 200") {
        return None;
    }
    let body = std::str::from_utf8(&response[sep + 4..]).ok()?;
    let mut fields = body.trim().split('|');
    let width: u32 = fields.next()?.parse().ok()?;
    let height: u32 = fields.next()?.parse().ok()?;
    let firmware = fields.next()?.trim();
    // the panels ZeDMD drives: 128x32 and 256x64 (and halves of them)
    let plausible = (64..=256).contains(&width) && (16..=64).contains(&height);
    let looks_like_version =
        firmware.split('.').count() >= 2 && firmware.split('.').all(|p| p.parse::<u32>().is_ok());
    (plausible && looks_like_version).then(|| ((width, height), firmware.to_string()))
}

/// This machine's address on its local network, private ranges only (the
/// search never leaves the local network).
fn local_ipv4() -> Option<Ipv4Addr> {
    // connecting a UDP socket sends nothing: it only picks the route
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if ip.is_private() => Some(ip),
        _ => None,
    }
}

/// Look for a ZeDMD: on USB, then at `known` (the address already in VPX's
/// settings), at its network name, and last at every address of the local
/// network (its /24), a few dozen at a time. Blocks for a few seconds at
/// most; run it off the UI thread.
pub fn find(known: Option<&str>) -> Option<Found> {
    if usb_port().is_some() {
        return Some(Found {
            link: Link::Usb,
            size: None,
            firmware: None,
        });
    }
    let named = known
        .map(str::trim)
        .filter(|k| !k.is_empty() && *k != DEFAULT_WIFI_HOST)
        .into_iter()
        .chain([DEFAULT_WIFI_HOST]);
    for host in named {
        if let Some(found) = probe_wifi(host, Duration::from_millis(1500)) {
            return Some(found);
        }
    }
    let me = local_ipv4()?;
    let [a, b, c, mine] = me.octets();
    let hosts: Vec<Ipv4Addr> = (1..=254u8)
        .filter(|&d| d != mine)
        .map(|d| Ipv4Addr::new(a, b, c, d))
        .collect();
    let hosts = Arc::new(hosts);
    let next = Arc::new(AtomicUsize::new(0));
    let done = Arc::new(AtomicBool::new(false));
    let (tx, rx) = crossbeam_channel::unbounded();
    let workers: Vec<_> = (0..32)
        .filter_map(|i| {
            let (hosts, next, done, tx) = (hosts.clone(), next.clone(), done.clone(), tx.clone());
            std::thread::Builder::new()
                .name(format!("zedmd-scan-{i}"))
                .spawn(move || {
                    while !done.load(Ordering::Relaxed) {
                        let Some(ip) = hosts.get(next.fetch_add(1, Ordering::Relaxed)) else {
                            break;
                        };
                        if let Some(found) = probe_wifi(&ip.to_string(), Duration::from_millis(400))
                        {
                            done.store(true, Ordering::Relaxed);
                            let _ = tx.send(found);
                        }
                    }
                })
                .ok()
        })
        .collect();
    drop(tx);
    let found = rx.recv().ok();
    done.store(true, Ordering::Relaxed);
    for w in workers {
        let _ = w.join();
    }
    found
}

/// A picture for the ZeDMD: RGBA pixels and their size.
pub struct Picture {
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
}

enum Msg {
    Show(Picture),
    Clear,
}

/// The connection to the ZeDMD, held by its own thread while the launcher
/// shows. Dropped (or [`Output::close`]d) before Visual Pinball starts: a
/// ZeDMD takes one client, and the table's DMDUtil must find it free.
pub struct Output {
    tx: Option<crossbeam_channel::Sender<Msg>>,
    done: crossbeam_channel::Receiver<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Output {
    pub fn open(link: Link) -> Option<Self> {
        let (tx, rx) = crossbeam_channel::bounded::<Msg>(2);
        let (done_tx, done) = crossbeam_channel::bounded(1);
        let thread = std::thread::Builder::new()
            .name("zedmd-output".into())
            .spawn(move || {
                run(&link, &rx);
                let _ = done_tx.send(());
            })
            .ok()?;
        Some(Self {
            tx: Some(tx),
            done,
            thread: Some(thread),
        })
    }

    /// Show `picture`, fitted to the panel. Dropped when the device is
    /// still busy with the previous one.
    pub fn show(&self, picture: Picture) {
        if let Some(tx) = &self.tx {
            let _ = tx.try_send(Msg::Show(picture));
        }
    }

    pub fn clear(&self) {
        if let Some(tx) = &self.tx {
            let _ = tx.try_send(Msg::Clear);
        }
    }

    /// Whether the thread ended by itself (the device was not found or went
    /// away).
    pub fn ended(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }

    /// Release the device, waiting a moment for it to be free.
    pub fn close(mut self) {
        self.shut(Duration::from_secs(3));
    }

    fn shut(&mut self, wait: Duration) {
        self.tx = None;
        if self.done.recv_timeout(wait).is_ok()
            && let Some(t) = self.thread.take()
        {
            let _ = t.join();
        }
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        self.shut(Duration::from_millis(500));
    }
}

fn run(link: &Link, rx: &crossbeam_channel::Receiver<Msg>) {
    let connected = match link {
        Link::Usb => {
            if usb_port().is_none() {
                return;
            }
            zedmd::connect()
        }
        Link::Wifi(host) => zedmd::connect_wifi(host),
    };
    let mut dmd = match connected {
        Ok(d) => d,
        Err(e) => {
            log::warn!("ZeDMD ({}): not reached: {e}", link.to_db());
            return;
        }
    };
    let _ = dmd.run();
    let (w, h) = (dmd.width(), dmd.height());
    log::info!("ZeDMD ({}): {w}x{h} connected", link.to_db());
    let mut last: Option<Vec<u8>> = None;
    loop {
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(Msg::Show(p)) => {
                let frame = fit_rgb(&p, w, h);
                dmd.render_rgb888_frame(&frame);
                last = Some(frame);
            }
            Ok(Msg::Clear) => {
                last = None;
                let _ = dmd.clear_screen();
            }
            // the same picture again: nothing changes on the panel, but the
            // device hears from us and keeps it up
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                if let Some(f) = &last {
                    dmd.render_rgb888_frame(f);
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
    }
    let _ = dmd.clear_screen();
    let _ = dmd.stop();
    log::info!("ZeDMD ({}): released", link.to_db());
}

/// `picture` fitted whole in `w`×`h`, centred on black, as packed RGB.
fn fit_rgb(picture: &Picture, w: u32, h: u32) -> Vec<u8> {
    let mut out = vec![0u8; (w * h * 3) as usize];
    let [sw, sh] = picture.size;
    let Some(src) = image::RgbaImage::from_raw(sw as u32, sh as u32, picture.rgba.clone()) else {
        return out;
    };
    if sw == 0 || sh == 0 || w == 0 || h == 0 {
        return out;
    }
    let (fw, fh) = crate::video::fit(sw as u32, sh as u32, w, h);
    let small = image::imageops::resize(
        &src,
        fw.max(1),
        fh.max(1),
        image::imageops::FilterType::Triangle,
    );
    let (ox, oy) = ((w - small.width()) / 2, (h - small.height()) / 2);
    for (x, y, px) in small.enumerate_pixels() {
        let i = (((oy + y) * w + ox + x) * 3) as usize;
        let a = px[3] as u32;
        for c in 0..3 {
            out[i + c] = (px[c] as u32 * a / 255) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(body: &str) -> Vec<u8> {
        format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\n{body}").into_bytes()
    }

    #[test]
    fn reads_a_zedmd_handshake() {
        let r = http("128|32|5.1.8|0|UDP|3333|5|256|7|3|0|0|8|2|30|0|net|0|63531");
        assert_eq!(parse_handshake(&r), Some(((128, 32), "5.1.8".to_string())));
        let r = http("256|64|5.2.0|1|TCP|3333");
        assert_eq!(parse_handshake(&r), Some(((256, 64), "5.2.0".to_string())));
    }

    #[test]
    fn other_web_servers_are_not_a_zedmd() {
        assert_eq!(parse_handshake(&http("<html>router</html>")), None);
        assert_eq!(parse_handshake(&http("1920|1080|1.0")), None);
        assert_eq!(parse_handshake(&http("128|32|hello")), None);
        let not_found = b"HTTP/1.1 404 Not Found\r\n\r\n128|32|5.1.8".to_vec();
        assert_eq!(parse_handshake(&not_found), None);
    }

    #[test]
    fn link_round_trips_through_the_database() {
        for link in [Link::Usb, Link::Wifi("192.168.0.63".into())] {
            assert_eq!(Link::from_db(&link.to_db()), Some(link));
        }
        assert_eq!(Link::from_db("wifi:"), None);
        assert_eq!(Link::from_db(""), None);
    }

    #[test]
    fn a_wide_clip_is_letterboxed_on_the_panel() {
        // 4x1 white picture on a 4x2 panel: one lit row
        let p = Picture {
            size: [4, 1],
            rgba: vec![255; 16],
        };
        let out = fit_rgb(&p, 4, 2);
        assert_eq!(out.len(), 24);
        assert!(out[..12].iter().all(|&b| b == 255) || out[12..].iter().all(|&b| b == 255));
    }
}
