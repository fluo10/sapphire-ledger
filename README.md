# sapphire-ledger

Local-first double-entry household ledger that keeps your data alive as plain text — timeless like fossils.

## Concept

- **TOML as source of truth** — all data lives in plain `.toml` files you can read and edit with any tool
- **SQLite as cache** — fast querying and indexing on top of the TOML files
- **Double-entry bookkeeping** — every transaction is a set of balanced postings (debits = credits per currency)
- **Multi-currency from day one** — postings carry a currency; cross-currency transactions use inline exchange prices
- **One file per record** — each transaction, account, and balance assertion lives in its own file to keep git merges conflict-free under human + AI co-editing
- **Human–AI collaborative editing** — designed to work alongside AI agents (Claude, etc.) that can read, create, and edit entries in the same ledger via git or Syncthing sync

## Project structure

```
sapphire-ledger/
├── sapphire-ledger-core/      # Data model, TOML parser/serializer, SQLite cache
├── sapphire-ledger-mcp/       # MCP server logic (library, reused by CLI and Desktop)
├── sapphire-ledger-cli/       # CLI binary (sale) with stdio MCP server bundled
└── sapphire-ledger-desktop/   # Desktop GUI (egui) with HTTP MCP server bundled
```

## Status

Early scaffolding. Data model, parser, and tools are not yet implemented.

## License

This repository contains components under different licenses:

| Component | License |
|-----------|---------|
| `sapphire-ledger-core` | MIT OR Apache-2.0 |
| `sapphire-ledger-mcp` | MIT OR Apache-2.0 |
| `sapphire-ledger-cli` | MIT OR Apache-2.0 |
| `sapphire-ledger-desktop` | MIT OR Apache-2.0 |

See the `LICENSE-MIT` / `LICENSE-APACHE` files in each component's directory for the full license text.
