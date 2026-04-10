//! HTML parser for DuckDuckGo's lite search results page.
//!
//! Mirrors the state-machine approach from the Python `ddgr` project:
//! track nested HTML tags via annotation stacks, switching between
//! handler contexts (root -> result -> title / abstract -> ...) as
//! we descend into and out of relevant DOM regions.

use crate::SearchResult;

// ---------------------------------------------------------------------------
// Parser state machine
// ---------------------------------------------------------------------------

/// Which handler context we are currently in.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    Root,
    Result,
    Title,
    TitleLink,
    TitleFiletype,
    Abstract,
    Input,
}

/// An annotation pushed onto the per-tag stack.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Annotation {
    None,
    ClickResult,
    Result,
    Title,
    TitleLink,
    TitleFiletype,
    Abstract,
    Input,
}

/// DuckDuckGo HTML parser targeting `html.duckduckgo.com/html` responses.
pub struct DdgParser {
    // Current scope
    scope: Scope,

    // Per-tag annotation stacks (keyed by tag name)
    annotations: std::collections::HashMap<String, Vec<Annotation>>,

    // Text buffer for accumulating data
    textbuf: String,
    recording: bool,

    // Current result fields
    current_title: String,
    current_url: String,
    current_abstract: String,
    current_filetype: String,

    // Pagination tokens
    pub np_prev: String,
    pub np_next: String,
    np_found: bool,
    pub vqd: String,

    // Click/instant answer result
    pub click_result: String,

    // Collected results
    pub results: Vec<SearchResult>,
    index_offset: usize,

    // Whether DDG returned a bot-detection / captcha page
    pub is_blocked: bool,
}

impl DdgParser {
    pub fn new(offset: usize) -> Self {
        Self {
            scope: Scope::Root,
            annotations: std::collections::HashMap::new(),
            textbuf: String::new(),
            recording: false,
            current_title: String::new(),
            current_url: String::new(),
            current_abstract: String::new(),
            current_filetype: String::new(),
            np_prev: String::new(),
            np_next: String::new(),
            np_found: false,
            vqd: String::new(),
            click_result: String::new(),
            results: Vec::new(),
            index_offset: offset,
            is_blocked: false,
        }
    }

    /// Parse a full HTML page.
    pub fn parse(&mut self, html: &str) {
        // We implement a lightweight, purpose-built HTML tokenizer rather
        // than pulling in a full HTML5 parser.  DuckDuckGo's lite page is
        // simple enough that we only need to handle open tags, close tags,
        // and text data.
        let bytes = html.as_bytes();
        let len = bytes.len();
        let mut i = 0;

        while i < len {
            if bytes[i] == b'<' {
                // Find end of tag
                if let Some(close) = memchr_from(b'>', bytes, i + 1) {
                    let tag_content = &html[i + 1..close];
                    if tag_content.starts_with('/') {
                        // Close tag
                        let tag_name = tag_content[1..]
                            .split_whitespace()
                            .next()
                            .unwrap_or("")
                            .to_ascii_lowercase();
                        self.handle_end_tag(&tag_name);
                    } else if tag_content.starts_with('!') || tag_content.starts_with('?') {
                        // Comment / doctype / processing instruction – skip
                    } else {
                        // Open tag (possibly self-closing)
                        let self_closing = tag_content.ends_with('/');
                        let raw = if self_closing {
                            &tag_content[..tag_content.len() - 1]
                        } else {
                            tag_content
                        };
                        let (tag_name, attrs) = parse_tag(raw);
                        self.handle_start_tag(&tag_name, &attrs);
                        if self_closing {
                            self.handle_end_tag(&tag_name);
                        }
                    }
                    i = close + 1;
                } else {
                    // Malformed – treat rest as text
                    if self.recording {
                        self.textbuf.push_str(&html[i..]);
                    }
                    break;
                }
            } else {
                // Text data: collect until next '<'
                let start = i;
                while i < len && bytes[i] != b'<' {
                    i += 1;
                }
                if self.recording {
                    let text = &html[start..i];
                    let decoded = decode_entities(text);
                    self.textbuf.push_str(&decoded);
                }
            }
        }
    }

