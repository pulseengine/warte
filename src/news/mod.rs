//! News (REQ-NEWS-001): release notes and announcements from the public feeds
//! of the tools a deployment uses, contract news v1 in `docs/topics/news.md`.
//!
//! warte polls each configured Atom or RSS feed (a GitHub repository's
//! `releases.atom` is one) once per `interval_s`, keeps every item it has
//! seen, and answers "what changed from <date> to now" through
//! `GET /api/news` and `warte news`. A reader marks items found, decided,
//! landed or dismissed through `POST /api/news/state`.
//!
//! Feed text is untrusted: it is stored as plain text, cut short, served as
//! JSON strings and never followed as instructions. Only feeds that need no
//! login are polled; errors are kept as a kind (`http 404`, `timeout`,
//! `parse`), never as a raw message.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::sync::Mutex;
use std::time::Duration;

pub const VERSION: u32 = 1;
pub const MAX_TITLE: usize = 200;
pub const MAX_EXCERPT: usize = 500;
pub const MAX_ITEMS_PER_FEED: usize = 50;
pub const MAX_QUERY: i64 = 500;
pub const STATES: &[&str] = &["found", "decided", "landed", "dismissed"];
pub const MAX_NOTE: usize = 300;

/// `[news]` in warte.toml.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NewsConfig {
    /// Seconds between polls of every feed (default one day, at least 600).
    #[serde(default = "default_interval")]
    pub interval_s: u64,
    pub sources: Vec<Source>,
}

fn default_interval() -> u64 {
    86_400
}

/// One feed.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Source {
    /// `[a-z0-9-]`, 1 to 32 characters, unique; what `--source` filters on.
    pub id: String,
    /// An `https://` Atom or RSS feed.
    pub url: String,
}

impl NewsConfig {
    /// The rules `warte serve` refuses to start on.
    pub fn check(&self) -> Result<(), String> {
        if self.interval_s < 600 {
            return Err("[news] interval_s must be at least 600".into());
        }
        if self.sources.is_empty() {
            return Err("[news] sources must list at least one feed".into());
        }
        let mut seen = std::collections::HashSet::new();
        for (i, s) in self.sources.iter().enumerate() {
            let ok = (1..=32).contains(&s.id.len())
                && s.id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
            if !ok {
                return Err(format!(
                    "[news] sources[{i}].id must be [a-z0-9-], 1 to 32 characters"
                ));
            }
            if !seen.insert(s.id.as_str()) {
                return Err(format!("[news] sources[{i}].id {:?} is not unique", s.id));
            }
            if !s.url.starts_with("https://") || s.url.chars().any(|c| c.is_whitespace()) {
                return Err(format!("[news] sources[{i}].url must be an https:// URL"));
            }
        }
        Ok(())
    }
}

/// One entry as read from a feed.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// The entry's own id (Atom `id`, RSS `guid`), else its link.
    pub key: String,
    pub title: String,
    pub link: String,
    /// RFC 3339, UTC; `None` when the feed gives no date warte can read.
    pub published: Option<String>,
    pub excerpt: String,
}

/// An item as stored and served.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, sqlx::FromRow)]
pub struct Item {
    pub id: i64,
    pub source: String,
    pub published: String,
    pub title: String,
    pub link: String,
    pub excerpt: String,
    pub hash: String,
    pub first_seen: String,
    pub state: String,
    pub note: String,
    pub state_at: Option<String>,
}

/// Collapse whitespace, drop markup, cut to `max` characters.
pub fn plain_text(s: &str, max: usize) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if in_tag => {}
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    let collapsed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut cut: String = collapsed.chars().take(max).collect();
    if collapsed.chars().count() > max {
        cut.push('…');
    }
    cut
}

fn date(s: &str) -> Option<String> {
    let s = s.trim();
    // RSS dates name a weekday; feeds get it wrong, and the date is what counts
    let no_weekday = match s.split_once(", ") {
        Some((d, rest)) if d.len() == 3 && d.chars().all(|c| c.is_ascii_alphabetic()) => rest,
        _ => s,
    };
    let t = chrono::DateTime::parse_from_rfc3339(s)
        .or_else(|_| chrono::DateTime::parse_from_rfc2822(no_weekday))
        .ok()?;
    Some(
        t.with_timezone(&chrono::Utc)
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    )
}

