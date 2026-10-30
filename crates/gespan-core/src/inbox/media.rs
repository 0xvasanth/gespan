//! Inbox attachment fetching, preserving the original host and redirect policy.
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAX_PREVIEW_BYTES: u64 = 25 * 1024 * 1024;

const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_REDIRECTS: usize = 5;
const MAX_URL_BYTES: usize = 8192;
const USER_AGENT: &str = "Gespan";

#[derive(Clone, Debug, PartialEq, Eq)]
struct MediaUrl {
    host: String,
    path: String,
    url: String,
}

pub fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let mut current = parse_allowed_media_url(url)?;
    let token = if github_auth_host(&current.host) {
        github_auth_token()
    } else {
        None
    };
    let agent = ureq::AgentBuilder::new()
        .timeout(HTTP_TIMEOUT)
        .redirects(0)
        .build();

    for _ in 0..MAX_REDIRECTS {
        if !is_allowed_media_target(&current) {
            return Err("That media host is not allowed".into());
        }
        let mut request = agent
            .get(&current.url)
            .set("Accept", "image/*,video/*,*/*;q=0.1")
            .set("User-Agent", USER_AGENT);
        if let Some(token) = token.as_ref() {
            if github_auth_host(&current.host) {
                request = request.set("Authorization", &format!("Bearer {token}"));
            }
        }
        // ureq with redirects(0) returns 3xx as Ok, not Err(Status).
        let response = match request.call() {
            Ok(response) => response,
            Err(ureq::Error::Status(status, response)) if is_redirect(status) => response,
            Err(ureq::Error::Status(status, _)) => {
                return Err(format!("Media request failed ({status})"));
            }
            Err(_) => return Err("Could not fetch media".into()),
        };
        if is_redirect(response.status()) {
            let location = response
                .header("Location")
                .ok_or_else(|| "Media redirect is missing a Location header".to_string())?
                .to_string();
            current = redirect_target(&current.url, &location)?;
            continue;
        }
        return read_media_body(response);
    }
    Err("Too many media redirects".into())
}

fn read_media_body(response: ureq::Response) -> Result<Vec<u8>, String> {
    let status = response.status();
    if !(200..300).contains(&status) {
        return Err(format!("Media request failed ({status})"));
    }
    if let Some(content_type) = response.header("Content-Type") {
        let mime = content_type
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if mime.starts_with("text/html")
            || mime.starts_with("text/javascript")
            || mime == "application/javascript"
            || mime == "application/xhtml+xml"
        {
            return Err("That URL is not image or video media".into());
        }
    }
    if let Some(length) = response
        .header("Content-Length")
        .and_then(|value| value.parse::<u64>().ok())
    {
        if length > MAX_PREVIEW_BYTES {
            return Err(too_large());
        }
    }
    let mut reader = response.into_reader();
    let mut bytes = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|_| "Could not read media".to_string())?;
        if n == 0 {
            break;
        }
        if bytes.len() + n > MAX_PREVIEW_BYTES as usize {
            return Err(too_large());
        }
        bytes.extend_from_slice(&buf[..n]);
    }
    if bytes.is_empty() {
        return Err("Media response was empty".into());
    }
    Ok(bytes)
}

fn too_large() -> String {
    format!(
        "Media is too large to preview (maximum {} MB).",
        MAX_PREVIEW_BYTES / 1024 / 1024
    )
}

fn parse_allowed_media_url(raw: &str) -> Result<MediaUrl, String> {
    let parsed = parse_https_url(raw)?;
    if !is_allowed_media_target(&parsed) {
        return Err("That media host is not allowed".into());
    }
    Ok(parsed)
}

fn parse_https_url(raw: &str) -> Result<MediaUrl, String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > MAX_URL_BYTES {
        return Err("Media URL is invalid".into());
    }
    let rest = raw
        .strip_prefix("https://")
        .ok_or_else(|| "Only HTTPS media URLs are allowed".to_string())?;
    if rest.is_empty() || rest.starts_with('/') {
        return Err("Media URL is invalid".into());
    }
    let (authority, path_query) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, "/"),
    };
    if authority.is_empty() || authority.contains('@') || authority.contains('\\') {
        return Err("Media URL is invalid".into());
    }
    if authority.starts_with('[') {
        return Err("Media URL is invalid".into());
    }
    let host = authority
        .split(':')
        .next()
        .unwrap_or("")
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host.is_empty() || host == "localhost" || host.parse::<std::net::IpAddr>().is_ok() {
        return Err("Media URL is invalid".into());
    }
    let without_hash = path_query.split('#').next().unwrap_or(path_query);
    let path = without_hash.split('?').next().unwrap_or(without_hash);
    if path_has_dotdot(path) {
        return Err("Media URL is invalid".into());
    }
    let url = raw.split('#').next().unwrap_or(raw).to_string();
    Ok(MediaUrl {
        host,
        path: path.to_string(),
        url,
    })
}

