//! The specification's structure: chapters, their prose, and their order.
//!
//! This module is what makes the generated document a *specification* rather
//! than an alphabetical list. rustdoc sorts items alphabetically and offers no
//! stable way to change that (`--sort-modules-by-appearance` is nightly-only and
//! orders modules, not items), so reading order has to be data.
//!
//! Each [`Chapter`] carries its own title and prose. Laws name a chapter instead
//! of a free-text section string, which means:
//!
//! - a law cannot cite a section that does not exist — it is a type error;
//! - renumbering is impossible, because there are no numbers to get wrong;
//! - the generator can order and group laws deterministically.

use core::cmp::Ordering;

/// A chapter of the specification, in reading order.
///
/// The `repr(u8)` discriminants *are* the reading order: keep the declaration
/// ordered as the document should be read. Insert new chapters in position
/// rather than appending, since nothing external depends on the numeric values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Chapter {
    /// Coordinates, directions, adjacency.
    Coordinates,
    /// The central hexagon.
    Hexagon,
    /// The six triangular camps.
    Camps,
    /// Rotation and the opposite-camp relation.
    Rotation,
    /// The complete board and its construction invariants.
    Board,
    /// Players, pieces, and the initial position.
    Players,
    /// Adjacent moves.
    Steps,
    /// Single jumps.
    Jumps,
    /// Jump sequences and reachability.
    JumpSequences,
    /// Move representation and generation.
    MoveGeneration,
    /// Applying a move.
    Applying,
    /// Turn order, passing, and termination.
    Turns,
    /// The winning condition.
    Winning,
    /// Position invariants.
    Invariants,
    /// Rule variants left open by the specification.
    Variants,
}

impl Chapter {
    /// Every chapter, in reading order.
    pub const ALL: [Chapter; 15] = [
        Chapter::Coordinates,
        Chapter::Hexagon,
        Chapter::Camps,
        Chapter::Rotation,
        Chapter::Board,
        Chapter::Players,
        Chapter::Steps,
        Chapter::Jumps,
        Chapter::JumpSequences,
        Chapter::MoveGeneration,
        Chapter::Applying,
        Chapter::Turns,
        Chapter::Winning,
        Chapter::Invariants,
        Chapter::Variants,
    ];

    /// Position in the reading order, starting at 1.
    pub fn number(self) -> usize {
        Chapter::ALL
            .iter()
            .position(|c| *c == self)
            .expect("every chapter is in ALL")
            + 1
    }

