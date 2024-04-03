use std::{process::{exit, Command}, thread, io::BufRead, collections::HashMap, sync::{Arc, RwLock, mpsc::channel}};

use crossbeam::channel::Receiver;
use image::Rgb;

use crate::{cli::Cli, open_image::{open_image, resize_as_needed, IBoft, SingleImage}, shared::{CompareTask, Pairing, make_groups_and_exec}};
struct ImageToCompare {
    path: String,
    image: IBoft,
}

impl<'a> SingleImage<'a, IBoft> for ImageToCompare {
    fn path(&'a self) -> &'a str {
        &self.path
    }

    fn content(&'a self) -> &IBoft {
        &self.image
    }
}

fn image_maker_loop(
    filename_rx: Receiver<String>,
    tx: std::sync::mpsc::Sender<ImageToCompare>,
    width: u32,
    height: u32,
) {
    loop {
        match filename_rx.recv() {
            Ok(filename) => {
                let image = open_image(&filename);
                match image {
                    Ok(image) => {
                        let image = resize_as_needed(image, width, height);
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
            }
        }
    }
}

struct ImageBundle {
    image_map: Vec<ImageToCompare>,
    name_map: HashMap<String, usize>,
}

impl ImageBundle {
    fn new() -> ImageBundle {
        ImageBundle {
            image_map: Vec::new(),
            name_map: HashMap::new(),
        }
    }
}

pub fn main_images(cli: Cli) {
    let cpu_count = num_cpus::get();

    let white = Rgb([255, 255, 255]);

    let threshold: f64 = cli.threshold.unwrap_or(90).into();
    let threshold = threshold * 0.01;

    let (filename_tx, filename_rx) = crossbeam::channel::unbounded();

    let mut dash_mode = false;

    if cli.files.len() == 0 {
        eprintln!("No files provided");
        exit(1);
    }

    for arg_filename in cli.files {
        if arg_filename == "-" {
            dash_mode = true;
        } else {
            filename_tx
                .send(arg_filename)
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

    // let image_map: HashMap<String, ImageToCompare> = HashMap::new();
    // let image_map = Arc::new(RwLock::new(image_map));
    // let mut image_pairs = Vec::new();
    let bundle = ImageBundle::new();
    let bundle = Arc::new(RwLock::new(bundle));

    let (img_tx, img_rx) = channel();

    let mut image_maker_threads = Vec::new();

    for _ in 0..cpu_count {
        let filename_rx = filename_rx.clone();
        let tx = img_tx.clone();
        image_maker_threads.push(thread::spawn(move || {
            image_maker_loop(filename_rx, tx, cli.width, cli.height);
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
                                task_tx.send(
                                    CompareTask {
                                        index1,
                                        index2,
                                    }
                                ).expect("Unable to send to task channel!");
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

                        let result = image_compare::rgba_blended_hybrid_compare(
                            (&image1.image).into(),
                            (&image2.image).into(),
                            white,
                        )
                        .expect("Resized images have different dimensions somehow.");

                        let pairing = Pairing {
                            index1: task.index1,
                            index2: task.index2,
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

    let name_map: Vec<String> = bundle.image_map.iter().map(|s| {
        s.path.clone()
    }).collect();

    if !cli.pairs {
        make_groups_and_exec(&name_map, pairings, &executable);
    }
}