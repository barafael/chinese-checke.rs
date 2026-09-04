//! Wire protocol and host-relay sequencing for networked play.
//!
//! Peer-to-peer over WebRTC via `bevy_matchbox`; the signaling server is used
//! only for introductions. One peer acts as **host**, the single sequencing
//! authority:
//!
//! 1. A guest submits its move as [`NetMsg::Move`], to the host only.
//! 2. The host assigns the next sequence number and rebroadcasts
//!    [`NetMsg::Sequenced`] to everyone, *including itself*.
//! 3. Every peer applies a move only on `Sequenced` — the host included, or
//!    it would run a move ahead of its guests.
//!
//! [`WireMove`] carries only `(kind, origin, destination)`, never a resulting
//! position. Receivers re-derive the move against their own
//! [`checkers_core::rules::legal_moves`]: the host orders moves; the rules,
//! not the sender, decide legality.

use bevy::prelude::*;
use bevy_matchbox::prelude::*;
use checkers_core::geometry::Coord;
use checkers_core::position::{Move, MoveKind, Player};
use serde::{Deserialize, Serialize};

/// Signaling server used to introduce peers, set at compile time with
/// `MATCHBOX_SERVER`.
///
/// The default is omdurman's shared deployment — its room namespace is not
/// ours, so pick a room ([`RoomId::DEFAULT`]) and point `MATCHBOX_SERVER` at
/// your own `matchbox_server` for anything beyond local testing. Only
/// peer-introduction traffic crosses it; game moves are peer-to-peer.
pub const SIGNALING_SERVER: &str = match option_env!("MATCHBOX_SERVER") {
    Some(s) => s,
    None => "wss://omdurman-matchbox.fly.dev",
};

/// Reliable, ordered channel. Everything here is game-mutating, so there is no
/// unreliable channel — unlike omdurman, this game has no cursors to stream.
pub const CH_RELIABLE: usize = 0;

/// A move as it travels on the wire.
///
/// Deliberately *not* [`Move`]: that type carries a presentational `route` and
/// is not serialisable. Sending only the identity triple `(kind, origin,
/// destination)` matches chapter 10's rule that a move *is* its endpoints, so
/// two routes to the same hole cannot arrive as different moves.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct WireMove {
    pub origin: (i32, i32),
    pub destination: (i32, i32),
    pub jump: bool,
}

impl WireMove {
    pub fn from_move(mv: &Move) -> Self {
        Self {
            origin: (mv.origin.q, mv.origin.r),
            destination: (mv.destination.q, mv.destination.r),
            jump: mv.kind == MoveKind::Jump,
        }
    }

    pub fn origin_coord(&self) -> Coord {
        Coord::new(self.origin.0, self.origin.1)
    }

    pub fn destination_coord(&self) -> Coord {
        Coord::new(self.destination.0, self.destination.1)
    }

    /// Find this move among `legal`, or `None` if no legal move matches.
    ///
    /// The only way a wire move becomes a [`Move`]: an unmatched triple is
    /// dropped rather than constructed, so a malicious or out-of-sync peer
    /// cannot push the game off the rules.
    pub fn resolve(&self, legal: &[Move]) -> Option<Move> {
        let kind = if self.jump {
            MoveKind::Jump
        } else {
            MoveKind::Step
        };
        legal
            .iter()
            .find(|m| {
                m.kind == kind
                    && m.origin == self.origin_coord()
                    && m.destination == self.destination_coord()
            })
            .cloned()
    }
}

/// Lobby-visible player slot.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Seat {
    /// Stable within a session; `PeerId`'s string form so it survives postcard.
    pub peer: String,
    pub name: String,
    /// Which of the six players this peer commands, once the host assigns it.
    /// `None` for spectators, and for peers beyond the seating's camps.
    pub player: Option<u32>,
    pub ready: bool,
    /// Spectators watch the game and touch nothing: no camp, no ready flag,
    /// no voice in when it starts. A seat that joined after the camps ran out
    /// is also a spectator in effect, but this field is the *declared* choice.
    #[serde(default)]
    pub spectate: bool,
    /// A seat the host gave to an engine rather than a peer. It joins, takes a
    /// camp, and reads as ready like anyone else; only the host's engine
    /// actually plays it, and its moves reach the table as ordinary sequenced
    /// moves — no privileged path.
    #[serde(default)]
    pub engine: bool,
}

