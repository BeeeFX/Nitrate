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
    validate(&decoded)?;
    Ok(Incoming { url: decoded })
}

/// Only plain web links, and never at an address inside the machine or network.
fn validate(url: &str) -> Result<(), Rejected> {
    let lower = url.to_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err(Rejected::UnsupportedScheme);
    }

    let authority = lower
        .split("://")
        .nth(1)
        .and_then(|rest| rest.split(['/', '?', '#']).next())
        .map(|h| h.split('@').next_back().unwrap_or(h))
        .unwrap_or("");

    // IPv6 authorities are bracketed, so the port can't be found by splitting
    // on the first colon — that would cut `[::1]:9000` to `[`.
    let host = match authority.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or(""),
        None => authority.split(':').next().unwrap_or(authority),
    }
    .to_string();

    if host.is_empty() {
        return Err(Rejected::Malformed);
    }
    if is_private_host(&host) {
        return Err(Rejected::PrivateAddress);
    }

    Ok(())
}

fn is_private_host(host: &str) -> bool {
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
        return true;
    }

    // IPv6 loopback and unique-local.
    if host == "::1" || host.starts_with("fc") || host.starts_with("fd") || host.starts_with("fe80")
    {
        return true;
    }

    let octets: Vec<u8> = host
        .split('.')
        .filter_map(|part| part.parse::<u8>().ok())
        .collect();
    if octets.len() != 4 || host.split('.').count() != 4 {
        // A name rather than a bare IP; DNS could still resolve inward, but
        // blocking every hostname would block the whole point of the feature.
        return false;
    }

    matches!(octets[0], 0 | 10 | 127)
        || (octets[0] == 192 && octets[1] == 168)
        || (octets[0] == 172 && (16..=31).contains(&octets[1]))
        || (octets[0] == 169 && octets[1] == 254)
        || octets[0] >= 224
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
    fn still_allows_ordinary_public_addresses() {
        for host in ["https://youtu.be/x", "https://8.8.8.8/video.mp4"] {
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
