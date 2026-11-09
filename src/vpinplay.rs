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
    /// File name of the `.vpx` (not its path).
    pub filename: String,
    /// Rating in tenths of a star; 0 = not rated.
    pub rating_tenths: u8,
    pub last_run: Option<i64>,
    pub start_count: u32,
    pub run_time_secs: u32,
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
pub fn build_payload(settings: &Settings, tables: &[PushTable], now: i64) -> Value {
    let tables: Vec<Value> = tables
        .iter()
        .map(|t| {
            let mut user = serde_json::Map::new();
            let stars = ((t.rating_tenths as f32) / 10.0).round() as u8;
            if stars > 0 {
                user.insert("rating".into(), json!(stars.min(5)));
            }
            if let Some(last) = t.last_run {
                user.insert("lastRun".into(), json!(iso(last)));
            }
            user.insert("startCount".into(), json!(t.start_count));
            user.insert("runTime".into(), json!(t.run_time_secs / 60));
            json!({
                "info": { "vpsId": t.vps_id },
                "user": Value::Object(user),
                "vpxFile": {
                    "filename": t.filename,
                    "filehash": "",
                    "version": "",
                    "vbsHash": "",
                    "rom": "",
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
        let payload = build_payload(settings, chunk, now);
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
            filename: "x.vpx".into(),
            rating_tenths: 34, // 3.4 stars -> 3
            last_run: Some(1_700_000_000),
            start_count: 12,
            run_time_secs: 3700, // 61 minutes
        };
        let unrated = PushTable {
            vps_id: "abc".into(),
            filename: "y.vpx".into(),
            rating_tenths: 0,
            last_run: None,
            start_count: 1,
            run_time_secs: 30,
        };
        let v = build_payload(&settings("http://x"), &[t, unrated], 1_700_000_100);
        assert_eq!(v["source"]["program"], "PinReady");
        assert_eq!(v["client"]["userId"], "cab-test");
        assert_eq!(v["client"]["machineId"].as_str().unwrap().len(), 64);
        assert_eq!(v["sentAt"], "2023-11-14T22:15:00Z");
        let t0 = &v["tables"][0];
        assert_eq!(t0["info"]["vpsId"], "lkSumsrF");
        assert_eq!(t0["user"]["rating"], 3);
        assert_eq!(t0["user"]["lastRun"], "2023-11-14T22:13:20Z");
        assert_eq!(t0["user"]["startCount"], 12);
        assert_eq!(t0["user"]["runTime"], 61);
        assert_eq!(t0["vpxFile"]["filename"], "x.vpx");
        assert_eq!(t0["vpxFile"]["filehash"], "");
        // No rating key for an unrated table, no lastRun without a date.
        let t1 = &v["tables"][1];
        assert!(t1["user"].get("rating").is_none());
        assert!(t1["user"].get("lastRun").is_none());
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
