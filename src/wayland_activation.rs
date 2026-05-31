//! Request a fresh xdg-activation-v1 token from the Wayland compositor so
//! a freshly-spawned VPX process can grab focus (mutter/kwin/sway otherwise
//! apply focus-stealing prevention and the table opens behind PinReady).
//!
//! Why a separate Wayland connection: PinReady's main display is owned by
//! winit through the egui fork, and reaching back into its wl_surface /
//! wl_seat would require either an unsafe raw-handle dance or extending
//! the fork. Opening a parallel Wayland client connection is ~50 lines,
//! costs a single roundtrip (~1ms on a warm session), and the compositor
//! still issues a token to an unauthenticated request from the focused
//! app — every desktop-portal-style launch works this way.
//!
//! Returns None when:
//! - WAYLAND_DISPLAY is unset (X11 or headless session)
//! - The compositor doesn't expose xdg_activation_v1 (very old kwin, etc.)
//! - The roundtrip exceeds the timeout (compositor wedged)
//! - Any wayland protocol error

#![cfg(target_os = "linux")]

use std::sync::mpsc;
use std::time::Duration;

use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::wl_registry,
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols::xdg::activation::v1::client::{
    xdg_activation_token_v1::{self, XdgActivationTokenV1},
    xdg_activation_v1::XdgActivationV1,
};

struct State {
    token: Option<String>,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<XdgActivationV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &XdgActivationV1,
        _: <XdgActivationV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<XdgActivationTokenV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &XdgActivationTokenV1,
        event: xdg_activation_token_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_activation_token_v1::Event::Done { token } = event {
            state.token = Some(token);
        }
    }
}

/// Synchronous, bounded-time. Spawns a short-lived helper thread so a
/// stuck compositor can't block the launch path.
pub fn request_token() -> Option<String> {
    if std::env::var("WAYLAND_DISPLAY").is_err() {
        return None;
    }
    let (tx, rx) = mpsc::sync_channel::<Option<String>>(1);
    std::thread::spawn(move || {
        let _ = tx.send(request_token_inner());
    });
    match rx.recv_timeout(Duration::from_millis(500)) {
        Ok(token) => token,
        Err(_) => {
            log::warn!("xdg-activation token request timed out after 500ms");
            None
        }
    }
}

fn request_token_inner() -> Option<String> {
    let conn = match Connection::connect_to_env() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Wayland connect failed: {e}");
            return None;
        }
    };
    let (globals, mut queue) = match registry_queue_init::<State>(&conn) {
        Ok(r) => r,
        Err(e) => {
            log::warn!("Wayland registry init failed: {e}");
            return None;
        }
    };
    let qh = queue.handle();

    let activation: XdgActivationV1 = match globals.bind(&qh, 1..=1, ()) {
        Ok(a) => a,
        Err(e) => {
            log::warn!("Compositor does not expose xdg_activation_v1: {e}");
            return None;
        }
    };

    let token_obj = activation.get_activation_token(&qh, ());
    token_obj.set_app_id("pinready".to_string());
    token_obj.commit();

    let mut state = State { token: None };
    if let Err(e) = queue.roundtrip(&mut state) {
        log::warn!("xdg-activation roundtrip failed: {e}");
        return None;
    }
    state.token
}
