use std::io::Write;

use proptest::prelude::*;
use raft_doc::Format;

use crate::{Block, Kind, TextError, parse};

const FB2: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0" xmlns:l="http://www.w3.org/1999/xlink">
 <description><title-info><book-title>Ёжик в тумане</book-title><author><first-name>Сергей</first-name></author></title-info></description>
 <body>
  <title><p>Ёжик в тумане</p></title>
  <epigraph><p>Эпиграф</p><text-author>Кто-то</text-author></epigraph>
  <section>
   <title><p>Глава 1</p><p>Начало</p></title>
   <p>Первый   абзац с <emphasis>курсивом</emphasis> и&#160;пробелом.</p>
   <empty-line/>
   <poem><stanza><v>строка один</v><v>строка два</v></stanza></poem>
   <section><title><p>1.1</p></title><p>Вложенный</p></section>
  </section>
 </body>
 <body name="notes"><section><title><p>1</p></title><p>Примечание</p></section></body>
 <binary id="x.jpg" content-type="image/jpeg">AAAA</binary>
</FictionBook>"#;

#[test]
fn reads_fb2_structure() {
    let b = parse(FB2.as_bytes(), Format::Fb2).unwrap();
    assert_eq!(b.title, "Ёжик в тумане");
    let texts: Vec<&str> = b.blocks.iter().map(Block::text).collect();
    assert!(
        texts.contains(&"Первый абзац с курсивом и пробелом."),
        "{texts:?}"
    );
    assert!(b.blocks.contains(&Block::Paragraph {
        kind: Kind::Quote,
        text: "Эпиграф".into()
    }));
    assert!(b.blocks.contains(&Block::Paragraph {
        kind: Kind::Verse,
        text: "строка один".into()
    }));
    assert!(b.blocks.contains(&Block::Break));
    assert!(!texts.iter().any(|t| t.contains("AAAA")));
    let titles: Vec<&str> = b.chapters.iter().map(|c| c.title.as_str()).collect();
    assert_eq!(
        titles,
        ["Ёжик в тумане", "Глава 1. Начало", "1.1", "Note", "1"]
    );
    // Chapter starts point at their heading.
    for c in &b.chapters {
        assert!(
            matches!(b.blocks.get(c.start), Some(Block::Heading { .. })),
            "{c:?}"
        );
    }
}

#[test]
fn decodes_windows_1251_fb2() {
    let xml = FB2.replace("encoding=\"utf-8\"", "encoding=\"windows-1251\"");
    let (bytes, _, _) = encoding_rs::WINDOWS_1251.encode(&xml);
    let b = parse(&bytes, Format::Fb2).unwrap();
    assert_eq!(b.title, "Ёжик в тумане");
}

#[test]
fn damaged_fb2_keeps_what_was_read() {
    let cut = &FB2[..FB2.find("<poem>").unwrap()];
    let b = parse(cut.as_bytes(), Format::Fb2).unwrap();
    assert!(b.blocks.iter().any(|x| x.text().starts_with("Первый")));
    assert_eq!(
        parse(b"<FictionBook></FictionBook>", Format::Fb2),
        Err(TextError::Empty)
    );
}

fn epub(files: &[(&str, &str)]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, body) in files {
            z.start_file(*name, opts).unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }
    buf.into_inner()
}

const CONTAINER: &str = r#"<?xml version="1.0"?><container><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
const OPF: &str = r#"<?xml version="1.0"?><package xmlns:dc="http://purl.org/dc/elements/1.1/"><metadata><dc:title>Rust &amp; Co</dc:title></metadata>
<manifest><item id="c1" href="text/ch%201.xhtml" media-type="application/xhtml+xml"/><item id="c2" href="text/ch2.xhtml#top" media-type="application/xhtml+xml"/><item id="img" href="i.png" media-type="image/png"/></manifest>
<spine><itemref idref="c1"/><itemref idref="img"/><itemref idref="missing"/><itemref idref="c2"/></spine></package>"#;
const CH1: &str = r#"<html><head><title>skip me</title><style>p{}</style></head><body>
<h1>Ownership</h1><p>Every value has an&nbsp;owner&mdash;exactly one.</p>
<blockquote><p>Fearless concurrency</p></blockquote><pre>fn main() {
    println!("hi");
}</pre><p>line one<br/>line two</p><hr/><div>Loose <b>bold</b> text</div></body></html>"#;
const CH2: &str = "<html><body><p>No heading here.</p></body></html>";

