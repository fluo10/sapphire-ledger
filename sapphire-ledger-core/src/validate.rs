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
        let mut issues = Vec::new();
        let accounts_by_name: HashMap<&str, &Account> = self
            .accounts
            .iter()
            .map(|a| (a.name.as_str(), a))
            .collect();

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
                match accounts_by_name.get(posting.account.as_str()) {
                    None => issues.push(ValidationIssue {
                        message: format!(
                            "transaction {} references undefined account {}",
                            tx.id, posting.account
                        ),
                        transaction_id: Some(tx.id.clone()),
                        assertion_id: None,
                        account: Some(posting.account.clone()),
                    }),
                    Some(account) => {
                        if !account.allows_currency(&posting.currency) {
                            issues.push(ValidationIssue {
                                message: format!(
                                    "transaction {} posts {} to {}, but that account only allows {}",
                                    tx.id,
                                    posting.currency,
                                    posting.account,
                                    account.currencies.join(", "),
                                ),
                                transaction_id: Some(tx.id.clone()),
                                assertion_id: None,
                                account: Some(posting.account.clone()),
                            });
                        }
                    }
                }
            }
        }

        for assertion in &self.assertions {
            match accounts_by_name.get(assertion.account.as_str()) {
                None => issues.push(ValidationIssue {
                    message: format!(
                        "assertion {} references undefined account {}",
                        assertion.id, assertion.account
                    ),
                    transaction_id: None,
                    assertion_id: Some(assertion.id.clone()),
                    account: Some(assertion.account.clone()),
                }),
                Some(account) => {
                    for balance in &assertion.balances {
                        if !account.allows_currency(&balance.currency) {
                            issues.push(ValidationIssue {
                                message: format!(
                                    "assertion {} asserts {} balance for {}, but that account only allows {}",
                                    assertion.id,
                                    balance.currency,
                                    assertion.account,
                                    account.currencies.join(", "),
                                ),
                                transaction_id: None,
                                assertion_id: Some(assertion.id.clone()),
                                account: Some(assertion.account.clone()),
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
