use crate::config::{Platform, normalize_feed_url};
use crate::sync::{
    HybridLogicalClock, SYNC_PROTOCOL_VERSION, SyncArticleRef, SyncEvent, SyncEventId,
    SyncEventPayload, SyncImportReport,
};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use sqlx::{Sqlite, SqlitePool, Transaction};
use std::collections::{BTreeSet, HashSet};

const MAX_DEVICE_ID_LENGTH: usize = 64;
const MAX_ENTITY_ID_LENGTH: usize = 4_096;
const MAX_URL_LENGTH: usize = 8_192;
const MAX_TITLE_LENGTH: usize = 16_384;
const MAX_AUTHOR_LENGTH: usize = 4_096;
const MAX_EVENTS_PER_IMPORT: usize = 1_000;

type EventRow = (String, i64, i64, i64, i64, String, String);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct EventVersion {
    physical_milliseconds: i64,
    logical_counter: i64,
    device_id: String,
    sequence: i64,
}

enum ApplyOutcome {
    Applied,
    Pending(&'static str),
}

/// Imports remote events and updates their local projections in one transaction.
pub(crate) async fn import_sync_events(
    pool: &SqlitePool,
    events: &[SyncEvent],
    observed_at: DateTime<Utc>,
) -> Result<SyncImportReport> {
    if events.len() > MAX_EVENTS_PER_IMPORT {
        bail!("A synchronization import cannot exceed {MAX_EVENTS_PER_IMPORT} events");
    }
    for event in events {
        validate_event(event)?;
    }

    let mut transaction = pool
        .begin()
        .await
        .context("Impossible de démarrer l'import de synchronisation")?;
    let (local_device_id, sync_enabled): (String, bool) =
        sqlx::query_as("SELECT device_id, is_enabled FROM sync_local_state WHERE singleton = 1")
            .fetch_one(&mut *transaction)
            .await
            .context("Impossible de lire l'état local de synchronisation")?;
    if !sync_enabled {
        bail!("Synchronization must be enabled before importing events");
    }

    let mut report = SyncImportReport {
        received: events.len(),
        ..SyncImportReport::default()
    };
    let mut imported_devices = BTreeSet::new();
    let mut imported_clocks = Vec::new();

    for event in events {
        if let Some(existing) =
            load_event(&mut transaction, &event.device_id, event.sequence).await?
        {
            if existing != *event {
                bail!(
                    "Synchronization event identity collision at {}:{}",
                    event.device_id,
                    event.sequence
                );
            }
            report.duplicates += 1;
            continue;
        }
        if event.device_id == local_device_id {
            bail!("Unknown event claims the local synchronization device identity");
        }

        insert_event(&mut transaction, event).await?;
        sqlx::query(
            "INSERT INTO sync_pending_events (device_id, sequence, reason) VALUES (?, ?, 'unprocessed')",
        )
        .bind(&event.device_id)
        .bind(event.sequence)
        .execute(&mut *transaction)
        .await
        .context("Impossible de préparer l'application d'un événement distant")?;
        report.imported += 1;
        imported_devices.insert(event.device_id.clone());
        imported_clocks.push(event.clock);
    }

    advance_local_clock(&mut transaction, &imported_clocks, observed_at).await?;
    report.applied = apply_pending_events(&mut transaction).await?;
    report.pending = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_pending_events")
        .fetch_one(&mut *transaction)
        .await
        .context("Impossible de compter les événements en attente")? as usize;

    for device_id in imported_devices {
        update_contiguous_cursor(&mut transaction, &device_id).await?;
    }

    transaction
        .commit()
        .await
        .context("Impossible de valider l'import de synchronisation")?;
    Ok(report)
}

fn validate_event(event: &SyncEvent) -> Result<()> {
    validate_non_empty("device id", &event.device_id, MAX_DEVICE_ID_LENGTH)?;
    if uuid::Uuid::parse_str(&event.device_id).is_err() {
        bail!("Synchronization event has an invalid device id");
    }
    if event.sequence <= 0 {
        bail!("Synchronization event sequence must be positive");
    }
    if event.clock.physical_milliseconds < 0 || event.clock.logical_counter < 0 {
        bail!("Synchronization event clock cannot be negative");
    }
    if event.protocol_version != SYNC_PROTOCOL_VERSION {
        bail!(
            "Unsupported synchronization protocol version {}",
            event.protocol_version
        );
    }
    if event.kind != event.payload.kind() {
        bail!("Synchronization event kind does not match its payload");
    }

    match &event.payload {
        SyncEventPayload::SubscriptionCreated {
            subscription_id,
            normalized_url,
            parent_tombstone,
            ..
        } => {
            validate_non_empty("subscription id", subscription_id, MAX_ENTITY_ID_LENGTH)?;
            validate_non_empty("subscription URL", normalized_url, MAX_URL_LENGTH)?;
            let normalized = normalize_feed_url(normalized_url)
                .map_err(|_| anyhow::anyhow!("Synchronization event has an invalid feed URL"))?;
            if normalized != *normalized_url {
                bail!("Synchronization event feed URL is not normalized");
            }
            if let Some(parent) = parent_tombstone {
                validate_event_id(parent)?;
            }
        }
        SyncEventPayload::SubscriptionActiveSet {
            subscription_id, ..
        }
        | SyncEventPayload::SubscriptionPlatformSet {
            subscription_id, ..
        }
        | SyncEventPayload::SubscriptionDeleted { subscription_id } => {
            validate_non_empty("subscription id", subscription_id, MAX_ENTITY_ID_LENGTH)?;
        }
        SyncEventPayload::ArticleReadSet { article, .. }
        | SyncEventPayload::ArticleFavoriteSet { article, .. }
        | SyncEventPayload::ArticleArchived { article } => validate_article_ref(article)?,
    }
    Ok(())
}

fn validate_event_id(event: &SyncEventId) -> Result<()> {
    validate_non_empty("event device id", &event.device_id, MAX_DEVICE_ID_LENGTH)?;
    if uuid::Uuid::parse_str(&event.device_id).is_err() || event.sequence <= 0 {
        bail!("Synchronization event contains an invalid event reference");
    }
    Ok(())
}

fn validate_article_ref(article: &SyncArticleRef) -> Result<()> {
    validate_non_empty(
        "article subscription id",
        &article.subscription_id,
        MAX_ENTITY_ID_LENGTH,
    )?;
    validate_non_empty(
        "article entry key",
        &article.entry_key,
        MAX_ENTITY_ID_LENGTH,
    )?;
    validate_optional("article title", article.title.as_deref(), MAX_TITLE_LENGTH)?;
    validate_optional("article URL", article.url.as_deref(), MAX_URL_LENGTH)?;
    validate_optional(
        "article author",
        article.author.as_deref(),
        MAX_AUTHOR_LENGTH,
    )?;
    if let Some(url) = article.url.as_deref() {
        let parsed = reqwest::Url::parse(url)
            .map_err(|_| anyhow::anyhow!("Synchronization article URL is invalid"))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            bail!("Synchronization article URL must use HTTP or HTTPS");
        }
    }
    if let Some(published_at) = article.published_at.as_deref() {
        DateTime::parse_from_rfc3339(published_at)
            .map_err(|_| anyhow::anyhow!("Synchronization publication date is invalid"))?;
    }
    Ok(())
}