/// Read the entries of an Atom or RSS 2.0 document, newest-first as given,
/// at most `MAX_ITEMS_PER_FEED`. Entries without an https link are skipped.
pub fn parse_feed(xml: &str) -> Result<Vec<Entry>, String> {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(xml);
    r.config_mut().trim_text(true);
    let mut out = Vec::new();
    let mut root: Option<String> = None;
    // the current entry's fields, and the element whose text is being read
    let mut cur: Option<std::collections::HashMap<String, String>> = None;
    let mut field: Option<String> = None;
    loop {
        let ev = r.read_event().map_err(|e| format!("parse: {e}"))?;
        match ev {
            Event::Start(e) | Event::Empty(e) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                if root.is_none() {
                    root = Some(name.clone());
                }
                if name == "entry" || name == "item" {
                    cur = Some(Default::default());
                    field = None;
                } else if let Some(m) = cur.as_mut() {
                    // Atom: <link rel="alternate" href="..."/>
                    if name == "link" {
                        let mut href = None;
                        let mut rel = None;
                        for a in e.attributes().flatten() {
                            let k =
                                String::from_utf8_lossy(a.key.local_name().as_ref()).to_string();
                            let v = a
                                .unescape_value()
                                .map(|v| v.to_string())
                                .unwrap_or_default();
                            if k == "href" {
                                href = Some(v);
                            } else if k == "rel" {
                                rel = Some(v);
                            }
                        }
                        if let Some(h) = href {
                            if rel.as_deref().is_none_or(|r| r == "alternate") {
                                m.entry("link".into()).or_insert(h);
                            }
                        }
                    }
                    field = Some(name);
                }
            }
            Event::Text(t) => {
                if let (Some(m), Some(f)) = (cur.as_mut(), field.as_ref()) {
                    let v = t.unescape().map(|v| v.to_string()).unwrap_or_default();
                    m.entry(f.clone()).or_default().push_str(&v);
                }
            }
            Event::CData(t) => {
                if let (Some(m), Some(f)) = (cur.as_mut(), field.as_ref()) {
                    m.entry(f.clone())
                        .or_default()
                        .push_str(&String::from_utf8_lossy(&t));
                }
            }
            Event::End(e) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                if name == "entry" || name == "item" {
                    if let Some(m) = cur.take() {
                        let link = m
                            .get("link")
                            .map(|s| s.trim().to_string())
                            .unwrap_or_default();
                        if link.starts_with("https://") && !link.chars().any(char::is_whitespace) {
                            let pick = |ks: &[&str]| {
                                ks.iter()
                                    .find_map(|k| m.get(*k).filter(|v| !v.trim().is_empty()))
                            };
                            let key = pick(&["id", "guid"]).map(|s| s.trim().to_string());
                            out.push(Entry {
                                key: key.unwrap_or_else(|| link.clone()),
                                title: plain_text(pick(&["title"]).map_or("", |s| s), MAX_TITLE),
                                link,
                                published: pick(&["published", "updated", "pubDate", "date"])
                                    .and_then(|d| date(d)),
                                excerpt: plain_text(
                                    pick(&["content", "summary", "description", "encoded"])
                                        .map_or("", |s| s),
                                    MAX_EXCERPT,
                                ),
                            });
                            if out.len() >= MAX_ITEMS_PER_FEED {
                                break;
                            }
                        }
                    }
                }
                field = None;
            }
            Event::Eof => break,
            _ => {}
        }
    }
    match root.as_deref() {
        Some("feed") | Some("rss") | Some("RDF") => Ok(out),
        _ => Err("parse: not an Atom or RSS document".into()),
    }
}

pub fn hash(e: &Entry) -> String {
    let mut h = Sha256::new();
    for part in [&e.title, &e.link, &e.excerpt] {
        h.update(part.as_bytes());
        h.update([0]);
    }
    hex::encode(h.finalize())
}

