use std::{
    collections::HashMap,
    io::BufRead,
    process::{exit, Command},
    sync::{mpsc::channel, Arc, RwLock},
    thread,
};

use clap::Parser;
use cli::Cli;
use crossbeam::channel::Receiver;
use image_match::{cosine_similarity, image::get_image_signature};
use main_image::main_images;
use open_image::SingleImage;
use shared::{make_groups_and_exec, CompareTask, Pairing};

use crate::open_image::open_image;

mod cli;
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

    fn content(&'a self) -> &Vec<i8> {
        &self.signature
    }
}

fn signature_maker_loop(
    filename_rx: Receiver<String>,
    tx: std::sync::mpsc::Sender<SignatureToCompare>,
) {
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
                    }
                    Err(e) => eprintln!("Unable to open image {}: {}", filename, e),
                }
            }
            Err(_) => {
                // println!("Image maker thread done");
                drop(filename_rx);
                return;
            }
        }
    }
}

/// A vector of image paths and their signatures. Also contains a HashMap of
/// paths to indices in that vector to quickly check for duplicates.
struct SignatureBundle {
    image_map: Vec<SignatureToCompare>,
    name_map: HashMap<String, usize>,
}

impl SignatureBundle {
    fn new() -> SignatureBundle {
        SignatureBundle {
            image_map: Vec::new(),
            name_map: HashMap::new(),
        }
    }
}

fn main_signatures(cli: Cli) {
    let cpu_count = num_cpus::get();

    let threshold: f64 = cli.threshold.unwrap_or(90).into();
    let threshold = threshold * 0.01;

    let (filename_tx, filename_rx) = crossbeam::channel::unbounded();

    let mut dash_mode = false;

    if cli.files.len() == 0 {
        eprintln!("No files provided");
        exit(1);
    }

    for arg_filename in &cli.files {
        if arg_filename == "-" {
            dash_mode = true;
        } else {
            filename_tx
                .send(arg_filename.clone())
                .expect("Unable to send filename to channel");
        }
    }

    if dash_mode {
        thread::spawn(move || {
            let mut stdin_lock = std::io::stdin().lock();
            let mut buf = String::new();
            loop {
                match stdin_lock.read_line(&mut buf) {
                    Ok(len) => {
                        if len == 0 {
                            return;
                        }
                        let slice = buf[0..(len - 1)].to_owned();
                        filename_tx
                            .send(slice)
                            .expect("Unable to send filename to channel");
                    }
                    Err(err) => {
                        eprintln!("{}", err);
                        return;
                    }
                }
                buf.clear();
            }
        });
    } else {
        drop(filename_tx);
    }

    // let image_map: Vec<SignatureToCompare> = Vec::new();
    // let image_map = Arc::new(RwLock::new(image_map));
    // let name_map: HashMap<String, usize> = HashMap::new();
    // let mut image_pairs = Vec::new();
    let bundle = SignatureBundle::new();
    let bundle = Arc::new(RwLock::new(bundle));

    let (img_tx, img_rx) = channel();

    let mut image_maker_threads = Vec::new();

    for _ in 0..cpu_count {
        let filename_rx = filename_rx.clone();
        let img_tx = img_tx.clone();
        image_maker_threads.push(thread::spawn(move || {
            signature_maker_loop(filename_rx, img_tx);
        }));
    }

    drop(img_tx);

    // Image task channel
    let (task_tx, task_rx) = crossbeam::channel::unbounded();

    let bundle_arc = bundle.clone();
    let task_master_thread = thread::spawn(move || {
        loop {
            match img_rx.recv() {
                Ok(image) => {
                    match bundle_arc.write() {
                        Ok(mut bundle) => {
                            if bundle.name_map.contains_key(image.path()) {
                                eprintln!("Image already in list: {}", image.path());
                                continue;
                            }

                            let index1 = bundle.image_map.len(); // 3

                            for (index2, _) in bundle.image_map.iter().enumerate() {
                                task_tx
                                    .send(CompareTask { index1, index2 })
                                    .expect("Unable to send to task channel!");
                                // println!("Image task created for {} {}", index1, index2);
                            }

                            bundle.name_map.insert(image.path.clone(), index1);
                            bundle.image_map.push(image);
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
        let bundle = bundle.clone();
        let pair_tx = pair_tx.clone();
        compare_threads.push(thread::spawn(move || loop {
            match task_rx.recv() {
                Ok(task) => match bundle.read() {
                    Ok(bundle) => {
                        let image1 = &bundle.image_map[task.index1];
                        let image2 = &bundle.image_map[task.index2];
                        // println!("Doing task {} {}", image1.path, image2.path);

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

    // eprintln!("");

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

    // eprintln!("Image maker threads done");

    task_master_thread.join().unwrap();
    // eprintln!("Task master thread done");

    for thread in compare_threads {
        thread.join().unwrap();
    }
    // eprintln!("Compare threads done");

    let bundle = bundle
        .read()
        .expect("Unable to read image bundle after all threads have completed.");

    let name_map: Vec<String> = bundle.image_map.iter().map(|s| s.path.clone()).collect();

    if !cli.pairs {
        println!("make_groups_and_exec");
        make_groups_and_exec(&name_map, &pairings, &executable);
    }
}

fn main() {
    let cli = Cli::parse();
    if cli.pixels {
        main_images(cli);
    } else {
        main_signatures(cli);
    }
}
