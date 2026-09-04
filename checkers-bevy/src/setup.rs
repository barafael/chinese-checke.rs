//! Game setup: how many players sit down, and where.
//!
//! This lives in the front-end rather than in `checkers-core` on purpose. The
//! rules crate *is* the specification: six players, `Player::ALL` filling every
//! camp, `turn.next()` rotating through all six, and a draw declared after
//! `PLAYERS` consecutive passes. Forty-two laws and fourteen proofs are stated
//! against those facts. Making the player count a rules-level parameter would
//! reopen every one of them to support a menu.
//!
//! So a shorter game is expressed as a *position*, not as a different rulebook:
//! [`Seating::position`] fills only the seated players' camps and hands the
//! result to `Game::from_position`. Unseated players still take their turn in
//! rotation, have no pieces, no legal moves, and are passed over automatically.
//! The rules never learn that anyone is missing.
//!
//! # Which counts are offered
//!
//! The three classical seatings, each symmetric under the board's own rotation
//! so no player is advantaged by where they sit:
//!
//! - **2** — players 0 and 3, facing each other across the board.
//! - **3** — players 0, 2 and 4, alternating.
//! - **6** — everyone; the standard game, identical to `Position::initial`.
//!
//! Soundness is [`Seating::is_sound`]: nobody starts already won, and everybody
//! has a legal move. It is checked against the real rules, and a test enumerates
//! all 64 subsets of the six camps to confirm every offered seating passes.
//!
//! Note that soundness does **not** rule out any particular count — four and
//! five players are perfectly playable, and are left out only because no
//! rotationally symmetric arrangement of them exists. I initially believed the
//! criterion was "each seated player's opposite camp starts empty", which would
//! have made four and five impossible. That reasoning was wrong in both
//! directions: it rejects $\{0,3\}$, the standard two-player game, and admits
//! $\{0,1\}$. Occupancy of the target camp at setup says nothing about
//! reachability, because those pieces move out of the way.
//!
//! # Auditing a partial board
//!
//! [`checkers_core::audit::audit_position`] requires all six players to own ten
//! pieces, because law `CC-POS-PIECES` says
//! $\forall i \in P: |\{v : s(v) = i\}| = 10$ over all six. That law is the
//! specified game and is not weakened to accommodate a menu, so a partial
//! seating cannot use that audit.
//!
//! [`Seating::audit`] applies the same underlying invariant — piece
//! conservation, chapter 14 — restricted to the players who are actually
//! seated, and additionally requires every unseated player to own *nothing*. At
//! [`Seating::Six`] it is exactly the core audit, which a test asserts.

use std::fmt;

use checkers_core::position::{Player, Position};
use checkers_core::rules::{Game, legal_moves};

/// A partial board that does not conserve pieces.
///
/// A struct rather than a `String`, so the seated count and the offending player
/// survive to the caller instead of being flattened into prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeatingFault {
    /// A seated player does not own ten pieces.
    SeatedCount { player: u8, found: usize },
    /// An unseated player owns pieces, which means the seating leaked.
    UnseatedHasPieces { player: u8, found: usize },
}

impl fmt::Display for SeatingFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SeatingFault::SeatedCount { player, found } => write!(
                f,
                "seated player {player} owns {found} pieces, expected 10 \
                 (law CC-POS-PIECES, restricted to seated players)"
            ),
            SeatingFault::UnseatedHasPieces { player, found } => write!(
                f,
                "unseated player {player} owns {found} pieces, expected none"
            ),
        }
    }
}

impl core::error::Error for SeatingFault {}

/// How many players sit down. The board is always the full six-camp star; this
/// only decides which camps start filled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Seating {
    /// Players 0 and 3 — facing each other across the board.
    Two,
    /// Players 0, 2 and 4 — alternating, so no two are opposite.
    Three,
    /// All six.
    #[default]
    Six,
}

impl Seating {
    /// In the order the menu offers them.
    pub const ALL: [Seating; 3] = [Seating::Two, Seating::Three, Seating::Six];