fn redirect_target(base: &str, location: &str) -> Result<MediaUrl, String> {
    let location = location.trim();
    if location.is_empty() {
        return Err("Media redirect is missing a Location header".into());
    }
    if location.starts_with("https://") {
        return parse_https_url(location);
    }
    if location.starts_with("http://") {
        return Err("Insecure media redirect".into());
    }
    let base = parse_https_url(base)?;
    let joined = if let Some(rest) = location.strip_prefix("//") {
        format!("https://{rest}")
    } else if location.starts_with('/') {
        format!("https://{}{}", base.host, location)
    } else {
        let dir = base
            .path
            .rsplit_once('/')
            .map(|(head, _)| head)
            .unwrap_or("");
        format!("https://{}{dir}/{location}", base.host)
    };
    parse_https_url(&joined)
}

fn is_allowed_media_target(url: &MediaUrl) -> bool {
    if is_github_site(&url.host) {
        return is_github_attachment_path(&url.path);
    }
    is_github_media_cdn(&url.host) || is_linear_uploads(&url.host) || is_github_asset_s3(&url.host)
}

fn is_github_attachment_path(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    if path.starts_with("/user-attachments/") {
        return true;
    }
    // /owner/repo/assets/<user-id>/<uuid>
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    parts.len() >= 4 && parts[2] == "assets" && parts[3].bytes().all(|byte| byte.is_ascii_digit())
}

fn is_github_site(host: &str) -> bool {
    host == "github.com" || host == "www.github.com"
}

fn is_github_media_cdn(host: &str) -> bool {
    host == "githubusercontent.com" || host.ends_with(".githubusercontent.com")
}

fn is_linear_uploads(host: &str) -> bool {
    host == "uploads.linear.app" || host.ends_with(".uploads.linear.app")
}

fn is_github_asset_s3(host: &str) -> bool {
    let s3 = host.ends_with(".s3.amazonaws.com")
        || (host.contains(".s3.") && host.ends_with(".amazonaws.com"));
    s3 && (host.starts_with("github-production-user-asset-")
        || host.starts_with("github-production-media.")
        || host.starts_with("github-production-media-"))
}

fn github_auth_host(host: &str) -> bool {
    // JWT-signed githubusercontent URLs reject an extra Authorization header.
    is_github_site(host)
}

fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

fn path_has_dotdot(path: &str) -> bool {
    path.split('/').any(|segment| {
        let lower = segment.to_ascii_lowercase();
        lower == ".." || lower == "%2e%2e" || lower == "%2e." || lower == ".%2e"
    })
}

fn github_auth_token() -> Option<String> {
    let home = crate::accounts::home()?;
    super::run_gh(&home, &["auth", "token"], false).ok()
}

/// Decode bounded image bytes before handing them to the native image cache.
/// Videos fetched for item descriptions, by URL. GPUI cannot play video in
/// place, so each one is saved once and the file viewer plays it.
static VIDEOS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, PathBuf>>> =
    std::sync::LazyLock::new(Default::default);

/// The saved copy of a description video, once it has been fetched.
pub fn video(url: &str) -> Option<PathBuf> {
    VIDEOS.lock().ok()?.get(url.trim()).cloned()
}

/// For tests of the views that show a saved video.
#[doc(hidden)]
pub fn remember_video(url: &str, path: PathBuf) {
    if let Ok(mut videos) = VIDEOS.lock() {
        videos.insert(url.trim().to_owned(), path);
    }
}

/// inboxMedia.ts's sniffVideoType: MP4 and QuickTime by their `ftyp` box
/// (AVIF images share it), and WebM by its EBML header. The file extension.
pub fn video_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        return match &bytes[8..12] {
            b"avif" | b"avis" => None,
            b"qt  " => Some("mov"),
            _ => Some("mp4"),
        };
    }
    bytes
        .starts_with(&[0x1a, 0x45, 0xdf, 0xa3])
        .then_some("webm")
}

