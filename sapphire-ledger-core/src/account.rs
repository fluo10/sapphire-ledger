use std::collections::HashMap;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

pub const ACCOUNT_NAME_SEPARATOR: char = ':';

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AccountType {
    Asset,
    Liability,
    Equity,
    Income,
    Expense,
}

/// Split an account name like `"Assets:Cash:USD"` into its colon-separated
/// segments after validating shape: non-empty name, non-empty segments, no
/// `.`/`..`, no path separators inside a segment.
pub fn account_name_segments(name: &str) -> Result<Vec<&str>> {
    if name.is_empty() {
        return Err(Error::Validation("account name is empty".into()));
    }
    let segments: Vec<&str> = name.split(ACCOUNT_NAME_SEPARATOR).collect();
    for segment in &segments {
        if segment.is_empty() {
            return Err(Error::Validation(format!(
                "account name has empty segment: {name}"
            )));
        }
        if *segment == "." || *segment == ".." {
            return Err(Error::Validation(format!(
                "account name segment cannot be `.` or `..`: {name}"
            )));
        }
        if segment.contains('/') || segment.contains('\\') {
            return Err(Error::Validation(format!(
                "account name segment contains a path separator: {name}"
            )));
        }
    }
    Ok(segments)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub account_type: AccountType,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub currencies: Vec<String>,
    pub opened_at: NaiveDate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<NaiveDate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Account {
    pub fn allows_currency(&self, currency: &str) -> bool {
        self.currencies.is_empty() || self.currencies.iter().any(|c| c == currency)
    }
}

/// Render an account reference for an error message, whichever half was given.
pub fn describe_ref(id: Option<&str>, name: Option<&str>) -> String {
    match (id, name) {
        (Some(id), Some(name)) => format!("{name} ({id})"),
        (Some(id), None) => id.to_string(),
        (None, Some(name)) => name.to_string(),
        (None, None) => "<no account reference>".to_string(),
    }
}

/// Resolve an account reference.
///
/// The id is authoritative: when it is present the name is not consulted at
/// all, which is what lets a rename leave old records alone. A name-only
/// reference is resolved by name, so hand-written TOML stays valid.
pub fn resolve_account<'a>(
    id: Option<&str>,
    name: Option<&str>,
    by_id: &HashMap<&str, &'a Account>,
    by_name: &HashMap<&str, &'a Account>,
) -> Result<&'a Account> {
    match (id, name) {
        (Some(id), _) => by_id.get(id).copied().ok_or_else(|| {
            Error::Validation(format!("undefined account id {id}"))
        }),
        (None, Some(name)) => by_name.get(name).copied().ok_or_else(|| {
            Error::Validation(format!("undefined account {name}"))
        }),
        (None, None) => Err(Error::Validation(
            "posting or assertion has no account reference".into(),
        )),
    }
}
