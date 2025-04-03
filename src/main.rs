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
#[cfg(feature = "pixel")]
use main_image::main_images;
use open_image::SingleImage;
use shared::{make_groups_and_exec, CompareTask, Pairing};

use crate::open_image::open_image;

mod cli;
#[cfg(feature = "pixel")]
mod main_image;
mod open_image;
mod shared;

struct SignatureToCompare {
    path: String,
    signature: Vec<i8>,
}

impl<'a> SingleImage<'a, Vec<i8>> for SignatureToCompare {
    fn path(&'a self) -> &'a str {
        &self.path
    }

    fn content(&self) -> &Vec<i8> {
        &self.signature
    }
}

fn signature_maker_loop(
    filename_rx: Receiver<String>,
    tx: Sender<SignatureToCompare>,
    calc_count_tx: Sender<u64>,
) {
    let mut total = 0;
    loop {
        match filename_rx.recv() {
            Ok(filename) => {
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
                        if total >= 5 {
                            if let Ok(_) = calc_count_tx.try_send(total) {
                                total = 0;
                            }
                        }
                    }
                    Err(e) => eprintln!("Unable to open image {}: {}", filename, e),
                }
            }
            Err(_) => {
                break;
            }
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

fn calc_pair_count(n: u64) -> u64 {
    (n - 1) * n / 2
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

    let comp_bar = bars.add(ProgressBar::new(0));
    comp_bar.set_style(style);
    comp_bar.set_message("Comparing images      ");

    loop {
        select! {
            recv(calc_total_rx) -> msg => if let Ok(msg) = msg {
                let length = calc_bar.length().unwrap_or(0) + msg;
                calc_bar.set_length(length);
                comp_bar.set_length(calc_pair_count(length));
            },
            recv(calc_count_rx) -> msg => if let Ok(msg) = msg {
                calc_bar.inc(msg);
            },
            recv(compare_count_rx) -> msg => match msg {
                Ok(msg) => comp_bar.inc(msg),
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

    let threshold: f64 = cli.threshold.unwrap_or(90).into();
    let threshold = threshold * 0.01;

    let (filename_tx, filename_rx) = crossbeam::channel::bounded(FILENAME_CHANNEL_BOUND);

    let mut dash_mode = false;

    if cli.files.len() == 0 {
        eprintln!("No files provided");
        exit(1);
    }

    let (calc_total_tx, calc_total_rx) = crossbeam::channel::bounded(2);
    let (calc_count_tx, calc_count_rx) = crossbeam::channel::bounded(CHANNEL_BOUND);
    let (compare_count_tx, compare_count_rx) = crossbeam::channel::bounded(CHANNEL_BOUND);

    thread::spawn(move || {
        progress_bar_loop(calc_total_rx, calc_count_rx, compare_count_rx);
    });

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

    calc_total_tx.send(total_filenames).ok();

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
            calc_total_tx.send(total).ok();
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

    let bundle_arc = bundle.clone();
    let task_master_thread = thread::spawn(move || {
        // let mut total = 0;
        loop {
            match img_rx.recv() {
                Ok(image) => {
                    match bundle_arc.write() {
                        Ok(mut bundle) => {
                            let index1 = bundle.image_map.len(); // 3

                            let to_send: Vec<_> = bundle
                                .image_map
                                .iter()
                                .enumerate()
                                .map(|(index2, _)| CompareTask { index1, index2 })
                                .collect();
                            // total += to_send.len() as u64;
                            bundle.image_map.push(image);

                            drop(bundle);

                            for task in to_send {
                                task_tx.send(task).expect("Unable to send to task channel!");
                            }

                            // if total >= 10000 {
                            //     compare_total_tx.send(CompareTotal::Increment(total)).ok();
                            //     total = 0;
                            // }
                        }
                        Err(err) => panic!("Unable to lock image map: {}", err),
                    };
                }
                Err(_) => {
                    break;
                }
            }
        }
    });

    // Image pairing channel
    let (pair_tx, pair_rx) = crossbeam::channel::bounded(CHANNEL_BOUND);

    let mut compare_threads = Vec::new();

    for _ in 0..cpu_count {
        let task_rx = task_rx.clone();
        let bundle = bundle.clone();
        let pair_tx = pair_tx.clone();
        let compare_count_tx = compare_count_tx.clone();
        compare_threads.push(thread::spawn(move || {
            let mut total = 0;
            loop {
                match task_rx.recv() {
                    Ok(task) => match bundle.read() {
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
                            total += 1;
                            if total >= 10000 {
                                if let Ok(_) = compare_count_tx.try_send(total) {
                                    total = 0;
                                }
                            }
                        }
                        Err(e) => panic!("{}", e),
                    },
                    Err(_) => {
                        break;
                    }
                }
            }
        }));
    }

    drop(pair_tx);
    drop(compare_count_tx);

    let mut pairings = Vec::new();

    let executable = {
        cli.exec.as_ref().map(|exec| {
            let mut split = exec.split(" ").into_iter();
            let command = split.next().expect("Command for exec not provided");
            let rest: Vec<&str> = split.collect();
            (command, rest)
        })
    };

    // If we use pairs, we execute for each pair right away.
    loop {
        match pair_rx.recv() {
            Ok(pair) => {
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
            Err(_) => break,
        }
    }

    for thread in image_maker_threads {
        thread.join().unwrap();
    }

    task_master_thread.join().unwrap();

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
        main_images(cli);
    } else {
        main_signatures(cli);
    }
}

#[cfg(not(feature = "pixel"))]
fn main() {
    main_signatures(Cli::parse());
}