/// Write `bytes` under `dir`, named by `url`, and return the file. The bytes
/// come from a signed-in fetch and may be private, so they go through a
/// uniquely named file only this user can read, then replace any older copy.
fn save_video(dir: &Path, url: &str, bytes: &[u8], extension: &str) -> Result<PathBuf, String> {
    use std::hash::{Hash, Hasher};
    use std::io::Write;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    url.trim().hash(&mut hasher);
    let path = dir.join(format!("{:016x}.{extension}", hasher.finish()));
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.persist(&path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Gespan's own folder for description videos, under its per-user data folder.
static VIDEO_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Set where description videos are kept, and empty it: a video is only ever
/// played from a copy this run fetched, never from one left behind.
pub fn init_video_dir(dir: PathBuf) {
    let _ = std::fs::remove_dir_all(&dir);
    let _ = VIDEO_DIR.set(dir);
}

/// A description image as PNG bytes. A video is saved for [`video`] instead
/// and reported as an error, so the same download serves both.
pub fn png(url: &str) -> Result<Vec<u8>, String> {
    let bytes = fetch(url)?;
    if let Some(extension) = video_extension(&bytes) {
        let dir = VIDEO_DIR
            .get()
            .ok_or_else(|| "Gespan has no folder for videos".to_owned())?;
        let path = save_video(dir, url, &bytes, extension)?;
        if let Ok(mut videos) = VIDEOS.lock() {
            videos.insert(url.trim().to_owned(), path);
        }
        return Err("This attachment is a video".into());
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "Unsupported image".to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|_| "Unsupported image".to_string())?;
    // Keep document images clear at 2x display scale without retaining huge bitmaps.
    crate::project_appearance::png_bytes(image.thumbnail(2304, 1792).into_rgba8())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn videos_are_told_from_images_by_their_bytes() {
        let ftyp = |brand: &[u8]| [&[0, 0, 0, 0x20][..], b"ftyp", brand, &[0; 8]].concat();
        assert_eq!(video_extension(&ftyp(b"isom")), Some("mp4"));
        assert_eq!(video_extension(&ftyp(b"mp42")), Some("mp4"));
        assert_eq!(video_extension(&ftyp(b"qt  ")), Some("mov"));
        assert_eq!(video_extension(&ftyp(b"avif")), None, "AVIF is an image");
        assert_eq!(video_extension(&ftyp(b"avis")), None);
        assert_eq!(video_extension(&[0x1a, 0x45, 0xdf, 0xa3, 1]), Some("webm"));
        assert_eq!(video_extension(b"\x89PNG\r\n\x1a\n...."), None);
        assert_eq!(video_extension(b"ftyp"), None, "too short");
    }

    #[test]
    fn a_video_is_saved_per_url_privately_and_never_half_written() {
        let dir = tempfile::tempdir().unwrap();
        let videos = dir.path().join("inbox-media");
        let url = "https://github.com/user-attachments/assets/clip";
        let first = save_video(&videos, url, b"movie", "mp4").unwrap();
        assert_eq!(first.parent().unwrap(), videos);
        assert_eq!(first.extension().unwrap(), "mp4");
        assert_eq!(std::fs::read(&first).unwrap(), b"movie");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = first.metadata().unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "only this user can read a private attachment");
        }
        // A newer fetch of the same URL replaces the copy in place.
        let again = save_video(&videos, &format!(" {url} "), b"movie 2", "mp4").unwrap();
        assert_eq!(again, first);
        assert_eq!(std::fs::read(&again).unwrap(), b"movie 2");
        let other = save_video(&videos, "https://uploads.linear.app/x", b"webm", "webm").unwrap();
        assert_ne!(other, first);
        assert_eq!(
            std::fs::read_dir(&videos).unwrap().count(),
            2,
            "no partial files are left"
        );
    }

    #[test]
    fn github_and_linear_attachments_are_allowed() {
        assert!(parse_allowed_media_url(
            "https://github.com/user-attachments/assets/aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
        )
        .is_ok());
        assert!(
            parse_allowed_media_url("https://user-images.githubusercontent.com/1/shot.png").is_ok()
        );
        assert!(parse_allowed_media_url("https://uploads.linear.app/org/uuid/file.png").is_ok());
        assert!(parse_allowed_media_url(
            "https://github-production-user-asset-6210df.s3.amazonaws.com/1/shot.png"
        )
        .is_ok());
        assert!(parse_allowed_media_url(
            "https://github.com/acme/web/assets/12/aaaaaaaa-bbbb-cccc"
        )
        .is_ok());
    }

    #[test]
    fn github_pages_other_hosts_and_traversal_are_rejected() {
        assert!(parse_allowed_media_url("https://github.com/acme/web/issues/1").is_err());
        assert!(parse_allowed_media_url("https://github.com/acme/web/assets").is_err());
        assert!(parse_allowed_media_url("https://github.com/user-attachments/../login").is_err());
        assert!(parse_allowed_media_url("http://github.com/user-attachments/assets/x").is_err());
        assert!(parse_allowed_media_url("https://evil.example/shot.png").is_err());
        assert!(parse_allowed_media_url(
            "https://github.com@evil.example/user-attachments/assets/x"
        )
        .is_err());
        assert!(parse_allowed_media_url("https://127.0.0.1/shot.png").is_err());
    }

    #[test]
    fn redirects_stay_on_allowed_hosts() {
        let next = redirect_target(
            "https://github.com/user-attachments/assets/abcd",
            "https://objects.githubusercontent.com/github-production-user-asset/1",
        )
        .unwrap();
        assert_eq!(next.host, "objects.githubusercontent.com");
        assert!(redirect_target(
            "https://github.com/user-attachments/assets/abcd",
            "https://evil.example/shot.png"
        )
        .is_ok());
        assert!(!is_allowed_media_target(
            &redirect_target(
                "https://github.com/user-attachments/assets/abcd",
                "https://evil.example/shot.png"
            )
            .unwrap()
        ));
        assert!(redirect_target(
            "https://github.com/user-attachments/assets/abcd",
            "http://objects.githubusercontent.com/x"
        )
        .is_err());
    }
}
