//! # websearch
//!
//! Search the web from the terminal. Queries DuckDuckGo (primary) with
//! automatic Mojeek fallback, plus ArXiv for academic papers.
//!
//! Auto-paginates to collect up to `max_results` results and outputs
//! as JSON or TOON (compact, LLM-friendly format).

pub mod arxiv;
pub mod ddg;
pub mod mojeek;

use std::time::Duration;

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) \
     Chrome/120.0.0.0 Safari/537.36";

const MAX_RESULTS_CAP: usize = 40;
const DEFAULT_MAX_RESULTS: usize = 10;

// ---------------------------------------------------------------------------
// Engine selection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Engine {
    #[default]
    DuckDuckGo,
    Mojeek,
    ArXiv,
}

impl std::fmt::Display for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Engine::DuckDuckGo => write!(f, "DuckDuckGo"),
            Engine::Mojeek => write!(f, "Mojeek"),
            Engine::ArXiv => write!(f, "ArXiv"),
        }
    }
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Search engine blocked the request (captcha/rate-limit)")]
    Blocked,

    #[error("No results found")]
    NoResults,
}

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchResult {
    pub index: usize,
    pub title: String,
    pub url: String,
    pub abstract_text: String,
}

impl SearchResult {
    pub fn to_json_value(&self) -> serde_json::Value {
        serde_json::json!({
            "index": self.index,
            "title": self.title,
            "url": self.url,
            "abstract": self.abstract_text,
        })
    }
}

// ---------------------------------------------------------------------------
// Search options
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub keywords: String,
    pub region: String,
    /// Safe-search level: -2 = off, -1 = moderate, 1 = strict.
    pub safe: i8,
    /// Time filter: "" (any), "d" (day), "w" (week), "m" (month).
    pub duration: String,
    /// Custom User-Agent string; empty string means send no UA header.
    pub user_agent: String,
    /// HTTPS proxy URL.
    pub proxy: Option<String>,
    pub toon: bool,
    /// Force a specific provider. `None` = DuckDuckGo with Mojeek fallback.
    pub provider: Option<Engine>,
    /// Maximum results to collect (auto-paginates as needed). Capped at 40.
    pub max_results: usize,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            keywords: String::new(),
            region: "wt-wt".into(),
            safe: 1,
            duration: String::new(),
            user_agent: DEFAULT_USER_AGENT.into(),
            proxy: None,
            toon: false,
            provider: None,
            max_results: DEFAULT_MAX_RESULTS,
        }
    }
}

// ---------------------------------------------------------------------------
// Pagination state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct PaginationState {
    pub engine: Engine,
    pub page: usize,
    pub cur_index: i64,
    pub result_count: usize,
    /// Total results available (ArXiv reports this explicitly).
    pub total_results: usize,
    // DDG-specific pagination tokens
    pub next_params: String,
    pub prev_params: String,
    pub vqd: String,
    /// The User-Agent used for this search session.
    /// DDG's vqd token is tied to the UA — pagination MUST use the same one.
    pub user_agent: String,
}

impl PaginationState {
    pub fn has_next(&self) -> bool {
        match self.engine {
            Engine::DuckDuckGo => !self.next_params.is_empty() && !self.vqd.is_empty(),
            Engine::Mojeek => self.result_count >= mojeek::RESULTS_PER_PAGE,
            Engine::ArXiv => {
                let fetched = (self.cur_index - 1) as usize;
                self.total_results > 0 && fetched < self.total_results
            }
        }
    }
}

// ---------------------------------------------------------------------------
// High-level search with auto-pagination and fallback
// ---------------------------------------------------------------------------

