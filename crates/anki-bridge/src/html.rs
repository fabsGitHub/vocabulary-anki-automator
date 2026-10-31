//! Plain card text <-> Anki field HTML.
//!
//! Anki stores every field as HTML. On export, card text is escaped so that
//! text which merely looks like markup (`Vec<String>`, a pasted `<script>`)
//! shows up literally instead of being rendered, or run, by Anki's reviewer.
//! On import, Anki's editor has added markup nobody typed: `&nbsp;` for
//! trailing spaces, a `<div>` per line, `<br>`, formatting tags, and
//! `[sound:...]` references wherever audio was attached. The import side is a
//! small converter for exactly that, not an HTML parser: anything it does not
//! recognise stays as text, so `a < b` or `AT&T` survive unchanged. Imported
//! notes are untrusted (shared decks), so every step is linear in the field
//! length.

/// Tags that start a new line in the rendered field.
const LINE_BREAK_TAGS: &[&str] = &[
    "br", "div", "p", "li", "tr", "h1", "h2", "h3", "h4", "h5", "h6",
];

/// Entity names longer than this are not looked up (`#x10FFFF` is 8 chars).
const MAX_ENTITY_LEN: usize = 10;

/// Card text as field HTML: `&`, `<` and `>` escaped, the minimum that keeps
/// text from being read as markup (quotes only matter inside attributes).
/// Line breaks stay `\n`, the legacy combined-example format (§ 9).
/// [`to_plain_text`] decodes the escapes again.
pub(crate) fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

/// Tags removed (line-level tags become line breaks), entities decoded once,
/// `[sound:...]` references removed, runs of spaces collapsed, and every line
/// trimmed with blank lines dropped.
pub(crate) fn to_plain_text(html: &str) -> String {
    let text = decode_entities(&strip_tags(html));
    normalize_whitespace(&strip_sound_refs(&text))
}

fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let looks_like_tag =
            after.starts_with(|c: char| c.is_ascii_alphabetic() || c == '/' || c == '!');
        if !looks_like_tag {
            out.push('<');
            rest = after;
            continue;
        }
        match after.find('>') {
            Some(end) => {
                if is_line_break(&after[..end]) {
                    out.push('\n');
                }
                rest = &after[end + 1..];
            }
            // No later `>` can close this or any following tag: the rest is
            // text. (Searching again for every later `<` would be quadratic.)
            None => {
                rest = &rest[start..];
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

fn is_line_break(tag: &str) -> bool {
    let name: String = tag
        .trim_start_matches('/')
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect();
    LINE_BREAK_TAGS.contains(&name.to_ascii_lowercase().as_str())
}

fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        // Only the next few bytes can hold the `;` of an entity; `;` is ASCII,
        // so its byte position is a char boundary.
        let decoded = after
            .bytes()
            .take(MAX_ENTITY_LEN + 1)
            .position(|b| b == b';')
            .and_then(|end| decode_entity(&after[..end]).map(|c| (c, end)));
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_entity(name: &str) -> Option<char> {
    match name {
        "nbsp" => Some('\u{a0}'),
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let number = name.strip_prefix('#')?;
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse().ok()?,
            };
            char::from_u32(code).filter(|&c| c != '\0')
        }
    }
}

fn strip_sound_refs(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("[sound:") {
        let Some(len) = rest[start..].find(']') else {
            break;
        };
        out.push_str(&rest[..start]);
        rest = &rest[start + len + 1..];
    }
    out.push_str(rest);
    out
}

/// Only ASCII blanks and no-break spaces collapse: an ideographic space
/// (U+3000) inside a Chinese sentence is content, not markup.
fn normalize_whitespace(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            line.split([' ', '\t', '\r', '\u{a0}'])
                .filter(|word| !word.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_anki_field_html_to_plain_text() {
        let cases = [
            ("plain", "plain"),
            ("Haus&nbsp;", "Haus"),
            ("the&nbsp; house", "the house"),
            ("a &amp; b", "a & b"),
            ("&lt;b&gt; is bold", "<b> is bold"),
            ("&quot;hi&quot; &apos;x&#39;", "\"hi\" 'x'"),
            ("&#x4E2D;&#25991;", "中文"),
            ("&amp;nbsp; is decoded once", "&nbsp; is decoded once"),
            (
                "AT&T & co; &unknown; &#xZZ; &#0;",
                "AT&T & co; &unknown; &#xZZ; &#0;",
            ),
            (
                "<b>bold</b> <i>and</i> <span style=\"color: red\">red</span>",
                "bold and red",
            ),
            ("one<br>two<br/>three<BR />four", "one\ntwo\nthree\nfour"),
            ("<div>a</div><div>b &amp; c</div>", "a\nb & c"),
            (
                "first line<div>second line</div>",
                "first line\nsecond line",
            ),
            ("<p>para</p><ul><li>item</li></ul>", "para\nitem"),
            ("a < b and c > d", "a < b and c > d"),
            ("x<y", "x<y"),
            ("<!-- note -->text", "text"),
            ("serendipity[sound:vocab_1_fwd.mp3]", "serendipity"),
            ("[sound:a.mp3] [sound:b.mp3]", ""),
            ("[sound:unterminated", "[sound:unterminated"),
            ("  spaced \t  out  ", "spaced out"),
            ("raw\nnewline\r\n kept", "raw\nnewline\nkept"),
            ("one<br><br>  <br>two", "one\ntwo"),
            ("我　爱 你", "我　爱 你"),
            ("", ""),
        ];
        for (html, expected) in cases {
            assert_eq!(to_plain_text(html), expected, "input: {html:?}");
        }
    }

    #[test]
    fn escaped_text_reads_back_unchanged() {
        let cases = [
            ("serendipity", "serendipity"),
            ("Vec<String>", "Vec&lt;String&gt;"),
            ("AT&T", "AT&amp;T"),
            ("&amp; stays literal", "&amp;amp; stays literal"),
            ("<b>not bold</b>", "&lt;b&gt;not bold&lt;/b&gt;"),
            (
                "<script>alert(1)</script>",
                "&lt;script&gt;alert(1)&lt;/script&gt;",
            ),
            ("don't \"quote\"", "don't \"quote\""),
            ("first\nsecond", "first\nsecond"),
            ("我们一起学习中文。", "我们一起学习中文。"),
            ("", ""),
        ];
        for (text, escaped) in cases {
            assert_eq!(escape_text(text), escaped, "{text:?}");
            assert_eq!(to_plain_text(escaped), text, "{text:?}");
        }
    }

    /// Imported notes are untrusted (shared decks): a field full of unmatched
    /// `&` or `<` must not make the conversion quadratic. Each input took tens
    /// of seconds when every `&`/`<` rescanned the rest of the field, and
    /// takes milliseconds when it does not. None of them contains a tag or an
    /// entity, so each converts to itself.
    #[test]
    fn hostile_fields_convert_in_linear_time() {
        let inputs = [
            "&".repeat(1_000_000),
            "<a".repeat(500_000),
            format!("{}>", "< ".repeat(500_000)),
        ];
        for html in inputs {
            let started = std::time::Instant::now();
            let text = to_plain_text(&html);
            let elapsed = started.elapsed();
            assert!(
                elapsed < std::time::Duration::from_secs(3),
                "{elapsed:?} for {:?}...",
                &html[..8]
            );
            assert!(text == html, "{:?}...", &text[..8]);
        }
    }
}
