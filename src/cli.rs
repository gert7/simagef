use clap::Parser;

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
    pub exec: Option<String>,
    /// If set, will only present the matched images in pairs rather than groups.
    #[arg(short('p'), long, default_value_t = false)]
    pub pairs: bool,
    /// The width to resize the images to before comparing in pixel mode.
    #[arg(long, default_value_t = 160)]
    pub width: u32,
    /// The height to resize the images to before comparing in pixel mode.
    #[arg(long, default_value_t = 160)]
    pub height: u32,
    /// The files to compare. If one of these is a dash '-' the program will
    /// also read filenames from stdin.
    pub files: Vec<String>,
}
