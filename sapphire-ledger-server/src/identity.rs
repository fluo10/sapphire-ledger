//! `user` and `device` commands: who a bearer key belongs to.
//!
//! A key authenticates a device, and every device belongs to a user. That
//! indirection is the one thing a shared ledger needs that a private one
//! does not: when a human and an AI both write to the same books, a record
//! has to be able to say which of them wrote it. `last_updated_by` (a later
//! task) will hold a device id and resolve it to a name via
//! `device.user_id -> user.name`; this module is where that identity is
//! created, listed, rotated and retired.
//!
//! There is no standalone key command. Every key names a device, so a key
//! nobody can attribute -- the thing this design exists to prevent -- simply
//! cannot be minted: `device add` is refused before it touches the key store
//! if the user it names does not exist.

use std::path::Path;

use anyhow::Context as _;
use chrono::Utc;
use sapphire_framework::registry::{Device, Devices, GrainId, Users};
use sapphire_framework::remote_server::{KeyEntry, KeyStore};
use sapphire_framework::workspace::Workspace;

use crate::cli::{Command, DeviceCommand, UserCommand};

/// How many times `device add` retries a generated id that collides with an
/// existing device. `Devices::add` deliberately does not retry itself -- a
/// collision is astronomically unlikely, and silently searching for a free
/// id there would risk masking a real bug -- so the retry lives here instead.
const DEVICE_ADD_RETRIES: usize = 8;

/// The key-store token prefix for every key this app mints.
const KEY_PREFIX: &str = "sl";

/// Run a `user` or `device` subcommand.
///
/// `stdout` is a machine contract: `device add` and `device rotate` print
/// the raw token there and nothing else. Every other line -- ids, reminders,
/// list output -- goes to stderr.
pub fn run(command: Command, ledger_dir: &Path, keys_path: &Path) -> anyhow::Result<()> {
    let workspace = Workspace::from_root(&sapphire_ledger_core::LEDGER_CTX, ledger_dir)
        .with_context(|| {
            format!(
                "{} is not a sapphire-ledger workspace",
                ledger_dir.display()
            )
        })?;

    match command {
        Command::User(cmd) => run_user(cmd, &workspace),
        Command::Device(cmd) => run_device(cmd, &workspace, keys_path),
    }
}

fn run_user(command: UserCommand, workspace: &Workspace) -> anyhow::Result<()> {
    match command {
        UserCommand::Add { name, description } => {
            let mut users =
                Users::load(&workspace.users_path()).context("failed to load users.toml")?;
            let user = users
                .add(&name, description)
                .with_context(|| format!("failed to add user {name:?}"))?;
            eprintln!("added user {} ({})", user.name, user.id);
            Ok(())
        }
        UserCommand::List => {
            let users =
                Users::load(&workspace.users_path()).context("failed to load users.toml")?;
            for user in users.entries() {
                let description = user
                    .description
                    .as_deref()
                    .map(|d| format!("  {d}"))
                    .unwrap_or_default();
                eprintln!("{}  {}{description}", user.id, user.name);
            }
            Ok(())
        }
    }
}

