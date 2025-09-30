//! EPUB 2/3: a zip with an OPF package whose spine lists XHTML documents.

use std::collections::HashMap;
use std::io::{Cursor, Read};

use zip::ZipArchive;

use crate::xml::{Item, attr, decode, walk};
use crate::{Block, Builder, Kind, TextBook, TextError};

/// Most entries accepted in the zip.
const MAX_ENTRIES: usize = 50_000;
/// Largest decompressed XML/XHTML entry, in bytes.
const MAX_ENTRY: u64 = 64 * 1024 * 1024;
/// Largest total decompressed text read, in bytes.
const MAX_TOTAL: u64 = 512 * 1024 * 1024;
/// Most spine documents read.
const MAX_SPINE: usize = 10_000;
/// Most XML events read from one document.
const MAX_EVENTS: usize = 5_000_000;

struct Zip<'a> {
    archive: ZipArchive<Cursor<&'a [u8]>>,
    read: u64,
}

impl Zip<'_> {
    /// Reads one entry, capped; None if missing, too large or damaged.
    fn read(&mut self, name: &str) -> Option<Vec<u8>> {
        let mut file = self.archive.by_name(name).ok()?;
        let budget = MAX_ENTRY.min(MAX_TOTAL.saturating_sub(self.read));
        let mut out = Vec::new();
        (&mut file).take(budget + 1).read_to_end(&mut out).ok()?;
        if out.len() as u64 > budget {
            return None;
        }
        self.read += out.len() as u64;
        Some(out)
    }
}

pub fn parse(bytes: &[u8]) -> Result<TextBook, TextError> {
    let archive = ZipArchive::new(Cursor::new(bytes)).map_err(|e| TextError::Epub(e.to_string()))?;
    if archive.len() > MAX_ENTRIES {
        return Err(TextError::Epub(format!("too many entries ({})", archive.len())));
    }
    let mut zip = Zip { archive, read: 0 };
    let container = zip
        .read("META-INF/container.xml")
        .ok_or_else(|| TextError::Epub("no META-INF/container.xml".into()))?;
    let opf_path = rootfile(&decode(&container)).ok_or_else(|| TextError::Epub("no rootfile in container.xml".into()))?;
    let opf = zip.read(&opf_path).ok_or_else(|| TextError::Epub(format!("cannot read {opf_path}")))?;
    let package = Package::parse(&decode(&opf));
    let base = opf_path.rsplit_once('/').map_or("", |(dir, _)| dir);

    let mut b = Builder::default();
    for (n, idref) in package.spine.iter().take(MAX_SPINE).enumerate() {
        let Some((href, media)) = package.manifest.get(idref) else {
            continue;
        };
        if !(media.contains("html") || media.contains("xml")) {
            continue;
        }
        let path = join(base, href);
        let Some(doc) = zip.read(&path) else {
            continue;
        };
        b.flush(Kind::Normal);
        b.chapter(String::new());
        let title = document(&decode(&doc), &mut b);
        if let Some(last) = b.book.chapters.last_mut() {
            last.title = if title.is_empty() { format!("Partea {}", n + 1) } else { title };
        }
    }
    let mut book = b.finish();
    book.title = package.title;
    Ok(book)
}

/// `full-path` of the first rootfile in container.xml.
fn rootfile(xml: &str) -> Option<String> {
    let mut found = None;
    walk(xml, 10_000, |item| {
        if let Item::Start { name, tag } | Item::Empty { name, tag } = item
            && name == "rootfile"
            && found.is_none()
        {
            found = attr(tag, "full-path");
        }
    });
    found.map(|p| percent_decode(&p))
}

#[derive(Default)]
struct Package {
    title: String,
    /// id → (href, media type)
    manifest: HashMap<String, (String, String)>,
    spine: Vec<String>,
}

