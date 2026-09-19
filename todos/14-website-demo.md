# Website & demo material

> **Priority 14 · deferred:** discoverability for the testing round and beyond.
> Owns the `ocid.dev` domain and the recorded demos. Split out of
> [#15](https://github.com/safonas/ocid/issues/15) so the extension work is
> not blocked on them.

## Tasks

- **GitHub Pages site on ocid.dev**
  - Static site (README content restructured: what/why, install, two-node
    demo, extension) — no framework needed; Jekyll default or plain HTML.
  - `CNAME` = `ocid.dev`, DNS A/ALIAS records to Pages; enforce HTTPS.
  - Keep the repo README as the source of truth; the site derives or links.
  - Later: extension install button/link (`pd://` deep link or copyable
    `podman desktop` image reference).
- **asciinema demo**
  - Record the two-node terminal flow (`ocid` start, push, follow, pull on
    node B, `ocitop` peek) as an asciicast for the README + site.
  - Embed via asciinema's player (self-host the JS to avoid a third-party
    script, or link to the cast).
  - Follow-up once the Podman Desktop extension is announced: a shorter
    cast of the GUI flow (screen recording, not asciinema).

## Why deferred

The testing round needs working install paths and docs, not polish; the site
and demos are most valuable at the public announcement, after early feedback
has shaken out the setup flow.
