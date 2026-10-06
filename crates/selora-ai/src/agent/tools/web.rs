//! Web tools (ARCHITECTURE.md §17.1, §17.2) — Phase 5e's file.
//!
//! Two tools:
//!
//! - [`web_search`] — DuckDuckGo Lite HTML by default (no API key; the
//!   Lite result table is parsed by a small hand-rolled scanner, no
//!   heavyweight scraper crate). The Brave Search API is available as an
//!   alternate provider: pass the key in as a parameter (never from a
//!   file owned by this crate) — the router/loop reads it from settings
//!   and hands it over, so the key stays out of this module's state.
//! - [`web_fetch`] — a GET via the crate's existing `reqwest` stack
//!   (no new dependency), HTML stripped to text by a hand-rolled
//!   tag-stripper, output capped at 256 KiB (truncated with a notice),
//!   and the source URL recorded in the result.
//!
//! The §17.2 SSRF guard is non-negotiable and lives here in two halves:
//! [`is_url_allowed`] (pure — scheme + host checks, exportable so 5b's
//! router reuses it) and [`resolve_and_check`] (DNS — the hostname is
//! resolved and **every** address must be public, not just the literal).
//! Redirects are followed manually with the client's redirect policy
//! disabled, so each hop goes back through the guard.
//!
//! §17.2's secret rule: the Brave key (or any secret) never appears in a
//! tool result, log line, or error message — errors carry status codes
//! and hosts only.
//!
//! Results carry an empty `id`: the 5b router stamps the originating
//! `ToolCall::id` before appending the result to the conversation.
#![allow(dead_code)] // entry points are wired by the 5b router at merge

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use crate::chat_types::ToolResult;
use reqwest::{redirect, Client, Url};

/// The §17.2 output cap for `web_fetch` (256 KiB of text, truncated
/// with a notice when exceeded).
pub const MAX_TEXT_BYTES: usize = 256 * 1024;

/// How much raw body we are willing to buffer before stripping tags —
/// bounds memory; the stripped text is then capped at [`MAX_TEXT_BYTES`].
const MAX_RAW_BYTES: usize = 1024 * 1024;

/// Manual redirect limit — each hop re-enters the SSRF guard.
const MAX_REDIRECTS: usize = 5;

/// Maximum search results reported per query.
const MAX_RESULTS: usize = 10;

const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

const DDG_LITE_URL: &str = "https://lite.duckduckgo.com/lite/";
const BRAVE_SEARCH_URL: &str = "https://api.search.brave.com/res/v1/web/search";

// ---------------------------------------------------------------------------
// SSRF guard (§17.2)
// ---------------------------------------------------------------------------

/// The pure half of the §17.2 SSRF guard: no I/O, so 5b's router can
/// reuse it directly. Refuses anything that is not `http`/`https`, URLs
/// without a host, `localhost` (any case, and `*.localhost`), and any
/// host that is an IP literal in a private, loopback, link-local,
/// multicast, or otherwise reserved range.
///
/// A bare domain name passes this check — it must still go through
/// [`resolve_and_check`] (DNS: every resolved address checked) before a
/// connection is made. `web_fetch` does exactly that, on every redirect
/// hop.
pub fn is_url_allowed(url: &Url) -> Result<(), String> {
    match url.scheme() {
        "http" | "https" => {}
        other => return Err(format!("refused: scheme '{other}' is not http/https")),
    }
    let Some(host) = url.host_str() else {
        return Err("refused: URL has no host".to_owned());
    };
    // IPv6 literals serialize with brackets; strip them for parsing.
    let host = host.trim_matches(|c| c == '[' || c == ']');
    if let Ok(ip) = host.parse::<IpAddr>() {
        return ip_is_allowed(ip);
    }
    let lower = host.to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") {
        return Err(format!("refused: '{host}' is a local hostname"));
    }
    Ok(())
}

/// The resolving half of the guard: [`is_url_allowed`] plus DNS. The
/// hostname is resolved and **every** address is checked — a name that
/// resolves to one public and one private address is refused, and so is
/// a name whose only "private-looking" form is a DNS rebinding away.
pub async fn resolve_and_check(url: &Url) -> Result<(), String> {
    is_url_allowed(url)?;
    let Some(host) = url.host_str() else {
        return Err("URL has no host".to_owned());
    };
    let host = host.trim_matches(|c| c == '[' || c == ']');
    if host.parse::<IpAddr>().is_ok() {
        // The literal itself was already checked by `is_url_allowed`.
        return Ok(());
    }
    let port = url.port_or_known_default().unwrap_or(0);
    let addrs = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| format!("DNS lookup for '{host}' failed: {e}"))?;
    let mut resolved = 0;
    for addr in addrs {
        resolved += 1;
        // EVERY resolved address, not just the first.
        ip_is_allowed(addr.ip())?;
    }
    if resolved == 0 {
        return Err(format!("DNS returned no addresses for '{host}'"));
    }
    Ok(())
}

