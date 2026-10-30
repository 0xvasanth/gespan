use super::*;
use std::{
    io::Write,
    net::{TcpListener, TcpStream},
    sync::atomic::{AtomicUsize, Ordering},
};

// ---- HTML parsing -------------------------------------------------------

#[test]
fn og_title_wins_over_the_document_title_with_attributes_in_any_order() {
    let html = r#"
      <html><head>
        <title>Fallback &amp; title</title>
        <meta content="A nicer &quot;title&quot;" property="og:title">
        <link href="/assets/icon.png" sizes="32x32" rel="shortcut icon">
      </head></html>
    "#;
    assert_eq!(document_title(html).as_deref(), Some("A nicer \"title\""));
    assert_eq!(
        document_icon_href(html).as_deref(),
        Some("/assets/icon.png")
    );
}

#[test]
fn twitter_title_is_used_when_there_is_no_og_title() {
    let html = r#"<title>Plain</title><meta name="Twitter:Title" content='Card title'>"#;
    assert_eq!(document_title(html).as_deref(), Some("Card title"));
}

#[test]
fn an_empty_og_title_falls_through_to_the_next_source() {
    let html = r#"<meta property="og:title" content="   "><title>Real title</title>"#;
    assert_eq!(document_title(html).as_deref(), Some("Real title"));
}

#[test]
fn the_document_title_is_cleaned_of_tags_entities_and_whitespace() {
    let html = "<TITLE data-x='a>b'>  Example <b>docs</b> &amp; API&#x21; &#8212;\n v2  </TITLE>";
    assert_eq!(
        document_title(html).as_deref(),
        Some("Example docs & API! \u{2014} v2")
    );
}

#[test]
fn unknown_or_unterminated_entities_are_kept_as_written() {
    assert_eq!(decode_entities("a &copy; b"), "a &copy; b");
    assert_eq!(decode_entities("Tom & Jerry"), "Tom & Jerry");
    assert_eq!(decode_entities("&#39;q&apos; &lt;x&gt;"), "'q' <x>");
    // An invalid code point is not turned into a character.
    assert_eq!(decode_entities("&#xD800;"), "&#xD800;");
}

#[test]
fn long_titles_are_cut_to_240_characters() {
    let html = format!("<title>{}</title>", "é".repeat(500));
    let title = document_title(&html).unwrap();
    assert_eq!(title.chars().count(), 240);
}

#[test]
fn a_page_without_a_title_or_icon_has_neither() {
    let html = "<html><head><meta charset=utf-8><link rel=stylesheet href=a.css></head></html>";
    assert_eq!(document_title(html), None);
    assert_eq!(document_icon_href(html), None);
    assert_eq!(document_title("<title>   </title>"), None);
    assert_eq!(document_title("<title>never closed"), None);
}

#[test]
fn icon_links_need_an_icon_rel_and_a_non_empty_href() {
    // `<linkx>` is not a link, `rel="apple-touch-icon"` is not `icon`, and an
    // empty href is skipped in favour of the next icon link.
    let html = r#"
      <linkx rel="icon" href="/wrong.png">
      <link rel="apple-touch-icon" href="/touch.png">
      <link rel="icon" href="  ">
      <LINK REL=ICON HREF=/unquoted.ico>
    "#;
    assert_eq!(document_icon_href(html).as_deref(), Some("/unquoted.ico"));
}

#[test]
fn quoted_angle_brackets_do_not_end_a_tag() {
    let html = r#"<meta name="description" content="a > b" property="og:title">"#;
    let tags = opening_tags(html, "meta");
    assert_eq!(tags, vec![html]);
    // The attribute after the quoted `>` is still read. Like Tauri, the
    // title's cleanup drops a stray bracket.
    assert_eq!(document_title(html).as_deref(), Some("a b"));
}

