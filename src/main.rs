mod cli;
#[cfg(feature = "pixel")]
mod main_image;
mod open_image;
mod shared;

use std::{
    io::BufRead,
    process::{exit, Command},
    sync::{Arc, RwLock},
    thread,
};

use clap::Parser;
use cli::Cli;
use crossbeam::{
    channel::{Receiver, Sender},
    select,
};
use image_match::{cosine_similarity, image::get_image_signature};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use shared::{make_groups_and_exec, CompareTask, Pairing};

use crate::open_image::open_image;


struct SignatureToCompare {
    path: String,
    signature: Vec<i8>,
}

fn signature_maker_loop(
    filename_rx: Receiver<String>,
    tx: Sender<SignatureToCompare>,
    calc_count_tx: Sender<u64>,
) {
    let mut total = 0;
    while let Ok(filename) = filename_rx.recv() {
        let image = open_image(&filename);
        match image {
            Ok(image) => {
                let signature = get_image_signature(image);
                tx.send(SignatureToCompare {
                    path: filename,
                    signature,
                })
                .expect("Unable to send signature to channel");

                total += 1;
                if total >= 5 && calc_count_tx.try_send(total).is_ok() {
                    total = 0;
                }
            }
            Err(e) => eprintln!("Unable to open image {}: {}", filename, e),
        }
    }
    calc_count_tx.send(total).ok();
}

/// A vector of image paths and their signatures. Also contains a HashMap of
/// paths to indices in that vector to quickly check for duplicates.
struct SignatureBundle {
    image_map: Vec<SignatureToCompare>,
    // name_map: HashMap<String, usize>,
}

impl SignatureBundle {
    fn new() -> SignatureBundle {
        SignatureBundle {
            image_map: Vec::new(),
            // name_map: HashMap::new(),
        }
    }
}

fn get_bucket_width(threshold: u8) -> f32 {
    if threshold < 10 {
        140.0
    } else if threshold < 20 {
        130.0
    } else if threshold < 30 {
        120.0
    } else if threshold < 40 {
        110.0
    } else if threshold < 50 {
        100.0
    } else if threshold < 60 {
        100.0
    } else if threshold < 70 {
        80.0
    } else if threshold < 80 {
        70.0
    } else if threshold < 90 {
        60.0
    } else if threshold < 95 {
        30.0
    } else {
        25.0
    }
}

fn progress_bar_loop(
    calc_total_rx: Receiver<u64>,
    calc_count_rx: Receiver<u64>,
    compare_count_rx: Receiver<u64>,
) {
    let bars = MultiProgress::new();
    let style = ProgressStyle::with_template("{msg} [{bar:40.cyan/blue}] {pos:>7}/{len:7}")
        .expect("Unable to style the progress bar")
        .progress_chars("#-");

    let calc_bar = bars.add(ProgressBar::new(0));
    calc_bar.set_style(style.clone());
    calc_bar.set_message("Calculating signatures");

    loop {
        select! {
            recv(calc_total_rx) -> msg => if let Ok(msg) = msg {
                let length = calc_bar.length().unwrap_or(0) + msg;
                calc_bar.set_length(length);
            },
            recv(calc_count_rx) -> msg => if let Ok(msg) = msg {
                calc_bar.inc(msg);
            },
            recv(compare_count_rx) -> msg => match msg {
                Ok(_) => (),
                Err(_) => break,
            },
        }
    }

    bars.clear().ok();
}

const FILENAME_CHANNEL_BOUND: usize = 65535;
const CHANNEL_BOUND: usize = 2048;

