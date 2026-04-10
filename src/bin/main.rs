use clap::Parser;
use ddgr::{results_to_json, results_to_toon, search, Engine, SearchOptions, DEFAULT_USER_AGENT};
use std::process;

/// Search the web from the terminal.
///
/// Queries DuckDuckGo's lite endpoint (default) with automatic Mojeek fallback.
/// Outputs results as JSON (default) or TOON for compact, LLM-friendly output.
#[derive(Parser, Debug)]
#[command(name = "ddgr", version, about)]
struct Cli {
    /// Search keywords
    #[arg(required = true)]
    keywords: Vec<String>,

    /// Number of results to return (max 40)
    #[arg(short = 'n', long, default_value = "10")]
    num: usize,

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

    /// Force a specific engine: ddg, mojeek (default: auto with fallback)
    #[arg(short = 'e', long)]
    engine: Option<String>,
}

fn main() {
    let cli = Cli::parse();

    let keywords = cli.keywords.join(" ");
    if keywords.is_empty() {
        eprintln!("Error: no search keywords provided");
        process::exit(1);
    }

    let engine = match cli.engine.as_deref() {
        Some("ddg") | Some("duckduckgo") => Some(Engine::DuckDuckGo),
        Some("mojeek") => Some(Engine::Mojeek),
        Some(other) => {
            eprintln!("Unknown engine '{}'. Available: ddg, mojeek", other);
            process::exit(1);
        }
        None => None,
    };

    let opts = SearchOptions {
        keywords,
        region: cli.region,
        safe: cli.safe,
        duration: cli.duration,
        user_agent: if cli.noua {
            String::new()
        } else {
            DEFAULT_USER_AGENT.into()
        },
        proxy: cli.proxy,
        toon: cli.toon,
        engine,
        max_results: cli.num,
    };

    match search(&opts) {
        Ok(results) => {
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
