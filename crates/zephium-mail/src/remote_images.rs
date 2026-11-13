//! Remote images for the reading pane (T-0028).
//!
//! The frame's CSP has no network for images and must not get any, so the
//! backend fetches the images a message references and hands them back as
//! `data:` URLs. Everything here is bounded and refuses by default:
//!
//! * only URLs that the message's own HTML references, at most
//!   [`MAX_REMOTE_IMAGES`] per request;
//! * `https:` on the default port, no credentials in the URL, no address
//!   that is not public (loopback, private, link-local, shared, multicast,
//!   documentation, unique-local, mapped or tunnelled forms of those);
//! * at most [`MAX_REMOTE_REDIRECTS`] redirects, each one checked again;
//! * raster images only (PNG, JPEG, GIF, WebP), decided by the bytes and not
//!   by the server's word; SVG is never returned;
//! * [`MAX_REMOTE_IMAGE_BYTES`] per image, [`MAX_REMOTE_TOTAL_BYTES`] in
//!   total and [`REMOTE_TOTAL_TIMEOUT`] for the whole request.
//!
//! The network itself sits behind [`ImageFetcher`] so the policy is tested
//! without one. A fetcher must send no cookies, no credentials and no
//! referrer.
use crate::error::MailError;
use base64::Engine as _;
use futures::stream::{self, StreamExt};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;
use url::{Host, Url};

/// Images fetched for one message.
pub const MAX_REMOTE_IMAGES: usize = 24;
/// Bytes of one image.
pub const MAX_REMOTE_IMAGE_BYTES: usize = 2 * 1024 * 1024;
/// Bytes of all images of one request.
pub const MAX_REMOTE_TOTAL_BYTES: usize = 8 * 1024 * 1024;
/// Bytes of one URL.
pub const MAX_REMOTE_URL_BYTES: usize = 2048;
/// Redirects followed for one image.
pub const MAX_REMOTE_REDIRECTS: usize = 3;
/// Time for one image, including its redirects.
pub const REMOTE_IMAGE_TIMEOUT: Duration = Duration::from_secs(10);
/// Time for the whole request.
pub const REMOTE_TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
/// Images fetched at the same time.
const CONCURRENCY: usize = 4;
/// `<img>` tags read from one message when looking for references.
const MAX_SCANNED_TAGS: usize = 2_000;

/// What a fetcher got back for one URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedImage {
    /// The `Content-Type` the server sent, if any.
    pub content_type: Option<String>,
    /// The body, at most the `max_bytes` that was asked for.
    pub bytes: Vec<u8>,
}

/// Fetches one `https:` URL. Implementations send no cookies, credentials or
/// referrer, follow redirects only through [`redirect_allowed`], refuse
/// non-public addresses at connection time (so DNS rebinding cannot help) and
/// stop reading at `max_bytes`.
pub trait ImageFetcher: Send + Sync {
    /// Fetches `url`; any failure is an error, never partial data.
    fn fetch<'a>(
        &'a self,
        url: &'a Url,
        max_bytes: usize,
    ) -> futures::future::BoxFuture<'a, Result<FetchedImage, MailError>>;
}

/// One image ready for the frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteImage {
    /// The URL exactly as the message wrote it.
    pub url: String,
    /// `data:image/...;base64,...`
    pub data_url: String,
}

// ---------------------------------------------------------------------------
// URL policy
// ---------------------------------------------------------------------------

/// True for an address that is reachable on the public internet.
#[must_use]
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => public_v4(v4),
        IpAddr::V6(v6) => public_v6(v6),
    }
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        // 0.0.0.0/8 "this network"
        || a == 0
        // 100.64.0.0/10 carrier-grade NAT
        || (a == 100 && (64..128).contains(&b))
        // 192.0.0.0/24 IETF protocol assignments
        || (a == 192 && b == 0 && c == 0)
        // 198.18.0.0/15 benchmarking
        || (a == 198 && (18..20).contains(&b))
        // 240.0.0.0/4 reserved
        || a >= 240)
}

