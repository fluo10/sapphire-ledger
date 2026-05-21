# Design

This document captures the locked-in design decisions for sapphire-ledger.
It is intended as a primer for contributors (human or AI) picking up the
project. The high-level concept lives in [`README.md`](../README.md); this
file goes deeper into the *why* and the on-disk shape.

## Goals

- **Local-first** household ledger using **double-entry bookkeeping**.
- **Plain-text source of truth** so a regular git workflow (or Syncthing,
  etc.) can carry the data between machines and between collaborators.
- **AI-collaborative**: a human and an AI agent (Claude, etc.) can edit the
  same ledger concurrently. Conflict surface must stay tiny.
- **Strict integrity**: a corrupt journal is worse than a slow one, so
  validation always errs on the side of rejecting bad data.

## Format: TOML

Every record on disk is a single TOML file. TOML was chosen over the
obvious alternatives for these reasons:

| Format | Verdict |
|---|---|
| YAML | Indentation-sensitive; AI agents often produce broken YAML; `serde_yaml` is effectively frozen. |
| JSON / JSONL | No comments; JSONL doesn't match "one file per record". |
| Beancount | Excellent semantics but its tooling assumes many entries per file, which is the opposite of what we want for git-merge tolerance. |
| **TOML** | Line-oriented (small, local merge conflicts), Rust ecosystem is first-class, AI writes it reliably, supports comments, already used elsewhere in the `sapphire-*` workspace. |

Decimal values (amounts, exchange rates) are persisted as **strings**
(`"1200"`, `"150.5"`) because TOML has no native decimal type — we parse
them with `rust_decimal::Decimal` via `#[serde(with = "rust_decimal::serde::str")]`.

## File granularity: one record, one file

The fundamental rule is **one record = one file**. Two people (or a person
and an AI) editing different receipts touch different files, so git merges
become trivial.

```
my-ledger/
├── .sapphire-ledger/
│   ├── config.toml          # workspace config (git-tracked)
│   ├── .gitignore           # ignores cache.sqlite
│   └── cache.sqlite         # local SQLite cache (planned, gitignored)
├── accounts/
│   └── {Type}/.../{Leaf}.toml
├── transactions/
│   └── {year}/{MM}/{caretta-id}.toml
└── assertions/
    └── {year}/{MM}/{caretta-id}.toml
```

A workspace is any directory that contains a `.sapphire-ledger/`
sub-directory, located by walking upward from the current directory — the
same convention `git` uses.

### Account hierarchy as directory hierarchy

Account names are colon-separated (`Assets:Cash:USD`). On disk this maps
directly to a directory tree under `accounts/`:

| Account name | File path |
|---|---|
| `Equity` | `accounts/Equity.toml` |
| `Assets:Cash:JPY` | `accounts/Assets/Cash/JPY.toml` |
| `Assets:Cash:USD` | `accounts/Assets/Cash/USD.toml` |

Path conversion functions (`account_relative_path`,
`account_name_from_relative_path`) live in
[`workspace.rs`](../sapphire-ledger-core/src/workspace.rs).

Account name validation rejects:

- Empty names or empty segments (`""`, `Assets::Cash`).
- `.` or `..` segments (path traversal).
- Segments containing `/` or `\`.

### Transaction and assertion paths

Both follow `{kind}/{year}/{MM}/{caretta-id}.toml`. The caretta-id is a
7-character BASE32 identifier with decisecond precision; two records
created more than 0.1s apart are guaranteed to have different IDs, which
removes the need for a central ID coordinator under concurrent edits.
(ID generation is not yet integrated — see open follow-ups.)

The ID is stored both as the filename and as a field inside the file
(`id = "0a1b2c3"`) so a record survives being moved or copied.

## Data model

All struct definitions live in [`sapphire-ledger-core/src/`](../sapphire-ledger-core/src/).
This section summarizes the shapes; the source is authoritative.

### Account ([`account.rs`](../sapphire-ledger-core/src/account.rs))

```toml
name = "Assets:Cash:USD"
type = "Asset"               # Asset | Liability | Equity | Income | Expense
currencies = ["USD"]         # empty/omitted = any currency allowed
opened_at = "2026-05-21"
# closed_at = "..."          # optional
# description = "ドル現金"   # optional
```

`Account::allows_currency(&str)` returns `true` if `currencies` is empty
or contains the requested currency. The validator uses this to flag
postings that violate an account's currency constraint.

### Transaction ([`transaction.rs`](../sapphire-ledger-core/src/transaction.rs))

```toml
id = "0a1b2c3"
date = "2026-05-21"
narration = "イオン買い物"
payee = "イオン"               # optional
tags = ["grocery"]             # optional
status = "cleared"             # optional: cleared | pending
created_at = "2026-05-21T18:30:00+09:00"
updated_at = "2026-05-21T18:30:00+09:00"