fn run_device(
    command: DeviceCommand,
    workspace: &Workspace,
    keys_path: &Path,
) -> anyhow::Result<()> {
    match command {
        DeviceCommand::Add {
            name,
            user,
            description,
            expires_in,
        } => {
            let users =
                Users::load(&workspace.users_path()).context("failed to load users.toml")?;
            // Resolve the user *first* and bail unchanged if it is unknown,
            // before touching the key store: a key naming an unknown user is
            // exactly what this design exists to prevent, so a failed lookup
            // here must mint nothing.
            let user_id = users
                .resolve(&user)
                .with_context(|| format!("unknown user {user:?}"))?
                .id;

            // Validate all input -- including `--expires-in` -- before the
            // first mutation. `devices.toml` is workspace content that syncs
            // to every machine, so a device add that fails partway through
            // must not leave a junk entry behind: a bad duration caught only
            // after `Devices::add` would take the name with it, and
            // `Devices::add` refuses a duplicate name outright, so even the
            // obvious retype would fail.
            let expires_at = parse_expires_in(expires_in.as_deref())?;

            let mut devices =
                Devices::load(&workspace.devices_path()).context("failed to load devices.toml")?;
            let device = add_device_retrying(&mut devices, &name, description, user_id)?;

            let mut keys = KeyStore::load(keys_path)
                .with_context(|| format!("failed to open the key file {}", keys_path.display()))?;
            let key = keys
                .generate(
                    KEY_PREFIX,
                    None,
                    Some(device.id),
                    Some(name.clone()),
                    expires_at,
                )
                .context("failed to mint a key for the new device")?;

            eprintln!("added device {} ({})", device.name, device.id);
            eprintln!("the token below is shown once and cannot be recovered -- store it now");
            println!("{}", key.token);
            Ok(())
        }
        DeviceCommand::List => {
            let devices =
                Devices::load(&workspace.devices_path()).context("failed to load devices.toml")?;
            let users =
                Users::load(&workspace.users_path()).context("failed to load users.toml")?;
            let keys = KeyStore::load(keys_path)
                .with_context(|| format!("failed to open the key file {}", keys_path.display()))?;

            for device in devices.entries() {
                let user_name = device
                    .user_id
                    .and_then(|id| users.get(id))
                    .map(|u| u.name.clone())
                    .unwrap_or_else(|| "-".to_string());
                let retired = if device.is_retired() { " retired" } else { "" };
                let masked = find_key_for_device(&keys, device.id)
                    .map(|k| mask_token(&k.token))
                    .unwrap_or_else(|| "-".to_string());
                eprintln!(
                    "{}  {}  user={user_name}{retired}  key={masked}",
                    device.id, device.name
                );
            }
            Ok(())
        }
        DeviceCommand::Rotate {
            selector,
            expires_in,
        } => {
            let devices =
                Devices::load(&workspace.devices_path()).context("failed to load devices.toml")?;
            let device = devices
                .resolve(&selector)
                .with_context(|| format!("no device matches {selector:?}"))?;

            let mut keys = KeyStore::load(keys_path)
                .with_context(|| format!("failed to open the key file {}", keys_path.display()))?;
            let key_id = find_key_for_device(&keys, device.id)
                .ok_or_else(|| anyhow::anyhow!("no key found for device {}", device.id))?
                .id;

            let expires_at = parse_expires_in(expires_in.as_deref())?;
            let rotated = keys
                .rotate(KEY_PREFIX, &key_id.to_string(), expires_at)
                .with_context(|| format!("failed to rotate the key for device {}", device.id))?;

            eprintln!("rotated the key for device {} ({})", device.name, device.id);
            println!("{}", rotated.token);
            Ok(())
        }
        DeviceCommand::Retire { selector } => {
            let mut devices =
                Devices::load(&workspace.devices_path()).context("failed to load devices.toml")?;
            let device = devices
                .retire(&selector)
                .with_context(|| format!("failed to retire device {selector:?}"))?;

            // A device with no key still retires: the key lookup here is
            // best-effort, not a precondition.
            let mut keys = KeyStore::load(keys_path)
                .with_context(|| format!("failed to open the key file {}", keys_path.display()))?;
            if let Some(key) = find_key_for_device(&keys, device.id) {
                let key_id = key.id.to_string();
                keys.revoke(&key_id).with_context(|| {
                    format!("failed to revoke the key for device {}", device.id)
                })?;
            }

            eprintln!("retired device {} ({})", device.name, device.id);
            Ok(())
        }
    }
}

/// Add a device, retrying a generated-id collision up to
/// [`DEVICE_ADD_RETRIES`] times. Any other failure (e.g. a duplicate name)
/// is returned immediately -- retrying it would only fail the same way.
fn add_device_retrying(
    devices: &mut Devices,
    name: &str,
    description: Option<String>,
    user_id: GrainId,
) -> anyhow::Result<Device> {
    let mut last_collision = None;
    for _ in 0..DEVICE_ADD_RETRIES {
        match devices.add(name, description.clone(), Some(user_id)) {
            Ok(device) => return Ok(device),
            Err(e) if e.to_string().contains("collides with an existing device") => {
                last_collision = Some(e);
            }
            Err(e) => return Err(e).with_context(|| format!("failed to add device {name:?}")),
        }
    }
    Err(anyhow::anyhow!(
        "failed to add device {name:?} after {DEVICE_ADD_RETRIES} attempts, each a generated-id \
         collision: {}",
        last_collision.expect("the loop only exits after at least one collision error"),
    ))
}

/// The key belonging to `device_id`, if one exists. Written once and shared
/// by `Rotate`, `Retire` and `List` so three commands don't each carry their
/// own copy of the same `find`.
fn find_key_for_device(keys: &KeyStore, device_id: GrainId) -> Option<&KeyEntry> {
    keys.entries()
        .iter()
        .find(|k| k.device_id == Some(device_id))
}

/// First four characters of a token, then an ellipsis. Enough to recognise a
/// key at a glance without printing anything usable.
fn mask_token(token: &str) -> String {
    let prefix: String = token.chars().take(4).collect();
    format!("{prefix}…")
}

fn parse_expires_in(s: Option<&str>) -> anyhow::Result<Option<chrono::DateTime<Utc>>> {
    let Some(s) = s else { return Ok(None) };
    let duration = parse_duration(s)?;
    // `DateTime<Utc> + Duration` panics on overflow rather than returning an
    // error, and `humantime` happily accepts a duration far outside chrono's
    // representable range (e.g. `1000000years`). Use the checked form so a
    // wild duration is a normal error, not a crash.
    let expires_at = Utc::now()
        .checked_add_signed(duration)
        .with_context(|| format!("{s:?} is too far in the future to represent"))?;
    Ok(Some(expires_at))
}

/// Parse a duration like `90d` or `12h` (anything `humantime::parse_duration`
/// accepts) into a [`chrono::Duration`].
pub fn parse_duration(s: &str) -> anyhow::Result<chrono::Duration> {
    let std_duration = humantime::parse_duration(s)
        .with_context(|| format!("{s:?} is not a valid duration (try e.g. \"90d\", \"12h\")"))?;
    chrono::Duration::from_std(std_duration)
        .with_context(|| format!("{s:?} is too large to represent"))
}