#[test]
fn reads_epub_spine_in_order() {
    let bytes = epub(&[
        ("mimetype", "application/epub+zip"),
        ("META-INF/container.xml", CONTAINER),
        ("OEBPS/content.opf", OPF),
        ("OEBPS/text/ch 1.xhtml", CH1),
        ("OEBPS/text/ch2.xhtml", CH2),
    ]);
    let b = parse(&bytes, Format::Epub).unwrap();
    assert_eq!(b.title, "Rust & Co");
    let texts: Vec<&str> = b.blocks.iter().map(Block::text).collect();
    assert_eq!(texts.first(), Some(&"Ownership"));
    assert!(
        texts.contains(&"Every value has an owner—exactly one."),
        "{texts:?}"
    );
    assert!(b.blocks.contains(&Block::Paragraph {
        kind: Kind::Quote,
        text: "Fearless concurrency".into()
    }));
    assert!(b.blocks.iter().any(
        |x| matches!(x, Block::Paragraph { kind: Kind::Pre, text } if text.contains("    println!"))
    ));
    assert!(texts.contains(&"line one\nline two"));
    assert!(texts.contains(&"Loose bold text"));
    assert!(!texts.iter().any(|t| t.contains("skip me")));
    let titles: Vec<&str> = b.chapters.iter().map(|c| c.title.as_str()).collect();
    assert_eq!(titles, ["Ownership", "Partea 4"]);
}

#[test]
fn bad_epubs_are_errors() {
    assert!(matches!(
        parse(b"not a zip", Format::Epub),
        Err(TextError::Epub(_))
    ));
    let no_container = epub(&[("mimetype", "application/epub+zip")]);
    assert!(matches!(
        parse(&no_container, Format::Epub),
        Err(TextError::Epub(_))
    ));
    assert!(matches!(
        parse(b"%PDF", Format::Pdf),
        Err(TextError::Unsupported("pdf"))
    ));
}

#[test]
fn joins_paths() {
    use crate::epub::join_for_test as join;
    assert_eq!(join("OEBPS", "text/a.xhtml"), "OEBPS/text/a.xhtml");
    assert_eq!(
        join("OEBPS/text", "../img/b%20c.png#x"),
        "OEBPS/img/b c.png"
    );
    assert_eq!(join("", "./a.html"), "a.html");
    assert_eq!(join("a", "../../../x"), "x");
}

proptest! {
    #[test]
    fn fb2_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let _ = parse(&bytes, Format::Fb2);
    }

    #[test]
    fn fb2_like_text_never_panics(body in "(<(p|v|section|title|poem|stanza|empty-line/|binary|body)>|</(p|v|section|title|poem|stanza|binary|body)>|[a-zа-я &;#0-9]){0,200}") {
        let doc = format!("<FictionBook><body>{body}</body></FictionBook>");
        if let Ok(b) = parse(doc.as_bytes(), Format::Fb2) {
            for c in &b.chapters {
                prop_assert!(c.start < b.blocks.len());
            }
        }
    }

    #[test]
    fn epub_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let _ = parse(&bytes, Format::Epub);
    }

    #[test]
    fn epub_with_random_documents_never_panics(doc in "(<(p|div|h1|br/|pre|blockquote|head|script)>|</(p|div|h1|pre|blockquote|head|script)>|[a-z &;]){0,200}") {
        let bytes = epub(&[
            ("META-INF/container.xml", CONTAINER),
            ("OEBPS/content.opf", OPF),
            ("OEBPS/text/ch 1.xhtml", &doc),
        ]);
        let _ = parse(&bytes, Format::Epub);
    }
}

/// Parses every EPUB and FB2 in the library: `cargo test -p raft-text -- --ignored --nocapture`.
#[test]
#[ignore = "reads the whole library"]
fn parses_the_library() {
    let Ok(dir) = std::fs::read_dir("/backup/books") else {
        return;
    };
    let (mut ok, mut failed) = (0, Vec::new());
    for entry in dir.flatten() {
        let path = entry.path();
        let format = match path.extension().and_then(|e| e.to_str()) {
            Some("epub") => Format::Epub,
            Some("fb2") => Format::Fb2,
            _ => continue,
        };
        let bytes = std::fs::read(&path).unwrap();
        match parse(&bytes, format) {
            Ok(b) if b.blocks.len() > 20 => ok += 1,
            Ok(b) => failed.push(format!(
                "{}: only {} blocks",
                path.display(),
                b.blocks.len()
            )),
            Err(e) => failed.push(format!("{}: {e}", path.display())),
        }
    }
    println!(
        "parsed {ok}, problems {}:\n{}",
        failed.len(),
        failed.join("\n")
    );
}