    /// Which players are seated.
    ///
    /// Each set is closed under the board's rotational symmetry — every 3rd camp
    /// for two players, every 2nd for three — so no seat is positionally better
    /// than another.
    pub fn players(self) -> Vec<Player> {
        let indices: &[u8] = match self {
            Seating::Two => &[0, 3],
            Seating::Three => &[0, 2, 4],
            Seating::Six => &[0, 1, 2, 3, 4, 5],
        };
        indices
            .iter()
            .map(|&i| Player::new(i).expect("seating indices are below six"))
            .collect()
    }

    pub fn count(self) -> usize {
        self.players().len()
    }

    /// The seated player indices, for [`checkers_net::NetMsg::Start`].
    pub fn indices(self) -> Vec<u32> {
        self.players()
            .into_iter()
            .map(|p| u32::from(p.index()))
            .collect()
    }

    /// The seating a set of player indices names, if it is one this build knows.
    ///
    /// Order-insensitive and duplicate-tolerant, because it parses data from
    /// another peer rather than from this process. `None` for anything
    /// unrecognised — a peer running a build that offers a seating this one does
    /// not, or a corrupt message. The caller decides what to do about it; this
    /// does not guess, since guessing would deal a board that disagrees with the
    /// rest of the table, which is the exact bug the field was added to fix.
    pub fn from_indices(indices: &[u32]) -> Option<Seating> {
        let mut wanted: Vec<u32> = indices.to_vec();
        wanted.sort_unstable();
        wanted.dedup();
        Seating::ALL.into_iter().find(|s| s.indices() == wanted)
    }

