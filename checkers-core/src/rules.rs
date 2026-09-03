//! Move generation and turn sequencing (chapters 9–15).

use std::collections::HashSet;

use crate::geometry::{Coord, Dir, on_board};
use crate::position::{Move, PLAYERS, Player, Position, is_legal_step};

/// Destinations reachable from `origin` by one or more jumps (chapter 9).
///
/// Exact and terminating. Within a turn only the moving piece moves and nothing
/// is captured, so the occupancy of every other hole — the set $\Omega$ — is
/// fixed. A jump out of a hole therefore depends only on that hole, and the
/// reachable set is the forward closure of a graph fixed once per turn. A single
/// visited set over **positions** suffices.
///
/// The origin is excluded from the result: a turn ending where it began is
/// indistinguishable from not moving.
pub fn jump_destinations(pos: &Position, origin: Coord) -> HashSet<Coord> {
    let omega: HashSet<Coord> = pos.occupied_except(origin).into_iter().collect();

    let mut visited = HashSet::from([origin]);
    let mut frontier = vec![origin];
    let mut reachable = HashSet::new();

    while !frontier.is_empty() {
        let mut next = Vec::new();
        for cur in frontier {
            for d in Dir::ALL {
                let mid = cur.neighbour(d);
                let dest = cur.jump_dest(d);
                if on_board(mid)
                    && on_board(dest)
                    && omega.contains(&mid)
                    && !omega.contains(&dest)
                    && visited.insert(dest)
                {
                    reachable.insert(dest);
                    next.push(dest);
                }
            }
        }
        frontier = next;
    }

    // The origin is seeded into `visited`, so the search can never offer it as a
    // destination: a turn ending where it began is indistinguishable from not
    // moving. No explicit removal is needed, and `CC-JUMP-REVISIT` checks it.
    debug_assert!(!reachable.contains(&origin));
    reachable
}

/// Enumerate jump *routes*, with a simple-path guard (chapter 9).
///
/// The guard is necessary, not cosmetic: a piece can hop out over a blocker and
/// straight back, so the space of routes is infinite. Forbidding repeats within
/// the current path bounds every route's length by the number of holes. This is
/// a presentational restriction and does not change the destination set.
pub fn jump_routes(pos: &Position, origin: Coord, max_hops: usize) -> Vec<Vec<Coord>> {
    let omega: HashSet<Coord> = pos.occupied_except(origin).into_iter().collect();
    let mut out = Vec::new();
    let mut path = vec![origin];
    walk_routes(&omega, origin, &mut path, max_hops, &mut out);
    out
}

fn walk_routes(
    omega: &HashSet<Coord>,
    cur: Coord,
    path: &mut Vec<Coord>,
    max_hops: usize,
    out: &mut Vec<Vec<Coord>>,
) {
    if path.len() > max_hops {
        return;
    }
    for d in Dir::ALL {
        let mid = cur.neighbour(d);
        let dest = cur.jump_dest(d);
        if on_board(mid)
            && on_board(dest)
            && omega.contains(&mid)
            && !omega.contains(&dest)
            && !path.contains(&dest)
        {
            path.push(dest);
            out.push(path.clone());
            walk_routes(omega, dest, path, max_hops, out);
            path.pop();
        }
    }
}

/// All legal moves for `player` (chapter 10).
///
/// One jump move per reachable destination, since distinct routes to the same
/// hole are the same move.
pub fn legal_moves(pos: &Position, player: Player) -> Vec<Move> {
    let mut moves = Vec::new();

    for origin in pos.pieces_of(player) {
        for d in Dir::ALL {
            let to = origin.neighbour(d);
            if on_board(to) && is_legal_step(pos, player, origin, to) {
                moves.push(Move::step(origin, to));
            }
        }
        for dest in jump_destinations(pos, origin) {
            moves.push(Move::jump(origin, dest));
        }
    }

    moves
}

/// Apply a move: vacate the origin, occupy the destination (chapter 11).
///
/// No intermediate hole is touched and nothing is captured.
pub fn apply(pos: &Position, mv: &Move) -> Position {
    let player = pos
        .occupant(mv.origin)
        .expect("move origin must hold a piece");
    let mut next = pos.clone();
    next.set(mv.origin, None);
    next.set(mv.destination, Some(player));
    next
}

