# sapphire-ledger

Local-first double-entry household ledger that keeps your data alive as plain text — timeless like fossils.

## Concept

- **TOML as source of truth** — all data lives in plain `.toml` files you can read and edit with any tool
- **No cache required** — `load_workspace` reads the TOML files directly on every load, which is fast enough at household scale; no SQLite, no database
- **Double-entry bookkeeping** — every transaction is a set of balanced postings (debits = credits per currency)
- **Multi-currency from day one** — postings carry a currency; cross-currency transactions use inline exchange prices
- **One file per record** — each transaction, account, and balance assertion lives in its own file to keep git merges conflict-free under human + AI co-editing
- **Human–AI collaborative editing** — designed to work alongside AI agents (Claude, etc.) that can read, create, and edit entries in the same ledger via git or Syncthing sync

## Project structure

```
sapphire-ledger/
├── sapphire-ledger-core/      # Data model, TOML parser/serializer, validation, write path
├── sapphire-ledger-mcp/       # MCP server logic (library, reused by CLI and the sync server)
├── sapphire-ledger-cli/       # CLI binary (sapphire-ledger) with stdio MCP server bundled
├── sapphire-ledger-desktop/   # Desktop GUI (egui); still a scaffold
└── sapphire-ledger-server/    # self-hosted sync + MCP server (planned)
```

## Status

The data model, validation, write path and MCP server work. You can point an
MCP client at `sapphire-ledger mcp` and have it read and record entries. The
desktop GUI is still a scaffold, and there are no CLI write commands yet —
records are created through the MCP tools or by hand.

## License

This repository contains components under different licenses:

| Component | License |
|-----------|---------|
| `sapphire-ledger-core` | MIT OR Apache-2.0 |
| `sapphire-ledger-mcp` | MIT OR Apache-2.0 |
| `sapphire-ledger-cli` | MIT OR Apache-2.0 |
| `sapphire-ledger-desktop` | MIT OR Apache-2.0 |

See the `LICENSE-MIT` / `LICENSE-APACHE` files in each component's directory for the full license text.
