//! Small image previews decoded on the archive worker, never on the UI thread.

use std::io::Cursor;

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};

const MAX_INPUT_BYTES: usize = 256 * 1024;
const MAX_SIDE: u32 = 2048;
const MAX_PIXELS: u64 = 4 * 1024 * 1024;
const MAX_DECODED_BYTES: u64 = 32 * 1024 * 1024;

/// A known image with invalid/oversized contents remains available as raw hex.
/// The decoder's allocation budget is best effort; dimensions, input length,
/// pixel count and output bytes are checked explicitly before pixel decoding.
pub(super) fn decode(bytes: &[u8]) -> Option<Result<egui::ColorImage, String>> {
    let format = match image::guess_format(bytes).ok()? {
        ImageFormat::Png => ImageFormat::Png,
        ImageFormat::Jpeg => ImageFormat::Jpeg,
        _ => return None,
    };
    Some(decode_image(bytes, format))
}

fn decode_image(bytes: &[u8], format: ImageFormat) -> Result<egui::ColorImage, String> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err("Image preview exceeds the 256 KiB entry limit.".into());
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(64 * 1024 * 1024);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(image_error)?;
    let (width, height) = decoder.dimensions();
    if width == 0
        || height == 0
        || width > MAX_SIDE
        || height > MAX_SIDE
        || u64::from(width) * u64::from(height) > MAX_PIXELS
        || decoder.total_bytes() > MAX_DECODED_BYTES
    {
        return Err("Image preview is limited to 2048 × 2048 pixels.".into());
    }
    let orientation = decoder.orientation().map_err(image_error)?;
    let mut decoded = DynamicImage::from_decoder(decoder).map_err(image_error)?;
    decoded.apply_orientation(orientation);
    let rgba = decoded.into_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        size,
        rgba.as_raw(),
    ))
}

fn image_error(error: image::ImageError) -> String {
    match error {
        image::ImageError::Limits(_) => {
            "Image exceeds the preview dimensions or decoder memory budget.".into()
        }
        _ => "Image data could not be decoded. The verified bytes remain available in Hex.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(width: u32, height: u32, format: ImageFormat) -> Vec<u8> {
        let image = image::RgbImage::from_pixel(width, height, image::Rgb([30, 120, 210]));
        let mut output = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(image)
            .write_to(&mut output, format)
            .expect("encode original test pixels");
        output.into_inner()
    }

    #[test]
    fn decodes_png_pixels_and_jpeg_dimensions() {
        let png = decode(&encoded(3, 2, ImageFormat::Png))
            .expect("PNG recognized")
            .expect("PNG decoded");
        assert_eq!(png.size, [3, 2]);
        assert_eq!(png.pixels, vec![egui::Color32::from_rgb(30, 120, 210); 6]);
        let jpeg = decode(&encoded(3, 2, ImageFormat::Jpeg))
            .expect("JPEG recognized")
            .expect("JPEG decoded");
        assert_eq!(jpeg.size, [3, 2]);
    }

    #[test]
    fn refuses_oversized_dimensions_and_input_without_decoding_pixels() {
        let wide = encoded(MAX_SIDE + 1, 1, ImageFormat::Png);
        assert!(wide.len() < MAX_INPUT_BYTES);
        assert!(decode(&wide).expect("PNG recognized").is_err());
        let mut excess = encoded(1, 1, ImageFormat::Png);
        excess.resize(MAX_INPUT_BYTES + 1, 0);
        assert!(decode(&excess).expect("PNG recognized").is_err());
    }

    #[test]
    fn distinguishes_invalid_image_data_from_non_image_content() {
        assert!(decode(b"ordinary text\n").is_none());
        assert!(
            decode(b"\x89PNG\r\n\x1a\ninvalid")
                .expect("PNG signature")
                .is_err()
        );
    }
}
