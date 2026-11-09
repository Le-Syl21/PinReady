//! VPinPlay client: community ratings of tables, and sharing the player's own
//! usage (rating, last played, number of starts, play time) in return.
//!
//! VPinPlay (https://www.vpinplay.com, source github.com/superhac/vpinplay)
//! is a volunteer-run service that VPinFE also talks to. Facts this module
//! relies on, read from its source:
//! - tables are identified by their VPS id; ratings are whole stars 1 to 5;
//! - `POST {base}/api/v1/sync` takes the player's data, `GET
//!   /api/v1/tables-plus/search` pages through the aggregate ratings;
//! - there is no authentication: a user name plus a random machine id, both
//!   chosen by the client, are the only identity, and everything sent is
//!   publicly readable by user name.
//!
//! So nothing here runs unless the player turned it on, and the settings page
//! says what is sent before they do.

use crate::db::Database;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

pub const DEFAULT_ENDPOINT: &str = "https://api.vpinplay.com:8888";
/// How long fetched community ratings are kept before they are fetched again.
pub const RATINGS_MAX_AGE_SECS: i64 = 24 * 3600;
/// Tables per request when sending everything.
const PUSH_CHUNK: usize = 200;
/// Safety stop when paging through the ratings (100 per page).
const MAX_PAGES: usize = 100;

const KEY_ENABLED: &str = "vpinplay_enabled";
const KEY_USER: &str = "vpinplay_user_id";
const KEY_INITIALS: &str = "vpinplay_initials";
const KEY_MACHINE: &str = "vpinplay_machine_id";
const KEY_ENDPOINT: &str = "vpinplay_endpoint";
pub const KEY_RATINGS_AT: &str = "vpinplay_ratings_fetched_at";

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .new_agent()
    })
}

/// The player's VPinPlay settings, stored in the config table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    pub user_id: String,
    pub initials: String,
    pub machine_id: String,
    pub endpoint: String,
}

impl Settings {
    pub fn load(db: &Database) -> Self {
        Self {
            enabled: db.get_config(KEY_ENABLED).as_deref() == Some("1"),
            user_id: db.get_config(KEY_USER).unwrap_or_default(),
            initials: db.get_config(KEY_INITIALS).unwrap_or_default(),
            machine_id: db.get_config(KEY_MACHINE).unwrap_or_default(),
            endpoint: db
                .get_config(KEY_ENDPOINT)
                .map(|e| e.trim().trim_end_matches('/').to_string())
                .filter(|e| !e.is_empty())
                .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string()),
        }
    }

    /// Turn the sharing on or off. The first time it is turned on a random
    /// machine id is made (and kept).
    pub fn set_enabled(&mut self, db: &Database, enabled: bool) -> Result<()> {
        if enabled && self.machine_id.len() != 64 {
            self.machine_id = generate_machine_id()?;
            db.set_config(KEY_MACHINE, &self.machine_id)?;
        }
        self.enabled = enabled;
        db.set_config(KEY_ENABLED, if enabled { "1" } else { "0" })
    }

    /// Store the name and initials (normalised, see `normalize_user_id` and
    /// `normalize_initials`).
    pub fn set_identity(&mut self, db: &Database, user_id: &str, initials: &str) -> Result<()> {
        self.user_id = normalize_user_id(user_id);
        self.initials = normalize_initials(initials);
        db.set_config(KEY_USER, &self.user_id)?;
        db.set_config(KEY_INITIALS, &self.initials)
    }

    /// Sharing is on and the identity is complete.
    pub fn ready(&self) -> bool {
        self.enabled
            && is_valid_user_id(&self.user_id)
            && !self.initials.is_empty()
            && self.machine_id.len() == 64
    }
}

/// The server trims and lowercases user names; we also keep to letters,
/// digits, `_` and `-`.
pub fn normalize_user_id(raw: &str) -> String {
    raw.trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .take(32)
        .collect()
}

