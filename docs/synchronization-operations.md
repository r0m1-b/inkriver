# InkRiver synchronization operations

This guide covers backup, recovery, device loss, diagnostics, privacy and the
implemented synchronization limits. It complements the
[protocol contract](synchronization.md); commands must be run against the
intended installation, not blindly copied between the CLI and graphical app.

## What each storage layer can recover

| Storage | Contains | Does not contain |
| --- | --- | --- |
| `inkriver.db` | subscriptions, article cache and local states, synchronization journal and projections, device UUID, non-secret WebDAV configuration | group key and WebDAV password |
| Native vault | 32-byte group key and WebDAV password | articles, journal and device roster |
| WebDAV directory | encrypted events, acknowledgements, rosters and recovery checkpoints | article bodies, feed logos, interface settings and plaintext business data |

A SQLite backup is the only complete copy of cached article bodies. WebDAV can
reconstruct subscriptions and synchronized user states when the group key is
still available, but article bodies must be fetched again from their feeds.
Restoring only SQLite preserves its device UUID; restoring or retaining the
matching native-vault entry is also required for that device to continue using
its existing synchronization configuration.

## Back up and restore SQLite

Close InkRiver before manipulating its files. On Linux, the graphical database
is usually:

```text
~/.local/share/io.github.r0m1-b.inkriver/inkriver.db
```

The development CLI uses `inkriver.db` at the repository root unless
`--database` selects another file. Android keeps the database in private
application storage; use a platform backup that preserves the complete app data
and native credential store if the device vendor explicitly guarantees both.
Do not assume an Android backup preserves Keystore keys; WebDAV recovery is the
portable procedure. InkRiver currently has no raw SQLite export on Android.

Create a consistent Linux backup with SQLite's backup API:

```bash
sqlite3 "$HOME/.local/share/io.github.r0m1-b.inkriver/inkriver.db" \
  ".backup '$HOME/inkriver-backup.db'"
sqlite3 "$HOME/inkriver-backup.db" "PRAGMA quick_check;"
```

`quick_check` must print `ok`. A plain copy is safe only while InkRiver and all
SQLite clients are stopped. Never copy just the main file while a `-wal` file
may still contain committed transactions.

To restore on Linux:

1. close InkRiver and keep a separate copy of the current database;
2. verify the backup with `PRAGMA quick_check`;
3. replace `inkriver.db` with the backup while no process has it open;
4. remove stale `inkriver.db-wal` and `inkriver.db-shm` files belonging to the
   replaced database;
5. start the same or a newer InkRiver version so embedded migrations can run;
6. verify the subscriptions and synchronization status before making changes.

Do not restore an older application over a database migrated by a newer one.
The database contains the device UUID, so cloning the same backup onto two live
installations would make both write the same journal identity and is forbidden.
A restored device whose UUID was revoked remains revoked permanently and must
instead join as a fresh installation with a new UUID.

## Rebuild an installation from WebDAV

Use this procedure when local SQLite is lost but one trusted paired device and
the remote synchronization directory still exist:

1. install InkRiver, which creates a fresh SQLite database and device UUID;
2. on a surviving device, open **Subscriptions → Synchronization** and create a
   pairing invitation;
3. join from the new installation with the QR code or invitation and enter the
   WebDAV password separately;
4. run **Synchronize now**, repeating bounded cycles if the UI still reports
   pending work;
5. verify that subscriptions and read/favorite/archive states are present;
6. refresh the feeds to repopulate article bodies that WebDAV never stores.

The importer authenticates a recovery checkpoint before applying it, then
continues with newer immutable segments. Do not manually copy, rename or edit
files in the WebDAV tree. A saved invitation can supply the group key, but it
must be protected like a password and the WebDAV password is still required.

## Lost, stolen or reinstalled device

From a surviving trusted device, select the missing device in the
**Synchronization** dialog, choose **Revoke**, then complete a successful
synchronization. Repeat synchronization on the other active devices so the
monotonic roster reaches each of them. Revocation is permanent for that UUID:
old roster documents cannot reactivate it and future segments from it are
ignored.

This is logical revocation, not cryptographic exclusion. A stolen device may
still possess the group key, previously downloaded data and WebDAV credentials.
Change the WebDAV password at the provider if those credentials are exposed,
then reconfigure trusted devices. Group-key rotation is not implemented, so a
device that obtained the key cannot currently be excluded from ciphertext that
it can still access.