    /// Short slug, for anchors and cross-references.
    pub const fn slug(self) -> &'static str {
        match self {
            Chapter::Coordinates => "coordinates",
            Chapter::Hexagon => "hexagon",
            Chapter::Camps => "camps",
            Chapter::Rotation => "rotation",
            Chapter::Board => "board",
            Chapter::Players => "players",
            Chapter::Steps => "steps",
            Chapter::Jumps => "jumps",
            Chapter::JumpSequences => "jump-sequences",
            Chapter::MoveGeneration => "move-generation",
            Chapter::Applying => "applying",
            Chapter::Turns => "turns",
            Chapter::Winning => "winning",
            Chapter::Invariants => "invariants",
            Chapter::Variants => "variants",
        }
    }

    pub const fn title(self) -> &'static str {
        match self {
            Chapter::Coordinates => "Coordinates and directions",
            Chapter::Hexagon => "The central hexagon",
            Chapter::Camps => "The six camps",
            Chapter::Rotation => "Rotation and opposite camps",
            Chapter::Board => "The complete board",
            Chapter::Players => "Players, pieces, and the initial position",
            Chapter::Steps => "Adjacent moves",
            Chapter::Jumps => "Jumps",
            Chapter::JumpSequences => "Jump sequences and reachability",
            Chapter::MoveGeneration => "Move representation and generation",
            Chapter::Applying => "Applying a move",
            Chapter::Turns => "Turn order, passing, and termination",
            Chapter::Winning => "The winning condition",
            Chapter::Invariants => "Position invariants",
            Chapter::Variants => "Rule variants",
        }
    }

    /// The chapter's normative prose.
    ///
    /// This is the text that was previously carried by the hand-written
    /// markdown. It lives here so it cannot drift from the laws that formalise
    /// it, and so the generator can emit a document in a deliberate order.
    pub const fn prose(self) -> &'static str {
        match self {
            Chapter::Coordinates => {
                "Every playable hole is identified by an axial hex coordinate \
                 $(q,r) \\in \\mathbb{Z}^2$, with the third cube coordinate \
                 implicit as $s = -q-r$.\n\n\
                 Six directions connect adjacent holes:\n\n\
                 $$D = \\{(1,0),\\ (1,-1),\\ (0,-1),\\ (-1,0),\\ (-1,1),\\ (0,1)\\}$$\n\n\
                 Two holes $u, v$ are adjacent exactly when $v - u \\in D$. The \
                 listing above is in rotational order, so consecutive directions \
                 are $60^\\circ$ apart. Only the set matters to the rules; the \
                 order is fixed so that direction indices are stable.\n\n\
                 In the implementation, directions are an enumeration rather than \
                 a collection of vectors, which makes \"there are exactly six \
                 directions\" a structural fact: no seventh direction is \
                 representable."
            }
            Chapter::Hexagon => {
                "The board's centre is a hexagon of radius four:\n\n\
                 $$H_4 = \\{(q,r) \\in \\mathbb{Z}^2 : |q| \\le 4,\\ |r| \\le 4,\\ |q+r| \\le 4\\}$$\n\n\
                 It contains $1 + 6(1+2+3+4) = 61$ holes.\n\n\
                 All three constraints are required. Dropping the bounds on $q$ \
                 and $r$ leaves $|q+r| \\le 4$, which describes an unbounded \
                 diagonal strip rather than a hexagon."
            }
            Chapter::Camps => {
                "Six triangular camps of ten holes each surround the hexagon. \
                 Camp $C_0$ is seated flush against the hexagon's $q = 4$ edge:\n\n\
                 $$C_0 = \\{(q,r) \\in \\mathbb{Z}^2 : 5 \\le q \\le 8,\\ -4 \\le r \\le -(q-4)\\}$$\n\n\
                 Its columns hold $4, 3, 2, 1$ holes, decreasing outward to a \
                 single apex at $(8,-4)$. The triangle therefore points **away** \
                 from the centre, and its four-hole base lies against the \
                 hexagon.\n\n\
                 The orientation is the whole content of this chapter, and it is \
                 easy to get wrong. The inward-pointing variant\n\n\
                 $$C_0^{\\text{bad}} = \\{(q,r) : 5 \\le q \\le 8,\\ -q+5 \\le r \\le 0\\}$$\n\n\
                 also has ten holes per camp and also yields a 121-hole board, so \
                 every cardinality check passes. But it meets the hexagon at the \
                 single hole $(5,0)$ instead of along a four-hole edge, leaving \
                 the six camps dangling from the hexagon's corners. It is not a \
                 six-pointed star.\n\n\
                 Cardinality alone cannot detect this. The distinguishing \
                 property is contact with the hexagon: a correct camp has four \
                 holes adjacent to $H_4$, contributing eight camp-to-hexagon \
                 adjacent pairs, whereas the degenerate one has a single contact \
                 hole and a single adjacent pair."
            }
            Chapter::Rotation => {
                "The remaining camps are rotations of $C_0$. A $60^\\circ$ \
                 rotation in axial coordinates is\n\n\
                 $$R(q,r) = (-r,\\ q+r)$$\n\n\
                 and $C_i = R^i(C_0)$ for $i = 0, \\ldots, 5$.\n\n\
                 $R$ has order six, and $R^3 = -\\mathrm{id}$. The second fact is \
                 the important one: it means the camp three positions away is the \
                 point reflection of the original,\n\n\
                 $$C_{(i+3) \\bmod 6} = -C_i = \\{-x : x \\in C_i\\}$$\n\n\
                 which is why a player's target is camp $i+3$ — it is \
                 geometrically opposite, directly across the centre.\n\n\
                 Whether $R$ reads as clockwise or counter-clockwise depends on \
                 the rendering convention and is not fixed here. Nothing in the \
                 rules depends on the choice, only on $R$ having order six and \
                 the camps being indexed consistently.\n\n\
                 Note that $R$ sends $(1,0) \\mapsto (0,1)$, stepping *backwards* \
                 through the direction order of chapter 1. That is harmless but \
                 worth knowing when comparing indices."
            }
            Chapter::Board => {
                "The board is the disjoint union of the hexagon and the six \
                 camps:\n\n\
                 $$V = H_4 \\mathbin{\\dot\\cup} C_0 \\mathbin{\\dot\\cup} \\cdots \\mathbin{\\dot\\cup} C_5,\\qquad |V| = 61 + 6 \\cdot 10 = 121$$\n\n\
                 A correct construction satisfies more than its cardinality. Each \
                 camp must meet the hexagon in exactly eight adjacent pairs; the \
                 board graph must be connected; and the whole board must be \
                 centrally symmetric, $V = -V$.\n\n\
                 A useful end-to-end check is that all six players have the same \
                 number of legal moves in the initial position — fourteen on the \
                 standard board. This follows from six-fold symmetry, so it fails \
                 loudly if a camp is misplaced.\n\n\
                 Rendering the construction is the fastest way to catch a \
                 mistake:\n\n\
                 ```text\n\
                 \x20              5\n\
                 \x20             5 5\n\
                 \x20            5 5 5\n\
                 \x20           5 5 5 5\n\
                 \x204 4 4 4 . . . . . 0 0 0 0\n\
                 \x20 4 4 4 . . . . . . 0 0 0\n\
                 \x20  4 4 . . . . . . . 0 0\n\
                 \x20   4 . . . . . . . . 0\n\
                 \x20    . . . . . . . . .\n\
                 \x20   3 . . . . . . . . 1\n\
                 \x20  3 3 . . . . . . . 1 1\n\
                 \x20 3 3 3 . . . . . . 1 1 1\n\
                 \x203 3 3 3 . . . . . 1 1 1 1\n\
                 \x20         2 2 2 2\n\
                 \x20          2 2 2\n\
                 \x20           2 2\n\
                 \x20            2\n\
                 ```\n\n\
                 Each row $r$ is drawn at horizontal offset $2q + r$. Opposite \
                 camps sit diametrically across the centre, as chapter 4 requires."
            }
            Chapter::Players => {
                "Six players each own ten indistinguishable pieces. A position is \
                 an occupancy function\n\n\
                 $$s : V \\rightarrow P \\cup \\{\\varnothing\\},\\qquad P = \\{0,\\ldots,5\\}$$\n\n\
                 with every hole holding at most one piece. Player $i$ starts with \
                 all ten pieces in camp $C_i$ and every other hole empty, so a \
                 valid position always has 60 occupied and 61 empty holes.\n\n\
                 Player $i$'s target is the opposite camp, $O_i = C_{(i+3) \\bmod 6}$."
            }
            Chapter::Steps => {
                "A turn moves exactly one piece belonging to the active player, \
                 and is either one adjacent step or a sequence of jumps — never a \
                 mixture.\n\n\
                 A piece at $x$ may step to $y$ exactly when\n\n\
                 $$s(x) = i \\ \\land\\ y - x \\in D \\ \\land\\ s(y) = \\varnothing$$\n\n\
                 that is, the destination is adjacent and empty. The piece vacates \
                 $x$ and occupies $y$; nothing else changes."
            }
            Chapter::Jumps => {
                "A piece at $x$ may jump in direction $d$ to $x + 2d$ exactly \
                 when the intervening hole is occupied and the landing hole is an \
                 empty board hole:\n\n\
                 $$x+d \\in V \\ \\land\\ s(x+d) \\neq \\varnothing \\ \\land\\ x+2d \\in V \\ \\land\\ s(x+2d) = \\varnothing$$\n\n\
                 The jumped piece is **never** captured or removed, and may belong \
                 to any player — legality depends only on the hole being occupied, \
                 not on who occupies it. The only occupancy changes are that $x$ \
                 becomes empty and $x+2d$ becomes the mover's.\n\n\
                 Since $d \\neq 0$, a jump displaces by $2d \\neq 0$ and so can \
                 never land on its own origin. The jumped hole is exactly the \
                 midpoint of the hop."
            }
            Chapter::JumpSequences => {
                "A turn may chain arbitrarily many jumps, all performed by the \
                 **same** piece, changing direction freely between hops. The \
                 player may stop after any jump; continuing is optional.\n\n\
                 Legality is evaluated against the position produced by the \
                 preceding jumps. That statement is true but routinely \
                 over-read, so it is worth being precise about what does and does \
                 not depend on the evolving position.\n\n\
                 Because a turn moves only one piece and jumps never capture, the \
                 occupancy of every *other* hole is fixed for the whole turn. \
                 Writing $\\Omega$ for the occupied holes excluding the moving \
                 piece, a jump from the piece's current hole $x$ is legal exactly \
                 when\n\n\
                 $$x+d \\in \\Omega \\ \\land\\ x+2d \\in V \\setminus \\Omega$$\n\n\
                 Only $x$, $d$, and the fixed set $\\Omega$ appear. Within a \
                 single turn, therefore, **occupancy is a function of the moving \
                 piece's position**, and the available jumps depend on that \
                 position alone. The moving piece can never block itself, since \
                 it is excluded from $\\Omega$.\n\n\
                 The consequence is practical: the set of reachable destinations \
                 is the forward closure of a directed graph fixed once per turn, \
                 so a breadth-first search over **positions**, with a single \
                 visited set, computes it exactly and always terminates.\n\n\
                 It is tempting to conclude instead that the search must be keyed \
                 on the pair (position, board state), on the grounds that the \
                 board changes after each hop. That is a mistake. Within a turn \
                 the state is determined by the position, so such a key can never \
                 distinguish two visits that the position alone would not — while \
                 making the search appear to need unbounded state. A search keyed \
                 that way does not terminate.\n\n\
                 Jump sequences genuinely may revisit holes, including the \
                 starting hole: with a blocker adjacent, a piece can hop out and \
                 straight back. The space of jump *paths* is therefore infinite, \
                 and any procedure enumerating paths needs an explicit guard — \
                 forbidding repeats within the current path is the natural choice. \
                 The space of *destinations* is finite regardless, which is all \
                 move generation requires. A turn is conventionally required to \
                 end somewhere other than where it began, since ending at the \
                 origin is indistinguishable from not moving."
            }
            Chapter::MoveGeneration => {
                "A move is identified by its kind, origin, and destination — not \
                 by the route taken. Distinct jump routes to the same hole produce \
                 the same resulting position, so they are the same move; counting \
                 them separately inflates move counts and any search built on \
                 them.\n\n\
                 A route may still be recorded for animation or notation, but it \
                 must be excluded from equality and hashing.\n\n\
                 The complete legal move set for player $i$ is the adjacent steps \
                 from each of their pieces, together with one jump move per \
                 reachable destination."
            }
            Chapter::Applying => {
                "Applying a move vacates the origin and occupies the destination. \
                 For a jump sequence, no intermediate hole is modified and nothing \
                 is captured, so\n\n\
                 $$s'(x) = \\varnothing,\\qquad s'(y) = i,\\qquad s'(z) = s(z)\\ \\ \\forall z \\notin \\{x,y\\}$$\n\n\
                 Replaying a route hole-by-hole and applying this net effect \
                 therefore agree, which is what justifies identifying moves by \
                 destination rather than by route."
            }
            Chapter::Turns => {
                "Turns proceed cyclically through the six players.\n\n\
                 The rules do **not** guarantee the active player has a legal \
                 move. The situation is reachable: if a player's ten pieces fill a \
                 camp, opponents can occupy that camp's frontier holes and the \
                 holes beyond them, leaving no step and no jump available. Such a \
                 player still holds all ten pieces, so this is neither a win nor a \
                 loss.\n\n\
                 This specification resolves it by **passing**: a player with no \
                 legal move forfeits the turn and play continues. If all six \
                 players pass in succession the position cannot change, and the \
                 game is a draw.\n\n\
                 One consequence is that the active player is not simply the turn \
                 number modulo six, since passing advances the player without \
                 consuming a turn in that sense. Implementations should track the \
                 active player as explicit state rather than deriving it.\n\n\
                 Rule sets differ here. Passing is the least intrusive convention; \
                 others forbid the blocking configuration outright, or oblige the \
                 blocking player to move aside."
            }
            Chapter::Winning => {
                "Player $i$ wins by occupying every hole of the opposite camp:\n\n\
                 $$\\forall x \\in C_{(i+3) \\bmod 6}:\\ s(x) = i$$\n\n\
                 Since the target camp has ten holes and the player has exactly \
                 ten pieces, this is equivalent to all of their pieces having \
                 arrived. The game ends at the first position satisfying this for \
                 some player."
            }
            Chapter::Invariants => {
                "Every completed move preserves the following. Each player owns \
                 exactly ten pieces; exactly 60 holes are occupied and 61 empty; \
                 every hole holds at most one piece. No move creates, destroys, or \
                 transfers ownership of a piece — jumping is not capture."
            }
            Chapter::Variants => {
                "Two points are deliberately left open, and an implementation must \
                 choose explicitly rather than letting the choice emerge from its \
                 geometry code.\n\n\
                 **Camp restrictions.** This specification uses the unrestricted \
                 convention: camp membership imposes no additional movement \
                 constraint, so a piece may enter, leave, or move within any camp \
                 subject only to the ordinary rules. Some rule sets restrict \
                 occupation of an opponent's camp. Such a restriction belongs in a \
                 separate legality predicate, not embedded in the movement rules.\n\n\
                 **Blocked players.** See chapter 12. This specification passes; \
                 other conventions exist."
            }
        }
    }
}

