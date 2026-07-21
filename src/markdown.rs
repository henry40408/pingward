//! A deliberately minimal, **escape-first** markdown subset for project and
//! check descriptions.
//!
//! [`render`] HTML-escapes the *entire* input before any markdown transform
//! runs, then only emits a small whitelist of tags (`<p>`, `<br>`, `<ul>`,
//! `<li>`, `<strong>`, `<em>`, `<code>`, `<a>`) built from the already-escaped
//! text. This makes raw HTML injection structurally impossible: a `<script>`
//! or `<img onerror=...>` in the source has already become `&lt;script&gt;`
//! before any transform sees it, and no transform can turn escaped text back
//! into a live tag. Do not add a sanitizer pass instead — the whole point of
//! this module is that escaping happens first, not "generate HTML then clean
//! it up".
//!
//! Supported syntax: `**bold**`, `*italic*`, `` `code` ``, `[text](url)`
//! (only `http://`/`https://`/`mailto:` become links — anything else, e.g.
//! `javascript:`, renders as literal text), bare `http(s)://` autolinks, and
//! `- ` bullet lists (a block where every line starts with `- `). Deliberately
//! unsupported: headings, images, tables, blockquotes, code fences, reference
//! links, nested lists, and `_underscore_` emphasis — all of those render as
//! literal (escaped) text.
//!
//! Inline constructs also do **not nest**: each matcher copies its content
//! verbatim rather than re-entering [`render`]'s inline pass, so
//! `**[text](url)**` renders the link syntax literally inside the `<strong>`.
//! That is deliberate — not re-scanning generated output is what guarantees an
//! emitted tag can never be reinterpreted as new markup.
//!
//! **Complexity**: [`render`] is worst-case O(n²) — an input of repeated `[`
//! with no closing `]` makes `match_link`'s `find(']')` rescan to the end of
//! the string on every one-character fallback advance. This is only safe
//! because every caller enforces `web::MAX_DESCRIPTION_CHARS` before this
//! module ever sees the text (measured: ~2000 chars is sub-millisecond,
//! ~80000 chars is ~83ms). Raising that cap, or adding a new caller that
//! passes unvalidated/unbounded input to `render`, must account for this.

/// Render `src` (raw markdown from a user) to a whitelisted HTML subset.
pub fn render(src: &str) -> String {
    let normalized = normalize_newlines(src);
    let escaped = escape_html(&normalized);
    let mut out = String::new();
    for block in split_blocks(&escaped) {
        if is_list_block(&block) {
            out.push_str("<ul>");
            for line in &block {
                let trimmed = line.trim_start();
                let content = trimmed.strip_prefix("- ").unwrap_or(trimmed);
                out.push_str("<li>");
                out.push_str(&render_inline(content));
                out.push_str("</li>");
            }
            out.push_str("</ul>");
        } else {
            out.push_str("<p>");
            let rendered_lines: Vec<String> =
                block.iter().map(|line| render_inline(line)).collect();
            out.push_str(&rendered_lines.join("<br>"));
            out.push_str("</p>");
        }
    }
    out
}

