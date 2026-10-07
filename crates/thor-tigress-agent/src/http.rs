//! Downloading one `https` page, from the address the checks chose, with
//! nothing of the user's sent along and hard limits on time, size and type.
//!
//! ```text
//! Url ──► PublicDns ──► one checked SocketAddr ──► Pinned resolver
//!                                                      │
//!                                          ureq GET, no cookies, no
//!                                          referrer, no Authorization
//!                                                      ▼
//!                                   status, type, Location, 2 MB of body
//! ```
//!
//! Redirects are not followed here; [`crate::fetch`] follows each `Location` by
//! hand so every hop goes through the same checks.

use std::{fmt, net::SocketAddr, time::Duration};

use ureq::{
    Agent,
    config::Config,
    http::Uri,
    tls::TlsConfig,
    unversioned::{
        resolver::{ResolvedSocketAddrs, Resolver},
        transport::{DefaultConnector, NextTimeout},
    },
};

use crate::{
    address::{self, Url},
    error::{AgentError, Outcome},
};

/// The most bytes downloaded from one page.
pub const MAX_BYTES: u64 = 2 * 1024 * 1024;

/// The longest one page may take, connection and body together.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// The user agent sent with every request; it names the project, not the user.
pub const USER_AGENT: &str =
    "thor-tigress-cub/0.1 (+https://github.com/arpanpathak/thor-thunder-tigress-platform)";

/// One page as it came back, before it is parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    /// The HTTP status, such as 200 or 302.
    pub status: u16,
    /// The `Content-Type` header, or an empty string when there is none.
    pub content_type: String,
    /// The `Location` header of a redirect, when there is one.
    pub location: Option<String>,
    /// The body, as lossy UTF-8.
    pub body: String,
}

/// Where a page's bytes come from. The real implementation speaks HTTPS; tests
/// provide their own.
pub trait Web: fmt::Debug + Send + Sync {
    /// Downloads `url`.
    ///
    /// # Errors
    ///
    /// [`AgentError::Refused`] when the address is not public;
    /// [`AgentError::Fetch`] when the connection, TLS handshake or read fails.
    fn get(&self, url: &Url) -> Outcome<Fetched>;
}

/// Turning a host name into the one address to connect to.
pub trait Addresses: fmt::Debug + Send + Sync {
    /// The checked address for `host`.
    ///
    /// # Errors
    ///
    /// [`AgentError::Refused`] for a non-public address, and
    /// [`AgentError::Fetch`] when the name cannot be resolved.
    fn checked(&self, host: &str) -> Outcome<SocketAddr>;
}

/// The real resolver: the system's DNS, then the public-address rule.
#[derive(Debug, Default, Clone, Copy)]
pub struct PublicDns;

impl Addresses for PublicDns {
    fn checked(&self, host: &str) -> Outcome<SocketAddr> {
        address::checked_address(host)
    }
}

/// Fetching over HTTPS with ureq.
#[derive(Debug)]
pub struct HttpWeb {
    addresses: Box<dyn Addresses>,
    tls: TlsConfig,
}

impl Default for HttpWeb {
    fn default() -> Self {
        HttpWeb::new()
    }
}

impl HttpWeb {
    /// A client that resolves names itself and trusts the system's roots.
    #[must_use]
    pub fn new() -> Self {
        HttpWeb {
            addresses: Box::new(PublicDns),
            tls: TlsConfig::default(),
        }
    }

    /// A client that resolves through `addresses` and trusts `tls`.
    #[cfg(test)]
    #[must_use]
    pub fn with_addresses(addresses: Box<dyn Addresses>, tls: TlsConfig) -> Self {
        HttpWeb { addresses, tls }
    }

    fn agent(&self, url: &Url) -> Outcome<Agent> {
        let address = self.addresses.checked(url.host())?;
        let config: Config = Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .max_redirects(0)
            .http_status_as_error(false)
            .user_agent(USER_AGENT)
            .https_only(true)
            .tls_config(self.tls.clone())
            .build();
        Ok(Agent::with_parts(
            config,
            DefaultConnector::default(),
            Pinned { address },
        ))
    }
}

impl Web for HttpWeb {
    fn get(&self, url: &Url) -> Outcome<Fetched> {
        let agent = self.agent(url)?;
        let mut response = agent
            .get(url.as_string())
            .call()
            .map_err(|error| AgentError::Fetch(format!("{url}: {error}")))?;
        let status = response.status().as_u16();
        let content_type = header(&response, "content-type").unwrap_or_default();
        let location = header(&response, "location");
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_BYTES)
            .lossy_utf8(true)
            .read_to_string()
            .map_err(|error| AgentError::Fetch(format!("{url}: {error}")))?;
        Ok(Fetched {
            status,
            content_type,
            location,
            body,
        })
    }
}

