//! MCP server logic for sapphire-ledger.
//!
//! Modelled on `sapphire-journal-mcp`: one server struct holding an
//! `Arc<Mutex<LedgerState>>`, tools declared with rmcp's `#[tool]` macro, and
//! a stdio entry point. The HTTP transport arrives later, with
//! `sapphire-ledger-server`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_router,
    transport::stdio,
};
use sapphire_ledger_core::{LedgerState, ops};
use serde::Deserialize;

/// Called after a tool writes, with every path the write touched.
///
/// One call is one batch: a write producing several files reports them
/// together, so a syncing receiver never sees half of a change.
pub type WriteObserver = Arc<dyn Fn(&[PathBuf]) + Send + Sync>;

#[derive(Clone)]
pub struct SapphireLedgerServer {
    state: Arc<Mutex<LedgerState>>,
    tool_router: ToolRouter<Self>,
    write_observer: Option<WriteObserver>,
}

impl std::fmt::Debug for SapphireLedgerServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SapphireLedgerServer")
            .finish_non_exhaustive()
    }
}

impl SapphireLedgerServer {
    pub fn new(state: LedgerState) -> Self {
        Self::from_shared(Arc::new(Mutex::new(state)))
    }

    /// Build a server sharing an existing state handle. The HTTP transport
    /// spawns one server per session, but all of them must see one ledger.
    pub fn from_shared(state: Arc<Mutex<LedgerState>>) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
            write_observer: None,
        }
    }

    pub fn shared_state(&self) -> Arc<Mutex<LedgerState>> {
        Arc::clone(&self.state)
    }

    /// Set where post-write notifications go. Unused by the stdio transport.
    pub fn with_write_observer(mut self, observer: WriteObserver) -> Self {
        self.write_observer = Some(observer);
        self
    }

    fn notify_write(&self, paths: &[PathBuf]) {
        if let Some(observer) = &self.write_observer {
            observer(paths);
        }
    }

    /// Take the state lock, recovering it if an earlier panic poisoned it.
    ///
    /// `.lock().unwrap()` would turn one panic under this mutex into a
    /// permanently dead server. `LedgerState`'s invariants do not depend on
    /// the previous holder finishing: it holds a snapshot that `reload()`
    /// rebuilds from scratch. So warn once and carry on.
    fn lock_state(&self) -> std::sync::MutexGuard<'_, LedgerState> {
        if self.state.is_poisoned() {
            tracing::warn!(
                "ledger state mutex was poisoned by an earlier panic; recovering it rather \
                 than failing every tool call for the rest of the process"
            );
        }
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

// ── parameter structs ─────────────────────────────────────────────────────────

