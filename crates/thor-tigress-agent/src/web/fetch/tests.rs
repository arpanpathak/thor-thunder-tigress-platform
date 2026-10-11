use std::{collections::HashMap, sync::Mutex};

use super::*;

#[derive(Debug, Default)]
struct Fake {
    pages: HashMap<String, Fetched>,
    asked: Mutex<Vec<String>>,
}

impl Fake {
    fn with(fetched: &[(&str, Fetched)]) -> Self {
        let mut fake = Fake::default();

        for (url, page) in fetched {
            fake.pages.insert((*url).to_string(), page.clone());
        }

        fake
    }

    fn asked(&self) -> Vec<String> {
        self.asked
            .lock()
            .map(|asked| asked.clone())
            .unwrap_or_default()
    }
}

impl Web for Fake {
    fn get(&self, url: &Url) -> Outcome<Fetched> {
        let mut asked = self
            .asked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        asked.push(url.as_string());

        self.pages
            .get(&url.as_string())
            .cloned()
            .ok_or_else(|| AgentError::Fetch(format!("{}: not in the fake", url.as_string())))
    }
}

fn page(status: u16, content_type: &str, body: &str) -> Fetched {
    Fetched {
        status,
        content_type: content_type.to_string(),
        location: None,
        body: body.to_string(),
    }
}

fn allowed(hosts: &[&str]) -> Allowed {
    let mut allowed = Allowed::new();

    for host in hosts {
        allowed.add_host(host);
    }

    allowed
}

fn url(raw: &str) -> Outcome<Url> {
    Url::parse(raw)
}

#[test]
fn an_allowed_set_is_the_hosts_it_was_given() -> Outcome {
    let mut allowed = Allowed::from_text(
        "try https://example.com/a, or (http://jobs.example.org/x)! not httpology",
    );
    allowed.add_url("https://example.com/other");
    allowed.add_url("not an address");
    allowed.add_host("  Sub.Example.COM. ");

    assert!(allowed.allows(&url("https://example.com/deep")?));
    assert!(allowed.allows(&url("https://www.example.com/")?));
    assert!(allowed.allows(&url("https://jobs.example.org/")?));
    assert!(allowed.allows(&url("https://sub.example.com/")?));
    assert!(!allowed.allows(&url("https://other.net/")?));
    assert_eq!(urls_in("no links here"), Vec::<&str>::new());
    assert_eq!(urls_in("http://a/"), ["http://a/"]);

    Ok(())
}

#[test]
fn reads_a_page_and_its_same_site_links() -> Outcome {
    let start = url("https://example.com/")?;
    let fake = Fake::with(&[
        (
            "https://example.com/",
            page(
                200,
                "text/html; charset=utf-8",
                "<title>Start</title><a href=\"/a\">A</a><a href=\"/b\">B</a>",
            ),
        ),
        (
            "https://example.com/a",
            page(200, "text/html", "<title>A</title><p>alpha</p>"),
        ),
        ("https://example.com/b", page(200, "text/plain", "beta")),
    ]);
    let report = read_recursive(&fake, &start, &allowed(&["example.com"]), MAX_PAGES)?;

    assert_eq!(
        report.pages,
        [
            Read {
                url: "https://example.com/".to_string(),
                title: "Start".to_string()
            },
            Read {
                url: "https://example.com/a".to_string(),
                title: "A".to_string()
            },
            Read {
                url: "https://example.com/b".to_string(),
                title: "example.com".to_string()
            },
        ]
    );
    assert!(report.text.starts_with("Untrusted text from example.com."));
    assert!(report.text.contains("[1] Start (https://example.com/)"));
    assert!(report.text.contains("alpha") && report.text.contains("beta"));

    Ok(())
}

