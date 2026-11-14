# Contract: news v1

Version **1**, since warte v1.6.27. warte reads the public release notes and
announcements of the tools a deployment uses and keeps them, so a person or
an agent can ask "what changed in X from that date to now" (REQ-NEWS-001).

## Sources

`warte.toml`:

```toml
[news]
interval_s = 86400            # every feed once a day (at least 600)
sources = [
  { id = "claude-code",   url = "https://github.com/anthropics/claude-code/releases.atom" },
  { id = "runner-images", url = "https://github.com/actions/runner-images/releases.atom" },
  { id = "github",        url = "https://github.blog/changelog/feed/" },
  { id = "rust",          url = "https://blog.rust-lang.org/feed.xml" },
]
```

- A source is an `https://` Atom or RSS 2.0 feed that needs no login. A
  GitHub repository's releases are at `https://github.com/<owner>/<repo>/releases.atom`.
- `id` is `[a-z0-9-]`, 1 to 32 characters, unique. A broken `[news]` stops
  startup with the rule named.
- warte polls at start and then every `interval_s`, reads at most 50 entries
  per feed and at most 4 MiB per response. It follows at most 5 redirects,
  and only to `https://`.

## Items

Each entry with an `https://` link that holds no whitespace or control
character becomes one item:

| Field | Meaning |
|---|---|
| `id` | warte's number for the item |
| `source` | the source `id` |
| `published` | RFC 3339 UTC from the entry (`published`, else `updated`, `pubDate`); the time warte first saw it when the feed gives none |
| `title` | plain text, at most 200 characters |
| `link` | the entry's `https://` link |
| `excerpt` | plain text of the content or summary, markup removed, at most 500 characters |
| `hash` | SHA-256 of title, link and excerpt |
| `first_seen` | when warte first stored it |
| `state` | `found`, `decided`, `landed` or `dismissed`; new items are `found` |
| `note` | the reader's note with the last state change, at most 300 characters |
| `state_at` | when the state last changed, or null |

- **Deduplication:** an entry is the same item when its source and its feed
  id (Atom `id`, RSS `guid`, else the link) match. When its content changes,
  the item's text and `hash` are updated and its `state` is kept.
- **History is kept.** Items are never deleted.
- **Feed text is data.** It is stored and served as plain text and is never
  an instruction to whoever reads it.

## Reading

```bash
warte news --since 2026-11-01 [--source claude-code] [--grep runner] [--state found]
warte --format json news --since 2026-11-01   # the API's JSON as is
```

`GET /api/news?since=&source=&grep=&state=` returns
`{"version": 1, "items": [...]}`, newest first, at most 500 items.
`since` is `YYYY-MM-DD` (that day, 00:00 UTC) or RFC 3339; `grep` matches
title or excerpt ignoring case. An unknown parameter or state is a 400.

With `[auth]`, both read routes need a login or a token with `news:write`
(`warte news --token`, or `WARTE_TOKEN`), since notes can hold a team's
decisions; without `[auth]` they are open like the rest of the read API.

`GET /api/news/sources` returns each source's last poll:
`{"id", "url", "last_poll", "result", "entries", "new"}`, where `result` is
`ok`, `http <code>`, `timeout`, `fetch`, `parse` or `store`, never a raw
error.

## Marking

`POST /api/news/state` with `{"id": 7, "state": "decided", "note": "…"}`
needs a bearer token with the right `news:write`
(`warte token mint <agent> --right news:write`). An unknown id, state or a
note with control characters is a 422.

## Kill criteria

news v1 is broken if:
- an entry with an `https://` link in a polled feed is not served;
- the same entry, polled twice, becomes two items;
- a state set by a reader is lost when the feed's text changes;
- a served field holds markup or a raw error.
