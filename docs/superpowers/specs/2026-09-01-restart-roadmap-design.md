# Restarting sapphire-ledger: server-centric, framework-backed

- **Scope**: all five crates, plus one new crate `sapphire-ledger-server`
- **Supersedes**: parts of [`docs/design.md`](../../design.md) — see "What changed since May"
- **Goal line**: an AI assistant that can read and write the ledger, nag me
  when I stop recording, and send a weekly summary — with every AI write
  reviewable and correctable from my own clients

## Why now

The project stopped on 2026-05-22 with the data model done and nothing on top
of it. The motivation to restart is concrete: money went wrong because nothing
was being recorded and nothing was watching. So the target is not "a complete
double-entry system" — it is **a loop that closes**:

1. I tell the agent what I spent, it records it.
2. The agent notices when I stop recording, and says so.
3. The agent sends me a weekly summary.
4. Anything the agent got wrong, I fix from a client, and the fix propagates.

Step 4 is not optional. An AI that writes to a ledger nobody reviews is how the
original problem gets a second, quieter form.

## What changed since May

Three assumptions in [`docs/design.md`](../../design.md) went stale while the
project was idle. They are corrected here; `design.md` itself should be updated
once this lands.

**1. The SQLite cache plan is now backwards.** `docs/design.md` -> "Cache
strategy" and issue #1 specify a `rusqlite` cache at
`.sapphire-ledger/cache.sqlite`. In the meantime `sapphire-framework` *deleted*
SQLite support outright and moved to redb + tantivy, because `libsqlite3-sys`
carries `links = "sqlite3"` and Cargo checks that uniqueness even for
feature-disabled optional dependencies — no single rusqlite version satisfied
both matrix-sdk (agent) and grain-id (journal). Writing ledger's own rusqlite
cache would walk back into the wall the framework just climbed out of.

