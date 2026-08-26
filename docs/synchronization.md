# InkRiver synchronization contract

Status: accepted design for `SYNC-001`

Protocol version: draft `1`

Scope: first Linux/Android synchronization version

This document defines the replicated domain and its conflict rules before any
SQLite migration or transport is implemented. Later synchronization tickets
must preserve these rules or explicitly version the protocol when changing
them.

## Principles

- Every installation keeps its own SQLite database as its local source of
  truth.
- InkRiver synchronizes immutable business events, never `inkriver.db`, its WAL
  or its SHM file.
- Reading and editing remain fully available without a network connection.
- Applying the same event more than once has no additional effect.
- Applying a complete set of events in any order produces the same replicated
  state.
- A local user mutation and its outgoing event are committed in the same SQLite
  transaction.
- Applying a remote event never generates another outgoing event.
- The merge engine is independent from its transport. A local directory,
  Syncthing and WebDAV all carry the same encrypted immutable segments.

## Terminology and versions

### Device and event identities

Each installation owns a random UUID `device_id`, generated once and retained
for the lifetime of that installation. Restoring a full InkRiver backup retains
the identity; reinstalling without restoring creates a new one.

Each device allocates a strictly increasing 64-bit `sequence`. An event is
identified by `(device_id, sequence)`. This pair is the idempotency key and must
never be reused, including after a failed upload.

Every event also carries a hybrid logical clock:

```text
(physical_milliseconds, logical_counter, device_id, sequence)
```

The tuple above is the total `version` order. The device and sequence fields
only break ties; arrival order is never used to resolve a conflict. Receiving a
remote clock advances the local hybrid clock before the next local event is
created.

### Subscription identity

An addition creates a random UUID `subscription_id`. It is stable for one
subscription incarnation and is distinct from its normalized URL.

Two devices may add the same normalized URL while offline and therefore create
different UUIDs. Create events sharing both the normalized URL and the same
`parent_tombstone` belong to one logical subscription:

- the lexicographically smallest UUID becomes the canonical ID;
- every other UUID is retained as an alias of that canonical ID;
- events referring to an alias are applied to the canonical subscription;
- the choice is calculated from the whole event set and is independent of
  import order.

For an initial subscription, `parent_tombstone` is absent. Deleting a
subscription closes that incarnation permanently. Adding the URL again after
observing its deletion creates a new incarnation whose `parent_tombstone`
references the winning deletion event. Concurrent re-additions referencing the
same tombstone are deduplicated by the rule above.

Concurrent deletion events for one incarnation form a single tombstone set.
The deletion with the greatest version is its canonical event, and references
to any other deletion in that set resolve to the canonical event. Therefore,
two devices that each delete and then re-add the same feed while offline still
converge once both histories are available.

An addition made by a device that has not yet observed the deletion still
belongs to the deleted incarnation and cannot resurrect it. Once synchronized,
the user may explicitly add the URL again to create a new incarnation.

URL normalization uses InkRiver's `normalize_feed_url` contract: surrounding
whitespace and fragments are removed, the URL parser canonicalizes its normal
components, and only HTTP(S) is accepted. Query strings are significant. The
first version does not attempt publisher-specific equivalence between distinct
URLs that happen to expose the same feed.

### Article identity

An article reference consists of:

```text
(logical subscription incarnation, entry_key)
```

`entry_key` uses the current feed parser precedence:

1. a non-empty publisher GUID;
2. the first article URL, canonicalized without its fragment;
3. the existing deterministic content fingerprint fallback.

The current SQLite article ID (`feed_id::entry_key`) remains a local projection
detail. During merge, subscription aliases are resolved before an article key
is compared. Consequently, articles from two concurrent additions of the same
feed converge even if their current SQLite IDs differ.

The fingerprint fallback is less stable than a publisher GUID or URL. A
publisher changing an entry without either stable field may still produce two
articles; improving that heuristic is outside protocol version 1.

## Replicated operations

Only user intent is journaled. Network refreshes and cache maintenance are not
replicated operations.

