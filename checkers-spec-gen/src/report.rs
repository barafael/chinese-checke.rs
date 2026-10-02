//! The single-page HTML report of every law.
//!
//! Assembled from the same link-time registry the tests and the markdown
//! specification read: formula, precise statement, plain-language note, and
//! evidence level per law, grouped by chapter, with figures computed from
//! [`checkers_core::geometry`].

use checkers_core::law::{Evidence, LawInfo, all_in_reading_order, for_chapter};
use checkers_core::spec::Chapter;
use pulldown_cmark::{Options, Parser, html};

use crate::svg;

const KATEX_HEADER: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../assets/katex-header.html"
));

const STYLES: &str = r#"
:root {
  --ink: #263238; --muted: #607d8b; --line: #e0e0e0;
  --card: #ffffff; --bg: #fafafa; --accent: #1e88e5;
}
* { box-sizing: border-box; }
body {
  margin: 0; background: var(--bg); color: var(--ink);
  font: 16px/1.65 -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto,
        Helvetica, Arial, sans-serif;
}
main { max-width: 62rem; margin: 0 auto; padding: 2rem 1.25rem 4rem; }
a { color: var(--accent); }
code {
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 0.88em; background: #eceff1; padding: 0.1em 0.35em;
  border-radius: 4px;
}
pre { background: #263238; color: #eceff1; padding: 0.9rem 1.1rem;
      border-radius: 8px; overflow-x: auto; }
