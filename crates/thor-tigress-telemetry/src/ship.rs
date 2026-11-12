//! Sends events to OpenObserve: `POST /api/{org}/{stream}/_json` with a JSON
//! array, authorized as a user with HTTP basic authentication.

use std::time::Duration;

use serde::Serialize;
use ureq::{Agent, config::Config};

use crate::error::{Outcome, TelemetryError};

/// How long one send may take.
const TIMEOUT: Duration = Duration::from_secs(10);

/// The characters of standard base64.
const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// An OpenObserve organization to send events to.
pub struct OpenObserve {
    base: String,
    org: String,
    authorization: String,
    agent: Agent,
}

impl OpenObserve {
    /// The organization `org` at `base` (such as `http://127.0.0.1:5080`), as
    /// `user` with `password`.
    #[must_use]
    pub fn new(base: &str, org: &str, user: &str, password: &str) -> Self {
        let config: Config = Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .build();

        OpenObserve {
            base: base.trim_end_matches('/').to_string(),
            org: org.to_string(),
            authorization: format!("Basic {}", base64(format!("{user}:{password}").as_bytes())),
            agent: config.into(),
        }
    }

    /// Sends `events` to `stream`. Nothing is sent when there are none.
    ///
    /// # Errors
    ///
    /// [`TelemetryError::Send`] when OpenObserve cannot be reached or answers
    /// with an error status.
    pub fn send(&self, stream: &str, events: &[impl Serialize]) -> Outcome {
        if events.is_empty() {
            return Ok(());
        }

        let url = format!("{}/api/{}/{}/_json", self.base, self.org, stream);
        let body = serde_json::to_vec(events)?;

        self.agent
            .post(&url)
            .header("Authorization", &self.authorization)
            .header("Content-Type", "application/json")
            .send(&body[..])
            .map_err(|error| TelemetryError::Send(error.to_string()))?;

        Ok(())
    }
}

/// `bytes` in standard base64, with `=` padding.
fn base64(bytes: &[u8]) -> String {
    bytes
        .chunks(3)
        .flat_map(encode_group)
        .map(char::from)
        .collect()
}

/// One group of up to three bytes as four base64 characters.
fn encode_group(group: &[u8]) -> Vec<u8> {
    let padded = [
        group[0],
        *group.get(1).unwrap_or(&0),
        *group.get(2).unwrap_or(&0),
    ];
    let joined = u32::from(padded[0]) << 16 | u32::from(padded[1]) << 8 | u32::from(padded[2]);
    let kept = group.len() + 1;

    [18, 12, 6, 0]
        .iter()
        .enumerate()
        .map(|(position, shift)| character(joined >> shift, position < kept))
        .collect()
}

fn character(sextet: u32, kept: bool) -> u8 {
    if !kept {
        return b'=';
    }

    BASE64[(sextet & 0x3f) as usize]
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;
    use crate::testing::fake_openobserve;

    #[test]
    fn encodes_base64_like_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(
            base64(b"admin@thor.local:p4ss"),
            "YWRtaW5AdGhvci5sb2NhbDpwNHNz"
        );
    }

    #[test]
    fn posts_the_events_as_a_json_array_with_basic_auth() -> Result<()> {
        let (address, received) = fake_openobserve(vec!["200 OK"])?;
        let sink = OpenObserve::new(
            &format!("{address}/"),
            "default",
            "admin@thor.local",
            "p4ss",
        );

        sink.send("requests", &[serde_json::json!({"who": "ada"})])?;

        let request = received.recv()?;

        assert!(request.starts_with("POST /api/default/requests/_json "));
        assert!(request.contains("authorization: Basic YWRtaW5AdGhvci5sb2NhbDpwNHNz"));
        assert!(request.ends_with(r#"[{"who":"ada"}]"#));

        Ok(())
    }

    #[test]
    fn a_refusal_or_no_server_is_an_error_and_nothing_to_send_is_not() -> Result<()> {
        let (address, _received) = fake_openobserve(vec!["401 Unauthorized"])?;
        let closed = format!(
            "http://{}",
            std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?
        );
        let event = [serde_json::json!({})];

        let refused = OpenObserve::new(&address, "default", "u", "p").send("requests", &event);
        let unreachable = OpenObserve::new(&closed, "default", "u", "p").send("requests", &event);
        let nothing: [serde_json::Value; 0] = [];
        let empty = OpenObserve::new(&closed, "default", "u", "p").send("requests", &nothing);

        assert!(matches!(refused, Err(TelemetryError::Send(_))));
        assert!(matches!(unreachable, Err(TelemetryError::Send(_))));
        assert!(empty.is_ok());

        Ok(())
    }
}