/// Resolve one redirect hop (`Location` header value against the URL it
/// came from). Pure; the caller routes the result back through the guard.
fn next_hop(base: &Url, location: &str) -> Result<Url, String> {
    base.join(location).map_err(|e| format!("{e}"))
}

fn ip_is_allowed(ip: IpAddr) -> Result<(), String> {
    let refused = match ip {
        IpAddr::V4(v4) => ipv4_refused_range(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => ipv4_refused_range(v4),
            None => ipv6_refused_range(v6),
        },
    };
    match refused {
        Some(range) => Err(format!("refused: {ip} is in {range}")),
        None => Ok(()),
    }
}

/// The §17.2 list plus the other reserved/non-routable IPv4 blocks.
fn ipv4_refused_range(ip: Ipv4Addr) -> Option<&'static str> {
    let [a, b, c, _] = ip.octets();
    if a == 0 {
        return Some("0.0.0.0/8 (unspecified)");
    }
    if a == 10 {
        return Some("10.0.0.0/8 (private)");
    }
    if a == 127 {
        return Some("127.0.0.0/8 (loopback)");
    }
    if a == 169 && b == 254 {
        return Some("169.254.0.0/16 (link-local, incl. cloud metadata)");
    }
    if a == 172 && (16..=31).contains(&b) {
        return Some("172.16.0.0/12 (private)");
    }
    if a == 192 && b == 0 && c == 0 {
        return Some("192.0.0.0/24 (reserved)");
    }
    if a == 192 && b == 0 && c == 2 {
        return Some("192.0.2.0/24 (TEST-NET-1, documentation)");
    }
    if a == 192 && b == 168 {
        return Some("192.168.0.0/16 (private)");
    }
    if a == 198 && (b == 18 || b == 19) {
        return Some("198.18.0.0/15 (benchmarking)");
    }
    if a == 198 && b == 51 && c == 100 {
        return Some("198.51.100.0/24 (TEST-NET-2, documentation)");
    }
    if a == 203 && b == 0 && c == 113 {
        return Some("203.0.113.0/24 (TEST-NET-3, documentation)");
    }
    if a == 100 && (64..=127).contains(&b) {
        return Some("100.64.0.0/10 (CGNAT)");
    }
    if (224..240).contains(&a) {
        return Some("224.0.0.0/4 (multicast)");
    }
    if a >= 240 {
        return Some("240.0.0.0/4 (reserved/broadcast)");
    }
    None
}

/// The IPv6 counterparts of the refused ranges (an IPv4-mapped address
/// is checked as IPv4 by [`ip_is_allowed`]).
fn ipv6_refused_range(ip: Ipv6Addr) -> Option<&'static str> {
    let seg = ip.segments();
    if ip.is_unspecified() {
        return Some(":: (unspecified)");
    }
    if ip.is_loopback() {
        return Some("::1/128 (loopback)");
    }
    if (seg[0] & 0xffc0) == 0xfe80 {
        return Some("fe80::/10 (link-local)");
    }
    if (seg[0] & 0xfe00) == 0xfc00 {
        return Some("fc00::/7 (unique local)");
    }
    if (seg[0] & 0xff00) == 0xff00 {
        return Some("ff00::/8 (multicast)");
    }
    if seg[0] == 0x2001 && seg[1] == 0x0db8 {
        return Some("2001:db8::/32 (documentation)");
    }
    None
}

// ---------------------------------------------------------------------------
// web_search
// ---------------------------------------------------------------------------

/// `web_search(query)` — a numbered result list (title + URL) the chat
/// renders as a web card.
///
/// Provider: Brave Search API when `brave_api_key` is supplied (a
/// non-empty key selects it), else DuckDuckGo Lite HTML. The key is used
/// for exactly one request header and is never copied into a result,
/// error, or log line.
pub async fn web_search(query: &str, brave_api_key: Option<&str>) -> ToolResult {
    let Some(client) = make_client() else {
        return ToolResult::error("", "web_search: could not build the HTTP client");
    };
    let brave = brave_api_key.map(str::trim).filter(|k| !k.is_empty());
    match brave {
        Some(key) => brave_search(query, key, &client).await,
        None => duckduckgo_search(query, &client).await,
    }
}

