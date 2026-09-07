//! Exchange rates, in two unrelated shapes.
//!
//! [`Price`] is the inline price on a single posting, used to make a
//! cross-currency transaction balance. [`PriceEntry`] is a standalone record
//! in the price log, used to report in a base currency at a past date. They
//! share a domain but not a lifecycle, and one of them is part of the
//! published MCP tool schema — so they are separate types.

use chrono::{DateTime, FixedOffset, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// The inline price on a posting: "this amount, valued at `value` `currency`".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Price {
    #[serde(with = "rust_decimal::serde::str")]
    pub value: Decimal,
    pub currency: String,
}

/// One observed exchange rate: `1 base = rate quote` on `date`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceEntry {
    pub id: String,
    pub date: NaiveDate,
    pub base: String,
    pub quote: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub rate: Decimal,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub created_at: DateTime<FixedOffset>,
    pub updated_at: DateTime<FixedOffset>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn price_entry_round_trips_through_toml() {
        let toml_text = r#"
id = "0a1b2c3"
date = "2026-05-21"
base = "USD"
quote = "JPY"
rate = "150.5"
source = "manual"
created_at = "2026-05-21T18:30:00+09:00"
updated_at = "2026-05-21T18:30:00+09:00"
"#;
        let entry: PriceEntry = toml::from_str(toml_text).expect("parse");
        assert_eq!(entry.rate.to_string(), "150.5");

        let rendered = toml::to_string_pretty(&entry).expect("serialize");
        let round_tripped: PriceEntry = toml::from_str(&rendered).expect("reparse");
        assert_eq!(entry, round_tripped);
    }

    #[test]
    fn price_entry_omits_absent_source() {
        let entry = PriceEntry {
            id: "0a1b2c3".into(),
            date: "2026-05-21".parse().unwrap(),
            base: "USD".into(),
            quote: "JPY".into(),
            rate: "150".parse().unwrap(),
            source: None,
            created_at: "2026-05-21T18:30:00+09:00".parse().unwrap(),
            updated_at: "2026-05-21T18:30:00+09:00".parse().unwrap(),
        };
        let rendered = toml::to_string_pretty(&entry).expect("serialize");
        assert!(!rendered.contains("source"), "got: {rendered}");
    }
}
