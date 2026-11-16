//! Runs a blocking handler and streams what it writes as the response.
//!
//! The chat and relay handlers are blocking code: they read an engine's
//! answer as it arrives and write it on. [`answered`] runs one on tokio's
//! blocking pool, with a [`Channel`] as its [`Client`]:
//!
//! ```text
//!   handler (blocking thread)                  axum (async)
//!   client.begin(status, type) ──► oneshot ──► the response's head
//!   client.write(bytes)        ──► mpsc    ──► the response's body, as it comes
//! ```
//!
//! When the client goes away, axum drops the body. The body carries a
//! [`LeaveSignal`], which marks the client as gone when it is dropped, so the
//! handler can see that through [`Client::presence`] without writing. The
//! next write fails too.

use std::{
    convert::Infallible,
    io::{self, Write},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};

use axum::{
    body::{Body, Bytes},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use tokio::sync::{mpsc, oneshot};
use tokio_stream::{Stream, StreamExt, wrappers::ReceiverStream};

use crate::{
    error::{AgentError, Outcome},
    server::response::{self, Client, Presence},
};

/// How many written pieces may wait for a slow client before the handler
/// waits too.
const PENDING_CHUNKS: usize = 64;

/// How an answer starts.
struct Head {
    status: StatusCode,
    content_type: String,
}

/// The handler's side of a response being streamed.
struct Channel {
    head: Option<oneshot::Sender<Head>>,
    chunks: mpsc::Sender<Bytes>,
    presence: Presence,
}

/// Marks the client as gone when it is dropped. axum drops it with the
/// response, which it does when the client disconnects, and also once the
/// whole answer is sent.
struct LeaveSignal(Arc<AtomicBool>);

impl Drop for LeaveSignal {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// The response body: the pieces the handler writes, with the signal that
/// tells the handler when the body is dropped.
struct WatchedBody {
    pieces: ReceiverStream<Bytes>,
    _signal: LeaveSignal,
}

impl Stream for WatchedBody {
    type Item = Bytes;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Bytes>> {
        Pin::new(&mut self.pieces).poll_next(context)
    }
}

/// The error of writing to a client that is no longer there.
fn gone() -> io::Error {
    io::Error::from(io::ErrorKind::BrokenPipe)
}

impl Write for Channel {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.chunks
            .blocking_send(Bytes::copy_from_slice(buffer))
            .map_err(|_| gone())?;

        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Client for Channel {
    fn begin(&mut self, status: StatusCode, content_type: &str) -> Outcome {
        let head = Head {
            status,
            content_type: content_type.to_string(),
        };

        self.head
            .take()
            .ok_or_else(gone)?
            .send(head)
            .map_err(|_| gone())?;

        Ok(())
    }

    fn presence(&self) -> Presence {
        self.presence.clone()
    }
}

/// Runs `work` on a blocking thread and answers with what it writes. A
/// handler that fails before it begins its answer is answered for, with the
/// status that explains the failure; one that fails later just ends.
pub async fn answered(work: impl FnOnce(&mut dyn Client) -> Outcome + Send + 'static) -> Response {
    let (head, begun) = oneshot::channel();
    let (chunks, body) = mpsc::channel(PENDING_CHUNKS);
    let left = Arc::new(AtomicBool::new(false));
    let signal = LeaveSignal(Arc::clone(&left));

    tokio::task::spawn_blocking(move || {
        let mut client = Channel {
            head: Some(head),
            chunks,
            presence: Presence::new(left),
        };

        if let Err(error) = work(&mut client) {
            report(&mut client, &error);
        }
    });

    let Ok(head) = begun.await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };

    let watched = WatchedBody {
        pieces: ReceiverStream::new(body),
        _signal: signal,
    };
    let body = Body::from_stream(watched.map(Ok::<Bytes, Infallible>));

    (
        head.status,
        [(header::CONTENT_TYPE, head.content_type)],
        body,
    )
        .into_response()
}

/// Logs a failed handler and tells the client, when it is still waiting for
/// the answer to begin.
fn report(client: &mut Channel, error: &AgentError) {
    eprintln!("request failed: {error}");

    if let Err(unsent) = response::failure(client, error) {
        eprintln!("could not tell the client: {unsent}");
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use axum::body::to_bytes;

    use super::*;
    use crate::server::response::{EVENT_STREAM, JSON};

    /// The status, content type and whole body of `response`.
    async fn read(response: Response) -> Result<(StatusCode, String, String)> {
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body = to_bytes(response.into_body(), usize::MAX).await?;

        Ok((
            status,
            content_type,
            String::from_utf8_lossy(&body).into_owned(),
        ))
    }

    #[tokio::test]
    async fn streams_what_the_handler_writes() -> Result<()> {
        let response = answered(|client| {
            client.begin(StatusCode::OK, EVENT_STREAM)?;
            response::send_event(client, "one")?;

            response::send_event(client, "two")
        })
        .await;

        assert_eq!(
            read(response).await?,
            (
                StatusCode::OK,
                EVENT_STREAM.to_string(),
                "data: one\n\ndata: two\n\n".to_string()
            )
        );

        Ok(())
    }

    #[tokio::test]
    async fn a_failure_before_the_answer_begins_is_the_answer() -> Result<()> {
        let refused = answered(|_| Err(AgentError::bad_request("no body"))).await;
        let broken = answered(|_| Err(AgentError::Config("unset".to_string()))).await;

        assert_eq!(
            read(refused).await?,
            (
                StatusCode::BAD_REQUEST,
                JSON.to_string(),
                r#"{"error":{"message":"bad request: no body"}}"#.to_string()
            )
        );
        assert_eq!(
            read(broken).await?,
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                String::new(),
                String::new()
            )
        );

        Ok(())
    }

    #[tokio::test]
    async fn a_failure_after_the_answer_began_ends_it() -> Result<()> {
        let response = answered(|client| {
            client.begin(StatusCode::OK, JSON)?;
            client.write_all(b"half")?;

            Err(AgentError::Upstream("engine stopped".to_string()))
        })
        .await;

        assert_eq!(
            read(response).await?,
            (StatusCode::OK, JSON.to_string(), "half".to_string())
        );

        Ok(())
    }

    #[tokio::test]
    async fn a_handler_stops_when_the_client_is_gone() -> Result<()> {
        let (stopped, seen) = oneshot::channel();
        let response = answered(move |client| {
            client.begin(StatusCode::OK, JSON)?;

            let outcome = std::iter::repeat_n(b"x", PENDING_CHUNKS * 4)
                .try_for_each(|piece| client.write_all(piece).and_then(|()| client.flush()));
            stopped
                .send(outcome.is_err())
                .map_err(|_| AgentError::Upstream("nobody is watching".to_string()))?;

            Ok(outcome?)
        })
        .await;

        drop(response);

        assert_eq!(seen.await.ok(), Some(true));

        Ok(())
    }

    /// Waits until `presence` reports the client gone, checking every 10 ms.
    fn wait_for_leave(presence: &Presence) {
        while !presence.is_gone() {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[tokio::test]
    async fn a_handler_sees_a_gone_client_without_writing() -> Result<()> {
        let (began, begun) = oneshot::channel();
        let (seen, saw) = oneshot::channel();
        let response = answered(move |client| {
            client.begin(StatusCode::OK, EVENT_STREAM)?;
            let presence = client.presence();
            let before = presence.is_gone();
            began
                .send(())
                .map_err(|_| AgentError::Upstream("nobody is watching".to_string()))?;

            wait_for_leave(&presence);

            seen.send(before)
                .map_err(|_| AgentError::Upstream("nobody is watching".to_string()))?;

            Ok(())
        })
        .await;

        begun.await?;
        drop(response);

        let gone_before_the_drop =
            tokio::time::timeout(std::time::Duration::from_secs(5), saw).await??;

        assert!(!gone_before_the_drop);

        Ok(())
    }

    #[test]
    fn an_answer_begins_once_and_only_for_a_waiting_client() {
        let (head, begun) = oneshot::channel();
        let (chunks, _body) = mpsc::channel(1);
        let mut waited_for = Channel {
            head: Some(head),
            chunks: chunks.clone(),
            presence: Presence::unwatched(),
        };
        let (head, gone) = oneshot::channel();
        let mut left = Channel {
            head: Some(head),
            chunks,
            presence: Presence::unwatched(),
        };

        drop(gone);

        assert!(waited_for.begin(StatusCode::OK, JSON).is_ok());
        assert!(waited_for.begin(StatusCode::OK, JSON).is_err());
        assert!(left.begin(StatusCode::OK, JSON).is_err());
        assert!(
            begun
                .blocking_recv()
                .is_ok_and(|head| head.status == StatusCode::OK)
        );
    }
}
