use sapphire_ledger_core::{Account, AccountType, Assertion, Balance, Posting, Transaction};

fn account(id: &str, name: &str) -> Account {
    Account {
        id: id.to_string(),
        name: name.to_string(),
        account_type: AccountType::Expense,
        currencies: vec![],
        opened_at: "2026-01-01".parse().unwrap(),
        closed_at: None,
        description: None,
    }
}

fn posting(id: Option<&str>, name: Option<&str>, amount: &str) -> Posting {
    Posting {
        account_id: id.map(str::to_string),
        account_name: name.map(str::to_string),
        amount: amount.parse().unwrap(),
        currency: "JPY".into(),
        price: None,
        memo: None,
    }
}

fn transaction(postings: Vec<Posting>) -> Transaction {
    Transaction {
        id: "tx00001".into(),
        date: "2026-05-21".parse().unwrap(),
        narration: "test".into(),
        payee: None,
        tags: vec![],
        status: None,
        created_at: "2026-05-21T18:30:00+09:00".parse().unwrap(),
        updated_at: "2026-05-21T18:30:00+09:00".parse().unwrap(),
        postings,
    }
}

fn workspace(accounts: Vec<Account>, transactions: Vec<Transaction>, assertions: Vec<Assertion>)
    -> sapphire_ledger_core::Workspace
{
    sapphire_ledger_core::Workspace {
        root: std::path::PathBuf::from("/nonexistent"),
        config: sapphire_ledger_core::Config {
            schema_version: sapphire_ledger_core::CURRENT_SCHEMA_VERSION,
            base_currency: "JPY".into(),
            cache: Default::default(),
        },
        accounts,
        transactions,
        assertions,
        prices: vec![],
    }
}

#[test]
fn a_posting_resolves_by_id_even_when_the_name_is_stale() {
    let ws = workspace(
        vec![account("acct001", "Expenses:Groceries")],
        vec![transaction(vec![
            posting(Some("acct001"), Some("Expenses:Food"), "1200"),
            posting(Some("acct001"), Some("Expenses:Food"), "-1200"),
        ])],
        vec![],
    );
    assert!(
        ws.validate().is_empty(),
        "a renamed account must not make old postings invalid: {:?}",
        ws.validate()
    );
}

#[test]
fn a_posting_resolves_by_name_when_no_id_is_given() {
    let ws = workspace(
        vec![account("acct001", "Expenses:Food")],
        vec![transaction(vec![
            posting(None, Some("Expenses:Food"), "1200"),
            posting(None, Some("Expenses:Food"), "-1200"),
        ])],
        vec![],
    );
    assert!(ws.validate().is_empty(), "{:?}", ws.validate());
}

#[test]
fn a_posting_with_neither_reference_is_an_error() {
    let ws = workspace(
        vec![account("acct001", "Expenses:Food")],
        vec![transaction(vec![
            posting(None, None, "1200"),
            posting(None, None, "-1200"),
        ])],
        vec![],
    );
    let issues = ws.validate();
    assert!(
        issues.iter().any(|i| i.message.contains("no account reference")),
        "{issues:?}"
    );
}

#[test]
fn an_unknown_account_id_is_an_error() {
    let ws = workspace(
        vec![account("acct001", "Expenses:Food")],
        vec![transaction(vec![
            posting(Some("nosuch"), Some("Expenses:Food"), "1200"),
            posting(Some("acct001"), None, "-1200"),
        ])],
        vec![],
    );
    let issues = ws.validate();
    assert!(
        issues.iter().any(|i| i.message.contains("undefined account")),
        "an id that resolves to nothing must fail even when the name would have \
         resolved -- the id is authoritative: {issues:?}"
    );
}

#[test]
fn duplicate_account_ids_are_an_error() {
    let ws = workspace(
        vec![account("acct001", "Expenses:Food"), account("acct001", "Expenses:Other")],
        vec![],
        vec![],
    );
    let issues = ws.validate();
    assert!(
        issues.iter().any(|i| i.message.contains("duplicate account id")),
        "a rename racing under sync can put one id at two paths: {issues:?}"
    );
}

#[test]
fn an_assertion_resolves_by_id_too() {
    let ws = workspace(
        vec![account("acct001", "Assets:Cash:JPY")],
        vec![],
        vec![Assertion {
            id: "as00001".into(),
            account_id: Some("acct001".into()),
            account_name: Some("Assets:Old:Name".into()),
            date: "2026-05-31".parse().unwrap(),
            balances: vec![Balance { amount: "5000".parse().unwrap(), currency: "JPY".into() }],
            created_at: "2026-05-31T23:59:00+09:00".parse().unwrap(),
            updated_at: "2026-05-31T23:59:00+09:00".parse().unwrap(),
        }],
    );
    assert!(ws.validate().is_empty(), "{:?}", ws.validate());
}