/// Explicit empty parameter object for zero-argument tools.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct EmptyParams {}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetTransactionParams {
    /// The transaction's 7-character id.
    pub id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct QueryPostingsParams {
    /// Account id, or account name. Either is accepted.
    pub account: Option<String>,
    /// Currency code, e.g. `JPY`.
    pub currency: Option<String>,
    /// Inclusive lower bound, `YYYY-MM-DD`.
    pub date_from: Option<String>,
    /// Inclusive upper bound, `YYYY-MM-DD`.
    pub date_to: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct QueryPricesParams {
    pub base: Option<String>,
    pub quote: Option<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
}

fn parse_date(raw: &str) -> anyhow::Result<chrono::NaiveDate> {
    raw.parse::<chrono::NaiveDate>()
        .with_context(|| format!("not a YYYY-MM-DD date: {raw}"))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddAccountParams {
    /// Colon-separated account name, e.g. `Assets:Cash:JPY`.
    pub name: String,
    /// One of: Asset, Liability, Equity, Income, Expense.
    #[serde(rename = "type")]
    pub account_type: String,
    /// Currencies this account may hold. Empty means any.
    #[serde(default)]
    pub currencies: Vec<String>,
    /// `YYYY-MM-DD` the account opened.
    pub opened_at: String,
    pub description: Option<String>,
}

/// One side of a transaction. Amounts are strings so decimals survive JSON.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PostingParam {
    /// Account id or account name — either is accepted, and the account must
    /// already exist. The stored record always keeps the id.
    pub account: String,
    /// Signed decimal, e.g. `1200` or `-1200`.
    pub amount: String,
    /// Currency code, e.g. `JPY`.
    pub currency: String,
    /// For a cross-currency posting: the unit price in `price_currency`.
    pub price_value: Option<String>,
    /// Currency the `price_value` is denominated in.
    pub price_currency: Option<String>,
    pub memo: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddTransactionParams {
    /// `YYYY-MM-DD`.
    pub date: String,
    /// What the transaction was.
    pub narration: String,
    pub payee: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// `cleared` or `pending`.
    pub status: Option<String>,
    /// At least two postings, summing to zero per currency.
    pub postings: Vec<PostingParam>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BalanceParam {
    /// Decimal amount as a string.
    pub amount: String,
    pub currency: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddAssertionParams {
    /// Account id or account name.
    pub account: String,
    /// `YYYY-MM-DD`. The balance is asserted at the END of this date.
    pub date: String,
    pub balances: Vec<BalanceParam>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddPriceParams {
    pub date: String,
    /// Base currency, e.g. `USD`.
    pub base: String,
    /// Quote currency, e.g. `JPY`.
    pub quote: String,
    /// How many `quote` units one `base` unit costs, as a decimal string.
    pub rate: String,
    /// Where the rate came from, e.g. `manual`.
    pub source: Option<String>,
}

fn parse_decimal(raw: &str) -> anyhow::Result<rust_decimal::Decimal> {
    raw.parse::<rust_decimal::Decimal>()
        .with_context(|| format!("not a decimal: {raw}"))
}

fn parse_account_type(raw: &str) -> anyhow::Result<sapphire_ledger_core::AccountType> {
    use sapphire_ledger_core::AccountType::*;
    Ok(match raw {
        "Asset" => Asset,
        "Liability" => Liability,
        "Equity" => Equity,
        "Income" => Income,
        "Expense" => Expense,
        other => anyhow::bail!(
            "unknown account type {other:?}; expected one of \
             Asset, Liability, Equity, Income, Expense"
        ),
    })
}

fn parse_status(raw: &str) -> anyhow::Result<sapphire_ledger_core::TransactionStatus> {
    use sapphire_ledger_core::TransactionStatus::*;
    Ok(match raw {
        "cleared" => Cleared,
        "pending" => Pending,
        other => anyhow::bail!("unknown status {other:?}; expected `cleared` or `pending`"),
    })
}

/// Classify a caller-supplied account reference into exactly one slot of the
/// id/name pair `ops` expects: `account_id` if `needle` matches a known
/// account's id, otherwise `account_name`. `ops::resolve_account` then
/// resolves whichever slot was filled -- an id that does not match is a hard
/// error, it is never retried as a name.
fn split_account_ref(
    needle: &str,
    accounts: &[sapphire_ledger_core::Account],
) -> (Option<String>, Option<String>) {
    if accounts.iter().any(|a| a.id == needle) {
        (Some(needle.to_string()), None)
    } else {
        (None, Some(needle.to_string()))
    }
}

/// Build a write tool's success message, appending a warning if the
/// post-write `reload()` failed. The record is on disk and the observer has
/// already been told either way -- this only affects what the caller reads
/// back, never whether the write is reported as having happened.
fn write_result(
    kind: &str,
    id: &str,
    dest: &Path,
    reload_err: Option<sapphire_ledger_core::Error>,
) -> String {
    let mut msg = format!("created {kind} {id}: {}", dest.display());
    if let Some(e) = reload_err {
        msg.push_str(&format!(
            "\nWARNING: the record was written, but the ledger snapshot could not be \
             reloaded afterwards: {e}"
        ));
    }
    msg
}

fn build_posting(
    p: PostingParam,
    accounts: &[sapphire_ledger_core::Account],
) -> anyhow::Result<sapphire_ledger_core::Posting> {
    let price = match (p.price_value, p.price_currency) {
        (Some(value), Some(currency)) => Some(sapphire_ledger_core::Price {
            value: parse_decimal(&value)?,
            currency,
        }),
        (None, None) => None,
        _ => anyhow::bail!("price_value and price_currency must be given together"),
    };
    let (account_id, account_name) = split_account_ref(&p.account, accounts);
    Ok(sapphire_ledger_core::Posting {
        account_id,
        account_name,
        amount: parse_decimal(&p.amount)?,
        currency: p.currency,
        price,
        memo: p.memo,
    })
}

/// One posting flattened with its transaction's context, which is what a
/// caller asking "what happened to this account" actually wants.
#[derive(serde::Serialize)]
struct PostingHit {
    transaction_id: String,
    date: chrono::NaiveDate,
    narration: String,
    account_id: Option<String>,
    account_name: Option<String>,
    amount: String,
    currency: String,
}

// ── tools ─────────────────────────────────────────────────────────────────────

#[tool_router]
impl SapphireLedgerServer {
    #[tool(
        description = "List every account with its id, name, type, allowed currencies \
        and open date. The id is the stable handle: it survives a rename, and other \
        tools accept it wherever they accept a name."
    )]
    fn list_accounts(&self, Parameters(_): Parameters<EmptyParams>) -> Result<String, String> {
        (|| -> anyhow::Result<String> {
            let guard = self.lock_state();
            Ok(serde_json::to_string_pretty(&guard.workspace().accounts)?)
        })()
        .map_err(|e| e.to_string())
    }

    #[tool(description = "Show one transaction, with all of its postings, by id.")]
    fn get_transaction(
        &self,
        Parameters(p): Parameters<GetTransactionParams>,
    ) -> Result<String, String> {
        (|| -> anyhow::Result<String> {
            let guard = self.lock_state();
            let found = guard
                .workspace()
                .transactions
                .iter()
                .find(|t| t.id == p.id)
                .with_context(|| format!("no transaction with id {}", p.id))?;
            Ok(serde_json::to_string_pretty(found)?)
        })()
        .map_err(|e| e.to_string())
    }

    #[tool(
        description = "Find postings, optionally filtered by account, currency and date \
        range. `account` matches either an account id or an account name. Each result \
        carries its transaction's id, date and narration."
    )]
    fn query_postings(
        &self,
        Parameters(p): Parameters<QueryPostingsParams>,
    ) -> Result<String, String> {
        (|| -> anyhow::Result<String> {
            let from = p.date_from.as_deref().map(parse_date).transpose()?;
            let to = p.date_to.as_deref().map(parse_date).transpose()?;
            let guard = self.lock_state();
            let ws = guard.workspace();

            // Resolve the filter once, so that filtering by a renamed
            // account's *current* name still finds postings whose stored
            // name is stale, and so that a posting carrying only a name
            // still matches.
            let wanted: Option<(Option<String>, String)> = p.account.as_ref().map(|needle| {
                match ws
                    .accounts
                    .iter()
                    .find(|a| &a.id == needle || &a.name == needle)
                {
                    Some(a) => (Some(a.id.clone()), a.name.clone()),
                    // The needle names no known account. Keep it literally, so a
                    // hand-edited reference to an account that does not exist is
                    // still findable -- that is worth surfacing, not hiding.
                    None => (None, needle.clone()),
                }
            });

            let mut hits: Vec<PostingHit> = Vec::new();
            for tx in &ws.transactions {
                if from.is_some_and(|f| tx.date < f) || to.is_some_and(|t| tx.date > t) {
                    continue;
                }
                for posting in &tx.postings {
                    if let Some((wanted_id, wanted_needle)) = &wanted {
                        let matches = match (&posting.account_id, wanted_id) {
                            // Both sides resolved: the id is authoritative and decides alone.
                            (Some(pid), Some(wid)) => pid == wid,
                            // The posting carries no id, so its name is its only reference.
                            (None, _) => {
                                posting.account_name.as_deref() == Some(wanted_needle.as_str())
                            }
                            // The needle named no known account; compare it literally so a
                            // dangling id reference is still findable.
                            (Some(pid), None) => pid == wanted_needle,
                        };
                        if !matches {
                            continue;
                        }
                    }
                    if p.currency.as_ref().is_some_and(|c| c != &posting.currency) {
                        continue;
                    }
                    hits.push(PostingHit {
                        transaction_id: tx.id.clone(),
                        date: tx.date,
                        narration: tx.narration.clone(),
                        account_id: posting.account_id.clone(),
                        account_name: posting.account_name.clone(),
                        amount: posting.amount.to_string(),
                        currency: posting.currency.clone(),
                    });
                }
            }
            hits.sort_by(|a, b| {
                a.date
                    .cmp(&b.date)
                    .then_with(|| a.transaction_id.cmp(&b.transaction_id))
            });
            Ok(serde_json::to_string_pretty(&hits)?)
        })()
        .map_err(|e| e.to_string())
    }

    #[tool(
        description = "Find price-log entries, optionally filtered by base, quote and \
        date range."
    )]
    fn query_prices(&self, Parameters(p): Parameters<QueryPricesParams>) -> Result<String, String> {
        (|| -> anyhow::Result<String> {
            let from = p.date_from.as_deref().map(parse_date).transpose()?;
            let to = p.date_to.as_deref().map(parse_date).transpose()?;
            let guard = self.lock_state();

            let mut hits: Vec<_> = guard
                .workspace()
                .prices
                .iter()
                .filter(|e| !from.is_some_and(|f| e.date < f))
                .filter(|e| !to.is_some_and(|t| e.date > t))
                .filter(|e| !p.base.as_ref().is_some_and(|b| b != &e.base))
                .filter(|e| !p.quote.as_ref().is_some_and(|q| q != &e.quote))
                .collect();
            hits.sort_by_key(|e| e.date);
            Ok(serde_json::to_string_pretty(&hits)?)
        })()
        .map_err(|e| e.to_string())
    }

    #[tool(
        description = "Re-read the ledger from disk and report every validation issue. \
        Returns an empty array when the ledger is consistent. A posting whose stored \
        account_name is out of date is NOT an issue -- the account_id is what counts."
    )]
    fn validate_workspace(&self, Parameters(_): Parameters<EmptyParams>) -> Result<String, String> {
        (|| -> anyhow::Result<String> {
            let mut guard = self.lock_state();
            guard.reload()?;
            Ok(serde_json::to_string_pretty(&guard.workspace().validate())?)
        })()
        .map_err(|e| e.to_string())
    }

    #[tool(
        description = "Create an account. Fails if one with that name already exists. \
        Every account a posting names must be created first. Returns the new account's id."
    )]
    fn add_account(&self, Parameters(p): Parameters<AddAccountParams>) -> Result<String, String> {
        (|| -> anyhow::Result<String> {
            let account_type = parse_account_type(&p.account_type)?;
            let opened_at = parse_date(&p.opened_at)?;

            let mut guard = self.lock_state();
            let (id, dest) = ops::create_account(
                guard.root(),
                p.name,
                account_type,
                p.currencies,
                opened_at,
                p.description,
            )?;
            let reload_err = guard.reload().err();
            drop(guard);
            // The file exists regardless of whether we could re-read the
            // workspace, so the observer must always hear about it.
            self.notify_write(std::slice::from_ref(&dest));
            Ok(write_result("account", &id, &dest, reload_err))
        })()
        .map_err(|e| e.to_string())
    }

    #[tool(
        description = "Record a transaction. Postings must sum to zero per currency, and \
        every account named must already exist. Nothing is written if validation fails."
    )]
    fn add_transaction(
        &self,
        Parameters(p): Parameters<AddTransactionParams>,
    ) -> Result<String, String> {
        (|| -> anyhow::Result<String> {
            let date = parse_date(&p.date)?;
            let status = p.status.as_deref().map(parse_status).transpose()?;

            let mut guard = self.lock_state();
            // Nothing has been written yet, so a failure here should refuse
            // the write rather than resolve accounts against a stale
            // snapshot -- `ops` reads the accounts directory fresh, and this
            // keeps the reference the MCP layer classifies in sync with it.
            guard.reload()?;
            let postings = {
                let accounts = &guard.workspace().accounts;
                p.postings
                    .into_iter()
                    .map(|param| build_posting(param, accounts))
                    .collect::<anyhow::Result<Vec<_>>>()?
            };
            let (id, dest) = ops::create_transaction(
                guard.root(),
                date,
                p.narration,
                p.payee,
                p.tags,
                status,
                postings,
            )?;
            let reload_err = guard.reload().err();
            drop(guard);
            self.notify_write(std::slice::from_ref(&dest));
            Ok(write_result("transaction", &id, &dest, reload_err))
        })()
        .map_err(|e| e.to_string())
    }

    #[tool(
        description = "Record a balance assertion: what an account should hold at the END \
        of a date. A mismatch is a hard error when the books are checked."
    )]
    fn add_assertion(
        &self,
        Parameters(p): Parameters<AddAssertionParams>,
    ) -> Result<String, String> {
        (|| -> anyhow::Result<String> {
            let date = parse_date(&p.date)?;
            let balances = p
                .balances
                .into_iter()
                .map(|b| {
                    Ok(sapphire_ledger_core::Balance {
                        amount: parse_decimal(&b.amount)?,
                        currency: b.currency,
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>()?;

            let mut guard = self.lock_state();
            // See add_transaction: refuse the write rather than classify the
            // account reference against a stale snapshot.
            guard.reload()?;
            let (account_id, account_name) =
                split_account_ref(&p.account, &guard.workspace().accounts);
            let (id, dest) =
                ops::create_assertion(guard.root(), account_id, account_name, date, balances)?;
            let reload_err = guard.reload().err();
            drop(guard);
            self.notify_write(std::slice::from_ref(&dest));
            Ok(write_result("assertion", &id, &dest, reload_err))
        })()
        .map_err(|e| e.to_string())
    }

    #[tool(
        description = "Record an observed exchange rate in the price log: one unit of \
        `base` costs `rate` units of `quote` on `date`."
    )]
    fn add_price(&self, Parameters(p): Parameters<AddPriceParams>) -> Result<String, String> {
        (|| -> anyhow::Result<String> {
            let date = parse_date(&p.date)?;
            let rate = parse_decimal(&p.rate)?;

            let mut guard = self.lock_state();
            let (id, dest) =
                ops::create_price(guard.root(), date, p.base, p.quote, rate, p.source)?;
            let reload_err = guard.reload().err();
            drop(guard);
            self.notify_write(std::slice::from_ref(&dest));
            Ok(write_result("price", &id, &dest, reload_err))
        })()
        .map_err(|e| e.to_string())
    }
}

#[rmcp::tool_handler(router = self.tool_router)]
impl ServerHandler for SapphireLedgerServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Sapphire Ledger is a plain-text double-entry household ledger. Use \
             list_accounts to see the chart of accounts, query_postings to find what \
             happened to an account, add_transaction to record spending, and \
             validate_workspace to check the books. Every transaction must balance: its \
             postings sum to zero per currency. An account must exist before anything \
             can be posted to it."
                .to_owned(),
        )
    }
}

