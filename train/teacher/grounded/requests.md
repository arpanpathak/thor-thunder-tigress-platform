<!-- source: requests/src/requests/api.py; section: 0bfb8dcba3c68aac; licence: Apache-2.0 -->
### User
I call `requests.get` for 2,000 URLs on the same host in a loop and it's slow. Is the server just slow?

### Assistant
Partly it may be, but the loop itself throws away work. Look at what `requests.get` does in `requests/api.py`: it calls `request("get", ...)`, which runs

```python,ignore
with sessions.Session() as session:
    return session.request(method=method, url=url, **kwargs)
```

Every call creates a new `Session` and closes it when the `with` block ends. A session owns the connection pool, so closing it closes the TCP connection (and TLS session). Your loop therefore does a fresh TCP handshake and TLS handshake for each of the 2,000 requests. The `with` is there on purpose, so one-off calls don't leave sockets open, but it rules out reuse.

Create one session and reuse it; requests to the same host then share kept-alive connections:

```python
import requests


def fetch_all(urls, timeout=(3.05, 10)):
    """The status code of each URL, sharing connections through one session."""
    with requests.Session() as session:
        return [session.get(url, timeout=timeout).status_code for url in urls]
```

Two more things in that loop are worth fixing while you are there:

- Pass a `timeout`. Without one, requests waits forever for a server that stops answering. A tuple sets the connect timeout and the read timeout separately.
- A session also keeps cookies and default headers (`session.headers.update(...)`) across requests, which is usually what you want for many requests to one service.

If it is still slow after that, the requests are serial: each waits for the previous response. Sending them concurrently (a thread pool with one session per thread, or an async client) is the next step.