async fn duckduckgo_search(query: &str, client: &Client) -> ToolResult {
    let url = match Url::parse_with_params(DDG_LITE_URL, &[("q", query)]) {
        Ok(u) => u,
        Err(e) => {
            return ToolResult::error(
                "",
                format!("web_search: could not build the query URL: {e}"),
            )
        }
    };
    // The endpoint is a constant, but the guard runs anyway — belt and braces.
    if let Err(reason) = is_url_allowed(&url) {
        return ToolResult::error("", format!("web_search: {reason}"));
    }
    let resp = match client.get(url).send().await {
        Ok(r) => r,
        Err(e) => return ToolResult::error("", format!("web_search: request failed: {e}")),
    };
    let status = resp.status();
    let body = match read_body_capped(resp).await {
        Ok(b) => b,
        Err(e) => return ToolResult::error("", format!("web_search: {e}")),
    };
    if !status.is_success() {
        return ToolResult::error("", format!("web_search: DuckDuckGo returned HTTP {status}"));
    }
    let hits = parse_ddg_lite(&body);
    ok_result(format_hits(query, &hits))
}

async fn brave_search(query: &str, api_key: &str, client: &Client) -> ToolResult {
    let url = match Url::parse_with_params(BRAVE_SEARCH_URL, &[("q", query), ("count", "10")]) {
        Ok(u) => u,
        Err(e) => {
            return ToolResult::error(
                "",
                format!("web_search: could not build the query URL: {e}"),
            )
        }
    };
    if let Err(reason) = is_url_allowed(&url) {
        return ToolResult::error("", format!("web_search: {reason}"));
    }
    let resp = match client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .header("X-Subscription-Token", api_key)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return ToolResult::error("", format!("web_search: request failed: {e}")),
    };
    let status = resp.status();
    let body = match read_body_capped(resp).await {
        Ok(b) => b,
        Err(e) => return ToolResult::error("", format!("web_search: {e}")),
    };
    if !status.is_success() {
        // Status only — never the key, never response headers.
        return ToolResult::error(
            "",
            format!("web_search: Brave Search API returned HTTP {status}"),
        );
    }
    match parse_brave(&body) {
        Ok(hits) => ok_result(format_hits(query, &hits)),
        Err(e) => ToolResult::error("", format!("web_search: {e}")),
    }
}

/// One search hit: the anchor text (title) and the unwrapped target URL.
#[derive(Debug)]
struct SearchHit {
    title: String,
    url: String,
}

/// Parse DuckDuckGo Lite result HTML: anchors with class `result-link`.
/// DDG's own navigation links are skipped; `uddg=`-wrapped redirect
/// links are unwrapped and percent-decoded.
fn parse_ddg_lite(html: &str) -> Vec<SearchHit> {
    let lower = html.to_ascii_lowercase();
    let mut hits: Vec<SearchHit> = Vec::new();
    let mut cursor = 0;
    while let Some(rel) = find_anchor(&lower[cursor..]) {
        let tag_start = cursor + rel;
        let Some(gt_rel) = lower[tag_start..].find('>') else {
            break;
        };
        let tag = &html[tag_start..tag_start + gt_rel + 1];
        let content_start = tag_start + gt_rel + 1;
        let Some(close_rel) = lower[content_start..].find("</a") else {
            break;
        };
        let content = &html[content_start..content_start + close_rel];
        let close_tag_start = content_start + close_rel;
        let Some(close_gt) = lower[close_tag_start..].find('>') else {
            break;
        };
        cursor = close_tag_start + close_gt + 1;
        if !anchor_is_result_link(tag) {
            continue;
        }
        let Some(href) = extract_attr(tag, "href") else {
            continue;
        };
        let url = normalize_ddg_url(&href);
        if url_is_ddg_internal(&url) || url.is_empty() {
            continue;
        }
        let title = strip_html_to_text(content);
        if title.is_empty() {
            continue;
        }
        if hits.iter().any(|h| h.url == url) {
            continue;
        }
        hits.push(SearchHit { title, url });
    }
    hits
}

/// Parse the Brave Search API JSON body (`web.results[].{title,url}`).
fn parse_brave(body: &str) -> Result<Vec<SearchHit>, String> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| format!("Brave response was not valid JSON: {e}"))?;
    let Some(results) = v.pointer("/web/results").and_then(|r| r.as_array()) else {
        return Ok(Vec::new());
    };
    Ok(results
        .iter()
        .filter_map(|r| {
            let title = r.get("title").and_then(|t| t.as_str()).unwrap_or("").trim();
            let url = r.get("url").and_then(|u| u.as_str()).unwrap_or("").trim();
            (!title.is_empty() && !url.is_empty()).then(|| SearchHit {
                title: title.to_owned(),
                url: url.to_owned(),
            })
        })
        .collect())
}

