//! Mojeek search engine — queries www.mojeek.com/search (GET, HTML).

pub mod parser;

use crate::{build_client, Engine, PaginationState, SearchError, SearchOptions, SearchResult};

const MOJEEK_URL: &str = "https://www.mojeek.com/search";

/// Mojeek returns ~10 results per page.
pub const RESULTS_PER_PAGE: usize = 10;

/// Fetch a single page of Mojeek results (first page).
pub fn search_page(
    opts: &SearchOptions,
) -> Result<(Vec<SearchResult>, PaginationState), SearchError> {
    let client = build_client(opts)?;
    let safe = if opts.safe >= 1 { "1" } else { "0" };

    let body = client
        .get(MOJEEK_URL)
        .query(&[("q", opts.keywords.as_str()), ("safe", safe)])
        .header("DNT", "1")
        .send()?
        .text()?;

    let results = parser::parse(&body, 0);

    let pag = PaginationState {
        engine: Engine::Mojeek,
        page: 0,
        cur_index: 1 + results.len() as i64,
        result_count: results.len(),
        user_agent: opts.user_agent.clone(),
        ..Default::default()
    };

    Ok((results, pag))
}

/// Fetch the next page of Mojeek results.
pub fn search_next_page(
    opts: &SearchOptions,
    pag: &PaginationState,
) -> Result<(Vec<SearchResult>, PaginationState), SearchError> {
    let client = build_client(opts)?;
    let next_page = pag.page + 1;
    let offset = next_page * RESULTS_PER_PAGE;
    let safe = if opts.safe >= 1 { "1" } else { "0" };

    let body = client
        .get(MOJEEK_URL)
        .query(&[
            ("q", opts.keywords.as_str()),
            ("s", &offset.to_string()),
            ("safe", safe),
        ])
        .header("DNT", "1")
        .send()?
        .text()?;

    let cur_offset = if pag.cur_index > 0 {
        pag.cur_index as usize - 1
    } else {
        0
    };
    let results = parser::parse(&body, cur_offset);

    let new_pag = PaginationState {
        engine: Engine::Mojeek,
        page: next_page,
        cur_index: pag.cur_index + results.len() as i64,
        result_count: results.len(),
        user_agent: pag.user_agent.clone(),
        ..Default::default()
    };

    Ok((results, new_pag))
}
