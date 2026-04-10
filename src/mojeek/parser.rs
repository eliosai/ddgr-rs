//! HTML parser for Mojeek search results pages.

use scraper::{Html, Selector};

use crate::SearchResult;

/// Parse a Mojeek HTML results page into structured results.
pub fn parse(html: &str, offset: usize) -> Vec<SearchResult> {
    let document = Html::parse_document(html);
    let mut results = Vec::new();

    let result_sel = sel("ul.results-standard li");
    let title_sel = sel("a.title");
    let snippet_sel = sel("p.s");

    for li in document.select(&result_sel) {
        let Some(a) = li.select(&title_sel).next() else {
            continue;
        };

        let title = collect_text(&a);
        let url = a.value().attr("href").unwrap_or_default().to_string();
        if url.is_empty() || title.is_empty() {
            continue;
        }

        let abstract_text = li
            .select(&snippet_sel)
            .next()
            .map(|e| collect_text(&e))
            .unwrap_or_default();

        results.push(SearchResult {
            index: offset + results.len() + 1,
            title,
            url,
            abstract_text,
        });
    }

    results
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mojeek_results() {
        let html = r##"
        <ul class="results-standard">
          <li>
            <a class="title" href="https://example.com">Example Title</a>
            <p class="s">This is the snippet text.</p>
          </li>
          <li>
            <a class="title" href="https://other.com">Other Title</a>
            <p class="s">Another snippet.</p>
          </li>
        </ul>
        "##;
        let results = parse(html, 0);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Example Title");
        assert_eq!(results[0].url, "https://example.com");
        assert_eq!(results[0].abstract_text, "This is the snippet text.");
        assert_eq!(results[0].index, 1);
        assert_eq!(results[1].title, "Other Title");
        assert_eq!(results[1].index, 2);
    }

    #[test]
    fn test_parse_with_offset() {
        let html = r##"
        <ul class="results-standard">
          <li>
            <a class="title" href="https://example.com">Title</a>
            <p class="s">Text</p>
          </li>
        </ul>
        "##;
        let results = parse(html, 10);
        assert_eq!(results[0].index, 11);
    }

    #[test]
    fn test_parse_empty_html() {
        let results = parse("<html><body></body></html>", 0);
        assert!(results.is_empty());
    }

    #[test]
    fn test_parse_skips_empty_entries() {
        let html = r##"
        <ul class="results-standard">
          <li>
            <a class="title" href="">Empty URL</a>
            <p class="s">Text</p>
          </li>
          <li>
            <a class="title" href="https://valid.com">Valid</a>
            <p class="s">Valid text</p>
          </li>
        </ul>
        "##;
        let results = parse(html, 0);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Valid");
    }

    #[test]
    fn test_parse_missing_snippet() {
        let html = r##"
        <ul class="results-standard">
          <li>
            <a class="title" href="https://example.com">No Snippet</a>
          </li>
        </ul>
        "##;
        let results = parse(html, 0);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].abstract_text, "");
    }
}