/// Search the web, auto-paginating to collect up to `opts.max_results` results.
///
/// If `opts.provider` is set, only that engine is used.
/// Otherwise, tries DuckDuckGo first; on any error, falls back to Mojeek.
pub fn search(opts: &SearchOptions) -> Result<Vec<SearchResult>, SearchError> {
    let max = opts.max_results.min(MAX_RESULTS_CAP);

    match opts.provider {
        Some(engine) => search_with_engine(engine, opts, max),
        None => match search_with_engine(Engine::DuckDuckGo, opts, max) {
            Ok(r) => Ok(r),
            Err(e) => {
                eprintln!("[websearch] DuckDuckGo failed ({}), trying Mojeek...", e);
                search_with_engine(Engine::Mojeek, opts, max)
            }
        },
    }
}

fn search_with_engine(
    engine: Engine,
    opts: &SearchOptions,
    max: usize,
) -> Result<Vec<SearchResult>, SearchError> {
    let (mut results, mut pag) = match engine {
        Engine::DuckDuckGo => ddg::search_page(opts),
        Engine::Mojeek => mojeek::search_page(opts),
        Engine::ArXiv => arxiv::search_page(opts),
    }?;

    while results.len() < max && pag.has_next() {
        std::thread::sleep(pagination_delay(engine));
        let (more, new_pag) = match engine {
            Engine::DuckDuckGo => ddg::search_next_page(opts, &pag),
            Engine::Mojeek => mojeek::search_next_page(opts, &pag),
            Engine::ArXiv => arxiv::search_next_page(opts, &pag),
        }?;
        if more.is_empty() {
            break;
        }
        results.extend(more);
        pag = new_pag;
    }

    results.truncate(max);
    Ok(results)
}

/// Per-engine delay between pagination requests.
fn pagination_delay(engine: Engine) -> Duration {
    match engine {
        Engine::DuckDuckGo | Engine::Mojeek => Duration::from_millis(1500),
        Engine::ArXiv => Duration::from_millis(3000), // ArXiv TOS: max 1 req/3s
    }
}

// ---------------------------------------------------------------------------
// Output formatters
// ---------------------------------------------------------------------------

pub fn results_to_json(results: &[SearchResult]) -> String {
    let objects: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            serde_json::json!({
                "title": r.title,
                "url": r.url,
                "abstract": r.abstract_text,
            })
        })
        .collect();
    serde_json::to_string_pretty(&objects).unwrap_or_else(|_| "[]".into())
}

pub fn results_to_toon(results: &[SearchResult]) -> String {
    let objects: Vec<serde_json::Value> = results.iter().map(|r| r.to_json_value()).collect();
    let value = serde_json::Value::Array(objects);
    toon::encode(&value, None)
}

// ---------------------------------------------------------------------------
// Shared HTTP client builder
// ---------------------------------------------------------------------------