    /// The label shown in the lobby.
    pub fn label(self) -> &'static str {
        match self {
            Seating::Two => "2 players",
            Seating::Three => "3 players",
            Seating::Six => "6 players",
        }
    }

    /// Piece conservation for this seating.
    ///
    /// The invariant of chapter 14 restricted to the seated players, because the
    /// core audit demands ten pieces for all six and a partial board has none
    /// for the empty camps. Weakening the law itself was not an option: it *is*
    /// the specification of the six-player game.
    pub fn audit(self, pos: &Position) -> Result<(), SeatingFault> {
        let seated = self.players();
        for player in Player::ALL {
            let found = pos.pieces_of(player).len();
            match (seated.contains(&player), found) {
                (true, 10) | (false, 0) => {}
                (true, found) => {
                    return Err(SeatingFault::SeatedCount {
                        player: player.index(),
                        found,
                    });
                }
                (false, found) => {
                    return Err(SeatingFault::UnseatedHasPieces {
                        player: player.index(),
                        found,
                    });
                }
            }
        }
        Ok(())
    }

    /// Cycle to the next seating, for the key that steps through them.
    pub fn next(self) -> Seating {
        let all = Seating::ALL;
        let i = all
            .iter()
            .position(|s| *s == self)
            .expect("every seating is in ALL");
        all[(i + 1) % all.len()]
    }

    /// The starting position: only the seated players' camps are filled.
    pub fn position(self) -> Position {
        let mut p = Position::empty();
        for player in self.players() {
            for &c in player.start_camp() {
                p.set(c, Some(player));
            }
        }
        p
    }

    /// A game starting from this seating, to move for the lowest seated player.
    ///
    /// Composed over *exactly* the seated players. `Game::from_position` would
    /// fill all six seats around a two-camp board: the turn would then visit
    /// four players nobody controls, kept moving only by the front-end's
    /// auto-pass, and a draw would need six consecutive passes instead of two.
    pub fn game(self) -> Game {
        let players = self.players();
        let first = *players.first().expect("a seating has at least one player");
        Game::compose(self.position(), first, &players)
    }

    /// Whether a set of seated players makes a playable game.
    ///
    /// Two conditions, both checked against the real rules rather than asserted
    /// from the geometry: nobody may start already having won, and everybody
    /// must have a legal move. A seating failing either shows up as a game that
    /// is over before it starts, or as a player who can never act.
    ///
    /// I first wrote this as "no seated player's opposite camp is also seated",
    /// reasoning that a pre-filled target camp is unreachable. That is wrong in
    /// both directions, and the enumeration test caught it: it rejects $\{0,3\}$
    /// — the standard two-player game, where the facing pieces simply move out
    /// of each other's way — while admitting $\{0,1\}$, which is playable but
    /// has no such symmetry. Occupancy of the target camp at *setup* says
    /// nothing about reachability, because those pieces move.
    pub fn is_sound(players: &[Player]) -> bool {
        if players.len() < 2 {
            return false;
        }
        let mut pos = Position::empty();
        for player in players {
            for &c in player.start_camp() {
                pos.set(c, Some(*player));
            }
        }
        players
            .iter()
            .all(|p| !pos.has_won(*p) && !legal_moves(&pos, *p).is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use checkers_core::audit::audit_position;

    /// Two-player mode used to compose the game over all six seats: a
    /// two-camp board with the turn cycling through four players nobody
    /// controls, kept moving only by the front-end's auto-pass.
    #[test]
    fn a_composed_game_cycles_only_its_seated_players() {
        let mut game = Seating::Two.game();
        assert_eq!(
            game.players().len(),
            2,
            "the game must know its real players"
        );

        let mv = game
            .legal_moves()
            .first()
            .cloned()
            .expect("a fresh camp has moves");
        game.play(&mv);
        assert_eq!(
            game.turn(),
            Player::new(3).expect("3 is a valid index"),
            "the turn must skip straight to the other seated player"
        );
        assert!(
            audit_position(game.position(), game.players()).is_ok(),
            "a composed game must pass the audit against its own players"
        );
    }

    #[test]
    fn each_seating_seats_the_number_it_claims() {
        assert_eq!(Seating::Two.count(), 2);
        assert_eq!(Seating::Three.count(), 3);
        assert_eq!(Seating::Six.count(), 6);
    }

    /// Ten pieces per seated player and none for anyone else.
    #[test]
    fn only_the_seated_players_have_pieces() {
        for seating in Seating::ALL {
            let pos = seating.position();
            let seated = seating.players();
            for player in Player::ALL {
                let n = pos.pieces_of(player).len();
                if seated.contains(&player) {
                    assert_eq!(n, 10, "{seating:?}: player {} seated", player.index());
                } else {
                    assert_eq!(n, 0, "{seating:?}: player {} not seated", player.index());
                }
            }
        }
    }

    /// Every offered seating must be sound, checked by enumerating all 64
    /// subsets of the six camps and confirming the offered ones appear among the
    /// sound set.
    ///
    /// Deliberately *not* asserting that the offered list is the whole sound set
    /// — it is not. Every subset of size two or more turns out to be playable,
    /// and the menu offers three of them because those are the rotationally
    /// symmetric ones. Pinning equality here would encode the false claim that
    /// four and five players are impossible.
    #[test]
    fn the_offered_seatings_are_among_the_sound_ones() {
        let sound: Vec<Vec<Player>> = (0u8..64)
            .filter_map(|mask| {
                let players: Vec<Player> = (0..6)
                    .filter(|i| mask & (1 << i) != 0)
                    .map(|i| Player::new(i).expect("below six"))
                    .collect();
                Seating::is_sound(&players).then_some(players)
            })
            .collect();

        for seating in Seating::ALL {
            assert!(
                sound.contains(&seating.players()),
                "{seating:?} is not among the sound seatings"
            );
        }

        // A single player is not a game, and the empty set is not either.
        assert!(!Seating::is_sound(&[]), "the empty seating is not a game");
        assert!(
            !Seating::is_sound(&[Player::new(0).expect("below six")]),
            "one player is not a game"
        );
    }

    /// Every offered seating must itself satisfy the condition.
    #[test]
    fn every_offered_seating_is_sound() {
        for seating in Seating::ALL {
            assert!(
                Seating::is_sound(&seating.players()),
                "{seating:?} is not a sound seating"
            );
        }
    }

    /// No seated player may start already having won, and every one of them must
    /// have somewhere to go. A seating that fails this looks like a hung game.
    #[test]
    fn no_seated_player_starts_won_or_stuck() {
        for seating in Seating::ALL {
            let pos = seating.position();
            for player in seating.players() {
                assert!(
                    !pos.has_won(player),
                    "{seating:?}: player {} starts already won",
                    player.index()
                );
                assert!(
                    !legal_moves(&pos, player).is_empty(),
                    "{seating:?}: player {} starts with no move",
                    player.index()
                );
            }
        }
    }

    /// The game must open with a seated player on the move, or the first thing
    /// the player sees is somebody else's turn being passed over.
    #[test]
    fn the_game_opens_on_a_seated_player() {
        for seating in Seating::ALL {
            let game = seating.game();
            assert!(
                seating.players().contains(&game.turn()),
                "{seating:?} opens on unseated player {}",
                game.turn().index()
            );
            assert!(
                !game.legal_moves().is_empty(),
                "{seating:?} opens with no legal move"
            );
        }
    }

    /// The starting position must satisfy the seating's own conservation check,
    /// since these positions are built here rather than by `Position::initial`.
    #[test]
    fn every_seating_passes_its_own_audit() {
        for seating in Seating::ALL {
            seating
                .audit(&seating.position())
                .unwrap_or_else(|f| panic!("{seating:?} violates conservation: {f}"));
        }
    }

    /// At six players the seating audit must agree with the specification's,
    /// which is what makes it a restriction of `CC-POS-PIECES` rather than a
    /// separate and weaker rule.
    #[test]
    fn at_six_players_the_audit_is_the_specifications() {
        let pos = Seating::Six.position();
        assert_eq!(pos, Position::initial(), "six players is the standard game");
        assert!(checkers_core::audit::audit_position(&pos, &Player::ALL).is_ok());
        assert_eq!(Seating::Six.audit(&pos), Ok(()));
    }

    /// The partial seatings must be *rejected* by the core audit. If they were
    /// not, the restricted audit would be pointless — and it would mean the
    /// six-player law had quietly stopped saying what it says.
    #[test]
    fn the_core_audit_rejects_a_partial_board() {
        for seating in [Seating::Two, Seating::Three] {
            assert!(
                checkers_core::audit::audit_position(&seating.position(), &Player::ALL).is_err(),
                "{seating:?} must not satisfy the six-player audit"
            );
        }
    }

    /// Conservation must be enforced, not merely declared: removing a seated
    /// player's piece has to be caught.
    #[test]
    fn a_removed_piece_is_caught() {
        for seating in Seating::ALL {
            let mut pos = seating.position();
            let victim = seating.players()[0];
            let hole = pos.pieces_of(victim)[0];
            pos.set(hole, None);
            assert_eq!(
                seating.audit(&pos),
                Err(SeatingFault::SeatedCount {
                    player: victim.index(),
                    found: 9,
                }),
                "{seating:?} must catch a missing piece"
            );
        }
    }

    /// A piece belonging to a player who is not seated must be caught too, since
    /// that is what a leaking seating would look like.
    #[test]
    fn a_piece_for_an_unseated_player_is_caught() {
        let seating = Seating::Two;
        let mut pos = seating.position();
        let intruder = Player::new(1).expect("below six");
        assert!(!seating.players().contains(&intruder));
        // An empty hole in an unseated camp.
        let hole = intruder.start_camp()[0];
        pos.set(hole, Some(intruder));
        assert_eq!(
            seating.audit(&pos),
            Err(SeatingFault::UnseatedHasPieces {
                player: 1,
                found: 1,
            })
        );
    }

    /// Both faults must name the player and the count, so a panic in the
    /// front-end is diagnosable from its message alone.
    #[test]
    fn faults_describe_themselves() {
        let seated = SeatingFault::SeatedCount {
            player: 3,
            found: 9,
        }
        .to_string();
        assert!(seated.contains("player 3"), "{seated}");
        assert!(seated.contains('9'), "{seated}");
        assert!(seated.contains("CC-POS-PIECES"), "{seated}");

        let unseated = SeatingFault::UnseatedHasPieces {
            player: 1,
            found: 2,
        }
        .to_string();
        assert!(unseated.contains("player 1"), "{unseated}");
        assert!(unseated.contains("unseated"), "{unseated}");
    }

    /// A partial seating must not be able to declare a spurious draw.
    ///
    /// `Game::pass` counts consecutive passes and calls a draw at six. Unseated
    /// players have no pieces, so they pass every rotation — four of them at two
    /// players. If a seated player were then also stuck, the count could reach
    /// six and end a live game as a draw. `play` resets the counter to zero, so
    /// this holds only as long as a move happens each rotation; the test pins
    /// that a long two-player game never ends in a draw.
    #[test]
    fn unseated_passes_do_not_force_a_draw() {
        let seating = Seating::Two;
        let mut game = seating.game();
        for ply in 0..60 {
            // Pass over anyone with nothing to do, exactly as the front-end does.
            while !game.is_over() && game.legal_moves().is_empty() {
                game.pass();
            }
            if game.is_over() {
                assert_ne!(
                    game.outcome(),
                    Some(checkers_core::rules::Outcome::Draw),
                    "spurious draw at ply {ply}: unseated passes reached the limit"
                );
                return;
            }
            let mv = game.legal_moves().swap_remove(0);
            game.play(&mv);
            seating
                .audit(game.position())
                .unwrap_or_else(|f| panic!("conservation broke at ply {ply}: {f}"));
        }
        assert!(!game.is_over(), "60 plies should not finish a game");
    }

    /// Every seating must survive the wire round-trip, or a guest deals a
    /// different board from the host — which is the bug the `players` field on
    /// `NetMsg::Start` exists to prevent.
    #[test]
    fn every_seating_round_trips_through_indices() {
        for seating in Seating::ALL {
            let indices = seating.indices();
            assert_eq!(
                Seating::from_indices(&indices),
                Some(seating),
                "{seating:?} did not survive {indices:?}"
            );
        }
    }

    /// The indices come from another peer, so parsing must not depend on their
    /// order or assume they are unique.
    #[test]
    fn parsing_indices_ignores_order_and_duplicates() {
        assert_eq!(Seating::from_indices(&[3, 0]), Some(Seating::Two));
        assert_eq!(Seating::from_indices(&[0, 3, 0, 3]), Some(Seating::Two));
        assert_eq!(Seating::from_indices(&[4, 0, 2]), Some(Seating::Three));
        assert_eq!(
            Seating::from_indices(&[5, 4, 3, 2, 1, 0]),
            Some(Seating::Six)
        );
    }

    /// An unrecognised set must be refused rather than rounded to something
    /// plausible. Dealing a *nearly* right board silently is worse than saying
    /// the seating is unknown, because both peers then disagree without knowing.
    #[test]
    fn an_unknown_seating_is_refused() {
        for bad in [
            vec![],
            vec![0],
            vec![0, 1],          // sound, but not offered
            vec![0, 1, 2, 3],    // four players
            vec![0, 1, 2, 3, 4], // five
            vec![9, 42],         // not player indices at all
        ] {
            assert_eq!(
                Seating::from_indices(&bad),
                None,
                "{bad:?} must not be accepted"
            );
        }
    }

    #[test]
    fn cycling_visits_every_seating_and_returns() {
        let mut seen = vec![Seating::default()];
        let mut s = Seating::default();
        for _ in 0..Seating::ALL.len() - 1 {
            s = s.next();
            assert!(!seen.contains(&s), "cycle repeated {s:?} early");
            seen.push(s);
        }
        assert_eq!(s.next(), Seating::default(), "cycling must wrap around");
        assert_eq!(seen.len(), Seating::ALL.len());
    }
}