    // ----- annotation helpers -----

    fn push_annotation(&mut self, tag: &str, ann: Annotation) {
        self.annotations
            .entry(tag.to_string())
            .or_default()
            .push(ann);
    }

    fn pop_annotation(&mut self, tag: &str) -> Annotation {
        self.annotations
            .get_mut(tag)
            .and_then(|stack| stack.pop())
            .unwrap_or(Annotation::None)
    }

    // ----- text buffer helpers -----

    fn start_recording(&mut self) {
        self.recording = true;
    }

    fn stop_recording(&mut self) {
        self.recording = false;
    }

    fn pop_textbuf(&mut self) -> String {
        let text = self.textbuf.trim().to_string();
        self.textbuf.clear();
        text
    }

    // ----- tag handlers -----

    fn handle_start_tag(&mut self, tag: &str, attrs: &Attrs) {
        match self.scope {
            Scope::Root => self.root_start(tag, attrs),
            Scope::Result => self.result_start(tag, attrs),
            Scope::Title => self.title_start(tag, attrs),
            Scope::TitleLink => self.title_link_start(tag, attrs),
            Scope::TitleFiletype => {
                self.push_annotation(tag, Annotation::None);
            }
            Scope::Abstract => self.abstract_start(tag, attrs),
            Scope::Input => self.input_start(tag, attrs),
        }
    }

    fn handle_end_tag(&mut self, tag: &str) {
        match self.scope {
            Scope::Root => self.root_end(tag),
            Scope::Result => self.result_end(tag),
            Scope::Title => self.title_end(tag),
            Scope::TitleLink => self.title_link_end(tag),
            Scope::TitleFiletype => self.title_filetype_end(tag),
            Scope::Abstract => self.abstract_end(tag),
            Scope::Input => self.input_end(tag),
        }
    }

    // -- ROOT scope --

    fn root_start(&mut self, tag: &str, attrs: &Attrs) {
        if tag == "div" {
            let classes = get_classes(attrs);
            if classes.iter().any(|c| c == "anomaly-modal__mask" || c == "anomaly-modal__modal") {
                self.is_blocked = true;
                self.push_annotation(tag, Annotation::None);
                return;
            }
            if classes.iter().any(|c| c == "zci__result") {
                self.start_recording();
                self.push_annotation(tag, Annotation::ClickResult);
                return;
            }
            if classes.iter().any(|c| c == "links_main") {
                self.current_title.clear();
                self.current_url.clear();
                self.current_abstract.clear();
                self.current_filetype.clear();
                self.scope = Scope::Result;
                self.push_annotation(tag, Annotation::Result);
                return;
            }
            if classes.iter().any(|c| c == "nav-link") {
                self.scope = Scope::Input;
                self.push_annotation(tag, Annotation::Input);
                return;
            }
        }
        self.push_annotation(tag, Annotation::None);
    }

    fn root_end(&mut self, tag: &str) {
        let ann = self.pop_annotation(tag);
        if ann == Annotation::ClickResult {
            self.stop_recording();
            self.click_result = self.pop_textbuf();
        }
    }

    // -- RESULT scope --

    fn result_start(&mut self, tag: &str, attrs: &Attrs) {
        if tag == "h2" {
            let classes = get_classes(attrs);
            if classes.iter().any(|c| c == "result__title") {
                self.scope = Scope::Title;
                self.push_annotation(tag, Annotation::Title);
                return;
            }
        }
        if tag == "a" {
            let classes = get_classes(attrs);
            if classes.iter().any(|c| c == "result__snippet") {
                self.start_recording();
                self.push_annotation(tag, Annotation::Abstract);
                return;
            }
        }
        self.push_annotation(tag, Annotation::None);
    }

