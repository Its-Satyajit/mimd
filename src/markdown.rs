//! Markdown parsing.
//!
//! Pure text in, styled runs out. Nothing here knows about fonts, pixels or
//! windows, which is what lets the layout and the renderer be reasoned about
//! (and tested) without a display.

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum S {
    Text,
    Bold,
    Italic,
    Code,
    Link,
    /// Both emphases at once; the parser never emits it directly.
    BoldItalic,
}

impl S {
    /// How this span's emphasis composites with an enclosing one.
    ///
    /// Code and link win outright; otherwise the innermost emphasis wins,
    /// and bold+italic is not a distinct style so the pair collapses to bold.
    fn merge(self, outer: S) -> S {
        match (self, outer) {
            (S::Code, _) | (S::Link, _) => self,
            (_, S::Code) | (_, S::Link) => outer,
            (S::Bold, S::Italic) | (S::Italic, S::Bold) => S::BoldItalic,
            (S::BoldItalic, _) | (_, S::BoldItalic) => S::BoldItalic,
            (a, _) => a,
        }
    }

    /// Promote any emphasis to bold.
    ///
    /// A heading line is already distinguished by size and ink; forcing its
    /// inline emphasis to bold keeps `**strong**` inside a heading visible
    /// instead of collapsing back to the heading's own weight.
    pub fn emphasised(self) -> S {
        match self {
            S::Bold | S::Italic | S::BoldItalic => S::Bold,
            other => other,
        }
    }
}

pub type Run = Vec<(char, S)>;

/// A semantic block. The renderer decides fonts and sizes; the parser
/// decides only structure. No font knowledge lives here.
#[derive(Debug, PartialEq)]
pub enum Block {
    Heading(u8, Run),
    Para(Run),
    Bullet(usize, Run),
    Number(usize, Run),
    Quote(Run),
    Rule,
    Code(Run),
}

/// Markdown source -> semantic blocks.
///
/// Deliberately a useful subset, not CommonMark.
pub fn parse(text: &str) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut para: Run = Vec::new();
    let mut fence: Option<Run> = None;

    let flush = |para: &mut Run, blocks: &mut Vec<Block>| {
        if !para.is_empty() {
            blocks.push(Block::Para(std::mem::take(para)));
        }
    };

    for raw in text.lines() {
        // Fenced code block
        if raw.trim_start().starts_with("```") {
            flush(&mut para, &mut blocks);
            if fence.is_some() {
                blocks.push(Block::Code(fence.take().unwrap()));
            } else {
                fence = Some(Run::new());
            }
            continue;
        }
        if let Some(acc) = fence.as_mut() {
            // Inside a fence nothing is markup.
            if !acc.is_empty() {
                acc.push(('\n', S::Code));
            }
            acc.extend(raw.chars().map(|c| (c, S::Code)));
            continue;
        }

        let indent = raw.len() - raw.trim_start().len();
        let t = raw.trim_start();

        if t.is_empty() {
            flush(&mut para, &mut blocks);
            continue;
        }

        if is_rule(t) {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Rule);
            continue;
        }

        let hashes = t.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Heading(
                hashes as u8,
                inline(t[hashes..].trim(), S::Text),
            ));
            continue;
        }

        // A list item ends any open paragraph; keep nesting via indent.
        if let Some(body) = bullet_body(t) {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Bullet(indent / 2, inline(body, S::Text)));
            continue;
        }
        if let Some((n, body)) = numbered(t) {
            flush(&mut para, &mut blocks);
            let mut run = inline(&n.to_string(), S::Text);
            run.push(('.', S::Text));
            run.push((' ', S::Text));
            run.extend(inline(body, S::Text));
            blocks.push(Block::Number(indent / 2, run));
            continue;
        }

        if let Some(body) = t.strip_prefix('>') {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Quote(inline(body.trim(), S::Text)));
            continue;
        }

        // Lazy continuation of the current paragraph.
        let mut run = inline(t, S::Text);
        if !para.is_empty() {
            para.push((' ', S::Text));
            para.append(&mut run);
        } else {
            para = run;
        }
    }
    if let Some(acc) = fence {
        // Unterminated fence: still show what we have.
        blocks.push(Block::Code(acc));
    }
    flush(&mut para, &mut blocks);
    blocks
}

fn is_rule(s: &str) -> bool {
    let t: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    t.len() >= 3
        && (t.chars().all(|c| c == '-')
            || t.chars().all(|c| c == '*')
            || t.chars().all(|c| c == '_'))
}

fn bullet_body(s: &str) -> Option<&str> {
    for m in ["- ", "* ", "+ "] {
        if let Some(b) = s.strip_prefix(m) {
            return Some(b);
        }
    }
    None
}

/// "1. text" -> (1, "text")
fn numbered(t: &str) -> Option<(u32, &str)> {
    let n = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if n == 0 {
        return None;
    }
    Some((t.get(..n)?.parse().ok()?, t.get(n..)?.strip_prefix(". ")?))
}