/// Store a feed's entries: a new key is a new item; a known key whose content
/// changed is updated in place, keeping its state. Returns the new items.
pub async fn store(
    pool: &SqlitePool,
    source: &str,
    entries: &[Entry],
    now: &str,
) -> Result<usize, sqlx::Error> {
    let mut new = 0;
    for e in entries {
        let h = hash(e);
        let published = e.published.clone().unwrap_or_else(|| now.to_string());
        let r = sqlx::query(
            "INSERT INTO news_items (source, key, published, title, link, excerpt, hash, first_seen)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(source, key) DO UPDATE SET
               title = excluded.title, link = excluded.link, excerpt = excluded.excerpt,
               hash = excluded.hash, published = excluded.published
             WHERE news_items.hash <> excluded.hash",
        )
        .bind(source)
        .bind(&e.key)
        .bind(&published)
        .bind(&e.title)
        .bind(&e.link)
        .bind(&e.excerpt)
        .bind(&h)
        .bind(now)
        .execute(pool)
        .await?;
        if r.rows_affected() > 0 {
            let fresh: (String,) =
                sqlx::query_as("SELECT first_seen FROM news_items WHERE source = ? AND key = ?")
                    .bind(source)
                    .bind(&e.key)
                    .fetch_one(pool)
                    .await?;
            if fresh.0 == now {
                new += 1;
            }
        }
    }
    Ok(new)
}

/// What `GET /api/news` and `warte news` filter on.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Query {
    /// `YYYY-MM-DD` or RFC 3339; items published at or after it.
    pub since: Option<String>,
    pub source: Option<String>,
    /// Case-insensitive substring of title or excerpt.
    pub grep: Option<String>,
    pub state: Option<String>,
}

fn since_bound(s: &str) -> Result<String, String> {
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(format!("{d}T00:00:00Z"));
    }
    date(s).ok_or_else(|| format!("since {s:?} is not YYYY-MM-DD or RFC 3339"))
}

/// Items matching `q`, newest first, at most `MAX_QUERY`.
pub async fn query(pool: &SqlitePool, q: &Query) -> Result<Vec<Item>, String> {
    let since = q.since.as_deref().map(since_bound).transpose()?;
    if let Some(s) = &q.state {
        if !STATES.contains(&s.as_str()) {
            return Err(format!("state must be one of {}", STATES.join(", ")));
        }
    }
    let grep = q.grep.as_ref().map(|g| format!("%{}%", g.to_lowercase()));
    sqlx::query_as::<_, Item>(
        "SELECT id, source, published, title, link, excerpt, hash, first_seen, state, note, state_at
         FROM news_items
         WHERE (?1 IS NULL OR published >= ?1)
           AND (?2 IS NULL OR source = ?2)
           AND (?3 IS NULL OR lower(title) LIKE ?3 OR lower(excerpt) LIKE ?3)
           AND (?4 IS NULL OR state = ?4)
         ORDER BY published DESC, id DESC LIMIT ?5",
    )
    .bind(since)
    .bind(&q.source)
    .bind(grep)
    .bind(&q.state)
    .bind(MAX_QUERY)
    .fetch_all(pool)
    .await
    .map_err(|_| "the news store could not be read".to_string())
}

/// A reader's verdict on an item.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateChange {
    pub id: i64,
    pub state: String,
    #[serde(default)]
    pub note: String,
}

pub async fn set_state(pool: &SqlitePool, c: &StateChange, now: &str) -> Result<(), String> {
    if !STATES.contains(&c.state.as_str()) {
        return Err(format!("state must be one of {}", STATES.join(", ")));
    }
    if c.note.chars().count() > MAX_NOTE || c.note.chars().any(|c| c.is_control()) {
        return Err(format!(
            "note must be at most {MAX_NOTE} characters, without control characters"
        ));
    }
    let r = sqlx::query("UPDATE news_items SET state = ?, note = ?, state_at = ? WHERE id = ?")
        .bind(&c.state)
        .bind(&c.note)
        .bind(now)
        .bind(c.id)
        .execute(pool)
        .await
        .map_err(|_| "the news store could not be written".to_string())?;
    if r.rows_affected() == 0 {
        return Err(format!("no item {}", c.id));
    }
    Ok(())
}

/// The last poll of one feed, as `GET /api/news/sources` shows it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SourceStatus {
    pub id: String,
    pub url: String,
    pub last_poll: Option<String>,
    /// `ok`, `http <code>`, `timeout`, `fetch` or `parse`; `None` before the first poll.
    pub result: Option<String>,
    pub entries: usize,
    pub new: usize,
}

pub struct News {
    pub config: NewsConfig,
    pub pool: SqlitePool,
    pub status: Mutex<Vec<SourceStatus>>,
    http: reqwest::Client,
}