fn validate_non_empty(label: &str, value: &str, maximum: usize) -> Result<()> {
    if value.trim().is_empty() || value.len() > maximum {
        bail!("Synchronization {label} must contain 1 to {maximum} bytes");
    }
    Ok(())
}

fn validate_optional(label: &str, value: Option<&str>, maximum: usize) -> Result<()> {
    if value.is_some_and(|value| value.len() > maximum) {
        bail!("Synchronization {label} exceeds {maximum} bytes");
    }
    Ok(())
}

async fn insert_event(transaction: &mut Transaction<'_, Sqlite>, event: &SyncEvent) -> Result<()> {
    let payload = serde_json::to_string(&event.payload)
        .context("Impossible de sérialiser un événement distant")?;
    sqlx::query(
        r#"
            INSERT INTO sync_events (
                device_id, sequence, hlc_physical_ms, hlc_counter,
                protocol_version, event_kind, payload_json
            ) VALUES (?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&event.device_id)
    .bind(event.sequence)
    .bind(event.clock.physical_milliseconds)
    .bind(event.clock.logical_counter)
    .bind(event.protocol_version)
    .bind(&event.kind)
    .bind(payload)
    .execute(&mut **transaction)
    .await
    .context("Impossible d'enregistrer un événement distant")?;
    Ok(())
}

async fn load_event(
    transaction: &mut Transaction<'_, Sqlite>,
    device_id: &str,
    sequence: i64,
) -> Result<Option<SyncEvent>> {
    let row: Option<EventRow> = sqlx::query_as(
        r#"
            SELECT device_id, sequence, hlc_physical_ms, hlc_counter,
                   protocol_version, event_kind, payload_json
            FROM sync_events
            WHERE device_id = ? AND sequence = ?
        "#,
    )
    .bind(device_id)
    .bind(sequence)
    .fetch_optional(&mut **transaction)
    .await
    .context("Impossible de rechercher un événement de synchronisation")?;
    row.map(event_from_row).transpose()
}

fn event_from_row(row: EventRow) -> Result<SyncEvent> {
    let (device_id, sequence, physical, counter, protocol_version, kind, payload) = row;
    let payload: SyncEventPayload = serde_json::from_str(&payload)
        .context("Le journal de synchronisation contient un payload invalide")?;
    if payload.kind() != kind {
        bail!("Le journal de synchronisation contient un type incohérent");
    }
    Ok(SyncEvent {
        device_id,
        sequence,
        clock: HybridLogicalClock {
            physical_milliseconds: physical,
            logical_counter: counter,
        },
        protocol_version,
        kind,
        payload,
    })
}

async fn advance_local_clock(
    transaction: &mut Transaction<'_, Sqlite>,
    imported: &[HybridLogicalClock],
    observed_at: DateTime<Utc>,
) -> Result<()> {
    if imported.is_empty() {
        return Ok(());
    }
    let (local_physical, local_counter): (i64, i64) = sqlx::query_as(
        "SELECT hlc_physical_ms, hlc_counter FROM sync_local_state WHERE singleton = 1",
    )
    .fetch_one(&mut **transaction)
    .await
    .context("Impossible de lire l'horloge de synchronisation")?;
    let remote_physical = imported
        .iter()
        .map(|clock| clock.physical_milliseconds)
        .max()
        .unwrap_or(0);
    let remote_counter = imported
        .iter()
        .filter(|clock| clock.physical_milliseconds == remote_physical)
        .map(|clock| clock.logical_counter)
        .max()
        .unwrap_or(0);
    let wall = observed_at.timestamp_millis().max(0);
    let physical = wall.max(local_physical).max(remote_physical);
    let counter = match (physical == local_physical, physical == remote_physical) {
        (true, true) => local_counter.max(remote_counter) + 1,
        (true, false) => local_counter + 1,
        (false, true) => remote_counter + 1,
        (false, false) => 0,
    };
    sqlx::query(
        "UPDATE sync_local_state SET hlc_physical_ms = ?, hlc_counter = ? WHERE singleton = 1",
    )
    .bind(physical)
    .bind(counter)
    .execute(&mut **transaction)
    .await
    .context("Impossible d'avancer l'horloge de synchronisation")?;
    Ok(())
}

async fn apply_pending_events(transaction: &mut Transaction<'_, Sqlite>) -> Result<usize> {
    let mut applied = 0;
    loop {
        let rows: Vec<EventRow> = sqlx::query_as(
            r#"
                SELECT event.device_id, event.sequence,
                       event.hlc_physical_ms, event.hlc_counter,
                       event.protocol_version, event.event_kind,
                       event.payload_json
                FROM sync_pending_events AS pending
                INNER JOIN sync_events AS event
                    ON event.device_id = pending.device_id
                   AND event.sequence = pending.sequence
                ORDER BY event.hlc_physical_ms, event.hlc_counter,
                         event.device_id, event.sequence
            "#,
        )
        .fetch_all(&mut **transaction)
        .await
        .context("Impossible de charger les événements en attente")?;
        if rows.is_empty() {
            break;
        }

        let mut progressed = false;
        for row in rows {
            let event = event_from_row(row)?;
            match apply_event(transaction, &event).await? {
                ApplyOutcome::Applied => {
                    sqlx::query(
                        "DELETE FROM sync_pending_events WHERE device_id = ? AND sequence = ?",
                    )
                    .bind(&event.device_id)
                    .bind(event.sequence)
                    .execute(&mut **transaction)
                    .await
                    .context("Impossible de terminer l'application d'un événement")?;
                    applied += 1;
                    progressed = true;
                }
                ApplyOutcome::Pending(reason) => {
                    sqlx::query(
                        "UPDATE sync_pending_events SET reason = ? WHERE device_id = ? AND sequence = ?",
                    )
                    .bind(reason)
                    .bind(&event.device_id)
                    .bind(event.sequence)
                    .execute(&mut **transaction)
                    .await
                    .context("Impossible de conserver une dépendance en attente")?;
                }
            }
        }
        if !progressed {
            break;
        }
    }
    Ok(applied)
}

async fn apply_event(
    transaction: &mut Transaction<'_, Sqlite>,
    event: &SyncEvent,
) -> Result<ApplyOutcome> {
    match &event.payload {
        SyncEventPayload::SubscriptionCreated {
            subscription_id,
            normalized_url,
            platform_hint,
            is_active,
            parent_tombstone,
        } => {
            apply_subscription_created(
                transaction,
                event,
                subscription_id,
                normalized_url,
                *platform_hint,
                *is_active,
                parent_tombstone.as_ref(),
            )
            .await
        }
        SyncEventPayload::SubscriptionActiveSet {
            subscription_id,
            is_active,
        } => {
            apply_subscription_field(
                transaction,
                event,
                subscription_id,
                "active",
                Some(*is_active),
                None,
            )
            .await
        }
        SyncEventPayload::SubscriptionPlatformSet {
            subscription_id,
            platform_hint,
        } => {
            apply_subscription_field(
                transaction,
                event,
                subscription_id,
                "platform",
                None,
                Some(*platform_hint),
            )
            .await
        }
        SyncEventPayload::SubscriptionDeleted { subscription_id } => {
            apply_subscription_deleted(transaction, event, subscription_id).await
        }
        SyncEventPayload::ArticleReadSet { article, is_read } => {
            apply_article_field(transaction, event, article, "read", *is_read).await
        }
        SyncEventPayload::ArticleFavoriteSet {
            article,
            is_favorite,
        } => apply_article_field(transaction, event, article, "favorite", *is_favorite).await,
        SyncEventPayload::ArticleArchived { article } => {
            apply_article_archived(transaction, event, article).await
        }
    }
}

async fn apply_subscription_created(
    transaction: &mut Transaction<'_, Sqlite>,
    event: &SyncEvent,
    subscription_id: &str,
    normalized_url: &str,
    platform: Platform,
    is_active: bool,
    parent: Option<&SyncEventId>,
) -> Result<ApplyOutcome> {
    let normalized_parent = match parent {
        Some(parent) => match canonical_parent_tombstone(transaction, parent).await? {
            Some(parent) => Some(parent),
            None => return Ok(ApplyOutcome::Pending("missing_parent_tombstone")),
        },
        None => None,
    };
    let existing: Option<(String, Option<String>, Option<i64>)> = sqlx::query_as(
        r#"
            SELECT normalized_url, parent_tombstone_device_id,
                   parent_tombstone_sequence
            FROM sync_subscription_aliases
            WHERE alias_id = ?
        "#,
    )
    .bind(subscription_id)
    .fetch_optional(&mut **transaction)
    .await
    .context("Impossible de vérifier l'identité d'un abonnement")?;
    if let Some((url, parent_device, parent_sequence)) = existing {
        let same_parent = parent_device.as_deref()
            == normalized_parent
                .as_ref()
                .map(|parent| parent.device_id.as_str())
            && parent_sequence == normalized_parent.as_ref().map(|parent| parent.sequence);
        if url != normalized_url || !same_parent {
            bail!("A subscription id is reused for a different incarnation");
        }
    } else {
        sqlx::query(
            r#"
                INSERT INTO sync_subscription_aliases (
                    alias_id, canonical_id, normalized_url,
                    parent_tombstone_device_id, parent_tombstone_sequence
                ) VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(subscription_id)
        .bind(subscription_id)
        .bind(normalized_url)
        .bind(normalized_parent.as_ref().map(|parent| &parent.device_id))
        .bind(normalized_parent.as_ref().map(|parent| parent.sequence))
        .execute(&mut **transaction)
        .await
        .context("Impossible d'enregistrer l'alias d'un abonnement")?;
    }

    let canonical =
        merge_subscription_alias_group(transaction, normalized_url, normalized_parent.as_ref())
            .await?;
    if subscription_is_deleted(transaction, &canonical).await? {
        remove_subscription_projection(transaction, &canonical).await?;
        return Ok(ApplyOutcome::Applied);
    }
    ensure_subscription_projection(transaction, &canonical, normalized_url, platform).await?;
    apply_subscription_register(
        transaction,
        event,
        &canonical,
        "platform",
        None,
        Some(platform),
    )
    .await?;
    apply_subscription_register(
        transaction,
        event,
        &canonical,
        "active",
        Some(is_active),
        None,
    )
    .await?;
    Ok(ApplyOutcome::Applied)
}

async fn canonical_parent_tombstone(
    transaction: &mut Transaction<'_, Sqlite>,
    parent: &SyncEventId,
) -> Result<Option<SyncEventId>> {
    let Some(parent_event) = load_event(transaction, &parent.device_id, parent.sequence).await?
    else {
        return Ok(None);
    };
    let SyncEventPayload::SubscriptionDeleted { subscription_id } = parent_event.payload else {
        bail!("A subscription parent does not reference a deletion event");
    };
    let Some(canonical) = canonical_subscription_id(transaction, &subscription_id).await? else {
        return Ok(None);
    };
    let tombstone: Option<(String, i64)> = sqlx::query_as(
        r#"
            SELECT event_device_id, event_sequence
            FROM sync_tombstones
            WHERE entity_kind = 'subscription' AND entity_key = ?
        "#,
    )
    .bind(canonical)
    .fetch_optional(&mut **transaction)
    .await
    .context("Impossible de résoudre la suppression parente")?;
    Ok(tombstone.map(|(device_id, sequence)| SyncEventId {
        device_id,
        sequence,
    }))
}

async fn merge_subscription_alias_group(
    transaction: &mut Transaction<'_, Sqlite>,
    normalized_url: &str,
    parent: Option<&SyncEventId>,
) -> Result<String> {
    let aliases: Vec<(String, String)> = sqlx::query_as(
        r#"
            SELECT alias_id, canonical_id
            FROM sync_subscription_aliases
            WHERE normalized_url = ?
              AND parent_tombstone_device_id IS ?
              AND parent_tombstone_sequence IS ?
            ORDER BY alias_id
        "#,
    )
    .bind(normalized_url)
    .bind(parent.map(|parent| &parent.device_id))
    .bind(parent.map(|parent| parent.sequence))
    .fetch_all(&mut **transaction)
    .await
    .context("Impossible de regrouper les abonnements concurrents")?;
    let canonical = aliases
        .first()
        .map(|(alias, _)| alias.clone())
        .context("Le groupe d'abonnements est vide")?;
    let old_canonicals = aliases
        .iter()
        .map(|(_, old)| old.clone())
        .collect::<HashSet<_>>();
    sqlx::query(
        r#"
            UPDATE sync_subscription_aliases
            SET canonical_id = ?
            WHERE normalized_url = ?
              AND parent_tombstone_device_id IS ?
              AND parent_tombstone_sequence IS ?
        "#,
    )
    .bind(&canonical)
    .bind(normalized_url)
    .bind(parent.map(|parent| &parent.device_id))
    .bind(parent.map(|parent| parent.sequence))
    .execute(&mut **transaction)
    .await
    .context("Impossible de fixer l'identité canonique de l'abonnement")?;
    for old in old_canonicals {
        if old != canonical {
            rekey_subscription_state(transaction, &old, &canonical).await?;
        }
    }
    Ok(canonical)
}

async fn rekey_subscription_state(
    transaction: &mut Transaction<'_, Sqlite>,
    old: &str,
    canonical: &str,
) -> Result<()> {
    rekey_versions(transaction, "subscription", old, canonical).await?;
    rekey_tombstone(transaction, "subscription", old, canonical).await?;

    let article_versions: Vec<(String, String)> = sqlx::query_as(
        "SELECT entity_key, field_name FROM sync_entity_versions WHERE entity_kind = 'article'",
    )
    .fetch_all(&mut **transaction)
    .await
    .context("Impossible de charger les versions d'articles à recanoniser")?;
    for (key, field) in article_versions {
        let Ok((subscription, entry_key)) = serde_json::from_str::<(String, String)>(&key) else {
            continue;
        };
        if subscription == old {
            let new_key = article_entity_key(canonical, &entry_key)?;
            rekey_one_version(transaction, "article", &key, &new_key, &field).await?;
        }
    }
    let article_tombstones: Vec<String> =
        sqlx::query_scalar("SELECT entity_key FROM sync_tombstones WHERE entity_kind = 'article'")
            .fetch_all(&mut **transaction)
            .await
            .context("Impossible de charger les archives à recanoniser")?;
    for key in article_tombstones {
        let Ok((subscription, entry_key)) = serde_json::from_str::<(String, String)>(&key) else {
            continue;
        };
        if subscription == old {
            let new_key = article_entity_key(canonical, &entry_key)?;
            rekey_tombstone(transaction, "article", &key, &new_key).await?;
        }
    }

    let identities: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT entry_key, article_id FROM sync_article_identities WHERE subscription_id = ?",
    )
    .bind(old)
    .fetch_all(&mut **transaction)
    .await
    .context("Impossible de charger les identités d'articles à recanoniser")?;
    for (entry_key, article_id) in identities {
        let current: Option<Option<String>> = sqlx::query_scalar(
            "SELECT article_id FROM sync_article_identities WHERE subscription_id = ? AND entry_key = ?",
        )
        .bind(canonical)
        .bind(&entry_key)
        .fetch_optional(&mut **transaction)
        .await
        .context("Impossible de rechercher l'identité canonique d'un article")?;
        if current.is_none() {
            sqlx::query(
                "UPDATE sync_article_identities SET subscription_id = ? WHERE subscription_id = ? AND entry_key = ?",
            )
            .bind(canonical)
            .bind(old)
            .bind(&entry_key)
            .execute(&mut **transaction)
            .await
            .context("Impossible de recanoniser l'identité d'un article")?;
        } else {
            if current.flatten().is_none() && article_id.is_some() {
                sqlx::query(
                    "UPDATE sync_article_identities SET article_id = ? WHERE subscription_id = ? AND entry_key = ?",
                )
                .bind(&article_id)
                .bind(canonical)
                .bind(&entry_key)
                .execute(&mut **transaction)
                .await
                .context("Impossible de rattacher la projection d'un article")?;
            }
            sqlx::query(
                "DELETE FROM sync_article_identities WHERE subscription_id = ? AND entry_key = ?",
            )
            .bind(old)
            .bind(&entry_key)
            .execute(&mut **transaction)
            .await
            .context("Impossible de supprimer une ancienne identité d'article")?;
        }
    }
    Ok(())
}

async fn rekey_versions(
    transaction: &mut Transaction<'_, Sqlite>,
    entity_kind: &str,
    old: &str,
    new: &str,
) -> Result<()> {
    let fields: Vec<String> = sqlx::query_scalar(
        "SELECT field_name FROM sync_entity_versions WHERE entity_kind = ? AND entity_key = ?",
    )
    .bind(entity_kind)
    .bind(old)
    .fetch_all(&mut **transaction)
    .await
    .context("Impossible de charger les registres à recanoniser")?;
    for field in fields {
        rekey_one_version(transaction, entity_kind, old, new, &field).await?;
    }
    Ok(())
}

async fn rekey_one_version(
    transaction: &mut Transaction<'_, Sqlite>,
    entity_kind: &str,
    old: &str,
    new: &str,
    field: &str,
) -> Result<()> {
    let source: Option<(String, i64)> = sqlx::query_as(
        r#"
            SELECT event_device_id, event_sequence FROM sync_entity_versions
            WHERE entity_kind = ? AND entity_key = ? AND field_name = ?
        "#,
    )
    .bind(entity_kind)
    .bind(old)
    .bind(field)
    .fetch_optional(&mut **transaction)
    .await
    .context("Impossible de charger un registre à recanoniser")?;
    let Some((device_id, sequence)) = source else {
        return Ok(());
    };
    if version_reference_is_newer(transaction, entity_kind, new, field, &device_id, sequence)
        .await?
    {
        store_version_reference(transaction, entity_kind, new, field, &device_id, sequence).await?;
    }
    sqlx::query(
        "DELETE FROM sync_entity_versions WHERE entity_kind = ? AND entity_key = ? AND field_name = ?",
    )
    .bind(entity_kind)
    .bind(old)
    .bind(field)
    .execute(&mut **transaction)
    .await
    .context("Impossible de supprimer l'ancien registre")?;
    Ok(())
}

async fn rekey_tombstone(
    transaction: &mut Transaction<'_, Sqlite>,
    entity_kind: &str,
    old: &str,
    new: &str,
) -> Result<()> {
    let source: Option<(String, i64)> = sqlx::query_as(
        r#"
            SELECT event_device_id, event_sequence FROM sync_tombstones
            WHERE entity_kind = ? AND entity_key = ?
        "#,
    )
    .bind(entity_kind)
    .bind(old)
    .fetch_optional(&mut **transaction)
    .await
    .context("Impossible de charger une pierre tombale à recanoniser")?;
    let Some((device_id, sequence)) = source else {
        return Ok(());
    };
    if tombstone_reference_is_newer(transaction, entity_kind, new, &device_id, sequence).await? {
        store_tombstone_reference(transaction, entity_kind, new, &device_id, sequence).await?;
    }
    sqlx::query("DELETE FROM sync_tombstones WHERE entity_kind = ? AND entity_key = ?")
        .bind(entity_kind)
        .bind(old)
        .execute(&mut **transaction)
        .await
        .context("Impossible de supprimer l'ancienne pierre tombale")?;
    Ok(())
}

async fn apply_subscription_field(
    transaction: &mut Transaction<'_, Sqlite>,
    event: &SyncEvent,
    subscription_id: &str,
    field: &str,
    active: Option<bool>,
    platform: Option<Platform>,
) -> Result<ApplyOutcome> {
    let Some(canonical) = canonical_subscription_id(transaction, subscription_id).await? else {
        return Ok(ApplyOutcome::Pending("missing_subscription"));
    };
    if subscription_is_deleted(transaction, &canonical).await? {
        return Ok(ApplyOutcome::Applied);
    }
    if projection_feed_id(transaction, &canonical).await?.is_none() {
        return Ok(ApplyOutcome::Pending("missing_subscription_projection"));
    }
    apply_subscription_register(transaction, event, &canonical, field, active, platform).await?;
    Ok(ApplyOutcome::Applied)
}

async fn apply_subscription_register(
    transaction: &mut Transaction<'_, Sqlite>,
    event: &SyncEvent,
    canonical: &str,
    field: &str,
    active: Option<bool>,
    platform: Option<Platform>,
) -> Result<()> {
    if !event_is_newer(transaction, "subscription", canonical, field, event).await? {
        return Ok(());
    }
    let feed_id = projection_feed_id(transaction, canonical)
        .await?
        .context("L'abonnement synchronisé n'a pas de projection locale")?;
    match field {
        "active" => {
            sqlx::query("UPDATE feeds SET is_active = ? WHERE id = ?")
                .bind(active.context("Valeur d'activation absente")?)
                .bind(feed_id)
                .execute(&mut **transaction)
                .await
                .context("Impossible d'appliquer l'activation synchronisée")?;
        }
        "platform" => {
            sqlx::query("UPDATE feeds SET platform = ? WHERE id = ?")
                .bind(platform.context("Plateforme absente")?.as_str())
                .bind(feed_id)
                .execute(&mut **transaction)
                .await
                .context("Impossible d'appliquer la plateforme synchronisée")?;
        }
        _ => bail!("Unknown synchronized subscription field"),
    }
    store_event_version(transaction, "subscription", canonical, field, event).await
}

async fn apply_subscription_deleted(
    transaction: &mut Transaction<'_, Sqlite>,
    event: &SyncEvent,
    subscription_id: &str,
) -> Result<ApplyOutcome> {
    let Some(canonical) = canonical_subscription_id(transaction, subscription_id).await? else {
        return Ok(ApplyOutcome::Pending("missing_subscription"));
    };
    let previous_tombstone: Option<(String, i64)> = sqlx::query_as(
        r#"
            SELECT event_device_id, event_sequence
            FROM sync_tombstones
            WHERE entity_kind = 'subscription' AND entity_key = ?
        "#,
    )
    .bind(&canonical)
    .fetch_optional(&mut **transaction)
    .await
    .context("Impossible de charger la suppression précédente")?;
    if tombstone_event_is_newer(transaction, "subscription", &canonical, event).await? {
        store_event_tombstone(transaction, "subscription", &canonical, event).await?;
        if let Some((device_id, sequence)) = previous_tombstone {
            replace_parent_tombstone_reference(
                transaction,
                &SyncEventId {
                    device_id,
                    sequence,
                },
                &SyncEventId {
                    device_id: event.device_id.clone(),
                    sequence: event.sequence,
                },
            )
            .await?;
        }
    }
    remove_subscription_projection(transaction, &canonical).await?;
    Ok(ApplyOutcome::Applied)
}

async fn replace_parent_tombstone_reference(
    transaction: &mut Transaction<'_, Sqlite>,
    old: &SyncEventId,
    new: &SyncEventId,
) -> Result<()> {
    if old == new {
        return Ok(());
    }
    let urls: Vec<String> = sqlx::query_scalar(
        r#"
            SELECT DISTINCT normalized_url
            FROM sync_subscription_aliases
            WHERE parent_tombstone_device_id = ?
              AND parent_tombstone_sequence = ?
        "#,
    )
    .bind(&old.device_id)
    .bind(old.sequence)
    .fetch_all(&mut **transaction)
    .await
    .context("Impossible de trouver les réinscriptions à recanoniser")?;
    sqlx::query(
        r#"
            UPDATE sync_subscription_aliases
            SET parent_tombstone_device_id = ?, parent_tombstone_sequence = ?
            WHERE parent_tombstone_device_id = ?
              AND parent_tombstone_sequence = ?
        "#,
    )
    .bind(&new.device_id)
    .bind(new.sequence)
    .bind(&old.device_id)
    .bind(old.sequence)
    .execute(&mut **transaction)
    .await
    .context("Impossible de recanoniser les réinscriptions")?;
    for url in urls {
        merge_subscription_alias_group(transaction, &url, Some(new)).await?;
    }
    Ok(())
}

async fn ensure_subscription_projection(
    transaction: &mut Transaction<'_, Sqlite>,
    canonical: &str,
    url: &str,
    platform: Platform,
) -> Result<String> {
    if let Some(feed_id) = projection_feed_id(transaction, canonical).await? {
        return Ok(feed_id);
    }
    sqlx::query("INSERT INTO feeds (id, platform, url, is_active) VALUES (?, ?, ?, 0)")
        .bind(canonical)
        .bind(platform.as_str())
        .bind(url)
        .execute(&mut **transaction)
        .await
        .context("Impossible de créer la projection d'un abonnement synchronisé")?;
    Ok(canonical.to_string())
}

async fn remove_subscription_projection(
    transaction: &mut Transaction<'_, Sqlite>,
    canonical: &str,
) -> Result<()> {
    let feed_ids: Vec<String> = sqlx::query_scalar(
        r#"
            SELECT feed.id
            FROM feeds AS feed
            INNER JOIN sync_subscription_aliases AS alias
                ON alias.alias_id = feed.id
            WHERE alias.canonical_id = ?
        "#,
    )
    .bind(canonical)
    .fetch_all(&mut **transaction)
    .await
    .context("Impossible de charger la projection supprimée")?;
    for feed_id in feed_ids {
        sqlx::query("DELETE FROM articles WHERE feed_id = ?")
            .bind(&feed_id)
            .execute(&mut **transaction)
            .await
            .context("Impossible de supprimer les articles synchronisés")?;
        sqlx::query("DELETE FROM feeds WHERE id = ?")
            .bind(&feed_id)
            .execute(&mut **transaction)
            .await
            .context("Impossible de supprimer l'abonnement synchronisé")?;
    }
    Ok(())
}

async fn canonical_subscription_id(
    transaction: &mut Transaction<'_, Sqlite>,
    subscription_id: &str,
) -> Result<Option<String>> {
    sqlx::query_scalar("SELECT canonical_id FROM sync_subscription_aliases WHERE alias_id = ?")
        .bind(subscription_id)
        .fetch_optional(&mut **transaction)
        .await
        .context("Impossible de résoudre l'identité d'un abonnement")
}

async fn projection_feed_id(
    transaction: &mut Transaction<'_, Sqlite>,
    canonical: &str,
) -> Result<Option<String>> {
    sqlx::query_scalar(
        r#"
            SELECT feed.id
            FROM feeds AS feed
            INNER JOIN sync_subscription_aliases AS alias
                ON alias.alias_id = feed.id
            WHERE alias.canonical_id = ?
            ORDER BY feed.id
            LIMIT 1
        "#,
    )
    .bind(canonical)
    .fetch_optional(&mut **transaction)
    .await
    .context("Impossible de trouver la projection d'un abonnement")
}

async fn subscription_is_deleted(
    transaction: &mut Transaction<'_, Sqlite>,
    canonical: &str,
) -> Result<bool> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sync_tombstones WHERE entity_kind = 'subscription' AND entity_key = ?)",
    )
    .bind(canonical)
    .fetch_one(&mut **transaction)
    .await
    .context("Impossible de vérifier la suppression d'un abonnement")
}

