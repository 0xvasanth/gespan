//! A web link's title and favicon for the chip that replaces it in a prompt,
//! ported from src-tauri/src/link_preview.rs and the metadata cache in
//! src/features/sessions/model/linkPreview.ts.
//!
//! Every hop is checked before a request is made: only plain http(s) URLs
//! without credentials, never a local name, and never a private, loopback,
//! link-local or documentation address. The address check also runs inside
//! the resolver that opens each connection, so a name cannot pass the check
//! and then resolve somewhere private.
use std::{
    collections::{HashMap, VecDeque},
    io::{self, Cursor, Read},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs},
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};
use url::{Host, Url};

const HTTP_TIMEOUT: Duration = Duration::from_secs(7);
const MAX_REDIRECTS: usize = 5;
const MAX_PAGE_BYTES: usize = 512 * 1024;
const MAX_ICON_BYTES: usize = 256 * 1024;
const MAX_URL_LEN: usize = 8192;
const MAX_TITLE_CHARS: usize = 240;
/// The chip draws its favicon at 14px; 32px keeps it sharp at 2x.
const ICON_SIDE: u32 = 32;
const USER_AGENT: &str = "Mozilla/5.0 (compatible; Gespan-LinkPreview/1.0)";
const CACHE_CAPACITY: usize = 256;

/// What a link's chip shows. Both are optional: the chip falls back to the
/// host and its first letter, as when the fetch fails.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkPreview {
    pub title: Option<String>,
    /// The site's icon, decoded and re-encoded as a small PNG.
    pub favicon_png: Option<Vec<u8>>,
}

/// Which addresses a fetch may reach. Tests serve pages from loopback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Network {
    Public,
    #[cfg(test)]
    Loopback,
}

/// The app's preview cache: successes are kept, failures are not, so a link
/// that failed is tried again the next time it is shown (linkPreview.ts).
pub fn cache() -> &'static Cache {
    static CACHE: LazyLock<Cache> = LazyLock::new(|| Cache::new(CACHE_CAPACITY));
    &CACHE
}

/// A cached preview, or a blocking fetch that fills the cache. Call it off
/// the UI thread.
pub fn load(url: &str) -> Result<Arc<LinkPreview>, String> {
    cache().load(url, fetch)
}

/// Fetch `url`'s title and favicon. Blocking; at most a few seconds per hop.
pub fn fetch(url: &str) -> Result<LinkPreview, String> {
    fetch_with(url, Network::Public)
}

/// A bounded map from URL to its preview; the oldest entry goes first.
pub struct Cache {
    inner: Mutex<CacheInner>,
}
struct CacheInner {
    capacity: usize,
    entries: HashMap<String, Arc<LinkPreview>>,
    order: VecDeque<String>,
}

impl Cache {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(CacheInner {
                capacity: capacity.max(1),
                entries: HashMap::new(),
                order: VecDeque::new(),
            }),
        }
    }
    pub fn get(&self, url: &str) -> Option<Arc<LinkPreview>> {
        self.inner.lock().ok()?.entries.get(url).cloned()
    }
    pub fn insert(&self, url: &str, preview: LinkPreview) -> Arc<LinkPreview> {
        let preview = Arc::new(preview);
        let Ok(mut inner) = self.inner.lock() else {
            return preview;
        };
        if inner
            .entries
            .insert(url.to_owned(), preview.clone())
            .is_none()
        {
            inner.order.push_back(url.to_owned());
        }
        while inner.order.len() > inner.capacity {
            if let Some(oldest) = inner.order.pop_front() {
                inner.entries.remove(&oldest);
            }
        }
        preview
    }
    /// The cached preview, or `fetch`'s result, kept only when it succeeds.
    pub fn load(
        &self,
        url: &str,
        fetch: impl FnOnce(&str) -> Result<LinkPreview, String>,
    ) -> Result<Arc<LinkPreview>, String> {
        if let Some(preview) = self.get(url) {
            return Ok(preview);
        }
        fetch(url).map(|preview| self.insert(url, preview))
    }
}

struct FetchedResource {
    final_url: Url,
    content_type: Option<String>,
    bytes: Vec<u8>,
}

