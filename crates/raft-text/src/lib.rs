//! Text books (layer 3): EPUB and FB2 parsed into chapters of plain blocks.
//!
//! Input is untrusted. Sizes, counts and nesting are capped; a damaged document
//! yields the blocks read so far instead of an error, and nothing here panics.

mod epub;
mod fb2;
mod xml;

use raft_doc::Format;

/// Largest accepted book file, in bytes.
pub const MAX_INPUT: usize = 256 * 1024 * 1024;
/// Most blocks kept for one book.
pub const MAX_BLOCKS: usize = 500_000;
/// Longest block, in bytes (longer text is split into several blocks).
pub const MAX_BLOCK_BYTES: usize = 64 * 1024;
/// Deepest element nesting followed.
pub const MAX_DEPTH: usize = 256;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TextError {
    #[error("the book is too large ({0} bytes)")]
    TooLarge(usize),
    #[error("{0} books cannot be read as text")]
    Unsupported(&'static str),
    #[error("not a valid EPUB: {0}")]
    Epub(String),
    #[error("no readable text in the book")]
    Empty,
}

/// How a paragraph is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Normal,
    /// Quotation, epigraph.
    Quote,
    /// Line of a poem.
    Verse,
    /// Preformatted text (code): whitespace kept.
    Pre,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// Level 1 (largest) to 6.
    Heading { level: u8, text: String },
    Paragraph { kind: Kind, text: String },
    /// An empty line / scene break.
    Break,
}

impl Block {
    pub fn text(&self) -> &str {
        match self {
            Block::Heading { text, .. } | Block::Paragraph { text, .. } => text,
            Block::Break => "",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Chapter {
    /// May be empty when the source has no title for it.
    pub title: String,
    /// Index of the chapter's first block in [`TextBook::blocks`].
    pub start: usize,
}

/// A book as a flat list of blocks with chapter marks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextBook {
    pub title: String,
    pub blocks: Vec<Block>,
    pub chapters: Vec<Chapter>,
}

/// Parses a book of the given format.
pub fn parse(bytes: &[u8], format: Format) -> Result<TextBook, TextError> {
    if bytes.len() > MAX_INPUT {
        return Err(TextError::TooLarge(bytes.len()));
    }
    let book = match format {
        Format::Fb2 => fb2::parse(bytes),
        Format::Epub => epub::parse(bytes)?,
        other => return Err(TextError::Unsupported(other.slug())),
    };
    if book.blocks.iter().all(|b| b.text().trim().is_empty()) {
        return Err(TextError::Empty);
    }
    Ok(book)
}

/// Collects blocks while a document is walked.
#[derive(Debug, Default)]
pub(crate) struct Builder {
    pub book: TextBook,
    text: String,
    full: bool,
}

impl Builder {
    /// Adds text to the paragraph being built (whitespace collapsed unless `pre`).
    pub fn push_text(&mut self, s: &str, pre: bool) {
        if self.full {
            return;
        }
        if pre {
            self.text.push_str(s);
        } else {
            for c in s.chars() {
                if c.is_whitespace() {
                    if !self.text.is_empty() && !self.text.ends_with([' ', '\n']) {
                        self.text.push(' ');
                    }
                } else if c != '\u{AD}' {
                    self.text.push(c);
                }
            }
        }
        if self.text.len() > MAX_BLOCK_BYTES {
            self.flush(Kind::Normal);
        }
    }

    pub fn line_break(&mut self) {
        if !self.text.is_empty() && !self.text.ends_with('\n') {
            let trimmed = self.text.trim_end().len();
            self.text.truncate(trimmed);
            self.text.push('\n');
        }
    }

    /// Ends the current paragraph, if it has text.
    pub fn flush(&mut self, kind: Kind) {
        let text = std::mem::take(&mut self.text);
        let text = if kind == Kind::Pre { text.trim_matches('\n').to_owned() } else { text.trim().to_owned() };
        if !text.is_empty() {
            self.block(Block::Paragraph { kind, text });
        }
    }

    /// Ends the current text as a heading; returns its text.
    pub fn flush_heading(&mut self, level: u8) -> String {
        let text = std::mem::take(&mut self.text).trim().replace('\n', " ");
        if !text.is_empty() {
            self.block(Block::Heading { level: level.clamp(1, 6), text: text.clone() });
        }
        text
    }

    /// Takes the current text without adding a block.
    pub fn take_text(&mut self) -> String {
        std::mem::take(&mut self.text).trim().to_owned()
    }

    pub fn block(&mut self, b: Block) {
        if self.book.blocks.len() >= MAX_BLOCKS {
            self.full = true;
            return;
        }
        if b == Block::Break && matches!(self.book.blocks.last(), None | Some(Block::Break)) {
            return;
        }
        self.book.blocks.push(b);
    }

    /// Starts a chapter at the next block.
    pub fn chapter(&mut self, title: String) {
        let start = self.book.blocks.len();
        match self.book.chapters.last_mut() {
            // An empty previous chapter is replaced, not kept.
            Some(last) if last.start == start => last.title = title,
            _ => self.book.chapters.push(Chapter { title, start }),
        }
    }

    pub fn finish(mut self) -> TextBook {
        self.flush(Kind::Normal);
        while matches!(self.book.blocks.last(), Some(Block::Break)) {
            self.book.blocks.pop();
        }
        let n = self.book.blocks.len();
        self.book.chapters.retain(|c| c.start < n);
        if self.book.chapters.first().is_none_or(|c| c.start > 0) {
            self.book.chapters.insert(0, Chapter { title: String::new(), start: 0 });
        }
        self.book
    }
}

#[cfg(test)]
mod tests;