pub fn is_valid_user_id(id: &str) -> bool {
    (3..=32).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// One to three upper-case letters or digits.
pub fn normalize_initials(raw: &str) -> String {
    raw.trim()
        .to_uppercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(3)
        .collect()
}

/// 64 random letters and digits, as VPinPlay requires.
pub fn generate_machine_id() -> Result<String> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut bytes = [0u8; 64];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("no random source: {e}"))?;
    // 256 is not a multiple of 62: drop the bytes that would favour some
    // letters, and draw more if needed.
    let mut id = String::with_capacity(64);
    let mut pool = bytes.to_vec();
    while id.len() < 64 {
        for b in pool.drain(..) {
            if b < 248 && id.len() < 64 {
                id.push(ALPHABET[(b % 62) as usize] as char);
            }
        }
        if id.len() < 64 {
            pool = vec![0u8; 64];
            getrandom::fill(&mut pool).map_err(|e| anyhow::anyhow!("no random source: {e}"))?;
        }
    }
    Ok(id)
}

/// What a background VPinPlay request found out.
pub enum Event {
    Ratings(Result<HashMap<String, CommunityRating>, String>),
    Pushed(Result<usize, String>),
    NameChecked(Result<bool, String>),
}

/// The community rating of a table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CommunityRating {
    /// Average, 1 to 5.
    pub avg: f32,
    pub count: u32,
}

#[derive(Deserialize)]
struct SearchPage {
    items: Vec<SearchItem>,
    pagination: Pagination,
}

#[derive(Deserialize)]
struct SearchItem {
    #[serde(rename = "vpsId")]
    vps_id: String,
    #[serde(rename = "avgRating")]
    avg_rating: Option<f64>,
    #[serde(rename = "ratingCount", default)]
    rating_count: u32,
}

#[derive(Deserialize)]
struct Pagination {
    #[serde(rename = "hasNext", default)]
    has_next: bool,
}

/// The aggregate rating of every table VPinPlay knows with at least one
/// rating, keyed by VPS id. Read-only: nothing about the player is sent.
pub fn fetch_ratings(endpoint: &str) -> Result<HashMap<String, CommunityRating>> {
    let mut out = HashMap::new();
    for page in 0..MAX_PAGES {
        let url = format!(
            "{endpoint}/api/v1/tables-plus/search?limit=100&offset={}&sort_by=ratingCount&sort_order=-1",
            page * 100
        );
        let body = agent()
            .get(&url)
            .header("User-Agent", "PinReady")
            .call()
            .with_context(|| format!("VPinPlay: could not read {url}"))?
            .into_body()
            .read_to_string()?;
        let parsed: SearchPage =
            serde_json::from_str(&body).context("VPinPlay: unexpected ratings answer")?;
        for item in parsed.items {
            if let Some(avg) = item.avg_rating
                && item.rating_count > 0
            {
                out.insert(
                    item.vps_id,
                    CommunityRating {
                        avg: avg as f32,
                        count: item.rating_count,
                    },
                );
            }
        }
        if !parsed.pagination.has_next {
            break;
        }
    }
    Ok(out)
}

/// Whether a user name is free (read-only).
pub fn user_available(endpoint: &str, user_id: &str) -> Result<bool> {
    let url = format!("{endpoint}/api/v1/users/{user_id}/available");
    let body = agent()
        .get(&url)
        .header("User-Agent", "PinReady")
        .call()
        .with_context(|| format!("VPinPlay: could not read {url}"))?
        .into_body()
        .read_to_string()?;
    let v: Value = serde_json::from_str(&body)?;
    v.get("available")
        .and_then(Value::as_bool)
        .context("VPinPlay: unexpected answer to the name check")
}