#[test]
fn only_html_responses_are_read_for_a_title() {
    assert!(is_html(None));
    assert!(is_html(Some("text/html; charset=utf-8")));
    assert!(is_html(Some("APPLICATION/XHTML+XML")));
    assert!(!is_html(Some("application/json")));
    assert!(!is_html(Some("image/png")));
}

#[test]
fn icons_are_sniffed_from_their_bytes_and_svg_is_refused() {
    assert_eq!(
        image_mime(None, b"\x89PNG\r\n\x1a\nrest"),
        Some("image/png")
    );
    assert_eq!(image_mime(None, &[0xff, 0xd8, 0xff, 0]), Some("image/jpeg"));
    assert_eq!(image_mime(None, b"GIF89a.."), Some("image/gif"));
    assert_eq!(
        image_mime(None, b"RIFF\0\0\0\0WEBPVP8 "),
        Some("image/webp")
    );
    assert_eq!(image_mime(None, &[0, 0, 1, 0, 1]), Some("image/x-icon"));
    // The header alone vouches only for `.ico`.
    assert_eq!(
        image_mime(Some("image/vnd.microsoft.icon"), b"??"),
        Some("image/x-icon")
    );
    assert_eq!(image_mime(Some("image/png"), b"not a png"), None);
    assert_eq!(image_mime(Some("image/svg+xml"), b"<svg/>"), None);
}

// ---- Guards -------------------------------------------------------------

#[test]
fn rejects_local_and_special_networks() {
    for address in [
        "0.0.0.0",
        "127.0.0.1",
        "10.0.0.1",
        "169.254.1.1",
        "172.20.1.1",
        "192.168.1.1",
        "100.64.0.1",
        "192.0.2.1",
        "198.18.0.1",
        "203.0.113.9",
        "224.0.0.1",
        "255.255.255.255",
        "::",
        "::1",
        "::127.0.0.1",
        "::ffff:127.0.0.1",
        "::ffff:10.0.0.1",
        "fc00::1",
        "fe80::1",
        "ff02::1",
        "2001:db8::1",
    ] {
        assert!(!is_public_ip(address.parse().unwrap()), "{address}");
    }
    for address in [
        "8.8.8.8",
        "172.32.0.1",
        "100.128.0.1",
        "2606:4700:4700::1111",
    ] {
        assert!(is_public_ip(address.parse().unwrap()), "{address}");
    }
}

#[test]
fn accepts_only_plain_public_http_urls() {
    for url in [
        "",
        "not a url",
        "file:///tmp/page.html",
        "ftp://example.com/",
        "javascript:alert(1)",
        "data:text/html,hi",
        "https://user:secret@example.com",
        "https://user@example.com",
        "http://localhost:3000",
        "http://LOCALHOST./",
        "http://app.localhost/",
        "http://printer.local/",
        "http://127.0.0.1/",
        "http://10.1.2.3/",
        "http://[::1]:3000",
        "http://[fe80::1]/",
        "http://169.254.169.254/latest/meta-data/",
    ] {
        assert!(parse_public_url(url, Network::Public).is_err(), "{url}");
    }
    let long = format!("https://example.com/{}", "a".repeat(MAX_URL_LEN));
    assert!(parse_public_url(&long, Network::Public).is_err());
    // A public IP literal needs no lookup to pass.
    assert!(parse_public_url(" https://8.8.8.8/x ", Network::Public).is_ok());
}

#[test]
fn the_resolver_refuses_names_that_reach_private_addresses() {
    for netloc in ["127.0.0.1:80", "[::1]:443", "10.0.0.8:80", "localhost:80"] {
        let error = checked_addresses(netloc, Network::Public).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{netloc}");
    }
    assert!(checked_addresses("8.8.8.8:443", Network::Public).is_ok());
    // The tests' loopback allowance still refuses other private ranges.
    assert!(checked_addresses("127.0.0.1:80", Network::Loopback).is_ok());
    assert!(checked_addresses("192.168.0.1:80", Network::Loopback).is_err());
}