impl News {
    pub fn new(config: NewsConfig, pool: SqlitePool) -> Self {
        let status = config
            .sources
            .iter()
            .map(|s| SourceStatus {
                id: s.id.clone(),
                url: s.url.clone(),
                last_poll: None,
                result: None,
                entries: 0,
                new: 0,
            })
            .collect();
        let http = reqwest::Client::builder()
            .user_agent(concat!("warte/", env!("CARGO_PKG_VERSION"), " (news v1)"))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client");
        News {
            config,
            pool,
            status: Mutex::new(status),
            http,
        }
    }

    /// Poll every feed once.
    pub async fn poll_all(&self) {
        for (i, s) in self.config.sources.iter().enumerate() {
            let now = crate::news::now();
            let (result, entries, new) = match self.fetch(&s.url).await {
                Err(kind) => (kind, 0, 0),
                Ok(body) => match parse_feed(&body) {
                    Err(_) => ("parse".to_string(), 0, 0),
                    Ok(es) => match store(&self.pool, &s.id, &es, &now).await {
                        Ok(n) => ("ok".to_string(), es.len(), n),
                        Err(_) => ("store".to_string(), es.len(), 0),
                    },
                },
            };
            if let Ok(mut st) = self.status.lock() {
                st[i] = SourceStatus {
                    id: s.id.clone(),
                    url: s.url.clone(),
                    last_poll: Some(now),
                    result: Some(result),
                    entries,
                    new,
                };
            }
        }
    }

    async fn fetch(&self, url: &str) -> Result<String, String> {
        let r = self.http.get(url).send().await.map_err(|e| {
            if e.is_timeout() {
                "timeout".to_string()
            } else {
                "fetch".to_string()
            }
        })?;
        if !r.status().is_success() {
            return Err(format!("http {}", r.status().as_u16()));
        }
        let b = r.bytes().await.map_err(|_| "fetch".to_string())?;
        if b.len() > 4 << 20 {
            return Err("fetch".into());
        }
        Ok(String::from_utf8_lossy(&b).into_owned())
    }
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Poll at start and then every `interval_s`.
pub fn spawn(n: std::sync::Arc<News>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(n.config.interval_s));
        loop {
            tick.tick().await;
            n.poll_all().await;
        }
    });
}

/// `warte news`: ask a running warte.
pub mod client {
    use super::{Item, Query};

    pub async fn fetch(
        base: &str,
        q: &Query,
        ca_cert: Option<&std::path::Path>,
        token: Option<&str>,
    ) -> anyhow::Result<String> {
        let mut http = reqwest::Client::builder();
        if let Some(p) = ca_cert {
            let bad = |e: &dyn std::fmt::Display| anyhow::anyhow!("--ca-cert {}: {e}", p.display());
            let pem = std::fs::read(p).map_err(|e| bad(&e))?;
            let certs = reqwest::Certificate::from_pem_bundle(&pem).map_err(|e| bad(&e))?;
            if certs.is_empty() {
                return Err(bad(&"no PEM certificate in the file"));
            }
            for c in certs {
                http = http.add_root_certificate(c);
            }
        }
        let mut query: Vec<(&str, &str)> = Vec::new();
        for (k, v) in [
            ("since", &q.since),
            ("source", &q.source),
            ("grep", &q.grep),
            ("state", &q.state),
        ] {
            if let Some(v) = v {
                query.push((k, v));
            }
        }
        let mut req = http.build()?.get(format!("{base}/api/news")).query(&query);
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        let r = req.send().await?;
        let status = r.status();
        let body = r.text().await?;
        if !status.is_success() {
            anyhow::bail!("warte answered {}: {}", status.as_u16(), body.trim());
        }
        Ok(body)
    }

    /// One block per item: date, source, state, title, link.
    pub fn text(body: &str) -> anyhow::Result<String> {
        #[derive(serde::Deserialize)]
        struct Page {
            items: Vec<Item>,
        }
        let p: Page = serde_json::from_str(body)?;
        let mut out = String::new();
        for i in &p.items {
            out.push_str(&format!(
                "{}  {:<16} {:<9} {}\n    {}\n",
                &i.published[..10.min(i.published.len())],
                i.source,
                i.state,
                i.title,
                i.link
            ));
        }
        out.push_str(&format!("{} items\n", p.items.len()));
        Ok(out)
    }
}
