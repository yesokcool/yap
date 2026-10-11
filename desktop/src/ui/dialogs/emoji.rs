//! Adding a custom emoji: the picture, shrunk like the web does, and a name.

use super::*;

pub struct EmojiUpload {
    pub(super) name: String,
    pub(super) bytes: Vec<u8>,
    pub(super) preview: Option<egui::TextureHandle>,
    pub(super) error: Option<String>,
}

impl EmojiUpload {
    pub fn new(ctx: &egui::Context, bytes: Vec<u8>, filename: &str) -> Self {
        let mut name: String = filename.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        name = name.trim_matches('_').chars().take(32).collect();
        if name.len() < 2 {
            name = format!("emoji_{name}");
        }
        let (bytes, preview, error) = match prepare_emoji(&bytes) {
            Ok((out, img)) => {
                let tex = ctx.load_texture("emoji-upload", img, egui::TextureOptions::LINEAR);
                (out, Some(tex), None)
            }
            Err(e) => (bytes, None, Some(e)),
        };
        Self { name, bytes, preview, error }
    }
}

/// Like the web client: animated GIF/WebP stay as they are (≤ 512 KB);
/// static images are shrunk to 128 px and sent as PNG.
pub(super) fn prepare_emoji(bytes: &[u8]) -> Result<(Vec<u8>, egui::ColorImage), String> {
    let format = image::guess_format(bytes).map_err(|_| "Use a PNG, GIF, WebP or JPEG.".to_string())?;
    let img = image::load_from_memory(bytes).map_err(|_| "Not an image Yap can read (use PNG, GIF, WebP or JPEG).".to_string())?;
    let preview = {
        let small = img.thumbnail(96, 96).to_rgba8();
        egui::ColorImage::from_rgba_unmultiplied([small.width() as usize, small.height() as usize], small.as_raw())
    };
    let animated = matches!(format, image::ImageFormat::Gif | image::ImageFormat::WebP);
    if animated {
        if bytes.len() > 512 * 1024 {
            return Err("Animated emojis must be under 512 KB.".into());
        }
        return Ok((bytes.to_vec(), preview));
    }
    if bytes.len() <= 512 * 1024 && img.width().max(img.height()) <= 128 && format == image::ImageFormat::Png {
        return Ok((bytes.to_vec(), preview));
    }
    let small = img.thumbnail(128, 128);
    let mut out = std::io::Cursor::new(Vec::new());
    small.write_to(&mut out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    let out = out.into_inner();
    if out.len() > 512 * 1024 {
        return Err("Too big even after shrinking.".into());
    }
    Ok((out, preview))
}
