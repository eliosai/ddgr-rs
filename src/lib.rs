//! # ddgr
//!
//! A Rust library and CLI for searching DuckDuckGo from the terminal.
//!
//! Uses the same strategy as the original Python `ddgr`: POST requests to
//! `https://html.duckduckgo.com/html` (DuckDuckGo's lite, JS-free endpoint),
//! then parses the static HTML response for search results.
//!
//! Supports optional TOON (Token-Oriented Object Notation) output via the
//! `toon` crate for compact, LLM-friendly serialization.

mod parser;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::cookie::Jar;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use parser::DdgParser;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const DDG_URL: &str = "https://html.duckduckgo.com/html";
const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) \
     Chrome/120.0.0.0 Safari/537.36";

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum DdgError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Failed to decompress gzip payload: {0}")]
    Gzip(#[from] std::io::Error),

    #[error("DuckDuckGo returned unusual-activity block (captcha). This is an IP-level restriction.")]
    Blocked,

    #[error("No results found")]
    NoResults,
}

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

/// A single DuckDuckGo search result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchResult {
    pub index: usize,
    pub title: String,
    pub url: String,
    pub abstract_text: String,
}

impl SearchResult {
    /// Convert to a serde_json::Value for toon encoding.
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

/// Options controlling a DuckDuckGo search.
#[derive(Debug, Clone)]
pub struct SearchOptions {
    /// The search query keywords.
    pub keywords: String,
    /// Region code (e.g. "us-en", "wt-wt" for no region).
    pub region: String,
    /// Safe-search level: -2 = off, -1 = moderate, 1 = strict.
    pub safe: i8,
    /// Time filter: "" (any), "d" (day), "w" (week), "m" (month).
    pub duration: String,
    /// Custom User-Agent string; empty string means send no UA header.
    pub user_agent: String,
    /// HTTPS proxy URL (e.g. "https://127.0.0.1:9050").
    pub proxy: Option<String>,
    /// Whether to use TOON encoding for output.
    pub toon: bool,
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
        }
    }
}

// ---------------------------------------------------------------------------
// Pagination state
// ---------------------------------------------------------------------------

/// Holds pagination state for multi-page fetches.
#[derive(Debug, Clone, Default)]
pub struct PaginationState {
    pub page: usize,
    pub cur_index: i64,
    pub next_params: String,
    pub prev_params: String,
    pub vqd: String,
    pub result_count: usize,
}

// ---------------------------------------------------------------------------
// Core search functions
// ---------------------------------------------------------------------------

/// Perform a first-page search and return results.
///
/// Mirrors the Python `ddgr` approach: POST to `html.duckduckgo.com/html`
/// with the same form fields and headers (`Accept-Encoding: gzip`,
/// `User-Agent`, `DNT: 1`). A cookie jar is used to persist any session
/// cookies DDG sets (Python's `urllib` does this implicitly when an opener
/// with `HTTPCookieProcessor` is used).
pub fn search(opts: &SearchOptions) -> Result<(Vec<SearchResult>, PaginationState), DdgError> {
    let client = build_client(opts)?;

    let mut form: HashMap<&str, String> = HashMap::new();
    form.insert("q", opts.keywords.clone());
    form.insert("b", String::new());
    form.insert("df", opts.duration.clone());
    form.insert("kf", "-1".into());
    form.insert("kh", "1".into());
    form.insert("kl", opts.region.clone());
    form.insert("kp", opts.safe.to_string());
    form.insert("k1", "-1".into());

    let resp = client
        .post(DDG_URL)
        .header("Accept-Encoding", "gzip")
        .header("DNT", "1")
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .header("Accept-Language", "en-US,en;q=0.5")
        .header("Referer", "https://html.duckduckgo.com/")
        .form(&form)
        .send()?;

    let body = resp.text()?;

    let mut parser = DdgParser::new(0);
    parser.parse(&body);

    if parser.is_blocked {
        return Err(DdgError::Blocked);
    }

    let pagination = PaginationState {
        page: 0,
        cur_index: 1 + parser.results.len() as i64,
        next_params: parser.np_next.clone(),
        prev_params: parser.np_prev.clone(),
        vqd: parser.vqd.clone(),
        result_count: parser.results.len(),
    };

    Ok((parser.results, pagination))
}

