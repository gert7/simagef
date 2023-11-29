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
use shared::{CompareTask, Pairing, run_executable};

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

    let image_map: HashMap<String, SignatureToCompare> = HashMap::new();
    let image_map = Arc::new(RwLock::new(image_map));
    // let mut image_pairs = Vec::new();

    let (img_tx, img_rx) = channel();

    let mut image_maker_threads = Vec::new();

    for _ in 0..cpu_count {
        let filename_rx = filename_rx.clone();
        let tx = img_tx.clone();
        image_maker_threads.push(thread::spawn(move || {
            signature_maker_loop(filename_rx, tx);
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
                            if image_map.contains_key(image.path()) {
                                eprintln!("Image already in list: {}", image.path());
                                continue;
                            }
                            let filename1 = image.path().to_owned();
                            for (filename2, _) in image_map.iter() {
                                // println!("Image task created for {} {}", filename1, filename2);
                                task_tx
                                    .send(CompareTask {
                                        filename1: filename1.clone(),
                                        filename2: filename2.clone(),
                                    })
                                    .unwrap();
                            }
                            image_map.insert(image.path().to_owned(), image);
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

    for _ in 0..1 {
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

                        let result = cosine_similarity(&image1.signature, &image2.signature);

                        let pairing = Pairing {
                            filename1: task.filename1.clone(),
                            filename2: task.filename2.clone(),
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

    loop {
        match pair_rx.recv() {
            Ok(pair) => {
                if cli.pairs {
                    println!("{} {}", pair.filename1, pair.filename2);
                    if let Some((program, args)) = &executable {
                        Command::new(program)
                            .args(args)
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

    if !cli.pairs {
        run_executable(&pairings, &executable);
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

// fn main() {
//     let pairs = vec![("A", "B"), ("B", "C"), ("D", "E"), ("D", "F"), ("D", "G")];
//     let result = make_groups(&pairs);
//     println!("{:?}", result);
// }
