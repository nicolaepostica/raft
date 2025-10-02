//! Shared XML helpers: encoding detection, entities, a lenient event walker.

use std::borrow::Cow;

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

/// Decodes a document to text: BOM, then the XML declaration's encoding, else UTF-8.
pub fn decode(bytes: &[u8]) -> Cow<'_, str> {
    if let Some((enc, bom)) = encoding_rs::Encoding::for_bom(bytes) {
        let rest = bytes.get(bom..).unwrap_or_default();
        return enc.decode_without_bom_handling(rest).0;
    }
    let head = bytes.get(..bytes.len().min(256)).unwrap_or_default();
    let head = String::from_utf8_lossy(head);
    let label = head
        .find("encoding=")
        .and_then(|i| head.get(i + 9..))
        .and_then(|rest| {
            let q = rest.chars().next()?;
            if q != '"' && q != '\'' {
                return None;
            }
            let rest = rest.get(1..)?;
            rest.find(q).and_then(|end| rest.get(..end))
        });
    let enc = label
        .and_then(|l| encoding_rs::Encoding::for_label(l.trim().as_bytes()))
        .unwrap_or(encoding_rs::UTF_8);
    enc.decode_without_bom_handling(bytes).0
}

/// The character for a named or numeric entity (`amp`, `nbsp`, `#8212`, `#x2014`).
pub fn entity(name: &str) -> Option<String> {
    if let Some(num) = name.strip_prefix('#') {
        let code = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => num.parse().ok()?,
        };
        return char::from_u32(code)
            .filter(|c| !c.is_control() || c.is_whitespace())
            .map(String::from);
    }
    let c = match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" | "ensp" | "emsp" | "thinsp" => ' ',
        "shy" => return Some(String::new()),
        "mdash" => '—',
        "ndash" => '–',
        "hellip" => '…',
        "laquo" => '«',
        "raquo" => '»',
        "ldquo" => '“',
        "rdquo" => '”',
        "bdquo" => '„',
        "lsquo" => '‘',
        "rsquo" => '’',
        "sbquo" => '‚',
        "bull" => '•',
        "middot" => '·',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "times" => '×',
        "divide" => '÷',
        "deg" => '°',
        "euro" => '€',
        "pound" => '£',
        "sect" => '§',
        "para" => '¶',
        "prime" => '′',
        "Prime" => '″',
        "larr" => '←',
        "rarr" => '→',
        "minus" => '−',
        "plusmn" => '±',
        "frac12" => '½',
        "le" => '≤',
        "ge" => '≥',
        "ne" => '≠',
        _ => return None,
    };
    Some(c.to_string())
}

/// What the walker reports.
pub enum Item<'a> {
    Start {
        name: String,
        tag: &'a BytesStart<'a>,
    },
    /// A self-closing element (`<br/>`): reported as start and end.
    Empty {
        name: String,
        tag: &'a BytesStart<'a>,
    },
    End {
        name: String,
    },
    Text(Cow<'a, str>),
}

/// Lower-cased local name (no namespace prefix).
pub fn local(raw: &str) -> String {
    raw.rsplit(':').next().unwrap_or(raw).to_ascii_lowercase()
}

/// The value of an attribute by local name.
pub fn attr(tag: &BytesStart<'_>, name: &str) -> Option<String> {
    tag.attributes()
        .with_checks(false)
        .flatten()
        .find(|a| local(a.key.as_ref()) == name)
        .map(|a| {
            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map(Cow::into_owned)
                .unwrap_or_else(|_| a.value.clone().into_owned())
        })
}

/// Walks a document leniently, calling `f` for each item; stops at the first
/// syntax error or after `max_events` events. Returns false if it stopped early.
pub fn walk(text: &str, max_events: usize, mut f: impl FnMut(Item<'_>)) -> bool {
    let mut reader = Reader::from_str(text);
    let cfg = reader.config_mut();
    cfg.check_end_names = false;
    cfg.allow_unmatched_ends = true;
    cfg.check_comments = false;
    let mut depth = 0usize;
    for _ in 0..max_events {
        match reader.read_event() {
            Ok(Event::Start(tag)) => {
                depth += 1;
                if depth > crate::MAX_DEPTH {
                    return false;
                }
                let name = local(tag.name().as_ref());
                f(Item::Start { name, tag: &tag });
            }
            Ok(Event::Empty(tag)) => {
                let name = local(tag.name().as_ref());
                f(Item::Empty { name, tag: &tag });
            }
            Ok(Event::End(tag)) => {
                depth = depth.saturating_sub(1);
                f(Item::End {
                    name: local(tag.name().as_ref()),
                });
            }
            Ok(Event::Text(t)) => f(Item::Text(t.xml10_content())),
            Ok(Event::CData(t)) => f(Item::Text(Cow::Owned(t.to_string()))),
            Ok(Event::GeneralRef(r)) => {
                if let Some(s) = entity(&r) {
                    f(Item::Text(Cow::Owned(s)));
                }
            }
            Ok(Event::Eof) => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
    false
}
