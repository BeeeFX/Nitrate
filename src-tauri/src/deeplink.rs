//! Handling `nitrate://` links fired from a browser.
//!
//! Registering a protocol opens a door that *any* web page can knock on — the
//! OS routes by scheme, not by which extension sent it, and a secret baked into
//! an extension is readable by anyone who unzips it. So nothing here trusts the
//! caller. The link is treated as hostile input and filtered on its own merits.

use std::time::{Duration, Instant};

/// The most links we'll accept in a burst, so a page can't flood the queue.
const BURST: usize = 5;
const BURST_WINDOW: Duration = Duration::from_secs(10);

pub const SCHEME: &str = "nitrate";

/// A link that survived validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incoming {
    pub url: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Rejected {
    NotOurScheme,
    Malformed,
    /// Anything that isn't plain http(s) — `file:`, `data:` and friends.
    UnsupportedScheme,
    /// Loopback, LAN and link-local addresses.
    ///
    /// Without this, a hostile page could use the downloader to probe a router's
    /// admin panel or a service listening on localhost. It would fail to find a
    /// video, but the request would still have been made.
    PrivateAddress,
    TooMany,
}

impl Rejected {
    pub fn message(&self) -> &'static str {
        match self {
            Rejected::NotOurScheme => "That link isn't for Nitrate.",
            Rejected::Malformed => "That link was malformed.",
            Rejected::UnsupportedScheme => "Only web links can be sent to Nitrate.",
            Rejected::PrivateAddress => "Links to local or private addresses are refused.",
            Rejected::TooMany => "Too many links at once — ignoring the rest.",
        }
    }
}

/// Pulls the target out of `nitrate://add?url=<encoded>`.
///
/// Also accepts `nitrate://<encoded>` so a hand-written bookmarklet doesn't
/// need to know the path.
pub fn parse(raw: &str) -> Result<Incoming, Rejected> {
    let trimmed = raw.trim().trim_end_matches('/');
    let rest = trimmed
        .strip_prefix(&format!("{SCHEME}://"))
        .or_else(|| trimmed.strip_prefix(&format!("{SCHEME}:")))
        .ok_or(Rejected::NotOurScheme)?;

    // Everything after the first `?`, or the whole thing when there's no query.
    let target = match rest.split_once('?') {
        Some((_path, query)) => query
            .split('&')
            .find_map(|pair| pair.strip_prefix("url="))
            .ok_or(Rejected::Malformed)?,
        None => rest,
    };

    let decoded = percent_decode(target);
    let url = validate(&decoded)?;
    Ok(Incoming { url })
}

/// Only plain web links, and never at an address inside the machine or network.
///
/// Returns the link as parsed, and that's what gets handed on. Checking one
/// reading of a string and fetching another is how filters like this are
/// usually got round — `http://a.com\@127.0.0.1/` is a.com to a browser and
/// 127.0.0.1 to Python's urllib — so the downloader only ever sees the
/// normalised form that was actually checked.
fn validate(raw: &str) -> Result<String, Rejected> {
    let lower = raw.to_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err(Rejected::UnsupportedScheme);
    }

    // The WHATWG parser, same as a browser's: it reads `127.1`, `2130706433`
    // and `0x7f000001` as the loopback address they resolve to, where matching
    // on dotted-quad text let every one of them through.
    let parsed = url::Url::parse(raw).map_err(|_| Rejected::Malformed)?;
    let host = parsed.host().ok_or(Rejected::Malformed)?;
    if is_private_host(&host) {
        return Err(Rejected::PrivateAddress);
    }

    Ok(parsed.into())
}

fn is_private_host(host: &url::Host<&str>) -> bool {
    match host {
        // A name rather than an address. DNS could still resolve inward, but
        // blocking every hostname would block the whole point of the feature.
        url::Host::Domain(name) => {
            let name = name.trim_end_matches('.');
            name == "localhost" || name.ends_with(".localhost") || name.ends_with(".local")
        }
        url::Host::Ipv4(ip) => is_private_v4(*ip),
        url::Host::Ipv6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                return is_private_v4(v4);
            }
            let first = ip.segments()[0];
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                // Unique-local, fc00::/7.
                || (first & 0xfe00) == 0xfc00
                // Link-local, fe80::/10.
                || (first & 0xffc0) == 0xfe80
        }
    }
}

fn is_private_v4(ip: std::net::Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || a == 0
        // Carrier-grade NAT, 100.64.0.0/10 — the ISP's side of the router.
        || (a == 100 && (b & 0xc0) == 64)
        // Multicast, reserved and broadcast.
        || a >= 224
}

