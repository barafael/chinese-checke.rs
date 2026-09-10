//! Reading and writing the room name through the page URL, and the random
//! identities a session starts with.
//!
//! On the web the room travels in the URL fragment (`#room=name`), which is
//! what makes multiplayer shareable: send the link, and the recipient lands in
//! the lobby already pointed at your room. The fragment is used rather than a
//! query parameter because the browser never sends it to the server — the
//! GitHub Pages deployment has no server to care.
//!
//! A room is never typed: opening the bare page is redirected to a freshly
//! generated five-character room, and a room named in the URL is honoured as
//! it stands. Either way the address bar ends up shareable.
//!
//! Native builds have no URL; the room then comes from the `CCHKRS_ROOM`
//! environment variable, or is generated like on the web. The write side is a
//! no-op there, so callers stay `cfg`-free.

use checkers_net::RoomId;
use std::sync::atomic::{AtomicU64, Ordering};

/// The room named in the URL, if any, validated like a typed room name. An
/// invalid fragment is ignored rather than reported: a bad share link should
/// degrade to a fresh room, not to an error screen.
pub fn room_from_url() -> Option<RoomId> {
    #[cfg(target_family = "wasm")]
    {
        room_from_fragment(&read_fragment()?)
    }
    #[cfg(not(target_family = "wasm"))]
    {
        let name = std::env::var("CCHKRS_ROOM").ok()?;
        RoomId::parse(&name).ok()
    }
}

/// Parse the room id out of the URL *fragment* (the part after `#`), given
/// already with or without its leading `#`. Kept pure and separate from
/// [`room_from_url`] so the parsing is unit-testable without a window.
///
/// A share link is written as `#room=name`, so the sender can append extra
/// params with `&` without breaking the lookup. The browser hands us the
/// fragment *including* the `#`, which `strip_prefix` would otherwise reject.
#[cfg(any(target_family = "wasm", test))]
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
    #[cfg(target_family = "wasm")]
    write_fragment(&format!("room={}", room.0));
    #[cfg(not(target_family = "wasm"))]
    let _ = room;
}

/// The alphabet of generated rooms: unambiguous and lowercase, so a room read
/// aloud or copied by hand survives the trip.
const ROOM_ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";

/// A freshly generated room id: five characters, roughly 33 million rooms,
/// which is far more than a collision ever needs to be unlikely.
pub fn random_room() -> RoomId {
    let mut seed = fresh_seed();
    let mut id = String::new();
    for _ in 0..5 {
        let pick = (seed % ROOM_ALPHABET.len() as u64) as usize;
        seed /= ROOM_ALPHABET.len() as u64;
        id.push(ROOM_ALPHABET[pick] as char);
    }
    RoomId(id)
}

/// Short pet names a player is born with. One word, easy to say at the table;
/// the roster marks the player by it.
const PET_NAMES: &[&str] = &[
    "otter", "falcon", "maple", "ember", "comet", "panda", "lynx", "heron", "quail", "gecko",
    "koala", "raven", "tiger", "bison", "crane", "dingo", "eagle", "ibex", "orca", "yak", "hare",
    "moth", "wren", "toad", "newt", "elk", "fox", "owl", "bee", "finch", "mole", "starling",
    "puffin", "badger", "marten", "vole",
];

/// The pet name this session goes by. Drawn once per launch; the player cannot
/// and need not change it.
pub fn petname() -> String {
    PET_NAMES[(fresh_seed() % PET_NAMES.len() as u64) as usize].to_string()
}

/// A seed that differs between processes and between calls.
///
/// The two platforms need different sources, and the difference is the whole
/// point: std has **no randomness at all on `wasm32-unknown-unknown`** — its
/// `RandomState` degenerates to a fixed per-process counter, and
/// `performance.now()` is coarsened to ~100 us — so the seed below was
/// effectively constant and every fresh page drew the same pet name (and the
/// same room!). Native std seeds `RandomState` from the OS, so it stays as it
/// was.
#[cfg(target_family = "wasm")]
fn fresh_seed() -> u64 {
    // The browser's CSPRNG through `Math.random()`: 52 bits of entropy per
    // draw, and a fresh draw per call.
    (js_sys::Math::random() * (1u64 << 53) as f64) as u64
}

