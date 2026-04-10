use clap::Parser;
use std::process;
use websearch::{
    results_to_json, results_to_toon, search, Engine, SearchOptions, DEFAULT_USER_AGENT,
};

/// Search the web from the terminal.
///
/// Providers: DuckDuckGo (default + Mojeek fallback), Mojeek, ArXiv.
/// Outputs results as JSON (default) or TOON for compact, LLM-friendly output.
#[derive(Parser, Debug)]
#[command(name = "websearch", version, about)]
struct Cli {
    /// Search keywords
    #[arg(required = true)]
    keywords: Vec<String>,

    /// Number of results to return (max 40)
    #[arg(short = 'n', long, default_value = "10")]
    num: usize,

    /// Search provider: ddg, mojeek, arxiv (default: ddg with mojeek fallback)
    #[arg(short = 'p', long)]
    provider: Option<String>,

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
    #[arg(long)]
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

    let provider = match cli.provider.as_deref() {
        Some("ddg") | Some("duckduckgo") => Some(Engine::DuckDuckGo),
        Some("mojeek") => Some(Engine::Mojeek),
        Some("arxiv") => Some(Engine::ArXiv),
        Some(other) => {
            eprintln!("Unknown provider '{}'. Available: ddg, mojeek, arxiv", other);
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
        provider,
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