async fn apply_article_field(
    transaction: &mut Transaction<'_, Sqlite>,
    event: &SyncEvent,
    article: &SyncArticleRef,
    field: &str,
    value: bool,
) -> Result<ApplyOutcome> {
    let Some(canonical) = canonical_subscription_id(transaction, &article.subscription_id).await?
    else {
        return Ok(ApplyOutcome::Pending("missing_subscription"));
    };
    if subscription_is_deleted(transaction, &canonical).await? {
        return Ok(ApplyOutcome::Applied);
    }
    let entity_key = article_entity_key(&canonical, &article.entry_key)?;
    if tombstone_exists(transaction, "article", &entity_key).await? {
        return Ok(ApplyOutcome::Applied);
    }
    let Some(article_id) = ensure_article_projection(transaction, &canonical, article).await?
    else {
        return Ok(ApplyOutcome::Pending("missing_subscription_projection"));
    };
    fill_article_metadata(transaction, &article_id, article).await?;
    if event_is_newer(transaction, "article", &entity_key, field, event).await? {
        match field {
            "read" => {
                sqlx::query(
                    r#"
                        UPDATE articles
                        SET is_read = ?,
                            is_archived = CASE
                                WHEN archive_reason = 'retention' AND ? = 0 THEN 0
                                ELSE is_archived
                            END,
                            archived_at = CASE
                                WHEN archive_reason = 'retention' AND ? = 0 THEN NULL
                                ELSE archived_at
                            END,
                            archive_reason = CASE
                                WHEN archive_reason = 'retention' AND ? = 0 THEN NULL
                                ELSE archive_reason
                            END
                        WHERE id = ?
                    "#,
                )
                .bind(value)
                .bind(value)
                .bind(value)
                .bind(value)
                .bind(&article_id)
                .execute(&mut **transaction)
                .await
                .context("Impossible d'appliquer l'état de lecture synchronisé")?;
            }
            "favorite" => {
                sqlx::query(
                    r#"
                        UPDATE articles
                        SET is_favorite = ?,
                            is_archived = CASE
                                WHEN archive_reason = 'retention' AND ? = 1 THEN 0
                                ELSE is_archived
                            END,
                            archived_at = CASE
                                WHEN archive_reason = 'retention' AND ? = 1 THEN NULL
                                ELSE archived_at
                            END,
                            archive_reason = CASE
                                WHEN archive_reason = 'retention' AND ? = 1 THEN NULL
                                ELSE archive_reason
                            END
                        WHERE id = ?
                    "#,
                )
                .bind(value)
                .bind(value)
                .bind(value)
                .bind(value)
                .bind(&article_id)
                .execute(&mut **transaction)
                .await
                .context("Impossible d'appliquer le favori synchronisé")?;
            }
            _ => bail!("Unknown synchronized article field"),
        }
        store_event_version(transaction, "article", &entity_key, field, event).await?;
    }
    Ok(ApplyOutcome::Applied)
}

