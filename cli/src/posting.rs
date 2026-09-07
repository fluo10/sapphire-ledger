//! Parsing the `--posting` argument.
//!
//! One posting per flag, in the shape a person can type:
//!
//! ```text
//! Expenses:Food 1200 JPY
//! Assets:Cash:USD 100 USD @ 150 JPY
//! ```
//!
//! Three fields, whitespace-separated: the account, a signed decimal, and a
//! currency. An optional `@ <value> <currency>` suffix carries the inline price
//! that makes a cross-currency transaction balance.
//!
//! The account may be a name or an id — this parser does not care, and must
//! not: resolving it is `core::ops`' job, and duplicating the rule here is how
//! the two drift apart. Everything here decides is *shape*.

use anyhow::{Context as _, bail};
use sapphire_ledger_core::{Posting, Price};

/// Parse one `--posting` value.
///
/// Errors name the offending input, because a mistyped posting is the most
/// likely thing to go wrong at this prompt and "invalid posting" alone leaves
/// the user comparing four flags by eye.
pub fn parse(spec: &str) -> anyhow::Result<Posting> {
    let (body, price) = match spec.split_once('@') {
        Some((body, price)) => (body, Some(parse_price(price.trim(), spec)?)),
        None => (spec, None),
    };

    let fields: Vec<&str> = body.split_whitespace().collect();
    let [account, amount, currency] = fields.as_slice() else {
        bail!(
            "expected `<account> <amount> <currency>` with an optional \
             `@ <value> <currency>`, got {spec:?}"
        );
    };

    Ok(Posting {
        // Offered as a name. `ops` resolves it against the loaded accounts and
        // fills in the id, accepting an id here just as readily.
        account_id: None,
        account_name: Some((*account).to_owned()),
        amount: amount
            .parse()
            .with_context(|| format!("not a decimal amount: {amount:?} in {spec:?}"))?,
        currency: (*currency).to_owned(),
        price,
        memo: None,
    })
}

fn parse_price(spec: &str, whole: &str) -> anyhow::Result<Price> {
    let fields: Vec<&str> = spec.split_whitespace().collect();
    let [value, currency] = fields.as_slice() else {
        bail!("expected `@ <value> <currency>`, got {spec:?} in {whole:?}");
    };
    Ok(Price {
        value: value
            .parse()
            .with_context(|| format!("not a decimal price: {value:?} in {whole:?}"))?,
        currency: (*currency).to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_three_field_form() {
        let p = parse("Expenses:Food 1200 JPY").expect("parse");
        assert_eq!(p.account_name.as_deref(), Some("Expenses:Food"));
        assert_eq!(p.amount.to_string(), "1200");
        assert_eq!(p.currency, "JPY");
        assert!(p.price.is_none());
    }

    #[test]
    fn parses_a_negative_amount() {
        let p = parse("Assets:Cash -1200 JPY").expect("parse");
        assert_eq!(p.amount.to_string(), "-1200");
    }

    #[test]
    fn parses_the_inline_price_suffix() {
        let p = parse("Assets:Cash:USD 100 USD @ 150 JPY").expect("parse");
        assert_eq!(p.amount.to_string(), "100");
        assert_eq!(p.currency, "USD");
        let price = p.price.expect("a price");
        assert_eq!(price.value.to_string(), "150");
        assert_eq!(price.currency, "JPY");
    }

    #[test]
    fn tolerates_extra_whitespace_around_every_field() {
        let p = parse("  Expenses:Food   1200   JPY  @  150   USD ").expect("parse");
        assert_eq!(p.account_name.as_deref(), Some("Expenses:Food"));
        assert_eq!(p.price.expect("a price").currency, "USD");
    }

    #[test]
    fn an_account_id_parses_as_readily_as_a_name() {
        // The parser must not try to tell them apart — `ops` resolves either,
        // and a shape check here would be a second place for that rule to live.
        let p = parse("0a1b2c3 1200 JPY").expect("parse");
        assert_eq!(p.account_name.as_deref(), Some("0a1b2c3"));
    }

    #[test]
    fn too_few_fields_names_the_input() {
        let err = parse("Expenses:Food 1200").expect_err("three fields are required");
        let msg = err.to_string();
        assert!(
            msg.contains("Expenses:Food 1200"),
            "the error must quote what was typed, got: {msg}"
        );
    }

    #[test]
    fn too_many_fields_is_refused() {
        parse("Expenses:Food 1200 JPY extra").expect_err("a fourth field is not a posting");
    }

    #[test]
    fn a_non_decimal_amount_names_the_offending_field() {
        let err = parse("Expenses:Food lots JPY").expect_err("not a number");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("lots"),
            "the error must name the field that failed, got: {msg}"
        );
    }

    #[test]
    fn a_malformed_price_suffix_is_refused() {
        parse("Expenses:Food 1200 JPY @ 150").expect_err("a price needs a currency");
        parse("Expenses:Food 1200 JPY @").expect_err("an empty price is not a price");
    }
}
