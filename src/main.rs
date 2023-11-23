use std::collections::{HashMap, VecDeque};

use clap::Parser;
use image::Rgb;
use open_image::IBoft;
use rayon::iter::IntoParallelRefIterator;

use crate::open_image::{open_image, resize_as_needed};

mod open_image;

#[derive(Parser)]
#[command(author = "Gert Oja", version, about, long_about = None)]
struct Cli {
    /// The amount of similarity as a percentage to be considered similar
    #[arg(short, long, value_parser = clap::value_parser!(u8).range(0..=100))]
    threshold: Option<u8>,
    /// The program to launch when the comparisons are finished.
    /// The program will be launched for each pair or grouping.
    #[arg(long)]
    program: Option<String>,
    /// Only present the matched images in pairs rather than groups.
    #[arg(short, long, default_value_t = false)]
    pairs: bool,
    #[arg(long, default_value_t = 160)]
    fingerprint_width: u32,
    #[arg(long, default_value_t = 160)]
    fingerprint_height: u32,
    files: Vec<String>,
}

#[derive(Debug)]
struct Pairing {
    filename1: String,
    filename2: String,
    result: f64,
}

fn main() {
    // let filename1 = "Papsid1.jpg";
    // let filename2 = "Papsid2.jpg";
    // let (image1, image2) = open_image_pair_resized(filename1, filename2);
    let white = Rgb([255, 255, 255]);
    // let result =
    //     image_compare::rgba_blended_hybrid_compare((&image1).into(), (&image2).into(), white)
    //         .expect("Images have different dimensions");
    // println!("{}", result.score);

    let cli = Cli::parse();

    let threshold = cli.threshold.unwrap_or(90);

    let mut filenames = VecDeque::new();

    let mut dash_mode = false;

    for arg_filename in cli.files {
        if arg_filename == "-" {
            dash_mode = true;
        } else {
            filenames.push_back(arg_filename);
        }
    }

    let mut image_map: HashMap<String, IBoft> = HashMap::new();
    let mut image_pairs = Vec::new();

    for filename1 in filenames {
        // println!("{}", filename1);

        let image = open_image(&filename1);
        match image {
            Ok(image) => {
                let image = resize_as_needed(image, cli.fingerprint_width, cli.fingerprint_height);
                for (filename2, image2) in image_map.iter() {
                    let result = image_compare::rgba_blended_hybrid_compare(
                        (&image).into(),
                        (image2).into(),
                        white,
                    )
                    .expect("Images have different dimensions somehow.");
                    image_pairs.push(Pairing {
                        filename1: filename1.clone(),
                        filename2: filename2.clone(),
                        result: result.score,
                    });
                }
                image_map.insert(filename1.clone(), image);
            }
            Err(e) => {
                eprintln!("Failed to open file {}: {}", filename1, e);
            }
        }
    }

    for p in image_pairs {
        println!("{:?}", p);
    }
}
