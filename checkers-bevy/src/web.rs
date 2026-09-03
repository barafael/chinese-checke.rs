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
    room_from_fragment(&read_fragment()?)
}

/// Parse the room id out of the URL *fragment* (the part after `#`), given
/// already with or without its leading `#`. Kept pure and separate from
/// [`room_from_url`] so the parsing is unit-testable without a window.
///
/// A share link is written as `#room=name`, so the sender can append extra
/// params with `&` without breaking the lookup. The browser hands us the
/// fragment *including* the `#`, which `strip_prefix` would otherwise reject.
fn room_from_fragment(fragment: &str) -> Option<RoomId> {
    let fragment = fragment.strip_prefix('#').unwrap_or(fragment);
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

/// Suppress the browser's right-click context menu so the 3D camera's
/// right-drag orbit is not interrupted by a menu popping up. No-op off-web.
pub fn prevent_context_menu() {
    #[cfg(target_family = "wasm")]
    if let Some(window) = web_sys::window() {
        let callback = js_sys::Function::new_no_args("event.preventDefault();");
        let _ = window.add_event_listener_with_callback("contextmenu", &callback);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The browser reports `window.location.hash` *with* its leading `#`;
    /// that is exactly what `room_from_url` passes in, so the parser must cope.
    #[test]
    fn parses_a_share_link_fragment_with_leading_hash() {
        let room = room_from_fragment("#room=myroom").expect("a valid share link must parse");
        assert_eq!(room.0, "myroom");
    }

    #[test]
    fn tolerates_a_fragment_without_the_hash() {
        let room = room_from_fragment("room=myroom").expect("a raw fragment must parse");
        assert_eq!(room.0, "myroom");
    }

    #[test]
    fn finds_room_among_extra_params() {
        let room = room_from_fragment("#other=x&room=friend&r=2").unwrap();
        assert_eq!(room.0, "friend");
    }

    #[test]
    fn default_is_no_room_when_none_is_named() {
        assert_eq!(room_from_fragment(""), None);
        assert_eq!(room_from_fragment("#spectate=1"), None);
        assert_eq!(room_from_fragment("#room="), None); // empty value is invalid
        assert_eq!(room_from_fragment("#room=has space!"), None); // invalid chars
    }
}