fn fetch_with(raw: &str, network: Network) -> Result<LinkPreview, String> {
    let url = parse_public_url(raw, network)?;
    let page = fetch_resource(
        url,
        "text/html,application/xhtml+xml;q=0.9",
        MAX_PAGE_BYTES,
        network,
    )?;
    if !is_html(page.content_type.as_deref()) {
        return Ok(LinkPreview::default());
    }

    let html = String::from_utf8_lossy(&page.bytes);
    let title = document_title(&html);
    let default_icon = page
        .final_url
        .join("/favicon.ico")
        .map_err(|_| "Could not resolve this site's icon".to_string())?;
    let linked_icon = document_icon_href(&html)
        .and_then(|href| page.final_url.join(&href).ok())
        .filter(is_http_url);

    let favicon_png = linked_icon
        .and_then(|url| fetch_icon_png(url, network))
        .or_else(|| fetch_icon_png(default_icon, network));

    Ok(LinkPreview { title, favicon_png })
}

fn fetch_icon_png(url: Url, network: Network) -> Option<Vec<u8>> {
    let icon = fetch_resource(url, "image/*,*/*;q=0.1", MAX_ICON_BYTES, network).ok()?;
    let mime = image_mime(icon.content_type.as_deref(), &icon.bytes)?;
    icon_png(mime, icon.bytes)
}

/// Decode a sniffed icon and shrink it to a small PNG for the chip.
fn icon_png(mime: &str, bytes: Vec<u8>) -> Option<Vec<u8>> {
    let format = match mime {
        "image/png" => image::ImageFormat::Png,
        "image/jpeg" => image::ImageFormat::Jpeg,
        "image/gif" => image::ImageFormat::Gif,
        "image/webp" => image::ImageFormat::WebP,
        "image/x-icon" => image::ImageFormat::Ico,
        _ => return None,
    };
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(1024);
    limits.max_image_height = Some(1024);
    limits.max_alloc = Some(16 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().ok()?;
    let image = if image.width() > ICON_SIDE || image.height() > ICON_SIDE {
        image.thumbnail(ICON_SIDE, ICON_SIDE)
    } else {
        image
    };
    crate::project_appearance::png_bytes(image.into_rgba8()).ok()
}

fn fetch_resource(
    mut url: Url,
    accept: &str,
    max_bytes: usize,
    network: Network,
) -> Result<FetchedResource, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(HTTP_TIMEOUT)
        .redirects(0)
        .try_proxy_from_env(false)
        .resolver(move |netloc: &str| checked_addresses(netloc, network))
        .build();

    for _ in 0..=MAX_REDIRECTS {
        validate_target(&url, network)?;
        let response = match agent
            .get(url.as_str())
            .set("Accept", accept)
            .set("User-Agent", USER_AGENT)
            .call()
        {
            Ok(response) => response,
            Err(ureq::Error::Status(status, response)) if is_redirect(status) => response,
            Err(ureq::Error::Status(status, _)) => {
                return Err(format!("Link preview request failed ({status})"));
            }
            Err(_) => return Err("Could not load link preview".into()),
        };

        if is_redirect(response.status()) {
            let location = response
                .header("Location")
                .ok_or_else(|| "Link preview redirect has no destination".to_string())?;
            url = url
                .join(location)
                .map_err(|_| "Link preview redirect is invalid".to_string())?;
            if !is_http_url(&url) {
                return Err("Link preview redirect is not HTTP".into());
            }
            continue;
        }

        if !(200..300).contains(&response.status()) {
            return Err(format!(
                "Link preview request failed ({})",
                response.status()
            ));
        }
        if response
            .header("Content-Length")
            .and_then(|value| value.trim().parse::<usize>().ok())
            .is_some_and(|length| length > max_bytes)
        {
            return Err("Link preview response is too large".into());
        }
        let content_type = response.header("Content-Type").map(str::to_string);
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(max_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Could not read link preview".to_string())?;
        if bytes.len() > max_bytes {
            return Err("Link preview response is too large".into());
        }
        return Ok(FetchedResource {
            final_url: url,
            content_type,
            bytes,
        });
    }

    Err("Too many link preview redirects".into())
}

fn parse_public_url(raw: &str, network: Network) -> Result<Url, String> {
    let value = raw.trim();
    if value.is_empty() || value.len() > MAX_URL_LEN {
        return Err("Link preview URL is invalid".into());
    }
    let url = Url::parse(value).map_err(|_| "Link preview URL is invalid".to_string())?;
    if !is_http_url(&url)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none()
    {
        return Err("Link preview URL is invalid".into());
    }
    validate_target(&url, network)?;
    Ok(url)
}

fn is_http_url(url: &Url) -> bool {
    url.scheme() == "https" || url.scheme() == "http"
}

/// The checks that need no lookup; the resolver checks the addresses.
fn validate_target(url: &Url, network: Network) -> Result<(), String> {
    if !is_http_url(url) || !url.username().is_empty() || url.password().is_some() {
        return Err("Link preview target is invalid".into());
    }
    let host = match url
        .host()
        .ok_or_else(|| "Link preview target is invalid".to_string())?
    {
        Host::Ipv4(ip) => return allowed_ip_result(IpAddr::V4(ip), network),
        Host::Ipv6(ip) => return allowed_ip_result(IpAddr::V6(ip), network),
        Host::Domain(host) => host.trim_end_matches('.').to_ascii_lowercase(),
    };
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
        return Err("Local addresses cannot be previewed".into());
    }
    url.port_or_known_default()
        .ok_or_else(|| "Link preview target has no port".to_string())?;
    Ok(())
}

