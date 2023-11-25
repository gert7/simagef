use std::{
    collections::HashMap,
    sync::{mpsc::channel, Arc, RwLock},
    thread, io::BufRead,
};

use clap::Parser;
use image::Rgb;
use open_image::IBoft;

use crate::open_image::{open_image, resize_as_needed};

mod open_image;

#[derive(Parser)]
#[command(version, about, long_about = None)]
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
    score: f64,
}

#[derive(Debug)]
struct CompareTask {
    filename1: String,
    filename2: String,
}

struct ImageToCompare {
    path: String,
    image: IBoft,
}

fn main() {
    let cpu_count = num_cpus::get();

    let white = Rgb([255, 255, 255]);

    let cli = Cli::parse();

    let threshold: f64 = cli.threshold.unwrap_or(90).into();
    let threshold = threshold * 0.01;

    let (filename_tx, filename_rx) = crossbeam::channel::unbounded();

    let mut dash_mode = false;

    for arg_filename in cli.files {
        if arg_filename == "-" {
            dash_mode = true;
        } else {
            filename_tx.send(arg_filename)
                .expect("Unable to send filename to channel");
        }
    }

    if dash_mode {
        thread::spawn(move || {
            let mut stdin_lock= std::io::stdin().lock();
            let mut buf = String::new();
            loop {
                match stdin_lock.read_line(&mut buf) {
                    Ok(len) => {
                        if len == 0 {
                            break;
                        }
                        let slice = buf[0..(len - 1)].to_owned();
                        filename_tx.send(slice).expect("Unable to send filename to channel");
                    },
                    Err(err) => {
                        eprintln!("{}", err);
                        return;
                    },
                }
                buf.clear();
            }
        });
    }

    let image_map: HashMap<String, ImageToCompare> = HashMap::new();
    let image_map = Arc::new(RwLock::new(image_map));
    // let mut image_pairs = Vec::new();

    let (img_tx, img_rx) = channel();

    let mut image_maker_threads = Vec::new();

    for _ in 0..cpu_count {
        let filename_rx = filename_rx.clone();
        let tx = img_tx.clone();
        image_maker_threads.push(thread::spawn(move || loop {
            match filename_rx.recv() {
                Ok(filename) => {
                    let image = open_image(&filename);
                    match image {
                        Ok(image) => {
                            let image = resize_as_needed(
                                image,
                                cli.fingerprint_width,
                                cli.fingerprint_height,
                            );
                            tx.send(ImageToCompare {
                                path: filename,
                                image,
                            })
                            .expect("Unable to send image to channel");
                        }
                        Err(e) => eprintln!("Unable to open image {}: {}", filename, e),
                    }
                }
                Err(_) => {
                    // println!("Image maker thread done");
                    drop(filename_rx);
                    return;
                },
            }
        }));
    }

    drop(img_tx);

    // Image task channel
    let (task_tx, task_rx) = crossbeam::channel::unbounded();

    let image_map_arc = image_map.clone();
    let task_master_thread = thread::spawn(move || {
        loop {
            match img_rx.recv() {
                Ok(image) => {
                    match image_map_arc.write() {
                        Ok(mut image_map) => {
                            if image_map.contains_key(&image.path) {
                                eprintln!("Image already in list: {}", image.path);
                                continue;
                            }
                            let filename1 = image.path.clone();
                            for (filename2, _) in image_map.iter() {
                                // println!("Image task created for {} {}", filename1, filename2);
                                task_tx
                                    .send(CompareTask {
                                        filename1: filename1.clone(),
                                        filename2: filename2.clone(),
                                    })
                                    .unwrap();
                            }
                            image_map.insert(image.path.clone(), image);
                        }
                        Err(err) => panic!("Unable to lock image map: {}", err),
                    };
                }
                Err(_) => {
                    // println!("Task master complete");
                    drop(img_rx);
                    return;
                }
            }
        }
    });

    // Image pairing channel
    let (pair_tx, pair_rx) = crossbeam::channel::unbounded();

    let mut compare_threads = Vec::new();

    for _ in 0..cpu_count {
        let task_rx = task_rx.clone();
        let image_map = image_map.clone();
        let pair_tx = pair_tx.clone();
        compare_threads.push(thread::spawn(move || loop {
            match task_rx.recv() {
                Ok(task) => match image_map.read() {
                    Ok(image_map) => {
                        let image1 = &image_map[&task.filename1];
                        let image2 = &image_map[&task.filename2];
                        // println!("Doing task {} {}", image1.path, image2.path);

                        let result = image_compare::rgba_blended_hybrid_compare(
                            (&image1.image).into(),
                            (&image2.image).into(),
                            white,
                        )
                        .expect("Resized images have different dimensions somehow.");

                        let pairing = Pairing {
                            filename1: task.filename1.clone(),
                            filename2: task.filename2.clone(),
                            score: result.score,
                        };

                        if pairing.score > threshold {
                            pair_tx
                                .send(pairing)
                                .expect("Unable to send pairing over channel");
                        }
                    }
                    Err(e) => panic!("{}", e),
                },
                Err(_) => {
                    // println!("Finished compare thread");
                    drop(task_rx);
                    return;
                }
            }
        }));
    }

    drop(pair_tx);

    // println!("Dropped pair_tx");

    loop {
        match pair_rx.recv() {
            Ok(pair) => println!("{} {}", pair.filename1, pair.filename2),
            Err(_) => break,
        }
    }

    for thread in image_maker_threads {
        thread.join().unwrap();
    }

    // println!("Image maker threads done");

    task_master_thread.join().unwrap();
    // println!("Task master thread done");

    for thread in compare_threads {
        thread.join().unwrap();
    }
    // eprintln!("Compare threads done");
}