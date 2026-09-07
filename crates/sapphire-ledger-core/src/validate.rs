use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::account::Account;
use crate::error::Error;
use crate::repository::Workspace;

#[derive(Debug, Clone, Serialize)]
pub struct ValidationIssue {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assertion_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

impl Workspace {
    /// Run all cross-record validations. Returns every issue found, never
    /// short-circuiting, so the caller can present a complete report.
    pub fn validate(&self) -> Vec<ValidationIssue> {
        use crate::account::{describe_ref, resolve_account};

        let mut issues = Vec::new();
        let mut by_id: HashMap<&str, &Account> = HashMap::new();
        let mut by_name: HashMap<&str, &Account> = HashMap::new();

        for account in &self.accounts {
            // Two accounts under one name is not the stale-name case the design
            // accepts: a name-only posting would resolve to whichever one
            // happened to win the `by_name` insert.
            if by_name.insert(account.name.as_str(), account).is_some() {
                issues.push(ValidationIssue {
                    message: format!("duplicate account name {}", account.name),
                    transaction_id: None,
                    assertion_id: None,
                    account: Some(account.name.clone()),
                });
            }
            // A rename is a file move; if that races under record-level sync,
            // one id can end up at two paths. Unlike a stale name, that is
            // genuinely broken.
            if by_id.insert(account.id.as_str(), account).is_some() {
                issues.push(ValidationIssue {
                    message: format!("duplicate account id {}", account.id),
                    transaction_id: None,
                    assertion_id: None,
                    account: Some(account.name.clone()),
                });
            }
        }

        // Ids are unique per record kind. Sync resolves per path,
        // last-writer-wins, so a record moved between months -- a corrected
        // date, a merge -- is a delete-plus-add that can race and leave one id
        // at two paths. This is also the check that would catch a minting bug.
        let mut seen_transaction_ids: HashSet<&str> = HashSet::new();
        let mut seen_assertion_ids: HashSet<&str> = HashSet::new();
        let mut seen_price_ids: HashSet<&str> = HashSet::new();

        for tx in &self.transactions {
            if !seen_transaction_ids.insert(tx.id.as_str()) {
                issues.push(ValidationIssue {
                    message: format!("duplicate transaction id {}", tx.id),
                    transaction_id: Some(tx.id.clone()),
                    assertion_id: None,
                    account: None,
                });
            }

            if let Err(err) = tx.validate() {
                issues.push(ValidationIssue {
                    message: render(&err),
                    transaction_id: Some(tx.id.clone()),
                    assertion_id: None,
                    account: None,
                });
            }

            for posting in &tx.postings {
                let id = posting.account_id.as_deref();
                let name = posting.account_name.as_deref();
                match resolve_account(id, name, &by_id, &by_name) {
                    Err(err) => issues.push(ValidationIssue {
                        message: format!("transaction {}: {}", tx.id, render(&err)),
                        transaction_id: Some(tx.id.clone()),
                        assertion_id: None,
                        account: Some(describe_ref(id, name)),
                    }),
                    Ok(account) => {
                        if !account.allows_currency(&posting.currency) {
                            issues.push(ValidationIssue {
                                message: format!(
                                    "transaction {} posts {} to {}, but that account only allows {}",
                                    tx.id,
                                    posting.currency,
                                    account.name,
                                    account.currencies.join(", "),
                                ),
                                transaction_id: Some(tx.id.clone()),
                                assertion_id: None,
                                account: Some(account.name.clone()),
                            });
                        }
                    }
                }
            }
        }

        for assertion in &self.assertions {
            if !seen_assertion_ids.insert(assertion.id.as_str()) {
                issues.push(ValidationIssue {
                    message: format!("duplicate assertion id {}", assertion.id),
                    transaction_id: None,
                    assertion_id: Some(assertion.id.clone()),
                    account: None,
                });
            }

            let id = assertion.account_id.as_deref();
            let name = assertion.account_name.as_deref();
            match resolve_account(id, name, &by_id, &by_name) {
                Err(err) => issues.push(ValidationIssue {
                    message: format!("assertion {}: {}", assertion.id, render(&err)),
                    transaction_id: None,
                    assertion_id: Some(assertion.id.clone()),
                    account: Some(describe_ref(id, name)),
                }),
                Ok(account) => {
                    for balance in &assertion.balances {
                        if !account.allows_currency(&balance.currency) {
                            issues.push(ValidationIssue {
                                message: format!(
                                    "assertion {} asserts {} balance for {}, but that account only allows {}",
                                    assertion.id,
                                    balance.currency,
                                    account.name,
                                    account.currencies.join(", "),
                                ),
                                transaction_id: None,
                                assertion_id: Some(assertion.id.clone()),
                                account: Some(account.name.clone()),
                            });
                        }
                    }
                }
            }
        }

        // A price entry carries no cross-record reference, so its id is the
        // only thing this pass can check.
        for entry in &self.prices {
            if !seen_price_ids.insert(entry.id.as_str()) {
                issues.push(ValidationIssue {
                    message: format!("duplicate price id {}", entry.id),
                    transaction_id: None,
                    assertion_id: None,
                    account: None,
                });
            }
        }

        issues
    }
}

fn render(err: &Error) -> String {
    match err {
        Error::Validation(msg) => msg.clone(),
        other => other.to_string(),
    }
}