/// Strip markdown markers and collapse all whitespace (including newlines)
/// into single spaces. Not HTML-escaped — callers pass this through Askama,
/// which escapes it on render.
pub fn to_plain(src: &str) -> String {
    let normalized = normalize_newlines(src);
    let stripped = strip_markdown_markers(&normalized);
    stripped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// [`to_plain`], truncated to at most `max_chars` **characters** (never bytes
/// — safe on multi-byte input), appending `…` when truncation happened.
pub fn truncate_plain(src: &str, max_chars: usize) -> String {
    let plain = to_plain(src);
    if plain.chars().count() <= max_chars {
        return plain;
    }
    let mut truncated: String = plain.chars().take(max_chars).collect();
    truncated.push('…');
    truncated
}

fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Split already-escaped text into blocks on blank lines. Each returned block
/// is a non-empty list of non-blank lines.
fn split_blocks(s: &str) -> Vec<Vec<&str>> {
    let mut blocks = Vec::new();
    let mut current = Vec::new();
    for line in s.split('\n') {
        if line.trim().is_empty() {
            if !current.is_empty() {
                blocks.push(std::mem::take(&mut current));
            }
        } else {
            current.push(line);
        }
    }
    if !current.is_empty() {
        blocks.push(current);
    }
    blocks
}

/// A block is a list block when every line, after trimming leading
/// whitespace, starts with a `- ` marker.
fn is_list_block(block: &[&str]) -> bool {
    block.iter().all(|line| line.trim_start().starts_with("- "))
}

/// Only these schemes ever become a live `<a href>`. Anything else — notably
/// `javascript:` and `data:` — must render as literal text.
fn is_allowed_scheme(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:")
}

/// Render one line/list item's inline markdown. This is a single left-to-right
/// pass over the (already HTML-escaped) text: at each position it tries the
/// constructs in priority order (code, bold, italic, link, autolink) and
/// falls back to copying one literal character. Because the source text is
/// scanned exactly once and generated output is never re-scanned, an emitted
/// `<a href="...">` can never be re-matched by a later autolink attempt.
fn render_inline(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while !rest.is_empty() {
        if rest.starts_with('`') {
            if let Some((consumed, rendered)) = match_code(rest) {
                out.push_str(&rendered);
                rest = &rest[consumed..];
                continue;
            }
        } else if rest.starts_with("**") {
            if let Some((consumed, rendered)) = match_bold(rest) {
                out.push_str(&rendered);
                rest = &rest[consumed..];
                continue;
            }
            // No closing `**`: emit the two literal asterisks and move on,
            // rather than falling through to `match_italic`, which would
            // greedily consume them as an empty `*...*` pair and emit a
            // stray `<em></em>`.
            out.push_str("**");
            rest = &rest[2..];
            continue;
        } else if rest.starts_with('*') {
            if let Some((consumed, rendered)) = match_italic(rest) {
                out.push_str(&rendered);
                rest = &rest[consumed..];
                continue;
            }
        } else if rest.starts_with('[') {
            if let Some((consumed, rendered)) = match_link(rest) {
                out.push_str(&rendered);
                rest = &rest[consumed..];
                continue;
            }
        } else if rest.starts_with("http://") || rest.starts_with("https://") {
            let (consumed, rendered) = match_autolink(rest);
            out.push_str(&rendered);
            rest = &rest[consumed..];
            continue;
        }
        let ch = rest.chars().next().expect("rest is non-empty");
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    out
}

/// `` `code` `` → `<code>code</code>`. `rest` must start with a backtick.
/// Content between the backticks is copied verbatim — no further transform
/// runs inside it. Returns `None` (unmatched trailing backtick) when there is
/// no closing backtick, leaving it to the caller's literal-character fallback.
fn match_code(rest: &str) -> Option<(usize, String)> {
    let after = &rest[1..];
    let p = after.find('`')?;
    let content = &after[..p];
    Some((1 + p + 1, format!("<code>{content}</code>")))
}

/// `**bold**` → `<strong>bold</strong>`. `rest` must start with `**`.
fn match_bold(rest: &str) -> Option<(usize, String)> {
    let after = &rest[2..];
    let p = after.find("**")?;
    let content = &after[..p];
    Some((2 + p + 2, format!("<strong>{content}</strong>")))
}

/// `*italic*` → `<em>italic</em>`. `rest` must start with `*`.
fn match_italic(rest: &str) -> Option<(usize, String)> {
    let after = &rest[1..];
    let p = after.find('*')?;
    let content = &after[..p];
    Some((1 + p + 1, format!("<em>{content}</em>")))
}

/// `[text](url)` → `<a href="url" rel="noopener noreferrer">text</a>`, or the
/// original literal text when `url`'s scheme isn't whitelisted. `rest` must
/// start with `[`.
fn match_link(rest: &str) -> Option<(usize, String)> {
    let after_bracket = &rest[1..];
    let close_idx = after_bracket.find(']')?;
    let link_text = &after_bracket[..close_idx];
    let after_close_bracket = &after_bracket[close_idx + 1..];
    let after_paren = after_close_bracket.strip_prefix('(')?;
    let paren_close = after_paren.find(')')?;
    let url = &after_paren[..paren_close];
    let consumed = 1 + close_idx + 1 + 1 + paren_close + 1;
    if is_allowed_scheme(url) {
        Some((
            consumed,
            format!("<a href=\"{url}\" rel=\"noopener noreferrer\">{link_text}</a>"),
        ))
    } else {
        Some((consumed, format!("[{link_text}]({url})")))
    }
}

/// A bare `http://`/`https://` run of non-whitespace becomes an autolink,
/// trimming trailing `.,;:!?)` off the URL back into literal trailing text.
/// `rest` must start with `http://` or `https://`; always consumes at least
/// one character, so the caller's scan always makes progress.
fn match_autolink(rest: &str) -> (usize, String) {
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let run = &rest[..end];
    let trim_chars = ['.', ',', ';', ':', '!', '?', ')'];
    let mut url_end = run.len();
    while url_end > 0 {
        let ch = run[..url_end]
            .chars()
            .next_back()
            .expect("url_end > 0 implies a preceding char");
        if trim_chars.contains(&ch) {
            url_end -= ch.len_utf8();
        } else {
            break;
        }
    }
    let url = &run[..url_end];
    let trailing = &run[url_end..];
    if url.starts_with("http://") || url.starts_with("https://") {
        (
            end,
            format!("<a href=\"{url}\" rel=\"noopener noreferrer\">{url}</a>{trailing}"),
        )
    } else {
        (end, run.to_string())
    }
}

/// Strip markdown markers for [`to_plain`]: leading `- ` list markers, code
/// backticks, `*`/`**` emphasis markers, and `[text](url)` → `text`.
fn strip_markdown_markers(s: &str) -> String {
    let delisted: String = s
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            trimmed.strip_prefix("- ").unwrap_or(line)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let no_code = delisted.replace('`', "");
    let no_emphasis = no_code.replace('*', "");
    strip_links_to_text(&no_emphasis)
}

/// `[text](url)` → `text`; malformed/unmatched brackets are left literal.
fn strip_links_to_text(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    loop {
        let Some(start) = rest.find('[') else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..start]);
        let after_bracket = &rest[start + 1..];
        let Some(close_idx) = after_bracket.find(']') else {
            out.push('[');
            rest = after_bracket;
            continue;
        };
        let text = &after_bracket[..close_idx];
        let after_close = &after_bracket[close_idx + 1..];
        if let Some((after_paren, paren_close)) = after_close
            .strip_prefix('(')
            .and_then(|after_paren| after_paren.find(')').map(|pc| (after_paren, pc)))
        {
            out.push_str(text);
            rest = &after_paren[paren_close + 1..];
        } else {
            out.push('[');
            out.push_str(text);
            out.push(']');
            rest = after_close;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string_renders_empty() {
        assert_eq!(render(""), "");
        assert_eq!(to_plain(""), "");
        assert_eq!(truncate_plain("", 10), "");
    }

    #[test]
    fn paragraph_with_single_newline_becomes_br() {
        assert_eq!(render("line one\nline two"), "<p>line one<br>line two</p>");
    }

    #[test]
    fn blank_line_separates_paragraphs() {
        assert_eq!(render("first\n\nsecond"), "<p>first</p><p>second</p>");
    }

    #[test]
    fn dash_list_becomes_ul() {
        assert_eq!(
            render("- one\n- two\n- three"),
            "<ul><li>one</li><li>two</li><li>three</li></ul>"
        );
    }

    #[test]
    fn mixed_block_is_not_a_list() {
        // One line missing the `- ` marker means the whole block is a
        // paragraph, not a list.
        assert_eq!(render("- one\ntwo"), "<p>- one<br>two</p>");
    }

    #[test]
    fn bold_renders_strong() {
        assert_eq!(render("**hi**"), "<p><strong>hi</strong></p>");
    }

    #[test]
    fn italic_renders_em() {
        assert_eq!(render("*hi*"), "<p><em>hi</em></p>");
    }

    #[test]
    fn unmatched_bold_opener_is_literal_not_empty_em() {
        // No closing `**`: must fall back to the two literal asterisks, not
        // fall through to `match_italic` (which would greedily consume them
        // as an empty `*...*` pair and emit a stray `<em></em>`).
        assert_eq!(render("**bold"), "<p>**bold</p>");
        assert!(!render("**bold").contains("<em>"));
    }

    #[test]
    fn unmatched_bold_opener_then_real_italic() {
        // The literal `**` is emitted, then scanning resumes and finds a
        // genuine `*b*` italic pair.
        assert_eq!(render("**a*b*"), "<p>**a<em>b</em></p>");
    }

    #[test]
    fn bold_takes_priority_over_italic() {
        assert_eq!(
            render("**bold** and *italic*"),
            "<p><strong>bold</strong> and <em>italic</em></p>"
        );
    }

    #[test]
    fn inline_code_renders_code_tag() {
        assert_eq!(render("`x`"), "<p><code>x</code></p>");
    }

    #[test]
    fn code_content_is_not_further_transformed() {
        assert_eq!(render("`**x**`"), "<p><code>**x**</code></p>");
    }

    #[test]
    fn unmatched_trailing_backtick_is_literal() {
        assert_eq!(render("a `b"), "<p>a `b</p>");
    }

    #[test]
    fn explicit_link_renders_anchor() {
        assert_eq!(
            render("[site](https://example.com)"),
            "<p><a href=\"https://example.com\" rel=\"noopener noreferrer\">site</a></p>"
        );
    }

    #[test]
    fn mailto_link_is_allowed() {
        assert_eq!(
            render("[mail](mailto:a@example.com)"),
            "<p><a href=\"mailto:a@example.com\" rel=\"noopener noreferrer\">mail</a></p>"
        );
    }

    #[test]
    fn javascript_link_is_literal_no_anchor() {
        let out = render("[click](javascript:alert(1))");
        assert!(!out.contains("<a"));
        assert!(!out.contains("javascript:alert(1))\">"));
    }

    #[test]
    fn data_link_is_literal_no_anchor() {
        let out = render("[img](data:text/html,evil)");
        assert!(!out.contains("<a"));
    }

    #[test]
    fn bare_url_autolinks() {
        assert_eq!(
            render("see https://example.com/x for more"),
            "<p>see <a href=\"https://example.com/x\" rel=\"noopener noreferrer\">https://example.com/x</a> for more</p>"
        );
    }

    #[test]
    fn bare_url_trims_trailing_punctuation() {
        assert_eq!(
            render("visit https://example.com."),
            "<p>visit <a href=\"https://example.com\" rel=\"noopener noreferrer\">https://example.com</a>.</p>"
        );
    }

    #[test]
    fn heading_is_not_supported_renders_literal() {
        assert_eq!(render("# heading"), "<p># heading</p>");
    }

    #[test]
    fn image_is_not_supported_renders_literal() {
        let out = render("![alt](https://example.com/x.png)");
        assert!(!out.contains("<img"));
    }

    #[test]
    fn blockquote_is_not_supported_renders_literal() {
        assert_eq!(render("> quoted"), "<p>&gt; quoted</p>");
    }

    #[test]
    fn underscore_emphasis_is_not_supported() {
        assert_eq!(render("_not italic_"), "<p>_not italic_</p>");
    }

    #[test]
    fn nested_list_is_not_supported_stays_flat_text() {
        // Leading whitespace before `- ` still counts as a list marker per
        // spec ("after leading-whitespace trim"); this asserts there is no
        // nested <ul>, just one flat list.
        let out = render("- one\n  - two");
        assert!(!out.contains("<ul><li><ul>"));
    }

    #[test]
    fn script_tag_is_escaped_not_executed() {
        let out = render("<script>alert(1)</script>");
        assert!(!out.contains("<script>"));
        assert!(out.contains("&lt;script&gt;"));
    }

    #[test]
    fn img_onerror_is_escaped() {
        let out = render(r"<img src=x onerror=alert(1)>");
        assert!(!out.contains("<img"));
        assert!(!out.contains("onerror=alert(1)>"));
        assert!(out.contains("&lt;img"));
    }

    #[test]
    fn quotes_are_escaped() {
        let out = render(r#"she said "hi" and 'bye'"#);
        assert!(out.contains("&quot;hi&quot;"));
        assert!(out.contains("&#39;bye&#39;"));
    }

    #[test]
    fn ampersand_is_escaped_first() {
        // If `&` weren't escaped first, escaping `<` would produce `&lt;`
        // whose `&` would then get double-escaped into `&amp;lt;`.
        assert_eq!(render("<"), "<p>&lt;</p>");
        assert_eq!(render("&lt;"), "<p>&amp;lt;</p>");
    }

    #[test]
    fn to_plain_strips_markers_and_collapses_whitespace() {
        assert_eq!(
            to_plain("**bold**  *italic*\n\n- one\n- two `code`"),
            "bold italic one two code"
        );
    }

    #[test]
    fn to_plain_reduces_link_to_text() {
        assert_eq!(to_plain("[site](https://example.com)"), "site");
    }

    #[test]
    fn truncate_plain_appends_ellipsis_when_truncated() {
        assert_eq!(truncate_plain("hello world", 5), "hello…");
        assert_eq!(truncate_plain("hello", 5), "hello");
    }

    #[test]
    fn truncate_plain_is_char_boundary_safe_on_multibyte() {
        // Each Chinese character is a multi-byte UTF-8 sequence; a byte-based
        // slice at an arbitrary offset would panic or corrupt the string.
        let src = "你好世界这是一段測試文字用來確認多位元組字元不會導致崩潰";
        let out = truncate_plain(src, 5);
        assert_eq!(out.chars().count(), 6); // 5 chars + the ellipsis
        assert!(out.ends_with('…'));
    }
}
