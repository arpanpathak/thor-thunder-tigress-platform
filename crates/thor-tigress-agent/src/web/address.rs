//! Where a page may be fetched from.
//!
//! Only `https` addresses on port 443 are accepted, and only when every address
//! the host resolves to is public. The ranges below keep the fetch tool from
//! reaching the Thor itself, the home network, the tailnet or a cloud metadata
//! service: three chapters' worth of rules, in one table.
//!
//! ```text
//! https://example.com/a?b  ──parse──►  Url { host, path }
//!                                          │
//!                                to_socket_addrs
//!                                          ▼
//!                             each address public?
//!                              yes ───────────► the one to connect to
//!                              no  ───────────► refused, naming the range
//! ```

use std::{
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs},
};

use crate::error::{AgentError, Outcome};

/// The port every page is fetched on.
const PORT: u16 = 443;

/// A page address: `https`, port 443, no user name, no password, no fragment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    host: String,
    path: String,
}

impl Url {
    /// Parses `raw`, accepting `http://` as `https://` and dropping any
    /// fragment.
    ///
    /// # Errors
    ///
    /// [`AgentError::Refused`] when the text is not an `https` address with a
    /// host, carries a user name or password, or names a port other than 443.
    pub fn parse(raw: &str) -> Outcome<Self> {
        let trimmed = raw.trim();
        let without_fragment = trimmed.split('#').next().unwrap_or(trimmed);
        let rest = without_fragment
            .strip_prefix("https://")
            .or_else(|| without_fragment.strip_prefix("http://"))
            .ok_or_else(|| refused(trimmed, "only https addresses may be read"))?;
        let (authority, path) = split_auth_path(rest);

        if authority.is_empty() || authority.contains('@') {
            return Err(refused(trimmed, "the address has no host"));
        }

        let (host, port) = split_host_port(authority)?;

        if port != PORT {
            return Err(refused(trimmed, "only port 443 is read"));
        }

        if host.is_empty() || host.contains(['/', ' ', '\t', '\\']) {
            return Err(refused(trimmed, "the host name is not valid"));
        }

        Ok(Url {
            host,
            path: as_path(path),
        })
    }

    /// The host name, without brackets for an IPv6 literal.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The path and query, starting with `/`.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The address as text, with brackets around an IPv6 literal.
    #[must_use]
    pub fn as_string(&self) -> String {
        if self.host.contains(':') {
            format!("https://[{}]{}", self.host, self.path())
        } else {
            format!("https://{}{}", self.host, self.path())
        }
    }

    /// Whether `other` is the same site: the same host, or one that ends with
    /// the other's name.
    #[must_use]
    pub fn is_same_site(&self, other: &Self) -> bool {
        same_site(&self.host, &other.host)
    }

    /// The address `raw` names, resolved against this one when it is relative:
    /// a link on a page, or a redirect's `Location`.
    #[must_use]
    pub fn absolute(&self, raw: &str) -> Option<Url> {
        let raw = raw.trim();

        if raw.is_empty() {
            return None;
        }

        let lowered = raw.to_ascii_lowercase();

        let candidate = if lowered.starts_with("https://") || lowered.starts_with("http://") {
            raw.to_string()
        } else if let Some(rest) = raw.strip_prefix("//") {
            format!("https://{rest}")
        } else if raw.starts_with('/') {
            format!("https://{}{raw}", self.host)
        } else {
            format!("https://{}{}", self.host, join_path(&self.path, raw))
        };

        Url::parse(&candidate).ok()
    }
}

/// Joins a relative path onto the directory of `path`.
fn join_path(path: &str, relative: &str) -> String {
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let directory = path.rsplit_once('/').map_or("", |(before, _)| before);
    let mut segments: Vec<&str> = directory
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();

    for part in relative.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }

    format!("/{}", segments.join("/"))
}

impl fmt::Display for Url {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.as_string())
    }
}

/// The authority and path of an address, split at the first `/` or `?`.
fn split_auth_path(rest: &str) -> (&str, &str) {
    match rest.find(['/', '?']) {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, "/"),
    }
}

/// The path as stored: always leading with `/`.
fn as_path(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

/// `host[:port]` from an authority, with brackets around an IPv6 literal.
///
/// # Errors
///
/// [`AgentError::Refused`] when the brackets, the port or the text after the
/// host is not valid.
fn split_host_port(authority: &str) -> Outcome<(String, u16)> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, after) = rest
            .split_once(']')
            .ok_or_else(|| AgentError::Refused("the address has no closing bracket".to_string()))?;

        let port = match after {
            "" => PORT,
            other => other
                .strip_prefix(':')
                .ok_or_else(|| {
                    AgentError::Refused("the address has text after the host".to_string())
                })?
                .parse()
                .map_err(|_| AgentError::Refused("the port is not a number".to_string()))?,
        };

        return Ok((host.to_string(), port));
    }

    match authority.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => Ok((
            host.to_string(),
            port.parse()
                .map_err(|_| AgentError::Refused("the port is not a number".to_string()))?,
        )),
        Some(_) => Err(AgentError::Refused(
            "an IPv6 address needs brackets".to_string(),
        )),
        None => Ok((authority.to_string(), PORT)),
    }
}

