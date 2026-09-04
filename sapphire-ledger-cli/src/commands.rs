//! The write and read subcommands.
//!
//! Every one of these is a thin shell over `core::ops`: parse the strings a
//! shell can carry into the domain types, hand them over, and report what came
//! back. **No validation lives here.** The balance check, the account
//! resolution, the currency constraint and the refuse-to-overwrite rule are all
//! `ops`' — that is the entire reason `ops` exists, and a second copy here
//! would be a second place for them to drift.

use std::path::Path;

use anyhow::{Context as _, bail};
use chrono::NaiveDate;
use sapphire_ledger_core::{AccountType, Balance, TransactionStatus, ops};

use crate::cli::{AccountCommand, AssertionCommand, PriceCommand, TxCommand};
use crate::posting;

pub fn account(command: AccountCommand, root: &Path) -> anyhow::Result<()> {
    let AccountCommand::Add {
        name,
        account_type,
        currencies,
        opened_at,
        description,
    } = command;

    let account_type = parse_account_type(&account_type)?;
    let opened_at = match opened_at.as_deref() {
        Some(raw) => parse_date(raw)?,
        None => today(),
    };

    let (id, dest) =
        ops::create_account(root, name, account_type, currencies, opened_at, description)?;
    println!("created account {id} at {}", shown(&dest));
    Ok(())
}

pub fn tx(command: TxCommand, root: &Path) -> anyhow::Result<()> {
    match command {
        TxCommand::Add {
            date,
            narration,
            payee,
            tags,
            status,
            postings,
        } => {
            let date = match date.as_deref() {
                Some(raw) => parse_date(raw)?,
                None => today(),
            };
            let status = status.as_deref().map(parse_status).transpose()?;
            let postings = postings
                .iter()
                .map(|spec| posting::parse(spec))
                .collect::<anyhow::Result<Vec<_>>>()?;

            let (id, dest) =
                ops::create_transaction(root, date, narration, payee, tags, status, postings)?;
            println!("created transaction {id} at {}", shown(&dest));
            Ok(())
        }
        TxCommand::List {
            account,
            currency,
            date_from,
            date_to,
        } => list(root, account, currency, date_from, date_to),
    }
}

pub fn assertion(command: AssertionCommand, root: &Path) -> anyhow::Result<()> {
    let AssertionCommand::Add {
        account,
        date,
        balances,
    } = command;

    let date = parse_date(&date)?;
    let balances = balances
        .iter()
        .map(|spec| parse_balance(spec))
        .collect::<anyhow::Result<Vec<_>>>()?;

    // Offered as a name; `ops` resolves an id just as readily and fills in
    // whichever half is missing.
    let (id, dest) = ops::create_assertion(root, None, Some(account), date, balances)?;
    println!("created assertion {id} at {}", shown(&dest));
    Ok(())
}

pub fn price(command: PriceCommand, root: &Path) -> anyhow::Result<()> {
    let PriceCommand::Add {
        date,
        base,
        quote,
        rate,
        source,
    } = command;

    let date = parse_date(&date)?;
    let rate = rate
        .parse()
        .with_context(|| format!("not a decimal rate: {rate:?}"))?;

    let (id, dest) = ops::create_price(root, date, base, quote, rate, source)?;
    println!("created price {id} at {}", shown(&dest));
    Ok(())
}