/// What is sent about one table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushTable {
    pub vps_id: String,
    /// The `.vpx`: read when sending, for the facts VPinPlay wants about the
    /// file itself (see [`VpxMeta`]).
    pub vpx_path: std::path::PathBuf,
    /// File name of the `.vpx` (not its path).
    pub filename: String,
    /// Rating in tenths of a star; 0 = not rated.
    pub rating_tenths: u8,
    pub last_run: Option<i64>,
    pub start_count: u32,
    pub run_time_secs: u32,
}

/// Facts about a script that VPinPlay records, the way VPinFE computes them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Detect {
    pub nfozzy: bool,
    pub fleep: bool,
    pub ssf: bool,
    pub lut: bool,
    pub scorebit: bool,
    pub fastflips: bool,
    pub flex: bool,
}

/// What VPinPlay wants to know about the `.vpx` file itself. Everything is
/// optional on the wire (an empty string passes), but the server keeps one
/// rating per user and per `vpxFile` object, so a table must always be sent
/// with the same facts or its rating would be counted twice.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VpxMeta {
    /// SHA-256 of the whole file.
    pub filehash: String,
    pub version: String,
    pub release_date: String,
    pub save_date: String,
    pub save_rev: String,
    pub manufacturer: String,
    pub year: String,
    pub table_type: String,
    /// SHA-256 of the script (the `.vbs` next to the table when there is
    /// one, else the embedded one), with CRLF line ends.
    pub vbs_hash: String,
    pub rom: String,
    pub detect: Detect,
}