Uninstalling or clearing app data without restoring the full installation
creates a new random UUID. Pair it as a new device; never try to reuse the old
UUID. Revoke the old entry when it will not return. Restoring one coherent
SQLite backup is different: it intentionally retains the backed-up UUID.

## Loss scenarios

| Situation | Recovery |
| --- | --- |
| Group key lost on one device | Pair again from a trusted device that still has the key, after removing the broken local synchronization configuration if necessary. |
| No device or protected invitation retains the group key | Remote ciphertext is unrecoverable. A surviving SQLite database can seed a new synchronization group; without SQLite too, synchronized state is lost. |
| WebDAV directory lost, SQLite survives | Keep the most complete database, create a new dedicated WebDAV group/directory, pair the other devices again and complete synchronization before treating it as the new recovery copy. |
| WebDAV and every SQLite copy lost | InkRiver cannot recover subscriptions, states or cached content. Feeds may be re-added manually, but prior read/favorite/archive history is gone. |
| WebDAV password lost | Reset it with the WebDAV provider. The password does not decrypt data; the group key does. Trusted devices must then be configured with the new password. |

Before deleting a damaged configuration or remote directory, preserve the
remaining SQLite database and export a redacted diagnostic.

## Metadata visible to the WebDAV host

Business payloads are encrypted and authenticated with XChaCha20-Poly1305, but
encryption does not hide all storage and traffic metadata. The WebDAV provider
can observe:

- the account, client IP addresses, request times and transferred byte counts;
- the stable group-key fingerprint used as a directory name;
- device UUIDs in segment, checkpoint, acknowledgement and roster paths;
- segment sequence ranges, protocol/envelope versions, ciphertext sizes and
  therefore approximate activity and compaction patterns;
- the authenticated checkpoint state hash, which lets the host correlate an
  unchanged or replaced checkpoint without revealing its state;
- the number of device documents, checkpoints and retained segments;
- temporary uploads and subsequent atomic `MOVE`/deletion operations.

The host cannot read encrypted subscription URLs, article metadata, state
changes, device display names or roster contents without the group key. Article
bodies and WebDAV credentials are never placed inside synchronization
documents. HTTPS is still required: plain HTTP exposes Basic Authentication and
traffic to network observers even though InkRiver payloads remain encrypted.

## Implemented limits and retention

| Area | Current bound |
| --- | --- |
| Immutable segment | at most 250 events and 2 MiB |
| One synchronization cycle | at most 20 segment downloads, four concurrent; at most eight checkpoint downloads |
| Event processing/compaction | at most 1,000 imported/read events and 1,000 compacted local events per bounded pass |
| Remote cleanup | at most 20 safe local-device segments per cycle |
| Recovery checkpoint | at most 10,000 retained events, 5 MiB plaintext state and 8 MiB encrypted document |
| Devices/control plane | at most 256 roster members, discovered rosters, acknowledgements, acknowledged sources and checkpoint frontiers |
| Control documents | roster and acknowledgement each limited to 256 KiB |
| WebDAV listing | at most 1,000 entries and 1 MiB per `PROPFIND`; 10 s connect and 20 s request timeout |
| Local article retention | after 30 days, only dated articles that are read and not favorite are locally archived and have their cached body released |

When a checkpoint exceeds its bounds, synchronization continues without
publishing that checkpoint, but compaction and remote deletion that require it
remain blocked. Bounded work progresses over repeated cycles. There is no fixed
age-based retention for encrypted synchronization history: safe checkpoint,
roster and acknowledgement proofs govern cleanup.

## Diagnostics and verification

In the graphical app, use **Subscriptions → Synchronization → Save diagnostic**.
The JSON excludes credentials, URLs, UUIDs, device names and article content.
Review it before sharing it.

For the development CLI, select the correct database explicitly when needed:

```bash
cargo run -- --database /path/to/inkriver.db sync-diagnostic \
  > inkriver-sync-diagnostic.json
sqlite3 /path/to/inkriver.db "PRAGMA quick_check;"
sqlite3 /path/to/inkriver.db "PRAGMA foreign_key_check;"
```

Successful checks print `ok` for `quick_check` and no rows for
`foreign_key_check`. These are read-only checks. Also record the app version,
platform, time of the last successful synchronization and exact error stage.
Never publish the raw database, native-vault contents, pairing invitation or an
unredacted WebDAV listing in a support ticket.