[[postings]]
account = "Expenses:Food"
amount = "1200"                # string-encoded Decimal
currency = "JPY"
# memo = "..."                 # optional, per-posting
# price = { value = "150", currency = "JPY" }   # optional, see Multi-currency

[[postings]]
account = "Liabilities:CreditCard:Rakuten"
amount = "-1200"
currency = "JPY"
```

`Transaction::validate()` enforces:

1. At least 2 postings.
2. Per-currency net contribution is zero, with each posting's
   contribution computed via `Posting::balance_contribution()` (which
   applies the inline `price` conversion when present — see below).

### Multi-currency

A cross-currency transaction uses Beancount-style inline prices:

```toml
# Move 15,000 JPY into a USD cash account at a rate of 150 JPY/USD.
[[postings]]
account = "Assets:Cash:USD"
amount = "100"
currency = "USD"
price = { value = "150", currency = "JPY" }   # 1 USD = 150 JPY

[[postings]]
account = "Assets:Cash:JPY"
amount = "-15000"
currency = "JPY"
```

`Posting::balance_contribution()` returns `(currency, signed_amount)`:

- No price set → `(self.currency, self.amount)` as-is.
- Price set → `(price.currency, self.amount * price.value)`.

The transaction balances if every resulting currency totals zero. In the
example above, both sides contribute to JPY and net to zero.

**Out of scope for now**: lot tracking, unrealized FX P&L, separate
price-log files for historical FX rates. See the open follow-ups.

### Assertion ([`assertion.rs`](../sapphire-ledger-core/src/assertion.rs))

A balance assertion declares the expected balance of an account at the end
of a given date (after all transactions on that date — hledger semantics,
not Beancount's before-the-date semantics).

```toml
id = "as0001"
account = "Assets:Brokerage"
date = "2026-05-31"
created_at = "2026-05-31T23:59:00+09:00"
updated_at = "2026-05-31T23:59:00+09:00"

[[balances]]
amount = "100"
currency = "USD"

[[balances]]
amount = "5000"
currency = "JPY"
```

One assertion file declares one account's expected balances at one date,
optionally across multiple currencies. Failure to match is a hard error
(no "warn and continue" mode). There is no Beancount-style `pad`
auto-balancing — mismatches must be fixed manually.

Subtree assertions ("`Assets` and all descendants total X") are not
supported in MVP. Leaf accounts only.

### Opening balances

When a new account begins with a non-zero balance, that's expressed as a
regular transaction posting against `Equity:OpeningBalances`. Assertions
are reserved for verification, not for declaring initial state. This
matches Beancount and hledger conventions.

### Workspace config ([`config.rs`](../sapphire-ledger-core/src/config.rs))

```toml
schema_version = 1
base_currency = "JPY"     # used for FX-converted reporting (planned)