/// The resolver every connection goes through: all of a name's addresses
/// must be public, or the request is refused before it connects.
fn checked_addresses(netloc: &str, network: Network) -> io::Result<Vec<SocketAddr>> {
    let addresses: Vec<SocketAddr> = netloc.to_socket_addrs()?.collect();
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| !is_allowed_ip(address.ip(), network))
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Private addresses cannot be previewed",
        ));
    }
    Ok(addresses)
}

fn allowed_ip_result(ip: IpAddr, network: Network) -> Result<(), String> {
    if is_allowed_ip(ip, network) {
        Ok(())
    } else {
        Err("Private addresses cannot be previewed".into())
    }
}

fn is_allowed_ip(ip: IpAddr, network: Network) -> bool {
    match network {
        Network::Public => is_public_ip(ip),
        #[cfg(test)]
        Network::Loopback => is_public_ip(ip) || ip.is_loopback(),
    }
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_ipv4(ip),
        IpAddr::V6(ip) => is_public_ipv6(ip),
    }
}

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0
        || a == 10
        || a == 127
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 192 && b == 168)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224)
}

fn is_public_ipv6(ip: Ipv6Addr) -> bool {
    if let Some(ipv4) = ip.to_ipv4() {
        return is_public_ipv4(ipv4);
    }
    let segments = ip.segments();
    // Only global unicast, 2000::/3. That leaves out loopback, unique-local,
    // link-local, multicast, discard (100::/64) and NAT64 (64:ff9b::/96 and
    // 64:ff9b:1::/48), whose embedded IPv4 could be private.
    if segments[0] & 0xe000 != 0x2000 {
        return false;
    }
    // Inside it, 6to4 (2002::/16) and Teredo (2001::/32) also carry an IPv4
    // address; benchmarking (2001:2::/48) and documentation (2001:db8::/32)
    // are never real hosts.
    !(segments[0] == 0x2002
        || (segments[0] == 0x2001 && segments[1] == 0)
        || (segments[0] == 0x2001 && segments[1] == 2 && segments[2] == 0)
        || (segments[0] == 0x2001 && segments[1] == 0x0db8))
}

fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

/// A page without a content type is read as HTML, as browsers do.
fn is_html(content_type: Option<&str>) -> bool {
    content_type.is_none_or(|value| {
        let mime = value.split(';').next().unwrap_or("").trim();
        mime.eq_ignore_ascii_case("text/html") || mime.eq_ignore_ascii_case("application/xhtml+xml")
    })
}

/// The icon's type from its leading bytes; a bare `.ico` may only say so in
/// its header. SVG and anything else is refused.
fn image_mime(content_type: Option<&str>, bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if bytes.starts_with(&[0, 0, 1, 0]) {
        return Some("image/x-icon");
    }
    match content_type
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("image/vnd.microsoft.icon") | Some("image/x-icon") => Some("image/x-icon"),
        _ => None,
    }
}

/// `og:title`, then `twitter:title`, then `<title>`.
fn document_title(html: &str) -> Option<String> {
    let metas = opening_tags(html, "meta");
    for key in ["og:title", "twitter:title"] {
        for &tag in &metas {
            let marker = attribute(tag, "property").or_else(|| attribute(tag, "name"));
            if marker
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case(key))
            {
                if let Some(value) = attribute(tag, "content").and_then(clean_text) {
                    return Some(value);
                }
            }
        }
    }

    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let opening_end = tag_end(html, start)?;
    let close = lower[opening_end + 1..].find("</title>")? + opening_end + 1;
    clean_text(html[opening_end + 1..close].to_string())
}

