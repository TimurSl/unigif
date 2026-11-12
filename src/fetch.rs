//! Quick add by URL, with SSRF protection.
//!
//! * the hostname is resolved by our own resolver, which drops every private,
//!   loopback, link-local and otherwise reserved address, so the connection can
//!   only go to a public one (no DNS-rebinding window: the address that was
//!   checked is the address that is connected to);
//! * literal IP hosts never reach the resolver, so they are checked up front;
//! * redirects are followed by hand (at most 3) and every hop is re-validated;
//! * no proxy, no credentials in the URL, no cookies, http(s) only;
//! * the size cap applies while streaming, and the whole fetch has a deadline.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::{redirect, Client, Url};

use crate::app::App;
use crate::error::ApiError;
use crate::ingest::{self, Upload};

const MAX_REDIRECTS: usize = 3;

/// Is this a globally routable address?
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_multicast()
        || o[0] == 0
        || (o[0] == 100 && (64..128).contains(&o[1])) // CGNAT 100.64/10
        || (o[0] == 192 && o[1] == 0 && o[2] == 0) // IETF protocol assignments
        || (o[0] == 198 && (18..20).contains(&o[1])) // benchmarking 198.18/15
        || o[0] >= 240) // reserved
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    let s = ip.segments();
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return false;
    }
    if (s[0] & 0xfe00) == 0xfc00 || (s[0] & 0xffc0) == 0xfe80 || (s[0] & 0xffc0) == 0xfec0 {
        return false; // unique-local, link-local, site-local
    }
    if s[0] == 0x2001 && (s[1] == 0x0db8 || s[1] == 0) {
        return false; // documentation, Teredo
    }
    // NAT64 and 6to4 embed an IPv4 address: judge that instead.
    if s[0] == 0x0064 && s[1] == 0xff9b && s[2..6].iter().all(|x| *x == 0) {
        return is_public_v4(Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8));
    }
    if s[0] == 0x2002 {
        return is_public_v4(Ipv4Addr::new((s[1] >> 8) as u8, s[1] as u8, (s[2] >> 8) as u8, s[2] as u8));
    }
    true
}

/// Resolves names but only ever returns public addresses.
struct PublicOnly {
    allow_private: bool,
}

impl Resolve for PublicOnly {
    fn resolve(&self, name: Name) -> Resolving {
        let allow = self.allow_private;
        Box::pin(async move {
            let found: Vec<SocketAddr> = tokio::net::lookup_host((name.as_str(), 0)).await?.collect();
            let ok: Vec<SocketAddr> = found.into_iter().filter(|a| allow || is_public_ip(a.ip())).collect();
            if ok.is_empty() {
                return Err("host resolves only to private or reserved addresses".into());
            }
            Ok(Box::new(ok.into_iter()) as Addrs)
        })
    }
}

/// Check a URL before connecting to it (initial URL and every redirect hop).
pub fn validate_url(url: &Url, allow_private: bool) -> Result<(), ApiError> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(ApiError::BadRequest("only http and https URLs can be fetched".into()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ApiError::BadRequest("URLs with embedded credentials are not allowed".into()));
    }
    let host = url.host().ok_or_else(|| ApiError::BadRequest("URL has no host".into()))?;
    if !allow_private {
        let literal = match host {
            url::Host::Ipv4(ip) => Some(IpAddr::V4(ip)),
            url::Host::Ipv6(ip) => Some(IpAddr::V6(ip)),
            url::Host::Domain(_) => None,
        };
        if literal.is_some_and(|ip| !is_public_ip(ip)) {
            return Err(ApiError::BadRequest("URL points to a private or reserved address".into()));
        }
    }
    Ok(())
}

/// Download `raw_url` into `tmp/` as an [`Upload`].
pub async fn fetch(app: &App, raw_url: &str) -> Result<Upload, ApiError> {
    let deadline = Duration::from_secs(app.cfg.fetch.timeout_secs);
    tokio::time::timeout(deadline, fetch_inner(app, raw_url))
        .await
        .map_err(|_| ApiError::FetchFailed("timed out fetching the URL".into()))?
}

async fn fetch_inner(app: &App, raw_url: &str) -> Result<Upload, ApiError> {
    let allow_private = app.cfg.fetch.allow_private;
    let mut url = Url::parse(raw_url).map_err(|e| ApiError::BadRequest(format!("invalid url: {e}")))?;
    let client = Client::builder()
        .no_proxy()
        .redirect(redirect::Policy::none())
        .dns_resolver(Arc::new(PublicOnly { allow_private }))
        .connect_timeout(Duration::from_secs(10))
        .user_agent("unigif")
        .build()
        .map_err(anyhow::Error::from)?;

    let limit = app.cfg.upload.max_size.0;
    for _ in 0..=MAX_REDIRECTS {
        validate_url(&url, allow_private)?;
        let resp = client.get(url.clone()).send().await.map_err(|e| {
            tracing::debug!(error = ?e, "url fetch failed");
            ApiError::FetchFailed("could not connect to the URL".into())
        })?;
        let status = resp.status();
        if status.is_redirection() {
            let loc = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| ApiError::FetchFailed("redirect without a Location header".into()))?;
            url = url.join(loc).map_err(|_| ApiError::FetchFailed("invalid redirect target".into()))?;
            continue;
        }
        if !status.is_success() {
            return Err(ApiError::FetchFailed(format!("the URL answered {status}")));
        }
        if resp.content_length().is_some_and(|n| n > limit) {
            return Err(ApiError::TooLarge { limit });
        }
        let stream = resp.bytes_stream().map(|r| r.map_err(|e| std::io::Error::other(e.to_string())));
        return ingest::receive_labeled(app, Box::pin(stream), "fetched file").await;
    }
    Err(ApiError::FetchFailed(format!("more than {MAX_REDIRECTS} redirects")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn public(s: &str) -> bool {
        is_public_ip(s.parse().unwrap())
    }

    #[test]
    fn classifies_addresses() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "0.0.0.0",
            "100.64.0.1",
            "255.255.255.255",
            "224.0.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "240.0.0.1",
            "::1",
            "::",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "ff02::1",
            "2001:db8::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "64:ff9b::7f00:1",
            "2002:7f00:1::",
        ] {
            assert!(!public(ip), "{ip} should be refused");
        }
        for ip in [
            "8.8.8.8",
            "1.1.1.1",
            "93.184.216.34",
            "172.32.0.1",
            "2606:4700:4700::1111",
            "::ffff:8.8.8.8",
            "64:ff9b::808:808",
        ] {
            assert!(public(ip), "{ip} should be allowed");
        }
    }

    #[test]
    fn validates_urls() {
        let v = |s: &str| validate_url(&Url::parse(s).unwrap(), false);
        assert!(v("https://example.com/a.gif").is_ok());
        assert!(v("http://8.8.8.8/a.gif").is_ok());
        assert!(v("http://127.0.0.1/a.gif").is_err());
        assert!(v("http://[::1]/a.gif").is_err());
        assert!(v("http://169.254.169.254/latest/meta-data").is_err());
        assert!(v("http://0x7f.1/").is_err()); // normalised to 127.0.0.1 by the URL parser
        assert!(v("ftp://example.com/a.gif").is_err());
        assert!(v("file:///etc/passwd").is_err());
        assert!(v("https://user:pw@example.com/").is_err());
        assert!(validate_url(&Url::parse("http://127.0.0.1/").unwrap(), true).is_ok());
    }
}