    fn result_end(&mut self, tag: &str) {
        let ann = self.pop_annotation(tag);
        match ann {
            Annotation::Abstract => {
                self.stop_recording();
                self.current_abstract = self.pop_textbuf();
            }
            Annotation::Result => {
                if !self.current_url.is_empty() {
                    let idx = self.index_offset + self.results.len() + 1;
                    self.results.push(SearchResult {
                        index: idx,
                        title: self.current_title.clone(),
                        url: self.current_url.clone(),
                        abstract_text: self.current_abstract.clone(),
                    });
                }
                self.scope = Scope::Root;
            }
            _ => {}
        }
    }

    // -- TITLE scope --

    fn title_start(&mut self, tag: &str, attrs: &Attrs) {
        if tag == "a" {
            if let Some(href) = attrs.get("href") {
                if href.starts_with("/search") {
                    self.push_annotation(tag, Annotation::None);
                    return;
                }
                self.current_url = clean_url(href);
                self.start_recording();
                self.scope = Scope::TitleLink;
                self.push_annotation(tag, Annotation::TitleLink);
                return;
            }
        }
        self.push_annotation(tag, Annotation::None);
    }

    fn title_end(&mut self, tag: &str) {
        let ann = self.pop_annotation(tag);
        if ann == Annotation::Title {
            self.scope = Scope::Result;
        }
    }

    // -- TITLE LINK scope --

    fn title_link_start(&mut self, tag: &str, _attrs: &Attrs) {
        if tag == "span" {
            // Potential filetype indicator like [PDF]
            self.stop_recording();
            // Save what we have so far as partial title
            let partial = self.pop_textbuf();
            self.current_title = partial;
            self.start_recording();
            self.scope = Scope::TitleFiletype;
            self.push_annotation(tag, Annotation::TitleFiletype);
            return;
        }
        self.push_annotation(tag, Annotation::None);
    }

    fn title_link_end(&mut self, tag: &str) {
        let ann = self.pop_annotation(tag);
        if ann == Annotation::TitleLink {
            self.stop_recording();
            let text = self.pop_textbuf();
            if self.current_filetype.is_empty() {
                self.current_title = text;
            } else {
                // Prepend filetype indicator
                self.current_title =
                    format!("{} {}", self.current_filetype, text);
                if self.current_title == format!("{} ", self.current_filetype) {
                    // No additional text after filetype, use what we had
                    self.current_title = format!(
                        "{} {}",
                        self.current_filetype,
                        self.current_title.trim()
                    );
                }
            }
            self.scope = Scope::Title;
        }
    }

    // -- TITLE FILETYPE scope --

    fn title_filetype_end(&mut self, tag: &str) {
        let ann = self.pop_annotation(tag);
        if ann == Annotation::TitleFiletype {
            self.stop_recording();
            let ft = self.pop_textbuf();
            if !ft.is_empty() {
                self.current_filetype = format!("[{}]", ft);
            }
            // Resume recording for the rest of the title
            self.start_recording();
            self.scope = Scope::TitleLink;
        }
    }

    // -- ABSTRACT scope --

    fn abstract_start(&mut self, tag: &str, _attrs: &Attrs) {
        // Keep recording through child tags
        self.push_annotation(tag, Annotation::None);
    }

    fn abstract_end(&mut self, tag: &str) {
        let ann = self.pop_annotation(tag);
        if ann == Annotation::Abstract {
            self.stop_recording();
            self.current_abstract = self.pop_textbuf();
            self.scope = Scope::Result;
        }
    }

    // -- INPUT scope (pagination nav) --

    fn input_start(&mut self, tag: &str, attrs: &Attrs) {
        if tag == "input" {
            if let Some(name) = attrs.get("name") {
                if name == "nextParams" {
                    if let Some(value) = attrs.get("value") {
                        if self.np_found {
                            self.np_prev = self.np_next.clone();
                        } else {
                            self.np_found = true;
                        }
                        self.np_next = value.clone();
                    }
                }
                if name == "vqd" {
                    if let Some(value) = attrs.get("value") {
                        if !value.is_empty() {
                            self.vqd = value.clone();
                        }
                    }
                }
            }
        }
        self.push_annotation(tag, Annotation::None);
    }

