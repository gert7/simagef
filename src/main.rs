mod cli;
mod database;
#[cfg(feature = "pixel")]
mod main_image;
mod open_image;
mod shared;

use core::fmt;
use std::{
    collections::{HashMap, HashSet},
    error::Error,
    io::BufRead,
    path::PathBuf,
    process::{Command, exit},
    thread::{self, JoinHandle},
};

use clap::Parser;
use cli::Cli;
use crossbeam::{
    channel::{Receiver, Sender, never},
    select,
};
mod image_match_rs;
use image_match_rs::{cosine_similarity, image::get_image_signature};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use rusqlite::Connection;

use crate::{database::InsertionMessage, open_image::open_image_path};

struct SignatureToCompare {
    path: String,
    signature: Vec<i8>,
}

struct CompareTask {
    pub index1: (usize, &'static SignatureToCompare),
    pub index2: (usize, &'static SignatureToCompare),
}

struct Pairing {
    pub index1: (usize, &'static SignatureToCompare),
    pub index2: (usize, &'static SignatureToCompare),
    pub score: f64,
}

fn make_groups<P>(pairs: P) -> Vec<Vec<usize>>
where
    P: IntoIterator<Item = Pairing>,
{
    let mut graph: HashMap<usize, Vec<usize>> = HashMap::new();

    // Build the graph
    for pair in pairs.into_iter() {
        // let (node1, node2) = *pair;
        let node1 = pair.index1.0;
        let node2 = pair.index2.0;
        graph.entry(node1).or_default().push(node2);
        graph.entry(node2).or_default().push(node1);
    }

    let mut visited: HashSet<usize> = HashSet::new();
    let mut groups: Vec<Vec<usize>> = Vec::new();

    // Perform DFS to find connected components
    fn dfs(
        node: usize,
        graph: &HashMap<usize, Vec<usize>>,
        visited: &mut HashSet<usize>,
        mut group: Vec<usize>,
    ) -> Vec<usize> {
        visited.insert(node);
        group.push(node);
        if let Some(neighbors) = graph.get(&node) {
            for neighbor in neighbors.iter() {
                if !visited.contains(neighbor) {
                    group = dfs(*neighbor, graph, visited, group);
                }
            }
        }
        group
    }

    for node in graph.keys() {
        if !visited.contains(node) {
            let mut group: Vec<usize> = Vec::new();
            group = dfs(*node, &graph, &mut visited, group);
            groups.push(group);
        }
    }

    groups
}

fn make_groups_and_exec<P>(name_map: &[String], pairings: P, executable: &Option<(&str, Vec<&str>)>)
where
    P: IntoIterator<Item = Pairing>,
{
    let groups = make_groups(pairings);
    for group in groups {
        let name_group: Vec<String> = group.iter().map(|index| name_map[*index].clone()).collect();
        let line = name_group.join(" ");
        println!("{}", line);
        if let Some((program, args)) = &executable {
            Command::new(program)
                .args(args)
                .args(name_group)
                .output()
                .expect("Unable to run program provided");
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

fn progress_bar_loop(calc_total_rx: Receiver<u64>, calc_count_rx: Receiver<u64>) {
    let bars = MultiProgress::new();
    let style = ProgressStyle::with_template("{msg} [{bar:40.cyan/blue}] {pos:>7}/{len:7}")
        .expect("Unable to style the progress bar")
        .progress_chars("#-");

    let calc_bar = bars.add(ProgressBar::new(0));
    calc_bar.set_style(style.clone());
    calc_bar.set_message("Calculating signatures");

    let mut calc_total_rx = Some(calc_total_rx);

    loop {
        select! {
            recv(calc_total_rx.as_ref().unwrap_or(&never())) -> msg => match msg {
                Ok(msg) => {
                    let length = calc_bar.length().unwrap_or(0) + msg;
                    calc_bar.set_length(length);
                },
                Err(_) => calc_total_rx = None,
            },
            recv(calc_count_rx) -> msg => match msg {
                Ok(msg) => calc_bar.inc(msg),
                Err(_) => break,
            },
        }
    }

    bars.clear().ok();
}

const FILENAME_CHANNEL_BOUND: usize = 65535;
const CHANNEL_BOUND: usize = 2048;

fn read_filenames(filename_tx: Sender<String>, dash_mode: bool, calc_total_tx: Sender<u64>) {
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
    }
}

#[derive(Debug)]
enum SigFetchError {
    PathConversionError(&'static str),
}

impl fmt::Display for SigFetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SigFetchError::PathConversionError(error) => {
                write!(f, "Error converting file path to string: {}", error)
            }
        }
    }
}

impl Error for SigFetchError {}

fn spawn_insertion_thread(
    insert_rx: Receiver<InsertionMessage>,
    db_path: &PathBuf,
) -> rusqlite::Result<JoinHandle<()>> {
    let mut conn = Connection::open(db_path)?;

    let t = thread::spawn(move || {
        let mut messages = vec![];

        while let Ok(msg) = insert_rx.recv() {
            messages.push(msg);

            if messages.len() >= 100 {
                if let Err(e) = database::insert_batch(&mut conn, &messages) {
                    eprintln!("{}", e);
                }
                messages.clear();
            }
        }
        if let Err(e) = database::insert_batch(&mut conn, &messages) {
            eprintln!("{}", e);
        }
    });

    Ok(t)
}

fn fetch_signature(
    filename: &str,
    db_conn: &Option<Connection>,
    insert_tx: &Sender<InsertionMessage>,
) -> anyhow::Result<Vec<i8>> {
    let filename = std::fs::canonicalize(filename)?;

    match db_conn {
        Some(conn) => {
            let filename_s = filename.to_str().ok_or(SigFetchError::PathConversionError(
                "Unable to convert file path",
            ))?;
            let stat = std::fs::metadata(&filename)?;
            let signature = database::fetch(conn, filename_s, &stat)?;
            match signature {
                Some(signature) => Ok(signature.signature),
                None => {
                    let image = open_image_path(&filename)?;
                    let signature = get_image_signature(image);
                    insert_tx
                        .send(InsertionMessage {
                            filename_s: filename_s.to_string(),
                            stat,
                            signature: signature.clone(),
                        })
                        .expect("Unable to send InsertionMessage");
                    Ok(signature)
                }
            }
        }
        None => {
            let image = open_image_path(&filename)?;
            Ok(get_image_signature(image))
        }
    }
}

fn spawn_signature_threads(
    filename_rx: Receiver<String>,
    img_tx: Sender<&'static SignatureToCompare>,
    calc_count_tx: Sender<u64>,
    db_path: Option<PathBuf>,
    insert_tx: Sender<InsertionMessage>,
) {
    let cpu_count = num_cpus::get();

    for _ in 0..cpu_count {
        let filename_rx = filename_rx.clone();
        let img_tx = img_tx.clone();
        let calc_count_tx = calc_count_tx.clone();
        let db_path = db_path.clone();
        let insert_tx = insert_tx.clone();
        thread::spawn(move || {
            let db_conn = db_path.as_ref().map(|db_path| {
                Connection::open(db_path).expect("Unable to open database connection")
            });
            let mut total = 0;
            while let Ok(filename) = filename_rx.recv() {
                match fetch_signature(&filename, &db_conn, &insert_tx) {
                    Ok(signature) => {
                        let stc = SignatureToCompare {
                            path: filename,
                            signature,
                        };
                        let stc = Box::from(stc);
                        let stc = Box::leak(stc);
                        img_tx
                            .send(stc)
                            .expect("Unable to send signature to channel");

                        total += 1;
                        if total >= 5 && calc_count_tx.try_send(total).is_ok() {
                            total = 0;
                        }
                    }
                    Err(e) => eprintln!("{}: {}", filename, e),
                }
            }
            calc_count_tx.send(total).ok();
        });
    }
}

fn spawn_cosine_threads(threshold: f64, task_rx: Receiver<CompareTask>, pair_tx: Sender<Pairing>) {
    let cpu_count = num_cpus::get();

    for _ in 0..cpu_count {
        let task_rx = task_rx.clone();
        let pair_tx = pair_tx.clone();
        thread::spawn(move || {
            while let Ok(task) = task_rx.recv() {
                let (_, image1) = task.index1;
                let (_, image2) = task.index2;
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
        });
    }
}

fn main_signatures(cli: Cli) {
    let db_path = platform_dirs::AppDirs::new(Some("simagef"), false).map(|v| v.cache_dir);

    let db_path = if !cli.no_database {
        let db_path = db_path.expect("Unable to figure out database path");
        let conn = Connection::open(&db_path).expect("Unable to open SQLite database");
        database::init(&conn).expect("Unable to initialize database");
        Some(db_path)
    } else {
        None
    };

    let (insert_tx, insert_rx) = crossbeam::channel::bounded(2048);

    let mut insertion_thread = None;

    if let Some(db_path) = &db_path {
        insertion_thread = Some(spawn_insertion_thread(insert_rx, db_path).expect("Unable to start database insertion thread"));
    }

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

    if !cli.pairs {
        thread::spawn(move || {
            progress_bar_loop(calc_total_rx, calc_count_rx);
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

    read_filenames(filename_tx, dash_mode, calc_total_tx);

    let (img_tx, img_rx) =
        crossbeam::channel::bounded::<&'static SignatureToCompare>(CHANNEL_BOUND);

    thread::spawn(move || {
        spawn_signature_threads(filename_rx, img_tx, calc_count_tx, db_path, insert_tx);
    });

    // Image task channel
    let (task_tx, task_rx) = crossbeam::channel::bounded(CHANNEL_BOUND);

    // Image list return channel
    let (ret_tx, ret_rx) = crossbeam::channel::bounded(1);

    thread::spawn(move || {
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

        let mut images: Vec<(usize, &'static SignatureToCompare)> = Vec::new();

        while let Ok(image) = img_rx.recv() {
            let signature: Vec<_> = image.signature.iter().map(|v| *v as f32).collect();
            let results = lsh
                .query_bucket_ids(&signature)
                .expect("Unable to query bucket");
            let index1: usize = lsh
                .store_vec(&signature)
                .expect("Unable to store signature")
                .try_into()
                .unwrap();

            let ipair = (index1, image);
            images.push(ipair);

            for index2 in results {
                let index2: usize = index2.try_into().expect("Unable to convert u32 to usize");
                task_tx
                    .send(CompareTask {
                        index1: ipair,
                        index2: images[index2],
                    })
                    .expect("Unable to send task");
            }
        }

        ret_tx.send(images).expect("Unable to send image list back");
    });

    // Image pairing channel
    let (pair_tx, pair_rx) = crossbeam::channel::bounded(CHANNEL_BOUND);

    spawn_cosine_threads(threshold, task_rx, pair_tx);

    let mut pairings = Vec::new();

    let executable = {
        cli.exec.as_ref().map(|exec| {
            let mut split = exec.split(" ");
            let command = split.next().expect("Command for exec not provided");
            let rest: Vec<&str> = split.collect();
            (command, rest)
        })
    };

    while let Ok(pair) = pair_rx.recv() {
        // If we use pairs, we execute for each pair right away.
        if cli.pairs {
            let (_, image1) = pair.index1;
            let (_, image2) = pair.index2;
            let filename1 = &image1.path;
            let filename2 = &image2.path;
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

    if let Some(thread) = insertion_thread {
        thread.join().expect("Database insertion thread error");
    }

    let images = ret_rx.recv().unwrap();

    let image_map: Vec<String> = images.iter().map(|(_, s)| s.path.clone()).collect();

    if !cli.pairs {
        make_groups_and_exec(&image_map, pairings, &executable);
    }
}

#[cfg(feature = "pixel")]
fn main_pixel(cli: Cli) {
    main_image::main_images(cli);
}

#[cfg(not(feature = "pixel"))]
fn main_pixel(_cli: Cli) {
    eprintln!("--pixels feature not enabled");
    exit(1);
}

fn main() {
    let cli = Cli::parse();
    if cli.pixels {
        main_pixel(cli);
    } else {
        main_signatures(cli);
    }
}
