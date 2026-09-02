//! Reading and writing the room name through the page URL.
//!
//! On the web the room travels in the URL fragment (`#room=name`), which is
//! what makes multiplayer shareable: send the link, and the recipient lands in
//! the lobby already pointed at your room. The fragment is used rather than a
//! query parameter because the browser never sends it to the server — the
//! GitHub Pages deployment has no server to care.
//!
//! Native builds have no URL; the functions are no-ops there, so callers stay
//! `cfg`-free.

use checkers_net::RoomId;

/// The room named in the URL, if any, validated like a typed room name. An
/// invalid fragment is ignored rather than reported: the lobby's own editor is
/// the place to complain about names, and a bad share link should degrade to
/// the default room, not to an error screen.
pub fn room_from_url() -> Option<RoomId> {
    let fragment = read_fragment()?;
    for pair in fragment.split(['&', ';']) {
        if let Some(value) = pair.strip_prefix("room=") {
            return RoomId::parse(value).ok();
        }
    }
    None
}

/// Publish the room in the URL so the address can be copied and shared.
pub fn share_room(room: &RoomId) {
    write_fragment(&format!("room={}", room.0));
}

#[cfg(target_family = "wasm")]
fn read_fragment() -> Option<String> {
    web_sys::window()?.location().hash().ok()
}

#[cfg(target_family = "wasm")]
fn write_fragment(fragment: &str) {
    if let Some(window) = web_sys::window()
        && let Err(e) = window.location().set_hash(fragment)
    {
        bevy::log::warn!("could not set the room in the URL: {e:?}");
    }
}

#[cfg(not(target_family = "wasm"))]
fn read_fragment() -> Option<String> {
    None
}

#[cfg(not(target_family = "wasm"))]
fn write_fragment(_fragment: &str) {}
