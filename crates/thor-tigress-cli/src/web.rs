//! Plain HTTP GET requests, for the Hugging Face Hub, documentation pages and the
//! engines' own `/v1/models`.

use std::time::Duration;

use crate::error::{Outcome, ThorError};

/// The largest page or JSON reply read into memory.
const TEXT_LIMIT: u64 = 64 * 1024 * 1024;

/// An HTTP client that waits at most `connect` to connect and `reply` for a reply's headers.
/// A body may take as long as it needs, so a download of many gigabytes is never cut off.
pub fn client(connect: Duration, reply: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(connect))
        .timeout_recv_response(Some(reply))
        .build()
        .into()
}

/// Starts a GET request, with `Authorization: Bearer` when a token is given and with the
/// extra headers given.
pub fn get(
    client: &ureq::Agent,
    url: &str,
    token: Option<&str>,
    headers: &[(&str, String)],
) -> Outcome<ureq::http::Response<ureq::Body>> {
    let mut request = client.get(url);

    if let Some(token) = token {
        request = request.header("Authorization", format!("Bearer {token}"));
    }

    for (name, value) in headers {
        request = request.header(*name, value);
    }

    request.call().map_err(|source| ThorError::Http {
        url: url.to_string(),
        source: Box::new(source),
    })
}

/// The body of a GET request as text.
pub fn get_text(client: &ureq::Agent, url: &str, token: Option<&str>) -> Outcome<String> {
    let mut response = get(client, url, token, &[])?;

    response
        .body_mut()
        .with_config()
        .limit(TEXT_LIMIT)
        .read_to_string()
        .map_err(|source| ThorError::Http {
            url: url.to_string(),
            source: Box::new(source),
        })
}

/// Text made safe to put in a URL's query: letters, digits and `-_.~` stay, every other
/// byte becomes `%XX`.
pub fn encode_query(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}
