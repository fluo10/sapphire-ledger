use std::path::PathBuf;

use sapphire_ledger_server::cli::{Command, DeviceCommand, UserCommand};
use sapphire_ledger_server::identity;

struct Fixture {
    _dir: tempfile::TempDir,
    ledger: PathBuf,
    keys: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = dir.path().join("ledger");
    sapphire_ledger_core::init_workspace(&ledger, "JPY").expect("init");
    let keys = dir.path().join("keys.toml");
    Fixture {
        _dir: dir,
        ledger,
        keys,
    }
}

fn run(f: &Fixture, command: Command) -> anyhow::Result<()> {
    identity::run(command, &f.ledger, &f.keys)
}

fn add_user(f: &Fixture, name: &str) {
    run(
        f,
        Command::User(UserCommand::Add {
            name: name.to_string(),
            description: None,
        }),
    )
    .expect("user add");
}

fn add_device(f: &Fixture, name: &str, user: &str) -> anyhow::Result<()> {
    run(
        f,
        Command::Device(DeviceCommand::Add {
            name: name.to_string(),
            user: user.to_string(),
            description: None,
            expires_in: None,
        }),
    )
}

fn workspace(f: &Fixture) -> sapphire_framework::workspace::Workspace {
    // `LEDGER_CTX` is a `pub static`, which is exactly the `&'static AppContext`
    // this takes. `from_root` requires the `.sapphire-ledger` marker directory,
    // which `init_workspace` created.
    sapphire_framework::workspace::Workspace::from_root(
        &sapphire_ledger_core::LEDGER_CTX,
        &f.ledger,
    )
    .expect("workspace")
}

fn devices(f: &Fixture) -> sapphire_framework::registry::Devices {
    sapphire_framework::registry::Devices::load(&workspace(f).devices_path()).expect("devices")
}

fn keys(f: &Fixture) -> sapphire_framework::remote_server::KeyStore {
    sapphire_framework::remote_server::KeyStore::load(&f.keys).expect("keys")
}

#[test]
fn a_device_is_registered_and_gets_exactly_one_key() {
    let f = fixture();
    add_user(&f, "me");
    add_device(&f, "laptop", "me").expect("device add");

    let devices = devices(&f);
    let device = devices.resolve("laptop").expect("device exists");
    assert!(!device.is_retired());

    let keys = keys(&f);
    let entries: Vec<_> = keys
        .entries()
        .iter()
        .filter(|k| k.device_id == Some(device.id))
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "a device gets one key, no more and no fewer"
    );
}

#[test]
fn a_device_under_an_unknown_user_is_refused_and_mints_no_key() {
    let f = fixture();
    add_device(&f, "laptop", "nobody").expect_err("unknown user must be refused");
    assert!(
        keys(&f).entries().is_empty(),
        "a refused device must leave no key behind -- an unattributable key is \
         exactly what this design exists to prevent"
    );
}

#[test]
fn rotate_keeps_the_device_and_changes_the_token() {
    let f = fixture();
    add_user(&f, "me");
    add_device(&f, "laptop", "me").expect("device add");

    let device_id = devices(&f).resolve("laptop").expect("device").id;
    let before = keys(&f)
        .entries()
        .iter()
        .find(|k| k.device_id == Some(device_id))
        .expect("key")
        .token
        .clone();

    run(
        &f,
        Command::Device(DeviceCommand::Rotate {
            selector: "laptop".into(),
            expires_in: None,
        }),
    )
    .expect("rotate");

    let after_device = devices(&f).resolve("laptop").expect("device").id;
    let after = keys(&f)
        .entries()
        .iter()
        .find(|k| k.device_id == Some(device_id))
        .expect("key")
        .token
        .clone();

    assert_eq!(
        after_device, device_id,
        "rotation keeps the device's identity"
    );
    assert_ne!(after, before, "rotation replaces the token");
}

#[test]
fn a_retired_device_keeps_resolving_to_its_name() {
    let f = fixture();
    add_user(&f, "me");
    add_device(&f, "laptop", "me").expect("device add");
    let device_id = devices(&f).resolve("laptop").expect("device").id;
    let token = keys(&f)
        .entries()
        .iter()
        .find(|k| k.device_id == Some(device_id))
        .expect("key")
        .token
        .clone();

    run(
        &f,
        Command::Device(DeviceCommand::Retire {
            selector: "laptop".into(),
        }),
    )
    .expect("retire");

    let devices = devices(&f);
    let device = devices.get(device_id).expect(
        "a retired device is a tombstone, not a deletion -- a record that named \
         this id must still resolve to a name",
    );
    assert!(device.is_retired());
    assert_eq!(device.name, "laptop");

    assert!(
        keys(&f).authenticate(&token).is_none(),
        "retiring a device must revoke its key -- its real token, not a bogus \
         one, must stop authenticating"
    );
}

#[test]
fn parse_duration_accepts_the_documented_forms() {
    assert_eq!(identity::parse_duration("90d").unwrap().num_days(), 90);
    assert_eq!(identity::parse_duration("12h").unwrap().num_hours(), 12);
    identity::parse_duration("soon").expect_err("not a duration");
}
