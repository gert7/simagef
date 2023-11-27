use std::{
    collections::{HashMap, HashSet},
    io::BufRead,
    process::{exit, Command},
    sync::{mpsc::channel, Arc, RwLock},
    thread,
};

use clap::Parser;
use crossbeam::channel::Receiver;
use image::Rgb;
use image_match::{image::get_image_signature, cosine_similarity};
use open_image::IBoft;

use crate::open_image::{open_image, resize_as_needed};

mod open_image;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    /// Compare images by shrinking them to identical sizes and comparing the pixel values, instead of signatures.
    #[arg(short('m'), long, default_value_t = false)]
    pixels: bool,
    /// The amount of similarity as a percentage to be considered similar.
    #[arg(short, long, value_parser = clap::value_parser!(u8).range(0..=100))]
    threshold: Option<u8>,
    /// The program to launch when the comparisons are finished.
    /// The program will be launched for each pair or grouping, one after another.
    #[arg(short('e'), long)]
    exec: Option<String>,
    /// If set, will only present the matched images in pairs rather than groups.
    #[arg(short('p'), long, default_value_t = false)]
    pairs: bool,
    /// The width to resize the images to before comparing in pixel mode.
    #[arg(long, default_value_t = 160)]
    width: u32,
    /// The height to resize the images to before comparing in pixel mode.
    #[arg(long, default_value_t = 160)]
    height: u32,
    /// The files to compare. If one of these is a dash '-' the program will
    /// also read filenames from stdin.
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

trait SingleImage<'a, T> {
    fn path(&'a self) -> &'a str;

    fn content(&'a self) -> &T;
}

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

fn make_groups<'a>(pairs: &Vec<Pairing>) -> Vec<Vec<String>> {
    let mut graph: HashMap<&str, Vec<&str>> = HashMap::new();

    // Build the graph
    for pair in pairs.iter() {
        // let (node1, node2) = *pair;
        let node1 = &pair.filename1;
        let node2 = &pair.filename2;
        graph.entry(node1).or_insert(vec![]).push(node2);
        graph.entry(node2).or_insert(vec![]).push(node1);
    }

    let mut visited: HashSet<&str> = HashSet::new();
    let mut groups: Vec<Vec<String>> = Vec::new();

    // Perform DFS to find connected components
    fn dfs<'a>(
        node: &'a str,
        graph: &'a HashMap<&str, Vec<&str>>,
        visited: &mut HashSet<&'a str>,
        mut group: Vec<String>,
    ) -> Vec<String> {
        visited.insert(node);
        group.push(node.to_string());
        if let Some(neighbors) = graph.get(node) {
            for &neighbor in neighbors.iter() {
                if !visited.contains(neighbor) {
                    group = dfs(neighbor, graph, visited, group);
                }
            }
        }
        group
    }

    for node in graph.keys() {
        if !visited.contains(node) {
            let mut group: Vec<String> = Vec::new();
            group = dfs(node, &graph, &mut visited, group);
            groups.push(group);
        }
    }

    groups
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

fn main_images(cli: Cli) {
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

    let image_map: HashMap<String, ImageToCompare> = HashMap::new();
    let image_map = Arc::new(RwLock::new(image_map));
    // let mut image_pairs = Vec::new();

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

    // eprintln!("");

    let mut pairings = Vec::new();

    loop {
        match pair_rx.recv() {
            Ok(pair) => {
                if cli.pairs {
                    println!("{} {}", pair.filename1, pair.filename2);
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
        let groups = make_groups(&pairings);
        for group in groups {
            let line = group.join(" ");
            println!("{}", line);
            if let Some(program) = &cli.exec {
                Command::new(program)
                    .args(group)
                    .output()
                    .expect("Unable to run program provided");
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

    eprintln!("");

    let mut pairings = Vec::new();

    loop {
        match pair_rx.recv() {
            Ok(pair) => {
                if cli.pairs {
                    println!("{} {}", pair.filename1, pair.filename2);
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
    eprintln!("Compare threads done");

    if !cli.pairs {
        let groups = make_groups(&pairings);
        for group in groups {
            let line = group.join(" ");
            println!("{}", line);
            if let Some(program) = &cli.exec {
                Command::new(program)
                    .args(group)
                    .output()
                    .expect("Unable to run program provided");
            }
        }
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
