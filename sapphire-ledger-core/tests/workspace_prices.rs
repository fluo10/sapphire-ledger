use sapphire_ledger_core::{init_workspace, load_workspace, price_relative_path, save_toml, PriceEntry};

#[test]
fn init_creates_the_prices_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_workspace(dir.path(), "JPY").expect("init");
    assert!(dir.path().join("prices").is_dir());
}

#[test]
fn price_path_is_year_month_id() {
    let date = "2026-05-21".parse().unwrap();
    assert_eq!(
        price_relative_path(date, "0a1b2c3"),
        std::path::Path::new("prices/2026/05/0a1b2c3.toml")
    );
}

#[test]
fn load_workspace_reads_price_entries() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_workspace(dir.path(), "JPY").expect("init");

    let entry = PriceEntry {
        id: "0a1b2c3".into(),
        date: "2026-05-21".parse().unwrap(),
        base: "USD".into(),
        quote: "JPY".into(),
        rate: "150".parse().unwrap(),
        source: Some("manual".into()),
        created_at: "2026-05-21T18:30:00+09:00".parse().unwrap(),
        updated_at: "2026-05-21T18:30:00+09:00".parse().unwrap(),
    };
    save_toml(&dir.path().join(price_relative_path(entry.date, &entry.id)), &entry).expect("save");

    let ws = load_workspace(dir.path()).expect("load");
    assert_eq!(ws.prices.len(), 1);
    assert_eq!(ws.prices[0].base, "USD");
}
