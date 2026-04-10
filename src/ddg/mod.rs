//! DuckDuckGo search engine — queries html.duckduckgo.com/html (lite endpoint).

pub mod parser;

use std::collections::HashMap;

use crate::{build_client, Engine, PaginationState, SearchError, SearchOptions, SearchResult};

const DDG_URL: &str = "https://html.duckduckgo.com/html";

/// Fetch a single page of DuckDuckGo results (first page).
pub fn search_page(
    opts: &SearchOptions,
) -> Result<(Vec<SearchResult>, PaginationState), SearchError> {
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

    let body = client
        .post(DDG_URL)
        .header("DNT", "1")
        .form(&form)
        .send()?
        .text()?;

    let page = parser::parse(&body, 0);

    if page.is_blocked {
        return Err(SearchError::Blocked);
    }

    let pag = PaginationState {
        engine: Engine::DuckDuckGo,
        page: 0,
        cur_index: 1 + page.results.len() as i64,
        next_params: page.np_next,
        prev_params: page.np_prev,
        vqd: page.vqd,
        user_agent: opts.user_agent.clone(),
        result_count: page.results.len(),
    };

    Ok((page.results, pag))
}

/// Fetch the next page of DuckDuckGo results.
///
/// Uses the User-Agent from the pagination state to match the vqd token
/// (DDG ties vqd to the UA — a mismatch causes silent failures).
pub fn search_next_page(
    opts: &SearchOptions,
    pag: &PaginationState,
) -> Result<(Vec<SearchResult>, PaginationState), SearchError> {
    // Enforce consistent UA: DDG's vqd token is tied to the User-Agent.
    let mut session_opts = opts.clone();
    if !pag.user_agent.is_empty() {
        session_opts.user_agent = pag.user_agent.clone();
    }

    let client = build_client(&session_opts)?;
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

    let body = client
        .post(DDG_URL)
        .header("DNT", "1")
        .form(&form)
        .send()?
        .text()?;

    let offset = if pag.cur_index > 0 {
        pag.cur_index as usize - 1
    } else {
        0
    };
    let page = parser::parse(&body, offset);

    if page.is_blocked {
        return Err(SearchError::Blocked);
    }

    let new_pag = PaginationState {
        engine: Engine::DuckDuckGo,
        page: next_page,
        cur_index: pag.cur_index + page.results.len() as i64,
        next_params: page.np_next,
        prev_params: page.np_prev,
        vqd: if page.vqd.is_empty() {
            pag.vqd.clone()
        } else {
            page.vqd
        },
        user_agent: pag.user_agent.clone(),
        result_count: page.results.len(),
    };

    Ok((page.results, new_pag))
}