fn public_v6(ip: Ipv6Addr) -> bool {
    if ip.is_unspecified() || ip.is_loopback() || ip.is_multicast() {
        return false;
    }
    if let Some(v4) = ip.to_ipv4_mapped() {
        return public_v4(v4);
    }
    let s = ip.segments();
    // fc00::/7 unique local, fe80::/10 link local, fec0::/10 site local.
    if s[0] & 0xfe00 == 0xfc00 || s[0] & 0xffc0 == 0xfe80 || s[0] & 0xffc0 == 0xfec0 {
        return false;
    }
    // 2001:db8::/32 documentation, 2001::/32 Teredo, 100::/64 discard.
    if (s[0] == 0x2001 && (s[1] == 0x0db8 || s[1] == 0))
        || (s[0] == 0x0100 && s[1] == 0 && s[2] == 0 && s[3] == 0)
    {
        return false;
    }
    // 64:ff9b::/96 NAT64 and 2002::/16 6to4 carry an IPv4 address.
    if s[0] == 0x0064 && s[1] == 0xff9b && s[2..6].iter().all(|part| *part == 0) {
        return public_v4(embedded_v4(s[6], s[7]));
    }
    if s[0] == 0x2002 {
        return public_v4(embedded_v4(s[1], s[2]));
    }
    // ::/8 holds the unspecified, loopback and compatible forms.
    s[0] >> 8 != 0
}

fn embedded_v4(high: u16, low: u16) -> Ipv4Addr {
    let [a, b] = high.to_be_bytes();
    let [c, d] = low.to_be_bytes();
    Ipv4Addr::new(a, b, c, d)
}

/// True for a host name that can only mean a private network.
fn private_name(host: &str) -> bool {
    let host = host.trim_end_matches('.');
    !host.contains('.')
        || host == "localhost"
        || [
            ".localhost",
            ".local",
            ".internal",
            ".lan",
            ".home.arpa",
            ".corp",
        ]
        .iter()
        .any(|suffix| host.ends_with(suffix))
}

/// Checks that `raw` may be fetched: a plain `https:` URL to a public host.
///
/// A host name is checked here only by its spelling; the fetcher must check
/// the addresses it resolves to ([`is_public_ip`]) when it connects.
///
/// # Errors
/// [`MailError::InvalidInput`] for anything else.
pub fn check_url(raw: &str) -> Result<Url, MailError> {
    if raw.is_empty() || raw.len() > MAX_REMOTE_URL_BYTES || raw.chars().any(char::is_control) {
        return Err(MailError::InvalidInput);
    }
    let url = Url::parse(raw).map_err(|_| MailError::InvalidInput)?;
    // `port()` is `None` for the default port of the scheme.
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err(MailError::InvalidInput);
    }
    let allowed = match url.host().ok_or(MailError::InvalidInput)? {
        Host::Domain(name) => !private_name(name),
        Host::Ipv4(ip) => public_v4(ip),
        Host::Ipv6(ip) => public_v6(ip),
    };
    if allowed {
        Ok(url)
    } else {
        Err(MailError::InvalidInput)
    }
}

/// Whether a redirect from a request that already followed `followed`
/// redirects may go to `next`. Every hop obeys [`check_url`].
#[must_use]
pub fn redirect_allowed(followed: usize, next: &Url) -> bool {
    followed < MAX_REMOTE_REDIRECTS && check_url(next.as_str()).is_ok()
}

// ---------------------------------------------------------------------------
// Which URLs a message references
// ---------------------------------------------------------------------------