/// Top-level wire envelope.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum NetMsg {
    /// Guest -> host: an unsequenced submission. Never applied directly.
    Move(WireMove),
    /// Host -> all: the canonical ordered form. The only one that is applied.
    Sequenced { seq: u32, mv: WireMove },
    /// Introduce yourself on join.
    Hello { name: String },
    /// Host -> all: the full lobby roster, whenever it changes.
    Roster(Vec<Seat>),
    /// Guest -> host: toggle my ready flag.
    Ready(bool),
    /// Guest -> host: declare or renounce spectator status.
    Spectate(bool),
    /// Host -> all: the seating the host has chosen, live — guests see the
    /// table change as it is changed, not only when the game starts. Indices,
    /// for the same reason as on [`NetMsg::Start`].
    Seating(Vec<u32>),
    /// Host -> all: assignments are final, start playing.
    ///
    /// `players` carries which of the six players are seated, so every peer
    /// deals the same board. Indices, not a front-end type: the wire format
    /// must not depend on a crate above it.
    Start { seats: Vec<Seat>, players: Vec<u32> },
}

pub fn encode(msg: &NetMsg) -> Option<Box<[u8]>> {
    match postcard::to_allocvec(msg) {
        // A WebRTC data channel can silently drop a zero-byte payload:
        // `try_send` returns Ok, but the receiver never fires. Any real NetMsg
        // encodes to at least a variant tag, so this only happens on encode
        // failure — skip the send rather than put an invisible packet on the wire.
        Ok(v) if !v.is_empty() => Some(v.into_boxed_slice()),
        Ok(_) => {
            error!("postcard produced an empty NetMsg encoding; dropping");
            None
        }
        Err(error) => {
            error!(%error, "postcard encode failed");
            None
        }
    }
}

pub fn decode(raw: &[u8]) -> Option<NetMsg> {
    postcard::from_bytes(raw)
        .inspect_err(|error| warn!(%error, "NetMsg decode failed"))
        .ok()
}

/// Everything the front-end needs to know about the connection.
#[derive(Resource, Default)]
pub struct NetState {
    pub peers: Vec<PeerId>,
    pub my_id: Option<PeerId>,
    pub is_host: bool,
    pub seats: Vec<Seat>,
    /// Host-only: next sequence number to assign.
    pub next_seq: u32,
    /// Highest applied sequence number, for dropping duplicate deliveries.
    pub last_applied_seq: Option<u32>,
    pub name: String,
    /// Human-readable explanation of the last refused action, shown in the
    /// lobby. Empty when there is nothing to explain.
    pub status: String,
    /// Peers we have already sent our [`NetMsg::Hello`] to. Without this a
    /// per-frame greet loop would flood the channel; disconnected peers may
    /// stay listed here — a reconnect arrives with a fresh id anyway.
    pub greeted: Vec<PeerId>,
}

impl NetState {
    /// Forget everything tied to the old room, keeping only this peer's name.
    ///
    /// Every other field is per-room and wrong the moment the room changes.
    /// The name survives: it identifies the player, not the session.
    pub fn leave_room(&mut self) {
        let name = std::mem::take(&mut self.name);
        *self = Self {
            name,
            ..Self::default()
        };
    }

    /// Am I the sequencing authority? True for the host, and for a solo peer so
    /// that a single player can start before anyone else joins.
    pub fn sequences(&self) -> bool {
        self.is_host || self.peers.is_empty()
    }