/// Fetch the next page given existing pagination state.
pub fn search_next(
    opts: &SearchOptions,
    pag: &PaginationState,
) -> Result<(Vec<SearchResult>, PaginationState), DdgError> {
    let client = build_client(opts)?;
    let next_page = pag.page + 1;

    let mut form: HashMap<&str, String> = HashMap::new();
    form.insert("q", opts.keywords.clone());
    form.insert("s", (50 * (next_page.saturating_sub(1)) + 30).to_string());
    form.insert("nextParams", pag.next_params.clone());
    form.insert("v", "l".into());
    form.insert("o", "json".into());
    form.insert("dc", pag.cur_index.to_string());
    form.insert("df", opts.duration.clone());
    form.insert("api", "/d.js".into());
    form.insert("kf", "-1".into());
    form.insert("kh", "1".into());
    form.insert("kl", opts.region.clone());
    form.insert("kp", opts.safe.to_string());
    form.insert("k1", "-1".into());
    form.insert("vqd", pag.vqd.clone());

    let resp = client
        .post(DDG_URL)
        .header("Accept-Encoding", "gzip")
        .header("DNT", "1")
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .header("Accept-Language", "en-US,en;q=0.5")
        .header("Referer", "https://html.duckduckgo.com/")
        .form(&form)
        .send()?;

    let body = resp.text()?;

    let offset = if pag.cur_index > 0 {
        pag.cur_index as usize - 1
    } else {
        0
    };
    let mut parser = DdgParser::new(offset);
    parser.parse(&body);

    if parser.is_blocked {
        return Err(DdgError::Blocked);
    }

    let new_pag = PaginationState {
        page: next_page,
        cur_index: pag.cur_index + parser.results.len() as i64,
        next_params: parser.np_next.clone(),
        prev_params: parser.np_prev.clone(),
        vqd: if parser.vqd.is_empty() {
            pag.vqd.clone()
        } else {
            parser.vqd.clone()
        },
        result_count: parser.results.len(),
    };

    Ok((parser.results, new_pag))
}

/// Format results as a JSON string.
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