#[test]
fn a_login_wall_is_not_followed() -> Outcome {
    let start = url("https://example.com/jobs/1")?;
    let fake = Fake::with(&[
        (
            "https://example.com/jobs/1",
            page(
                200,
                "text/html",
                "<title>Job</title><a href=\"/login\">Sign in</a><a href=\"/jobs/2\">Next</a>",
            ),
        ),
        (
            "https://example.com/jobs/2",
            page(200, "text/html", "<title>Next</title>"),
        ),
    ]);
    let report = read_recursive(&fake, &start, &allowed(&["example.com"]), MAX_PAGES)?;
    let read: Vec<&str> = report.pages.iter().map(|page| page.url.as_str()).collect();

    assert_eq!(
        read,
        ["https://example.com/jobs/1", "https://example.com/jobs/2"]
    );
    assert!(is_chrome(&url("https://example.com/login")?));
    assert!(!is_chrome(&url("https://example.com/jobs/1")?));

    Ok(())
}

#[test]
fn a_page_at_the_depth_limit_has_no_links_to_follow() -> Outcome {
    let address = url("https://example.com/a")?;
    let page = html_page(&address, "<title>A</title><a href=\"/b\">B</a>", MAX_DEPTH)?;

    assert!(page.1.is_empty());

    Ok(())
}

#[test]
fn a_page_budget_stops_the_crawl_early() -> Outcome {
    let start = url("https://example.com/")?;
    let pages = [
        (
            "https://example.com/",
            page(
                200,
                "text/html",
                "<title>Start</title><a href=\"/a\">A</a><a href=\"/b\">B</a>",
            ),
        ),
        (
            "https://example.com/a",
            page(200, "text/html", "<title>A</title>"),
        ),
        (
            "https://example.com/b",
            page(200, "text/html", "<title>B</title>"),
        ),
    ];
    let fake = Fake::with(&pages);
    let report = read_recursive(&fake, &start, &allowed(&["example.com"]), 2)?;

    assert_eq!(report.pages.len(), 2);
    assert_eq!(fake.asked().len(), 2);

    let none = Fake::with(&pages);

    assert_eq!(
        read_recursive(&none, &start, &allowed(&["example.com"]), 0)?
            .pages
            .len(),
        1
    );

    Ok(())
}

#[test]
fn the_crawl_stops_at_two_hops_and_six_pages() -> Outcome {
    let start = url("https://example.com/")?;
    let mut pages = vec![(
        "https://example.com/".to_string(),
        page(200, "text/html", "<a href=\"/1\">1</a>"),
    )];

    for level in 1..=3 {
        for n in 0..3 {
            let url = format!("https://example.com/{level}{n}");
            let body = format!("<a href=\"/{}{n}\">next</a>", level + 1);
            pages.push((url, page(200, "text/html", &body)));
        }
    }

    let fake = Fake::default();
    let mut fake = fake;

    for (url, fetched) in pages {
        fake.pages.insert(url, fetched);
    }

    let report = read_recursive(&fake, &start, &allowed(&["example.com"]), MAX_PAGES)?;

    assert!(report.pages.len() <= MAX_PAGES, "{:?}", report.pages);
    assert!(
        !fake
            .asked()
            .iter()
            .any(|url| url.starts_with("https://example.com/4")),
        "depth 3 was queued"
    );

    Ok(())
}

#[test]
fn stops_at_six_pages_with_links_still_queued() -> Outcome {
    let start = url("https://example.com/")?;
    let links: String = (1..=10)
        .map(|n| format!("<a href=\"/{n}\">x</a>"))
        .collect();
    let mut fake = Fake::with(&[("https://example.com/", page(200, "text/html", &links))]);

    for n in 1..=10 {
        fake.pages.insert(
            format!("https://example.com/{n}"),
            page(200, "text/plain", "leaf"),
        );
    }

    let report = read_recursive(&fake, &start, &allowed(&["example.com"]), MAX_PAGES)?;

    assert_eq!(report.pages.len(), MAX_PAGES);
    assert_eq!(fake.asked().len(), MAX_PAGES);

    Ok(())
}

