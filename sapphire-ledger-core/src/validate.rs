use std::collections::HashMap;

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
            by_name.insert(account.name.as_str(), account);
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

        for tx in &self.transactions {
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

        issues
    }
}

fn render(err: &Error) -> String {
    match err {
        Error::Validation(msg) => msg.clone(),
        other => other.to_string(),
    }
}
