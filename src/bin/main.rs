use clap::Parser;
use ddgr::{results_to_json, results_to_toon, search, SearchOptions};
use std::process;

/// DuckDuckGo search from the terminal.
///
/// Searches DuckDuckGo's lite HTML endpoint and outputs results as JSON
/// (default) or TOON (Token-Oriented Object Notation) for compact,
/// LLM-friendly output.
#[derive(Parser, Debug)]
#[command(name = "ddgr", version, about)]
struct Cli {
    /// Search keywords
    #[arg(required = true)]
    keywords: Vec<String>,

    /// Region code (e.g. "us-en", "wt-wt" for no region)
    #[arg(short = 'r', long, default_value = "wt-wt")]
    region: String,

    /// Safe search: -2 = off, -1 = moderate, 1 = strict
    #[arg(short = 's', long, default_value = "1")]
    safe: i8,

    /// Time filter: d (day), w (week), m (month), or empty for any
    #[arg(short = 'd', long, default_value = "")]
    duration: String,

    /// Disable User-Agent header
    #[arg(long)]
    noua: bool,

    /// HTTPS proxy URL
    #[arg(short = 'p', long)]
    proxy: Option<String>,

    /// Output results in TOON format instead of JSON
    #[arg(long)]
    toon: bool,
}

fn main() {
    let cli = Cli::parse();

    let keywords = cli.keywords.join(" ");
    if keywords.is_empty() {
        eprintln!("Error: no search keywords provided");
        process::exit(1);
    }

    let opts = SearchOptions {
        keywords,
        region: cli.region,
        safe: cli.safe,
        duration: cli.duration,
        user_agent: if cli.noua {
            String::new()
        } else {
            ddgr::SearchOptions::default().user_agent
        },
        proxy: cli.proxy,
        toon: cli.toon,
    };

    match search(&opts) {
        Ok((results, _pagination)) => {
            if results.is_empty() {
                eprintln!("No results.");
                process::exit(1);
            }

            let output = if opts.toon {
                results_to_toon(&results)
            } else {
                results_to_json(&results)
            };

            println!("{}", output);
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    }
}