/// Minimal percent-decoding — enough for a URL in a query string.
fn percent_decode(input: &str) -> String {
    let bytes = input.replace('+', " ");
    let bytes = bytes.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(byte) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }

    String::from_utf8_lossy(&out).into_owned()
}

/// Sliding window over recent arrivals, so a page firing links in a loop only
/// gets a handful through.
#[derive(Default)]
pub struct RateLimit {
    recent: Vec<Instant>,
}

impl RateLimit {
    pub fn allow(&mut self) -> bool {
        let now = Instant::now();
        self.recent
            .retain(|at| now.duration_since(*at) < BURST_WINDOW);
        if self.recent.len() >= BURST {
            return false;
        }
        self.recent.push(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_normal_link() {
        let got = parse("nitrate://add?url=https%3A%2F%2Fwww.youtube.com%2Fwatch%3Fv%3Dabc");
        assert_eq!(
            got,
            Ok(Incoming {
                url: "https://www.youtube.com/watch?v=abc".into()
            })
        );
    }

    #[test]
    fn accepts_the_short_form() {
        let got = parse("nitrate://https%3A%2F%2Fclips.twitch.tv%2Fsomething");
        assert_eq!(
            got,
            Ok(Incoming {
                url: "https://clips.twitch.tv/something".into()
            })
        );
    }

    #[test]
    fn refuses_non_web_schemes() {
        for raw in [
            "nitrate://add?url=file%3A%2F%2F%2FC%3A%2FWindows%2Fsystem.ini",
            "nitrate://add?url=data%3Atext%2Fhtml%2Chello",
            "nitrate://add?url=javascript%3Aalert(1)",
        ] {
            assert_eq!(parse(raw), Err(Rejected::UnsupportedScheme), "{raw}");
        }
    }

    #[test]
    fn refuses_addresses_inside_the_network() {
        // A hostile page shouldn't be able to make the downloader knock on the
        // router or on something listening locally.
        for host in [
            "http://127.0.0.1:8080/admin",
            "http://localhost/",
            "http://192.168.1.1/",
            "http://10.0.0.5/",
            "http://172.16.4.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://[::1]:9000/",
            "http://printer.local/",
        ] {
            let raw = format!("nitrate://add?url={}", urlencode(host));
            assert_eq!(parse(&raw), Err(Rejected::PrivateAddress), "{host}");
        }
    }

    #[test]
    fn refuses_other_spellings_of_the_same_addresses() {
        // Every one of these resolves to loopback or the LAN, and every one
        // got past a check that only recognised dotted-quad text.
        for host in [
            "http://127.1/",
            "http://2130706433/",
            "http://0x7f000001/",
            "http://0177.0.0.1/",
            "http://localhost./",
            "http://[0:0:0:0:0:0:0:1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://[::ffff:192.168.0.1]/",
            "http://[::]/",
            "http://[fe90::1]/",
            "http://[fd12:3456::1]/",
            "http://100.64.0.1/",
            "http://0.0.0.0/",
        ] {
            let raw = format!("nitrate://add?url={}", urlencode(host));
            assert_eq!(parse(&raw), Err(Rejected::PrivateAddress), "{host}");
        }
    }

    #[test]
    fn hands_on_the_address_that_was_checked() {
        // urllib would read the host here as 127.0.0.1. The parser reads it as
        // a.com, so what goes on must be its reading, not the raw string.
        let raw = format!(
            "nitrate://add?url={}",
            urlencode("http://a.com\\@127.0.0.1/")
        );
        let got = parse(&raw).expect("a.com is public");
        assert_eq!(url::Url::parse(&got.url).unwrap().host_str(), Some("a.com"));
        assert!(!got.url.contains('\\'), "{}", got.url);
    }

    #[test]
    fn still_allows_ordinary_public_addresses() {
        for host in [
            "https://youtu.be/x",
            "https://8.8.8.8/video.mp4",
            // Names that merely start like an IPv6 prefix are still names.
            "https://fcbarcelona.com/video",
            "https://fdrlive.com/",
            "https://fe80.example/",
        ] {
            let raw = format!("nitrate://add?url={}", urlencode(host));
            assert!(parse(&raw).is_ok(), "{host}");
        }
    }

    #[test]
    fn ignores_links_that_are_not_ours() {
        assert_eq!(parse("https://example.com"), Err(Rejected::NotOurScheme));
    }

    #[test]
    fn rate_limit_stops_a_flood() {
        let mut limit = RateLimit::default();
        for _ in 0..BURST {
            assert!(limit.allow());
        }
        assert!(!limit.allow(), "a burst beyond the cap should be refused");
    }

    fn urlencode(s: &str) -> String {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    (b as char).to_string()
                }
                _ => format!("%{b:02X}"),
            })
            .collect()
    }
}