    fn input_end(&mut self, tag: &str) {
        let ann = self.pop_annotation(tag);
        if ann == Annotation::Input {
            self.scope = Scope::Root;
        }
    }
}

// ---------------------------------------------------------------------------
// Lightweight HTML helpers
// ---------------------------------------------------------------------------

type Attrs = std::collections::HashMap<String, String>;

/// Parse a tag body like `div class="foo" id="bar"` into (tag_name, attrs).
fn parse_tag(raw: &str) -> (String, Attrs) {
    let raw = raw.trim();
    let mut attrs = Attrs::new();

    // Split tag name from the rest
    let (tag_name, rest) = match raw.find(|c: char| c.is_whitespace()) {
        Some(pos) => (&raw[..pos], &raw[pos + 1..]),
        None => (raw, ""),
    };

    let tag_name = tag_name.to_ascii_lowercase();

    // Parse attributes (simple state machine)
    let rest = rest.trim();
    if !rest.is_empty() {
        parse_attrs(rest, &mut attrs);
    }

    (tag_name, attrs)
}

fn parse_attrs(input: &str, attrs: &mut Attrs) {
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Skip whitespace
        while i < len && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= len {
            break;
        }

        // Read attribute name
        let name_start = i;
        while i < len && chars[i] != '=' && !chars[i].is_whitespace() {
            i += 1;
        }
        let name = chars[name_start..i].iter().collect::<String>().to_ascii_lowercase();

        // Skip whitespace
        while i < len && chars[i].is_whitespace() {
            i += 1;
        }

        if i < len && chars[i] == '=' {
            i += 1; // skip '='
            // Skip whitespace
            while i < len && chars[i].is_whitespace() {
                i += 1;
            }

            if i < len && (chars[i] == '"' || chars[i] == '\'') {
                let quote = chars[i];
                i += 1;
                let val_start = i;
                while i < len && chars[i] != quote {
                    i += 1;
                }
                let value: String = chars[val_start..i].iter().collect();
                attrs.insert(name, decode_entities(&value));
                if i < len {
                    i += 1; // skip closing quote
                }
            } else {
                // Unquoted value
                let val_start = i;
                while i < len && !chars[i].is_whitespace() {
                    i += 1;
                }
                let value: String = chars[val_start..i].iter().collect();
                attrs.insert(name, decode_entities(&value));
            }
        } else {
            // Boolean attribute
            attrs.insert(name, String::new());
        }
    }
}

fn get_classes(attrs: &Attrs) -> Vec<String> {
    attrs
        .get("class")
        .map(|c| c.split_whitespace().map(|s| s.to_string()).collect())
        .unwrap_or_default()
}

