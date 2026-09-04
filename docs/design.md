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

The fundamental rule is **one record = one file** — a transaction, an
account, an assertion, or a price-log entry. Two people (or a person and an
AI) editing different records touch different files, so git merges become
trivial.

```
my-ledger/
├── .sapphire-ledger/
│   ├── config.toml          # workspace config (git-tracked)
│   └── .gitignore           # vestigial: still lists cache.sqlite
├── accounts/
│   └── {Type}/.../{Leaf}.toml
├── transactions/
│   └── {year}/{MM}/{grain-id}.toml
├── assertions/
│   └── {year}/{MM}/{grain-id}.toml
└── prices/
    └── {year}/{MM}/{grain-id}.toml
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

### Transaction, assertion and price-log paths

All three follow `{kind}/{year}/{MM}/{grain-id}.toml`. The id is stored both
as the filename and as a field inside the file (`id = "0a1b2c3"`) so a record
survives being moved or copied.

Ids come from [`grain-id`](https://crates.io/crates/grain-id). Records whose id
is their filename — transactions, assertions, prices — mint with
`GrainId::now_unix()` on the first attempt, whose decisecond resolution makes a
`{year}/{MM}/` listing time-ordered. Uniqueness is checked across the whole
record kind — every id already in use under `transactions/`, `assertions/` or
`prices/` — not merely against the destination path: two records minted in one
decisecond but dated in different months land in different `{year}/{MM}/`
directories, so a per-path check alone would let both keep the same id. A
collision therefore means another record of the same kind was minted in the
same decisecond; `now_unix()` is a pure function of that decisecond, so
re-minting it would return the same id forever, and `core::ops` mints
`GrainId::random()` on every retry instead. Uniqueness is the hard requirement
and ordering is the nicety, so ordering is what gives way — and inside a burst
it was unobtainable anyway. The `date` field inside each record remains the real
ordering key.

**Accounts use `GrainId::random()` from the start**: an account's id is not in
its path, so it does no ordering work, and a chart of accounts created in one
sitting would otherwise share a long leading prefix — the opposite of what CLI
completion wants. Because an account's id is not its filename, the
refuse-to-overwrite check cannot see an id collision, so `create_account`
checks the minted id against the accounts already on disk.

## Data model

All struct definitions live in [`sapphire-ledger-core/src/`](../sapphire-ledger-core/src/).
This section summarizes the shapes; the source is authoritative.

### Account references

`Account` carries an `id`. Postings and assertions reference an account by
`account_id`, with a denormalized `account_name` beside it:

```toml
[[postings]]
account_id   = "0a1b2c3"        # authoritative; survives a rename
account_name = "Expenses:Food"  # for whoever reads the raw file
```

The id is what links; the name is never used for matching when the id is
present. This is what makes renaming an account cheap: the account file
changes and nothing else has to.

The alternative — name-only references plus a rename operation that rewrites
every transaction — was rejected because that rewrite spans many files while
remote sync resolves conflicts per path, last-writer-wins. A client adding a
transaction under the old name mid-rename would leave a record pointing at
nothing.

Three rules follow:

1. **A stale `account_name` is not an error.** Older records keep the old name
   until rewritten for some other reason. Flagging it would reintroduce the
   whole-history rewrite this design avoids.
2. **Duplicate account ids *are* an error.** An account's path comes from its
   name, so a rename is a file move; if that races under sync, one id can land
   at two paths.
3. **Either field alone is accepted on read.** Both present means the id wins;
   name-only is resolved to an id when the record is written; neither is an
   error. Hand-written TOML must not require an id lookup first.

### Account ([`account.rs`](../sapphire-ledger-core/src/account.rs))

```toml
id = "a3k9m2p"
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
account_id   = "0f2e9a1"       # authoritative; account_name is denormalized
account_name = "Expenses:Food"
amount = "1200"                # string-encoded Decimal
currency = "JPY"
# memo = "..."                 # optional, per-posting
# price = { value = "150", currency = "JPY" }   # optional, see Multi-currency