/// Find `<a` as a real anchor (followed by whitespace, `/`, or `>`) —
/// not `<abbr` and friends.
fn find_anchor(hay: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(pos) = hay[from..].find("<a") {
        let next = hay[from + pos + 2..].chars().next();
        match next {
            Some(c) if c.is_whitespace() || c == '/' || c == '>' => return Some(from + pos),
            _ => from += pos + 2,
        }
    }
    None
}

fn anchor_is_result_link(tag: &str) -> bool {
    extract_attr(tag, "class").is_some_and(|classes| {
        classes
            .split_whitespace()
            .any(|c| c.eq_ignore_ascii_case("result-link"))
    })
}

/// Extract an attribute value from a tag string, honoring single or
/// double quotes. `name` must be a whole attribute name (the match must
/// be preceded by whitespace), so `data-href` never satisfies `href`.
fn extract_attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let needle = format!("{name}=");
    let mut from = 0;
    while let Some(pos) = lower[from..].find(&needle) {
        let abs = from + pos;
        if !lower[..abs].chars().last().is_some_and(char::is_whitespace) {
            from = abs + needle.len();
            continue;
        }
        let rest = &tag[abs + needle.len()..];
        let first = rest.chars().next()?;
        if first == '"' || first == '\'' {
            let end = rest[1..].find(first)? + 1;
            return Some(rest[1..end].to_owned());
        }
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '>')
            .unwrap_or(rest.len());
        return Some(rest[..end].to_owned());
    }
    None
}

/// Unwrap DDG's `uddg=` redirect links, normalize protocol-relative and
/// root-relative hrefs to absolute URLs.
fn normalize_ddg_url(href: &str) -> String {
    let href = href.trim();
    if let Some(idx) = href.find("uddg=") {
        let val = &href[idx + 5..];
        let end = val.find('&').unwrap_or(val.len());
        return percent_decode(&val[..end]);
    }
    if let Some(rest) = href.strip_prefix("//") {
        return format!("https://{rest}");
    }
    if href.starts_with('/') {
        return format!("https://lite.duckduckgo.com{href}");
    }
    href.to_owned()
}

fn url_is_ddg_internal(url: &str) -> bool {
    Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .is_some_and(|h| h.eq_ignore_ascii_case("duckduckgo.com") || h.ends_with(".duckduckgo.com"))
}

fn format_hits(query: &str, hits: &[SearchHit]) -> String {
    if hits.is_empty() {
        return format!("No results found for \"{query}\".");
    }
    let mut out = format!("Web search results for \"{query}\":\n");
    for (i, hit) in hits.iter().enumerate().take(MAX_RESULTS) {
        out.push_str(&format!("{}. {}\n   {}\n", i + 1, hit.title, hit.url));
    }
    out
}

// ---------------------------------------------------------------------------
// web_fetch
// ---------------------------------------------------------------------------

/// `web_fetch(url)` — GET the URL (SSRF-guarded, redirects re-checked
/// per hop), strip HTML to readable text, cap at 256 KiB (truncation
/// notice appended), and record the source URL in the result.
pub async fn web_fetch(url: &str) -> ToolResult {
    let Some(client) = make_client() else {
        return ToolResult::error("", "web_fetch: could not build the HTTP client");
    };
    let trimmed = url.trim();
    let mut current = match Url::parse(trimmed) {
        Ok(u) => u,
        Err(e) => {
            return ToolResult::error("", format!("web_fetch: not a valid URL ({e}): {trimmed}"))
        }
    };
    // Redirects are followed manually (policy: none) so that EVERY hop
    // goes back through resolve_and_check — a public page that bounces
    // to 169.254.169.254 is refused at the hop, never fetched.
    for _ in 0..=MAX_REDIRECTS {
        if let Err(reason) = resolve_and_check(&current).await {
            return ToolResult::error("", format!("web_fetch: {reason}"));
        }
        let resp = match client.get(current.clone()).send().await {
            Ok(r) => r,
            Err(e) => return ToolResult::error("", format!("web_fetch: request failed: {e}")),
        };
        let status = resp.status();
        if status.is_redirection() {
            let location = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let Some(location) = location else {
                return ToolResult::error(
                    "",
                    format!("web_fetch: HTTP {status} redirect without a Location header"),
                );
            };
            match next_hop(&current, &location) {
                Ok(next) => {
                    current = next;
                    continue;
                }
                Err(e) => {
                    return ToolResult::error("", format!("web_fetch: bad redirect target: {e}"))
                }
            }
        }
        if !status.is_success() {
            return ToolResult::error("", format!("web_fetch: HTTP {status} fetching {current}"));
        }
        let is_html = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|ct| ct.to_ascii_lowercase().contains("html"));
        let body = match read_body_capped(resp).await {
            Ok(b) => b,
            Err(e) => return ToolResult::error("", format!("web_fetch: {e}")),
        };
        let text = if is_html {
            strip_html_to_text(&body)
        } else {
            body
        };
        let content = truncate_with_notice(&text, MAX_TEXT_BYTES);
        return ok_result(format!("Source: {current}\n\n{content}"));
    }
    ToolResult::error(
        "",
        format!("web_fetch: too many redirects (more than {MAX_REDIRECTS})"),
    )
}