/// Replay a move's route hole-by-hole. Used to check that route application and
/// the net effect agree (chapter 11).
pub fn apply_route(pos: &Position, mv: &Move) -> Position {
    let Some(route) = &mv.route else {
        return apply(pos, mv);
    };
    let player = pos
        .occupant(mv.origin)
        .expect("move origin must hold a piece");

    let mut next = pos.clone();
    let mut cur = mv.origin;
    for &hop in route.iter().skip(1) {
        next.set(cur, None);
        next.set(hop, Some(player));
        cur = hop;
    }
    next
}

/// How a game ended (chapter 12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Winner(Player),
    /// All six players passed in succession, so the position is frozen.
    Draw,
    /// The named player conceded. No chapter forces or forbids conceding —
    /// the rules govern play once a game is underway, not the decision to
    /// stop — so the engine offers it to the front-end instead of deriving
    /// it from the position.
    Resigned(Player),
}

/// A game in progress.
///
/// The active player is explicit state rather than a turn counter modulo six,
/// because passing advances the player without consuming a turn in that sense
/// (chapter 12).
///
/// The game also records **which players are in it**. Chapter 15 deliberately
/// leaves the player count open, and a front-end that seats fewer than six
/// peers composes a smaller game from these rules. Turn order still advances
/// cyclically through all six seats (chapter 12), skipping the empty ones;
/// with every player active the skip is a no-op and the game is exactly the
/// specification's. This matters for correctness, not convenience: unseated
/// players' camps start empty, so a two-player game has a reachable target
/// camp for each side, and the turn can never land on a player nobody controls.
#[derive(Debug, Clone)]
pub struct Game {
    position: Position,
    turn: Player,
    outcome: Option<Outcome>,
    consecutive_passes: u32,
    /// The players in this game, in turn order. Never empty.
    players: Vec<Player>,
}

impl Game {
    pub fn new() -> Self {
        Self::from_position(Position::initial(), Player::ALL[0])
    }

    /// The specification's game: all six players, whatever position is given.
    pub fn from_position(position: Position, turn: Player) -> Self {
        Self::compose(position, turn, &Player::ALL)
    }

    /// A variant game over `players` only (chapter 15 leaves the count open).
    ///
    /// Each listed player starts with their own camp filled and every other
    /// hole is empty — including the other camps, which is what keeps each
    /// player's target reachable. `players` is normalised to index order, so
    /// turn order follows the seats around the board regardless of how the
    /// caller lists them.
    pub fn for_players(players: &[Player]) -> Self {
        let mut players = players.to_vec();
        players.sort_by_key(|p| p.index());
        players.dedup();
        assert!(!players.is_empty(), "a game needs at least one player");

        let mut position = Position::empty();
        for &p in &players {
            for c in p.start_camp() {
                position.set(c, Some(p));
            }
        }
        Self::compose(position, players[0], &players)
    }

    /// A game over an explicit position, active player, and player set.
    ///
    /// `turn` must be one of `players`; a position whose pieces belong to
    /// nobody in the game is not rejected here — [`crate::audit::audit_position`]
    /// is what catches that, on the live board.
    pub fn compose(position: Position, turn: Player, players: &[Player]) -> Self {
        let mut players = players.to_vec();
        players.sort_by_key(|p| p.index());
        players.dedup();
        assert!(!players.is_empty(), "a game needs at least one player");
        assert!(
            players.contains(&turn),
            "the active player must be in the game"
        );

        Self {
            position,
            turn,
            outcome: None,
            consecutive_passes: 0,
            players,
        }
    }

    pub fn position(&self) -> &Position {
        &self.position
    }

    pub fn turn(&self) -> Player {
        self.turn
    }

    /// The players in this game, in turn order.
    pub fn players(&self) -> &[Player] {
        &self.players
    }

    pub fn outcome(&self) -> Option<Outcome> {
        self.outcome
    }

    pub fn is_over(&self) -> bool {
        self.outcome.is_some()
    }

    pub fn legal_moves(&self) -> Vec<Move> {
        legal_moves(&self.position, self.turn)
    }

    /// The next player in turn order who is actually in this game.
    ///
    /// Chapter 12's cycle is over all six seats; a composed game skips the
    /// vacant ones. With every seat filled this is exactly one advance.
    fn next_active(&self) -> Player {
        let mut next = self.turn.next();
        while !self.players.contains(&next) {
            next = next.next();
        }
        next
    }

    /// Play one move for the active player (chapter 12).
    pub fn play(&mut self, mv: &Move) {
        assert!(!self.is_over(), "the game is already over");
        debug_assert!(
            self.legal_moves().contains(mv),
            "illegal move {mv:?} for player {:?}",
            self.turn
        );

        self.position = apply(&self.position, mv);
        self.consecutive_passes = 0;

        if self.position.has_won(self.turn) {
            self.outcome = Some(Outcome::Winner(self.turn));
            return;
        }
        self.turn = self.next_active();
    }

