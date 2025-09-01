//! Cover textures, decoded lazily and capped per frame.

use std::collections::HashMap;

use raft_doc::Book;

use crate::Files;

/// Most covers decoded in one frame (keeps scrolling smooth).
const DECODES_PER_FRAME: usize = 6;
/// Largest accepted image side, in pixels.
const MAX_SIDE: u32 = 8192;
/// Largest accepted encoded image, in bytes.
const MAX_BYTES: usize = 32 * 1024 * 1024;

pub enum Cover {
    Ready(egui::TextureHandle),
    Missing,
}

#[derive(Default)]
pub struct Covers {
    map: HashMap<String, Cover>,
    frame: u64,
    decoded_this_frame: usize,
}

impl Covers {
    /// The cover texture for `book`, decoding it if this frame's budget allows.
    pub fn get(
        &mut self,
        ctx: &egui::Context,
        files: &dyn Files,
        book: &Book,
    ) -> Option<egui::TextureHandle> {
        let frame = ctx.cumulative_frame_nr();
        if frame != self.frame {
            self.frame = frame;
            self.decoded_this_frame = 0;
        }
        if let Some(c) = self.map.get(&book.id) {
            return match c {
                Cover::Ready(t) => Some(t.clone()),
                Cover::Missing => None,
            };
        }
        if self.decoded_this_frame >= DECODES_PER_FRAME {
            ctx.request_repaint();
            return None;
        }
        self.decoded_this_frame += 1;
        let cover = files
            .cover(book)
            .and_then(|bytes| decode_image(&bytes).ok())
            .map_or(Cover::Missing, |img| {
                Cover::Ready(ctx.load_texture(
                    format!("cover-{}", book.id),
                    img,
                    egui::TextureOptions::LINEAR,
                ))
            });
        let tex = match &cover {
            Cover::Ready(t) => Some(t.clone()),
            Cover::Missing => None,
        };
        self.map.insert(book.id.clone(), cover);
        tex
    }
}

/// Decodes an encoded image (WebP, PNG, JPEG) with size limits.
pub fn decode_image(bytes: &[u8]) -> Result<egui::ColorImage, String> {
    if bytes.len() > MAX_BYTES {
        return Err("image is too large".into());
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| e.to_string())?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Ok(egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()))
}
