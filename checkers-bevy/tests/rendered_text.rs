//! Every character the front-end draws must exist in the font it draws with.
//!
//! Bevy's default font covers ASCII and nothing else. A character outside it is
//! not an error and not a warning — it silently renders as a replacement box, so
//! the failure is invisible to every check except looking at the screen.
//!
//! That happened twice. The lobby's hint read `play solo now <box> all six
//! players` for as long as the lobby existed, because the source contained a
//! typographic em-dash. The second time, my own fixer missed it: the literal was
//! wrapped across two source lines, and the fixer skipped continuation lines.
//! Hence a test rather than a habit.
//!
//! This scans the source rather than the running app because there is no way to
//! ask Bevy which glyphs it failed to find; the text is drawn regardless.

use std::path::Path;

/// Characters allowed in a rendered string: printable ASCII, plus the
/// whitespace that legitimately reaches the screen.
///
/// Newlines and tabs are included because Bevy lays them out rather than
/// looking up a glyph — a multi-line status panel is one string with `\n` in it.
/// A raw newline also appears when `rustfmt` wraps a literal across source lines.
fn is_renderable(c: char) -> bool {
    c.is_ascii_graphic() || matches!(c, ' ' | '\n' | '\t' | '\r')
}

/// Pull out the contents of every double-quoted literal in `source`.
///
/// Deliberately *not* line-based: `rustfmt` wraps long literals across lines
/// with a trailing backslash, and a line-based scan skips the continuation —
/// which is exactly how the second em-dash survived the first fix. Comments are
/// stripped first, since prose in doc comments is read in an editor and may use
/// whatever typography it likes.
fn literals(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes: Vec<char> = source.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            // Skip line comments, including doc comments.
            '/' if i + 1 < bytes.len() && bytes[i + 1] == '/' => {
                while i < bytes.len() && bytes[i] != '\n' {
                    i += 1;
                }
            }
            // Skip block comments.
            '/' if i + 1 < bytes.len() && bytes[i + 1] == '*' => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == '*' && bytes[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
            }
            '"' => {
                i += 1;
                let mut lit = String::new();
                while i < bytes.len() && bytes[i] != '"' {
                    if bytes[i] == '\\' && i + 1 < bytes.len() {
                        // Keep the escape's payload out of the scan: `\n` is two
                        // source characters and neither is drawn.
                        i += 2;
                        continue;
                    }
                    lit.push(bytes[i]);
                    i += 1;
                }
                i += 1;
                out.push(lit);
            }
            _ => i += 1,
        }
    }
    out
}

/// The front-end sources whose string literals can reach the screen.
fn sources() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    for name in ["lobby.rs", "main.rs", "lib.rs", "net.rs", "setup.rs"] {
        let path = dir.join(name);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        out.push((name.to_string(), text));
    }
    out
}

#[test]
fn every_rendered_string_is_ascii() {
    let mut offenders = Vec::new();
    for (name, text) in sources() {
        for lit in literals(&text) {
            for c in lit.chars() {
                if !is_renderable(c) {
                    offenders.push(format!(
                        "{name}: U+{:04X} ({c:?}) in {:?}",
                        c as u32,
                        lit.chars().take(60).collect::<String>()
                    ));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "Bevy's default font renders these as empty boxes:\n  {}",
        offenders.join("\n  ")
    );
}

/// The scanner has to actually catch the thing it exists to catch. Without this,
/// a scanner that silently matched nothing would look like a clean codebase —
/// which is the failure mode that let the second em-dash through.
#[test]
fn the_scanner_finds_a_dash_in_a_wrapped_literal() {
    let wrapped = "        \"\\nPress S to play solo now \u{2014} all six players.\\n\\\n         Enter starts a shared game.\",";
    let found = literals(wrapped);
    assert_eq!(
        found.len(),
        1,
        "a wrapped literal is one literal: {found:?}"
    );
    assert!(
        found[0].contains('\u{2014}'),
        "the em-dash must be seen: {:?}",
        found[0]
    );
    assert!(
        found[0].chars().any(|c| !is_renderable(c)),
        "and must be rejected"
    );
}

/// Comments may use whatever typography they like, since nothing draws them.
#[test]
fn prose_in_comments_is_left_alone() {
    let source = "/// A doc comment \u{2014} with an em-dash.\nlet x = \"plain\";\n";
    assert_eq!(literals(source), vec!["plain".to_string()]);
}

/// Escapes are source syntax, not drawn characters.
#[test]
fn escapes_are_not_mistaken_for_content() {
    let source = r#"let x = "a\nb\tc\"d";"#;
    let found = literals(source);
    assert_eq!(found.len(), 1, "{found:?}");
    // The payloads of \n, \t and \" are skipped, leaving only the real letters.
    assert_eq!(found[0], "abcd");
    assert!(found[0].chars().all(is_renderable));
}