/// A header value as text, lowercased name; `None` when it is absent.
fn header(response: &ureq::http::Response<ureq::Body>, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

/// A resolver that returns the one address already checked for the request.
#[derive(Debug)]
struct Pinned {
    address: SocketAddr,
}

impl Resolver for Pinned {
    fn resolve(
        &self,
        _uri: &Uri,
        _config: &Config,
        _timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let mut addresses = ResolvedSocketAddrs::from_fn(|_| self.address);
        addresses.push(self.address);
        Ok(addresses)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        sync::{Arc, mpsc},
        thread,
    };

    use rustls::{
        ServerConfig,
        pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    };

    use super::*;

    #[derive(Debug)]
    struct Local {
        address: SocketAddr,
    }

    impl Addresses for Local {
        fn checked(&self, _host: &str) -> Outcome<SocketAddr> {
            Ok(self.address)
        }
    }

    fn web(address: SocketAddr) -> HttpWeb {
        let tls = TlsConfig::builder().disable_verification(true).build();
        HttpWeb::with_addresses(Box::new(Local { address }), tls)
    }

    fn https_server(response: &str) -> Outcome<(SocketAddr, mpsc::Receiver<Outcome<String>>)> {
        let certified = rcgen::generate_simple_self_signed(["localhost".to_string()])
            .map_err(|error| AgentError::Fetch(format!("cert: {error}")))?;
        let certificate = CertificateDer::from(certified.cert.der().to_vec());
        let key =
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(certified.key_pair.serialize_der()));
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![certificate], key)
            .map_err(|error| AgentError::Fetch(format!("tls: {error}")))?;
        let config = Arc::new(config);
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let (sender, received) = mpsc::channel();
        let owned = response.to_string();
        thread::spawn(move || {
            let _ = sender.send(serve_once(&listener, &config, &owned));
        });
        Ok((address, received))
    }

    fn serve_once(
        listener: &TcpListener,
        config: &Arc<ServerConfig>,
        response: &str,
    ) -> Outcome<String> {
        let (stream, _) = listener.accept()?;
        let connection = rustls::ServerConnection::new(Arc::clone(config))
            .map_err(|error| AgentError::Fetch(format!("tls: {error}")))?;
        let mut tls = rustls::StreamOwned::new(connection, stream);
        let request = read_request(&mut tls)?;
        tls.write_all(response.as_bytes())?;
        tls.flush()?;
        Ok(request)
    }

    fn read_request(stream: &mut impl Read) -> Outcome<String> {
        let mut reader = BufReader::new(stream);
        let mut raw = String::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line)?;
            if line.trim().is_empty() {
                break;
            }
            raw.push_str(&line);
        }
        Ok(raw)
    }

    fn page(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn url() -> Outcome<Url> {
        Url::parse("https://localhost/page?q=1")
    }

    #[test]
    fn downloads_a_page_over_tls_and_sends_none_of_the_user() -> Outcome {
        let (address, request) = https_server(&page("<p>hi</p>"))?;
        let fetched = web(address).get(&url()?)?;
        let seen = request.recv().ok().and_then(Result::ok).unwrap_or_default();
        assert_eq!(fetched.status, 200);
        assert_eq!(fetched.content_type, "text/html");
        assert_eq!(fetched.location, None);
        assert_eq!(fetched.body, "<p>hi</p>");
        assert!(seen.starts_with("GET /page?q=1 HTTP/1.1\r\n"), "{seen}");
        assert!(
            seen.to_ascii_lowercase().contains("host: localhost"),
            "{seen}"
        );
        assert!(seen.contains(USER_AGENT), "{seen}");
        for absent in ["cookie", "authorization", "referer"] {
            assert!(
                !seen.to_ascii_lowercase().contains(absent),
                "{absent} in {seen}"
            );
        }
        Ok(())
    }

    #[test]
    fn reports_the_status_of_an_error_page() -> Outcome {
        let response = "HTTP/1.1 500 Oops\r\nContent-Type: text/plain\r\nContent-Length: 4\r\nConnection: close\r\n\r\nnope";
        let (address, request) = https_server(response)?;
        let fetched = web(address).get(&url()?)?;
        request.recv().ok();
        assert_eq!((fetched.status, fetched.body.as_str()), (500, "nope"));
        Ok(())
    }

    #[test]
    fn reports_a_redirect_without_following_it() -> Outcome {
        let response = "HTTP/1.1 302 Found\r\nLocation: https://elsewhere.example/x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        let (address, request) = https_server(response)?;
        let fetched = web(address).get(&url()?)?;
        request.recv().ok();
        assert_eq!(fetched.status, 302);
        assert_eq!(
            fetched.location.as_deref(),
            Some("https://elsewhere.example/x")
        );
        Ok(())
    }

    #[test]
    fn a_failed_connection_is_a_fetch_error() -> Outcome {
        let refused = web("127.0.0.1:1"
            .parse()
            .unwrap_or(std::net::SocketAddr::from(([127, 0, 0, 1], 1))));
        assert!(refused.get(&url()?).is_err_and(|error| {
            error
                .to_string()
                .starts_with("fetch: https://localhost/page?q=1:")
        }));
        Ok(())
    }

    #[test]
    fn the_public_resolver_refuses_private_names() -> Outcome {
        let dns = PublicDns;
        assert!(dns.checked("1.1.1.1").is_ok());
        assert!(dns.checked("127.0.0.1").is_err());
        assert!(dns.checked("localhost").is_err());
        assert!(
            HttpWeb::new()
                .agent(&url()?)
                .is_err_and(|error| { error.to_string().starts_with("refused: localhost:") })
        );
        assert!(HttpWeb::default().agent(&url()?).is_err());
        assert_eq!(url()?.to_string(), "https://localhost/page?q=1");
        Ok(())
    }
}