/// The shared HTTP client: no auto-redirects (guard re-checks each hop),
/// short timeouts. No API key is ever attached here.
fn make_client() -> Option<Client> {
    Client::builder()
        .redirect(redirect::Policy::none())
        .timeout(FETCH_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .user_agent(concat!("SeloraIDE/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()
}

/// Read a response body as lossy UTF-8, buffered no further than
/// [`MAX_RAW_BYTES`].
async fn read_body_capped(mut resp: reqwest::Response) -> Result<String, String> {
    let mut raw: Vec<u8> = Vec::new();
    loop {
        let chunk = match resp.chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => return Err(format!("reading the response body failed: {e}")),
        };
        if raw.len() >= MAX_RAW_BYTES {
            break;
        }
        raw.extend_from_slice(&chunk);
    }
    raw.truncate(MAX_RAW_BYTES);
    Ok(String::from_utf8_lossy(&raw).into_owned())
}

// ---------------------------------------------------------------------------
// HTML → text (hand-rolled; no heavyweight dependency)
// ---------------------------------------------------------------------------

/// Strip tags, drop `<script>`/`<style>`/comment bodies, decode the
/// common entities, and collapse the whitespace runs the markup leaves
/// behind. Block-level tags become line breaks.
fn strip_html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut rest = html;
    while let Some(lt) = rest.find('<') {
        if lt > 0 {
            out.push_str(&decode_entities(&rest[..lt]));
        }
        let after_lt = &rest[lt + 1..];
        let Some(gt) = after_lt.find('>') else {
            // A stray '<' that opens no tag: keep it as text.
            out.push('<');
            out.push_str(&decode_entities(after_lt));
            rest = "";
            break;
        };
        let tag_body = &after_lt[..gt];
        rest = &after_lt[gt + 1..];
        if tag_body.starts_with("!--") {
            rest = skip_past(rest, "-->");
            continue;
        }
        let name = tag_name(tag_body);
        let is_closing = tag_body.starts_with('/');
        if !is_closing && matches!(name.as_str(), "script" | "style") {
            rest = skip_until_closing(rest, &name);
            continue;
        }
        if is_block_tag(&name) {
            out.push('\n');
        }
    }
    out.push_str(&decode_entities(rest));
    collapse_whitespace(&out)
}

fn tag_name(tag_body: &str) -> String {
    let end = tag_body
        .find(|c: char| c.is_whitespace() || c == '/')
        .unwrap_or(tag_body.len());
    tag_body[..end].to_ascii_lowercase()
}

fn is_block_tag(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "br"
            | "hr"
            | "li"
            | "ul"
            | "ol"
            | "dl"
            | "dt"
            | "dd"
            | "tr"
            | "table"
            | "td"
            | "th"
            | "thead"
            | "tbody"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "blockquote"
            | "pre"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "nav"
            | "aside"
            | "title"
            | "form"
            | "option"
    )
}

/// Skip to just past `</{tag}>` (case-insensitive); unterminated → empty.
fn skip_until_closing<'a>(rest: &'a str, tag: &str) -> &'a str {
    let close = format!("</{tag}");
    let hay = rest.to_ascii_lowercase();
    match hay.find(&close) {
        Some(pos) => match rest[pos..].find('>') {
            Some(gt) => &rest[pos + gt + 1..],
            None => "",
        },
        None => "",
    }
}

fn skip_past<'a>(rest: &'a str, marker: &str) -> &'a str {
    match rest.to_ascii_lowercase().find(marker) {
        Some(pos) => &rest[pos + marker.len()..],
        None => "",
    }
}

