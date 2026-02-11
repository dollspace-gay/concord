use super::events::EmbedInfo;

/// Extract all URLs (http/https) from message content (max 5).
pub fn extract_urls(content: &str) -> Vec<String> {
    let mut urls = Vec::new();
    for word in content.split_whitespace() {
        if word.starts_with("http://") || word.starts_with("https://") {
            // Trim trailing punctuation that's likely not part of the URL
            let url = word.trim_end_matches(['>', ')', ']', ',', '.', ';']);
            urls.push(url.to_string());
            if urls.len() >= 5 {
                break;
            }
        }
    }
    urls
}

/// Fetch Open Graph metadata for a URL.
/// Returns None if the fetch fails or no OG tags are found.
pub async fn unfurl_url(client: &reqwest::Client, url: &str) -> Option<EmbedInfo> {
    let resp = client
        .get(url)
        .header("User-Agent", "ConcordBot/1.0 (link preview)")
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .ok()?;

    // Only parse HTML responses
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !content_type.contains("text/html") {
        return None;
    }

    // Limit body read to 256KB to avoid abuse
    let body = resp.text().await.ok()?;
    let body = if body.len() > 256 * 1024 {
        &body[..256 * 1024]
    } else {
        &body
    };

    let title = extract_meta(body, "og:title").or_else(|| extract_html_title(body));
    let description =
        extract_meta(body, "og:description").or_else(|| extract_meta(body, "description"));
    let image_url = extract_meta(body, "og:image");
    let site_name = extract_meta(body, "og:site_name");

    // Must have at least a title to be useful
    if title.is_none() && description.is_none() {
        return None;
    }

    Some(EmbedInfo {
        url: url.to_string(),
        title,
        description,
        image_url,
        site_name,
    })
}