fn clean_url(href: &str) -> String {
    // DuckDuckGo sometimes wraps URLs in a redirect: //duckduckgo.com/l/?uddg=...
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

/// Decode HTML character and numeric entity references.
fn decode_entities(text: &str) -> String {
    html_escape::decode_html_entities(text).into_owned()
}

/// Find a byte in a slice starting from position `from`.
fn memchr_from(needle: u8, haystack: &[u8], from: usize) -> Option<usize> {
    for i in from..haystack.len() {
        if haystack[i] == needle {
            return Some(i);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Parser unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tag_simple() {
        let (name, attrs) = parse_tag("div class=\"foo bar\"");
        assert_eq!(name, "div");
        assert_eq!(attrs.get("class").unwrap(), "foo bar");
    }

    #[test]
    fn test_parse_tag_no_attrs() {
        let (name, attrs) = parse_tag("br");
        assert_eq!(name, "br");
        assert!(attrs.is_empty());
    }

    #[test]
    fn test_decode_entities() {
        assert_eq!(decode_entities("&amp;"), "&");
        assert_eq!(decode_entities("&lt;b&gt;"), "<b>");
        assert_eq!(decode_entities("&#39;"), "'");
        assert_eq!(decode_entities("plain text"), "plain text");
    }

    #[test]
    fn test_clean_url_with_uddg() {
        let href = "//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com&rut=abc";
        let cleaned = clean_url(href);
        assert_eq!(cleaned, "https://example.com");
    }

    #[test]
    fn test_clean_url_plain() {
        let href = "https://example.com/page";
        assert_eq!(clean_url(href), href);
    }

    #[test]
    fn test_parser_minimal_html() {
        let html = r##"
        <div class="links_main">
          <h2 class="result__title">
            <a href="https://example.com">Example Title</a>
          </h2>
          <a class="result__snippet" href="#">This is the abstract text.</a>
        </div>
        "##;
        let mut parser = DdgParser::new(0);
        parser.parse(html);
        assert_eq!(parser.results.len(), 1);
        assert_eq!(parser.results[0].title, "Example Title");
        assert_eq!(parser.results[0].url, "https://example.com");
        assert_eq!(parser.results[0].abstract_text, "This is the abstract text.");
    }

    #[test]
    fn test_parser_multiple_results() {
        let html = r##"
        <div class="links_main">
          <h2 class="result__title">
            <a href="https://one.com">First</a>
          </h2>
          <a class="result__snippet" href="#">Abstract one</a>
        </div>
        <div class="links_main">
          <h2 class="result__title">
            <a href="https://two.com">Second</a>
          </h2>
          <a class="result__snippet" href="#">Abstract two</a>
        </div>
        "##;
        let mut parser = DdgParser::new(0);
        parser.parse(html);
        assert_eq!(parser.results.len(), 2);
        assert_eq!(parser.results[0].index, 1);
        assert_eq!(parser.results[1].index, 2);
        assert_eq!(parser.results[0].title, "First");
        assert_eq!(parser.results[1].title, "Second");
    }

    #[test]
    fn test_parser_pagination_tokens() {
        let html = r##"
        <div class="nav-link">
          <input name="nextParams" value="abc123">
          <input name="vqd" value="vqd_token_42">
        </div>
        "##;
        let mut parser = DdgParser::new(0);
        parser.parse(html);
        assert_eq!(parser.np_next, "abc123");
        assert_eq!(parser.vqd, "vqd_token_42");
    }

    #[test]
    fn test_parser_click_result() {
        let html = r##"
        <div class="zci__result">Instant answer text here</div>
        "##;
        let mut parser = DdgParser::new(0);
        parser.parse(html);
        assert_eq!(parser.click_result, "Instant answer text here");
    }

    #[test]
    fn test_url_decode() {
        assert_eq!(
            url_decode("https%3A%2F%2Fexample.com%2Fpath"),
            "https://example.com/path"
        );
    }

    #[test]
    fn test_parser_blocked_detection() {
        let html = r##"
        <html>
        <body>
          <div class="anomaly-modal__mask">
            <div class="anomaly-modal__modal">
              <p>If this error persists, please let us know.</p>
            </div>
          </div>
        </body>
        </html>
        "##;
        let mut parser = DdgParser::new(0);
        parser.parse(html);
        assert!(parser.is_blocked, "parser should detect anomaly-modal as blocked");
        assert!(parser.results.is_empty(), "blocked page should have no results");
    }

    #[test]
    fn test_parser_uddg_redirect_url() {
        let html = r##"
        <div class="links_main">
          <h2 class="result__title">
            <a href="//duckduckgo.com/l/?uddg=https%3A%2F%2Freal-site.com%2Fpage&rut=xyz">Real Site</a>
          </h2>
          <a class="result__snippet" href="#">Some text</a>
        </div>
        "##;
        let mut parser = DdgParser::new(0);
        parser.parse(html);
        assert_eq!(parser.results.len(), 1);
        assert_eq!(parser.results[0].url, "https://real-site.com/page");
    }
}