[[postings]]
account_id   = "7c4d8b0"
account_name = "Liabilities:CreditCard:Rakuten"
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
account_id   = "9k3n7x2"
account_name = "Assets:Cash:USD"
amount = "100"
currency = "USD"
price = { value = "150", currency = "JPY" }   # 1 USD = 150 JPY

[[postings]]
account_id   = "2m5p8q1"
account_name = "Assets:Cash:JPY"
amount = "-15000"
currency = "JPY"
```

`Posting::balance_contribution()` returns `(currency, signed_amount)`:

- No price set → `(self.currency, self.amount)` as-is.
- Price set → `(price.currency, self.amount * price.value)`.

The transaction balances if every resulting currency totals zero. In the
example above, both sides contribute to JPY and net to zero.

**Out of scope for now**: lot tracking, unrealized FX P&L, and converting a
stored rate back into a balance. The price log itself is stored — see
[Price log](#price-log-pricesrs) below — but nothing yet reads it back to
convert. See the open follow-ups.

### Assertion ([`assertion.rs`](../sapphire-ledger-core/src/assertion.rs))

A balance assertion declares the expected balance of an account at the end
of a given date (after all transactions on that date — hledger semantics,
not Beancount's before-the-date semantics).

```toml
id = "b7f3n0q"
account_id   = "4h6j2k9"       # authoritative; account_name is denormalized
account_name = "Assets:Brokerage"
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

### Price log ([`prices.rs`](../sapphire-ledger-core/src/prices.rs))

A standalone record of one observed exchange rate, independent of any
transaction: `1 base = rate quote` on `date`. Stored under
`prices/{year}/{MM}/{grain-id}.toml`, the same layout as transactions and
assertions.

```toml
id = "0a1b2c3"
date = "2026-05-21"
base = "USD"
quote = "JPY"
rate = "150.5"
source = "manual"              # optional
created_at = "2026-05-21T18:30:00+09:00"
updated_at = "2026-05-21T18:30:00+09:00"
```