| Current operation | Replicated event | Notes |
| --- | --- | --- |
| Add or re-add a subscription | `subscription_created` | Contains the normalized URL, current platform hint and optional parent tombstone. |
| Activate or deactivate a subscription | `subscription_active_set` | A field-level last-writer-wins register. |
| Delete a subscription | `subscription_deleted` | Permanent tombstone for that incarnation. |
| Mark an article read or unread | `article_read_set` | Opening an unread article counts as user intent and produces `true`. |
| Add or remove a favorite | `article_favorite_set` | Independent from the read register. |
| Archive an article manually | `article_archived` | Permanent tombstone in version 1. |

### Audited storage entry points

The contract above covers the current mutation paths as follows:

| Rust storage operation | Classification |
| --- | --- |
| `add_feed` | Replicated create, including reactivation of a retained inactive feed as an activation change. |
| `set_feed_active` | Replicated activation register. |
| `delete_feed` | Replicated subscription tombstone; physical deletion must change once synchronization is enabled. |
| `set_read`, `set_read_many` | Replicated read register. |
| `set_favorite` | Replicated favorite register. |
| `archive_article`, `archive_article_now`, `archive_articles_now` | Replicated manual article tombstone. |
| `import_feeds` | Local CLI configuration import, not replicated. |
| `upsert_articles`, `record_feed_refreshes` | Local remote-cache maintenance, not replicated. |
| `archive_expired_read_articles*`, `apply_article_retention` | Local retention only, not replicated. |
| extraction and feed-logo recording operations | Local cache and diagnostics only, not replicated. |

A grouped read or archive action produces one event per affected article, with
consecutive sequences, in the same transaction as all affected rows. This keeps
conflict resolution per article while preserving atomic local behavior.

The following operations remain local and produce no event:

- RSS/Atom collection and remote metadata updates;
- article body storage, extraction and sanitization;
- automatic 30-day retention;
- feed logos and favicon discovery;
- refresh timestamps, errors, retry counters and extraction diagnostics;
- CLI `feeds.toml` imports;
- interface filters, text size and other presentation preferences.

The `platform` value is synchronized in version 1 because it can currently be
overridden by the user and affects collection behavior. It is a provider hint,
not part of a subscription or article identity. If platform distinctions later
become fully derived, a future protocol may ignore this field without changing
the identity rules.

## Event payloads

All events contain their ID, version, protocol version and one of these payloads:

```text
subscription_created {
  subscription_id, normalized_url, platform_hint, is_active,
  parent_tombstone?
}

subscription_active_set { subscription_id, is_active }
subscription_deleted    { subscription_id }

article_read_set     { article_ref, is_read }
article_favorite_set { article_ref, is_favorite }
article_archived     { article_ref }
```

An `article_ref` contains the originating subscription ID, `entry_key`, and an
optional metadata snapshot: title, URL, author and publication date. The
snapshot lets another device recognize and display an article before its own
next feed refresh. It never contains the HTML body.

Metadata snapshots are fill-only cache hints: they populate absent local fields
but never replace newer non-empty metadata obtained from the local feed. They
are not last-writer-wins registers and do not generate events when refreshed.

## Merge registers and tombstones

Read state, favorite state and subscription activation are independent
last-writer-wins registers. Only events for the same logical entity and field
compete. The greatest total `version` wins, so concurrent changes to different
fields are both preserved.

Deletion and manual archiving are permanent tombstones in protocol version 1:

- a subscription tombstone dominates every create or activation event in the
  same incarnation, regardless of arrival order;
- an article tombstone dominates every read, favorite or refresh update for
  that article;
- an older device cannot undo either tombstone;
- manual article restoration is not supported in version 1;
- a deleted subscription can only return as a new incarnation explicitly
  linked to the observed subscription tombstone.

Automatic retention is not a replicated tombstone. It only releases local
cached content and hides a locally eligible article. If an imported state makes
such an article unread or favorite, the local retention archive is removed and
the metadata-only article becomes visible again. A subsequent feed refresh may
restore its body. A manual archive is never removed this way.

## Conflict matrix

