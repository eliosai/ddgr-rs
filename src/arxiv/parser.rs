//! XML parser for ArXiv API Atom feed responses.

use crate::SearchResult;

const ATOM_NS: &str = "http://www.w3.org/2005/Atom";
const OPENSEARCH_NS: &str = "http://a9.com/-/spec/opensearch/1.1/";

/// Parsed output from an ArXiv API response.
pub struct ParsedFeed {
    pub results: Vec<SearchResult>,
    pub total_results: usize,
}

/// Parse an ArXiv Atom XML feed into structured results.
pub fn parse(xml: &str, offset: usize) -> ParsedFeed {
    let mut feed = ParsedFeed {
        results: Vec::new(),
        total_results: 0,
    };

    let Ok(doc) = roxmltree::Document::parse(xml) else {
        return feed;
    };

    // Total results count (for pagination)
    if let Some(total) = doc
        .descendants()
        .find(|n| n.has_tag_name((OPENSEARCH_NS, "totalResults")))
        .and_then(|n| n.text())
        .and_then(|t| t.parse::<usize>().ok())
    {
        feed.total_results = total;
    }

    // Parse entries
    for entry in doc
        .descendants()
        .filter(|n| n.has_tag_name((ATOM_NS, "entry")))
    {
        let title = child_text(&entry, "title");
        let summary = child_text(&entry, "summary");

        // Use <id> as URL (always present, e.g. http://arxiv.org/abs/2306.04338v1)
        let url = entry
            .children()
            .find(|n| n.has_tag_name((ATOM_NS, "id")))
            .map(|n| text_content(n))
            .unwrap_or_default();

        if title.is_empty() || url.is_empty() {
            continue;
        }

        feed.results.push(SearchResult {
            index: offset + feed.results.len() + 1,
            title,
            url,
            abstract_text: summary,
        });
    }

    feed
}

/// Extract text from a named child element, normalizing whitespace.
fn child_text(parent: &roxmltree::Node, local_name: &str) -> String {
    parent
        .children()
        .find(|n| n.has_tag_name((ATOM_NS, local_name)))
        .map(|n| text_content(n))
        .unwrap_or_default()
}

/// Collect all descendant text nodes and normalize whitespace.
fn text_content(node: roxmltree::Node) -> String {
    let raw: String = node
        .descendants()
        .filter(|n| n.is_text())
        .filter_map(|n| n.text())
        .collect();
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom"
      xmlns:opensearch="http://a9.com/-/spec/opensearch/1.1/"
      xmlns:arxiv="http://arxiv.org/schemas/atom">
  <opensearch:totalResults>12345</opensearch:totalResults>
  <opensearch:startIndex>0</opensearch:startIndex>
  <opensearch:itemsPerPage>2</opensearch:itemsPerPage>
  <entry>
    <id>http://arxiv.org/abs/2306.04338v1</id>
    <title>  Machine Learning
      for Data Science  </title>
    <summary>  This paper explores machine learning
      techniques for data science applications.  </summary>
    <link href="https://arxiv.org/abs/2306.04338v1" rel="alternate" type="text/html"/>
    <link href="https://arxiv.org/pdf/2306.04338v1" rel="related" type="application/pdf" title="pdf"/>
    <published>2023-06-07T11:08:12Z</published>
  </entry>
  <entry>
    <id>http://arxiv.org/abs/2401.00001v2</id>
    <title>Deep Learning Advances</title>
    <summary>A survey of recent deep learning advances.</summary>
    <link href="https://arxiv.org/abs/2401.00001v2" rel="alternate" type="text/html"/>
  </entry>
</feed>"#;

    #[test]
    fn test_parse_feed() {
        let feed = parse(SAMPLE_FEED, 0);
        assert_eq!(feed.total_results, 12345);
        assert_eq!(feed.results.len(), 2);
    }

    #[test]
    fn test_parse_title_whitespace_normalized() {
        let feed = parse(SAMPLE_FEED, 0);
        assert_eq!(feed.results[0].title, "Machine Learning for Data Science");
    }

    #[test]
    fn test_parse_url_from_id() {
        let feed = parse(SAMPLE_FEED, 0);
        assert_eq!(feed.results[0].url, "http://arxiv.org/abs/2306.04338v1");
    }

    #[test]
    fn test_parse_abstract() {
        let feed = parse(SAMPLE_FEED, 0);
        assert_eq!(
            feed.results[0].abstract_text,
            "This paper explores machine learning techniques for data science applications."
        );
    }

    #[test]
    fn test_parse_indices() {
        let feed = parse(SAMPLE_FEED, 0);
        assert_eq!(feed.results[0].index, 1);
        assert_eq!(feed.results[1].index, 2);
    }

    #[test]
    fn test_parse_with_offset() {
        let feed = parse(SAMPLE_FEED, 10);
        assert_eq!(feed.results[0].index, 11);
        assert_eq!(feed.results[1].index, 12);
    }

    #[test]
    fn test_parse_invalid_xml() {
        let feed = parse("not xml at all", 0);
        assert!(feed.results.is_empty());
        assert_eq!(feed.total_results, 0);
    }

    #[test]
    fn test_parse_empty_feed() {
        let xml = r#"<?xml version="1.0"?>
        <feed xmlns="http://www.w3.org/2005/Atom"
              xmlns:opensearch="http://a9.com/-/spec/opensearch/1.1/">
          <opensearch:totalResults>0</opensearch:totalResults>
        </feed>"#;
        let feed = parse(xml, 0);
        assert!(feed.results.is_empty());
        assert_eq!(feed.total_results, 0);
    }
}