**2. The MCP template moved.** Issues #2 and #3 name
[fluo10/sapphire-journal#229](https://github.com/fluo10/sapphire-journal/pull/229)
as the reference. The *shape* of `sapphire-journal-mcp` is still the right
template (`server.rs`, `http.rs`), but `sapphire-journal-core` now sits on
`sapphire-framework-workspace` and `sapphire-framework-track`. The layer beneath
the template is different from what #2/#3 assumed.

**3. `sapphire-ledger` is the only app not on the framework.** journal and agent
both consume it. Staying off it means re-implementing workspace discovery, mtime
tracking, search, sync, and blob storage that already exist.

Minor: `docs/design.md` still says "caretta-id". The crate is published as
[`grain-id`](https://crates.io/crates/grain-id).

## Decisions

- **Adopt `sapphire-framework` from the start**, rather than after an
  independent MVP. TOML is already an indexed extension
  (`crates/sapphire-framework-workspace/src/indexer.rs:23`) and `IndexHook`
  (`on_changed` / `on_removed` / `after_sweep`) is exactly the extension point
  ledger needs. `sapphire-journal-core` already consumes it this way.
- **Defer the ledger-specific cache.** Framework `track` (mtime deltas) and
  `retrieve` (full-text) come for free. A ledger-specific index — postings by
  account, running balances — is *not* built yet. Balances come from walking the
  loaded workspace. At household scale (thousands of records) this is
  milliseconds. Issue #1 is rewritten to mean this, not rusqlite.
- **Build `sapphire-ledger-server`**, mirroring `sapphire-journal-server`: one
  binary serving `/rpc` (framework remote sync) and `/mcp` (ledger MCP over
  HTTP) behind one set of API keys. This is what makes step 4 above real.
- **Write path lives in `core`, not in the CLI.** Issue #4 is written as a CLI
  feature, but CLI, MCP, GUI and receipt import all perform the same sequence:
  mint id -> resolve canonical path -> `validate()` -> save. That sequence is
  written once in `core::ops`; the four entry points are thin.
- **Split issue #5.** The storage half (record type, paths, write op, one read
  tool) is cheaper now than later; the conversion half (`price_at`, `convert`,
  net-worth reporting) is exactly as cheap later. Take the first, defer the
  second. Rationale below.
- **HTTP is the agent's transport.** stdio stays for local development.

### Why the price-log storage half is cheaper now

`Price` — the inline per-posting price — is a public re-export
(`sapphire-ledger-core/src/lib.rs:24`). Once `sapphire-ledger-mcp` generates
tool schemas with `schemars`, that name becomes part of the published MCP
surface. Separating the inline type from the price-log record type
(`postings::Price` vs `prices::PriceEntry`) is a single rename today, before
anything references it, and a schema break after the MCP server ships.

Secondarily, `core::ops` has to handle three record kinds regardless; adding a
fourth while writing it is one more arm on a match. Retrofitting means reopening
`ops`, the CLI, MCP tool registration, `init_workspace`, and `validate`.
`PriceEntry` is structurally near-identical to `Assertion`
(`sapphire-ledger-core/src/assertion.rs`, 22 lines): id, date, small body, one
file per record under `{kind}/{year}/{MM}/{id}.toml`.

The conversion half is pure functions over already-loaded data. It adds no file
format, touches no write path, and breaks nothing — and neither reminder use
case needs it.

## Architecture

```
sapphire-ledger-core/     data model + TOML I/O + validation + core::ops (write)
                          depends on sapphire-framework-{workspace,track}
                          implements IndexHook
sapphire-ledger-mcp/      MCP server as a LIBRARY. stdio by default,
                          `http-server` feature for the HTTP transport
sapphire-ledger-cli/      `sapphire-ledger` binary: init / check / mcp
                          (`add` subcommands deferred — see Deferred)
sapphire-ledger-server/   NEW. /rpc + /mcp behind shared API keys
sapphire-ledger-desktop/  egui, via sapphire-framework-backend (local or remote)
```

### Data flow

The TOML files remain the source of truth. The server holds a workspace, serves
deltas over `/rpc`, and exposes the ledger over `/mcp`. Critically — following
`sapphire-journal-server/src/serve.rs` — **MCP writes are fed into the change
log through an observer**, so an agent write is visible to every synced client
immediately, and a human correction from any client syncs back the same way.
That is the review loop, and it falls out of the composition rather than needing
its own mechanism.

`sapphire-agent` connects as an outbound MCP client:

```toml
[[tools.mcp_servers]]
name    = "ledger"
type    = "http"
url     = "http://<host>:<port>/mcp"
api_key = "<key from `sapphire-ledger-server gen-key`>"
```

Its tools then appear to the agent as `mcp__ledger__*`. Note this is unrelated
to `[tools.host_access]`, which gates the agent's *own* filesystem and shell
tools and stays `false`.

Reminders need no new agent code. `<workspace>/heartbeat/*.md` files carry a
cron `schedule` and a `room_id` in YAML frontmatter, and the body is used
verbatim as the prompt. Two files cover both requested behaviours.

## Phase 1 — the self-hosting line

Claude Code access ends in early September 2026 (see the agent's own roadmap).
Phase 1 is scoped as *what must exist before the agent can be the tool that
builds the rest*, not as everything worth building.

1. **`core::ops` write API.** Mint ids via `grain-id`; resolve canonical paths;
   `validate()` before save; refuse to overwrite. Covers accounts,
   transactions, assertions, and price entries. Includes the
   `Price` / `PriceEntry` split.
2. **Framework migration.** `core` depends on `sapphire-framework-workspace`
   and `-track`; implement `IndexHook`; drop `rusqlite` from the workspace
   manifest. Mirror journal's feature chain, where each crate forwards
   `redb-store` down to the framework and the top-level binaries enable it by
   default — the framework warns that an app which leaves it off silently falls
   back to a volatile in-memory store.
3. **`sapphire-ledger-mcp`.** Read tools `list_accounts`, `get_transaction`,
   `query_postings`, `validate_workspace`, `query_prices`; write tools
   `add_transaction`, `add_account`, `add_assertion`. `http-server` feature
   gated as in `sapphire-journal-mcp`.
4. **`sapphire-ledger-server`.** `/rpc` + `/mcp`, shared key protection, Host
   header checks, `gen-key` / `list-keys` / `rotate-key` / `revoke-key`.
5. **Agent wiring.** One `[[tools.mcp_servers]]` entry and two heartbeat tasks:
   a recording-gap nag and a weekly summary.

**If time runs out**, steps 1-3 are the minimum that closes the loop: the agent
can reach a stdio MCP server without step 4, and reminders still work. Step 4 is
what makes human review comfortable, so it should not be dropped lightly — but
it is droppable, and steps 1-3 are not.

## Deferred

Recorded so the deferral is intentional, not lost. Most of this is expected to
be built *by* the agent once phase 1 lands.

- CLI `add` subcommands — thin wrappers over `core::ops` (issue #4's CLI half)
- Desktop GUI beyond the current 27-line scaffold
- Receipt photo import, via `sapphire-framework-blob` (issue #6)
- Price conversion and base-currency reporting (issue #5's second half)
- A ledger-specific index for balances, if walking ever gets slow (issue #1)
- `cargo-deny` (issue #8)
- Everything else already in issue #6

Explicitly **not** planned, because the chosen reminders do not need them:
budgets, recurring transaction templates, and spending alerts. If the reminder
set changes, these come back.

## Issue disposition

| Issue | Disposition |
|---|---|
| #1 SQLite cache | **Rewrite.** No rusqlite. Means "framework-backed indexing now, ledger-specific balance index later." |
| #2 MCP stdio | **Keep**, retargeted at the current `sapphire-journal-mcp`, and widened to include write tools. |
| #3 Desktop HTTP MCP | **Absorbed** into `sapphire-ledger-server`. Desktop gets HTTP MCP later, if ever — the server is the better host for it. |
| #4 CLI write commands | **Split.** `core::ops` + grain-id into phase 1; the CLI surface deferred. |
| #5 Price log | **Split.** Storage half into phase 1; conversion half deferred. |
| #6 Phase 2 backlog | **Keep.** Receipt attachments are now concrete via `sapphire-framework-blob`. |
| #8 cargo-deny | **Keep**, deferred. |

New issue needed: `sapphire-ledger-server`.

## Risks

- **The framework is a moving git dependency**, not a published crate; journal
  pins `branch = "main"`. Ledger inherits that churn. Mitigation: none beyond
  pinning a commit if it becomes disruptive.
- **Phase 1 is not small** relative to the remaining Claude Code window. The
  1-3 fallback exists for that reason and should be invoked early rather than
  discovered late.
- **Server exposure.** `sapphire-journal-server` warns that binding beyond
  loopback without `--allowed-host` leaves `/rpc` answering any client. The same
  discipline applies here, and a ledger is a more sensitive target than a
  journal.