/// Order chapters by reading position rather than by name.
pub fn by_reading_order(a: Chapter, b: Chapter) -> Ordering {
    a.number().cmp(&b.number())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_contains_every_chapter_once() {
        let mut seen = Vec::new();
        for c in Chapter::ALL {
            assert!(!seen.contains(&c), "{c:?} listed twice in ALL");
            seen.push(c);
        }
        assert_eq!(seen.len(), Chapter::ALL.len());
    }

    #[test]
    fn numbering_is_dense_and_starts_at_one() {
        let numbers: Vec<usize> = Chapter::ALL.iter().map(|c| c.number()).collect();
        assert_eq!(numbers, (1..=Chapter::ALL.len()).collect::<Vec<_>>());
    }

    #[test]
    fn slugs_are_unique_and_url_safe() {
        let mut slugs: Vec<&str> = Chapter::ALL.iter().map(|c| c.slug()).collect();
        let total = slugs.len();
        slugs.sort_unstable();
        slugs.dedup();
        assert_eq!(slugs.len(), total, "duplicate slugs");

        for c in Chapter::ALL {
            let s = c.slug();
            assert!(
                s.chars().all(|ch| ch.is_ascii_lowercase() || ch == '-'),
                "{s} is not url-safe"
            );
        }
    }

    #[test]
    fn every_chapter_has_prose_and_title() {
        for c in Chapter::ALL {
            assert!(!c.title().is_empty(), "{c:?} has no title");
            assert!(
                c.prose().len() > 80,
                "{c:?} prose is suspiciously short ({} bytes)",
                c.prose().len()
            );
        }
    }

    /// The prose is embedded in generated markdown and rendered by KaTeX, so
    /// unbalanced math delimiters would corrupt both.
    #[test]
    fn prose_math_delimiters_are_balanced() {
        for c in Chapter::ALL {
            let display = c.prose().matches("$$").count();
            assert_eq!(
                display % 2,
                0,
                "{c:?} has an odd number of $$ delimiters ({display})"
            );
        }
    }

    #[test]
    fn prose_fences_are_balanced() {
        for c in Chapter::ALL {
            let fences = c.prose().matches("```").count();
            assert_eq!(fences % 2, 0, "{c:?} has an unclosed code fence");
        }
    }
}
