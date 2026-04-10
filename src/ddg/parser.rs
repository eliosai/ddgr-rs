//! HTML parser for DuckDuckGo's lite search results page.

use scraper::{Html, Selector};

use crate::SearchResult;

/// Parsed output from a DuckDuckGo HTML response.
pub struct ParsedPage {
    pub results: Vec<SearchResult>,
    pub np_prev: String,
    pub np_next: String,
    pub vqd: String,
    pub is_blocked: bool,
}

/// Parse a DuckDuckGo HTML results page into structured data.
pub fn parse(html: &str, offset: usize) -> ParsedPage {
    let document = Html::parse_document(html);
    let mut page = ParsedPage {
        results: Vec::new(),
        np_prev: String::new(),
        np_next: String::new(),
        vqd: String::new(),
        is_blocked: false,
    };

    // Bot / captcha detection
    let blocked = sel(".anomaly-modal__mask, .anomaly-modal__modal");
    if document.select(&blocked).next().is_some() {
        page.is_blocked = true;
        return page;
    }

    // Search results
    let result_div = sel("div.links_main");
    let title_link = sel("h2.result__title a");
    let snippet = sel("a.result__snippet");

    for div in document.select(&result_div) {
        let Some(a) = div.select(&title_link).next() else {
            continue;
        };

        let href = a.value().attr("href").unwrap_or_default();
        if href.starts_with("/search") {
            continue;
        }

        let title = collect_text(&a);
        let url = clean_url(href);
        let abstract_text = div
            .select(&snippet)
            .next()
            .map(|e| collect_text(&e))
            .unwrap_or_default();

        page.results.push(SearchResult {
            index: offset + page.results.len() + 1,
            title,
            url,
            abstract_text,
        });
    }

    // Pagination tokens (scoped to nav-link sections)
    let np_sel = sel("div.nav-link input[name='nextParams']");
    let vqd_sel = sel("div.nav-link input[name='vqd']");

    let np_values: Vec<&str> = document
        .select(&np_sel)
        .filter_map(|e| e.value().attr("value"))
        .collect();

    match np_values.len() {
        0 => {}
        1 => page.np_next = np_values[0].to_string(),
        n => {
            page.np_prev = np_values[n - 2].to_string();
            page.np_next = np_values[n - 1].to_string();
        }
    }

    for elem in document.select(&vqd_sel) {
        if let Some(v) = elem.value().attr("value") {
            if !v.is_empty() {
                page.vqd = v.to_string();
            }
        }
    }

    page
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn sel(s: &str) -> Selector {
    Selector::parse(s).expect("invalid CSS selector")
}

fn collect_text(elem: &scraper::ElementRef) -> String {
    elem.text().collect::<String>().trim().to_string()
}

/// Unwrap DuckDuckGo's tracking redirect URLs.
/// DDG wraps URLs as `//duckduckgo.com/l/?uddg=<encoded_url>&rut=...`.
fn clean_url(href: &str) -> String {
    if let Some(pos) = href.find("uddg=") {
        let start = pos + 5;
        let end = href[start..]
            .find('&')
            .map(|p| start + p)
            .unwrap_or(href.len());
        return url_decode(&href[start..end]);
    }
    href.to_string()
}

fn url_decode(s: &str) -> String {
    url::form_urlencoded::parse(s.as_bytes())
        .map(|(k, v)| {
            if v.is_empty() {
                k.into_owned()
            } else {
                format!("{}={}", k, v)
            }
        })
        .collect::<Vec<_>>()
        .join("&")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_html() {
        let html = r##"
        <div class="links_main">
          <h2 class="result__title">
            <a href="https://example.com">Example Title</a>
          </h2>
          <a class="result__snippet" href="#">This is the abstract text.</a>
        </div>
        "##;
        let page = parse(html, 0);
        assert_eq!(page.results.len(), 1);
        assert_eq!(page.results[0].title, "Example Title");
        assert_eq!(page.results[0].url, "https://example.com");
        assert_eq!(page.results[0].abstract_text, "This is the abstract text.");
    }

    #[test]
    fn test_parse_multiple_results() {
        let html = r##"
        <div class="links_main">
          <h2 class="result__title"><a href="https://one.com">First</a></h2>
          <a class="result__snippet" href="#">Abstract one</a>
        </div>
        <div class="links_main">
          <h2 class="result__title"><a href="https://two.com">Second</a></h2>
          <a class="result__snippet" href="#">Abstract two</a>
        </div>
        "##;
        let page = parse(html, 0);
        assert_eq!(page.results.len(), 2);
        assert_eq!(page.results[0].index, 1);
        assert_eq!(page.results[1].index, 2);
    }

    #[test]
    fn test_parse_pagination_tokens() {
        let html = r##"
        <div class="nav-link">
          <input name="nextParams" value="abc123">
          <input name="vqd" value="vqd_token_42">
        </div>
        "##;
        let page = parse(html, 0);
        assert_eq!(page.np_next, "abc123");
        assert_eq!(page.vqd, "vqd_token_42");
    }

    #[test]
    fn test_parse_blocked_detection() {
        let html = r##"
        <div class="anomaly-modal__mask"><div class="anomaly-modal__modal"></div></div>
        "##;
        let page = parse(html, 0);
        assert!(page.is_blocked);
        assert!(page.results.is_empty());
    }

    #[test]
    fn test_parse_uddg_redirect() {
        let html = r##"
        <div class="links_main">
          <h2 class="result__title">
            <a href="//duckduckgo.com/l/?uddg=https%3A%2F%2Freal-site.com%2Fpage&rut=xyz">Real</a>
          </h2>
          <a class="result__snippet" href="#">Text</a>
        </div>
        "##;
        let page = parse(html, 0);
        assert_eq!(page.results[0].url, "https://real-site.com/page");
    }

    #[test]
    fn test_clean_url_with_uddg() {
        assert_eq!(
            clean_url("//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com&rut=abc"),
            "https://example.com"
        );
    }

    #[test]
    fn test_clean_url_plain() {
        assert_eq!(clean_url("https://example.com/page"), "https://example.com/page");
    }

    #[test]
    fn test_url_decode() {
        assert_eq!(
            url_decode("https%3A%2F%2Fexample.com%2Fpath"),
            "https://example.com/path"
        );
    }

    #[test]
    fn test_parse_with_offset() {
        let html = r##"
        <div class="links_main">
          <h2 class="result__title"><a href="https://example.com">Title</a></h2>
          <a class="result__snippet" href="#">Text</a>
        </div>
        "##;
        let page = parse(html, 10);
        assert_eq!(page.results[0].index, 11);
    }

    #[test]
    fn test_parse_two_pagination_buttons() {
        let html = r##"
        <div class="nav-link"><input name="nextParams" value="prev_token"></div>
        <div class="nav-link"><input name="nextParams" value="next_token"></div>
        "##;
        let page = parse(html, 0);
        assert_eq!(page.np_prev, "prev_token");
        assert_eq!(page.np_next, "next_token");
    }
}
