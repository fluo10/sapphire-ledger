# sapphire-ledger

> Language: **English** | [日本語](README.ja.md)

A double-entry household ledger built on [sapphire-framework](https://github.com/fluo10/sapphire-framework) — file-based, local-first, made for human-agent collaboration.

## Concept

- **TOML as source of truth** — all data lives in plain `.toml` files you can read and edit with any tool
- **No cache required** — `load_workspace` reads the TOML files directly on every load, which is fast enough at household scale; no SQLite, no database
- **Double-entry bookkeeping** — every transaction is a set of balanced postings (debits = credits per currency)
- **Multi-currency from day one** — postings carry a currency; cross-currency transactions use inline exchange prices
- **One file per record** — each transaction, account, balance assertion, and price-log entry lives in its own file to keep git merges conflict-free under human + AI co-editing
- **Human–AI collaborative editing** — designed to work alongside AI agents (Claude, etc.) that can read, create, and edit entries in the same ledger via git or Syncthing sync

## Project structure

```
sapphire-ledger/
├── cli/                     # CLI binary (sapphire-ledger) with stdio MCP server bundled
├── desktop/                 # Desktop GUI (egui); still a scaffold
├── server/                  # self-hosted MCP server over HTTP, authenticated per device (no /rpc sync yet)
└── crates/
    ├── sapphire-ledger-core/  # Data model, TOML parser/serializer, validation, write path
    └── sapphire-ledger-mcp/   # MCP server logic (library, used by the CLI and by server/)
```

## Status

The data model, validation, write path and MCP server work. You can point an
MCP client at `sapphire-ledger mcp` and have it read and record entries over
stdio, or at a `sapphire-ledger-server` instance's `/mcp` and do the same over
HTTP, authenticated per device (see [`docs/design.md`](docs/design.md#mcp-server)
for the CLI and the auth model). `/rpc` sync is not built, so a correction to
what an agent wrote still has to happen on the machine holding the
workspace's files — the review loop the project exists for is not yet
closed. The desktop GUI is still a scaffold, but the CLI can now write:

```
sapphire-ledger account add Expenses:Food --type Expense
sapphire-ledger tx add --narration "groceries" \
  --posting "Expenses:Food 1200 JPY" --posting "Assets:Cash -1200 JPY"
sapphire-ledger tx list --account Expenses:Food
```

plus `assertion add` and `price add` — all thin wrappers over the same
`core::ops` the MCP tools use, so the rules are identical whoever is typing.

## License

This repository contains components under different licenses:

| Component | License |
|-----------|---------|
| `sapphire-ledger-core` | MIT OR Apache-2.0 |
| `sapphire-ledger-mcp` | MIT OR Apache-2.0 |
| `sapphire-ledger-cli` | MIT OR Apache-2.0 |
| `sapphire-ledger-desktop` | MIT OR Apache-2.0 |
| `sapphire-ledger-server` | MIT OR Apache-2.0 |

See the `LICENSE-MIT` / `LICENSE-APACHE` files in each component's directory for the full license text.