/// The first `<link rel="… icon …" href>`, as written in the page.
fn document_icon_href(html: &str) -> Option<String> {
    opening_tags(html, "link").into_iter().find_map(|tag| {
        let rel = attribute(tag, "rel")?;
        if !rel
            .split_ascii_whitespace()
            .any(|part| part.eq_ignore_ascii_case("icon"))
        {
            return None;
        }
        attribute(tag, "href").filter(|href| !href.trim().is_empty())
    })
}

/// More tags than any real page head has; the rest of the page is ignored.
const MAX_TAGS: usize = 4096;

fn opening_tags<'a>(html: &'a str, name: &str) -> Vec<&'a str> {
    let lower = html.to_ascii_lowercase();
    let needle = format!("<{name}");
    let mut tags = Vec::new();
    let mut offset = 0;
    while let Some(relative) = lower[offset..].find(&needle) {
        let start = offset + relative;
        let after = start + needle.len();
        let boundary = lower.as_bytes().get(after).copied();
        if boundary.is_some_and(|byte| byte.is_ascii_whitespace() || byte == b'/' || byte == b'>') {
            // A tag that never closes means none after it can either, so
            // stop rather than rescan the rest of the page for each one.
            let Some(end) = tag_end(html, start) else {
                break;
            };
            tags.push(&html[start..=end]);
            if tags.len() >= MAX_TAGS {
                break;
            }
            offset = end + 1;
            continue;
        }
        offset = after;
    }
    tags
}

fn tag_end(html: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    for (relative, byte) in html.as_bytes()[start..].iter().copied().enumerate() {
        match (quote, byte) {
            (Some(expected), value) if value == expected => quote = None,
            (None, b'\'' | b'"') => quote = Some(byte),
            (None, b'>') => return Some(start + relative),
            _ => {}
        }
    }
    None
}

fn attribute(tag: &str, wanted: &str) -> Option<String> {
    let bytes = tag.as_bytes();
    let mut cursor = 1;
    while cursor < bytes.len() && is_attribute_name_byte(bytes[cursor]) {
        cursor += 1;
    }
    while cursor < bytes.len() {
        while cursor < bytes.len() && (bytes[cursor].is_ascii_whitespace() || bytes[cursor] == b'/')
        {
            cursor += 1;
        }
        let name_start = cursor;
        while cursor < bytes.len() && is_attribute_name_byte(bytes[cursor]) {
            cursor += 1;
        }
        if cursor == name_start {
            cursor += 1;
            continue;
        }
        let name = &tag[name_start..cursor];
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'=') {
            continue;
        }
        cursor += 1;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let (value_start, value_end) = match bytes.get(cursor).copied() {
            Some(quote @ (b'\'' | b'"')) => {
                cursor += 1;
                let start = cursor;
                while cursor < bytes.len() && bytes[cursor] != quote {
                    cursor += 1;
                }
                (start, cursor)
            }
            Some(_) => {
                let start = cursor;
                while cursor < bytes.len()
                    && !bytes[cursor].is_ascii_whitespace()
                    && bytes[cursor] != b'>'
                {
                    cursor += 1;
                }
                (start, cursor)
            }
            None => return None,
        };
        if name.eq_ignore_ascii_case(wanted) {
            return Some(decode_entities(&tag[value_start..value_end]));
        }
        cursor = cursor.saturating_add(1);
    }
    None
}

fn is_attribute_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':')
}

/// Tags stripped, entities decoded, whitespace collapsed, 240 characters.
fn clean_text(value: String) -> Option<String> {
    let mut without_tags = String::with_capacity(value.len());
    let mut in_tag = false;
    for character in value.chars() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => without_tags.push(character),
            _ => {}
        }
    }
    let decoded = decode_entities(&without_tags);
    let collapsed = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    Some(collapsed.chars().take(MAX_TITLE_CHARS).collect())
}

fn decode_entities(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(index) = rest.find('&') {
        result.push_str(&rest[..index]);
        let entity_start = &rest[index + 1..];
        let Some(end) = entity_start.find(';').filter(|end| *end <= 12) else {
            result.push('&');
            rest = entity_start;
            continue;
        };
        let entity = &entity_start[..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            value if value.starts_with("#x") || value.starts_with("#X") => {
                u32::from_str_radix(&value[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            value if value.starts_with('#') => value[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        if let Some(character) = decoded {
            result.push(character);
        } else {
            result.push('&');
            result.push_str(entity);
            result.push(';');
        }
        rest = &entity_start[end + 1..];
    }
    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests;
