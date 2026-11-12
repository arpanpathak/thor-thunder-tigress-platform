//! A fake OpenObserve for tests: answers one request per status it is given,
//! and hands back each request it received.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    thread,
};

use crate::error::Outcome;

/// Starts the fake on a free port. Returns its `http://` address and the
/// requests it receives, one string each: request line, headers and body.
pub fn fake_openobserve(statuses: Vec<&'static str>) -> Outcome<(String, mpsc::Receiver<String>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = format!("http://{}", listener.local_addr()?);
    let (sender, received) = mpsc::channel();

    thread::spawn(move || {
        statuses
            .into_iter()
            .map(|status| answer_one(&listener, status))
            .map_while(Result::ok)
            .for_each(|request| drop(sender.send(request)));
    });

    Ok((address, received))
}

fn answer_one(listener: &TcpListener, status: &str) -> Outcome<String> {
    let (stream, _) = listener.accept()?;
    let request = read_request(&stream)?;

    (&stream).write_all(format!("HTTP/1.1 {status}\r\ncontent-length: 0\r\n\r\n").as_bytes())?;

    Ok(request)
}

fn read_request(stream: &TcpStream) -> Outcome<String> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut head = String::new();

    while !head.ends_with("\r\n\r\n") {
        reader.read_line(&mut head)?;
    }

    let mut body = vec![0; content_length(&head)];
    reader.read_exact(&mut body)?;

    Ok(head + &String::from_utf8_lossy(&body))
}

fn content_length(head: &str) -> usize {
    let lowered = head.to_lowercase();
    let value = lowered
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .unwrap_or("0");

    value.trim().parse().unwrap_or(0)
}