async fn apply_article_archived(
    transaction: &mut Transaction<'_, Sqlite>,
    event: &SyncEvent,
    article: &SyncArticleRef,
) -> Result<ApplyOutcome> {
    let Some(canonical) = canonical_subscription_id(transaction, &article.subscription_id).await?
    else {
        return Ok(ApplyOutcome::Pending("missing_subscription"));
    };
    let entity_key = article_entity_key(&canonical, &article.entry_key)?;
    if tombstone_event_is_newer(transaction, "article", &entity_key, event).await? {
        store_event_tombstone(transaction, "article", &entity_key, event).await?;
    }
    if subscription_is_deleted(transaction, &canonical).await? {
        return Ok(ApplyOutcome::Applied);
    }
    if let Some(article_id) = ensure_article_projection(transaction, &canonical, article).await? {
        sqlx::query(
            r#"
                UPDATE articles
                SET is_archived = 1, archived_at = ?, archive_reason = 'manual',
                    content = NULL, content_kind = 'missing'
                WHERE id = ?
            "#,
        )
        .bind(event_datetime(event))
        .bind(article_id)
        .execute(&mut **transaction)
        .await
        .context("Impossible d'appliquer l'archivage synchronisé")?;
    }
    Ok(ApplyOutcome::Applied)
}