/// Decode the common named entities plus decimal/hex numeric references.
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp..];
        // Entity names/references are at most 10 chars to the ';'.
        let semi = after
            .char_indices()
            .take(10)
            .find(|(_, c)| *c == ';')
            .map(|(i, _)| i);
        match semi {
            Some(i) if (2..=9).contains(&i) => {
                let ent = &after[1..i];
                if let Some(ch) = named_entity(ent) {
                    out.push(ch);
                } else if let Some(num) = ent.strip_prefix('#') {
                    let code = num
                        .strip_prefix('x')
                        .or_else(|| num.strip_prefix('X'))
                        .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                        .or_else(|| num.parse::<u32>().ok());
                    match code.and_then(char::from_u32) {
                        Some(ch) => out.push(ch),
                        None => out.push_str(&after[..=i]),
                    }
                } else {
                    // Unknown entity: keep it verbatim.
                    out.push_str(&after[..=i]);
                }
                rest = &after[i + 1..];
            }
            _ => {
                out.push('&');
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn named_entity(ent: &str) -> Option<char> {
    match ent {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        _ => None,
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3])
                    .ok()
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(b) => {
                        out.push(b);
                        i += 3;
                    }
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Cap `text` at `cap` bytes (never splitting a character) and append a
/// notice when truncation happened.
fn truncate_with_notice(text: &str, cap: usize) -> String {
    if text.len() <= cap {
        return text.to_owned();
    }
    let mut cut = cap;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut out = String::from(&text[..cut]);
    out.push_str(&format!(
        "\n\n[truncated at the {} KiB cap — the original page was {} bytes]",
        cap / 1024,
        text.len()
    ));
    out
}

fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    let mut pending_newlines = 0usize;
    for ch in s.chars() {
        match ch {
            '\n' => {
                pending_space = false;
                pending_newlines += 1;
            }
            ' ' | '\t' | '\r' => pending_space = true,
            other => {
                if pending_newlines > 0 {
                    out.push('\n');
                    if pending_newlines > 1 {
                        out.push('\n');
                    }
                    pending_newlines = 0;
                    pending_space = false;
                } else if pending_space && !out.is_empty() {
                    out.push(' ');
                    pending_space = false;
                }
                out.push(other);
            }
        }
    }
    out.trim().to_owned()
}

fn ok_result(content: String) -> ToolResult {
    ToolResult {
        id: String::new(),
        ok: true,
        content,
    }
}

// ---------------------------------------------------------------------------
// Tests — pure functions and fixture HTML only; never a network call.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).expect("valid test URL")
    }

    // ---- guard: schemes and hosts ----

    #[test]
    fn guard_allows_public_http_and_https() {
        assert!(is_url_allowed(&u("http://example.com/page")).is_ok());
        assert!(is_url_allowed(&u("https://example.com:8443/path?q=1")).is_ok());
        assert!(is_url_allowed(&u("http://8.8.8.8/")).is_ok());
        assert!(is_url_allowed(&u("https://[2606:4700::1111]/")).is_ok());
        // an IPv4-mapped IPv6 that maps to a *public* v4 is fine
        assert!(is_url_allowed(&u("http://[::ffff:93.184.216.34]/")).is_ok());
    }

    #[test]
    fn guard_refuses_non_http_schemes() {
        assert!(is_url_allowed(&u("ftp://example.com/file")).is_err());
        assert!(is_url_allowed(&u("file:///etc/passwd")).is_err());
        assert!(is_url_allowed(&u("javascript:alert(1)")).is_err());
        assert!(is_url_allowed(&u("gopher://localhost")).is_err());
    }

    #[test]
    fn guard_refuses_localhost_names() {
        assert!(is_url_allowed(&u("http://localhost/")).is_err());
        assert!(is_url_allowed(&u("http://LOCALHOST:8080/")).is_err());
        assert!(is_url_allowed(&u("https://app.localhost/")).is_err());
    }

    // ---- guard: every §17.2 range, each one refused ----

    #[test]
    fn guard_refuses_every_specified_private_range() {
        let refused = [
            "http://127.0.0.1/",
            "http://127.255.255.254/",
            "http://10.0.0.1/",
            "http://10.255.255.255/",
            "http://172.16.0.1/",
            "http://172.31.255.254/",
            "http://192.168.0.1/",
            "http://192.168.255.255/",
            "http://169.254.0.1/",
            "http://169.254.169.254/", // cloud metadata
            "http://0.0.0.0/",
            "http://100.64.0.1/",      // CGNAT
            "http://192.0.0.9/",       // reserved
            "http://192.0.2.1/",       // TEST-NET-1
            "http://198.18.0.1/",      // benchmarking
            "http://198.51.100.7/",    // TEST-NET-2
            "http://203.0.113.7/",     // TEST-NET-3
            "http://224.0.0.1/",       // multicast
            "http://240.0.0.1/",       // reserved
            "http://255.255.255.255/", // broadcast
        ];
        for url in refused {
            let err = is_url_allowed(&u(url)).unwrap_err();
            assert!(
                err.contains("refused"),
                "wrong refusal message for {url}: {err}"
            );
        }
    }

    #[test]
    fn guard_allows_just_outside_the_private_ranges() {
        for url in [
            "http://9.255.255.255/",
            "http://11.0.0.1/",
            "http://172.15.255.255/",
            "http://172.32.0.1/",
            "http://192.169.0.1/",
            "http://193.0.113.7/",
        ] {
            assert!(is_url_allowed(&u(url)).is_ok(), "should allow {url}");
        }
    }

    #[test]
    fn guard_refuses_v6_special_ranges() {
        for url in [
            "http://[::1]/",
            "http://[::]/",
            "http://[fe80::1]/",
            "http://[fc00::1]/",
            "http://[fd12:3456::1]/",
            "http://[ff02::1]/",
            "http://[2001:db8::1]/",
            // IPv4-mapped IPv6 wrapping a private v4 (the url crate may
            // already fold these to plain IPv4 — both paths refuse).
            "http://[::ffff:127.0.0.1]/",
            "http://[::ffff:169.254.169.254]/",
            "http://[::ffff:10.0.0.1]/",
        ] {
            assert!(is_url_allowed(&u(url)).is_err(), "should refuse {url}");
        }
    }

    #[test]
    fn guard_ip_literals_mapped_v6() {
        assert!(ip_is_allowed(IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1))).is_err());
        let mapped_private: Ipv6Addr = "::ffff:127.0.0.1".parse().unwrap();
        assert!(ip_is_allowed(IpAddr::V6(mapped_private)).is_err());
        let mapped_public: Ipv6Addr = "::ffff:8.8.8.8".parse().unwrap();
        assert!(ip_is_allowed(IpAddr::V6(mapped_public)).is_ok());
    }

    // ---- guard: redirects re-checked per hop ----

    #[test]
    fn redirect_hop_to_private_target_is_refused() {
        let base = u("https://example.com/article");
        // An attacker-controlled page bounces to internal addresses.
        for location in [
            "http://127.0.0.1:8080/admin",
            "http://169.254.169.254/latest/meta-data/",
            "//10.0.0.5/x",
        ] {
            let hop = next_hop(&base, location).expect("valid hop URL");
            assert!(
                is_url_allowed(&hop).is_err(),
                "should refuse hop {location} -> {hop}"
            );
        }
        // A relative location stays on the (public) origin and is fine.
        let hop = next_hop(&base, "/next/page").expect("valid hop URL");
        assert_eq!(hop.as_str(), "https://example.com/next/page");
        assert!(is_url_allowed(&hop).is_ok());
    }

    // ---- DuckDuckGo Lite parser (fixture HTML, no network) ----

    const DDG_FIXTURE: &str = r#"