#[test]
fn follows_redirects_and_reports_too_many() -> Outcome {
    let start = url("https://example.com/")?;
    let mut moved = page(302, "text/html", "");
    moved.location = Some("/loop".to_string());

    let mut forever = page(307, "text/html", "");
    forever.location = Some("/loop".to_string());

    let fake = Fake::with(&[
        ("https://example.com/", moved),
        ("https://example.com/loop", forever),
    ]);

    assert!(
        read_recursive(&fake, &start, &allowed(&["example.com"]), MAX_PAGES)
            .is_err_and(|error| error.to_string().contains("too many redirects"))
    );

    let mut once = page(302, "text/html", "");
    once.location = Some("/final".to_string());

    let fake = Fake::with(&[
        ("https://example.com/", once),
        (
            "https://example.com/final",
            page(200, "text/plain", "arrived"),
        ),
    ]);
    let report = read_recursive(&fake, &start, &allowed(&["example.com"]), MAX_PAGES)?;

    assert_eq!(report.pages[0].url, "https://example.com/final");
    assert!(report.text.contains("arrived"));

    let mut bad = page(302, "text/html", "");
    bad.location = Some("//:bad".to_string());

    let fake = Fake::with(&[("https://example.com/", bad)]);

    assert!(
        read_recursive(&fake, &start, &allowed(&["example.com"]), MAX_PAGES)
            .is_err_and(|error| error.to_string().contains("bad redirect"))
    );

    Ok(())
}

#[test]
fn a_refused_link_is_noted_and_left_out() -> Outcome {
    let start = url("https://example.com/")?;
    let fake = Fake::with(&[(
        "https://example.com/",
        page(200, "text/html", "<a href=\"/missing\">x</a>"),
    )]);
    let report = read_recursive(&fake, &start, &allowed(&["example.com"]), MAX_PAGES)?;

    assert_eq!(report.pages.len(), 1);
    assert!(
        report
            .text
            .contains("could not read https://example.com/missing"),
        "{}",
        report.text
    );

    Ok(())
}

#[test]
fn the_first_page_must_be_read_and_the_host_must_be_allowed() -> Outcome {
    let start = url("https://example.com/")?;
    let empty = Fake::default();

    assert!(read_recursive(&empty, &start, &allowed(&["example.com"]), MAX_PAGES).is_err());
    assert!(
        read_recursive(&empty, &start, &Allowed::new(), MAX_PAGES).is_err_and(|error| {
            error
                .to_string()
                .contains("not in this answer's search results")
        })
    );

    Ok(())
}

#[test]
fn only_text_pages_are_read() -> Outcome {
    let start = url("https://example.com/a.pdf")?;
    let fake = Fake::with(&[(
        "https://example.com/a.pdf",
        page(200, "application/pdf", "%PDF"),
    )]);

    assert!(
        read_recursive(&fake, &start, &allowed(&["example.com"]), MAX_PAGES)
            .is_err_and(|error| { error.to_string().contains("is not a text page") })
    );
    assert_eq!(kind_of("text/markdown", &start).ok(), Some(Kind::Text));
    assert_eq!(
        kind_of("application/xhtml+xml;charset=utf-8", &start).ok(),
        Some(Kind::Html)
    );

    Ok(())
}

#[test]
fn a_visited_page_is_read_once() -> Outcome {
    let start = url("https://example.com/")?;
    let fake = Fake::with(&[(
        "https://example.com/",
        page(200, "text/html", "<a href=\"/\">again</a>"),
    )]);
    let report = read_recursive(&fake, &start, &allowed(&["example.com"]), MAX_PAGES)?;

    assert_eq!(report.pages.len(), 1);
    assert_eq!(fake.asked().len(), 1);

    Ok(())
}

#[test]
fn the_total_text_is_cut_at_a_line_end() -> Outcome {
    let start = url("https://example.com/")?;
    let long = "word ".repeat(4_000);
    let mut pages = Vec::new();

    for n in 0..4 {
        pages.push((
            format!("https://example.com/{n}"),
            page(200, "text/plain", &long),
        ));
    }

    let mut root = page(
        200,
        "text/html",
        "<a href=\"/0\">a</a><a href=\"/1\">b</a><a href=\"/2\">c</a><a href=\"/3\">d</a>",
    );
    root.body = format!("<title>root</title>{}", root.body);

    let mut fake = Fake::with(&[("https://example.com/", root)]);

    for (url, fetched) in pages {
        fake.pages.insert(url, fetched);
    }

    let report = read_recursive(&fake, &start, &allowed(&["example.com"]), MAX_PAGES)?;

    assert!(report.text.ends_with("[...]\n") || report.text.len() < MAX_TOTAL);
    assert!(report.text.chars().count() <= MAX_TOTAL);

    Ok(())
}