pub(crate) fn build_client(opts: &SearchOptions) -> Result<Client, reqwest::Error> {
    let mut builder = Client::builder()
        .gzip(true)
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::limited(10));

    if !opts.user_agent.is_empty() {
        builder = builder.user_agent(&opts.user_agent);
    }

    if let Some(ref proxy_url) = opts.proxy {
        builder = builder.proxy(reqwest::Proxy::https(proxy_url)?);
    }

    builder.build()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_options() {
        let opts = SearchOptions::default();
        assert_eq!(opts.region, "wt-wt");
        assert_eq!(opts.safe, 1);
        assert!(opts.duration.is_empty());
        assert_eq!(opts.user_agent, DEFAULT_USER_AGENT);
        assert!(opts.proxy.is_none());
        assert!(!opts.toon);
        assert!(opts.provider.is_none());
        assert_eq!(opts.max_results, DEFAULT_MAX_RESULTS);
    }

    #[test]
    fn test_search_result_serialization() {
        let result = SearchResult {
            index: 1,
            title: "Test Title".into(),
            url: "https://example.com".into(),
            abstract_text: "A test abstract".into(),
        };
        let json = serde_json::to_string(&result).unwrap();
        let deserialized: SearchResult = serde_json::from_str(&json).unwrap();
        assert_eq!(result, deserialized);
    }

    #[test]
    fn test_results_to_json_format() {
        let results = vec![
            SearchResult {
                index: 1,
                title: "Rust".into(),
                url: "https://rust-lang.org".into(),
                abstract_text: "A systems language".into(),
            },
            SearchResult {
                index: 2,
                title: "Go".into(),
                url: "https://go.dev".into(),
                abstract_text: "Another language".into(),
            },
        ];
        let json_str = results_to_json(&results);
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["title"], "Rust");
    }

    #[test]
    fn test_toon_more_compact_than_json() {
        let results: Vec<SearchResult> = (1..=10)
            .map(|i| SearchResult {
                index: i,
                title: format!("Result Title Number {}", i),
                url: format!("https://example.com/page/{}", i),
                abstract_text: format!("This is the abstract for result number {}", i),
            })
            .collect();
        let json_str = results_to_json(&results);
        let toon_str = results_to_toon(&results);
        assert!(toon_str.len() < json_str.len());
    }

    #[test]
    fn test_build_client_defaults() {
        assert!(build_client(&SearchOptions::default()).is_ok());
    }

    #[test]
    fn test_pagination_has_next_ddg() {
        let pag = PaginationState {
            engine: Engine::DuckDuckGo,
            next_params: "abc".into(),
            vqd: "xyz".into(),
            ..Default::default()
        };
        assert!(pag.has_next());
        assert!(!PaginationState::default().has_next());
    }

    #[test]
    fn test_pagination_has_next_mojeek() {
        let pag = PaginationState {
            engine: Engine::Mojeek,
            result_count: 10,
            ..Default::default()
        };
        assert!(pag.has_next());

        let partial = PaginationState {
            engine: Engine::Mojeek,
            result_count: 5,
            ..Default::default()
        };
        assert!(!partial.has_next());
    }

    #[test]
    fn test_pagination_has_next_arxiv() {
        let pag = PaginationState {
            engine: Engine::ArXiv,
            cur_index: 11,
            total_results: 100,
            ..Default::default()
        };
        assert!(pag.has_next());

        let done = PaginationState {
            engine: Engine::ArXiv,
            cur_index: 101,
            total_results: 100,
            ..Default::default()
        };
        assert!(!done.has_next());
    }

    #[test]
    fn test_pagination_delay_arxiv_slower() {
        let ddg_delay = pagination_delay(Engine::DuckDuckGo);
        let arxiv_delay = pagination_delay(Engine::ArXiv);
        assert!(arxiv_delay > ddg_delay);
    }

    // -----------------------------------------------------------------------
    // Integration tests — run with: cargo test -- --ignored
    // -----------------------------------------------------------------------

    #[test]
    #[ignore]
    fn test_ddg_search() {
        let opts = SearchOptions {
            keywords: "rust programming language".into(),
            provider: Some(Engine::DuckDuckGo),
            ..Default::default()
        };
        let results = search(&opts).expect("DDG search should succeed");
        assert!(!results.is_empty());
    }

    #[test]
    #[ignore]
    fn test_mojeek_search() {
        let opts = SearchOptions {
            keywords: "rust programming language".into(),
            provider: Some(Engine::Mojeek),
            ..Default::default()
        };
        let results = search(&opts).expect("Mojeek search should succeed");
        assert!(!results.is_empty());
    }

    #[test]
    #[ignore]
    fn test_arxiv_search() {
        let opts = SearchOptions {
            keywords: "machine learning".into(),
            provider: Some(Engine::ArXiv),
            ..Default::default()
        };
        let results = search(&opts).expect("ArXiv search should succeed");
        assert!(!results.is_empty());
        assert!(results[0].url.contains("arxiv.org"));
    }

    #[test]
    #[ignore]
    fn test_auto_fallback() {
        let opts = SearchOptions {
            keywords: "rust programming".into(),
            ..Default::default()
        };
        let results = search(&opts).expect("auto search should succeed");
        assert!(!results.is_empty());
    }
}