fn main_signatures(cli: Cli) {
    let cpu_count = num_cpus::get();

    let threshold_u8: u8 = cli.threshold.unwrap_or(90);
    let threshold: f64 = cli.threshold.unwrap_or(90).into();
    let threshold = threshold * 0.01;

    let (filename_tx, filename_rx) = crossbeam::channel::bounded(FILENAME_CHANNEL_BOUND);

    let mut dash_mode = false;

    if cli.files.is_empty() {
        eprintln!("No files provided");
        exit(1);
    }

    let (calc_total_tx, calc_total_rx) = crossbeam::channel::bounded(2);
    let (calc_count_tx, calc_count_rx) = crossbeam::channel::bounded(CHANNEL_BOUND);
    let (compare_count_tx, compare_count_rx) = crossbeam::channel::bounded(CHANNEL_BOUND);

    if !cli.pairs {
        thread::spawn(move || {
            progress_bar_loop(calc_total_rx, calc_count_rx, compare_count_rx);
        });
    }

    let mut total_filenames = 0;

    for arg_filename in &cli.files {
        if arg_filename == "-" {
            dash_mode = true;
        } else {
            filename_tx
                .send(arg_filename.clone())
                .expect("Unable to send filename to channel");
            total_filenames += 1;
        }
    }

    calc_total_tx.try_send(total_filenames).ok();

    if dash_mode {
        thread::spawn(move || {
            let mut total = 0;
            let mut stdin_lock = std::io::stdin().lock();
            let mut buf = String::new();
            loop {
                match stdin_lock.read_line(&mut buf) {
                    Ok(len) => {
                        if len == 0 {
                            break;
                        }
                        let slice = buf[0..(len - 1)].to_owned();
                        filename_tx
                            .send(slice)
                            .expect("Unable to send filename to channel");
                        total += 1;
                    }
                    Err(err) => {
                        eprintln!("{}", err);
                        break;
                    }
                }
                buf.clear();
            }
            calc_total_tx.try_send(total).ok();
        });
    } else {
        drop(filename_tx);
        drop(calc_total_tx);
    }

    let bundle = SignatureBundle::new();
    let bundle = Arc::new(RwLock::new(bundle));

    let (img_tx, img_rx) = crossbeam::channel::bounded(CHANNEL_BOUND);

    let mut image_maker_threads = Vec::new();

    for _ in 0..cpu_count {
        let filename_rx = filename_rx.clone();
        let img_tx = img_tx.clone();
        let calc_count_tx = calc_count_tx.clone();
        image_maker_threads.push(thread::spawn(move || {
            signature_maker_loop(filename_rx, img_tx, calc_count_tx);
        }));
    }

    drop(img_tx);
    drop(calc_count_tx);

    // Image task channel
    let (task_tx, task_rx) = crossbeam::channel::bounded(CHANNEL_BOUND);

    use lsh_rs2::prelude::*;
    let bucket_width = get_bucket_width(threshold_u8);
    // println!("Bucket width is {}", bucket_width);
    let n_projections = 5;
    let n_hash_tables = 20;
    let dim = 625;
    let mut lsh = LshMem::<_, f32>::new(n_projections, n_hash_tables, dim)
        .seed(4001)
        .only_index()
        .l2(bucket_width)
        .expect("Unable to set up LSH");

    let bundle_arc = bundle.clone();
    thread::spawn(move || {
        while let Ok(image) = img_rx.recv() {
            let signature: Vec<_> = image.signature.iter().map(|v| *v as f32).collect();
            // println!("{:?}, {}", signature, signature.len());
            let results = lsh
                .query_bucket_ids(&signature)
                .expect("Unable to query bucket");
            let index1 = lsh
                .store_vec(&signature)
                .expect("Unable to store signature") as u32;

            let mut guard = bundle_arc.write().expect("Unable to write to bundle");
            guard.image_map.push(image);
            drop(guard);

            for index2 in results {
                task_tx
                    .send(CompareTask {
                        index1: index1 as usize,
                        index2: index2 as usize,
                    })
                    .expect("Unable to send task");
            }

            // println!("{}", index1);
        }
    });
    // drop(img_rx);
    // println!("LSH-accelerated Task master thread done");

    // let mut query_point = vec![0.0; dim];
    // let neighbors = lsh.query(&query_point, 10).unwrap();
    // println!("{:?}", neighbors);

    // Image pairing channel
    let (pair_tx, pair_rx) = crossbeam::channel::bounded(CHANNEL_BOUND);

    let mut compare_threads = Vec::new();

    for _ in 0..cpu_count {
        let task_rx = task_rx.clone();
        let bundle = bundle.clone();
        let pair_tx = pair_tx.clone();
        compare_threads.push(thread::spawn(move || {
            // let mut total = 0;
            while let Ok(task) = task_rx.recv() {
                match bundle.read() {
                    Ok(bundle) => {
                        let image1 = &bundle.image_map[task.index1];
                        let image2 = &bundle.image_map[task.index2];

                        let result = cosine_similarity(&image1.signature, &image2.signature);

                        let pairing = Pairing {
                            index1: task.index1,
                            index2: task.index2,
                            score: result,
                        };

                        if pairing.score > threshold {
                            pair_tx
                                .send(pairing)
                                .expect("Unable to send pairing over channel");
                        }
                        // total += 1;
                    }
                    Err(e) => panic!("{}", e),
                }
            }
            // println!("Compared {} pairs", total);
        }));
    }

    drop(pair_tx);

    let mut pairings = Vec::new();

    let executable = {
        cli.exec.as_ref().map(|exec| {
            let mut split = exec.split(" ");
            let command = split.next().expect("Command for exec not provided");
            let rest: Vec<&str> = split.collect();
            (command, rest)
        })
    };

    // If we use pairs, we execute for each pair right away.
    while let Ok(pair) = pair_rx.recv() {
        if cli.pairs {
            let bundle = bundle
                .read()
                .expect("Unable to read image bundle for pairs");
            let filename1 = bundle.image_map[pair.index1].path.clone();
            let filename2 = bundle.image_map[pair.index2].path.clone();
            println!("{} {}", filename1, filename2);
            if let Some((program, args)) = &executable {
                Command::new(program)
                    .args(args)
                    .arg(filename1)
                    .arg(filename2)
                    .output()
                    .expect("Unable to run executable provided");
            }
        }
        pairings.push(pair);
    }

    // println!("Reaching joins");
    drop(compare_count_tx);
    for thread in image_maker_threads {
        thread.join().unwrap();
    }
    // println!("Image maker threads joined");

    for thread in compare_threads {
        thread.join().unwrap();
    }
    // eprintln!("Compare threads done");

    let bundle = bundle
        .read()
        .expect("Unable to read image bundle after all threads have completed.");

    let image_map: Vec<String> = bundle.image_map.iter().map(|s| s.path.clone()).collect();

    if !cli.pairs {
        make_groups_and_exec(&image_map, pairings, &executable);
    }
}

#[cfg(feature = "pixel")]
fn main() {
    let cli = Cli::parse();
    if cli.pixels {
        main_image::main_images(cli);
    } else {
        main_signatures(cli);
    }
}

#[cfg(not(feature = "pixel"))]
fn main() {
    main_signatures(Cli::parse());
}
