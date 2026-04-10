//! ArXiv search engine — queries export.arxiv.org/api/query (Atom XML API).
//!
//! Rate limit: max 1 request per 3 seconds per ArXiv Terms of Use.

pub mod parser;

use crate::{build_client, Engine, PaginationState, SearchError, SearchOptions, SearchResult};

const ARXIV_API_URL: &str = "https://export.arxiv.org/api/query";

/// ArXiv returns up to this many results per request.
pub const RESULTS_PER_PAGE: usize = 10;

/// Fetch a single page of ArXiv results (first page).
pub fn search_page(
    opts: &SearchOptions,
) -> Result<(Vec<SearchResult>, PaginationState), SearchError> {
    let client = build_client(opts)?;
    let query = format!("all:{}", opts.keywords);

    let body = client
        .get(ARXIV_API_URL)
        .query(&[
            ("search_query", query.as_str()),
            ("start", "0"),
            ("max_results", &RESULTS_PER_PAGE.to_string()),
            ("sortBy", "relevance"),
            ("sortOrder", "descending"),
        ])
        .send()?
        .text()?;

    let feed = parser::parse(&body, 0);

    let pag = PaginationState {
        engine: Engine::ArXiv,
        page: 0,
        cur_index: 1 + feed.results.len() as i64,
        result_count: feed.results.len(),
        total_results: feed.total_results,
        user_agent: opts.user_agent.clone(),
        ..Default::default()
    };

    Ok((feed.results, pag))
}

/// Fetch the next page of ArXiv results using offset-based pagination.
pub fn search_next_page(
    opts: &SearchOptions,
    pag: &PaginationState,
) -> Result<(Vec<SearchResult>, PaginationState), SearchError> {
    let client = build_client(opts)?;
    let next_page = pag.page + 1;
    let start = next_page * RESULTS_PER_PAGE;
    let query = format!("all:{}", opts.keywords);

    let body = client
        .get(ARXIV_API_URL)
        .query(&[
            ("search_query", query.as_str()),
            ("start", &start.to_string()),
            ("max_results", &RESULTS_PER_PAGE.to_string()),
            ("sortBy", "relevance"),
            ("sortOrder", "descending"),
        ])
        .send()?
        .text()?;

    let cur_offset = if pag.cur_index > 0 {
        pag.cur_index as usize - 1
    } else {
        0
    };
    let feed = parser::parse(&body, cur_offset);

    let new_pag = PaginationState {
        engine: Engine::ArXiv,
        page: next_page,
        cur_index: pag.cur_index + feed.results.len() as i64,
        result_count: feed.results.len(),
        total_results: feed.total_results,
        user_agent: pag.user_agent.clone(),
        ..Default::default()
    };

    Ok((feed.results, new_pag))
}