fn sha256_file(path: &std::path::Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// A script's comments removed (a `'` outside a string starts one), as
/// VPinFE does before looking for the ROM name.
fn strip_vbs_comments(script: &str) -> String {
    script
        .lines()
        .map(|line| {
            let chars: Vec<char> = line.chars().collect();
            let mut code = String::new();
            let mut in_string = false;
            let mut i = 0;
            while i < chars.len() {
                let c = chars[i];
                if c == '"' {
                    code.push(c);
                    if in_string && i + 1 < chars.len() && chars[i + 1] == '"' {
                        code.push(chars[i + 1]);
                        i += 2;
                        continue;
                    }
                    in_string = !in_string;
                } else if c == '\'' && !in_string {
                    break;
                } else {
                    code.push(c);
                }
                i += 1;
            }
            code
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every line end as CRLF, whatever it was.
fn to_crlf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

/// The ROM name a script uses, as VPinFE finds it: `cGameName = "..."`, else
/// `cOptRom = "..."`.
fn rom_of(script: &str) -> String {
    use regex::Regex;
    static GAME: OnceLock<Regex> = OnceLock::new();
    static OPT: OnceLock<Regex> = OnceLock::new();
    let game = GAME.get_or_init(|| Regex::new(r#"(?i)c?gamename\s*=\s*"([^"]+)""#).unwrap());
    let opt = OPT.get_or_init(|| Regex::new(r#"(?i)c?OptRom\s*=\s*"([^\s]+)""#).unwrap());
    let code = strip_vbs_comments(script);
    game.captures(&code)
        .or_else(|| opt.captures(&code))
        .map(|c| c[1].to_string())
        .unwrap_or_default()
}

fn detect_in(script: &str) -> Detect {
    let s = script.to_lowercase();
    Detect {
        nfozzy: s.contains("class flipperpolarity"),
        fleep: s.contains("rubberstrongsoundfactor"),
        ssf: s.contains("playsoundat"),
        lut: s.contains("lut"),
        scorebit: s.contains("scorebit"),
        fastflips: s.contains("fastflips"),
        flex: s.contains("flexdmd"),
    }
}

impl VpxMeta {
    /// The facts of a script: its hash, ROM name and what it uses.
    pub fn from_script(script: &str) -> Self {
        let script = to_crlf(script);
        Self {
            vbs_hash: crate::vbs_patches::sha256_hex(script.as_bytes()),
            rom: rom_of(&script),
            detect: detect_in(&script),
            ..Self::default()
        }
    }

    /// Read what can be read from the `.vpx`; whatever fails stays empty.
    pub fn read(vpx_path: &std::path::Path) -> Self {
        let sidecar = vpx_path.with_extension("vbs");
        let script = match std::fs::read(&sidecar) {
            Ok(bytes) => {
                let bytes = bytes
                    .strip_prefix(&[0xEF, 0xBB, 0xBF][..])
                    .unwrap_or(&bytes);
                Some(String::from_utf8_lossy(bytes).into_owned())
            }
            Err(_) => crate::vbs_patches::extract_embedded_vbs(vpx_path)
                .map_err(|e| log::warn!("VPinPlay: no script for {}: {e:#}", vpx_path.display()))
                .ok(),
        };
        let mut meta = script.as_deref().map(Self::from_script).unwrap_or_default();
        match sha256_file(vpx_path) {
            Ok(h) => meta.filehash = h,
            Err(e) => log::warn!("VPinPlay: cannot hash {}: {e}", vpx_path.display()),
        }
        match vpin::vpx::open(vpx_path).and_then(|mut v| v.read_tableinfo()) {
            Ok(info) => {
                let prop = |name: &str| {
                    info.properties
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case(name))
                        .map(|(_, v)| v.trim_matches('\0').to_string())
                        .unwrap_or_default()
                };
                meta.version = info.table_version.unwrap_or_default();
                meta.release_date = info.release_date.unwrap_or_default();
                meta.save_date = info.table_save_date.unwrap_or_default();
                meta.save_rev = info.table_save_rev.unwrap_or_default();
                meta.manufacturer = prop("CompanyName");
                meta.year = prop("CompanyYear");
                meta.table_type = prop("TableType");
            }
            Err(e) => log::warn!("VPinPlay: no table info in {}: {e}", vpx_path.display()),
        }
        meta
    }
}

fn iso(unix: i64) -> String {
    use time::format_description::well_known::Rfc3339;
    time::OffsetDateTime::from_unix_timestamp(unix)
        .ok()
        .and_then(|t| t.format(&Rfc3339).ok())
        .unwrap_or_default()
}

/// The JSON body of a sync request (pure, so it can be tested). VPinPlay
/// wants whole stars (the rating is rounded; unrated tables carry none) and
/// run time in minutes, as VPinFE sends it.
pub fn build_payload(settings: &Settings, tables: &[(&PushTable, VpxMeta)], now: i64) -> Value {
    let tables: Vec<Value> = tables
        .iter()
        .map(|(t, m)| {
            let stars = ((t.rating_tenths as f32) / 10.0).round() as u8;
            json!({
                "info": { "vpsId": t.vps_id, "rom": m.rom },
                // Like VPinFE: 0 = unrated, no date yet = null.
                "user": {
                    "rating": stars.min(5),
                    "lastRun": t.last_run.map(iso),
                    "startCount": t.start_count,
                    "runTime": t.run_time_secs / 60,
                    "score": Value::Null,
                },
                "vpxFile": {
                    "filename": t.filename,
                    "filehash": m.filehash,
                    "version": m.version,
                    "releaseDate": m.release_date,
                    "saveDate": m.save_date,
                    "saveRev": m.save_rev,
                    "manufacturer": m.manufacturer,
                    "year": m.year,
                    "type": m.table_type,
                    "vbsHash": m.vbs_hash,
                    "rom": m.rom,
                    "detectNfozzy": m.detect.nfozzy,
                    "detectFleep": m.detect.fleep,
                    "detectSSF": m.detect.ssf,
                    "detectLUT": m.detect.lut,
                    "detectScorebit": m.detect.scorebit,
                    "detectFastflips": m.detect.fastflips,
                    "detectFlex": m.detect.flex,
                },
                "vpinfe": { "alttitle": "", "altvpsid": "" },
            })
        })
        .collect();
    json!({
        "source": { "program": "PinReady", "programVersion": env!("CARGO_PKG_VERSION") },
        "client": {
            "userId": settings.user_id,
            "initials": settings.initials,
            "machineId": settings.machine_id,
        },
        "sentAt": iso(now),
        "tables": tables,
    })
}

/// Send the player's data. Returns how many tables the server accepted.
/// VPinPlay answers HTTP 200 with `status: "error"` on an identity clash
/// (name already used with another machine id), so the body is checked.
pub fn push(settings: &Settings, tables: &[PushTable], now: i64) -> Result<usize> {
    if !settings.ready() {
        bail!("VPinPlay is not set up");
    }
    let url = format!("{}/api/v1/sync", settings.endpoint);
    let mut sent = 0;
    for chunk in tables.chunks(PUSH_CHUNK) {
        // Reading the files (hash, script, table info) is the slow part.
        let items: Vec<(&PushTable, VpxMeta)> = chunk
            .iter()
            .map(|t| (t, VpxMeta::read(&t.vpx_path)))
            .collect();
        let payload = build_payload(settings, &items, now);
        let body = agent()
            .post(&url)
            .header("User-Agent", "PinReady")
            .header("Content-Type", "application/json")
            .send(payload.to_string())
            .with_context(|| format!("VPinPlay: could not send to {url}"))?
            .into_body()
            .read_to_string()?;
        let answer: Value =
            serde_json::from_str(&body).context("VPinPlay: unexpected answer to the sync")?;
        if answer.get("status").and_then(Value::as_str) != Some("ok") {
            bail!("VPinPlay refused the data (is the user name already taken by another cabinet?)");
        }
        sent += chunk.len();
    }
    Ok(sent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// `(request line, body)` of each request the mock received.
    type RequestLog = Arc<Mutex<Vec<(String, String)>>>;

    fn settings(endpoint: &str) -> Settings {
        Settings {
            enabled: true,
            user_id: "cab-test".into(),
            initials: "AB".into(),
            machine_id: "A".repeat(64),
            endpoint: endpoint.into(),
        }
    }

    /// A one-thread HTTP server answering each request with the next canned
    /// body, recording `(request line, body)`.
    fn mock(answers: Vec<String>) -> (String, RequestLog) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let log2 = log.clone();
        std::thread::spawn(move || {
            for answer in answers {
                let (mut s, _) = listener.accept().unwrap();
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                let (head_end, content_length) = loop {
                    let n = s.read(&mut chunk).unwrap();
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..p]).to_lowercase();
                        let cl = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        break (p + 4, cl);
                    }
                };
                while buf.len() < head_end + content_length {
                    let n = s.read(&mut chunk).unwrap();
                    buf.extend_from_slice(&chunk[..n]);
                }
                let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                let body = String::from_utf8_lossy(&buf[head_end..]).to_string();
                log2.lock()
                    .unwrap()
                    .push((head.lines().next().unwrap_or("").to_string(), body));
                let resp = format!(
                    "HTTP/1.1 200 OK\\r\\nContent-Type: application/json\\r\\nContent-Length: {}\\r\\nConnection: close\\r\\n\\r\\n{}",
                    answer.len(),
                    answer
                )
                .replace("\\r\\n", "\r\n");
                s.write_all(resp.as_bytes()).unwrap();
            }
        });
        (format!("http://{addr}"), log)
    }

    #[test]
    fn names_and_initials_are_normalised() {
        assert_eq!(normalize_user_id("  Cab_One! "), "cab_one");
        assert!(is_valid_user_id("cab-1"));
        assert!(!is_valid_user_id("ab"));
        assert_eq!(normalize_initials(" a.b.c.d "), "ABC");
    }

    #[test]
    fn machine_ids_are_64_alphanumerics_and_differ() {
        let a = generate_machine_id().unwrap();
        let b = generate_machine_id().unwrap();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(a, b);
    }

    #[test]
    fn the_payload_matches_what_vpinplay_expects() {
        let t = PushTable {
            vps_id: "lkSumsrF".into(),
            vpx_path: "/nonexistent/x.vpx".into(),
            filename: "x.vpx".into(),
            rating_tenths: 34, // 3.4 stars -> 3
            last_run: Some(1_700_000_000),
            start_count: 12,
            run_time_secs: 3700, // 61 minutes
        };
        let unrated = PushTable {
            vps_id: "abc".into(),
            vpx_path: "/nonexistent/y.vpx".into(),
            filename: "y.vpx".into(),
            rating_tenths: 0,
            last_run: None,
            start_count: 1,
            run_time_secs: 30,
        };
        let meta = VpxMeta {
            filehash: "ab".repeat(32),
            version: "1.2".into(),
            release_date: "2022-05-01".into(),
            save_date: "Mon May 02 2022".into(),
            save_rev: "77".into(),
            manufacturer: "Bally".into(),
            year: "1995".into(),
            table_type: "Solid State".into(),
            vbs_hash: "cd".repeat(32),
            rom: "afm_113b".into(),
            detect: Detect {
                nfozzy: true,
                flex: true,
                ..Detect::default()
            },
        };
        let v = build_payload(
            &settings("http://x"),
            &[(&t, meta), (&unrated, VpxMeta::default())],
            1_700_000_100,
        );
        assert_eq!(v["source"]["program"], "PinReady");
        assert_eq!(v["client"]["userId"], "cab-test");
        assert_eq!(v["client"]["machineId"].as_str().unwrap().len(), 64);
        assert_eq!(v["sentAt"], "2023-11-14T22:15:00Z");
        let t0 = &v["tables"][0];
        assert_eq!(t0["info"]["vpsId"], "lkSumsrF");
        assert_eq!(t0["info"]["rom"], "afm_113b");
        assert_eq!(t0["user"]["rating"], 3);
        assert_eq!(t0["user"]["lastRun"], "2023-11-14T22:13:20Z");
        assert_eq!(t0["user"]["startCount"], 12);
        assert_eq!(t0["user"]["runTime"], 61);
        assert!(t0["user"]["score"].is_null());
        let f = &t0["vpxFile"];
        assert_eq!(f["filename"], "x.vpx");
        assert_eq!(f["filehash"].as_str().unwrap().len(), 64);
        assert_eq!(f["version"], "1.2");
        assert_eq!(f["releaseDate"], "2022-05-01");
        assert_eq!(f["saveRev"], "77");
        assert_eq!(f["manufacturer"], "Bally");
        assert_eq!(f["year"], "1995");
        assert_eq!(f["type"], "Solid State");
        assert_eq!(f["vbsHash"].as_str().unwrap().len(), 64);
        assert_eq!(f["rom"], "afm_113b");
        assert_eq!(f["detectNfozzy"], true);
        assert_eq!(f["detectFlex"], true);
        assert_eq!(f["detectSSF"], false);
        // Unrated: 0 and no date, like VPinFE.
        let t1 = &v["tables"][1];
        assert_eq!(t1["user"]["rating"], 0);
        assert!(t1["user"]["lastRun"].is_null());
    }

    #[test]
    fn a_script_gives_its_hash_rom_and_features() {
        let script = "' cGameName = \"commented\"\nConst cGameName = \"afm_113b\" ' the rom\nDim x\nClass FlipperPolarity\nEnd Class\nx = \"it's not a comment\"\n";
        let m = VpxMeta::from_script(script);
        assert_eq!(m.rom, "afm_113b");
        assert!(m.detect.nfozzy && !m.detect.flex && !m.detect.fleep);
        // The hash is over the CRLF form, whatever the line ends were.
        let crlf = "Dim x\r\nx = 1\r\n";
        assert_eq!(
            VpxMeta::from_script("Dim x\nx = 1\n").vbs_hash,
            VpxMeta::from_script(crlf).vbs_hash
        );
        assert_eq!(
            VpxMeta::from_script(crlf).vbs_hash,
            crate::vbs_patches::sha256_hex(crlf.as_bytes())
        );
        // OptRom is the fallback.
        assert_eq!(rom_of("cOptRom = \"tz_94h\""), "tz_94h");
        assert_eq!(rom_of("nothing here"), "");
        assert_eq!(
            strip_vbs_comments("a = \"x'y\" ' c\nb = 1"),
            "a = \"x'y\" \nb = 1"
        );
    }

    /// Reads a real table: `VPP_REAL_VPX=/path/t.vpx cargo test reads_a_real -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn reads_a_real_vpx() {
        let Ok(path) = std::env::var("VPP_REAL_VPX") else {
            return;
        };
        let m = VpxMeta::read(std::path::Path::new(&path));
        println!("{m:#?}");
        if let Ok(mut v) = vpin::vpx::open(std::path::Path::new(&path))
            && let Ok(info) = v.read_tableinfo()
        {
            println!(
                "table info properties: {:?}",
                info.properties.keys().collect::<Vec<_>>()
            );
        }
        assert_eq!(m.filehash.len(), 64);
        assert_eq!(m.vbs_hash.len(), 64);
    }

    #[test]
    fn a_file_is_hashed_in_full() {
        let dir = std::env::temp_dir().join(format!("pinready-vpp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("t.vpx");
        std::fs::write(&file, b"abc").unwrap();
        assert_eq!(
            sha256_file(&file).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // A file that cannot be read still gives a (partly empty) meta.
        std::fs::write(dir.join("t.vbs"), b"\xEF\xBB\xBFcGameName = \"abc_1\"\r\n").unwrap();
        let m = VpxMeta::read(&file);
        assert_eq!(m.rom, "abc_1");
        assert_eq!(m.filehash.len(), 64);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ratings_are_read_page_by_page() {
        let page1 = r#"{"items":[{"vpsId":"a","avgRating":4.5,"ratingCount":2},{"vpsId":"b","avgRating":null,"ratingCount":0}],"pagination":{"hasNext":true}}"#;
        let page2 = r#"{"items":[{"vpsId":"c","avgRating":3.0,"ratingCount":7}],"pagination":{"hasNext":false}}"#;
        let (url, log) = mock(vec![page1.into(), page2.into()]);
        let ratings = fetch_ratings(&url).unwrap();
        assert_eq!(ratings.len(), 2);
        assert_eq!(ratings["a"], CommunityRating { avg: 4.5, count: 2 });
        assert_eq!(ratings["c"].count, 7);
        let log = log.lock().unwrap();
        assert!(
            log[0]
                .0
                .starts_with("GET /api/v1/tables-plus/search?limit=100&offset=0")
        );
        assert!(log[1].0.contains("offset=100"));
    }

    #[test]
    fn a_sync_is_posted_and_its_status_is_checked() {
        let (url, log) = mock(vec![
            r#"{"status":"ok","summary":{}}"#.into(),
            r#"{"status":"error","summary":{"errors":1}}"#.into(),
        ]);
        let s = settings(&url);
        let t = PushTable {
            vps_id: "a".into(),
            vpx_path: "/nonexistent/a.vpx".into(),
            filename: "a.vpx".into(),
            rating_tenths: 40,
            last_run: None,
            start_count: 1,
            run_time_secs: 0,
        };
        assert_eq!(push(&s, std::slice::from_ref(&t), 0).unwrap(), 1);
        assert!(log.lock().unwrap()[0].0.starts_with("POST /api/v1/sync"));
        assert!(log.lock().unwrap()[0].1.contains("\"vpsId\":\"a\""));
        // HTTP 200 with status "error": an identity clash.
        assert!(push(&s, &[t], 0).is_err());
    }

    #[test]
    fn nothing_is_sent_when_not_set_up() {
        let mut s = settings("http://127.0.0.1:1");
        s.enabled = false;
        assert!(push(&s, &[], 0).is_err());
    }
}