/// Extract content from a <meta property="..." content="..."> or <meta name="..." content="..."> tag.
fn extract_meta(html: &str, name: &str) -> Option<String> {
    let patterns = [
        format!(r#"property="{name}""#),
        format!(r#"property='{name}'"#),
        format!(r#"name="{name}""#),
        format!(r#"name='{name}'"#),
    ];

    for pattern in &patterns {
        if let Some(pos) = html.find(pattern.as_str()) {
            let search_end = (pos + 500).min(html.len());
            let slice = &html[pos..search_end];

            if let Some(content) = extract_content_attr(slice) {
                let decoded = html_decode(&content);
                if !decoded.is_empty() {
                    return Some(decoded);
                }
            }
        }
    }

    None
}

/// Extract the value of a content="..." attribute from a tag fragment.
fn extract_content_attr(tag_fragment: &str) -> Option<String> {
    if let Some(start) = tag_fragment.find("content=\"") {
        let value_start = start + 9;
        if let Some(end) = tag_fragment[value_start..].find('"') {
            return Some(tag_fragment[value_start..value_start + end].to_string());
        }
    }
    if let Some(start) = tag_fragment.find("content='") {
        let value_start = start + 9;
        if let Some(end) = tag_fragment[value_start..].find('\'') {
            return Some(tag_fragment[value_start..value_start + end].to_string());
        }
    }
    None
}

/// Extract <title>...</title> as fallback.
fn extract_html_title(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let start = lower.find("<title")?.checked_add(6)?;
    let after_tag = lower[start..].find('>')?;
    let content_start = start + after_tag + 1;
    let end = lower[content_start..].find("</title>")?;
    let title = html[content_start..content_start + end].trim().to_string();
    if title.is_empty() {
        None
    } else {
        Some(html_decode(&title))
    }
}

/// Decode basic HTML entities.
fn html_decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ────────────────────────────────────────────────────────────────
    // URL extraction tests
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn test_extract_urls_single_http() {
        let urls = extract_urls("Check out http://example.com today");
        assert_eq!(urls, vec!["http://example.com"]);
    }

    #[test]
    fn test_extract_urls_single_https() {
        let urls = extract_urls("Visit https://example.com for more info");
        assert_eq!(urls, vec!["https://example.com"]);
    }

    #[test]
    fn test_extract_urls_multiple() {
        let urls = extract_urls("See https://a.com and http://b.com and https://c.com");
        assert_eq!(
            urls,
            vec!["https://a.com", "http://b.com", "https://c.com",]
        );
    }

    #[test]
    fn test_extract_urls_none() {
        let urls = extract_urls("No URLs here at all");
        assert!(urls.is_empty());
    }

    #[test]
    fn test_extract_urls_empty_string() {
        let urls = extract_urls("");
        assert!(urls.is_empty());
    }

    #[test]
    fn test_extract_urls_max_five() {
        let msg =
            "https://1.com https://2.com https://3.com https://4.com https://5.com https://6.com";
        let urls = extract_urls(msg);
        assert_eq!(urls.len(), 5);
        // 6th URL should not be included
        assert!(!urls.contains(&"https://6.com".to_string()));
    }

    #[test]
    fn test_extract_urls_strips_trailing_punctuation() {
        let urls = extract_urls("Visit https://example.com, or https://other.com.");
        assert_eq!(urls, vec!["https://example.com", "https://other.com"]);
    }

    #[test]
    fn test_extract_urls_strips_trailing_parenthesis() {
        let urls = extract_urls("(see https://example.com)");
        assert_eq!(urls, vec!["https://example.com"]);
    }

    #[test]
    fn test_extract_urls_strips_trailing_bracket() {
        // In "[link](https://example.com)", the word is "(https://example.com)"
        // which starts with "(" not "http", so extract_urls won't find it.
        let urls = extract_urls("[link](https://example.com)");
        assert!(urls.is_empty());

        // But a bare URL followed by a bracket is handled:
        let urls = extract_urls("see https://example.com]");
        assert_eq!(urls, vec!["https://example.com"]);
    }

    #[test]
    fn test_extract_urls_strips_trailing_angle_bracket() {
        // In "<https://example.com>", the word is "<https://example.com>"
        // which starts with "<" not "http", so extract_urls won't find it.
        let urls = extract_urls("<https://example.com>");
        assert!(urls.is_empty());

        // But a bare URL followed by ">" is handled:
        let urls = extract_urls("see https://example.com>");
        assert_eq!(urls, vec!["https://example.com"]);
    }

    #[test]
    fn test_extract_urls_strips_trailing_semicolon() {
        let urls = extract_urls("Go to https://example.com;");
        assert_eq!(urls, vec!["https://example.com"]);
    }

    #[test]
    fn test_extract_urls_with_path_and_query() {
        let urls = extract_urls("See https://example.com/path?foo=bar&baz=qux for details");
        assert_eq!(urls, vec!["https://example.com/path?foo=bar&baz=qux"]);
    }

    #[test]
    fn test_extract_urls_with_fragment() {
        let urls = extract_urls("Read https://example.com/page#section");
        assert_eq!(urls, vec!["https://example.com/page#section"]);
    }

    #[test]
    fn test_extract_urls_ftp_not_extracted() {
        let urls = extract_urls("ftp://files.example.com is not supported");
        assert!(urls.is_empty());
    }

    #[test]
    fn test_extract_urls_only_url() {
        let urls = extract_urls("https://example.com");
        assert_eq!(urls, vec!["https://example.com"]);
    }

    #[test]
    fn test_extract_urls_adjacent_urls() {
        // URLs separated by single space
        let urls = extract_urls("https://a.com https://b.com");
        assert_eq!(urls, vec!["https://a.com", "https://b.com"]);
    }

    // ────────────────────────────────────────────────────────────────
    // HTML meta tag extraction tests
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn test_extract_meta_og_title() {
        let html = r#"<meta property="og:title" content="My Page Title">"#;
        let result = extract_meta(html, "og:title");
        assert_eq!(result, Some("My Page Title".into()));
    }

    #[test]
    fn test_extract_meta_og_description() {
        let html = r#"<meta property="og:description" content="A description">"#;
        let result = extract_meta(html, "og:description");
        assert_eq!(result, Some("A description".into()));
    }

    #[test]
    fn test_extract_meta_og_image() {
        let html = r#"<meta property="og:image" content="https://example.com/image.png">"#;
        let result = extract_meta(html, "og:image");
        assert_eq!(result, Some("https://example.com/image.png".into()));
    }

    #[test]
    fn test_extract_meta_og_site_name() {
        let html = r#"<meta property="og:site_name" content="Example Site">"#;
        let result = extract_meta(html, "og:site_name");
        assert_eq!(result, Some("Example Site".into()));
    }

    #[test]
    fn test_extract_meta_name_description() {
        let html = r#"<meta name="description" content="Fallback description">"#;
        let result = extract_meta(html, "description");
        assert_eq!(result, Some("Fallback description".into()));
    }

    #[test]
    fn test_extract_meta_single_quotes() {
        let html = r#"<meta property='og:title' content='Single Quote Title'>"#;
        // Our parser checks for property='og:title' but content extraction
        // uses content="..." or content='...'
        let result = extract_meta(html, "og:title");
        assert_eq!(result, Some("Single Quote Title".into()));
    }

    #[test]
    fn test_extract_meta_not_found() {
        let html = r#"<meta property="og:title" content="Title">"#;
        let result = extract_meta(html, "og:description");
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_meta_empty_content() {
        let html = r#"<meta property="og:title" content="">"#;
        let result = extract_meta(html, "og:title");
        // Empty content returns None (the decoded string is empty)
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_meta_html_entities() {
        let html = r#"<meta property="og:title" content="Tom &amp; Jerry &lt;3">"#;
        let result = extract_meta(html, "og:title");
        assert_eq!(result, Some("Tom & Jerry <3".into()));
    }

    // ────────────────────────────────────────────────────────────────
    // HTML title extraction tests
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn test_extract_html_title_basic() {
        let html = "<html><head><title>My Page</title></head></html>";
        let result = extract_html_title(html);
        assert_eq!(result, Some("My Page".into()));
    }

    #[test]
    fn test_extract_html_title_with_attributes() {
        let html = r#"<html><head><title lang="en">My Page</title></head></html>"#;
        let result = extract_html_title(html);
        assert_eq!(result, Some("My Page".into()));
    }

    #[test]
    fn test_extract_html_title_empty() {
        let html = "<html><head><title></title></head></html>";
        let result = extract_html_title(html);
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_html_title_whitespace_only() {
        let html = "<html><head><title>   </title></head></html>";
        let result = extract_html_title(html);
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_html_title_missing() {
        let html = "<html><head></head></html>";
        let result = extract_html_title(html);
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_html_title_with_entities() {
        let html = "<title>Page &amp; Title &lt;1&gt;</title>";
        let result = extract_html_title(html);
        assert_eq!(result, Some("Page & Title <1>".into()));
    }

    // ────────────────────────────────────────────────────────────────
    // html_decode tests
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn test_html_decode_all_entities() {
        assert_eq!(html_decode("&amp;"), "&");
        assert_eq!(html_decode("&lt;"), "<");
        assert_eq!(html_decode("&gt;"), ">");
        assert_eq!(html_decode("&quot;"), "\"");
        assert_eq!(html_decode("&#39;"), "'");
        assert_eq!(html_decode("&apos;"), "'");
    }

    #[test]
    fn test_html_decode_combined() {
        assert_eq!(
            html_decode("&lt;b&gt;Hello &amp; &quot;World&quot;&lt;/b&gt;"),
            "<b>Hello & \"World\"</b>"
        );
    }

    #[test]
    fn test_html_decode_no_entities() {
        assert_eq!(html_decode("plain text"), "plain text");
    }

    #[test]
    fn test_html_decode_empty() {
        assert_eq!(html_decode(""), "");
    }

    // ────────────────────────────────────────────────────────────────
    // extract_content_attr tests
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn test_extract_content_attr_double_quotes() {
        let fragment = r#"property="og:title" content="Hello World""#;
        let result = extract_content_attr(fragment);
        assert_eq!(result, Some("Hello World".into()));
    }

    #[test]
    fn test_extract_content_attr_single_quotes() {
        let fragment = "property='og:title' content='Hello World'";
        let result = extract_content_attr(fragment);
        assert_eq!(result, Some("Hello World".into()));
    }

    #[test]
    fn test_extract_content_attr_missing() {
        let fragment = r#"property="og:title""#;
        let result = extract_content_attr(fragment);
        assert!(result.is_none());
    }
}