/// Decodes the character references a browser decodes in an attribute value.
fn decode_entities(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let end = rest.find(';').filter(|end| *end <= 10);
        let decoded = end.and_then(|end| {
            let name = &rest[1..end];
            let ch = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                // The few more the frame's sanitiser also decodes.
                "colon" => Some(':'),
                "tab" => Some('\t'),
                "newline" => Some('\n'),
                "lpar" => Some('('),
                "rpar" => Some(')'),
                "nbsp" => Some('\u{a0}'),
                _ => name
                    .strip_prefix("#x")
                    .or_else(|| name.strip_prefix("#X"))
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| name.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                    .and_then(char::from_u32),
            };
            ch.map(|ch| (ch, end))
        });
        match decoded {
            Some((ch, end)) => {
                out.push(ch);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The `src` of every `<img>` in `html`, decoded, in order, without
/// duplicates. Comments are skipped. At most [`MAX_SCANNED_TAGS`] tags are read.
#[must_use]
pub fn referenced_image_urls(html: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let mut urls: Vec<String> = Vec::new();
    let mut at = 0;
    let mut tags = 0;
    while let Some(found) = lower[at..].find('<') {
        let start = at + found;
        if lower[start..].starts_with("<!--") {
            at = lower[start + 4..]
                .find("-->")
                .map_or(lower.len(), |end| start + 4 + end + 3);
            continue;
        }
        at = start + 1;
        let is_img = lower[start + 1..].starts_with("img")
            && lower[start + 4..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_whitespace() || c == '/' || c == '>');
        if !is_img {
            continue;
        }
        tags += 1;
        if tags > MAX_SCANNED_TAGS {
            break;
        }
        let Some((src, end)) = read_src(html, start + 4) else {
            continue;
        };
        at = end;
        let src = decode_entities(src.trim());
        if !src.is_empty() && !urls.contains(&src) {
            urls.push(src);
        }
    }
    urls
}

/// Reads the attributes of a tag from `from` and returns its `src` value and
/// the index of the tag's end.
fn read_src(html: &str, from: usize) -> Option<(&str, usize)> {
    let bytes = html.as_bytes();
    let mut at = from;
    let mut src = None;
    while at < bytes.len() {
        match bytes[at] {
            b'>' => return src.map(|value| (value, at + 1)),
            b if b.is_ascii_whitespace() || b == b'/' => at += 1,
            _ => {
                let name_start = at;
                while at < bytes.len()
                    && !matches!(bytes[at], b'=' | b'>' | b'/')
                    && !bytes[at].is_ascii_whitespace()
                {
                    at += 1;
                }
                let name = &html[name_start..at];
                while at < bytes.len() && bytes[at].is_ascii_whitespace() {
                    at += 1;
                }
                let mut value = "";
                if bytes.get(at) == Some(&b'=') {
                    at += 1;
                    while at < bytes.len() && bytes[at].is_ascii_whitespace() {
                        at += 1;
                    }
                    match bytes.get(at) {
                        Some(&quote @ (b'"' | b'\'')) => {
                            let close = html[at + 1..].find(quote as char)?;
                            value = &html[at + 1..at + 1 + close];
                            at += close + 2;
                        }
                        _ => {
                            let begin = at;
                            while at < bytes.len()
                                && !bytes[at].is_ascii_whitespace()
                                && bytes[at] != b'>'
                            {
                                at += 1;
                            }
                            value = &html[begin..at];
                        }
                    }
                }
                if name.eq_ignore_ascii_case("src") && src.is_none() {
                    src = Some(value);
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Image bytes
// ---------------------------------------------------------------------------

/// The raster type the bytes really are, or `None`. SVG and everything else
/// is refused: only the magic bytes count.
#[must_use]
pub fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// True when a `Content-Type` header, if present, names a raster image.
fn declared_type_ok(content_type: Option<&str>) -> bool {
    let Some(value) = content_type else {
        return true;
    };
    let essence = value
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    matches!(
        essence.as_str(),
        "image/png" | "image/jpeg" | "image/jpg" | "image/pjpeg" | "image/gif" | "image/webp"
    )
}

fn data_url(kind: &str, bytes: &[u8]) -> String {
    format!(
        "data:{kind};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

async fn fetch_one(fetcher: &dyn ImageFetcher, raw: String) -> Option<RemoteImage> {
    let url = check_url(&raw).ok()?;
    let fetched = tokio::time::timeout(
        REMOTE_IMAGE_TIMEOUT,
        fetcher.fetch(&url, MAX_REMOTE_IMAGE_BYTES),
    )
    .await
    .ok()?
    .ok()?;
    if fetched.bytes.is_empty()
        || fetched.bytes.len() > MAX_REMOTE_IMAGE_BYTES
        || !declared_type_ok(fetched.content_type.as_deref())
    {
        return None;
    }
    let kind = sniff_image(&fetched.bytes)?;
    Some(RemoteImage {
        data_url: data_url(kind, &fetched.bytes),
        url: raw,
    })
}

/// Fetches the images the person asked for, as `data:` URLs.
///
/// `html` is the message's own HTML: a URL that it does not reference is a
/// request error. Images that cannot be fetched safely are left out of the
/// result; the frame keeps them as blocked placeholders.
///
/// # Errors
/// [`MailError::InvalidInput`] when more than [`MAX_REMOTE_IMAGES`] URLs are
/// asked for, or one is too long or not referenced by `html`.
pub async fn fetch_remote_images(
    html: &str,
    requested: &[String],
    fetcher: &dyn ImageFetcher,
) -> Result<Vec<RemoteImage>, MailError> {
    if requested.len() > MAX_REMOTE_IMAGES {
        return Err(MailError::InvalidInput);
    }
    let referenced = referenced_image_urls(html);
    let mut wanted: Vec<String> = Vec::with_capacity(requested.len());
    for url in requested {
        if url.len() > MAX_REMOTE_URL_BYTES || !referenced.contains(url) {
            return Err(MailError::InvalidInput);
        }
        if !wanted.contains(url) {
            wanted.push(url.clone());
        }
    }
    let work = async {
        let mut images = Vec::new();
        let mut total = 0usize;
        let mut results = stream::iter(wanted)
            .map(|raw| fetch_one(fetcher, raw))
            .buffered(CONCURRENCY);
        while let Some(result) = results.next().await {
            let Some(image) = result else { continue };
            // The data URL is about a third larger than the bytes: count the bytes.
            let size = image.data_url.len() / 4 * 3;
            if total + size > MAX_REMOTE_TOTAL_BYTES {
                break;
            }
            total += size;
            images.push(image);
        }
        images
    };
    Ok(tokio::time::timeout(REMOTE_TOTAL_TIMEOUT, work)
        .await
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

    #[derive(Default)]
    struct Fake {
        replies: HashMap<String, Result<FetchedImage, MailError>>,
        calls: Mutex<Vec<String>>,
        hang: bool,
    }

    impl Fake {
        fn with(url: &str, content_type: Option<&str>, bytes: &[u8]) -> Self {
            let mut fake = Self::default();
            fake.add(url, content_type, bytes);
            fake
        }
        fn add(&mut self, url: &str, content_type: Option<&str>, bytes: &[u8]) {
            self.replies.insert(
                url.to_owned(),
                Ok(FetchedImage {
                    content_type: content_type.map(str::to_owned),
                    bytes: bytes.to_vec(),
                }),
            );
        }
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl ImageFetcher for Fake {
        fn fetch<'a>(
            &'a self,
            url: &'a Url,
            _max_bytes: usize,
        ) -> futures::future::BoxFuture<'a, Result<FetchedImage, MailError>> {
            Box::pin(async move {
                self.calls.lock().unwrap().push(url.as_str().to_owned());
                if self.hang {
                    std::future::pending::<()>().await;
                }
                self.replies
                    .get(url.as_str())
                    .cloned()
                    .unwrap_or(Err(MailError::NotFound))
            })
        }
    }

    fn html_for(urls: &[&str]) -> String {
        urls.iter()
            .map(|url| format!("<img src=\"{url}\">"))
            .collect()
    }

    async fn run(fake: &Fake, urls: &[&str]) -> Result<Vec<RemoteImage>, MailError> {
        let requested: Vec<String> = urls.iter().map(|url| (*url).to_owned()).collect();
        fetch_remote_images(&html_for(urls), &requested, fake).await
    }

    #[tokio::test]
    async fn a_referenced_https_png_comes_back_as_a_data_url() {
        let fake = Fake::with("https://img.example.com/a.png", Some("image/png"), PNG);
        let images = run(&fake, &["https://img.example.com/a.png"])
            .await
            .unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].url, "https://img.example.com/a.png");
        assert!(images[0].data_url.starts_with("data:image/png;base64,"));
    }

    #[tokio::test]
    async fn the_type_comes_from_the_bytes_not_the_header() {
        let fake = Fake::with("https://img.example.com/a", None, b"\xff\xd8\xff\xe0jpeg");
        let images = run(&fake, &["https://img.example.com/a"]).await.unwrap();
        assert!(images[0].data_url.starts_with("data:image/jpeg;base64,"));
    }

    #[tokio::test]
    async fn a_url_the_message_does_not_reference_is_refused_without_a_fetch() {
        let fake = Fake::with("https://evil.example.com/a.png", None, PNG);
        let requested = vec!["https://evil.example.com/a.png".to_owned()];
        let html = html_for(&["https://img.example.com/other.png"]);
        assert_eq!(
            fetch_remote_images(&html, &requested, &fake).await,
            Err(MailError::InvalidInput)
        );
        assert!(fake.calls().is_empty());
    }

    #[tokio::test]
    async fn a_link_or_script_url_is_not_an_image_reference() {
        let html = "<a href=\"https://a.example.com/x.png\">x</a><script>\"<img src='https://b.example.com/y.png'>\"</script><!-- <img src=\"https://c.example.com/z.png\"> -->";
        let urls = referenced_image_urls(html);
        assert!(!urls.contains(&"https://a.example.com/x.png".to_owned()));
        assert!(!urls.contains(&"https://c.example.com/z.png".to_owned()));
    }

    #[tokio::test]
    async fn too_many_urls_are_refused() {
        let fake = Fake::default();
        let owned: Vec<String> = (0..=MAX_REMOTE_IMAGES)
            .map(|n| format!("https://img.example.com/{n}.png"))
            .collect();
        let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
        assert_eq!(run(&fake, &refs).await, Err(MailError::InvalidInput));
        assert!(fake.calls().is_empty());
    }

    #[tokio::test]
    async fn an_overlong_url_is_refused() {
        let fake = Fake::default();
        let long = format!(
            "https://img.example.com/{}",
            "a".repeat(MAX_REMOTE_URL_BYTES)
        );
        assert_eq!(
            run(&fake, &[long.as_str()]).await,
            Err(MailError::InvalidInput)
        );
        assert!(fake.calls().is_empty());
    }

    #[tokio::test]
    async fn unsafe_urls_are_left_out_and_never_fetched() {
        let bad = [
            "http://img.example.com/a.png",
            "ftp://img.example.com/a.png",
            "https://user:pw@img.example.com/a.png",
            "https://img.example.com:8443/a.png",
            "https://localhost/a.png",
            "https://app.localhost/a.png",
            "https://intranet/a.png",
            "https://printer.local/a.png",
            "https://127.0.0.1/a.png",
            "https://2130706433/a.png",
            "https://0x7f.1/a.png",
            "https://10.0.0.5/a.png",
            "https://172.16.4.4/a.png",
            "https://192.168.1.1/a.png",
            "https://169.254.169.254/latest/meta-data",
            "https://100.64.0.1/a.png",
            "https://0.0.0.0/a.png",
            "https://[::1]/a.png",
            "https://[fe80::1]/a.png",
            "https://[fc00::1]/a.png",
            "https://[::ffff:127.0.0.1]/a.png",
            "https://[::ffff:10.1.2.3]/a.png",
            "https://[64:ff9b::7f00:1]/a.png",
            "https://[2002:7f00:1::1]/a.png",
        ];
        let mut fake = Fake::default();
        for url in bad {
            fake.add(url, Some("image/png"), PNG);
        }
        let images = run(&fake, &bad).await.unwrap();
        assert!(images.is_empty(), "{images:?}");
        assert!(fake.calls().is_empty(), "{:?}", fake.calls());
    }

    #[tokio::test]
    async fn public_addresses_pass_the_policy() {
        for url in [
            "https://93.184.216.34/a.png",
            "https://[2606:2800:220:1:248:1893:25c8:1946]/a.png",
            "https://cdn.example.co.uk/a.png?x=1",
        ] {
            assert!(check_url(url).is_ok(), "{url}");
        }
    }

    #[tokio::test]
    async fn non_image_bodies_are_left_out() {
        let mut fake = Fake::default();
        fake.add(
            "https://a.example.com/html",
            Some("image/png"),
            b"<html>not an image</html>",
        );
        fake.add(
            "https://a.example.com/svg",
            Some("image/svg+xml"),
            b"<svg onload=alert(1)/>",
        );
        fake.add(
            "https://a.example.com/svg2",
            Some("image/png"),
            b"<svg xmlns='x'/>",
        );
        fake.add("https://a.example.com/type", Some("text/html"), PNG);
        fake.add("https://a.example.com/empty", Some("image/png"), b"");
        let urls = [
            "https://a.example.com/html",
            "https://a.example.com/svg",
            "https://a.example.com/svg2",
            "https://a.example.com/type",
            "https://a.example.com/empty",
        ];
        assert!(run(&fake, &urls).await.unwrap().is_empty());
        assert_eq!(fake.calls().len(), urls.len());
    }

    #[tokio::test]
    async fn an_image_over_the_per_image_cap_is_left_out() {
        let mut big = PNG.to_vec();
        big.resize(MAX_REMOTE_IMAGE_BYTES + 1, 0);
        let fake = Fake::with("https://a.example.com/big.png", Some("image/png"), &big);
        assert!(run(&fake, &["https://a.example.com/big.png"])
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn a_failing_fetch_leaves_only_that_image_out() {
        let fake = Fake::with("https://a.example.com/ok.png", Some("image/png"), PNG);
        let images = run(
            &fake,
            &[
                "https://a.example.com/missing.png",
                "https://a.example.com/ok.png",
            ],
        )
        .await
        .unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].url, "https://a.example.com/ok.png");
    }

    #[tokio::test]
    async fn the_total_size_is_capped() {
        let mut image = PNG.to_vec();
        image.resize(MAX_REMOTE_IMAGE_BYTES - 16, 7);
        let mut fake = Fake::default();
        let owned: Vec<String> = (0..10)
            .map(|n| format!("https://a.example.com/{n}.png"))
            .collect();
        for url in &owned {
            fake.add(url, Some("image/png"), &image);
        }
        let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
        let images = run(&fake, &refs).await.unwrap();
        assert!(!images.is_empty());
        let total: usize = images.iter().map(|i| i.data_url.len() / 4 * 3).sum();
        assert!(total <= MAX_REMOTE_TOTAL_BYTES);
        assert!(images.len() < 10);
    }

    #[tokio::test]
    async fn duplicates_are_fetched_once() {
        let fake = Fake::with("https://a.example.com/a.png", Some("image/png"), PNG);
        let requested = vec![
            "https://a.example.com/a.png".to_owned(),
            "https://a.example.com/a.png".to_owned(),
        ];
        let html = html_for(&["https://a.example.com/a.png"]);
        let images = fetch_remote_images(&html, &requested, &fake).await.unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(fake.calls().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_hanging_server_times_out_per_image_and_overall() {
        let fake = Fake {
            hang: true,
            ..Fake::default()
        };
        let owned: Vec<String> = (0..MAX_REMOTE_IMAGES)
            .map(|n| format!("https://a.example.com/{n}.png"))
            .collect();
        let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
        let started = tokio::time::Instant::now();
        let images = run(&fake, &refs).await.unwrap();
        assert!(images.is_empty());
        assert!(started.elapsed() <= REMOTE_TOTAL_TIMEOUT + Duration::from_secs(1));
    }

    #[test]
    fn redirects_are_counted_and_rechecked() {
        let public = Url::parse("https://cdn.example.com/a.png").unwrap();
        assert!(redirect_allowed(0, &public));
        assert!(redirect_allowed(MAX_REMOTE_REDIRECTS - 1, &public));
        assert!(!redirect_allowed(MAX_REMOTE_REDIRECTS, &public));
        for next in [
            "http://cdn.example.com/a.png",
            "https://127.0.0.1/a.png",
            "https://[::1]/a.png",
            "https://192.168.0.1/a.png",
            "https://localhost/a.png",
            "https://u:p@cdn.example.com/a.png",
        ] {
            assert!(!redirect_allowed(0, &Url::parse(next).unwrap()), "{next}");
        }
    }

    #[test]
    fn references_are_found_decoded_and_deduplicated() {
        let html = "<IMG SRC='https://a.example.com/x?a=1&amp;b=2'><img alt=x src=https://b.example.com/y.png>\
                    <img src=\"https://a.example.com/x?a=1&amp;b=2\"><imgx src=\"https://no.example.com/\">\
                    <img src=\"https://c.example.com/&#x7a;.png\"/>";
        assert_eq!(
            referenced_image_urls(html),
            vec![
                "https://a.example.com/x?a=1&b=2",
                "https://b.example.com/y.png",
                "https://c.example.com/z.png",
            ]
        );
    }

    #[test]
    fn sniffing_accepts_only_raster_magic() {
        assert_eq!(sniff_image(PNG), Some("image/png"));
        assert_eq!(sniff_image(b"GIF89a.."), Some("image/gif"));
        assert_eq!(sniff_image(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff_image(b"RIFF\0\0\0\0WAVEfmt "), None);
        assert_eq!(sniff_image(b"<svg"), None);
        assert_eq!(sniff_image(b""), None);
    }
}