<html><head><title>ropey at DuckDuckGo</title></head>
<body>
<table>
<tr>
<td>1.&nbsp;<a rel="nofollow" href="https://docs.rs/ropey/latest/ropey/" class='result-link'>ropey - Rust</a></td>
</tr>
<tr><td colspan="3"><span class='result-snippet'>A fast and robust rope for Rust &amp; other text editing</span></td></tr>
<tr>
<td>2.&nbsp;<a rel="nofollow" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fgithub.com%2Fcessen%2Fropey&amp;rut=deadbeef" class='result-link'>GitHub - cessen/ropey: Rope &lt;data structure&gt;</a></td>
</tr>
<tr>
<td>3.&nbsp;<a rel="nofollow" href="https://example.com/ropey-guide" class='result-link'>The Ropey Guide &amp; Reference</a></td>
</tr>
<tr><td><a href="/lite/?q=ropey&amp;s=20" class='result-link'>More results</a></td></tr>
<tr><td><a href="/about" class="navlink">about DuckDuckGo</a></td></tr>
</table>
</body></html>
"#;

    #[test]
    fn parses_ddg_lite_fixture() {
        let hits = parse_ddg_lite(DDG_FIXTURE);
        assert_eq!(hits.len(), 3, "hits: {hits:?}");
        assert_eq!(hits[0].title, "ropey - Rust");
        assert_eq!(hits[0].url, "https://docs.rs/ropey/latest/ropey/");
        // uddg-wrapped redirect links are unwrapped and percent-decoded.
        assert_eq!(hits[1].url, "https://github.com/cessen/ropey");
        assert_eq!(
            hits[1].title,
            "GitHub - cessen/ropey: Rope <data structure>"
        );
        // Entities in titles are decoded.
        assert_eq!(hits[2].title, "The Ropey Guide & Reference");
        assert_eq!(hits[2].url, "https://example.com/ropey-guide");
    }

    #[test]
    fn parses_ddg_lite_when_there_are_no_results() {
        assert!(parse_ddg_lite("<html><body>No results.</body></html>").is_empty());
        assert!(parse_ddg_lite("").is_empty());
    }

    #[test]
    fn formats_a_numbered_result_list() {
        let hits = vec![
            SearchHit {
                title: "T1".into(),
                url: "https://a.example/1".into(),
            },
            SearchHit {
                title: "T2".into(),
                url: "https://b.example/2".into(),
            },
        ];
        let out = format_hits("ropey", &hits);
        assert!(out.contains("Web search results for \"ropey\""));
        assert!(out.contains("1. T1"));
        assert!(out.contains("https://a.example/1"));
        assert!(out.contains("2. T2"));
        assert!(out.contains("https://b.example/2"));
        assert!(format_hits("nothing", &[])
            .to_lowercase()
            .contains("no results"));
    }

    // ---- Brave parser (fixture JSON, no network) ----

    const BRAVE_FIXTURE: &str = r#"{"web":{"results":[
        {"title":"ropey - crates.io","url":"https://crates.io/crates/ropey","description":"ropey"},
        {"title":"no url","url":"","description":"skipped"},
        {"title":"Docs","url":"https://docs.rs/ropey"}
    ]}}"#;

    #[test]
    fn parses_brave_json_fixture() {
        let hits = parse_brave(BRAVE_FIXTURE).expect("parse ok");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "ropey - crates.io");
        assert_eq!(hits[0].url, "https://crates.io/crates/ropey");
        assert_eq!(hits[1].url, "https://docs.rs/ropey");
        assert!(parse_brave("{}").expect("parse ok").is_empty());
        assert!(parse_brave("not json").is_err());
    }

    // ---- HTML → text ----

    #[test]
    fn strips_html_to_readable_text() {
        let html = r#"<html><head><title>T</title><style>body { color: red }</style></head>
<body><script>var secret = "nope";</script>
<h1>Hello &amp; welcome</h1><p>Some <b>bold</b> text with an <a href="/x">anchor</a>.</p>
<!-- a comment that must vanish -->
</body></html>"#;
        let text = strip_html_to_text(html);
        assert!(
            !text.to_lowercase().contains("secret"),
            "script body dropped: {text}"
        );
        assert!(!text.contains("color"), "style body dropped: {text}");
        assert!(!text.contains("comment"), "comment dropped: {text}");
        assert!(text.contains("Hello & welcome"));
        assert!(text.contains("Some bold text with an anchor."));
    }

    #[test]
    fn keeps_stray_angle_bracket_as_text() {
        assert_eq!(strip_html_to_text("5 < 6 and done"), "5 < 6 and done");
    }

    #[test]
    fn decodes_common_entities() {
        assert_eq!(decode_entities("a &amp; b"), "a & b");
        assert_eq!(decode_entities("&lt;&gt;&quot;&#39;"), "<>\"'");
        assert_eq!(decode_entities("&#65;&#x42;"), "AB");
        assert_eq!(decode_entities("no entities"), "no entities");
        // Malformed (no semicolon) and unknown entities stay verbatim.
        assert_eq!(decode_entities("5 &amp 6"), "5 &amp 6");
        assert_eq!(decode_entities("a & b"), "a & b");
        assert_eq!(decode_entities("&unknown;"), "&unknown;");
    }

    #[test]
    fn percent_decodes_uddg_targets() {
        assert_eq!(
            percent_decode("https%3A%2F%2Fexample.com%2Fa%3Fb%3D1"),
            "https://example.com/a?b=1"
        );
        assert_eq!(percent_decode("a+b"), "a b");
        assert_eq!(percent_decode("plain"), "plain");
        assert_eq!(percent_decode("bad%2"), "bad%2");
    }

    // ---- 256 KiB truncation ----

    #[test]
    fn truncates_at_the_256_kib_cap_with_a_notice() {
        // At or under the cap: untouched, no notice.
        let exact = "y".repeat(MAX_TEXT_BYTES);
        assert_eq!(truncate_with_notice(&exact, MAX_TEXT_BYTES), exact);
        let small = "x".repeat(100);
        assert_eq!(truncate_with_notice(&small, MAX_TEXT_BYTES), small);

        // Over the cap: cut at the cap, notice appended.
        let big = "x".repeat(MAX_TEXT_BYTES + 50_000);
        let out = truncate_with_notice(&big, MAX_TEXT_BYTES);
        assert!(out.len() > MAX_TEXT_BYTES);
        assert!(out.len() < big.len());
        assert!(out.contains("truncated at the 256 KiB cap"));
        assert!(out.contains(&"x".repeat(1024)));

        // The cut never splits a multi-byte character.
        let multi = "é".repeat(MAX_TEXT_BYTES / 2 + 100);
        let out = truncate_with_notice(&multi, MAX_TEXT_BYTES);
        assert!(out.len() >= MAX_TEXT_BYTES - 1);
        assert!(out.len() < multi.len() + 200);
        assert!(out.contains("truncated at the 256 KiB cap"));
    }
}
