use std::cmp::max;

use image::{ImageBuffer, Rgba, ImageResult};

/** Image buffer of type. */
pub type IBoft = ImageBuffer<Rgba<u8>, Vec<u8>>;

pub fn open_image(filename: &str) -> ImageResult<IBoft> {
    Ok(image::open(filename)?.into_rgba8())
}

pub fn resize_as_needed(image: IBoft, target_width: u32, target_height: u32) -> IBoft {
    image::imageops::resize(
        &image,
        target_width,
        target_height,
        image::imageops::FilterType::Nearest,
    )
}

fn resize_images_to_even(image1: IBoft, image2: IBoft) -> (IBoft, IBoft) {
    let (w1, h1) = image1.dimensions();
    let (w2, h2) = image2.dimensions();
    let tw = max(w1, w2);
    let th = max(h1, h2);
    let mut new_image1 = image1;
    let mut new_image2 = image2;

    if w1 < tw || h1 < tw {
        new_image1 = resize_as_needed(new_image1, tw, th);
    }

    if w2 < tw || h2 < tw {
        new_image2 = resize_as_needed(new_image2, tw, th);
    }

    (new_image1, new_image2)
}

// fn open_image_pair_resized(filename1: &str, filename2: &str) -> (IBoft, IBoft) {
//     let image1 = open_image(filename1);
//     let image2 = open_image(filename2);
//     resize_images_to_even(image1, image2)
// }