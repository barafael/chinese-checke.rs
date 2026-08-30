//! Players, pieces, and positions (chapters 6–7).
//!
//! The types here are deliberately narrow so that some of the specification's
//! claims hold *structurally* rather than by check. [`Player`] cannot be out of
//! range; [`Position`] cannot put two pieces in one hole, because it is a map
//! from holes to occupants; [`Move`] cannot compare equal on its route, because
//! the route is not part of its identity.

use crate::geometry::{Coord, Dir, all_holes, in_camp, on_board};

/// The holes of each camp, computed once on first use.
///
/// [`Player::start_camp`] and [`Position::has_won`] run on every committed
/// move and every law sample; deriving them from [`all_holes`] per call would
/// re-enumerate the bounding box each time.
fn camp_holes(camp: usize) -> &'static [Coord] {
    static CAMPS: std::sync::OnceLock<Vec<Vec<Coord>>> = std::sync::OnceLock::new();
    CAMPS
        .get_or_init(|| {
            (0..PLAYERS)
                .map(|i| {
                    all_holes()
                        .into_iter()
                        .filter(|c| in_camp(*c, i as u32))
                        .collect()
                })
                .collect()
        })
        .get(camp % PLAYERS)
        .expect("camp index is taken modulo PLAYERS")
}

/// Number of players, and equally the number of camps.
pub const PLAYERS: usize = 6;
/// Pieces per player, and equally the size of a camp.
pub const PIECES_PER_PLAYER: usize = 10;
/// Playable holes.
pub const HOLES: usize = 121;

/// A player index, structurally constrained to `0..6`.
///
/// Constructing one out of range is impossible, so no law needs to check it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Player(u8);

impl Player {
    pub const ALL: [Player; PLAYERS] = [
        Player(0),
        Player(1),
        Player(2),
        Player(3),
        Player(4),
        Player(5),
    ];

    /// Returns `None` if `i >= 6`.
    pub const fn new(i: u8) -> Option<Player> {
        if (i as usize) < PLAYERS {
            Some(Player(i))
        } else {
            None
        }
    }

    /// Wraps into range; useful for turn advancement.
    pub const fn wrapping(i: u8) -> Player {
        Player(i % PLAYERS as u8)
    }

    pub const fn index(self) -> u8 {
        self.0
    }

    /// The next player in turn order.
    pub const fn next(self) -> Player {
        Player((self.0 + 1) % PLAYERS as u8)
    }

    /// The player whose camp is this player's target: $i + 3 \bmod 6$.
    pub const fn opposite(self) -> Player {
        Player((self.0 + 3) % PLAYERS as u8)
    }

    /// Holes of this player's starting camp.
    pub fn start_camp(self) -> Vec<Coord> {
        camp_holes(self.index() as usize).to_vec()
    }

    /// Holes of this player's target camp, $O_i = C_{(i+3) \bmod 6}$.
    pub fn target_camp(self) -> Vec<Coord> {
        camp_holes(self.opposite().index() as usize).to_vec()
    }
}

/// An occupancy function $s : V \rightarrow P \cup \{\varnothing\}$.
///
/// Backed by a dense array indexed by hole, so "at most one piece per hole" is
/// structural rather than checked.
#[derive(Clone, PartialEq, Eq)]
pub struct Position {
    /// Indexed by position in [`all_holes`].
    occupants: Vec<Option<Player>>,
    holes: Vec<Coord>,
}

/// Compact, so law violations stay readable: only occupied holes are shown.
///
/// The derived formatting prints all 121 occupancy slots and all 121 hole
/// coordinates, which buries the diagnostic in a violation message.
impl core::fmt::Debug for Position {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Position[")?;
        let mut first = true;
        for (c, o) in self.holes.iter().zip(&self.occupants) {
            if let Some(p) = o {
                if !first {
                    write!(f, " ")?;
                }
                write!(f, "{},{}={}", c.q, c.r, p.index())?;
                first = false;
            }
        }
        write!(f, "]")
    }
}

impl Position {
    /// The empty position: every hole vacant.
    pub fn empty() -> Self {
        let holes = all_holes();
        Self {
            occupants: vec![None; holes.len()],
            holes,
        }
    }

    /// The initial position: player `i` fills camp $C_i$ (chapter 6).
    pub fn initial() -> Self {
        let mut p = Self::empty();
        for player in Player::ALL {
            for c in player.start_camp() {
                p.set(c, Some(player));
            }
        }
        p
    }

