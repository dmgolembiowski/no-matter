//! Render message bodies as GitHub-flavored markdown with syntax-
//! highlighted code blocks. Output is HTML, intended for `inner_html`
//! in `MessageRow`.
//!
//! Two safety / scope notes:
//!
//! 1. Raw HTML in the source is *dropped*, not passed through. We
//!    filter `Event::Html` and `Event::InlineHtml` so a user typing
//!    `<script>…</script>` in chat doesn't get to inject script tags
//!    into another user's DOM. Code blocks we generate ourselves are
//!    re-emitted as `Event::Html`, which is fine — that pathway is
//!    server-untrusted-input-free.
//!
//! 2. `syntect`'s syntax + theme sets are loaded once via `OnceLock`.
//!    They're a few hundred KB of bundled defaults but the load is
//!    one-time and synchronous; subsequent renders are cheap.

use std::sync::OnceLock;

use pulldown_cmark::{
    html, CodeBlockKind, CowStr, Event, Options, Parser, Tag, TagEnd,
};
use syntect::{
    easy::HighlightLines,
    highlighting::ThemeSet,
    html::{styled_line_to_highlighted_html, IncludeBackground},
    parsing::SyntaxSet,
    util::LinesWithEndings,
};

use crate::theme::Theme;

/// Render a message body to HTML. Cheap enough to call per-row on
/// every theme switch; a Memo on the call-site keeps it from running
/// on unrelated state changes.
pub fn render(body: &str, theme: Theme) -> String {
    let opts = Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS;
    let mut parser = Parser::new_ext(body, opts);

    let mut events: Vec<Event<'_>> = Vec::new();

    while let Some(ev) = parser.next() {
        match ev {
            // Strip user-supplied HTML — this is the XSS gate.
            Event::Html(_) | Event::InlineHtml(_) => continue,

            // Intercept fenced code blocks. We collect the contents,
            // hand them to syntect, and emit a single `Event::Html`
            // with the highlighted markup. Indented (4-space) code
            // blocks fall through to default rendering since they
            // don't carry a language hint.
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(lang))) => {
                let mut code = String::new();
                while let Some(next) = parser.next() {
                    match next {
                        Event::Text(t) => code.push_str(&t),
                        Event::End(TagEnd::CodeBlock) => break,
                        _ => {}
                    }
                }
                let highlighted = highlight(&code, lang.as_ref(), theme);
                events.push(Event::Html(CowStr::Boxed(highlighted.into_boxed_str())));
            }

            other => events.push(other),
        }
    }

    let mut out = String::new();
    html::push_html(&mut out, events.into_iter());
    out
}

fn highlight(code: &str, lang: &str, theme: Theme) -> String {
    let ss = syntax_set();
    let ts = theme_set();

    // pulldown-cmark passes the fence info as-is (e.g. "rust", "py",
    // "rs"). syntect indexes by token (the alias), then extension.
    let syntax = ss
        .find_syntax_by_token(lang)
        .or_else(|| ss.find_syntax_by_extension(lang))
        .unwrap_or_else(|| ss.find_syntax_plain_text());

    let theme_name = match theme {
        Theme::Dark => "base16-ocean.dark",
        Theme::Light => "InspiredGitHub",
    };
    let syn_theme = ts
        .themes
        .get(theme_name)
        .or_else(|| ts.themes.values().next())
        .expect("syntect ships with at least one default theme");

    let mut h = HighlightLines::new(syntax, syn_theme);
    let lang_class = if lang.is_empty() {
        String::from("hl")
    } else {
        format!("hl lang-{}", escape_class(lang))
    };
    let mut html = format!("<pre class=\"{lang_class}\"><code>");
    for line in LinesWithEndings::from(code) {
        let regions = match h.highlight_line(line, ss) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if let Ok(s) = styled_line_to_highlighted_html(&regions, IncludeBackground::No) {
            html.push_str(&s);
        }
    }
    html.push_str("</code></pre>");
    html
}

fn syntax_set() -> &'static SyntaxSet {
    static S: OnceLock<SyntaxSet> = OnceLock::new();
    S.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme_set() -> &'static ThemeSet {
    static T: OnceLock<ThemeSet> = OnceLock::new();
    T.get_or_init(ThemeSet::load_defaults)
}

/// Stick to identifier-safe characters in the language class; user-
/// supplied fence labels otherwise risk breaking out of the attribute.
fn escape_class(lang: &str) -> String {
    lang.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}