async fn ensure_article_projection(
    transaction: &mut Transaction<'_, Sqlite>,
    canonical: &str,
    article: &SyncArticleRef,
) -> Result<Option<String>> {
    if let Some(article_id) = sqlx::query_scalar(
        "SELECT article_id FROM sync_article_identities WHERE subscription_id = ? AND entry_key = ?",
    )
    .bind(canonical)
    .bind(&article.entry_key)
    .fetch_optional(&mut **transaction)
    .await
    .context("Impossible de rechercher l'identité logique d'un article")?
    .flatten()
    {
        return Ok(Some(article_id));
    }
    let Some(feed_id) = projection_feed_id(transaction, canonical).await? else {
        return Ok(None);
    };
    if let Some(article_id) = sqlx::query_scalar::<_, String>(
        "SELECT id FROM articles WHERE feed_id = ? AND entry_key = ?",
    )
    .bind(&feed_id)
    .bind(&article.entry_key)
    .fetch_optional(&mut **transaction)
    .await
    .context("Impossible de rechercher une projection d'article existante")?
    {
        store_article_identity(transaction, canonical, &article.entry_key, &article_id).await?;
        return Ok(Some(article_id));
    }

    let article_id = format!("{feed_id}::{}", article.entry_key);
    let source: String = sqlx::query_scalar("SELECT platform FROM feeds WHERE id = ?")
        .bind(&feed_id)
        .fetch_one(&mut **transaction)
        .await
        .context("Impossible de déterminer la source de l'article synchronisé")?;
    let published_at = article
        .published_at
        .as_deref()
        .map(DateTime::parse_from_rfc3339)
        .transpose()
        .context("Date de publication synchronisée invalide")?
        .map(|date| date.with_timezone(&Utc));
    sqlx::query(
        r#"
            INSERT INTO articles (
                id, feed_id, entry_key, title, author, published_at, url,
                source, content_kind
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'missing')
        "#,
    )
    .bind(&article_id)
    .bind(&feed_id)
    .bind(&article.entry_key)
    .bind(&article.title)
    .bind(&article.author)
    .bind(published_at)
    .bind(&article.url)
    .bind(source)
    .execute(&mut **transaction)
    .await
    .context("Impossible de créer la projection d'un article synchronisé")?;
    store_article_identity(transaction, canonical, &article.entry_key, &article_id).await?;
    Ok(Some(article_id))
}