[cache]
scan_interval = 60         # SQLite cache rescan interval in seconds
```

`base_currency` will drive net-worth / FX-converted views once the price
log lands. Without it, only per-currency views are possible.

## Validation pipeline

Validation runs at two layers.

**Per-record**, on parse:

- `Transaction::validate()` — balance + posting count.
- Account-name shape (`account_name_segments`).

**Cross-record**, against the whole workspace
([`validate.rs`](../sapphire-ledger-core/src/validate.rs)):

`Workspace::validate()` returns `Vec<ValidationIssue>` and never
short-circuits — the user gets every issue in one pass. Issues currently
detected:

- Transaction fails per-record validation (balance, posting count).
- Posting references an account that doesn't exist in `accounts/`.
- Posting currency violates the account's `currencies` constraint.
- Assertion references an undefined account.
- Assertion balance currency violates the account's `currencies`
  constraint.

`ValidationIssue` carries optional `transaction_id`, `assertion_id`, and
`account` fields so MCP tools can return structured reports later.

**Not yet implemented** (see issues):

- Balance assertions actually compared against historical posting sums.
- Date ordering / future-date sanity.
- Account `opened_at` / `closed_at` enforcement against posting dates.

## Cache strategy

The plan (not yet built) is a hybrid:

1. **Event-driven**: in-process edits update the SQLite cache directly.
2. **Periodic mtime scan**: catches external changes from `git pull`,
   Syncthing, manual editor saves, etc.

The cache database lives at `.sapphire-ledger/cache.sqlite` and is
gitignored. Initial sketch of tables: `transactions`, `postings`,
`accounts`, `assertions`, `file_index (path, mtime, content_hash)`.

## Crate structure

```
sapphire-ledger/
├── sapphire-ledger-core/      # data model, TOML I/O, validation
├── sapphire-ledger-mcp/       # MCP server logic — LIBRARY only
├── sapphire-ledger-cli/       # `sapphire-ledger` binary, embeds stdio MCP
└── sapphire-ledger-desktop/   # egui GUI, embeds opt-in HTTP MCP
```

The MCP crate is intentionally **a library, not a binary**. Both the CLI
(stdio transport) and the Desktop GUI (HTTP transport on loopback) embed
the same server logic, so a single install gives both human and agent
interfaces. See the [MCP server](#mcp-server) section for the details.

Mobile builds (potentially with Dioxus) and a VS Code extension are
deferred — see open follow-ups.

## MCP server

Aligned with the sapphire-journal pattern introduced in
[fluo10/sapphire-journal#229](https://github.com/fluo10/sapphire-journal/pull/229),
which is the template implementation to mirror.

### Crate is library-only

`sapphire-ledger-mcp` exposes the MCP server as reusable code. It does
**not** produce a binary. There is no `sapphire-ledger-mcp` CLI to
install separately.

### CLI: stdio transport via `sapphire-ledger mcp`

The CLI gains a `Mcp` subcommand that hands control to the library:

```rust
Command::Mcp { init } => sapphire_ledger_mcp::run(cli.ledger_dir.as_deref(), init)?,
```

`sapphire-ledger mcp [--init] [--ledger-dir DIR]` speaks the MCP protocol over
stdio so it can be wired directly into Claude Desktop / Claude Code as
an `mcp__sapphire-ledger__*` server. `--init` lets the agent create a
fresh workspace if the target directory isn't one yet (no-op when it
already is).

### Desktop: opt-in HTTP transport

The HTTP path is gated behind an **`http-server` cargo feature on the
mcp crate** that pulls in `rmcp/transport-streamable-http-server` and
`axum`. The default build (used by the CLI) stays stdio-only and avoids
the axum dependency tree.

When the desktop binary enables that feature, it can run an in-process
HTTP MCP server at `http://127.0.0.1:<port>/mcp` whenever a ledger is
open. The port lives in the user's settings and the server is
reconciled against (ledger-is-open, feature-enabled, port) every
frame, so it starts, stops, and restarts automatically as the user
toggles the setting, changes the port, or switches workspaces.

**Loopback only.** The server binds to `127.0.0.1` exclusively. Exposing
the ledger beyond loopback would require an auth layer (token /
API-key) that is explicitly out of scope for MVP. Adding that is the
prerequisite for ever binding to `0.0.0.0`.

### Library API shape

Mirroring the journal:

- A public `SapphireLedgerServer` type implementing the rmcp server
  trait.
- `SapphireLedgerServer::from_shared(state)` constructor so multiple
  concurrent HTTP sessions can share a single in-memory workspace
  state without each rebuilding it.
- Shared setup helpers (the journal calls them `prepare_state`,
  `spawn_periodic_git_sync`) factored out of the stdio entry point so
  both transports reuse them — the only divergence between stdio and
  HTTP should be the rmcp transport wiring itself.

## Licensing

| Crate | License |
|---|---|
| `sapphire-ledger-core` | MIT OR Apache-2.0 |
| `sapphire-ledger-mcp`  | MIT OR Apache-2.0 |
| `sapphire-ledger-cli`  | MIT OR Apache-2.0 |
| `sapphire-ledger-desktop` | MIT OR Apache-2.0 (initial) |

Permissive everywhere for MVP. If a desktop or mobile build is eventually
distributed through an official store (Mac App Store, Microsoft Store,
iOS/Android), the corresponding crate may switch to GPL-3.0-or-later for
that distribution channel. The reverse direction (GPL → permissive) is
hard to undo without contributor consent, so the default stays
permissive until there is a reason to tighten it.

## Status

- ✅ Workspace scaffold (`core`/`mcp`/`cli`/`desktop`), `cargo build` clean.
- ✅ Data model (accounts, transactions, postings, prices, assertions, config).
- ✅ TOML round-trip with serde.
- ✅ Path conventions, workspace discovery, `init_workspace`.
- ✅ Repository I/O (`load_toml`, `save_toml`, `walk_toml_files`, `load_workspace`).
- ✅ Cross-record validation + `sapphire-ledger check`.
- 🚧 SQLite cache — designed, not implemented.
- 🚧 MCP server logic — empty library stub.
- 🚧 CLI write commands (`sapphire-ledger account add`, `sapphire-ledger tx add`).
- 🚧 caretta-id / grain-id integration for ID generation.
- 🚧 Price log (FX rate timeseries).
- 🚧 Phase 2: subtree assertions, `pad`, attachments, recurring templates, VS Code extension, mobile.

See [GitHub issues](https://github.com/fluo10/sapphire-ledger/issues) for
follow-up work.