#[cfg(not(target_family = "wasm"))]
fn fresh_seed() -> u64 {
    use std::hash::{BuildHasher, Hash, Hasher};
    static CALLS: AtomicU64 = AtomicU64::new(0);
    static START: std::sync::OnceLock<bevy::platform::time::Instant> = std::sync::OnceLock::new();
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    CALLS.fetch_add(1, Ordering::Relaxed).hash(&mut hasher);
    START
        .get_or_init(bevy::platform::time::Instant::now)
        .elapsed()
        .as_nanos()
        .hash(&mut hasher);
    hasher.finish()
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

// --- Saved rounds (.cchkrs) -------------------------------------------------

/// The browser slot a saved round lives in. One save per page: enough to
/// close the tab after lunch and finish the game, and it cannot grow.
#[cfg(target_family = "wasm")]
const SAVE_SLOT: &str = "cchkrs.save";

/// Save a round's `.cchkrs` text. Native asks for a file with a dialog and
/// returns where it went, for the status line; the web writes the page's
/// single localStorage slot and says so.
#[cfg(not(target_family = "wasm"))]
pub fn save_record(text: &str) -> Result<String, String> {
    let path = rfd::FileDialog::new()
        .set_title("Save the game")
        .add_filter("Chinese Checkers record", &["cchkrs"])
        .set_file_name("game.cchkrs")
        .save_file()
        .ok_or("no file chosen")?;
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(format!(" to {}", path.display()))
}

#[cfg(target_family = "wasm")]
pub fn save_record(text: &str) -> Result<String, String> {
    let storage = web_sys::window()
        .and_then(|w| w.local_storage().ok())
        .flatten()
        .ok_or("the browser refuses local storage")?;
    storage
        .set_item(SAVE_SLOT, text)
        .map_err(|_| "the browser refused the save".to_string())?;
    Ok(" to this browser".to_string())
}

/// Load a round's `.cchkrs` text; the empty string faults name their source.
#[cfg(not(target_family = "wasm"))]
pub fn load_record() -> Result<String, String> {
    let path = rfd::FileDialog::new()
        .set_title("Open a saved game")
        .add_filter("Chinese Checkers record", &["cchkrs"])
        .pick_file()
        .ok_or("no file chosen")?;
    std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(target_family = "wasm")]
pub fn load_record() -> Result<String, String> {
    let storage = web_sys::window()
        .and_then(|w| w.local_storage().ok())
        .flatten()
        .ok_or("the browser refuses local storage")?;
    storage
        .get_item(SAVE_SLOT)
        .map_err(|_| "the browser refused the read".to_string())?
        .ok_or_else(|| "nothing has been saved in this browser yet".to_string())
}

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

    /// Generated rooms are five unambiguous characters, and consecutive draws
    /// from one process must differ more often than not — the counter in
    /// [`fresh_seed`] exists precisely so they do.
    #[test]
    fn generated_rooms_are_five_characters_and_vary() {
        let a = random_room();
        let b = random_room();
        assert_eq!(a.0.len(), 5);
        assert!(
            a.0.chars().all(|c| ROOM_ALPHABET.contains(&(c as u8))),
            "{}",
            a.0
        );
        assert_ne!(a, b, "two draws must not coincide");
    }

    /// A pet name is one word off the list; two launches need not differ (the
    /// list is short), but the name must always be one the roster can show.
    #[test]
    fn petnames_come_from_the_list() {
        let name = petname();
        assert!(PET_NAMES.contains(&name.as_str()), "{name}");
    }
}