    /// Release every seat's camp binding, for offline play: hotseat and solo
    /// drive all camps from one device, so no seat may pin the local player to
    /// a single camp left over from lobby greetings.
    pub fn unbind_players(&mut self) {
        for seat in &mut self.seats {
            seat.player = None;
        }
    }

    /// This peer's roster entry, if it has one.
    pub fn my_seat(&self) -> Option<&Seat> {
        let me = self.my_id?.to_string();
        self.seats.iter().find(|s| s.peer == me)
    }

    /// Which [`Player`] this peer commands, if the host has assigned one.
    /// Spectators — and any seat beyond the seating — command nothing.
    pub fn my_player(&self) -> Option<Player> {
        self.my_seat()?
            .player
            .and_then(|i| Player::ALL.get(i as usize).copied())
    }

    /// Has this sequence number already been applied? The reliable channel is
    /// ordered and `seq` is monotonic, so anything at or below the high-water
    /// mark is a duplicate.
    pub fn is_duplicate(&self, seq: u32) -> bool {
        self.last_applied_seq.is_some_and(|last| seq <= last)
    }
}

/// Why a room name was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomIdError {
    Empty,
    TooLong {
        len: usize,
    },
    /// The offending character, so the message can name it rather than saying
    /// "invalid".
    BadChar(char),
}

impl core::fmt::Display for RoomIdError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RoomIdError::Empty => write!(f, "a room name cannot be empty"),
            RoomIdError::TooLong { len } => write!(
                f,
                "a room name may be at most {} characters, got {len}",
                RoomId::MAX_LEN
            ),
            RoomIdError::BadChar(c) => write!(
                f,
                "'{c}' is not allowed in a room name; use letters, digits, '-' or '_'"
            ),
        }
    }
}

impl core::error::Error for RoomIdError {}

/// The room to join. Peers sharing a room id find each other.
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub struct RoomId(pub String);

impl Default for RoomId {
    fn default() -> Self {
        // Namespaced, because the default signaling server is shared with other
        // projects. The bare name `checkers` collided with live sessions there:
        // two unrelated peers joined, this build lost the host election, and it
        // sat in the lobby forever waiting for a `Start` that those peers — a
        // different game entirely — were never going to send.
        Self(Self::DEFAULT.into())
    }
}

impl RoomId {
    pub const DEFAULT: &'static str = "chinese-checkers-rs-v1";

    /// Long enough for a descriptive name, short enough to type and to read
    /// back to someone over the phone.
    pub const MAX_LEN: usize = 40;