async fn store_article_identity(
    transaction: &mut Transaction<'_, Sqlite>,
    canonical: &str,
    entry_key: &str,
    article_id: &str,
) -> Result<()> {
    sqlx::query(
        r#"
            INSERT INTO sync_article_identities (
                subscription_id, entry_key, article_id
            ) VALUES (?, ?, ?)
            ON CONFLICT(subscription_id, entry_key) DO UPDATE SET
                article_id = excluded.article_id
        "#,
    )
    .bind(canonical)
    .bind(entry_key)
    .bind(article_id)
    .execute(&mut **transaction)
    .await
    .context("Impossible d'enregistrer l'identité logique d'un article")?;
    Ok(())
}

async fn fill_article_metadata(
    transaction: &mut Transaction<'_, Sqlite>,
    article_id: &str,
    article: &SyncArticleRef,
) -> Result<()> {
    let published_at = article
        .published_at
        .as_deref()
        .map(DateTime::parse_from_rfc3339)
        .transpose()
        .context("Date de publication synchronisée invalide")?
        .map(|date| date.with_timezone(&Utc));
    sqlx::query(
        r#"
            UPDATE articles
            SET title = COALESCE(title, ?), author = COALESCE(author, ?),
                published_at = COALESCE(published_at, ?), url = COALESCE(url, ?)
            WHERE id = ?
        "#,
    )
    .bind(&article.title)
    .bind(&article.author)
    .bind(published_at)
    .bind(&article.url)
    .bind(article_id)
    .execute(&mut **transaction)
    .await
    .context("Impossible de compléter les métadonnées d'un article")?;
    Ok(())
}

