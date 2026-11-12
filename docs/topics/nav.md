# Contract: nav v1

Version **1**, since warte v1.6.25. The apps on one host share a home link and
an app switcher (REQ-NAV-001). The operator writes one JSON file, and every app
renders the same links from it. warte implements this contract, and any other
app that renders nav v1 follows the same rules, so all apps accept and refuse
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
| `version` | number | exactly 1 |
| `home` | object | `name` and `url`, both required |
| `apps` | array | 1 to 20 entries, in the order they render |
| `id` | string | `[a-z0-9-]`, 1 to 32 characters, unique |
| `name` | string | 1 to 40 characters after trimming (also `home.name`) |
| `url` | string | `/…` (not `//`) or `https://…`; no whitespace, control characters, quotes, `<` or `>` (also `home.url`) |
| `desc` | string | optional, at most 120 characters, shown only as the link's title |

- **Unknown keys** at any level are refused. A new field means version 2.
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

## Rendering

- **Placement:** a `<nav class="apps">` in the page header.
- **Home link:** first, `← {home.name}`, pointing at `home.url`.
- **Apps:** every app follows in file order, each as `<a href="{url}">{name}`,
  with `title="{desc}"` when a description is given. The app's own entry is
  marked `aria-current="page"` and stays a link.
- **Link count:** a page has exactly `1 + len(apps)` nav links.
- **URLs:** rendered as given and never rewritten. Names and descriptions are
  HTML-escaped.
- **Without the setting:** nothing renders, and the page is byte-identical to
  a build without nav.

## Kill criteria

nav v1 is broken in an app if:
- a link in the file is not rendered;
- a rendered link is not in the file;
- a page differs when the setting is absent.
