---
name: Issues (GitHub + Radicle)
description: Create, read, update, comment, label, assign, and close GitHub and Radicle issues; keeps the repo's todos/ files in sync as a third medium; mirrors every change across both trackers when the repo has both remotes; backfills unpaired issues; polls both trackers for new comments and activity. Use for any issue creation, triage, lookup, status change, backfill, todo sync, or fetch-new-comments task.
---

Manage issues on GitHub and Radicle, keeping the repo's `todos/` files in
sync as a third medium. In a repo with both a `github` and a `rad` remote,
work in **mirror mode**: every create/update/close applies to BOTH trackers
AND the paired todo file, with all three cross-referenced. If the user names
a single medium ("on github", "radicle only", "just the todo", …), operate on
just that one. If only one remote exists, use it and say so.

## Discovery

- `git remote -v` → which trackers exist. `rad .` → the Radicle RID.
- Radicle is local-first: mutations succeed offline and announce to peers when
  the node is next running. If `rad node status` reports stopped, continue
  anyway and mention it — do not block.
- Never pass `--no-announce`; p2p sync is wanted ("✓ Synced with N seed(s)").

## Command map

| Operation  | GitHub | Radicle |
|------------|--------|---------|
| List       | `gh issue list [--state open\|closed\|all] [--label L] [--limit N]` (default limit 30) | `rad issue list [--open\|--closed\|--solved\|--all] [--assigned [<DID>]]` |
| Show       | `gh issue view <n> [--json number,title,state,body,labels,assignees,comments]` | `rad issue show <id> [--header]` |
| Create     | `gh issue create --title T --body B` → prints URL (number = #N) | `rad issue open --title T --description B` → prints OID |
| Edit title/body | `gh issue edit <n> [--title T] [--body B]` | `rad issue edit <id> [--title T] [--description B]` |
| Close / reopen | `gh issue close <n>` / `gh issue reopen <n>` | `rad issue state <id> --closed\|--open\|--solved` |
| Comment    | `gh issue comment <n> --body C` | `rad issue comment <id> --message C [--reply-to <comment-id>]` |
| Labels     | `gh issue edit <n> --add-label L --remove-label L` | `rad issue label <id> --add L` / `--delete L` |
| Assignees  | `gh issue edit <n> --add-assignee @me` / `--remove-assignee <login>` | `rad issue assign <id> --add <DID>` / `--delete <DID>` |
| Delete     | impossible via API (web UI only, repo admin) | `rad issue delete <id>` |

Radicle issue IDs are 40-hex OIDs; any unambiguous prefix works in every
command (`rad issue list` shows 7 chars). Use `-r <RID>` (rad) or
`--repo owner/name` (gh) to operate outside the working copy.

## Mirror pairing

A footer line in BOTH issue bodies is the pairing record:

    ---
    Mirrored: gh#76 | rad:3d5afee6f4e16707c5ebe5ac7c1b5d24aa738ccf

To find an issue's counterpart, read its body and parse the `Mirrored:` line.

## Todos (third medium)

If the repo has a `todos/` directory of markdown roadmap files, they are the
third medium of the mirror. Todos are SIBLINGS of their issues, not copies:
the todo is the internal, prioritized design note (Context / Proposal /
Status, editorial voice); the issues are the public tracker records. Never
copy issue bodies verbatim into todos, or todo prose into issues — flow
summarized state only.

Pairing chain (any one medium reaches the other two):

- todo → gh: the blockquote header line —
  `> **Priority NN · Tier N — tag:** rationale. Tracked in [#N](<gh url>) (mirrored: `rad:<oid-prefix>`).`
- gh → rad: the `Mirrored:` footer in the issue bodies (see Mirror pairing).
- gh → todo (optional): a `Refs:` line in the issue body naming the todo
  file; keep it correct if the todo is renamed.
- label: todo-tied issue pairs carry the `todos` label on BOTH trackers
  (create it on gh first if missing:
  `gh label create todos --color "#d4c5f9" --description "Tracked in a todos/ roadmap file in the repo"`).

Sync rules (mirror mode):

- Create: after creating the issue pair, if a todo for the topic exists,
  extend its `Tracked in` line with ` (mirrored: `rad:<oid>`)`. If none
  exists, OFFER to create one as `todos/NN-slug.md` (next free priority
  number, following the existing header style) — do not create unasked.
- State: closing/reopening/solving the pair APPENDS a dated line to the
  todo's `## Status` section (create the section if missing), e.g.
  `**Closed** 2026-10-02 — fixed by #77.` Never rewrite or delete earlier
  status lines; the file is an append-only log.
- Comments: substantive status updates on the issue pair are DISTILLED into
  the todo's Status section, prose/checkboxes the way `todos/01` summarizes
  its issue's comments. Bot comments never reach todos. Distillation is
  editorial — offer first, write on acceptance.
- Content edits: proposal/design changes made to a todo do NOT auto-flow to
  the issues; offer to update the issue bodies when the change is substantive.
- Todos with no `Tracked in` line are roadmap-only: leave them alone unless
  asked to promote one to an issue pair.

## Create (mirror)

1. `gh issue create --title T --body B` → capture number N from the printed URL
2. `rad issue open --title T --description B` → capture the OID
3. Append the footer to both bodies:
   - `gh issue edit N --body "B\n\n---\nMirrored: gh#N | rad:OID"`
   - `rad issue edit <oid> --description "B\n\n---\nMirrored: gh#N | rad:OID"`
4. Link or offer the todo: if a `todos/` file for the topic exists, extend
   its `Tracked in` line with the rad OID; if none exists, offer to create
   one (see Todos (third medium)).

## Backfill (give an existing issue its counterpart)

An offered action: whenever a combined view (see Read (mirror)) shows unpaired
issues, offer to backfill them; run it when the user accepts or asks directly.
When one side predates mirroring, backfill it:

1. Fetch the existing side fully (`gh issue view N --json body,labels,comments`
   or `rad issue show <id>`); save the body to a file to avoid shell-quoting
   pitfalls with large markdown.
2. Create the counterpart with the SAME title/body → capture its ID.
   Capture a rad OID with `rad issue open … | grep -oE '[0-9a-f]{40}' | head -1`
   (the `Issue` line precedes the description, so the first 40-hex match is safe).
3. Append the `Mirrored:` footer to both bodies — one shared body file feeds
   `gh issue edit N --body-file` and `rad issue edit <oid> -d "$(cat file)"`.
4. Copy labels, then comments TO THE NEW COUNTERPART ONLY — the original
   already has them by definition, so a backfill never posts comments to the
   original side and double-posting is impossible by construction.
   Guard mechanically: note the original's comment count before starting;
   when done, the counterpart must have exactly that many comments and the
   original's count must be unchanged.
5. Mirror state LAST (closed/solved), after bodies, labels and comments are
   in place.
6. Do NOT mirror bot-managed GitHub issues (Renovate Dependency Dashboard,
   stale bot reports): their checkboxes and updates act through the GitHub API
   only, so a copy is dead weight that instantly drifts. Skip it and say so.
   Convention: bot-managed issues AND bot PRs (all Renovate dependency
   updates, any state) carry the `renovate` label on GitHub only —
   `gh label create renovate --color "#39d353" --description "Renovate bot: dependency dashboard and dependency-update PRs (GitHub-only, never mirrored)"`;
   apply it retroactively to past bot PRs so `label:renovate` is a complete
   provenance query (`gh pr edit <n> --add-label renovate`; works on merged
   and closed PRs). Never on Radicle.
7. Deleting a GitHub comment requires its GraphQL node ID (`IC_…`), not a REST
   numeric id: `gh api graphql -f query='mutation($id: ID!) { deleteIssueComment(input: {id: $id}) { clientMutationId } }' -f id=IC_…`.
   Useful for removing accidentally duplicated backfill comments.

## Update (mirror)

- Title/body: apply identical `gh issue edit` + `rad issue edit` (keep the
  footer in the body).
- State: `gh issue close N` + `rad issue state <oid> --closed`, plus a dated
  line in the paired todo's Status section (see Todos (third medium)).
  `solved` is
  Radicle-only — use `--closed` when mirroring unless the user explicitly says
  solved. Reopen: `gh issue reopen N` + `rad issue state <oid> --open`.
- Comments: same content on both (`--body` / `--message`). `--reply-to` threads
  a Radicle reply; GitHub has no comment threading.
- Labels: same labels on both. GitHub labels must exist first — create with
  `gh label create L --color <hex> --description "D"`. Radicle labels are
  freeform strings.
- Assignees: GitHub uses logins (`@me` for self); Radicle uses DIDs
  (`rad self --did` for self; ask for a collaborator's DID).

## Read (mirror)

When asked to list or summarize issues in a mirror repo, fetch both
(`gh issue list --limit N` and `rad issue list [--all]`), pair them by their
`Mirrored:` footers and their todos' `Tracked in` lines, and present one
combined view (gh#N | rad:OID | todos/NN-slug.md), flagging unpaired issues.
If unpaired issues exist (and are not bot-managed, see Backfill rule 6),
end the summary by offering to backfill them — do not run it unasked.

## Poll (fetch new comments/activity)

When asked "any new comments?", "did anyone reply?", "check for activity",
"poll the issues": read both trackers, present what's new, change nothing.

1. **Radicle:** fresh peer data arrives only via sync, so run `rad sync`
   first (node must be running; if stopped, report local-storage state as of
   the last sync and say so). Then:
   - Repo-wide: `rad inbox` (repo-scoped in a working copy; `rad inbox --all`
     covers every repo). Entries carry a notification number, the object ID,
     and type (issue/patch). `rad inbox show <n>` displays the item AND marks
     it read — only `show` items you are presenting. `rad inbox clear` deletes
     notifications without reading.
   - One issue: `rad issue show <id>` and compare against what was seen before.
   - Own activity does not notify: comments the user/skill posted locally
   appear in `rad issue show`, not the inbox.
2. **GitHub:** repo-wide by time —
   `gh api "repos/<OWNER>/<REPO>/issues/comments?since=<ISO8601>&per_page=100" --jq '.[] | {issue: (.issue_url | capture("(?<n>[0-9]+)$").n), author: .user.login, updated: .updated_at, body: .body}'`
   — returns comments created OR edited since T (verified shape). One issue:
   `gh issue view <n> --json comments`. Derive OWNER/REPO from the remote.
3. **Window:** use the user's stated time ("since Monday"); else the last
   poll time if one happened this session; else default to 24 hours ago.
4. **Present:** group by issue (paired via footers), oldest→newest, with
   author, time, tracker. If the issue has a paired todo, note whether its
   Status section is behind the issue's comments. **Dedupe mirrored comments:** a comment the skill
   mirrored exists identically on both trackers — same body text twice is ONE
   comment (authors read differently: gh login vs rad alias); report it once,
   noting it is on both. Bot comments (renovate[bot], stale bot) have no rad
   twin and never will — show them gh-only.
5. **Follow-ups are offers, not actions:** offer to mirror a genuinely new
   unmirrored comment to its counterpart, or to reply — only act when accepted.

## Reactions

- Radicle: `rad issue react <id> --emoji 🎉 [--to <comment-id>]`
- GitHub: no native command; only if explicitly asked, use GraphQL:
  `gh issue view N --json id` then
  `gh api graphql -f query='mutation { addReaction(input: {subjectId: "ID", content: HEART}) { reaction { content } } }'`

## Radicle vs GitHub capabilities

Radicle lacks: milestones, projects/boards, due dates, text search in list
output, REST API/webhooks, cross-issue auto-linking. Radicle extras: `solved`
state, threaded replies, issue deletion, signed authorship, works fully
offline. `rad issue list` prints a box-drawn table only (no JSON) — take IDs
from its ID column.

## Failure handling

If one tracker fails (gh auth, network, missing label), apply the change to
the other, then report exactly which side failed and why. Never mirror
partially in silence.
