use std::{collections::HashMap, sync::Mutex};

use super::*;

/// A page that answers 200 with `body`.
fn html(body: &str) -> Fetched {
    page(200, "text/html", body)
}

/// A redirect with `status` to `location`.
fn redirect(status: u16, location: &str) -> Fetched {
    let mut moved = page(status, "text/html", "");
    moved.location = Some(location.to_string());

    moved
}

/// What reading `https://example.com/` finds when the site holds `pages`.
fn read_from(pages: &[(&str, Fetched)]) -> Outcome<Report> {
    let start = url("https://example.com/")?;

    read_recursive(
        &Fake::with(pages),
        &start,
        &allowed(&["example.com"]),
        MAX_PAGES,
    )
}

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
    let links = "<title>Start</title><a href=\"/a\">A</a><a href=\"/b\">B</a>";
    let report = read_from(&[
        (
            "https://example.com/",
            page(200, "text/html; charset=utf-8", links),
        ),
        (
            "https://example.com/a",
            html("<title>A</title><p>alpha</p>"),
        ),
        ("https://example.com/b", page(200, "text/plain", "beta")),
    ])?;
    let read: Vec<(&str, &str)> = report
        .pages
        .iter()
        .map(|page| (page.url.as_str(), page.title.as_str()))
        .collect();

    assert_eq!(
        read,
        [
            ("https://example.com/", "Start"),
            ("https://example.com/a", "A"),
            ("https://example.com/b", "example.com"),
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
    let links = "<title>Start</title><a href=\"/a\">A</a><a href=\"/b\">B</a>";
    let pages = [
        ("https://example.com/", html(links)),
        ("https://example.com/a", html("<title>A</title>")),
        ("https://example.com/b", html("<title>B</title>")),
    ];
    let fake = Fake::with(&pages);
    let report = read_recursive(&fake, &start, &allowed(&["example.com"]), 2)?;
    let without_a_budget =
        read_recursive(&Fake::with(&pages), &start, &allowed(&["example.com"]), 0)?;

    assert_eq!(report.pages.len(), 2);
    assert_eq!(fake.asked().len(), 2);
    assert_eq!(without_a_budget.pages.len(), 1);

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
fn follows_a_redirect_to_the_page() -> Outcome {
    let report = read_from(&[
        ("https://example.com/", redirect(302, "/final")),
        (
            "https://example.com/final",
            page(200, "text/plain", "arrived"),
        ),
    ])?;

    assert_eq!(report.pages[0].url, "https://example.com/final");
    assert!(report.text.contains("arrived"));

    Ok(())
}

#[test]
fn reports_a_redirect_loop_and_a_bad_redirect() {
    let looping = read_from(&[
        ("https://example.com/", redirect(302, "/loop")),
        ("https://example.com/loop", redirect(307, "/loop")),
    ]);
    let bad = read_from(&[("https://example.com/", redirect(302, "//:bad"))]);

    assert!(looping.is_err_and(|error| error.to_string().contains("too many redirects")));
    assert!(bad.is_err_and(|error| error.to_string().contains("bad redirect")));
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

/// A site whose every page takes a moment to arrive, counting how many are on
/// their way at once.
#[derive(Debug, Default)]
struct Slow {
    site: Fake,
    on_the_way: std::sync::atomic::AtomicUsize,
    most_at_once: std::sync::atomic::AtomicUsize,
}

impl Web for Slow {
    fn get(&self, url: &Url) -> Outcome<Fetched> {
        use std::sync::atomic::Ordering;

        let now = self.on_the_way.fetch_add(1, Ordering::SeqCst) + 1;
        self.most_at_once.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(std::time::Duration::from_millis(100));
        self.on_the_way.fetch_sub(1, Ordering::SeqCst);

        self.site.get(url)
    }
}

#[test]
fn the_links_of_a_page_are_read_at_the_same_time_and_kept_in_order() -> Outcome {
    let start = url("https://example.com/")?;
    let links = "<title>Start</title><a href=\"/a\">A</a><a href=\"/b\">B</a><a href=\"/c\">C</a>";
    let slow = Slow {
        site: Fake::with(&[
            ("https://example.com/", html(links)),
            ("https://example.com/a", html("<title>A</title>")),
            ("https://example.com/b", html("<title>B</title>")),
            ("https://example.com/c", html("<title>C</title>")),
        ]),
        ..Slow::default()
    };

    let report = read_recursive(&slow, &start, &allowed(&["example.com"]), MAX_PAGES)?;

    let titles: Vec<&str> = report
        .pages
        .iter()
        .map(|page| page.title.as_str())
        .collect();

    assert_eq!(titles, ["Start", "A", "B", "C"]);
    assert_eq!(
        slow.most_at_once.load(std::sync::atomic::Ordering::SeqCst),
        3
    );

    Ok(())
}
