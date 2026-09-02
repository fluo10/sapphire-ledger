# sapphire-ledger-server: `/mcp` over HTTP, behind a key set

- **Scope**: a new `sapphire-ledger-server` crate, plus an `http-server` feature on `sapphire-ledger-mcp`
- **Issue**: [#10](https://github.com/fluo10/sapphire-ledger/issues/10)
- **Builds on**: [the restart design](2026-09-01-restart-roadmap-design.md), phase 1 step 4
- **Depends on**: [#9](https://github.com/fluo10/sapphire-ledger/pull/9), which built the MCP server as a stdio library

## What this is

One self-hosted binary that serves the ledger's MCP surface over HTTP, authenticated with a bearer key set, so an agent on another process — or another machine — can read and record entries without spawning a child process per session.

## What this is not, yet

**`/rpc` is deliberately absent.** The restart design puts `/rpc` (the framework's delta sync) and `/mcp` in one binary behind one key set, and that is still the destination. It is not this change, because **`/rpc` has no client today**: the desktop crate is a 27-line scaffold, the CLI has no sync client, and there is no mobile build. Writing a sync server now means writing it against no consumer and finding out whether it works months later.

Adding it later is cheap and the design keeps it cheap: the framework's `router()` produces the `/rpc` routes as an `axum::Router`, and mounting it is a `.merge()` against the same `ServerState`. Nothing here has to be undone.

The `remote-server` feature is taken all the same — for `KeyStore` and `protect`, not for sync. Enabling a feature and mounting its routes are separate acts.

## What we compose, and what we write

`sapphire-framework`'s `remote-server` feature supplies:

- `KeyStore` / `KeyEntry` — `load`, `generate`, `rotate`, `revoke`, `authenticate`, `has_usable_key`
- `protect(state, router)` — a plain axum layer, independent of `router()`
- `ServerState`

So the key machinery is not ours to invent. What we write is the CLI presentation over it (duration parsing, token masking, table output) and the wiring.

`sapphire-journal-server` is the working template for both, and `sapphire-journal-mcp/src/http.rs` for the transport.

## Crate layout

```
sapphire-ledger-mcp/
  src/http.rs              NEW, behind the `http-server` feature
                           mcp_router(state, cancel, observer, allowed_hosts) -> Router

sapphire-ledger-server/    NEW binary
  src/main.rs              clap entry, dispatch, tracing setup
  src/cli.rs               Serve | GenKey | ListKeys | RotateKey | RevokeKey
  src/keys.rs              CLI presentation over the framework's KeyStore
  src/serve.rs             build the router, protect it, bind, drain on shutdown
```

There is no `dedupe.rs`. See "Duplicate ids" below.

The journal's `http.rs` also exposes a `serve_http` that binds a socket of its own, for a host that wants MCP and nothing else — its desktop app. Ledger has no such consumer (its desktop crate is a scaffold), so `mcp_router` is the whole public surface here. `serve_http` is a dozen lines to add when something needs it.

## The `Host` allowlist is an argument, not a default

rmcp defends against DNS rebinding by refusing any request whose `Host` header is not on an allowlist, and its default list is loopback only. That default is correct for a loopback bind and **wrong the moment the process binds anywhere else**: a client reaching the server as `http://box.tailnet.ts.net/mcp` sends a `Host` that matches nothing and gets `403`.

`mcp_router` therefore takes the extra hostnames as a parameter rather than leaving them at a default nobody remembers to change. Loopback is always added on top of whatever the caller passes, so widening never costs local use, and passing an empty list can never *disable* the guard — rmcp reads an empty list as "allow every host", and this module never hands it one.

This is copied from the journal's implementation, including the reasoning, because the failure mode it describes is quiet: the server looks healthy and every MCP request 403s.

## Auth and keys

One bearer key set. `protect()` layers over the `/mcp` router; there is no unauthenticated route.

**The key file is host-local, not in the workspace.** The workspace is the thing that will sync once `/rpc` lands, and a key that syncs is a key on every machine that ever pulled. Whether a given host is trusted is a host-local judgment, the same reasoning `sapphire-agent` applies to `acp-permissions.json`.

CLI: `gen-key`, `list-keys`, `rotate-key`, `revoke-key`. `list-keys` masks tokens. Rotation keeps a key's identity and replaces its token.

A running server holds a snapshot of the key file taken at startup, so a rotation or revocation takes effect on restart. That is the framework's behaviour and this crate does not paper over it — `list-keys` and the docs say so plainly rather than implying live revocation.

## Exposure, and one deliberate divergence from the journal

Default bind is loopback. `--addr` widens the bind; `--allowed-host` (repeatable) widens the `Host` allowlist.

**Bound beyond loopback with no `--allowed-host`, this server refuses to start.** The journal only warns, and that is right *there*: its `/rpc` keeps answering, so the process is still doing useful work and a hard failure would be worse than a degraded one.

Here there is no `/rpc`. A wide bind with an empty allowlist means every request 403s — the process is not dangerous, it is **useless**, and it will look like a client bug. Failing at startup with a message naming the missing flag turns a confusing runtime symptom into an obvious configuration error.

TLS is out of scope. Put it behind a reverse proxy.

## Duplicate ids: detected, never resolved

The framework's sync resolves per path, last-writer-wins on a **client-supplied** `updated_at`, with deletes as tombstones that are themselves subject to that comparison. A record whose path derives from mutable data can therefore be resurrected: a client that was offline across a rename pushes to the old path with a later timestamp, the tombstone loses, and two files now carry one id.

Ledger has two such paths, and both are on operations the design actively encourages:

- transactions, assertions and prices live under `{year}/{MM}/` derived from the record's **date**, so correcting a date across a month boundary moves the file;
- accounts live at a path derived from their **name**, so a rename moves the file — and cheap renames are the entire reason [#9](https://github.com/fluo10/sapphire-ledger/pull/9) made references id-linked.

`sapphire-journal-server` converges this automatically: the later arrival gets a fresh id and **both records are kept**. That is right for a journal, where a duplicated entry is noise.

**It is wrong for a ledger, where a duplicated transaction is a wrong balance.** Neither is the obvious alternative — last-writer-wins on the record's own `updated_at`, deleting the loser — acceptable: that timestamp is client-supplied wall-clock, so a machine with a skewed clock could delete a correct transaction and nothing would say so.

So this server **detects and reports; it does not resolve.** `Workspace::validate()` already emits `duplicate transaction id` / `duplicate assertion id` / `duplicate price id` / `duplicate account id` / `duplicate account name` (added in #9).

Concretely: the server runs `validate()` once at startup and then on a fixed five-minute tick, and logs any duplicate at `WARN` naming the id and every path carrying it — the paths are what a human needs to fix it, and `ValidationIssue` does not carry them, so the tick resolves them itself. Only duplicates are logged this way; the other validation issues are the ledger's ordinary business and belong to `validate_workspace`, not to the server's log. The interval is a constant, not a setting: nothing yet suggests one number is wrong, and a knob nobody has asked for is a knob to maintain.

Where money is concerned, a loud broken state beats a quiet resolved one. Automatic convergence is a decision to make with a real duplicate in front of us, not in advance.

Note the hazard is latent until `/rpc` exists — a single writer cannot produce it. Detection is being built now because the detection is nearly free, and because whoever adds `/rpc` should find this written down rather than rediscover it.

## Testing

- Key lifecycle: generate, list (token masked), rotate (identity kept, token changed), revoke.
- `protect`: no key store configured refuses everything; a valid bearer passes; an invalid or expired one does not.
- `Host` allowlist: loopback passes with an empty configured list; a configured host passes; an unlisted host gets 403. This is the quiet failure, so it gets an explicit test rather than trust.
- Refuse-to-start: a wide `--addr` with no `--allowed-host` exits non-zero with a message naming the flag.
- End to end: an MCP client calls one read tool and one write tool over HTTP and gets valid results.

## Risks

- **`sapphire-framework` is a git dependency tracking `main`**, and this change takes a second feature (`remote-server`) from it. More surface to move under us. `--locked` in CI pins the revision actually tested.
- **The key file's format is the framework's**, so a framework change to it is a change to this server's on-disk state. Acceptable — the alternative is maintaining a second key implementation.
- **No `/rpc` means the review loop is still not closed.** An agent can write, but a human still has to read the TOML files directly or run the CLI on the same host. That is a real gap, and it is the next piece of work rather than something this design solves.