| Concurrent or reordered operations | Deterministic result |
| --- | --- |
| Add the same normalized URL on two offline devices | One subscription; smallest UUID is canonical and the other is an alias. |
| Add two different normalized URLs | Both subscriptions remain. |
| Activate vs deactivate the same subscription | The value with the greatest version wins. |
| Change platform hints on aliased concurrent additions | The hint from the greatest create-event version wins. |
| Activate/deactivate vs delete | Deletion wins for that incarnation. |
| Stale add vs an already issued delete | Deletion wins; the stale device cannot resurrect the incarnation. |
| Re-add after observing deletion | A new incarnation is created through `parent_tombstone`. |
| Mark read vs mark unread | The value with the greatest version wins. |
| Add favorite vs remove favorite | The value with the greatest version wins. |
| Change read and favorite concurrently | Both changes survive in their independent registers. |
| Read/favorite change vs manual archive | Manual archive wins and the article remains hidden. |
| Feed refresh vs manual archive | The tombstone remains; downloaded content cannot restore the article. |
| Automatic retention vs imported unread or favorite | Imported user state wins and removes the local retention archive. |
| Article state received before its article | The event is retained; a metadata-only article is projected once its subscription dependency exists. |
| Article event received before its subscription create | The event remains pending and is retried after later imports. |
| Event received more than once | The second and subsequent imports are no-ops. |
| Events imported in different orders | Field versions, aliases and tombstones produce the same final state. |

## Missing dependencies and metadata-only articles

An event is never discarded merely because a referenced subscription or
article is not available yet.

- If the subscription create event is missing, the event is stored as pending.
- Once the subscription exists, an article-state event may create a local
  metadata-only row with `content_kind = missing`.
- A later local feed refresh joins that row through the logical article key,
  fills its cache fields and body, and preserves the synchronized state.
- An archive event may remain represented only by its tombstone; it never needs
  a visible article row.
- Pending events are idempotently retried after every successful import that
  adds one of their dependencies.

Deleting a subscription also hides or removes its local article projections,
but retains the subscription tombstone, article tombstones and versions needed
to reject stale events. A new subscription incarnation does not inherit the
deleted incarnation's article states.

## Existing installations and bootstrap

Enabling synchronization on an existing installation creates a transactional
bootstrap journal without changing user-visible state:

- one create event and current activation value for every retained feed;
- current read and favorite values for locally known articles;
- tombstones for articles archived manually;
- no event for retention-only archives or technical cache metadata.

Existing permanent feed deletions cannot be reconstructed because current
versions physically removed those rows. They predate the synchronization group
and are therefore outside its history. Once synchronization is enabled, future
deletions always retain a tombstone.

When two existing installations are joined, both bootstrap histories are
merged using the same alias, version and conflict rules; neither database file
is considered authoritative as a whole.

## Required invariants

1. `(device_id, sequence)` is globally unique within one synchronization group.
2. A device sequence and its hybrid clock never move backwards.
3. Event records and exported segments are immutable.
4. Local state and its outgoing events commit or roll back together.
5. One imported event is applied at most once and never produces an outgoing
   event.
6. Merge results depend only on the validated event set, never import order.
7. Subscription aliases are resolved before article identities.
8. A tombstone is retained while any authorized device could still send an
   older event for its entity.
9. Remote data cannot replace local HTML bodies, logos or technical cache
   fields.
10. Malformed, oversized, unauthenticated or unsupported events have no local
    effect.

## Explicitly outside version 1

- synchronizing SQLite, WAL or SHM files;
- article HTML, images, extracted content and feed logos;
- application settings, filters, tags, ordering and reading position;
- a hosted InkRiver account or server;
- authenticated Medium or Substack content;
- shared multi-user editing and permissions;
- restoring manually archived articles;
- automatically recognizing two genuinely different feed URLs as one feed;
- reviving state from an incarnation deleted before synchronization was
  enabled;
- protocol compaction, snapshots, device revocation and key rotation, which are
  reserved for later tickets already listed in `FR-012`.

## Consequences for the next tickets

`SYNC-002` stores the device identity, sequence, hybrid clock, immutable events,
per-field versions, subscription aliases, tombstones and pending dependencies.
It also adds an article entry key that is independent from the current
namespaced SQLite ID.

`SYNC-003` must route every replicated mutation through transaction-aware
storage operations and bootstrap existing installations. `SYNC-004` must test
the conflict matrix by importing every relevant permutation into independent
databases.

`SYNC-005` should implement a local-directory segment transport first. That
transport provides an offline test harness and can be used directly with
Syncthing. `SYNC-007` then adds WebDAV as the first transport managed entirely
from the InkRiver interface.