pre code { background: none; color: inherit; padding: 0; }
.site h1 { margin: 0 0 0.35rem; font-size: 1.9rem; }
.site .tagline { color: var(--muted); margin: 0 0 0.6rem; }
.site .crumbs { font-size: 0.9rem; margin-bottom: 1.2rem; }
.stats { display: flex; flex-wrap: wrap; gap: 0.5rem; margin: 0.8rem 0 0; }
.stat {
  background: var(--card); border: 1px solid var(--line);
  border-radius: 8px; padding: 0.45rem 0.8rem; font-size: 0.9rem;
}
.stat strong { font-size: 1.05rem; }
.card {
  background: var(--card); border: 1px solid var(--line);
  border-radius: 12px; padding: 1.2rem 1.4rem; margin: 1.4rem 0;
}
.toc ol { margin: 0.4rem 0 0; padding-left: 1.4rem; }
.toc li { margin: 0.15rem 0; }
.toc .count { color: var(--muted); font-size: 0.85rem; }
.chapter h2 { margin: 0 0 0.8rem; font-size: 1.45rem; }
.chapter h2 .num {
  display: inline-block; min-width: 1.9em; color: var(--muted);
  font-weight: 600;
}
.prose { color: #37474f; }
.prose p { margin: 0.6rem 0; }
.law-figure { margin: 1.2rem auto; text-align: center; }
.law-figure svg { max-width: 100%; height: auto; }
.law-figure figcaption, .law-figure figcaption em {
  color: var(--muted); font-size: 0.88rem; margin-top: 0.4rem;
}
.fig-row { display: flex; flex-wrap: wrap; gap: 1rem; justify-content: center; }
.fig-row figure { flex: 1 1 16rem; max-width: 24rem; margin: 0.8rem 0; }
.law {
  border: 1px solid var(--line); border-radius: 10px;
  padding: 0.9rem 1.1rem; margin: 0.9rem 0; background: #fcfcfc;
}
.law-head { margin: 0 0 0.3rem; display: flex; align-items: center;
            gap: 0.6rem; flex-wrap: wrap; }
.badge {
  font-size: 0.75rem; font-weight: 600; border-radius: 999px;
  padding: 0.12rem 0.6rem; border: 1px solid; white-space: nowrap;
}
.badge.proof { color: #2e7d32; background: #e8f5e9; border-color: #a5d6a7; }
.badge.exhaustive { color: #1565c0; background: #e3f2fd; border-color: #90caf9; }
.badge.property { color: #b26a00; background: #fff3e0; border-color: #ffcc80; }
.math { overflow-x: auto; margin: 0.5rem 0; }
.katex-display { margin: 0.4rem 0; }
.law p { margin: 0.45rem 0; }
.coverage table { border-collapse: collapse; margin: 0.6rem 0; }
.coverage th, .coverage td {
  border: 1px solid var(--line); padding: 0.3rem 0.8rem; text-align: left;
}
footer.site-footer {
  color: var(--muted); font-size: 0.85rem; margin-top: 2.5rem;
  border-top: 1px solid var(--line); padding-top: 1rem;
}
"#;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn markdown(src: &str) -> String {
    let mut out = String::new();
    html::push_html(&mut out, Parser::new_ext(src, Options::ENABLE_TABLES));
    out
}

fn badge(evidence: Evidence) -> String {
    format!(
        "<span class=\"badge {}\">{}</span>",
        match evidence {
            Evidence::Proof => "proof",
            Evidence::Exhaustive => "exhaustive",
            Evidence::Property => "property",
        },
        evidence.label()
    )
}

fn figure(caption: &str, svg: String) -> String {
    format!("<figure class=\"law-figure\">{svg}<figcaption>{caption}</figcaption></figure>")
}

fn figure_row(items: &[(&str, String)]) -> String {
    let inner: String = items
        .iter()
        .map(|(caption, svg)| figure(caption, svg.clone()))
        .collect();
    format!("<div class=\"fig-row\">{inner}</div>")
}

fn figures(chapter: Chapter) -> String {
    match chapter {
        Chapter::Coordinates => figure(
            "The six directions from a hole, in rotational order, each 60° from the last.",
            svg::directions(),
        ),
        Chapter::Hexagon => figure(
            "H₄ as concentric rings of radius 0–4: 1 + 6·(1+2+3+4) = 61 holes.",
            svg::hexagon(),
        ),
        Chapter::Camps => format!(
            "{}{}",
            figure(
                "Camp–hexagon contact: C₀'s four dark contact holes contribute the eight red adjacent pairs.",
                svg::contact_pairs(),
            ),
            figure_row(&[
                (
                    "C₀ outward — four contact holes, eight pairs. The real star.",
                    svg::proper_camp(),
                ),
                (
                    "C₀<sup>bad</sup> inward — one contact hole, one pair. Not a star.",
                    svg::inward_camp(),
                ),
            ]),
        ),
        Chapter::Rotation => figure(
            "R maps each camp onto the next; R³ maps each camp onto the one opposite, through the centre.",
            svg::rotation(),
        ),
        Chapter::Board => figure(
            "V as the disjoint union of H₄ and the six camps C₀…C₅: 61 + 6·10 = 121 holes.",
            svg::board_regions(),
        ),
        Chapter::Players => figure(
            "Player 0 starts with all ten pieces in C₀; the target is the opposite camp C₃.",
            svg::start_target(),
        ),
        Chapter::Steps => figure(
            "A step goes to an adjacent empty hole; an occupied neighbour is not a destination.",
            svg::steps(),
        ),
        Chapter::Jumps => figure(
            "A jump crosses the occupied midpoint x+d and lands on the empty hole x+2d = x+(x+2d). The crossed piece is not captured.",
            svg::jump(),
        ),
        Chapter::JumpSequences => figure(
            "One piece chaining two hops around blockers that never move: what is reachable depends only on the piece's position.",
            svg::route(),
        ),
        Chapter::Winning => figure(
            "A win: every hole of the opposite camp C₃ holds player 0's pieces.",
            svg::win(),
        ),
        _ => String::new(),
    }
}

fn render_law(law: &LawInfo) -> String {
    format!(
        "<article class=\"law\" id=\"{id}\">\n\
         <p class=\"law-head\"><code>{id}</code>{badge}</p>\n\
         <div class=\"math\">$${statement}$$</div>\n\
         <p><strong>Statement.</strong> {summary}</p>\n\
         <p><strong>In plain terms.</strong> {note}</p>\n\
         </article>\n",
        id = law.id,
        badge = badge(law.evidence),
        statement = esc(law.statement),
        summary = esc(law.summary),
        note = esc(law.note),
    )
}

fn render_chapter(chapter: Chapter) -> String {
    let laws = for_chapter(chapter);
    let law_list: String = laws.iter().map(|law| render_law(law)).collect();
    let laws_html = if laws.is_empty() {
        String::from("<p><em>No laws formalise this chapter yet.</em></p>\n")
    } else {
        format!("<h3>Laws</h3>\n{law_list}")
    };
    let figures = figures(chapter);
    format!(
        "<section class=\"chapter card\" id=\"{slug}\">\n\
         <h2><span class=\"num\">{n}</span>{title}</h2>\n\
         <div class=\"prose\">{prose}</div>\n\
         {figures}\n\
         {laws_html}\
         </section>\n",
        slug = chapter.slug(),
        n = chapter.number(),
        title = chapter.title(),
        prose = markdown(chapter.prose()),
    )
}

fn render_toc() -> String {
    let mut items = String::new();
    for chapter in Chapter::ALL {
        let n = for_chapter(chapter).len();
        let count = match n {
            0 => String::new(),
            1 => " — 1 law".into(),
            n => format!(" — {n} laws"),
        };
        items.push_str(&format!(
            "<li><a href=\"#{slug}\">{n}. {title}</a><span class=\"count\">{count}</span></li>\n",
            slug = chapter.slug(),
            n = chapter.number(),
            title = chapter.title(),
        ));
    }
    format!("<nav class=\"toc card\"><h2>Contents</h2><ol>\n{items}</ol></nav>\n")
}

/// How many laws rest on `evidence`.
fn count_with(laws: &[&LawInfo], evidence: Evidence) -> usize {
    laws.iter().filter(|l| l.evidence == evidence).count()
}

fn render_coverage() -> String {
    let laws = all_in_reading_order();
    let mut rows = String::new();
    for evidence in [Evidence::Proof, Evidence::Exhaustive, Evidence::Property] {
        let n = count_with(&laws, evidence);
        rows.push_str(&format!(
            "<tr><td>{}</td><td>{n}</td></tr>\n",
            evidence.label()
        ));
    }
    format!(
        "<section class=\"coverage card\"><h2>Coverage</h2>\n\
         <table><tr><th>Evidence</th><th>Laws</th></tr>\n\
         {rows}<tr><td><strong>total</strong></td><td><strong>{}</strong></td></tr></table>\n\
         <p>Every chapter's prose and every law come from <code>checkers-core</code>; \
         the page is generated, never edited.</p>\n\
         </section>\n",
        laws.len()
    )
}

/// The whole page.
pub fn render() -> String {
    let laws = all_in_reading_order();
    let proven = count_with(&laws, Evidence::Proof);
    let exhaustive = count_with(&laws, Evidence::Exhaustive);
    let property = count_with(&laws, Evidence::Property);

    let chapters: String = Chapter::ALL.iter().map(|&c| render_chapter(c)).collect();

    format!(
        "<!DOCTYPE html>\n\
         <html lang=\"en\">\n\
         <head>\n\
         <meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>Chinese Checkers — the laws</title>\n\
         {KATEX_HEADER}\
         <style>{STYLES}</style>\n\
         </head>\n\
         <body>\n\
         <main>\n\
         <header class=\"site\">\n\
         <h1>Chinese Checkers — the laws</h1>\n\
         <p class=\"tagline\">Every normative claim in <code>checkers-core</code>, as a \
         machine-checked law: formula, precise statement, plain-language meaning, and the \
         evidence that establishes it. Generated from the link-time law registry by \
         <code>checkers-spec-gen</code>.</p>\n\
         <p class=\"crumbs\"><a href=\"../\">← back to the game</a> · \
         <a href=\"https://github.com/barafael/chinese-checke.rs\">source</a></p>\n\
         <div class=\"stats\">\n\
         <span class=\"stat\"><strong>{}</strong> laws</span>\n\
         <span class=\"stat\"><strong>{proven}</strong> proven (Kani)</span>\n\
         <span class=\"stat\"><strong>{exhaustive}</strong> exhaustive</span>\n\
         <span class=\"stat\"><strong>{property}</strong> property-tested</span>\n\
         <span class=\"stat\"><strong>{}</strong> chapters</span>\n\
         </div>\n\
         </header>\n\
         {}\n{}\n{}\n\
         <footer class=\"site-footer\">Generated file — do not edit. \
         Regenerate with <code>cargo run -p checkers-spec-gen -- --html index.html</code>. \
         A <code>proof (Kani)</code> badge means bounded model checking established the law \
         over its whole domain; the others are checked exhaustively or by property test.</footer>\n\
         </main>\n\
         </body>\n\
         </html>\n",
        laws.len(),
        Chapter::ALL.len(),
        render_toc(),
        render_coverage(),
        chapters,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use checkers_core::law::all_sorted;

    #[test]
    fn the_page_lists_every_law_once_per_id() {
        let page = render();
        for law in all_sorted() {
            let occurrences = page.matches(&format!("id=\"{}\"", law.id)).count();
            assert_eq!(occurrences, 1, "{} appears {occurrences} time(s)", law.id);
            // The page holds the escaped text: a LaTeX `&` is `&amp;` there.
            assert!(
                page.contains(&format!("$${}", esc(law.statement))),
                "{} statement",
                law.id
            );
            assert!(page.contains(&esc(law.note)), "{} note", law.id);
        }
    }

    #[test]
    fn the_page_is_well_formed_enough() {
        let page = render();
        assert!(page.starts_with("<!DOCTYPE html>"));
        assert_eq!(
            page.matches("<article class=\"law\"").count(),
            all_sorted().len()
        );
        for tag in ["</html>", "</main>", "renderMathInElement"] {
            assert!(page.contains(tag), "missing {tag}");
        }
    }

    #[test]
    fn every_chapter_section_exists() {
        let page = render();
        for chapter in Chapter::ALL {
            assert!(
                page.contains(&format!(
                    "<section class=\"chapter card\" id=\"{}\">",
                    chapter.slug()
                )),
                "chapter {} missing",
                chapter.slug()
            );
        }
    }
}