    /// The active player has no legal move and forfeits the turn (chapter 12).
    ///
    /// This case is reachable, not impossible: a player whose ten pieces fill a
    /// camp can be sealed in.
    pub fn pass(&mut self) {
        assert!(!self.is_over(), "the game is already over");
        assert!(
            self.legal_moves().is_empty(),
            "player {:?} has legal moves and may not pass",
            self.turn
        );

        self.consecutive_passes += 1;
        if self.consecutive_passes as usize == self.players.len() {
            self.outcome = Some(Outcome::Draw);
            return;
        }
        self.turn = self.next_active();
    }

    /// The named player concedes, ending the game at once. The position is
    /// deliberately untouched — a resignation says nothing about the board.
    pub fn resign(&mut self, p: Player) {
        assert!(!self.is_over(), "the game is already over");
        assert!(
            self.players.contains(&p),
            "player {p:?} is not seated in this game"
        );
        self.outcome = Some(Outcome::Resigned(p));
    }

    /// Advance until the game ends or `max_plies` is exhausted.
    pub fn run<F>(&mut self, max_plies: usize, mut choose: F) -> Option<Outcome>
    where
        F: FnMut(&Position, Player, &[Move]) -> usize,
    {
        for _ in 0..max_plies {
            if self.is_over() {
                break;
            }
            let moves = self.legal_moves();
            if moves.is_empty() {
                self.pass();
                continue;
            }
            let idx = choose(&self.position, self.turn, &moves);
            let mv = moves[idx].clone();
            self.play(&mv);
        }
        self.outcome
    }
}

impl Default for Game {
    fn default() -> Self {
        Self::new()
    }
}

/// A position where player 0 holds all ten pieces yet has no legal move.
///
/// Demonstrates that "the active player always has a move" is false, so turn
/// handling must cope with passing (chapter 12).
pub fn blocked_position() -> Position {
    let mut pos = Position::empty();
    let player = Player::ALL[0];
    let blocker = Player::ALL[1];

    let camp = player.start_camp();
    for c in &camp {
        pos.set(*c, Some(player));
    }

    // Block the camp's frontier.
    let frontier: HashSet<Coord> = camp
        .iter()
        .flat_map(|c| Dir::ALL.map(|d| c.neighbour(d)))
        .filter(|c| on_board(*c) && !camp.contains(c))
        .collect();
    for c in &frontier {
        pos.set(*c, Some(blocker));
    }

    // Block every hole a camp piece could land on.
    let landings: HashSet<Coord> = camp
        .iter()
        .flat_map(|c| Dir::ALL.map(|d| (c.neighbour(d), c.jump_dest(d))))
        .filter(|(mid, dest)| on_board(*mid) && on_board(*dest) && !pos.is_empty_hole(*mid))
        .map(|(_, dest)| dest)
        .collect();
    for c in landings {
        if pos.is_empty_hole(c) {
            pos.set(c, Some(blocker));
        }
    }

    pos
}

/// A fully packed board: nobody can move at all.
pub fn frozen_position() -> Position {
    let mut pos = Position::empty();
    for (i, c) in pos.holes().to_vec().into_iter().enumerate() {
        pos.set(c, Some(Player::wrapping((i % PLAYERS) as u8)));
    }
    pos
}

/// A piece with two blockers in line, so single hops chain twice.
///
/// Exported next to [`blocked_position`] and [`frozen_position`] so tests in
/// other crates can share one definition of this fixture rather than each
/// rebuilding it.
pub fn two_hop_position() -> (Position, Coord) {
    let mut pos = Position::empty();
    let origin = Coord::new(0, 0);
    pos.set(origin, Some(Player::ALL[0]));
    pos.set(Coord::new(1, 0), Some(Player::ALL[1]));
    pos.set(Coord::new(3, 0), Some(Player::ALL[1]));
    (pos, origin)
}

#[cfg(test)]
mod variant_tests {
    use super::*;
    use crate::position::PIECES_PER_PLAYER;