This is storage only. `sapphire-ledger` can record a rate, but nothing yet
reads the price log back to convert a balance into `base_currency` or build a
net-worth report — that's the conversion half deferred out of Multi-currency
above.

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
scan_interval = 60         # reserved for a future mtime-rescan consumer; unread today
```

`base_currency` will drive net-worth / FX-converted views once price
*conversion* is built on top of the price log (see [Price
log](#price-log-pricesrs)). Without it, only per-currency views are possible.

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

- Two accounts share an `id` (a rename that raced under sync).
- Two accounts share a `name`. A name-only posting would otherwise resolve
  to whichever of them won the lookup table.
- Two transactions, two assertions or two price entries share an `id`. Ids
  are unique per record kind, not per `{year}/{MM}/` directory; a record
  whose date was corrected moves between directories, and that move is a
  delete-plus-add that can race under sync.
- Transaction fails per-record validation (balance, posting count).
- Posting references an account that doesn't exist in `accounts/`.
- Posting or assertion carries no account reference at all (neither
  `account_id` nor `account_name`).
- Posting currency violates the account's `currencies` constraint.
- Assertion references an undefined account.
- Assertion balance currency violates the account's `currencies`
  constraint.

`ValidationIssue` carries optional `transaction_id`, `assertion_id`, and
`account` fields so MCP tools (`validate_workspace`, and `sapphire-ledger
check` on the CLI) can return structured reports.

**Not yet implemented** (see issues):

- Balance assertions actually compared against historical posting sums.
- Date ordering / future-date sanity.
- Account `opened_at` / `closed_at` enforcement against posting dates.

## Cache strategy

There is deliberately **no ledger-specific cache**. `load_workspace` walks the
TOML files on every load, which is milliseconds at household scale.

The framework supplies mtime tracking (`sapphire-framework-track`) and search
(`sapphire-framework-retrieve`) when a consumer needs them; neither is wired
into any crate here yet — the `[cache]` section of the workspace config
(`scan_interval`) is reserved for whichever of them ends up consuming it. A
ledger-specific index — postings by account, running balances — is deferred
until walking is measurably slow. See issue #1.

`rusqlite` is not a dependency of this workspace, and no crate here adds one.
`grain-id` ships an optional `rusqlite` feature — off by default, and not
enabled by anything in this workspace — that only adds `ToSql`/`FromSql` impls
for `GrainId`; it is not a caching mechanism and does not pull rusqlite into
the build unless a crate here explicitly turns the feature on, so `Cargo.lock`
currently has no rusqlite entry at all. A SQLite-backed cache is a deliberate
non-goal here, not a stopgap for a slow walk that hasn't been measured yet.

## Crate structure

```
sapphire-ledger/
├── sapphire-ledger-core/      # data model, TOML I/O, validation, write path
├── sapphire-ledger-mcp/       # MCP server logic — LIBRARY only
├── sapphire-ledger-cli/       # `sapphire-ledger` binary, embeds stdio MCP
├── sapphire-ledger-desktop/   # egui GUI (no MCP transport of its own)
└── sapphire-ledger-server/    # self-hosted MCP server over HTTP, per-device auth (no /rpc sync yet)
```

The MCP crate is intentionally **a library, not a binary**. The CLI embeds it
for the stdio transport; `sapphire-ledger-server` embeds it, behind its
`http-server` feature, for the HTTP transport. See the [MCP
server](#mcp-server) section for the details.

Mobile builds (potentially with Dioxus) and a VS Code extension are
deferred — see open follow-ups.

## MCP server

Modelled on `sapphire-journal-mcp` as it stands today: a server struct holding
an `Arc<Mutex<LedgerState>>`, tools declared with rmcp's `#[tool]` macro, and
a stdio entry point. The HTTP transport is `sapphire-ledger-server`, which
puts `/mcp` behind the framework's per-device authentication — not with the
desktop GUI, as this document originally planned. `/rpc` is designed to land
in the same binary behind the same keys eventually, but it is not mounted
today; see [Server: `sapphire-ledger-server`](#server-sapphire-ledger-server)
below.

### Tools

Nine tools, declared in [`server.rs`](../sapphire-ledger-mcp/src/server.rs):
five read, four write. Descriptions below are the tools' own
`#[tool(description = ...)]` text, condensed to one line each.

| Tool | Does |
|---|---|
| `list_accounts` | List every account with its id, name, type, allowed currencies and open date. The id is the stable handle — it survives a rename. |
| `get_transaction` | Show one transaction, with all of its postings, by id. |
| `query_postings` | Find postings, optionally filtered by account (id or name), currency and date range; each result carries its transaction's id, date and narration. |
| `query_prices` | Find price-log entries, optionally filtered by base, quote and date range. |
| `validate_workspace` | Re-read the ledger from disk and report every validation issue. Empty array means consistent; a stale `account_name` alone is NOT an issue. |
| `add_account` | Create an account. Fails if one with that name already exists. Every account a posting names must be created first. Returns the new account's id. |
| `add_transaction` | Record a transaction. Postings must sum to zero per currency, and every account named must already exist. Nothing is written if validation fails. |
| `add_assertion` | Record a balance assertion: what an account should hold at the END of a date. A mismatch is a hard error when the books are checked. |
| `add_price` | Record an observed exchange rate in the price log: one unit of `base` costs `rate` units of `quote` on `date`. |

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

### Server: `sapphire-ledger-server`

**Built.** `sapphire-ledger-server` is a self-hosted binary that composes
`sapphire-ledger-mcp`'s `http-server` feature (`mcp_router`) with the
framework's `remote-server` and `registry` features, and serves `/mcp` behind
`protect()`. The full reasoning behind every decision below lives in
[`docs/superpowers/specs/2026-09-02-ledger-server-design.md`](superpowers/specs/2026-09-02-ledger-server-design.md);
this section covers what a reader needs so as not to be surprised.

**`/rpc` is deliberately not mounted.** The `remote-server` feature is taken
for `KeyStore` and `protect` — authentication — not for sync: the framework's
`router()` (the `/rpc` routes) is never merged into this server's
`axum::Router`. `/rpc` has no client today (the desktop crate is a scaffold,
the CLI has no sync client), so there is nothing to test a sync server
against yet. Adding it later is a `.merge()` against the same `ServerState`;
nothing here has to be undone. Concretely, this means **the review loop the
project exists for is not yet closed**: an agent can read and write over
`/mcp`, but a human still has to be on the machine holding the workspace's
files to review or correct what it wrote.

**The `Host` allowlist is a parameter, not a default.** rmcp refuses any
request whose `Host` header isn't on an allowlist, and its own default is
loopback-only. `mcp_router` takes the extra hostnames as an argument and
always adds loopback on top of them — an empty list from the caller is never
handed to rmcp, which would read that as "allow every host" instead of
"loopback only". `--addr` (default `127.0.0.1:3173`) is what widens the
bind, and `--allowed-host` (repeatable) is what widens this list. Bound
beyond loopback with no `--allowed-host` at all, `sapphire-ledger-server`
**refuses to start**: a wide bind with an empty allowlist would 403 every
request, which is useless rather than dangerous, and would present as a
client bug rather than a configuration mistake.

**TLS is out of scope.** `sapphire-ledger-server` speaks plain HTTP and has
no plans to speak anything else, so any bind past loopback puts every
device's bearer token on the wire in cleartext. Put it behind a reverse
proxy that terminates TLS, or reach it over a network that is already
encrypted (a WireGuard or Tailscale interface, say), rather than exposing
`--addr` directly.

**Clients are devices, and every device belongs to a user.** A bearer key
authenticates a device; it is not itself the unit of identity — the
framework's registry (`Devices` / `Users`) supplies that, the same way
`sapphire-agent` already manages its clients. There is no standalone key
command: `device add` registers a device and mints its key in the same act,
because a key naming no device is a key nobody can attribute. Retirement
tombstones a device rather than deleting it, so a `device_id` already written
into a record still resolves to a name afterward.

**Rotation and retirement are not live.** A running server authenticates
against a snapshot of the key file taken at start-up — `ServerState` has no
reload path — so `device rotate` and `device retire` only take effect when
the server is restarted, and until then it keeps accepting the old token.
`device rotate`, `device retire` and `device list` all say so on stderr, and
if you are retiring a device because it was compromised, restarting the
server is the step that actually cuts its access.

Two files, two homes:

- **`.sapphire-ledger/devices.toml` and `users.toml` live in the workspace.**
  They're content — who the devices and people are — and once `/rpc` exists
  they sync, which is what lets a `device_id` written on one machine resolve
  on another.
- **`keys.toml` lives in this app's host-local cache directory**, not the
  workspace. It holds secrets; a token that synced would be a token on every
  machine that ever pulled. It sits in that ledger's *own* subdirectory of
  the cache (`{cache}/{path_uuid}/keys.toml`), not the cache root, so a
  personal and a business ledger on one host do not share a credential —
  `protect()` only checks that a bearer names a key, never that the key's
  `device_id` belongs to the workspace being served, so a shared key file
  would let a device registered under one ledger authenticate in full
  against the other. Pass `--keys` to put it somewhere else.

**Duplicate ids are reported, never resolved.** A background watch re-walks
the workspace once at startup and then every five minutes, and logs any id
(or, for accounts, any name) claimed by more than one file at `WARN`, naming
the id and every path carrying it. It does not merge, delete, or re-id
anything. This matters because two of ledger's own path conventions embed
mutable data — a transaction's or assertion's path derives from its *date*,
an account's from its *name* — so a client offline across a rename or a date
correction can resurrect a stale path under sync and leave one id at two
files. A duplicated transaction is a wrong balance, so the server surfaces
the problem loudly rather than guessing at a fix.

CLI:

```
sapphire-ledger-server --ledger-dir DIR              # serve
sapphire-ledger-server init [--base-currency CCY]
sapphire-ledger-server user add <name>
sapphire-ledger-server user list
sapphire-ledger-server device add <name> --user <selector>
sapphire-ledger-server device list
sapphire-ledger-server device rotate <selector>
sapphire-ledger-server device retire <selector>
```

`--ledger-dir` (or `SAPPHIRE_LEDGER_SERVER_DIR`) is required for every one of
these, not only `serve`: the user/device registry lives under the ledger
root, so `identity::run` needs it to resolve the workspace before it can add,
list, rotate or retire anything. For `init` it names the directory to create.

`init` does the same job as `sapphire-ledger init`, offered here so that
setting up a server does not need a second binary. It stops at the workspace —
the registry and the first token come from `user add` and `device add`, which
already own that concern — and prints those two commands on its way out. A
first-time setup is therefore:

```
sapphire-ledger-server --ledger-dir ~/ledger init
sapphire-ledger-server --ledger-dir ~/ledger user add me
sapphire-ledger-server --ledger-dir ~/ledger device add laptop --user me   # token to stdout, once
sapphire-ledger-server --ledger-dir ~/ledger
```

Note the ordering constraint `init` introduces: the key file's location is
derived from the *canonicalized* ledger root, and canonicalization fails on a
directory that does not exist yet. `main` therefore resolves the key path per
command rather than once up front, so that `init` never asks for it — asking
before the directory exists would name one location during setup and a
different one for every command afterwards.

`last_updated_by` is not implemented. `Authenticated` carries an optional
`device_id`, and every key this server's CLI mints sets one — that is the
hook the field will use once it exists, but no record schema has the field
yet.

### Library API shape

Mirroring the journal:

- A public `SapphireLedgerServer` type implementing the rmcp server
  trait.
- `SapphireLedgerServer::from_shared(state)` constructor so multiple
  concurrent HTTP sessions can share a single in-memory workspace
  state without each rebuilding it.
- Shared setup helpers (`prepare_state`, mirroring the journal's naming)
  factored out of the stdio entry point so both transports reuse them — the
  only divergence in *building the ledger state* is the rmcp transport
  wiring itself. (Beyond state setup, the two paths do differ:
  `sapphire-ledger-server` hardcodes `prepare_state`'s `init` argument to
  `false` where the CLI threads `--init` through, and it additionally wraps
  the router in `protect()` and spawns the duplicate-id watch.)

## Licensing

| Crate | License |
|---|---|
| `sapphire-ledger-core` | MIT OR Apache-2.0 |
| `sapphire-ledger-mcp`  | MIT OR Apache-2.0 |
| `sapphire-ledger-cli`  | MIT OR Apache-2.0 |
| `sapphire-ledger-desktop` | MIT OR Apache-2.0 (initial) |
| `sapphire-ledger-server` | MIT OR Apache-2.0 |

Permissive everywhere for MVP. If a desktop or mobile build is eventually
distributed through an official store (Mac App Store, Microsoft Store,
iOS/Android), the corresponding crate may switch to GPL-3.0-or-later for
that distribution channel. The reverse direction (GPL → permissive) is
hard to undo without contributor consent, so the default stays
permissive until there is a reason to tighten it.

## Status

- ✅ Workspace scaffold, `cargo build` clean.
- ✅ Data model (accounts, transactions, postings, prices, assertions, config).
- ✅ TOML round-trip with serde.
- ✅ Path conventions, workspace discovery, `init_workspace`.
- ✅ Repository I/O and cross-record validation + `sapphire-ledger check`.
- ✅ Record ids everywhere, with id-linked account references.
- ✅ Price-log records (storage; conversion and reporting deferred).
- ✅ `core::ops` write path.
- ✅ Framework dependency (`AppContext`) + `LedgerState` session object.
- ✅ MCP server over stdio: read and write tools, via `sapphire-ledger mcp`.
- ✅ `sapphire-ledger-server`: `/mcp` over HTTP, authenticated per device,
  plus `user`/`device` management and a duplicate-id watch.
- 🚧 `/rpc` sync — no client exists yet, and reviewing or correcting an
  agent's write still means being on the machine holding the workspace's
  files.
- 🚧 CLI write commands.
- 🚧 Desktop GUI.
- 🚧 Price conversion / base-currency reporting.
- 🚧 Phase 2: see the issue tracker.

See [GitHub issues](https://github.com/fluo10/sapphire-ledger/issues) for
follow-up work.