impl Package {
    fn parse(xml: &str) -> Self {
        let mut p = Package::default();
        let mut in_title = false;
        walk(xml, 1_000_000, |item| match item {
            Item::Start { name, tag } | Item::Empty { name, tag } => match name.as_str() {
                "item" => {
                    if let (Some(id), Some(href)) = (attr(tag, "id"), attr(tag, "href")) {
                        let media = attr(tag, "media-type").unwrap_or_default();
                        p.manifest.insert(id, (href, media));
                    }
                }
                "itemref" => {
                    if let Some(id) = attr(tag, "idref")
                        && p.spine.len() < MAX_SPINE
                    {
                        p.spine.push(id);
                    }
                }
                "title" if p.title.is_empty() => in_title = true,
                _ => {}
            },
            Item::End { name } if name == "title" => in_title = false,
            Item::Text(t) if in_title => p.title.push_str(t.trim()),
            _ => {}
        });
        p
    }
}

/// Resolves `href` against the OPF folder: drops `#fragment`, decodes %XX, folds `..`.
fn join(base: &str, href: &str) -> String {
    let href = href.split('#').next().unwrap_or_default();
    let href = percent_decode(href);
    let mut parts: Vec<&str> = if base.is_empty() { Vec::new() } else { base.split('/').collect() };
    for seg in href.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&c) = bytes.get(i) {
        if c == b'%'
            && let Some(hex) = bytes.get(i + 1..i + 3)
            && let Ok(v) = u8::from_str_radix(&String::from_utf8_lossy(hex), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(c);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Elements that end the current paragraph.
fn is_block(name: &str) -> bool {
    matches!(
        name,
        "p" | "div" | "li" | "dt" | "dd" | "blockquote" | "pre" | "td" | "th" | "tr" | "table" | "ul" | "ol"
            | "dl" | "figure" | "figcaption" | "section" | "article" | "header" | "footer" | "aside"
            | "nav" | "body" | "address" | "center"
    )
}

fn heading_level(name: &str) -> Option<u8> {
    match name {
        "h1" => Some(1),
        "h2" => Some(2),
        "h3" => Some(3),
        "h4" => Some(4),
        "h5" => Some(5),
        "h6" => Some(6),
        _ => None,
    }
}

/// Reads one XHTML document into `b`; returns its first heading (the chapter title).
fn document(xml: &str, b: &mut Builder) -> String {
    let mut title = String::new();
    let mut skip = 0usize;
    let mut quote = 0usize;
    let mut pre = 0usize;
    let mut in_body = !xml.contains("<body");
    walk(xml, MAX_EVENTS, |item| match item {
        Item::Start { name, .. } => {
            // Inside a skipped element every child counts, so its end is found again.
            if skip > 0 || matches!(name.as_str(), "head" | "script" | "style" | "svg" | "math" | "title") {
                skip += 1;
                return;
            }
            if name == "body" {
                in_body = true;
            }
            if heading_level(&name).is_some() || is_block(&name) {
                let kind = if pre > 0 { Kind::Pre } else if quote > 0 { Kind::Quote } else { Kind::Normal };
                b.flush(kind);
            }
            match name.as_str() {
                "blockquote" => quote += 1,
                "pre" => pre += 1,
                _ => {}
            }
        }
        Item::Empty { name, .. } => {
            if skip == 0 && in_body {
                match name.as_str() {
                    "br" => b.line_break(),
                    "hr" => {
                        b.flush(Kind::Normal);
                        b.block(Block::Break);
                    }
                    _ => {}
                }
            }
        }
        Item::End { name } => {
            if skip > 0 {
                skip -= 1;
                return;
            }
            if let Some(level) = heading_level(&name) {
                let t = b.flush_heading(level);
                if title.is_empty() {
                    title = t;
                }
            } else if is_block(&name) {
                let kind = if pre > 0 { Kind::Pre } else if quote > 0 { Kind::Quote } else { Kind::Normal };
                b.flush(kind);
            }
            match name.as_str() {
                "blockquote" => quote = quote.saturating_sub(1),
                "pre" => pre = pre.saturating_sub(1),
                _ => {}
            }
        }
        Item::Text(t) => {
            if skip == 0 && in_body {
                b.push_text(&t, pre > 0);
            }
        }
    });
    b.flush(Kind::Normal);
    title
}

#[cfg(test)]
pub(crate) fn join_for_test(base: &str, href: &str) -> String {
    join(base, href)
}