    /// A front-end seats fewer than six peers and composes a smaller game.
    /// Turn order must skip the vacant seats — walking into one strands the
    /// game on a player nobody controls, which is how networked two-player
    /// play first deadlocked after one move each.
    #[test]
    fn turn_order_skips_vacant_seats() {
        let mut game = Game::for_players(&[Player::ALL[0], Player::ALL[3]]);
        assert_eq!(game.turn(), Player::ALL[0]);

        let mv = game.legal_moves().first().cloned().unwrap();
        game.play(&mv);
        assert_eq!(game.turn(), Player::ALL[3], "the vacant seats are skipped");

        let mv = game.legal_moves().first().cloned().unwrap();
        game.play(&mv);
        assert_eq!(game.turn(), Player::ALL[0], "two players alternate");
    }

    /// Every seat filled is exactly the specification's game: one advance per
    /// turn, unchanged.
    #[test]
    fn a_full_game_turns_as_before() {
        let mut game = Game::new();
        assert_eq!(game.players().len(), PLAYERS);
        let mv = game.legal_moves().first().cloned().unwrap();
        game.play(&mv);
        assert_eq!(game.turn(), Player::ALL[1]);
    }

    /// Unseated camps start empty, so each player's target camp is reachable
    /// and chapter 13's win can actually fire in a small game.
    #[test]
    fn unseated_camps_start_empty() {
        let game = Game::for_players(&[Player::ALL[0], Player::ALL[1]]);
        for p in [
            Player::ALL[2],
            Player::ALL[3],
            Player::ALL[4],
            Player::ALL[5],
        ] {
            assert_eq!(
                game.position().count_of(p),
                0,
                "player {} sits out",
                p.index()
            );
        }
        assert_eq!(game.position().count_of(Player::ALL[0]), PIECES_PER_PLAYER);
        assert_eq!(game.position().count_of(Player::ALL[1]), PIECES_PER_PLAYER);
    }

    /// Conceding ends the game immediately, whatever the board says — the
    /// outcome names the player who gave up.
    #[test]
    fn a_resignation_ends_the_game_for_the_resigned_seat() {
        let mut game = Game::for_players(&[Player::ALL[0], Player::ALL[3]]);
        game.resign(Player::ALL[3]);
        assert!(game.is_over());
        assert_eq!(game.outcome(), Some(Outcome::Resigned(Player::ALL[3])));
    }

    /// A resigned game accepts no further move.
    #[test]
    #[should_panic(expected = "already over")]
    fn a_resigned_game_takes_no_further_move() {
        let mut game = Game::for_players(&[Player::ALL[0], Player::ALL[3]]);
        game.resign(Player::ALL[3]);
        let mv = game.legal_moves().first().cloned().unwrap();
        game.play(&mv);
    }

    /// Only a seated player can resign; the vacant camps are not in the game.
    #[test]
    #[should_panic(expected = "not seated")]
    fn an_unseated_player_cannot_resign() {
        let mut game = Game::for_players(&[Player::ALL[0], Player::ALL[3]]);
        game.resign(Player::ALL[1]);
    }

    /// A finished game cannot end again.
    #[test]
    #[should_panic(expected = "already over")]
    fn a_finished_game_cannot_be_resigned() {
        let mut game = Game::for_players(&[Player::ALL[0], Player::ALL[3]]);
        game.resign(Player::ALL[0]);
        game.resign(Player::ALL[3]);
    }

    /// Chapter 12's draw is "all players pass in succession" — over the
    /// players actually in the game. On a full board, a two-player game ends
    /// after two passes, not six.
    #[test]
    fn all_active_players_passing_is_a_draw() {
        let mut game = Game::compose(
            frozen_position(),
            Player::ALL[0],
            &[Player::ALL[0], Player::ALL[1]],
        );
        game.pass();
        assert_eq!(game.outcome(), None, "one pass of two is not a draw yet");
        game.pass();
        assert_eq!(game.outcome(), Some(Outcome::Draw));
    }

    /// `for_players` normalises to index order, so turn order follows the
    /// board rather than the caller's listing.
    #[test]
    fn players_are_normalised_to_turn_order() {
        let game = Game::for_players(&[Player::ALL[4], Player::ALL[1], Player::ALL[2]]);
        assert_eq!(
            game.players(),
            &[Player::ALL[1], Player::ALL[2], Player::ALL[4]]
        );
        assert_eq!(game.turn(), Player::ALL[1]);
    }

    /// The composed game's position must survive the live audit the front-end
    /// runs after every move.
    #[test]
    fn composed_positions_pass_the_audit() {
        let game = Game::for_players(&[Player::ALL[0], Player::ALL[1], Player::ALL[2]]);
        crate::audit::audit_position(game.position(), game.players())
            .expect("a freshly composed game satisfies its own invariants");
    }
}
