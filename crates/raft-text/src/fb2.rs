//! FictionBook 2 (FB2): XML with sections, titles, paragraphs, poems and notes.

use crate::xml::{Item, attr, decode, walk};
use crate::{Block, Builder, Kind, TextBook};

/// Most XML events read from one FB2 file.
const MAX_EVENTS: usize = 20_000_000;

pub fn parse(bytes: &[u8]) -> TextBook {
    let text = decode(bytes);
    let mut b = Builder::default();
    // Element names from the root to the current element.
    let mut stack: Vec<String> = Vec::new();
    let mut skip = 0usize;
    let mut sections = 0usize;
    let mut in_title = false;
    let mut title_parts: Vec<String> = Vec::new();
    let mut book_title = String::new();

    let inside = |stack: &[String], name: &str| stack.iter().any(|s| s == name);

    walk(&text, MAX_EVENTS, |item| match item {
        Item::Start { name, tag } => {
            if skip > 0 || matches!(name.as_str(), "binary" | "stylesheet") {
                skip += 1;
                stack.push(name);
                return;
            }
            match name.as_str() {
                "body" => {
                    b.flush(Kind::Normal);
                    if let Some(kind) =
                        attr(tag, "name").filter(|n| n == "notes" || n == "comments")
                    {
                        b.chapter(if kind == "notes" {
                            "Note".into()
                        } else {
                            "Comentarii".into()
                        });
                    }
                }
                "section" => {
                    b.flush(Kind::Normal);
                    sections += 1;
                }
                "title" if !inside(&stack, "description") => {
                    b.flush(Kind::Normal);
                    in_title = true;
                    title_parts.clear();
                }
                "p" | "v" | "subtitle" | "text-author" | "td" | "th" => {
                    if in_title {
                        let t = b.take_text();
                        if !t.is_empty() {
                            title_parts.push(t);
                        }
                    } else {
                        b.flush(Kind::Normal);
                    }
                }
                _ => {}
            }
            stack.push(name);
        }
        Item::Empty { name, .. } => {
            if skip == 0 && name == "empty-line" && !in_title {
                b.flush(Kind::Normal);
                b.block(Block::Break);
            }
        }
        Item::End { name } => {
            // Pop up to the matching element (tolerates unclosed children).
            let Some(pos) = stack.iter().rposition(|s| *s == name) else {
                return;
            };
            let popped: Vec<String> = stack.drain(pos..).collect();
            if skip > 0 {
                skip = skip.saturating_sub(popped.len());
                return;
            }
            let in_description = inside(&stack, "description");
            match name.as_str() {
                "book-title" if in_description => {
                    book_title = b.take_text();
                }
                _ if in_description => {
                    b.take_text();
                }
                "title" if in_title => {
                    in_title = false;
                    let t = b.take_text();
                    if !t.is_empty() {
                        title_parts.push(t);
                    }
                    let title = title_parts.join(". ");
                    if !title.is_empty() {
                        let level = u8::try_from(sections.max(1)).unwrap_or(6);
                        b.chapter(title.clone());
                        b.block(Block::Heading {
                            level: level.min(6),
                            text: title,
                        });
                    }
                }
                "p" | "text-author" | "td" | "th" if !in_title => {
                    let kind = if inside(&stack, "cite") || inside(&stack, "epigraph") {
                        Kind::Quote
                    } else {
                        Kind::Normal
                    };
                    b.flush(kind);
                }
                "v" if !in_title => b.flush(Kind::Verse),
                "subtitle" if !in_title => {
                    b.flush_heading(u8::try_from(sections + 1).unwrap_or(6).min(6));
                }
                "stanza" | "poem" => b.block(Block::Break),
                "section" => {
                    b.flush(Kind::Normal);
                    sections = sections.saturating_sub(1);
                }
                _ => {}
            }
        }
        Item::Text(t) => {
            if skip == 0 {
                b.push_text(&t, false);
            }
        }
    });
    let mut book = b.finish();
    book.title = book_title;
    book
}