    /// Validate a room name typed by the player.
    ///
    /// The name is interpolated into the signaling URL's path, so it is
    /// restricted to characters that need no escaping: letters, digits, `-` and
    /// `_`. That rules out `/`, which would otherwise let a typo silently
    /// redirect the socket to a different path, and whitespace, which is
    /// invisible in a name two people are trying to match.
    ///
    /// ASCII-only, deliberately: the point of a room name here is that two
    /// people can agree on it out of band and type it identically, and
    /// non-ASCII invites homoglyph and normalisation mismatches that look like
    /// the network being broken.
    pub fn parse(name: &str) -> Result<Self, RoomIdError> {
        if name.is_empty() {
            return Err(RoomIdError::Empty);
        }
        if name.chars().count() > Self::MAX_LEN {
            return Err(RoomIdError::TooLong {
                len: name.chars().count(),
            });
        }
        if let Some(c) = name
            .chars()
            .find(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_'))
        {
            return Err(RoomIdError::BadChar(c));
        }
        Ok(Self(name.to_string()))
    }
}

pub fn open_socket(mut commands: Commands, room: Res<RoomId>) {
    let url = format!("{SIGNALING_SERVER}/{}", room.0);
    info!(%url, "opening matchbox socket");
    commands.insert_resource(MatchboxSocket::from(
        WebRtcSocketBuilder::new(url)
            .reconnect_attempts(None)
            .add_reliable_channel(),
    ));
}

/// Send to one peer on the reliable channel.
pub fn send_to(socket: &mut MatchboxSocket, peer: PeerId, msg: &NetMsg) {
    let Some(bytes) = encode(msg) else {
        return;
    };
    if let Err(error) = socket.channel_mut(CH_RELIABLE).try_send(bytes, peer) {
        warn!(%error, "send failed");
    }
}

/// Send to every connected peer.
pub fn broadcast(socket: &mut MatchboxSocket, peers: &[PeerId], msg: &NetMsg) {
    let Some(bytes) = encode(msg) else {
        return;
    };
    for &peer in peers {
        if let Err(error) = socket
            .channel_mut(CH_RELIABLE)
            .try_send(bytes.clone(), peer)
        {
            warn!(%error, %peer, "send failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use checkers_core::rules::legal_moves;

    #[test]
    fn a_wire_move_round_trips_through_the_legal_move_list() {
        let pos = checkers_core::position::Position::initial();
        let legal = legal_moves(&pos, Player::ALL[0]);
        let mv = legal.first().expect("the initial position has moves");

        let wire = WireMove::from_move(mv);
        let resolved = wire.resolve(&legal).expect("its own move must resolve");
        assert_eq!(resolved.origin, mv.origin);
        assert_eq!(resolved.destination, mv.destination);
        assert_eq!(resolved.kind, mv.kind);
    }

    #[test]
    fn an_illegal_wire_move_does_not_resolve() {
        let pos = checkers_core::position::Position::initial();
        let legal = legal_moves(&pos, Player::ALL[0]);

        // Structurally well-formed, but not a legal move in this position.
        let bogus = WireMove {
            origin: (0, 0),
            destination: (0, 1),
            jump: false,
        };
        assert!(
            bogus.resolve(&legal).is_none(),
            "an unmatched triple must be dropped, not constructed"
        );
    }

    #[test]
    fn a_step_and_a_jump_to_the_same_hole_are_distinct_on_the_wire() {
        let a = WireMove {
            origin: (0, 0),
            destination: (2, 0),
            jump: true,
        };
        let b = WireMove { jump: false, ..a };
        assert_ne!(a, b);
    }

    #[test]
    fn encode_decode_round_trips() {
        let msg = NetMsg::Sequenced {
            seq: 7,
            mv: WireMove {
                origin: (1, -2),
                destination: (3, -4),
                jump: true,
            },
        };
        let bytes = encode(&msg).expect("encodes");
        let Some(NetMsg::Sequenced { seq, mv }) = decode(&bytes) else {
            panic!("decoded to the wrong variant");
        };
        assert_eq!(seq, 7);
        assert_eq!(mv.origin, (1, -2));
        assert!(mv.jump);
    }

    #[test]
    fn decoding_garbage_yields_none() {
        assert!(decode(&[0xff, 0xff, 0xff, 0xff]).is_none());
    }

    #[test]
    fn duplicate_sequence_numbers_are_recognised() {
        let mut net = NetState::default();
        assert!(!net.is_duplicate(0), "nothing applied yet");
        net.last_applied_seq = Some(3);
        assert!(net.is_duplicate(3), "the same seq is a duplicate");
        assert!(net.is_duplicate(2), "an older seq is a duplicate");
        assert!(!net.is_duplicate(4), "the next seq is new");
    }

    /// Only the no-peer arms; the `peers` branch needs a real `PeerId`, which
    /// requires a `uuid` dev-dependency for no gain — the guest-with-peers path
    /// is covered end-to-end by `sequencing_is_the_only_path_to_the_game` in
    /// `checkers-bevy/tests/net.rs`.
    #[test]
    fn a_solo_peer_sequences_whether_or_not_it_is_host() {
        let mut net = NetState::default();
        assert!(net.sequences(), "a lone guest must be able to start");
        net.is_host = true;
        assert!(net.sequences(), "and so must a lone host");
    }

    #[test]
    fn my_player_is_none_until_the_host_assigns_a_seat() {
        let net = NetState::default();
        assert!(net.my_player().is_none(), "no id and no seats yet");
    }
}

#[cfg(test)]
mod room_tests {
    use super::*;

    #[test]
    fn the_default_room_is_valid() {
        assert_eq!(
            RoomId::parse(RoomId::DEFAULT),
            Ok(RoomId(RoomId::DEFAULT.into())),
            "the default must survive its own validator"
        );
        assert_eq!(RoomId::default().0, RoomId::DEFAULT);
    }

    #[test]
    fn ordinary_names_are_accepted() {
        for name in ["a", "game", "Room_7", "my-game-2", "ABC123"] {
            assert!(RoomId::parse(name).is_ok(), "{name} should be accepted");
        }
    }

    #[test]
    fn an_empty_name_is_refused() {
        assert_eq!(RoomId::parse(""), Err(RoomIdError::Empty));
    }

    #[test]
    fn an_overlong_name_is_refused() {
        let long = "a".repeat(RoomId::MAX_LEN + 1);
        assert_eq!(
            RoomId::parse(&long),
            Err(RoomIdError::TooLong {
                len: RoomId::MAX_LEN + 1
            })
        );
        // The boundary itself is allowed.
        assert!(RoomId::parse(&"a".repeat(RoomId::MAX_LEN)).is_ok());
    }

    /// A slash would redirect the socket to a different URL path, and
    /// whitespace is invisible in a name two people are trying to match. Both
    /// must be refused rather than silently reinterpreted.
    #[test]
    fn characters_that_would_change_the_url_are_refused() {
        for (name, bad) in [
            ("a/b", '/'),
            ("a b", ' '),
            ("a?b", '?'),
            ("a#b", '#'),
            ("a:b", ':'),
            ("a.b", '.'),
            ("../evil", '.'),
        ] {
            assert_eq!(
                RoomId::parse(name),
                Err(RoomIdError::BadChar(bad)),
                "{name} must be refused"
            );
        }
    }

    /// Non-ASCII invites homoglyph and normalisation mismatches, which present
    /// as two peers in "the same" room never seeing each other.
    #[test]
    fn non_ascii_is_refused() {
        assert_eq!(RoomId::parse("café"), Err(RoomIdError::BadChar('é')));
        assert_eq!(RoomId::parse("рум"), Err(RoomIdError::BadChar('р')));
    }

    /// Every refusal must name the problem, since the message is shown to the
    /// player as the only explanation of why nothing happened.
    #[test]
    fn every_error_explains_itself() {
        assert!(RoomIdError::Empty.to_string().contains("empty"));

        let long = RoomIdError::TooLong { len: 99 }.to_string();
        assert!(long.contains("99"), "{long}");
        assert!(long.contains(&RoomId::MAX_LEN.to_string()), "{long}");

        let bad = RoomIdError::BadChar('/').to_string();
        assert!(bad.contains('/'), "must name the character: {bad}");
    }

    /// Leaving a room must not carry that room's identity into the next one.
    #[test]
    fn leaving_a_room_forgets_everything_but_the_name() {
        let mut net = NetState {
            is_host: true,
            next_seq: 12,
            last_applied_seq: Some(11),
            name: "ada".into(),
            status: "stale".into(),
            seats: vec![Seat {
                peer: "p".into(),
                name: "p".into(),
                player: Some(0),
                ready: true,
                spectate: false,
                engine: false,
            }],
            ..NetState::default()
        };

        net.leave_room();

        assert_eq!(net.name, "ada", "the player's name is not per-room");
        assert!(!net.is_host, "host status belongs to the old room");
        assert!(net.seats.is_empty(), "seats were assigned by the old host");
        assert!(net.peers.is_empty());
        assert_eq!(net.my_id, None, "the id came from the old socket");
        assert_eq!(net.next_seq, 0);
        assert_eq!(net.last_applied_seq, None, "or the first move looks stale");
        assert!(net.greeted.is_empty());
        assert!(net.status.is_empty());
    }
}