#[test]
fn fetching_a_private_link_fails_before_any_request() {
    assert!(fetch("http://127.0.0.1:9/").is_err());
    assert!(fetch("http://localhost:9/").is_err());
    assert!(fetch("file:///etc/hosts").is_err());
}

// ---- Fetching from a local server --------------------------------------

/// One canned response: status, headers and body.
#[derive(Clone)]
struct Reply {
    status: &'static str,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}
impl Reply {
    fn ok(content_type: &str, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: "200 OK",
            headers: vec![("Content-Type".into(), content_type.into())],
            body: body.into(),
        }
    }
    fn status(status: &'static str) -> Self {
        Self {
            status,
            headers: vec![],
            body: vec![],
        }
    }
    fn redirect(location: &str) -> Self {
        Self {
            status: "302 Found",
            headers: vec![("Location".into(), location.into())],
            body: vec![],
        }
    }
    fn body(mut self, body: Vec<u8>) -> Self {
        self.body = body;
        self
    }
    fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

/// A loopback HTTP server answering each path from `routes` (404 otherwise)
/// and counting requests. Each connection is closed after one response, and
/// a body without a `Content-Length` header runs until that close.
struct Server {
    base: String,
    hits: Arc<AtomicUsize>,
}
fn serve(routes: Vec<(&'static str, Reply)>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    let routes: HashMap<&'static str, Reply> = routes.into_iter().collect();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            counter.fetch_add(1, Ordering::SeqCst);
            answer(stream, &routes);
        }
    });
    Server { base, hits }
}
fn answer(mut stream: TcpStream, routes: &HashMap<&'static str, Reply>) {
    let mut request = Vec::new();
    let mut byte = [0u8; 1];
    while !request.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(1) => request.push(byte[0]),
            _ => return,
        }
    }
    let request = String::from_utf8_lossy(&request);
    let path = request.split_whitespace().nth(1).unwrap_or("/");
    let reply = routes
        .get(path)
        .cloned()
        .unwrap_or_else(|| Reply::status("404 Not Found"));
    let mut head = format!("HTTP/1.1 {}\r\nConnection: close\r\n", reply.status);
    for (name, value) in &reply.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&reply.body);
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(width, height, image::Rgba([200, 30, 30, 255]));
    crate::project_appearance::png_bytes(image).unwrap()
}
fn ico() -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(16, 16, image::Rgba([0, 90, 200, 255]));
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Ico).unwrap();
    bytes.into_inner()
}
fn decoded_side(png: &[u8]) -> (u32, u32) {
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png).unwrap();
    (image.width(), image.height())
}

#[test]
fn a_page_gives_its_title_and_its_linked_icon_resolved_against_the_page() {
    let server = serve(vec![
        (
            "/docs/guide",
            Reply::ok(
                "text/html; charset=utf-8",
                r#"<html><head><title>Guide &amp; API</title>
                   <link rel="icon" href="../static/icon.png"></head></html>"#,
            ),
        ),
        ("/static/icon.png", Reply::ok("image/png", png(64, 48))),
    ]);
    let preview = fetch_with(&format!("{}/docs/guide", server.base), Network::Loopback).unwrap();
    assert_eq!(preview.title.as_deref(), Some("Guide & API"));
    // Shrunk to the chip's size, keeping its shape.
    assert_eq!(decoded_side(&preview.favicon_png.unwrap()), (32, 24));
}

#[test]
fn a_missing_linked_icon_falls_back_to_the_sites_favicon_ico() {
    let server = serve(vec![
        (
            "/a/page",
            Reply::ok(
                "text/html",
                r#"<title>Page</title><link rel=icon href=/gone.png>"#,
            ),
        ),
        // Served without a content type: the bytes say it is an icon.
        ("/favicon.ico", Reply::status("200 OK").body(ico())),
    ]);
    let preview = fetch_with(&format!("{}/a/page", server.base), Network::Loopback).unwrap();
    assert_eq!(preview.title.as_deref(), Some("Page"));
    assert_eq!(decoded_side(&preview.favicon_png.unwrap()), (16, 16));
}