/// Format results as TOON-encoded string.
pub fn results_to_toon(results: &[SearchResult]) -> String {
    let objects: Vec<serde_json::Value> = results.iter().map(|r| r.to_json_value()).collect();
    let value = serde_json::Value::Array(objects);
    toon::encode(&value, None)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn build_client(opts: &SearchOptions) -> Result<Client, reqwest::Error> {
    let cookie_jar = Arc::new(Jar::default());

    let mut builder = Client::builder()
        .cookie_provider(cookie_jar)
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

    // -----------------------------------------------------------------------
    // Unit tests (offline, always pass)
    // -----------------------------------------------------------------------

    #[test]
    fn test_default_options() {
        let opts = SearchOptions::default();
        assert_eq!(opts.region, "wt-wt");
        assert_eq!(opts.safe, 1);
        assert!(opts.duration.is_empty());
        assert_eq!(opts.user_agent, DEFAULT_USER_AGENT);
        assert!(opts.proxy.is_none());
        assert!(!opts.toon);
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
        assert!(parsed.is_array());
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["title"], "Rust");
        assert_eq!(arr[0]["url"], "https://rust-lang.org");
        assert_eq!(arr[0]["abstract"], "A systems language");
        assert_eq!(arr[1]["title"], "Go");
    }

    #[test]
    fn test_results_to_toon_format() {
        let results = vec![SearchResult {
            index: 1,
            title: "Rust".into(),
            url: "https://rust-lang.org".into(),
            abstract_text: "A systems language".into(),
        }];
        let toon_str = results_to_toon(&results);
        assert!(!toon_str.is_empty());
        // TOON output should contain the data
        assert!(toon_str.contains("Rust"));
        assert!(toon_str.contains("rust-lang.org"));
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

        assert!(
            toon_str.len() < json_str.len(),
            "TOON ({} bytes) should be more compact than JSON ({} bytes)",
            toon_str.len(),
            json_str.len()
        );
    }

    #[test]
    fn test_json_vs_toon_same_data() {
        let results = vec![
            SearchResult {
                index: 1,
                title: "Alpha".into(),
                url: "https://alpha.com".into(),
                abstract_text: "First result".into(),
            },
            SearchResult {
                index: 2,
                title: "Beta".into(),
                url: "https://beta.com".into(),
                abstract_text: "Second result".into(),
            },
        ];

        // JSON path
        let json_str = results_to_json(&results);
        let json_parsed: Vec<serde_json::Value> = serde_json::from_str(&json_str).unwrap();
        assert_eq!(json_parsed.len(), 2);

        // TOON path
        let toon_str = results_to_toon(&results);
        assert!(!toon_str.is_empty());

        // Both should contain the same data
        assert!(toon_str.contains("Alpha"));
        assert!(toon_str.contains("Beta"));
        assert!(toon_str.contains("alpha.com"));
        assert!(toon_str.contains("beta.com"));
    }

    #[test]
    fn test_build_client_defaults() {
        let opts = SearchOptions::default();
        let client = build_client(&opts);
        assert!(client.is_ok(), "default client should build successfully");
    }

    #[test]
    fn test_build_client_no_ua() {
        let opts = SearchOptions {
            user_agent: String::new(),
            ..Default::default()
        };
        let client = build_client(&opts);
        assert!(client.is_ok(), "client with empty UA should build");
    }

    // -----------------------------------------------------------------------
    // Integration tests (hit actual DDG API)
    //
    // These are marked #[ignore] because they require network access to
    // DuckDuckGo and will fail if the IP is captcha-blocked. Run them with:
    //   cargo test -- --ignored
    // -----------------------------------------------------------------------

    #[test]
    #[ignore]
    fn test_search_returns_results() {
        let opts = SearchOptions {
            keywords: "rust programming language".into(),
            ..Default::default()
        };
        let (results, pag) = search(&opts).expect("search should succeed");
        assert!(!results.is_empty(), "should return at least one result");
        assert!(!pag.vqd.is_empty(), "should have a vqd token for pagination");

        let first = &results[0];
        assert!(!first.title.is_empty(), "title should not be empty");
        assert!(first.url.starts_with("http"), "url should start with http");
    }

    #[test]
    #[ignore]
    fn test_search_json_output() {
        let opts = SearchOptions {
            keywords: "rust programming language".into(),
            ..Default::default()
        };
        let (results, _) = search(&opts).expect("search should succeed");
        assert!(!results.is_empty(), "should have results");
        let json_str = results_to_json(&results);
        let parsed: serde_json::Value =
            serde_json::from_str(&json_str).expect("JSON should be valid");
        assert!(parsed.is_array());
        let arr = parsed.as_array().unwrap();
        assert!(!arr.is_empty());
        for item in arr {
            assert!(item.get("title").is_some());
            assert!(item.get("url").is_some());
            assert!(item.get("abstract").is_some());
        }
    }

    #[test]
    #[ignore]
    fn test_search_toon_output() {
        let opts = SearchOptions {
            keywords: "rust programming language".into(),
            ..Default::default()
        };
        let (results, _) = search(&opts).expect("search should succeed");
        assert!(!results.is_empty(), "should have results");
        let toon_str = results_to_toon(&results);
        let json_str = results_to_json(&results);
        assert!(
            toon_str.len() <= json_str.len(),
            "TOON should be more compact than JSON (toon={}, json={})",
            toon_str.len(),
            json_str.len()
        );
    }

    #[test]
    #[ignore]
    fn test_search_no_toon_vs_toon() {
        let opts = SearchOptions {
            keywords: "DuckDuckGo search engine".into(),
            ..Default::default()
        };
        let (results, _) = search(&opts).expect("search should succeed");
        assert!(!results.is_empty(), "should have results");

        let json_out = results_to_json(&results);
        let toon_out = results_to_toon(&results);
        assert!(!json_out.is_empty());
        assert!(!toon_out.is_empty());

        let parsed: serde_json::Value = serde_json::from_str(&json_out).unwrap();
        assert!(parsed.is_array());
        assert!(
            toon_out.len() <= json_out.len(),
            "TOON should be <= JSON size"
        );
    }

    #[test]
    #[ignore]
    fn test_search_with_region() {
        let opts = SearchOptions {
            keywords: "weather today".into(),
            region: "us-en".into(),
            ..Default::default()
        };
        let (results, _) = search(&opts).expect("search should succeed");
        assert!(!results.is_empty(), "region search should return results");
    }

    #[test]
    #[ignore]
    fn test_search_safe_off() {
        let opts = SearchOptions {
            keywords: "rust programming language".into(),
            safe: -2,
            ..Default::default()
        };
        let (results, _) = search(&opts).expect("search should succeed");
        assert!(!results.is_empty(), "safe=off search should return results");
    }

    #[test]
    #[ignore]
    fn test_pagination() {
        let opts = SearchOptions {
            keywords: "linux kernel".into(),
            ..Default::default()
        };
        let (first_results, pag) = search(&opts).expect("first page should succeed");
        assert!(!first_results.is_empty(), "first page should have results");

        if !pag.next_params.is_empty() && !pag.vqd.is_empty() {
            std::thread::sleep(Duration::from_secs(2));
            let (second_results, _) = search_next(&opts, &pag).expect("next page should succeed");
            if !second_results.is_empty() {
                assert_ne!(
                    first_results[0].url, second_results[0].url,
                    "page 2 results should differ from page 1"
                );
            }
        }
    }
}
