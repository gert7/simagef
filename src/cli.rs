use std::fmt::Display;

use clap::Parser;

#[derive(Debug, Clone, Copy)]
pub enum Fmt {
    /// Filenames are separated by spaces. Groups are separated by newlines.
    /// Spaces in paths are ambiguous.
    Regular,
    /// Same as regular, but each filename is written in quotemarks. Quotemarks
    /// in the path are escaped with a backslash.
    Quote,
    /// Filenames are separated by NUL characters. Groups are separated by
    /// two consecutive NUL characters.
    Null,
}

impl Display for Fmt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Fmt::Regular => f.write_str("regular"),
            Fmt::Quote => f.write_str("quote"),
            Fmt::Null => f.write_str("null"),
        }
    }
}

impl From<&str> for Fmt {
    fn from(value: &str) -> Self {
        match value {
            "regular" => Self::Regular,
            "quote" => Self::Quote,
            "null" => Self::Null,
            _ => panic!("Unknown option for --format"),
        }
    }
}

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Compare images by shrinking them to identical sizes and comparing the pixel values, instead of signatures.
    #[arg(short('m'), long, default_value_t = false)]
    pub pixels: bool,
    /// The amount of similarity as a percentage to be considered similar.
    #[arg(short, long, value_parser = clap::value_parser!(u8).range(0..=100))]
    pub threshold: Option<u8>,
    /// The program to launch when the comparisons are finished.
    /// The program will be launched for each pair or grouping, one after another.
    #[arg(short('e'), long)]
    #[cfg(not(feature = "no-exec"))]
    pub exec: Option<String>,
    /// If set, will only present the matched images in pairs rather than groups.
    #[arg(short('p'), long, default_value_t = false)]
    pub pairs: bool,
    /// By default we use a database to store signatures, speeding up subsequent runs.
    #[arg(short('d'), long, default_value_t = false)]
    pub no_database: bool,
    /// The path for the database file. Will be created if it doesn't exist.
    #[arg(long)]
    pub database_file: Option<String>,
    /// Print database file location and exit.
    #[arg(long)]
    pub print_database_location: bool,
    /// The width to resize the images to before comparing in pixel mode.
    #[arg(long, default_value_t = 160)]
    pub width: u32,
    /// The height to resize the images to before comparing in pixel mode.
    #[arg(long, default_value_t = 160)]
    pub height: u32,
    /// The files to compare. If one of these is a dash '-' the program will
    /// also read filenames from stdin.
    pub files: Vec<String>,
    /// Format to use for printing the filenames - regular, quote, null.
    #[arg(long, default_value_t = Fmt::Regular)]
    pub format: Fmt,
}