#[test]
fn an_icon_that_is_not_an_image_leaves_no_favicon() {
    let server = serve(vec![
        ("/", Reply::ok("text/html", "<title>No icon</title>")),
        (
            "/favicon.ico",
            Reply::ok("text/html", "<html>not found</html>"),
        ),
    ]);
    let preview = fetch_with(&format!("{}/", server.base), Network::Loopback).unwrap();
    assert_eq!(preview.title.as_deref(), Some("No icon"));
    assert_eq!(preview.favicon_png, None);
}

#[test]
fn a_non_html_link_has_no_title_and_no_icon_requests() {
    let server = serve(vec![(
        "/data.json",
        Reply::ok("application/json", r#"{"title":"<title>no</title>"}"#),
    )]);
    let preview = fetch_with(&format!("{}/data.json", server.base), Network::Loopback).unwrap();
    assert_eq!(preview, LinkPreview::default());
    assert_eq!(server.hits.load(Ordering::SeqCst), 1);
}

#[test]
fn redirects_are_followed_and_icons_resolve_against_the_final_page() {
    let server = serve(vec![
        ("/old", Reply::redirect("/new/")),
        (
            "/new/",
            Reply::ok(
                "text/html",
                r#"<title>Moved</title><link rel=icon href="i.png">"#,
            ),
        ),
        ("/new/i.png", Reply::ok("image/png", png(8, 8))),
    ]);
    let preview = fetch_with(&format!("{}/old", server.base), Network::Loopback).unwrap();
    assert_eq!(preview.title.as_deref(), Some("Moved"));
    assert_eq!(decoded_side(&preview.favicon_png.unwrap()), (8, 8));
}

#[test]
fn a_redirect_to_a_private_address_or_another_scheme_is_refused() {
    let server = serve(vec![
        ("/to-private", Reply::redirect("http://10.0.0.1/admin")),
        ("/to-metadata", Reply::redirect("http://169.254.169.254/")),
        ("/to-local-name", Reply::redirect("http://localhost/")),
        ("/to-file", Reply::redirect("file:///etc/passwd")),
        ("/nowhere", Reply::status("302 Found")),
    ]);
    for path in [
        "/to-private",
        "/to-metadata",
        "/to-local-name",
        "/to-file",
        "/nowhere",
    ] {
        let result = fetch_with(&format!("{}{path}", server.base), Network::Loopback);
        assert!(result.is_err(), "{path}: {result:?}");
    }
    // Only the five redirecting requests were made; nothing followed them.
    assert_eq!(server.hits.load(Ordering::SeqCst), 5);
}

#[test]
fn a_redirect_loop_stops_after_five_hops() {
    let server = serve(vec![("/loop", Reply::redirect("/loop"))]);
    let error = fetch_with(&format!("{}/loop", server.base), Network::Loopback).unwrap_err();
    assert_eq!(error, "Too many link preview redirects");
    assert_eq!(server.hits.load(Ordering::SeqCst), MAX_REDIRECTS + 1);
}

#[test]
fn oversized_pages_are_refused_by_header_or_by_reading() {
    let big = vec![b'a'; MAX_PAGE_BYTES + 1];
    let exact = format!("<title>Fits</title>{}", " ".repeat(MAX_PAGE_BYTES - 19));
    assert_eq!(exact.len(), MAX_PAGE_BYTES);
    let server = serve(vec![
        // No length header: the body runs until the connection closes.
        ("/streamed", Reply::ok("text/html", big)),
        (
            "/declared",
            Reply::ok("text/html", "<title>small</title>")
                .header("Content-Length", &(MAX_PAGE_BYTES + 1).to_string()),
        ),
        ("/exact", Reply::ok("text/html", exact)),
    ]);
    for path in ["/streamed", "/declared"] {
        let error = fetch_with(&format!("{}{path}", server.base), Network::Loopback).unwrap_err();
        assert_eq!(error, "Link preview response is too large", "{path}");
    }
    let preview = fetch_with(&format!("{}/exact", server.base), Network::Loopback).unwrap();
    assert_eq!(preview.title.as_deref(), Some("Fits"));
}

#[test]
fn an_oversized_icon_is_dropped_but_the_title_is_kept() {
    let mut huge = png(4, 4);
    huge.resize(MAX_ICON_BYTES + 1, 0);
    let server = serve(vec![
        (
            "/",
            Reply::ok(
                "text/html",
                "<title>Big icon</title><link rel=icon href=/big.png>",
            ),
        ),
        ("/big.png", Reply::ok("image/png", huge.clone())),
        ("/favicon.ico", Reply::ok("image/png", huge)),
    ]);
    let preview = fetch_with(&format!("{}/", server.base), Network::Loopback).unwrap();
    assert_eq!(preview.title.as_deref(), Some("Big icon"));
    assert_eq!(preview.favicon_png, None);
}

#[test]
fn an_error_status_fails_the_preview() {
    let server = serve(vec![("/missing", Reply::status("404 Not Found"))]);
    let error = fetch_with(&format!("{}/missing", server.base), Network::Loopback).unwrap_err();
    assert_eq!(error, "Link preview request failed (404)");
}

// ---- Cache --------------------------------------------------------------

#[test]
fn the_cache_keeps_successes_and_retries_failures() {
    let cache = Cache::new(8);
    let calls = AtomicUsize::new(0);
    let failing = |_: &str| {
        calls.fetch_add(1, Ordering::SeqCst);
        Err::<LinkPreview, _>("offline".to_string())
    };
    assert!(cache.load("https://a.example/", failing).is_err());
    assert!(cache.load("https://a.example/", failing).is_err());
    // A failure is not remembered: the second load asked again.
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(cache.get("https://a.example/").is_none());

    let ok = |_: &str| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(LinkPreview {
            title: Some("A".into()),
            favicon_png: None,
        })
    };
    let first = cache.load("https://a.example/", ok).unwrap();
    let second = cache.load("https://a.example/", ok).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(
        cache.get("https://a.example/").unwrap().title.as_deref(),
        Some("A")
    );
}