fn article_entity_key(subscription_id: &str, entry_key: &str) -> Result<String> {
    serde_json::to_string(&(subscription_id, entry_key))
        .context("Impossible de sérialiser l'identité logique d'un article")
}

fn event_datetime(event: &SyncEvent) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(event.clock.physical_milliseconds)
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
}

async fn event_is_newer(
    transaction: &mut Transaction<'_, Sqlite>,
    entity_kind: &str,
    entity_key: &str,
    field: &str,
    event: &SyncEvent,
) -> Result<bool> {
    version_reference_is_newer(
        transaction,
        entity_kind,
        entity_key,
        field,
        &event.device_id,
        event.sequence,
    )
    .await
}

async fn version_reference_is_newer(
    transaction: &mut Transaction<'_, Sqlite>,
    entity_kind: &str,
    entity_key: &str,
    field: &str,
    device_id: &str,
    sequence: i64,
) -> Result<bool> {
    let candidate = load_event_version(transaction, device_id, sequence)
        .await?
        .context("La version candidate est absente du journal")?;
    let current: Option<(i64, i64, String, i64)> = sqlx::query_as(
        r#"
            SELECT event.hlc_physical_ms, event.hlc_counter,
                   event.device_id, event.sequence
            FROM sync_entity_versions AS version
            INNER JOIN sync_events AS event
                ON event.device_id = version.event_device_id
               AND event.sequence = version.event_sequence
            WHERE version.entity_kind = ? AND version.entity_key = ?
              AND version.field_name = ?
        "#,
    )
    .bind(entity_kind)
    .bind(entity_key)
    .bind(field)
    .fetch_optional(&mut **transaction)
    .await
    .context("Impossible de comparer un registre synchronisé")?;
    Ok(current.is_none_or(|(physical, counter, device, sequence)| {
        candidate
            > EventVersion {
                physical_milliseconds: physical,
                logical_counter: counter,
                device_id: device,
                sequence,
            }
    }))
}

