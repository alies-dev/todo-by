use clap::Parser;

/// Flag todo-by tags whose deadline date has passed, across any file type.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    /// Paths to scan (files or directories). Defaults to current directory.
    #[arg(default_value = ".")]
    paths: Vec<String>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,

    /// Treat tags due on or before this date as overdue (YYYY-MM-DD). Defaults to today.
    #[arg(long)]
    today: Option<String>,
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum Format {
    /// Human-readable lines.
    Text,
    /// JSON array of findings.
    Json,
    /// GitHub Actions annotations.
    Github,
}

fn main() {
    let cli = Cli::parse();

    // todo-by 2026-12-31 walk `cli.paths` with the `ignore` crate (respects .gitignore).
    // todo-by 2026-12-31 match `todo-by YYYY-MM-DD`, validate the date, capture file/line.
    // todo-by 2026-12-31 compare the parsed date against `cli.today` (or chrono::Local::now).
    // todo-by 2026-12-31 render per `cli.format`; exit non-zero when overdue tags exist.

    eprintln!("todo-by: not implemented yet ({:?})", cli);
    std::process::exit(0);
}