/// Inline scan: **bold**, *italic*, `code`, [link](url), backslash escapes.
fn inline(text: &str, base: S) -> Run {
    let c: Vec<char> = text.chars().collect();
    let mut out = Run::new();
    let mut i = 0;
    while i < c.len() {
        if c[i] == '\\' && i + 1 < c.len() {
            out.push((c[i + 1], base));
            i += 2;
            continue;
        }
        if c[i] == '`' {
            if let Some(e) = (i + 1..c.len()).find(|&j| c[j] == '`') {
                out.extend(c[i + 1..e].iter().map(|&ch| (ch, S::Code)));
                i = e + 1;
                continue;
            }
        }
        if c[i] == '*' || c[i] == '_' {
            let dbl = i + 1 < c.len() && c[i + 1] == c[i];
            let open = if dbl { 2 } else { 1 };
            let style = if dbl { S::Bold } else { S::Italic };
            let from = i + open;
            if let Some(e) = find_close(&c, from, c[i], dbl) {
                if e > from {
                    let inner: String = c[from..e].iter().collect();
                    for (ch, st) in inline(&inner, style) {
                        out.push((ch, st.merge(base)));
                    }
                    i = e + open;
                    continue;
                }
            }
        }
        if c[i] == '[' {
            if let Some(cb) = (i + 1..c.len()).find(|&j| c[j] == ']') {
                if c.get(cb + 1) == Some(&'(') {
                    if let Some(p) = (cb + 2..c.len()).find(|&j| c[j] == ')') {
                        out.extend(c[i + 1..cb].iter().map(|&ch| (ch, S::Link)));
                        i = p + 1;
                        continue;
                    }
                }
            }
        }
        out.push((c[i], base));
        i += 1;
    }
    out
}

fn find_close(c: &[char], from: usize, target: char, pair: bool) -> Option<usize> {
    (from..c.len()).find(|&i| c[i] == target && (!pair || c.get(i + 1) == Some(&target)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_styles() {
        let r = inline("a **b** `c` [d](e)", S::Text);
        assert_eq!(r[0], ('a', S::Text));
        for (want, style) in [('b', S::Bold), ('c', S::Code), ('d', S::Link)] {
            assert!(
                r.iter().any(|&(ch, s)| ch == want && s == style),
                "missing {want:?} as {style:?}"
            );
        }
    }

    #[test]
    fn unclosed_marker_is_literal() {
        let r = inline("a * b", S::Text);
        assert_eq!(r.iter().map(|x| x.0).collect::<String>(), "a * b");
    }

    #[test]
    fn parser_emits_blocks_not_rows() {
        let b = parse("# T\n\n- item\n\n> quoted\n\n1. first\n\n---\n");
        assert_eq!(b[0], Block::Heading(1, vec![('T', S::Text)]));
        assert_eq!(
            b[1],
            Block::Bullet(
                0,
                vec![
                    ('i', S::Text),
                    ('t', S::Text),
                    ('e', S::Text),
                    ('m', S::Text)
                ]
            )
        );
        assert_eq!(
            b[2],
            Block::Quote(vec![
                ('q', S::Text),
                ('u', S::Text),
                ('o', S::Text),
                ('t', S::Text),
                ('e', S::Text),
                ('d', S::Text)
            ])
        );
        assert!(matches!(b[3], Block::Number(0, _)));
        assert_eq!(b[4], Block::Rule);
    }

    #[test]
    fn nested_lists_keep_their_depth() {
        let b = parse("- a\n    - b\n        - c\n");
        let depths: Vec<usize> = b
            .iter()
            .filter_map(|x| match x {
                Block::Bullet(d, _) => Some(*d),
                _ => None,
            })
            .collect();
        assert_eq!(depths, vec![0, 2, 4]);
    }

    #[test]
    fn italic_is_its_own_style_and_drops_delimiters() {
        let r = inline("*em*", S::Text);
        assert_eq!(r, vec![('e', S::Italic), ('m', S::Italic)]);
        let b = inline("**strong**", S::Text);
        assert!(b.iter().all(|&(_, s)| s == S::Bold));
        assert!(!b.iter().any(|&(c, _)| c == '*'));
    }

    #[test]
    fn fenced_code_becomes_a_code_block() {
        let b = parse("```\nlet x = 1;\n# not a heading\n```\n");
        match &b[0] {
            Block::Code(run) => {
                let t: String = run.iter().map(|x| x.0).collect();
                assert_eq!(t, "let x = 1;\n# not a heading");
                assert!(run.iter().all(|&(_, s)| s == S::Code));
            }
            other => panic!("expected Code, got {other:?}"),
        }
    }

    #[test]
    fn code_span_wins_over_surrounding_emphasis() {
        let r = inline("**a `b`**", S::Text);
        assert!(r.iter().any(|&(c, s)| c == 'b' && s == S::Code));
    }
}