async fn store_event_version(
    transaction: &mut Transaction<'_, Sqlite>,
    entity_kind: &str,
    entity_key: &str,
    field: &str,
    event: &SyncEvent,
) -> Result<()> {
    store_version_reference(
        transaction,
        entity_kind,
        entity_key,
        field,
        &event.device_id,
        event.sequence,
    )
    .await
}

async fn store_version_reference(
    transaction: &mut Transaction<'_, Sqlite>,
    entity_kind: &str,
    entity_key: &str,
    field: &str,
    device_id: &str,
    sequence: i64,
) -> Result<()> {
    sqlx::query(
        r#"
            INSERT INTO sync_entity_versions (
                entity_kind, entity_key, field_name,
                event_device_id, event_sequence
            ) VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(entity_kind, entity_key, field_name) DO UPDATE SET
                event_device_id = excluded.event_device_id,
                event_sequence = excluded.event_sequence
        "#,
    )
    .bind(entity_kind)
    .bind(entity_key)
    .bind(field)
    .bind(device_id)
    .bind(sequence)
    .execute(&mut **transaction)
    .await
    .context("Impossible d'enregistrer la version d'un registre synchronisé")?;
    Ok(())
}

async fn tombstone_event_is_newer(
    transaction: &mut Transaction<'_, Sqlite>,
    entity_kind: &str,
    entity_key: &str,
    event: &SyncEvent,
) -> Result<bool> {
    tombstone_reference_is_newer(
        transaction,
        entity_kind,
        entity_key,
        &event.device_id,
        event.sequence,
    )
    .await
}

async fn tombstone_reference_is_newer(
    transaction: &mut Transaction<'_, Sqlite>,
    entity_kind: &str,
    entity_key: &str,
    device_id: &str,
    sequence: i64,
) -> Result<bool> {
    let candidate = load_event_version(transaction, device_id, sequence)
        .await?
        .context("La version candidate de suppression est absente")?;
    let current: Option<(i64, i64, String, i64)> = sqlx::query_as(
        r#"
            SELECT event.hlc_physical_ms, event.hlc_counter,
                   event.device_id, event.sequence
            FROM sync_tombstones AS tombstone
            INNER JOIN sync_events AS event
                ON event.device_id = tombstone.event_device_id
               AND event.sequence = tombstone.event_sequence
            WHERE tombstone.entity_kind = ? AND tombstone.entity_key = ?
        "#,
    )
    .bind(entity_kind)
    .bind(entity_key)
    .fetch_optional(&mut **transaction)
    .await
    .context("Impossible de comparer une pierre tombale")?;
    Ok(current.is_none_or(|(physical, counter, device, sequence)| {
        candidate
            > EventVersion {
                physical_milliseconds: physical,
                logical_counter: counter,
                device_id: device,
                sequence,
            }
    }))
}

async fn store_event_tombstone(
    transaction: &mut Transaction<'_, Sqlite>,
    entity_kind: &str,
    entity_key: &str,
    event: &SyncEvent,
) -> Result<()> {
    store_tombstone_reference(
        transaction,
        entity_kind,
        entity_key,
        &event.device_id,
        event.sequence,
    )
    .await
}

async fn store_tombstone_reference(
    transaction: &mut Transaction<'_, Sqlite>,
    entity_kind: &str,
    entity_key: &str,
    device_id: &str,
    sequence: i64,
) -> Result<()> {
    sqlx::query(
        r#"
            INSERT INTO sync_tombstones (
                entity_kind, entity_key, event_device_id, event_sequence
            ) VALUES (?, ?, ?, ?)
            ON CONFLICT(entity_kind, entity_key) DO UPDATE SET
                event_device_id = excluded.event_device_id,
                event_sequence = excluded.event_sequence
        "#,
    )
    .bind(entity_kind)
    .bind(entity_key)
    .bind(device_id)
    .bind(sequence)
    .execute(&mut **transaction)
    .await
    .context("Impossible d'enregistrer une pierre tombale synchronisée")?;
    Ok(())
}

async fn tombstone_exists(
    transaction: &mut Transaction<'_, Sqlite>,
    entity_kind: &str,
    entity_key: &str,
) -> Result<bool> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sync_tombstones WHERE entity_kind = ? AND entity_key = ?)",
    )
    .bind(entity_kind)
    .bind(entity_key)
    .fetch_one(&mut **transaction)
    .await
    .context("Impossible de rechercher une pierre tombale")
}

async fn load_event_version(
    transaction: &mut Transaction<'_, Sqlite>,
    device_id: &str,
    sequence: i64,
) -> Result<Option<EventVersion>> {
    let row: Option<(i64, i64)> = sqlx::query_as(
        "SELECT hlc_physical_ms, hlc_counter FROM sync_events WHERE device_id = ? AND sequence = ?",
    )
    .bind(device_id)
    .bind(sequence)
    .fetch_optional(&mut **transaction)
    .await
    .context("Impossible de charger une version d'événement")?;
    Ok(
        row.map(|(physical_milliseconds, logical_counter)| EventVersion {
            physical_milliseconds,
            logical_counter,
            device_id: device_id.to_string(),
            sequence,
        }),
    )
}

async fn update_contiguous_cursor(
    transaction: &mut Transaction<'_, Sqlite>,
    device_id: &str,
) -> Result<()> {
    let mut cursor: i64 = sqlx::query_scalar(
        "SELECT contiguous_sequence FROM sync_import_cursors WHERE remote_device_id = ?",
    )
    .bind(device_id)
    .fetch_optional(&mut **transaction)
    .await
    .context("Impossible de lire le curseur d'import")?
    .unwrap_or(0);
    loop {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sync_events WHERE device_id = ? AND sequence = ?)",
        )
        .bind(device_id)
        .bind(cursor + 1)
        .fetch_one(&mut **transaction)
        .await
        .context("Impossible d'avancer le curseur d'import")?;
        if !exists {
            break;
        }
        cursor += 1;
    }
    sqlx::query(
        r#"
            INSERT INTO sync_import_cursors (remote_device_id, contiguous_sequence)
            VALUES (?, ?)
            ON CONFLICT(remote_device_id) DO UPDATE SET
                contiguous_sequence = MAX(contiguous_sequence, excluded.contiguous_sequence)
        "#,
    )
    .bind(device_id)
    .bind(cursor)
    .execute(&mut **transaction)
    .await
    .context("Impossible d'enregistrer le curseur d'import")?;
    Ok(())
}
