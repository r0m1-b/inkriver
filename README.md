# InkRiver

**English** | [Français](README.fr.md)

InkRiver is an RSS/Atom feed reader written in Rust. It brings Medium, Substack,
and other compatible subscriptions together in a single timeline and makes
articles available offline.

The project includes a command-line application and a Tauri 2 desktop
application for Linux. Both use the same Rust core and SQLite schema. Android
support is planned for a later stage.

## Current features

- load multiple feeds from `feeds.toml`;
- support Medium, Substack, and other RSS/Atom feeds;
- download feeds asynchronously with `reqwest` and Tokio;
- sanitize remote HTML before storing it;
- deduplicate articles with feed-scoped identifiers;
- store data locally in SQLite with automatic migrations;
- disable a subscription without losing its history, or permanently delete it
  with its articles;
- maintain local read and favorite states in the Rust core;
- display articles from newest to oldest;
- read cached articles even when some feeds are unavailable;
- use separate commands to refresh, list, read, and update article states;
- use a two-pane Linux interface with subscription management;
- distinguish full content, excerpts, and missing content;
- safely open the original article when a feed only provides an excerpt.

## Ubuntu prerequisites

The project requires:

- a recent Rust and Cargo installation, preferably through
  [rustup](https://rustup.rs/);
- a C compiler to build the bundled SQLite library;
- Git to clone the repository.

Install the system tools required by the CLI with:

```bash
sudo apt update
sudo apt install build-essential git
```

To build the Tauri application on Ubuntu, also install Node.js, `pkg-config`,
WebKitGTK, and the libraries recommended by Tauri:

```bash
sudo apt update
sudo apt install pkg-config libwebkit2gtk-4.1-dev libssl-dev \
  libayatana-appindicator3-dev librsvg2-dev libxdo-dev curl wget file
```

The frontend has been tested with Node.js 24 and npm 11. A recent Node.js LTS
release should work as well.

SQLite is compiled into the application. No SQLite server or system development
library is required.

The optional `sqlite3` program is useful for inspecting or backing up the
database manually:

```bash
sudo apt install sqlite3
```

## Install the project

From a local checkout:

```bash
cargo build
```

Cargo downloads the crates declared in `Cargo.toml` and creates the development
binary at `target/debug/inkriver`.

For an optimized build:

```bash
cargo build --release
```

The resulting binary is located at `target/release/inkriver`.

Then install the frontend dependencies:

```bash
cd app
npm install
```

`app/package-lock.json` pins the resolved versions and should remain committed.

## Use the Linux application

From `app/`, start the development application with:

```bash
npm run tauri dev
```

On first launch, InkRiver automatically creates `inkriver.db` in the data
directory for the `io.github.r0m1-b.inkriver` bundle—usually
`~/.local/share/io.github.r0m1-b.inkriver/` on Ubuntu. This database is separate
from the CLI's `inkriver.db`.

The application displays its cache immediately and never accesses the network
on startup. The **Subscriptions** page lists every active or inactive feed with
its URL, author, description, latest publication, last successful refresh, and
most recent detailed error. This status is stored in SQLite and remains visible
after restarting InkRiver. Disabling and deleting actions are available on this
page; **Add subscription** opens a separate dialog containing only the add form.
Use **Refresh** to download articles and update these details.

Opening an unread article automatically marks it as read. The reading panel
shows the current state and lets you explicitly mark the article as read or
unread; the change is stored in SQLite and immediately reflected in the
timeline. Each timeline row also provides always-visible star and envelope
buttons to change favorite and read states without opening the article.
Source badges pair the Medium or Substack brand mark—or a generic RSS icon—with
their text label. The brand vectors come from Simple Icons v16.21.0; Medium and
Substack retain ownership of their respective trademarks.
The **All** and **Favorites** tabs above the timeline provide an immediate,
offline view of starred articles while keeping the same reading panel.

The reading panel also provides an archive icon. After mandatory confirmation,
the article disappears from every list and cannot be restored from the current
interface. InkRiver keeps a small database tombstone so a later refresh cannot
recreate it, but removes the stored article body.

On application startup and after every refresh, InkRiver applies a fixed
30-day retention rule. It automatically archives only articles that are read,
not favorites, have a publication date, and are strictly older than 30 days.
Unread, favorite, undated, and exactly 30-day-old articles are preserved.

HTTP(S) links contained in an article open in the system browser instead of
navigating inside InkRiver. Relative links are resolved from the article URL.
Links to sections within the current article are currently ignored.
The reading panel always identifies the article's original source by domain and
opens it in the system browser. When the feed contains only an excerpt or no
content, the more prominent **Read original** button remains available as well.
Articles without a usable HTTP(S) source display a non-interactive status.

Disabling a subscription preserves its identifier, articles, favorites, and
read states. Adding the same URL again reactivates that subscription instead of
creating a new history.

Deletion is a separate, permanent action. After confirmation, the application
deletes the subscription, all of its articles, and their read and favorite
states in a single transaction. Adding the URL again creates a new subscription
without restoring the previous history.

Build optimized Linux packages with:

```bash
cd app
npm run tauri build -- --bundles deb,appimage
```

Packages are generated under `target/release/bundle/deb/` and
`target/release/bundle/appimage/`.

## Configure subscriptions

Create a `feeds.toml` file at the project root:

```toml
[[feeds]]
id = "my-substack"
platform = "substack"
url = "https://example.substack.com/feed"

[[feeds]]
id = "my-medium"
platform = "medium"
url = "https://medium.com/feed/@example"

[[feeds]]
id = "another-blog"
platform = "other"
url = "https://example.org/feed.xml"
```

Configuration rules:

- every `id` must be non-empty and unique;
- `platform` accepts lowercase `medium`, `substack`, or `other`;
- `url` must point directly to a public RSS or Atom feed;
- two active subscriptions cannot use exactly the same URL.

`feeds.toml` is intentionally ignored by Git because it contains the
developer's personal configuration.

## Use the CLI

Display the help and available commands:

```bash
cargo run -- --help
```

Refresh subscriptions:

```bash
cargo run -- refresh
```

The `refresh` command:

1. reads `feeds.toml`;
2. opens or creates `inkriver.db`;
3. applies missing SQLite migrations;
4. imports the current subscription list;
5. downloads the feeds;
6. inserts or updates articles;
7. applies the same 30-day retention rule as the application;
8. attempts to extract complete pages for eligible non-Medium/non-Substack
   articles;
9. reports how many articles were received, inserted, updated, extracted, and
   automatically archived.

A feed error is written to standard error but does not erase cached articles.
The remaining feeds continue to be processed.

List stored articles without loading `feeds.toml` or accessing the network:

```bash
cargo run -- list
```

The list displays a one-based number and a stable identifier for every article.
The following commands accept either form:

```bash
cargo run -- show 1
cargo run -- show "my-substack::publisher-identifier"

cargo run -- mark-read 1
cargo run -- mark-unread 1
cargo run -- favorite 1
cargo run -- unfavorite 1
```

`show` loads only the selected article, converts its HTML to readable terminal
text, displays its original URL, and automatically marks it as read.

A number refers to the article's current position in the timeline and may
change after a refresh. Scripts should use the stable identifier instead.

Paths can be overridden for a command:

```bash
cargo run -- \
  --config /path/to/feeds.toml \
  --database /path/to/inkriver.db \
  refresh
```

`--config` is only consulted by `refresh`. `list`, `show`, and the state
commands work entirely offline.

Exit codes:

- `0`: command completed successfully;
- `1`: fatal configuration, SQLite, selection, or rendering error;
- `2`: partially successful refresh with at least one feed error.

The current CLI uses compile-time paths anchored to the project root. This is a
development behavior and is not yet suitable for a portable system
installation.

## SQLite database

### Installation and creation

The database does not require a separate installation step. On the first
`cargo run -- refresh` or `cargo run -- list`, SQLx automatically creates this
file at the project root:

```text
inkriver.db
```

Migrations from `migrations/` are embedded in the binary and applied when the
database is opened. The database currently contains:

- `feeds`: subscriptions, platforms, URLs, active states, feed metadata, and
  last refresh success or error;
- `articles`: remote content, content kinds, feed relationships, read and
  favorite states, plus archive tombstones and their reason;
- `_sqlx_migrations`: migrations already applied by SQLx.

SQLite may also create temporary `inkriver.db-wal` and `inkriver.db-shm` files.
These files and the main database are ignored by Git.

### Inspect the database

With the optional `sqlite3` client:

```bash
sqlite3 inkriver.db ".tables"
sqlite3 inkriver.db ".schema feeds"
sqlite3 inkriver.db ".schema articles"
```

Some read-only diagnostic queries:

```bash
sqlite3 -header -column inkriver.db \
  "SELECT id, platform, is_active, url FROM feeds ORDER BY id;"

sqlite3 -header -column inkriver.db \
  "SELECT id, title, published_at, is_read, is_favorite FROM articles ORDER BY published_at DESC LIMIT 20;"
```

Avoid editing these tables manually: the Rust API enforces their constraints
and preserves local states during refreshes.

Archived rows intentionally retain their identifiers and metadata so the same
remote entries remain hidden on later refreshes. Their content is set to
`NULL`, but SQLite does not immediately shrink the database file and InkRiver
does not run `VACUUM` automatically.

### Back up the database

After stopping InkRiver, use SQLite's backup command:

```bash
sqlite3 inkriver.db ".backup 'inkriver-backup.db'"
```

A regular copy is also safe while InkRiver and every SQLite client are stopped:

```bash
cp inkriver.db inkriver-backup.db
```

The database contains articles, imported subscriptions, and read and favorite
states. Back up `feeds.toml` separately.

### Reset the database completely

Warning: this operation deletes article history, favorites, and read states.
Stop InkRiver, make an optional backup, and run the following commands from the
project root:

```bash
rm -f inkriver.db inkriver.db-shm inkriver.db-wal
cargo run -- refresh
```

The next run creates an empty database, reapplies every migration, imports
`feeds.toml`, and downloads articles still available in the feeds. Older
articles no longer present in the feeds cannot be recovered without a backup.

To reset the Tauri application's database, close InkRiver, create a backup if
needed, and remove all three SQLite files from its AppData directory:

```bash
rm -f ~/.local/share/io.github.r0m1-b.inkriver/inkriver.db \
  ~/.local/share/io.github.r0m1-b.inkriver/inkriver.db-shm \
  ~/.local/share/io.github.r0m1-b.inkriver/inkriver.db-wal
```

The next launch creates an empty database. Unlike the CLI, the application does
not automatically import `feeds.toml`; subscriptions must be added again from
the interface.

### Remove or reactivate a subscription

This section applies to the CLI. In the graphical application, “Disable” keeps
the history, while “Delete” permanently removes the subscription and all of its
articles after confirmation.

Removing an entry from `feeds.toml` and then running `cargo run -- refresh`
marks the subscription as inactive. Its articles, favorites, and read states
remain in SQLite.

Restoring the same `id` to `feeds.toml` and refreshing reactivates the
subscription and updates its URL and platform if necessary.

### Evolve the schema

Never edit an applied migration. To evolve the database:

1. add a new versioned SQL file under `migrations/`;
2. keep every previous migration;
3. compile and run the tests;
4. restart InkRiver to apply the new migration.

The `build.rs` script tells Cargo to rebuild the binary whenever the migrations
directory changes.

## Development and quality

### Branch workflow

- `dev` is the integration branch for day-to-day development. Regular work is
  committed and pushed there.
- Short-lived branches may be created from `dev` for changes that benefit from
  an isolated review, then merged back into `dev`.
- `main` represents a stable, releasable version. It is updated from `dev` only
  when a release is explicitly frozen, after all validation commands pass.
- Releases are tagged on `main`; ordinary development is never committed
  directly to that branch.

Common commands:

```bash
cargo fmt
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings

cd app
npm run typecheck
npm test
npm run build
```

Collection tests use injected content and local fixtures. SQLite tests use
in-memory databases or temporary files and never modify `inkriver.db`.

### Optional local extraction corpus

The main-content extractor has committed synthetic fixtures and therefore runs
with the regular test suite. A larger corpus of complete third-party pages can
be kept locally under `tests/pages/`; this directory is ignored by Git and is
never accessed by ordinary tests.

Each page is described by the local `tests/pages/description.json`. Run its
explicitly ignored regression test with:

```bash
cargo test --test extraction_corpus -- --ignored
```

Only the saved HTML documents are required. Browser-generated `*_files`
resource directories are not read by the extractor.

### Automatic article-page extraction

During a manual refresh, InkRiver can complete articles from `Other` feeds when
their RSS entry contains only an excerpt or no body. Medium, Substack, complete
RSS entries, and archived articles are never fetched this way. A successful
page is stored with `content_kind = extracted`; a failure leaves the RSS
fallback untouched.

Page downloads are limited to 20 attempts per refresh and four concurrent
requests, with a 10-second timeout, a 2 MiB body limit, and at most five
validated HTTP(S) redirects. Private or local network destinations are
rejected. Failed pages enter a seven-day cooldown; changing an article URL
makes it eligible immediately.

Main project structure:

```text
src/config.rs   feeds.toml loading and validation
src/cli.rs      arguments, commands, rendering, and exit codes
src/http.rs     asynchronous HTTP downloads
src/feed.rs     RSS/Atom conversion to the shared model
src/content_extractor.rs  offline main-content extraction and sanitization
src/page_http.rs  bounded and public-network-only article page downloads
src/enrichment.rs  extraction selection, concurrency, and persistence
src/service.rs  collection, deduplication, and sorting
src/storage.rs  SQLite storage and local states
src/refresh.rs  import → collection → storage orchestration
src/main.rs     CLI entry point
migrations/     versioned schema changes
app/src/        Vanilla TypeScript interface
app/src-tauri/  Tauri adapter, commands, and configuration
```

## Current limitations

- the numbers displayed by `list` are not stable between timelines, unlike
  article identifiers;
- content requiring authentication or a paid subscription is not supported;
- the interface is not yet adapted to mobile screens;
- the CLI's `inkriver.db` is still stored in the development repository.

The next step is to adapt the interface to Android. In the installed
application, SQLite is already the source of truth and lives in the AppData
directory for the `io.github.r0m1-b.inkriver` bundle.

## License

InkRiver is distributed under the [MIT License](LICENSE).

## Quick troubleshooting

### `feeds.toml` cannot be found

Create the file at the project root. Its path does not depend on the directory
from which the binary is launched. The file is only required by `refresh`.

### The TOML configuration is rejected

Check the quotation marks, `[[feeds]]` blocks, unique identifiers, and accepted
`platform` values.

### A feed fails

Make sure its URL returns RSS or Atom directly. Cached articles remain
available when a server is unavailable.

### The database is locked

Close other `inkriver` processes and open `sqlite3` sessions, then try again. Do
not remove database files while any process is using them.

### A migration fails

Keep the error message, back up the database, and check the order and contents
of the files under `migrations/`. In development only, a full reset provides a
clean schema.

### A GLIBC error mentions `/snap/core20`

An integrated terminal from a snap installation of VS Code may inject its own
GTK/GIO variables, mixing snap libraries with system libraries. Run InkRiver from
a regular Ubuntu terminal. For a one-off diagnosis in the integrated terminal,
unset `GTK_PATH`, `GIO_MODULE_DIR`, `GDK_PIXBUF_MODULE_FILE`,
`GSETTINGS_SCHEMA_DIR`, and `LOCPATH` before running `npm run tauri dev`.
