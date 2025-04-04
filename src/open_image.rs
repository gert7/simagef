use image::{ImageBuffer, Rgba, ImageResult};

/** Image buffer of type. */
pub type IBoft = ImageBuffer<Rgba<u8>, Vec<u8>>;

#[cfg(feature = "pixel")]
pub trait SingleImage<T> {
    fn path(&self) -> &str;
}

pub fn open_image(filename: &str) -> ImageResult<IBoft> {
    Ok(image::open(filename)?.into_rgba8())
}

#[cfg(feature = "pixel")]
pub fn resize_as_needed(image: IBoft, target_width: u32, target_height: u32) -> IBoft {
    image::imageops::resize(
        &image,
        target_width,
        target_height,
        image::imageops::FilterType::Nearest,
    )
}