// ── startup helpers ───────────────────────────────────────────────────────────

/// Open the ledger at `dir`, or find one by walking up from the working
/// directory when `dir` is None.
///
/// With `init`, the literal target (no upward search) is created and turned
/// into a ledger if it is not one already. An existing ledger is reused.
pub fn prepare_state(ledger_dir: Option<&Path>, init: bool) -> anyhow::Result<LedgerState> {
    if init {
        let target: PathBuf = match ledger_dir {
            Some(d) => d.to_path_buf(),
            None => std::env::current_dir().context("failed to read current directory")?,
        };
        if !target.join(".sapphire-ledger").exists() {
            sapphire_ledger_core::init_workspace(&target, "JPY")
                .context("failed to initialize ledger")?;
            tracing::info!("initialized sapphire-ledger in {}", target.display());
        }
        return LedgerState::open(&target).context("failed to open ledger after init");
    }

    match ledger_dir {
        Some(d) => LedgerState::open(d).with_context(|| {
            format!(
                "not a sapphire-ledger: {} — pass --init to create one",
                d.display()
            )
        }),
        None => {
            let cwd = std::env::current_dir().context("failed to read current directory")?;
            LedgerState::find(&cwd).context(
                "no sapphire-ledger found in the current directory or any parent \
                 — pass --init to create one",
            )
        }
    }
}