    fn index_of(&self, c: Coord) -> Option<usize> {
        self.holes.binary_search(&c).ok()
    }

    pub fn holes(&self) -> &[Coord] {
        &self.holes
    }

    pub fn occupant(&self, c: Coord) -> Option<Player> {
        self.index_of(c).and_then(|i| self.occupants[i])
    }

    pub fn is_empty_hole(&self, c: Coord) -> bool {
        self.index_of(c)
            .is_some_and(|i| self.occupants[i].is_none())
    }

    /// Panics if `c` is not a board hole — an off-board write is a bug, not a
    /// recoverable condition.
    pub fn set(&mut self, c: Coord, occupant: Option<Player>) {
        let i = self
            .index_of(c)
            .unwrap_or_else(|| panic!("{c:?} is not a board hole"));
        self.occupants[i] = occupant;
    }

    pub fn pieces_of(&self, player: Player) -> Vec<Coord> {
        self.holes
            .iter()
            .zip(&self.occupants)
            .filter_map(|(c, o)| (*o == Some(player)).then_some(*c))
            .collect()
    }

    pub fn count_of(&self, player: Player) -> usize {
        self.occupants
            .iter()
            .filter(|o| **o == Some(player))
            .count()
    }

    pub fn occupied_count(&self) -> usize {
        self.occupants.iter().filter(|o| o.is_some()).count()
    }

    pub fn empty_count(&self) -> usize {
        self.occupants.iter().filter(|o| o.is_none()).count()
    }

    /// Occupied holes other than `exclude` — the set $\Omega$ of chapter 9.
    pub fn occupied_except(&self, exclude: Coord) -> Vec<Coord> {
        self.holes
            .iter()
            .zip(&self.occupants)
            .filter_map(|(c, o)| (o.is_some() && *c != exclude).then_some(*c))
            .collect()
    }

    /// Has `player` occupied every hole of their target camp? (chapter 13)
    pub fn has_won(&self, player: Player) -> bool {
        camp_holes(player.opposite().index() as usize)
            .iter()
            .all(|&c| self.occupant(c) == Some(player))
    }
}

/// Whether a move is a single step or a jump turn (chapter 10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MoveKind {
    Step,
    Jump,
}

/// A move, identified by `(kind, origin, destination)`.
///
/// `route` is presentational only. It is excluded from [`PartialEq`] and
/// [`core::hash::Hash`], which makes chapter 10's identity rule structural: two
/// routes to the same hole *cannot* compare unequal.
#[derive(Debug, Clone)]
pub struct Move {
    pub kind: MoveKind,
    pub origin: Coord,
    pub destination: Coord,
    pub route: Option<Vec<Coord>>,
}

impl Move {
    pub fn step(origin: Coord, destination: Coord) -> Self {
        Self {
            kind: MoveKind::Step,
            origin,
            destination,
            route: None,
        }
    }

    pub fn jump(origin: Coord, destination: Coord) -> Self {
        Self {
            kind: MoveKind::Jump,
            origin,
            destination,
            route: None,
        }
    }

    pub fn with_route(mut self, route: Vec<Coord>) -> Self {
        self.route = Some(route);
        self
    }

    /// The identity of this move: route deliberately excluded.
    pub fn key(&self) -> (MoveKind, Coord, Coord) {
        (self.kind, self.origin, self.destination)
    }
}

impl PartialEq for Move {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl Eq for Move {}

impl core::hash::Hash for Move {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.key().hash(state);
    }
}

/// Is `origin -> to` a legal adjacent step for `player`? (chapter 7)
pub fn is_legal_step(pos: &Position, player: Player, origin: Coord, to: Coord) -> bool {
    pos.occupant(origin) == Some(player)
        && on_board(to)
        && Dir::ALL.iter().any(|d| origin.neighbour(*d) == to)
        && pos.is_empty_hole(to)
}

/// Is a single jump from `origin` in direction `d` legal? (chapter 8)
pub fn is_legal_jump(pos: &Position, player: Player, origin: Coord, d: Dir) -> bool {
    let mid = origin.neighbour(d);
    let dest = origin.jump_dest(d);
    pos.occupant(origin) == Some(player)
        && on_board(mid)
        && on_board(dest)
        && !pos.is_empty_hole(mid)
        && pos.is_empty_hole(dest)
}
