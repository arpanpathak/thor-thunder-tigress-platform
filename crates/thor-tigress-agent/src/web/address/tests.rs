use super::*;

fn parsed(raw: &str) -> Outcome<Url> {
    Url::parse(raw)
}

#[test]
fn parses_https_http_and_bare_paths() -> Outcome {
    let page = parsed("https://example.com/a/b?c=d#top")?;

    assert_eq!(page.host(), "example.com");
    assert_eq!(page.path(), "/a/b?c=d");
    assert_eq!(page.as_string(), "https://example.com/a/b?c=d");
    assert_eq!(
        parsed("http://example.com")?.as_string(),
        "https://example.com/"
    );
    assert_eq!(
        parsed(" https://example.com?q=1 ")?,
        Url {
            host: "example.com".to_string(),
            path: "/?q=1".to_string()
        }
    );

    Ok(())
}

#[test]
fn parses_ipv6_with_brackets() -> Outcome {
    let page = parsed("https://[::1]:443/x")?;

    assert_eq!(page.host(), "::1");
    assert_eq!(page.as_string(), "https://[::1]/x");

    Ok(())
}

#[test]
fn refuses_addresses_it_cannot_read() {
    for raw in [
        "ftp://example.com",
        "example.com",
        "https://",
        "https://user@example.com/",
        "https://example.com:8443/",
        "https://example.com:notaport/",
        "https://[::1]:notaport/",
        "https://[::1/x",
        "https://::1/x",
        "https://[::1]x",
        "https://exa mple.com/",
        "https://example.com\\x",
    ] {
        assert!(parsed(raw).is_err(), "{raw}");
    }
}

#[test]
fn a_site_is_its_host_and_its_subdomains() -> Outcome {
    let site = parsed("https://example.com/")?;

    assert!(site.is_same_site(&parsed("https://example.com/docs")?));
    assert!(site.is_same_site(&parsed("https://www.example.com/")?));
    assert!(parsed("https://www.example.com/")?.is_same_site(&site));
    assert!(!site.is_same_site(&parsed("https://other.com/")?));
    assert!(!site.is_same_site(&parsed("https://notexample.com/")?));

    Ok(())
}

#[test]
fn absolute_addresses_resolve_against_a_page() -> Outcome {
    let base = parsed("https://example.com/docs/start")?;

    assert_eq!(base.absolute(""), None);
    assert_eq!(base.absolute("   "), None);
    assert_eq!(
        base.absolute("http://other.com/x")
            .map(|url| url.as_string()),
        Some("https://other.com/x".to_string())
    );
    assert_eq!(
        base.absolute("/a").map(|url| url.as_string()),
        Some("https://example.com/a".to_string())
    );
    assert_eq!(
        base.absolute("https://cdn.example.net/y")
            .map(|url| url.as_string()),
        Some("https://cdn.example.net/y".to_string())
    );
    assert!(base.absolute("//not a host/").is_none());

    Ok(())
}

/// One address from every range the fetch tool must never reach.
const BLOCKED: [&str; 21] = [
    "0.0.0.0",
    "127.0.0.1",
    "10.1.2.3",
    "172.16.5.4",
    "172.31.255.1",
    "192.168.0.1",
    "100.84.254.65",
    "169.254.169.254",
    "192.0.0.1",
    "198.18.0.1",
    "224.0.0.1",
    "240.0.0.1",
    "255.255.255.255",
    "::",
    "::1",
    "fe80::1",
    "fc00::1",
    "fd7a:115c:a1e0::1",
    "ff02::1",
    "::ffff:127.0.0.1",
    "64:ff9b::7f00:1",
];

/// Whether `raw` is an address, and a public one.
fn public(raw: &str) -> bool {
    raw.parse::<IpAddr>().is_ok_and(is_public)
}

#[test]
fn blocks_the_documented_ranges() {
    for raw in BLOCKED {
        assert!(raw.parse::<IpAddr>().is_ok(), "{raw}");
        assert!(!public(raw), "{raw}");
    }
}

#[test]
fn allows_public_addresses_up_to_the_edge_of_a_private_range() {
    assert!(public("172.15.0.1"));
    assert!(!public("172.16.0.1"));
    assert!(!public("172.31.255.1"));
    assert!(public("172.32.0.1"));
    assert!(public("8.8.8.8"));
    assert!(public("1.1.1.1"));
    assert!(public("2606:4700::1111"));
    assert!(public("64:ff9b::808:808"));
}

#[test]
fn every_resolved_address_must_be_public() -> Outcome {
    let public: SocketAddr = "1.1.1.1:443"
        .parse()
        .unwrap_or(SocketAddr::from(([1, 1, 1, 1], 443)));
    let private: SocketAddr = "127.0.0.1:443"
        .parse()
        .unwrap_or(SocketAddr::from(([127, 0, 0, 1], 443)));

    assert_eq!(choose(&[public], "public.example")?, public);
    assert!(
        choose(&[public, private], "mixed.example").is_err_and(|error| {
            error.to_string() == "refused: mixed.example: resolves to 127.0.0.1"
        })
    );
    assert!(
        choose(&[], "empty.example")
            .is_err_and(|error| error.to_string() == "fetch: empty.example: resolved to nothing")
    );

    Ok(())
}

#[test]
fn a_checked_address_is_the_only_one_returned() -> Outcome {
    let public = checked_address("1.1.1.1")?;

    assert_eq!(public.port(), 443);
    assert!(checked_address("127.0.0.1").is_err_and(|error| {
        error.to_string() == "refused: 127.0.0.1: is not a public address"
    }));
    assert!(checked_address("localhost").is_err_and(|error| {
        error
            .to_string()
            .starts_with("refused: localhost: resolves to ")
    }));
    assert!(checked_address("no-such-host.invalid").is_err());

    Ok(())
}

#[test]
fn split_helpers_cover_their_edges() {
    assert_eq!(split_auth_path("example.com"), ("example.com", "/"));
    assert_eq!(split_auth_path("example.com?q"), ("example.com", "?q"));
    assert_eq!(as_path("?q=1"), "/?q=1");
    assert!(!v4_in(Ipv4Addr::UNSPECIFIED, "not-an-ip", 8));
    assert!(!v6_in(Ipv6Addr::LOCALHOST, "not-an-ip", 8));
    assert!(nat64(Ipv6Addr::LOCALHOST).is_none());
}