// ── stdio entry point ─────────────────────────────────────────────────────────

/// Serve the ledger over stdio, speaking MCP.
#[tokio::main]
pub async fn run(ledger_dir: Option<&Path>, init: bool) -> anyhow::Result<()> {
    // stdout carries JSON-RPC, so every log line goes to stderr.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    let state = prepare_state(ledger_dir, init)?;
    let server = SapphireLedgerServer::new(state);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sapphire_ledger_core::AccountType;

    /// Build a throwaway server over a freshly initialized ledger. Hold the
    /// `TempDir` for the test's duration — dropping it removes the directory
    /// the open state points at.
    fn test_server() -> (tempfile::TempDir, SapphireLedgerServer) {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = prepare_state(Some(dir.path()), true).expect("init test ledger");
        (dir, SapphireLedgerServer::new(state))
    }

    #[test]
    fn list_accounts_is_empty_on_a_fresh_ledger() {
        let (_dir, server) = test_server();
        let json = server
            .list_accounts(Parameters(EmptyParams {}))
            .expect("ok");
        assert_eq!(json.trim(), "[]");
    }

    /// Zero-argument tools must advertise a top-level `type: object`, or
    /// Anthropic rejects the tool with
    /// `tools.<N>.custom.input_schema.type: Field required`.
    #[test]
    fn every_tool_input_schema_declares_object_type() {
        let tools = SapphireLedgerServer::tool_router().list_all();
        assert_eq!(tools.len(), 9);
        for tool in tools {
            let schema = serde_json::to_value(&tool.input_schema).expect("schema to json");
            assert_eq!(
                schema.get("type").and_then(|t| t.as_str()),
                Some("object"),
                "tool {} has a non-object input schema: {schema}",
                tool.name
            );
        }
    }

    #[test]
    fn validate_workspace_reports_an_undefined_account() {
        let (dir, server) = test_server();
        // Written straight to disk: ops::create_transaction would refuse it,
        // which is the point -- this is the hand-edited-file case that
        // validate_workspace exists to catch.
        let path = dir.path().join("transactions/2026/05/tx00001.toml");
        std::fs::create_dir_all(path.parent().unwrap()).expect("mkdir");
        std::fs::write(
            &path,
            r#"
id = "tx00001"
date = "2026-05-21"
narration = "orphan"
created_at = "2026-05-21T18:30:00+09:00"
updated_at = "2026-05-21T18:30:00+09:00"

[[postings]]
account_name = "Expenses:Nowhere"
amount = "10"
currency = "JPY"

[[postings]]
account_name = "Assets:Nowhere"
amount = "-10"
currency = "JPY"
"#,
        )
        .expect("write");

        let json = server
            .validate_workspace(Parameters(EmptyParams {}))
            .expect("ok");
        assert!(json.contains("undefined account"), "got: {json}");
    }

    /// A posting can carry only `account_name` (no `account_id`) -- that is
    /// a valid posting, not an error. `query_postings` must still find it
    /// when filtering by that account's name; matching the id-only arm and
    /// dropping the name-only posting silently would make the tool look
    /// empty for exactly the postings it exists to surface.
    #[test]
    fn query_postings_matches_a_name_only_posting_by_account_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        prepare_state(Some(dir.path()), true).expect("init test ledger");

        ops::create_account(
            dir.path(),
            "Expenses:Food".to_string(),
            AccountType::Expense,
            vec!["JPY".to_string()],
            "2026-01-01".parse().expect("date"),
            None,
        )
        .expect("create account");
        ops::create_account(
            dir.path(),
            "Assets:Cash".to_string(),
            AccountType::Asset,
            vec!["JPY".to_string()],
            "2026-01-01".parse().expect("date"),
            None,
        )
        .expect("create account");

        // Written straight to disk, like the validate_workspace fixture
        // above: this is the only way to produce a posting with a name but
        // no id, since ops::create_transaction always resolves and fills
        // account_id.
        let path = dir.path().join("transactions/2026/05/tx00002.toml");
        std::fs::create_dir_all(path.parent().unwrap()).expect("mkdir");
        std::fs::write(
            &path,
            r#"
id = "tx00002"
date = "2026-05-21"
narration = "groceries"
created_at = "2026-05-21T18:30:00+09:00"
updated_at = "2026-05-21T18:30:00+09:00"

[[postings]]
account_name = "Expenses:Food"
amount = "10"
currency = "JPY"

[[postings]]
account_name = "Assets:Cash"
amount = "-10"
currency = "JPY"
"#,
        )
        .expect("write");

        let state = LedgerState::open(dir.path()).expect("reopen ledger");
        let server = SapphireLedgerServer::new(state);

        let json = server
            .query_postings(Parameters(QueryPostingsParams {
                account: Some("Expenses:Food".to_string()),
                currency: None,
                date_from: None,
                date_to: None,
            }))
            .expect("ok");
        assert!(json.contains("\"tx00002\""), "got: {json}");
    }

    fn posting_param(account: &str, amount: &str) -> PostingParam {
        PostingParam {
            account: account.to_string(),
            amount: amount.to_string(),
            currency: "JPY".to_string(),
            price_value: None,
            price_currency: None,
            memo: None,
        }
    }

    fn add_account(server: &SapphireLedgerServer, name: &str, ty: &str) -> String {
        server
            .add_account(Parameters(AddAccountParams {
                name: name.into(),
                account_type: ty.into(),
                currencies: vec![],
                opened_at: "2026-01-01".into(),
                description: None,
            }))
            .expect("add_account")
    }

    #[test]
    fn add_transaction_records_and_is_then_queryable() {
        let (_dir, server) = test_server();
        add_account(&server, "Expenses:Food", "Expense");
        add_account(&server, "Assets:Cash:JPY", "Asset");

        let created = server
            .add_transaction(Parameters(AddTransactionParams {
                date: "2026-05-21".into(),
                narration: "イオン買い物".into(),
                payee: Some("イオン".into()),
                tags: vec!["grocery".into()],
                status: None,
                postings: vec![
                    posting_param("Expenses:Food", "1200"),
                    posting_param("Assets:Cash:JPY", "-1200"),
                ],
            }))
            .expect("add_transaction");
        assert!(created.contains("created"), "got: {created}");

        let hits = server
            .query_postings(Parameters(QueryPostingsParams {
                account: Some("Expenses:Food".into()),
                currency: None,
                date_from: None,
                date_to: None,
            }))
            .expect("query");
        assert!(hits.contains("イオン買い物"), "got: {hits}");

        let issues = server
            .validate_workspace(Parameters(EmptyParams {}))
            .expect("validate");
        assert_eq!(issues.trim(), "[]", "ledger should be clean, got: {issues}");
    }

    #[test]
    fn a_posting_can_name_an_account_by_its_id() {
        let (_dir, server) = test_server();
        let food_msg = add_account(&server, "Expenses:Food", "Expense");
        add_account(&server, "Assets:Cash:JPY", "Asset");

        // add_account reports "created account <id>: <path>"; pull the id out.
        let food_id = food_msg
            .split_whitespace()
            .nth(2)
            .expect("id in message")
            .trim_end_matches(':')
            .to_string();

        server
            .add_transaction(Parameters(AddTransactionParams {
                date: "2026-05-21".into(),
                narration: "by id".into(),
                payee: None,
                tags: vec![],
                status: None,
                postings: vec![
                    posting_param(&food_id, "500"),
                    posting_param("Assets:Cash:JPY", "-500"),
                ],
            }))
            .expect("add_transaction by id");

        let issues = server
            .validate_workspace(Parameters(EmptyParams {}))
            .expect("validate");
        assert_eq!(issues.trim(), "[]", "got: {issues}");
    }

    #[test]
    fn add_transaction_rejects_an_unbalanced_entry() {
        let (_dir, server) = test_server();
        add_account(&server, "Expenses:Food", "Expense");
        add_account(&server, "Assets:Cash:JPY", "Asset");

        let err = server
            .add_transaction(Parameters(AddTransactionParams {
                date: "2026-05-21".into(),
                narration: "wrong".into(),
                payee: None,
                tags: vec![],
                status: None,
                postings: vec![
                    posting_param("Expenses:Food", "1200"),
                    posting_param("Assets:Cash:JPY", "-999"),
                ],
            }))
            .expect_err("must reject");
        assert!(err.contains("does not balance"), "got: {err}");
    }

    #[test]
    fn add_account_rejects_an_unknown_type() {
        let (_dir, server) = test_server();
        let err = server
            .add_account(Parameters(AddAccountParams {
                name: "Assets:Cash:JPY".into(),
                account_type: "Bogus".into(),
                currencies: vec![],
                opened_at: "2026-01-01".into(),
                description: None,
            }))
            .expect_err("must reject");
        assert!(
            err.contains("Bogus"),
            "the error should name the bad input, got: {err}"
        );
    }

    /// Build a throwaway server that records every path it is asked to
    /// notify about, so a test can assert the observer actually fired.
    fn test_server_with_observer() -> (
        tempfile::TempDir,
        SapphireLedgerServer,
        Arc<Mutex<Vec<PathBuf>>>,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = prepare_state(Some(dir.path()), true).expect("init test ledger");
        let seen: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
        let seen_for_observer = Arc::clone(&seen);
        let server = SapphireLedgerServer::new(state).with_write_observer(Arc::new(
            move |paths: &[PathBuf]| {
                seen_for_observer.lock().unwrap().extend_from_slice(paths);
            },
        ));
        (dir, server, seen)
    }

    #[test]
    fn a_successful_write_notifies_the_observer() {
        let (_dir, server, seen) = test_server_with_observer();

        server
            .add_price(Parameters(AddPriceParams {
                date: "2026-05-21".into(),
                base: "USD".into(),
                quote: "JPY".into(),
                rate: "150".into(),
                source: None,
            }))
            .expect("add_price");

        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "observer should fire exactly once for one write"
        );
    }

    /// `load_workspace` is fail-fast over every file in the workspace, so a
    /// single malformed record anywhere -- a human edit, a half-finished
    /// sync, a git conflict marker -- makes the post-write `reload()` fail.
    /// That must not be reported as a failed write: the file is genuinely on
    /// disk, the observer must still be told, and an AI caller must not be
    /// tempted to retry (which would mint a second, duplicate record).
    #[test]
    fn a_broken_record_does_not_turn_a_successful_write_into_a_reported_failure() {
        let (dir, server, seen) = test_server_with_observer();

        let broken = dir.path().join("transactions/2026/05/tx-broken.toml");
        std::fs::create_dir_all(broken.parent().unwrap()).expect("mkdir");
        std::fs::write(&broken, "this is not valid toml >>>>>>> HEAD\n")
            .expect("write broken toml");

        let result = server
            .add_price(Parameters(AddPriceParams {
                date: "2026-05-21".into(),
                base: "USD".into(),
                quote: "JPY".into(),
                rate: "150".into(),
                source: None,
            }))
            .expect("the write itself succeeded; only the post-write reload should fail");

        assert!(result.contains("created price"), "got: {result}");
        assert!(
            result.contains("WARNING"),
            "the caller should be told the snapshot reload failed, got: {result}"
        );
        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "the observer must still fire even though reload() failed"
        );
    }
}
