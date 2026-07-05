//! Launch-time display reconciliation across SDL video drivers.
//!
//! The wizard assigns roles to physical monitors and persists, per role, a
//! [`DisplayAnchor`](crate::display_id::DisplayAnchor) (layout position + EDID
//! fingerprint) — driver-independent. At launch we may run VPX under a
//! *different* SDL driver than the wizard used (the framerate escape hatch runs
//! VPX under XWayland while PinReady stays native Wayland), and the same
//! monitor is named differently there. This module re-resolves each role's
//! anchor to the target driver's SDL name and rewrites `*Display=` accordingly,
//! so VPX finds its screens whatever the driver.
//!
//! The switch is **atomic**: we only adopt the preferred driver if *every*
//! assigned role resolves under it; otherwise we leave the config untouched and
//! fall back to the session's own driver. No half-rewritten state ever reaches
//! VPX.

use crate::config::VpxConfig;
use crate::db::Database;
use crate::display_id::{self, DisplayAnchor, SdlDisplay};
use crate::screens::{DisplayInfo, DisplayRole};

/// `(role, ini section, `*Display*` key)` for the four assignable roles.
const ROLE_KEYS: &[(DisplayRole, &str, &str)] = &[
    (DisplayRole::Playfield, "Player", "PlayfieldDisplay"),
    (DisplayRole::Backglass, "Backglass", "BackglassDisplay"),
    (DisplayRole::Dmd, "ScoreView", "ScoreViewDisplay"),
    (DisplayRole::Topper, "Topper", "TopperDisplay"),
];

fn anchor_db_key(role: DisplayRole) -> String {
    format!("display_anchor_{role:?}")
}

fn serialize_anchor(a: &DisplayAnchor) -> String {
    format!(
        "{},{},{},{},{}",
        a.x,
        a.y,
        a.width,
        a.height,
        a.fingerprint.as_deref().unwrap_or("")
    )
}

fn parse_anchor(s: &str) -> Option<DisplayAnchor> {
    let mut it = s.splitn(5, ',');
    let x = it.next()?.parse().ok()?;
    let y = it.next()?.parse().ok()?;
    let width = it.next()?.parse().ok()?;
    let height = it.next()?.parse().ok()?;
    let fp = it.next().unwrap_or("");
    Some(DisplayAnchor {
        x,
        y,
        width,
        height,
        fingerprint: (!fp.is_empty()).then(|| fp.to_string()),
    })
}

fn load_anchor(db: &Database, role: DisplayRole) -> Option<DisplayAnchor> {
    db.get_config(&anchor_db_key(role))
        .as_deref()
        .and_then(parse_anchor)
}

/// Persist a [`DisplayAnchor`] per assigned role from the wizard's current
/// display enumeration (PinReady's own driver). The EDID fingerprint is filled
/// in by correlating against the kernel DRM monitors; roles whose monitor has
/// no usable EDID keep `fingerprint: None` and rely on position alone.
pub fn persist_anchors(db: &Database, displays: &[DisplayInfo]) {
    let sdl: Vec<SdlDisplay> = displays.iter().map(to_sdl).collect();
    let correlation = display_id::correlate(&sdl, &display_id::read_drm_monitors());

    for &(role, _, _) in ROLE_KEYS {
        let Some(d) = displays.iter().find(|d| d.role == role) else {
            let _ = db.set_config(&anchor_db_key(role), "");
            continue;
        };
        let fingerprint = correlation
            .iter()
            .find(|(name, _)| *name == d.name)
            .map(|(_, id)| id.fingerprint.clone());
        let anchor = DisplayAnchor {
            x: d.x,
            y: d.y,
            width: d.width,
            height: d.height,
            fingerprint,
        };
        if let Err(e) = db.set_config(&anchor_db_key(role), &serialize_anchor(&anchor)) {
            log::warn!("could not persist display anchor for {role:?}: {e}");
        }
    }
}

fn to_sdl(d: &DisplayInfo) -> SdlDisplay {
    SdlDisplay {
        name: d.name.clone(),
        x: d.x,
        y: d.y,
        width: d.width,
        height: d.height,
        width_mm: d.width_mm,
        height_mm: d.height_mm,
    }
}

/// Enumerate the displays SDL reports under `driver`, by re-invoking ourselves
/// as `--enumerate-displays <driver>`. A subprocess keeps SDL's video driver
/// isolated from our own winit/Wayland stack; it inherits our session env
/// (`DISPLAY`/`WAYLAND_DISPLAY`/`XAUTHORITY`) so both drivers can connect.
pub fn enumerate_via_subprocess(driver: &str) -> Vec<SdlDisplay> {
    let Ok(exe) = std::env::current_exe() else {
        return Vec::new();
    };
    let out = match std::process::Command::new(exe)
        .arg("--enumerate-displays")
        .arg(driver)
        .output()
    {
        Ok(o) if o.status.success() => o.stdout,
        Ok(o) => {
            log::warn!(
                "--enumerate-displays {driver} exited {}: {}",
                o.status,
                String::from_utf8_lossy(&o.stderr).trim()
            );
            return Vec::new();
        }
        Err(e) => {
            log::warn!("could not spawn --enumerate-displays {driver}: {e}");
            return Vec::new();
        }
    };

    String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|line| {
            // name \t x \t y \t w \t h \t refresh \t wmm \t hmm
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() < 8 {
                return None;
            }
            Some(SdlDisplay {
                name: f[0].to_string(),
                x: f[1].parse().ok()?,
                y: f[2].parse().ok()?,
                width: f[3].parse().ok()?,
                height: f[4].parse().ok()?,
                width_mm: f[6].parse().ok()?,
                height_mm: f[7].parse().ok()?,
            })
        })
        .collect()
}

