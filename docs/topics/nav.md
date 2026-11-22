# Contract: nav v2

Version **2**, since warte v1.6.42; version **1** since warte v1.6.25. A reader
accepts both. The apps on one host share a home link and
an app switcher (REQ-NAV-001). The operator writes one JSON file, and every app
renders the same links from it. warte implements this contract, and any other
app that renders nav follows the same rules, so all apps accept and refuse
the same files.

## The file

Written by the operator, mode 0644, no secrets. Example:

```json
{
  "version": 1,
  "home": {"name": "tower", "url": "/"},
  "apps": [
    {"id": "warte", "name": "warte", "url": "/warte/", "desc": "control tower"},
    {"id": "photos", "name": "Photos", "url": "/photos/"},
    {"id": "login", "name": "Login", "url": "https://login.example.internal/"}
  ]
}
```

| Field | Type | Rule |
|---|---|---|
| `version` | number | 1 or 2 |
| `home` | object | `name` and `url`, both required |
| `apps` | array | 1 to 20 entries, in the order they render |
| `id` | string | `[a-z0-9-]`, 1 to 32 characters, unique |
| `name` | string | 1 to 40 characters after trimming (also `home.name`) |
| `url` | string | `/…` (not `//`) or `https://…`; no whitespace, control characters, quotes, `<`, `>` or `\` (also `home.url`; browsers read `/\` as `//`) |
| `desc` | string | optional, at most 120 characters, shown under the name |
| `mark` | string | version 2 only, optional: the app's icon, 1 to 8 characters, no whitespace, control characters, quotes, `<`, `>`, `&` or `\` |

The same file as version 2, with marks:

```json
{
  "version": 2,
  "home": {"name": "tower", "url": "/"},
  "apps": [
    {"id": "warte", "name": "warte", "url": "/warte/", "desc": "control tower", "mark": "見"},
    {"id": "photos", "name": "Photos", "url": "/photos/", "mark": "光"},
    {"id": "login", "name": "Login", "url": "https://login.example.internal/"}
  ]
}
```

- **Unknown keys** at any level are refused. A new field means a new
  version. `mark` in a version 1 file is refused.
- **Exact JSON shapes:** the file, `home` and each app are objects (an array
  in their place is refused); `version` is the integer `1` or `2` (`1.0` and
  `"1"` are refused); a key given twice is refused; `"desc": null` means no
  description and `"mark": null` no mark.
- **A mark counts code points, not graphemes.** Grapheme rules change with the
  Unicode version, so two apps could disagree on one file. 家, 見 and 光 are
  one code point each; eight leave room for an emoji joined with ZWJ.
- **Lengths count characters** as Unicode code points, after trimming for
  names.
- **A refused file names the field**, including a value of the wrong type
  (`apps[1].name: invalid type …`).
- **Relative URLs** (`/warte/`) work under every name the host has. Use an
  absolute `https://` URL only for another host.
- **Read once at startup.** An app checks the file when it starts and refuses
  to start if any rule fails, naming the field. After changing the file, the
  operator restarts the apps.

## Each app's setting

The file contract does not fix how an app is told the path and its own id.
- **warte:** `warte.toml`:

  ```toml
  [nav]
  file = "/etc/pulseengine/nav.json"
  self = "warte"
  ```

- **Other apps** choose their own setting, for example a pair of environment
  variables `<APP>_NAV=<file>` and `<APP>_NAV_SELF=<id>`.

`self` must be one of the ids, and a file set without `self` refuses startup.

## The check

`warte nav check <file> [--self <id>]` (since warte v1.6.26) applies the same
rules and prints the same message as startup, and exits 1 when one fails, so
a configuration can refuse a bad file before it reaches a running app.

## Rendering

The same for both versions (owner decision 2026-11-21): a menu at the top
right of the page header, identical in every app.

```html
<details class="appswitch"><summary aria-label="House apps" title="House apps"><span class="dots" aria-hidden="true"><i></i><i></i><i></i><i></i></span></summary>
<nav class="apps" aria-label="House apps">
<a class="home" href="{home.url}"><span class="mark" aria-hidden="true">←</span><b>{home.name}</b><span>Home</span></a>
<a href="{url}"[ aria-current="page"]><span class="mark" aria-hidden="true">{mark}</span><b>{name}</b>[<span>{desc}</span>]</a>  (per app)
</nav></details>
```

- **Button:** four dots in a 2×2 grid, the first vermilion.
- **Panel:** 300 px wide, anchored right, scrolls when taller than the screen.
  A tap outside it or Escape closes it.
- **Home link:** first, mark `←`, the home name, then `Home`.
- **Apps:** every app follows in file order. Each mark sits in a 34 px rounded
  tile; the app's own entry is marked `aria-current="page"`, stays a link, and
  its tile is filled with the app's accent colour. Each app picks its own
  colours.
- **Mark:** the file's `mark`, or else the first character of the trimmed
  name, upper-cased.
- **Description:** visible under the name; there is no `title` attribute.
- **Link count:** a page has exactly `1 + len(apps)` nav links.
- **URLs:** rendered as given and never rewritten. Names, descriptions and
  marks are HTML-escaped.
- **Without the setting:** nothing renders, and the page is byte-identical to
  a build without nav.

## Moving to version 2

1. Every app on the host ships a release that accepts versions 1 and 2.
2. Only then does the operator switch the file to version 2 and add the marks
   (house 家, warte 見, lichtbild 光), and restart the apps.

An app that reads only version 1 refuses a version 2 file and does not start,
so the order matters.

## Kill criteria

nav is broken in an app if:
- a link in the file is not rendered;
- a rendered link is not in the file;
- a page differs when the setting is absent;
- an app accepts a file another app refuses, or the other way round;
- a shown mark is neither the file's mark nor the name's first letter.