/// Whether two host names are the same site: equal, or one a subdomain of the
/// other.
#[must_use]
pub fn same_site(one: &str, other: &str) -> bool {
    one == other || one.ends_with(&format!(".{other}")) || other.ends_with(&format!(".{one}"))
}

/// A refusal that names the host and the reason.
fn refused(host: &str, why: &str) -> AgentError {
    AgentError::Refused(format!("{host}: {why}"))
}

/// Resolves `host` and returns the one address to connect to.
///
/// # Security
///
/// Rules 2 and 3 of the fetch design: every address the name resolves to must
/// be public, and the single address returned is the only one the caller may
/// connect to, so a second lookup cannot swap in another address.
///
/// # Errors
///
/// [`AgentError::Refused`] when an address is in a private, loopback,
/// link-local, shared, multicast or reserved range; [`AgentError::Fetch`] when
/// the name cannot be resolved or resolves to nothing.
pub fn checked_address(host: &str) -> Outcome<SocketAddr> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return public_socket(ip, host);
    }

    let resolved: Vec<SocketAddr> = (host, PORT)
        .to_socket_addrs()
        .map_err(|error| AgentError::Fetch(format!("{host}: {error}")))?
        .collect();

    choose(&resolved, host)
}

/// The one address to connect to, after every resolved address has been
/// checked.
///
/// # Errors
///
/// [`AgentError::Refused`] when any address is not public, and
/// [`AgentError::Fetch`] when nothing resolved.
fn choose(resolved: &[SocketAddr], host: &str) -> Outcome<SocketAddr> {
    let first = resolved
        .first()
        .ok_or_else(|| AgentError::Fetch(format!("{host}: resolved to nothing")))?;

    if let Some(private) = resolved.iter().find(|address| !is_public(address.ip())) {
        return Err(refused(host, &format!("resolves to {}", private.ip())));
    }

    Ok(*first)
}

/// The address for a literal IP, if it is public.
fn public_socket(ip: IpAddr, host: &str) -> Outcome<SocketAddr> {
    if is_public(ip) {
        Ok(SocketAddr::new(ip, PORT))
    } else {
        Err(refused(host, "is not a public address"))
    }
}

/// Whether `ip` may be contacted: public, routable, and not this machine.
#[must_use]
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => public_v4(v4),
        IpAddr::V6(v6) => public_v6(v6),
    }
}

/// Whether an IPv4 address is public.
fn public_v4(ip: Ipv4Addr) -> bool {
    const BLOCKED: [(&str, u8); 11] = [
        ("0.0.0.0", 8),
        ("10.0.0.0", 8),
        ("100.64.0.0", 10),
        ("127.0.0.0", 8),
        ("169.254.0.0", 16),
        ("172.16.0.0", 12),
        ("192.0.0.0", 24),
        ("192.168.0.0", 16),
        ("198.18.0.0", 15),
        ("224.0.0.0", 4),
        ("240.0.0.0", 4),
    ];

    !BLOCKED
        .iter()
        .any(|(base, prefix)| v4_in(ip, base, *prefix))
}

/// Whether an IPv4 address falls inside `base/prefix`.
fn v4_in(ip: Ipv4Addr, base: &str, prefix: u8) -> bool {
    let Some(base) = base.parse::<Ipv4Addr>().ok() else {
        return false;
    };

    let mask = u32::MAX.checked_shl(u32::from(32 - prefix)).unwrap_or(0);

    (u32::from(ip) & mask) == (u32::from(base) & mask)
}

/// Whether an IPv6 address is public.
fn public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return public_v4(v4);
    }

    if let Some(v4) = nat64(ip) {
        return public_v4(v4);
    }

    !(v6_in(ip, "::", 128)
        || v6_in(ip, "::1", 128)
        || v6_in(ip, "fe80::", 10)
        || v6_in(ip, "fc00::", 7)
        || v6_in(ip, "ff00::", 8))
}

/// The IPv4 address inside `64:ff9b::/96`, if this is that form.
fn nat64(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    if !v6_in(ip, "64:ff9b::", 96) {
        return None;
    }

    let octets = ip.octets();

    Some(Ipv4Addr::new(
        octets[12], octets[13], octets[14], octets[15],
    ))
}

/// Whether an IPv6 address falls inside `base/prefix`.
fn v6_in(ip: Ipv6Addr, base: &str, prefix: u8) -> bool {
    let Some(base) = base.parse::<Ipv6Addr>().ok() else {
        return false;
    };

    let mask = u128::MAX.checked_shl(u32::from(128 - prefix)).unwrap_or(0);

    (u128::from(ip) & mask) == (u128::from(base) & mask)
}

#[cfg(test)]
mod tests;
