use chrono::{DateTime, FixedOffset, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransactionStatus {
    Cleared,
    Pending,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Price {
    #[serde(with = "rust_decimal::serde::str")]
    pub value: Decimal,
    pub currency: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Posting {
    pub account: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub amount: Decimal,
    pub currency: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<Price>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
}

impl Posting {
    /// The (currency, signed amount) contribution this posting makes to the
    /// transaction-level balance check. If a price is set, the amount is
    /// converted into the price's currency.
    pub fn balance_contribution(&self) -> (String, Decimal) {
        match &self.price {
            Some(price) => (price.currency.clone(), self.amount * price.value),
            None => (self.currency.clone(), self.amount),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction {
    pub id: String,
    pub date: NaiveDate,
    pub narration: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payee: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<TransactionStatus>,
    pub created_at: DateTime<FixedOffset>,
    pub updated_at: DateTime<FixedOffset>,
    pub postings: Vec<Posting>,
}

impl Transaction {
    /// Verify the transaction is well-formed:
    /// - at least 2 postings
    /// - per-currency (after inline price conversion) sums to zero
    pub fn validate(&self) -> Result<()> {
        if self.postings.len() < 2 {
            return Err(Error::Validation(format!(
                "transaction {} has fewer than 2 postings",
                self.id
            )));
        }

        let mut totals: BTreeMap<String, Decimal> = BTreeMap::new();
        for posting in &self.postings {
            let (currency, contribution) = posting.balance_contribution();
            *totals.entry(currency).or_insert(Decimal::ZERO) += contribution;
        }

        for (currency, total) in &totals {
            if !total.is_zero() {
                return Err(Error::Validation(format!(
                    "transaction {} does not balance in {}: net {}",
                    self.id, currency, total
                )));
            }
        }

        Ok(())
    }
}