/// Roles that currently carry a non-empty `*Display=` in the config — i.e. the
/// screens VPX will actually try to open.
fn assigned_roles(config: &VpxConfig) -> Vec<(DisplayRole, &'static str, &'static str)> {
    ROLE_KEYS
        .iter()
        .copied()
        .filter(|&(_, section, key)| config.get(section, key).is_some_and(|v| !v.is_empty()))
        .collect()
}

/// Try to reconcile every assigned role's `*Display=` to `driver`'s SDL names.
///
/// Returns `true` and commits the rewrites only if **all** assigned roles
/// resolved; otherwise returns `false` and leaves `config` untouched (atomic).
fn reconcile_all(config: &mut VpxConfig, db: &Database, driver: &str) -> bool {
    let roles = assigned_roles(config);
    if roles.is_empty() {
        return false;
    }
    let sdl = enumerate_via_subprocess(driver);
    if sdl.is_empty() {
        return false; // could not enumerate — never wipe names
    }
    let correlation = display_id::correlate(&sdl, &display_id::read_drm_monitors());

    // Resolve everything first; only apply if all succeed.
    let mut resolved = Vec::new();
    for (role, section, key) in roles {
        let Some(anchor) = load_anchor(db, role) else {
            return false; // no anchor for an assigned role → can't safely switch
        };
        let Some(name) = display_id::resolve_anchor(&anchor, &sdl, &correlation) else {
            return false; // monitor gone under this driver → bail
        };
        resolved.push((section, key, name));
    }
    for (section, key, name) in resolved {
        config.set(section, key, &name);
    }
    true
}

/// Which assigned roles' monitors are **not** currently present, checked in an
/// anchor-aware way: a role resolves if its [`DisplayAnchor`] resolves against
/// the currently-connected displays (by layout position or EDID fingerprint) —
/// regardless of what name the ini currently holds.
///
/// This supersedes the old raw-name check at startup: after a launch under a
/// different driver the ini may hold e.g. x11 names while PinReady runs
/// Wayland, and the anchor still resolves, so we no longer re-open the wizard
/// spuriously on an X11↔Wayland transition. Roles without a stored anchor fall
/// back to the legacy name match. Returns the `*Display=` values that failed.
pub fn unresolvable_assigned_displays(
    config: &VpxConfig,
    db: &Database,
    connected: &[DisplayInfo],
) -> Vec<String> {
    let sdl: Vec<SdlDisplay> = connected.iter().map(to_sdl).collect();
    let correlation = display_id::correlate(&sdl, &display_id::read_drm_monitors());

    let mut unresolved = Vec::new();
    for &(role, section, key) in ROLE_KEYS {
        let Some(name) = config.get(section, key).filter(|v| !v.is_empty()) else {
            continue; // role not assigned
        };
        let ok = match load_anchor(db, role) {
            Some(anchor) => display_id::resolve_anchor(&anchor, &sdl, &correlation).is_some(),
            None => connected.iter().any(|d| d.name == name), // legacy fallback
        };
        if !ok {
            unresolved.push(name);
        }
    }
    unresolved
}

/// Pick the SDL video driver to launch VPX under, reconciling `*Display=` names
/// to it as a side effect.
///
/// Prefers the framerate-optimal driver ([`preferred_vpx_driver`]): native
/// Wayland when the compositor supports `wp_fifo_v1`, else XWayland. That
/// switch is taken only when every assigned screen re-resolves under it
/// (config rewritten in place); otherwise we fall back to the session's own
/// driver with the names left as the wizard wrote them.
///
/// [`preferred_vpx_driver`]: crate::wayland_caps::preferred_vpx_driver
pub fn choose_driver_and_reconcile(config: &mut VpxConfig, db: &Database) -> Option<&'static str> {
    let session = crate::session::detect();
    if let Some(preferred) = crate::wayland_caps::preferred_vpx_driver(session) {
        if reconcile_all(config, db, preferred) {
            log::info!("VPX display driver: {preferred} (screens reconciled)");
            return Some(preferred);
        }
        log::info!(
            "VPX display driver: falling back to session driver {:?} \
             (preferred {preferred} could not reconcile all screens)",
            session
        );
    }
    session
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_roundtrip_with_fingerprint() {
        let a = DisplayAnchor {
            x: 0,
            y: 0,
            width: 3840,
            height: 2160,
            fingerprint: Some("deadbeef".into()),
        };
        let parsed = parse_anchor(&serialize_anchor(&a)).unwrap();
        assert_eq!(parsed.x, 0);
        assert_eq!(parsed.width, 3840);
        assert_eq!(parsed.fingerprint.as_deref(), Some("deadbeef"));
    }

    #[test]
    fn anchor_roundtrip_without_fingerprint() {
        let a = DisplayAnchor {
            x: 3840,
            y: 0,
            width: 1920,
            height: 1080,
            fingerprint: None,
        };
        let parsed = parse_anchor(&serialize_anchor(&a)).unwrap();
        assert_eq!(parsed.x, 3840);
        assert_eq!(parsed.fingerprint, None);
    }

    #[test]
    fn parse_anchor_rejects_short() {
        assert!(parse_anchor("0,0,3840").is_none());
    }
}