#[test]
fn the_cache_evicts_its_oldest_entry_when_full() {
    let cache = Cache::new(2);
    let preview = |title: &str| LinkPreview {
        title: Some(title.into()),
        favicon_png: None,
    };
    cache.insert("https://1.example/", preview("1"));
    cache.insert("https://2.example/", preview("2"));
    // Replacing an entry does not make room or reorder it.
    cache.insert("https://1.example/", preview("1b"));
    cache.insert("https://3.example/", preview("3"));
    assert!(cache.get("https://1.example/").is_none());
    assert_eq!(
        cache.get("https://2.example/").unwrap().title.as_deref(),
        Some("2")
    );
    assert_eq!(
        cache.get("https://3.example/").unwrap().title.as_deref(),
        Some("3")
    );
}

#[test]
fn a_cached_link_is_served_without_a_request() {
    let server = serve(vec![("/", Reply::ok("text/html", "<title>Once</title>"))]);
    let url = format!("{}/", server.base);
    let cache = Cache::new(4);
    let fetch = |url: &str| fetch_with(url, Network::Loopback);
    assert_eq!(
        cache.load(&url, fetch).unwrap().title.as_deref(),
        Some("Once")
    );
    let after_first = server.hits.load(Ordering::SeqCst);
    assert_eq!(
        cache.load(&url, fetch).unwrap().title.as_deref(),
        Some("Once")
    );
    assert_eq!(server.hits.load(Ordering::SeqCst), after_first);
}