fn list(
    root: &Path,
    account: Option<String>,
    currency: Option<String>,
    date_from: Option<String>,
    date_to: Option<String>,
) -> anyhow::Result<()> {
    let from = date_from.as_deref().map(parse_date).transpose()?;
    let to = date_to.as_deref().map(parse_date).transpose()?;
    let workspace = sapphire_ledger_core::load_workspace(root)?;

    // Resolve the filter to an id once, so that filtering by a renamed
    // account's *current* name still finds postings whose stored name is
    // stale. An unresolvable needle is compared literally, which is how a
    // dangling reference stays findable.
    let wanted: Option<(Option<String>, String)> = account.as_ref().map(|needle| {
        match workspace
            .accounts
            .iter()
            .find(|a| &a.id == needle || &a.name == needle)
        {
            Some(a) => (Some(a.id.clone()), a.name.clone()),
            None => (None, needle.clone()),
        }
    });

    let mut rows: Vec<(NaiveDate, String, String, String, String)> = Vec::new();
    for tx in &workspace.transactions {
        if from.is_some_and(|f| tx.date < f) || to.is_some_and(|t| tx.date > t) {
            continue;
        }
        for p in &tx.postings {
            if let Some((wanted_id, wanted_needle)) = &wanted {
                let matches = match (&p.account_id, wanted_id) {
                    (Some(pid), Some(wid)) => pid == wid,
                    (None, _) => p.account_name.as_deref() == Some(wanted_needle.as_str()),
                    (Some(pid), None) => pid == wanted_needle,
                };
                if !matches {
                    continue;
                }
            }
            if currency.as_ref().is_some_and(|c| c != &p.currency) {
                continue;
            }
            rows.push((
                tx.date,
                tx.id.clone(),
                tx.narration.clone(),
                p.account_name
                    .clone()
                    .unwrap_or_else(|| p.account_id.clone().unwrap_or_else(|| "?".to_owned())),
                format!("{} {}", p.amount, p.currency),
            ));
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

    for (date, id, narration, account, amount) in rows {
        println!("{date}  {id}  {account:<28}  {amount:>16}  {narration}");
    }
    Ok(())
}

/// Render a path for a person to read.
///
/// `find_workspace_root` canonicalizes, and on Windows `canonicalize` returns
/// the `\\?\C:\…` verbatim form. That is correct and most tools accept it, but
/// it is not what anyone typed and it is not what they want to copy out of a
/// "created X at …" line. Strip the prefix for display only — never for
/// anything that touches the filesystem, where the verbatim form is the one
/// that survives long paths.
fn shown(path: &Path) -> String {
    let s = path.display().to_string();
    match s.strip_prefix(r"\\?\UNC\") {
        Some(rest) => format!(r"\\{rest}"),
        None => s.strip_prefix(r"\\?\").unwrap_or(&s).to_owned(),
    }
}

fn today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

fn parse_date(raw: &str) -> anyhow::Result<NaiveDate> {
    raw.parse()
        .with_context(|| format!("not a YYYY-MM-DD date: {raw:?}"))
}

fn parse_account_type(raw: &str) -> anyhow::Result<AccountType> {
    Ok(match raw {
        "Asset" => AccountType::Asset,
        "Liability" => AccountType::Liability,
        "Equity" => AccountType::Equity,
        "Income" => AccountType::Income,
        "Expense" => AccountType::Expense,
        other => bail!(
            "unknown account type {other:?}; expected one of \
             Asset, Liability, Equity, Income, Expense"
        ),
    })
}

fn parse_status(raw: &str) -> anyhow::Result<TransactionStatus> {
    Ok(match raw {
        "cleared" => TransactionStatus::Cleared,
        "pending" => TransactionStatus::Pending,
        other => bail!("unknown status {other:?}; expected `cleared` or `pending`"),
    })
}

/// Parse one `--balance` value: `<amount> <currency>`.
fn parse_balance(spec: &str) -> anyhow::Result<Balance> {
    let fields: Vec<&str> = spec.split_whitespace().collect();
    let [amount, currency] = fields.as_slice() else {
        bail!("expected `<amount> <currency>`, got {spec:?}");
    };
    Ok(Balance {
        amount: amount
            .parse()
            .with_context(|| format!("not a decimal amount: {amount:?} in {spec:?}"))?,
        currency: (*currency).to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_balance_takes_the_two_field_form() {
        let b = parse_balance("5000 JPY").expect("parse");
        assert_eq!(b.amount.to_string(), "5000");
        assert_eq!(b.currency, "JPY");
    }

    #[test]
    fn parse_balance_takes_a_negative_amount() {
        assert_eq!(
            parse_balance("-1200 JPY")
                .expect("parse")
                .amount
                .to_string(),
            "-1200"
        );
    }

    #[test]
    fn parse_balance_names_the_input_it_could_not_read() {
        let err = parse_balance("5000").expect_err("a currency is required");
        assert!(err.to_string().contains("5000"), "got: {err}");
    }

    #[test]
    fn parse_account_type_names_the_bad_input() {
        let err = parse_account_type("Bogus").expect_err("not a type");
        assert!(err.to_string().contains("Bogus"), "got: {err}");
    }

    #[test]
    fn shown_strips_the_windows_verbatim_prefix() {
        assert_eq!(
            shown(Path::new(
                r"\\?\C:\ledger\transactions\2026\09\a1b2c3d.toml"
            )),
            r"C:\ledger\transactions\2026\09\a1b2c3d.toml"
        );
    }

    #[test]
    fn shown_restores_a_unc_path_to_its_familiar_form() {
        assert_eq!(
            shown(Path::new(r"\\?\UNC\server\share\ledger")),
            r"\\server\share\ledger"
        );
    }

    #[test]
    fn shown_leaves_an_ordinary_path_alone() {
        assert_eq!(shown(Path::new("/home/me/ledger")), "/home/me/ledger");
        assert_eq!(shown(Path::new(r"C:\ledger")), r"C:\ledger");
    }
}
