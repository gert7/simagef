use crate::cli::Fmt;

pub fn print_fmt(group: &Vec<&str>, fmt: Fmt) {
    match fmt {
        Fmt::Regular => {
            println!("{}", group.join(" "));
        }
        Fmt::Quote => {
            let lines: Vec<String> = group
                .into_iter()
                .map(|s| format!("\"{}\"", s.replace("\"", "\\\"")))
                .collect();
            let line = lines.join(" ");
            println!("{}", line);
        }
        Fmt::Null => {
            for filename in group {
                print!("{}\0", filename);
            }
            print!("\0");
        }
    }
}
