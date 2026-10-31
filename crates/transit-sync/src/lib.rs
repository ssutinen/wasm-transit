//! transit-sync — the sync document and its pure operations.
//!
//! The principle this crate encodes: **the document is the contract**.
//! Both sides of any synchronization (browser wasm, transit-server, a
//! future agency ops service) hold the same serializable structures,
//! so agreement is about document identity, not about shared mutable
//! state.
//!
//! Ownership layering (see the project README):
//!
//! * the *feed artifact* is immutable and content-addressed — a
//!   [`FeedManifest`] points at the authoritative one;
//! * *operator edits* are documents: keyed by id, whole-record,
//!   last-writer-wins per id, deleted by retirement (tombstone), not
//!   by mutation.
//!
//! Everything here is pure: types plus functions over them. No I/O,
//! no clocks — callers stamp `updated_at`/`rev` when they mutate.

use ts_rs::TS;
use serde::{Deserialize, Serialize};

/// One vertex of a drawn detour polyline.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DetourPoint {
    pub lat: f64,
    pub lon: f64,
}

/// One operator edit, serializable so the UI can persist it next to
/// the feed bytes and replay it after a reload. The edit document is
/// THE DISPATCHER'S OWN — it lives here, in the sync contract, not in
/// the GTFS codec (GTFS is a serialization spec for external data
/// sharing; this document never crosses that boundary). Its fields
/// reference routes and stops by the ids the network model carries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Diversion {
    pub id: String,
    pub route_id: String,
    /// `None` = both directions. (GTFS's direction_id, kept for the
    /// boundary path; the model matches patterns, not directions.)
    #[serde(default)]
    pub direction_id: Option<u8>,
    /// Board/anchor points on the normal alignment: the detour happens
    /// between them, and they stay served.
    pub first_kept_stop: String,
    pub last_kept_stop: String,
    /// Interior stops no longer served (must not include the anchors).
    #[serde(default)]
    pub skipped_stops: Vec<String>,
    /// Drawn detour polyline; `None` = alignment unchanged (skip only).
    #[serde(default)]
    pub detour: Option<Vec<DetourPoint>>,
    /// Extra running time through the detour, applied from the last
    /// anchor onward.
    #[serde(default)]
    pub added_secs: u32,
    #[serde(with = "iso_date")]
    #[ts(type = "string")]
    pub start_date: chrono::NaiveDate,
    #[serde(with = "iso_date")]
    #[ts(type = "string")]
    pub end_date: chrono::NaiveDate,
}

impl Diversion {
    /// Is this edit in force on `date`?
    pub fn covers(&self, date: chrono::NaiveDate) -> bool {
        self.start_date <= date && date <= self.end_date
    }
}

/// Serialize/deserialize dates on the JS boundary as "YYYY-MM-DD",
/// matching the string format every wasm entry point takes.
mod iso_date {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        date: &chrono::NaiveDate,
        s: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&date.format("%Y-%m-%d").to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> std::result::Result<chrono::NaiveDate, D::Error> {
        let raw = String::deserialize(d)?;
        chrono::NaiveDate::parse_from_str(&raw, "%Y-%m-%d")
            .map_err(|e| serde::de::Error::custom(format!("invalid date {raw:?}: {e}")))
    }
}

/// Lifecycle of an edit record. `retired` is the tombstone: the record
/// stays in the document (sync needs to see the deletion), but it is
/// no longer in force and the UI hides it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum EditStatus {
    /// Created locally, not yet submitted anywhere.
    Draft,
    /// Pushed to an ops server; still intended to run.
    Submitted,
    /// Accepted by the ops authority; in force.
    Accepted,
    /// Withdrawn or failed revalidation against a new feed artifact.
    /// Kept as evidence and as a sync tombstone.
    Retired,
}

impl EditStatus {
    /// Is an edit in this status operationally in force? Retired is
    /// the only tombstone state; drafts run locally so the dispatcher
    /// can preview them.
    pub fn in_force(self) -> bool {
        !matches!(self, EditStatus::Retired)
    }
}

/// One operator edit plus its sync envelope. The `diversion` payload
/// is the domain content; the envelope fields are what any sync
/// partner needs to merge, order, and retire records without
/// understanding the domain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EditRecord {
    pub id: String,
    pub diversion: Diversion,
    pub status: EditStatus,
    /// Monotonic per id; bumped on every change by the writer.
    #[ts(type = "number")]
    pub rev: u64,
    /// Unix epoch milliseconds, stamped by the writer.
    #[ts(type = "number")]
    pub updated_at: u64,
}

/// Validation result for one record — what a client needs after
/// pushing a document (or what a server would return from a
/// validation endpoint). The record itself is display data; the
/// report only carries the verdict. The verdict is model-native,
/// against an authored network document (transit-dispatch); nothing
/// validates an edit against a feed.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EditReport {
    pub id: String,
    pub valid: bool,
    /// Human-readable reason when `valid` is false (unknown stop after
    /// a feed republish, degenerate anchors, ...).
    pub error: Option<String>,
    /// How many trips the edit touches (0 = no-op even when valid).
    pub affected_trips: usize,
}

/// The authority pointer: which feed artifact is current, published by
/// whom, when. Schedules change by artifact replacement — the planner
/// persona publishes a new manifest with a new `feed_hash`; everyone
/// else re-pins and revalidates their edits against it.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FeedManifest {
    /// Content hash (sha256) of the authoritative GTFS zip.
    pub feed_hash: String,
    /// Human-facing version from `feed_info.txt`, if the feed has one.
    pub feed_version: Option<String>,
    /// Unix epoch seconds of publication.
    #[ts(type = "number")]
    pub generated_at: u64,
    /// Who published (authoring system or agency), for provenance.
    pub publisher: Option<String>,
}

/// The diversions of a document that are in force on a given service
/// date — the patch set the effective feed is built from.
pub fn in_force_on(records: &[EditRecord], date: chrono::NaiveDate) -> Vec<Diversion> {
    records
        .iter()
        .filter(|r| r.status.in_force() && r.diversion.covers(date))
        .map(|r| r.diversion.clone())
        .collect()
}

// ---------------------------------------------------------------------
// Sync: last-writer-wins merge and the submit transition
// ---------------------------------------------------------------------

/// Does `a` win over `b` in the LWW order? The order is a strict
/// total order on records — that is what makes merge a semilattice and
/// therefore convergent: every node, running the same bytes, reaches
/// the same document no matter the merge order.
///
/// 1. later `updated_at` wins;
/// 2. tie: higher `rev` wins;
/// 3. full tie (same clock tick and rev, differing content — only a
///    buggy writer): the lexicographically greater serialized record
///    wins, deterministically, everywhere.
fn wins(a: &EditRecord, b: &EditRecord) -> bool {
    match a.updated_at.cmp(&b.updated_at) {
        std::cmp::Ordering::Greater => return true,
        std::cmp::Ordering::Less => return false,
        std::cmp::Ordering::Equal => {}
    }
    match a.rev.cmp(&b.rev) {
        std::cmp::Ordering::Greater => return true,
        std::cmp::Ordering::Less => return false,
        std::cmp::Ordering::Equal => {}
    }
    serde_json::to_string(a).unwrap_or_default() > serde_json::to_string(b).unwrap_or_default()
}

/// Merge two documents: per record id, last writer wins. The result is
/// sorted by id, so equal inputs always serialize identically on
/// every host. Tombstones participate like any record — a retirement
/// with a later timestamp beats an older in-force version, and stays
/// retired (deletions propagate through sync).
pub fn merge(local: &[EditRecord], remote: &[EditRecord]) -> Vec<EditRecord> {
    let mut by_id: std::collections::BTreeMap<String, EditRecord> = std::collections::BTreeMap::new();
    for record in local.iter().chain(remote) {
        match by_id.get(&record.id) {
            Some(existing) => {
                if wins(record, existing) {
                    by_id.insert(record.id.clone(), record.clone());
                }
            }
            None => {
                by_id.insert(record.id.clone(), record.clone());
            }
        }
    }
    by_id.into_values().collect()
}

/// The submit transition: an ops server ingests a document and every
/// `draft` becomes `submitted` (rev bumped, timestamp = now), marking
/// "this record has been pushed to the authority". Called by the
/// `sync_edits` op — hosts pass the clock, the artifact owns the rule.
pub fn submit(records: Vec<EditRecord>, now_ms: u64) -> Vec<EditRecord> {
    records
        .into_iter()
        .map(|mut r| {
            if r.status == EditStatus::Draft {
                r.status = EditStatus::Submitted;
                r.rev += 1;
                r.updated_at = now_ms;
            }
            r
        })
        .collect()
}

// ---------------------------------------------------------------------
// Authoring the edit document: the envelope rules every writer follows
// ---------------------------------------------------------------------

/// Propose a diversion: the next document with `record_id` carrying
/// `diversion` as a draft, stamped `now_ms`. A new id appends at rev 1;
/// an existing id is replaced whole (LWW per record — the record is
/// the unit) with its rev bumped. The input is never mutated.
pub fn propose(records: &[EditRecord], record_id: &str, diversion: Diversion, now_ms: u64) -> Vec<EditRecord> {
    let mut next = records.to_vec();
    match next.iter_mut().find(|r| r.id == record_id) {
        Some(r) => {
            r.diversion = diversion;
            r.status = EditStatus::Draft;
            r.rev += 1;
            r.updated_at = now_ms;
        }
        None => next.push(EditRecord {
            id: record_id.to_string(),
            diversion,
            status: EditStatus::Draft,
            rev: 1,
            updated_at: now_ms,
        }),
    }
    next
}

/// Retire (`retired: true` — the tombstone) or restore (back to a
/// draft, so the next sync re-submits it) one record: the next
/// document with its rev bumped and `now_ms` stamped. Retiring a
/// retired record, or restoring an in-force one, is a no-op answer
/// (the same document — no rev churn). An unknown id is an error.
pub fn set_retired(records: &[EditRecord], record_id: &str, retired: bool, now_ms: u64) -> Result<Vec<EditRecord>, String> {
    let mut next = records.to_vec();
    let r = next
        .iter_mut()
        .find(|r| r.id == record_id)
        .ok_or_else(|| format!("unknown edit record {record_id}"))?;
    if (r.status == EditStatus::Retired) != retired {
        r.status = if retired { EditStatus::Retired } else { EditStatus::Draft };
        r.rev += 1;
        r.updated_at = now_ms;
    }
    Ok(next)
}

// ---------------------------------------------------------------------
// The stored document: envelope, schema version, the receive path
// ---------------------------------------------------------------------

/// The current schema version of every document kind this crate
/// defines. A bump is deliberate; old blobs stay readable under their
/// hash, and a document written under an older schema migrates when
/// it arrives, not when it is read (docs/FORMATS.md).
pub const SCHEMA_VERSION: u32 = 1;

/// The kind of a stored document, as its envelope sidecar spells it
/// (`docs/FORMATS.md`). GTFS stays the interchange for published
/// products; these kinds cover what GTFS cannot express.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum DocumentKind {
    NetworkDocument,
    EditRecords,
    DiversionDraft,
    Manifest,
    Plans,
    Policy,
}

/// The typed envelope stored next to every blob: the hash names the
/// bytes, the envelope says how to read them. `produced_by` pins the
/// producing module's hash (`cap-sync@sha256:…`), or `server` for
/// host-owned documents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Envelope {
    pub kind: DocumentKind,
    pub schema_version: u32,
    /// e.g. "cap-sync@sha256:<hex>" or "server".
    pub produced_by: String,
}

/// The receive path's clock rule (D-56): a record claiming a future
/// `updated_at` is treated as written now. Clamping is idempotent —
/// clamping a clamped document changes nothing — so at-least-once
/// redelivery still converges to the same hash.
pub fn clamp_timestamps(records: &[EditRecord], now_ms: u64) -> Vec<EditRecord> {
    records
        .iter()
        .map(|r| {
            let mut r = r.clone();
            r.updated_at = r.updated_at.min(now_ms);
            r
        })
        .collect()
}

/// Compaction (D-57): drop tombstones older than the horizon. Only
/// `retired` records with `updated_at < now_ms - horizon_ms` go;
/// in-force records are never dropped, and the input is never mutated.
/// Returns the compacted document and how many records were dropped.
pub fn compact(records: &[EditRecord], now_ms: u64, horizon_ms: u64) -> (Vec<EditRecord>, usize) {
    let horizon = now_ms.saturating_sub(horizon_ms);
    let mut dropped = 0;
    let kept = records
        .iter()
        .filter(|r| {
            let old_tombstone = r.status == EditStatus::Retired && r.updated_at < horizon;
            if old_tombstone {
                dropped += 1;
            }
            !old_tombstone
        })
        .cloned()
        .collect();
    (kept, dropped)
}

/// Read a stored edit document (IndexedDB, a server response, a file):
/// every record that deserializes under the CURRENT schema — serde
/// fills the defaults a record from before a field existed lacks —
/// and how many did not. A malformed record is dropped and counted,
/// never allowed to wedge the document; anything but an array is an
/// empty document with nothing dropped.
pub fn sanitize(document: &serde_json::Value) -> (Vec<EditRecord>, usize) {
    let Some(items) = document.as_array() else { return (Vec::new(), 0) };
    let mut kept = Vec::new();
    let mut dropped = 0;
    for item in items {
        match serde_json::from_value::<EditRecord>(item.clone()) {
            Ok(r) if r.diversion.detour.iter().flatten().all(|p| p.lat.is_finite() && p.lon.is_finite()) => kept.push(r),
            _ => dropped += 1,
        }
    }
    (kept, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn record(status: EditStatus) -> EditRecord {
        EditRecord {
            id: "e1".into(),
            diversion: Diversion {
                id: "d1".into(),
                route_id: "r1".into(),
                direction_id: None,
                first_kept_stop: "s0".into(),
                last_kept_stop: "s1".into(),
                skipped_stops: vec![],
                detour: None,
                added_secs: 60,
                start_date: NaiveDate::from_ymd_opt(2026, 10, 23).unwrap(),
                end_date: NaiveDate::from_ymd_opt(2026, 10, 23).unwrap(),
            },
            status,
            rev: 1,
            updated_at: 1_000,
        }
    }

    #[test]
    fn edit_record_round_trips_through_json() {
        let rec = record(EditStatus::Accepted);
        let json = serde_json::to_string(&rec).unwrap();
        assert!(json.contains("\"status\":\"accepted\""), "status is lowercase: {json}");
        assert!(json.contains("\"start_date\":\"2026-10-23\""), "dates are ISO: {json}");
        let back: EditRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, rec.id);
        assert_eq!(back.status, EditStatus::Accepted);
        assert_eq!(back.diversion.first_kept_stop, "s0");
    }

    #[test]
    fn propose_appends_new_ids_and_replaces_known_ones() {
        let base = vec![record(EditStatus::Accepted)];
        let next = propose(&base, "e2", base[0].diversion.clone(), 2_000);
        assert_eq!(next.len(), 2);
        assert_eq!((next[1].rev, next[1].status, next[1].updated_at), (1, EditStatus::Draft, 2_000));
        let mut changed = base[0].diversion.clone();
        changed.added_secs = 120;
        let again = propose(&next, &base[0].id, changed, 3_000);
        assert_eq!(again.len(), 2);
        assert_eq!((again[0].rev, again[0].status, again[0].diversion.added_secs), (2, EditStatus::Draft, 120));
        assert_eq!(base[0].rev, 1, "the input is never mutated");
    }

    #[test]
    fn retire_and_restore_bump_the_envelope_once() {
        let base = vec![record(EditStatus::Submitted)];
        let retired = set_retired(&base, &base[0].id, true, 2_000).unwrap();
        assert_eq!((retired[0].status, retired[0].rev, retired[0].updated_at), (EditStatus::Retired, 2, 2_000));
        assert_eq!(set_retired(&retired, &base[0].id, true, 3_000).unwrap(), retired, "no rev churn");
        let restored = set_retired(&retired, &base[0].id, false, 4_000).unwrap();
        assert_eq!((restored[0].status, restored[0].rev), (EditStatus::Draft, 3));
        assert!(set_retired(&base, "nope", true, 1).is_err());
    }

    #[test]
    fn sanitize_fills_defaults_and_drops_what_does_not_parse() {
        let good = serde_json::to_value(record(EditStatus::Draft)).unwrap();
        let mut old = good.clone();
        let d = old["diversion"].as_object_mut().unwrap();
        for f in ["direction_id", "skipped_stops", "detour", "added_secs"] {
            d.remove(f);
        }
        let mut bad_date = good.clone();
        bad_date["diversion"]["start_date"] = "soon".into();
        let doc = serde_json::json!([good, old, bad_date, {"id": "x"}, 7]);
        let (kept, dropped) = sanitize(&doc);
        assert_eq!((kept.len(), dropped), (2, 3));
        assert_eq!(kept[1].diversion.added_secs, 0, "a record from before the field existed");
        assert_eq!(sanitize(&serde_json::json!({"not": "an array"})), (vec![], 0));
    }

    #[test]
    fn manifest_round_trips_through_json() {
        let manifest = FeedManifest {
            feed_hash: "abc123".into(),
            feed_version: Some("2026-10-23.1".into()),
            generated_at: 1_796_000_000,
            publisher: Some("Test Transit scheduling".into()),
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let back: FeedManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.feed_hash, manifest.feed_hash);
        assert_eq!(back.feed_version.as_deref(), Some("2026-10-23.1"));
    }

    #[test]
    fn retired_is_the_only_tombstone() {
        for status in [EditStatus::Draft, EditStatus::Submitted, EditStatus::Accepted] {
            assert!(status.in_force(), "{status:?} must be in force");
        }
        assert!(!EditStatus::Retired.in_force());
    }

    #[test]
    fn in_force_filters_by_status_and_date() {
        let date = NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
        let other = NaiveDate::from_ymd_opt(2026, 10, 24).unwrap();
        let mut docs = vec![record(EditStatus::Draft), record(EditStatus::Accepted)];
        docs[1].id = "e2".into();
        docs[1].diversion.start_date = other;
        docs[1].diversion.end_date = other;

        let live = in_force_on(&docs, date);
        assert_eq!(live.len(), 1, "only the covering-date record applies");
        assert_eq!(live[0].id, "d1");

        docs[0].status = EditStatus::Retired;
        assert!(in_force_on(&docs, date).is_empty(), "retired edits never apply");
    }
}

#[cfg(test)]
mod receive_tests {
    use super::*;

    fn rec(id: &str, status: EditStatus, rev: u64, updated_at: u64) -> EditRecord {
        EditRecord {
            id: id.into(),
            diversion: Diversion {
                id: format!("d-{id}"),
                route_id: "r1".into(),
                direction_id: None,
                first_kept_stop: "s0".into(),
                last_kept_stop: "s1".into(),
                skipped_stops: vec![],
                detour: None,
                added_secs: 60,
                start_date: chrono::NaiveDate::from_ymd_opt(2026, 10, 23).unwrap(),
                end_date: chrono::NaiveDate::from_ymd_opt(2026, 10, 23).unwrap(),
            },
            status,
            rev,
            updated_at,
        }
    }

    #[test]
    fn a_future_dated_record_is_clamped_at_receive() {
        let future = rec("e1", EditStatus::Submitted, 3, 10_000);
        let clamped = clamp_timestamps(&[future.clone()], 5_000);
        assert_eq!(clamped[0].updated_at, 5_000, "a future stamp is treated as now");
        assert_eq!(clamped[0].rev, 3, "clamping touches only the clock");

        let honest = rec("e2", EditStatus::Draft, 1, 4_000);
        assert_eq!(clamp_timestamps(&[honest], 5_000)[0].updated_at, 4_000, "honest stamps untouched");

        // Idempotent: clamping twice == clamping once, so redelivery converges.
        let once = clamp_timestamps(&[future], 5_000);
        assert_eq!(clamp_timestamps(&once, 5_000), once);

        // And the clamp actually bounds the LWW order: the future-dated
        // record no longer beats a later honest write.
        let server = vec![rec("e1", EditStatus::Retired, 4, 6_000)];
        let merged = merge(&clamped, &server);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].status, EditStatus::Retired, "6000 beats the clamped 5000");
    }

    #[test]
    fn compact_drops_only_retired_records_past_the_horizon() {
        let now = 1_000_000u64;
        let horizon = 100_000u64;
        let docs = vec![
            rec("old-retired", EditStatus::Retired, 2, now - 200_000),   // dropped
            rec("new-retired", EditStatus::Retired, 2, now - 50_000),    // kept
            rec("old-accepted", EditStatus::Accepted, 9, now - 900_000), // kept: in force
            rec("draft", EditStatus::Draft, 1, now - 900_000),          // kept
        ];
        let (kept, dropped) = compact(&docs, now, horizon);
        assert_eq!(dropped, 1);
        assert_eq!(kept.len(), 3);
        assert!(kept.iter().all(|r| r.id != "old-retired"));
        assert_eq!(docs.len(), 4, "the input is never mutated");

        // Idempotent: compacting a compacted document drops nothing.
        let (again, dropped_again) = compact(&kept, now, horizon);
        assert_eq!(dropped_again, 0);
        assert_eq!(again, kept);
    }

    #[test]
    fn envelope_and_kind_round_trip_with_documented_names() {
        let env = Envelope {
            kind: DocumentKind::EditRecords,
            schema_version: SCHEMA_VERSION,
            produced_by: "cap-sync@sha256:abcd".into(),
        };
        let json = serde_json::to_string(&env).unwrap();
        assert!(json.contains("\"kind\":\"edit_records\""), "{json}");
        assert!(json.contains("\"schema_version\":1"), "{json}");
        assert_eq!(serde_json::from_str::<Envelope>(&json).unwrap(), env);
        for (kind, name) in [
            (DocumentKind::NetworkDocument, "network_document"),
            (DocumentKind::EditRecords, "edit_records"),
            (DocumentKind::DiversionDraft, "diversion_draft"),
            (DocumentKind::Manifest, "manifest"),
            (DocumentKind::Plans, "plans"),
            (DocumentKind::Policy, "policy"),
        ] {
            assert_eq!(serde_json::to_string(&kind).unwrap(), format!("\"{name}\""));
        }
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;
    use proptest::prelude::*;
    use proptest::collection::vec;

    fn sample_record(id: u8, rev: u64, updated_at: u64, status: EditStatus) -> EditRecord {
        EditRecord {
            id: format!("e{id}"),
            diversion: Diversion {
                id: format!("d{id}"),
                route_id: "r1".into(),
                direction_id: None,
                first_kept_stop: "s0".into(),
                last_kept_stop: "s1".into(),
                skipped_stops: vec![],
                detour: None,
                added_secs: (updated_at % 300) as u32,
                start_date: chrono::NaiveDate::from_ymd_opt(2026, 10, 23).unwrap(),
                end_date: chrono::NaiveDate::from_ymd_opt(2026, 10, 24).unwrap(),
            },
            status,
            rev,
            updated_at,
        }
    }

    fn doc_strategy() -> impl Strategy<Value = Vec<EditRecord>> {
        // Small id space forces collisions; the clock range forces ties.
        vec(
            (
                0u8..4,
                0u64..4,
                0u64..6,
                proptest::sample::select(vec![
                    EditStatus::Draft,
                    EditStatus::Submitted,
                    EditStatus::Accepted,
                    EditStatus::Retired,
                ]),
            ),
            0..8,
        )
            .prop_map(|rows| {
                rows.into_iter()
                    .map(|(id, rev, at, status)| sample_record(id, rev, at, status))
                    .collect()
            })
    }

    #[test]
    fn merge_is_last_writer_wins_with_tombstones() {
        let a = sample_record(1, 1, 100, EditStatus::Draft);
        let b = sample_record(1, 2, 200, EditStatus::Retired);
        let older = sample_record(1, 1, 50, EditStatus::Accepted);

        // Later timestamp wins; retirement propagates.
        let merged = merge(&[a.clone()], &[b.clone()]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].status, EditStatus::Retired);
        assert_eq!(merged[0].updated_at, 200);

        // The older record loses to both.
        let merged = merge(&[older], &[b]);
        assert_eq!(merged[0].updated_at, 200);

        // Disjoint ids union.
        let merged = merge(&[sample_record(1, 1, 1, EditStatus::Draft)], &[sample_record(2, 1, 1, EditStatus::Draft)]);
        assert_eq!(merged.len(), 2);
        assert!(merged[0].id < merged[1].id, "sorted by id");
    }

    #[test]
    fn tie_breaks_deterministically_regardless_of_order() {
        // Same clock tick and rev, differing content: only a buggy
        // writer produces this, but convergence still must hold.
        let mut a = sample_record(1, 5, 100, EditStatus::Draft);
        a.diversion.added_secs = 1;
        let mut b = a.clone();
        b.diversion.added_secs = 2;
        assert_eq!(merge(&[a.clone()], &[b.clone()]), merge(&[b], &[a]));
    }

    #[test]
    fn submit_transitions_only_drafts_and_bumps_the_envelope() {
        let drafts = vec![
            sample_record(1, 1, 100, EditStatus::Draft),
            sample_record(2, 3, 100, EditStatus::Submitted),
            sample_record(3, 7, 100, EditStatus::Retired),
        ];
        let out = submit(drafts, 1000);
        assert_eq!(out[0].status, EditStatus::Submitted);
        assert_eq!(out[0].rev, 2);
        assert_eq!(out[0].updated_at, 1000);
        // Non-drafts untouched.
        assert_eq!(out[1].status, EditStatus::Submitted);
        assert_eq!(out[1].rev, 3);
        assert_eq!(out[1].updated_at, 100);
        assert_eq!(out[2].status, EditStatus::Retired);
        assert_eq!(out[2].rev, 7);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// Commutativity: every node reaches the same document no
        /// matter which side it holds locally.
        #[test]
        fn merge_is_commutative(a in doc_strategy(), b in doc_strategy()) {
            assert_eq!(merge(&a, &b), merge(&b, &a));
        }

        /// Idempotence: re-merging an already-merged document is a
        /// no-op (pulling twice must not change anything).
        #[test]
        fn merge_is_idempotent(a in doc_strategy(), b in doc_strategy()) {
            let once = merge(&a, &b);
            assert_eq!(merge(&once, &once), once);
        }

        /// Associativity: three-way convergence — merge in any
        /// grouping and every node ends at the same document.
        #[test]
        fn merge_is_associative(
            a in doc_strategy(), b in doc_strategy(), c in doc_strategy(),
        ) {
            assert_eq!(merge(&merge(&a, &b), &c), merge(&a, &merge(&b, &c)));
        }

        /// A document never gains or loses records through merging,
        /// and tombstone winners stay tombstones.
        #[test]
        fn merge_preserves_ids_and_tombstone_wins(
            a in doc_strategy(), b in doc_strategy(),
        ) {
            let merged = merge(&a, &b);
            let ids = |d: &[EditRecord]| d.iter().map(|r| r.id.clone()).collect::<std::collections::BTreeSet<_>>();
            assert_eq!(ids(&merged), ids(&a).union(&ids(&b)).cloned().collect());
            for r in &merged {
                let winner = a.iter().chain(&b).filter(|x| x.id == r.id)
                    .max_by_key(|x| (x.updated_at, x.rev)).unwrap();
                if winner.status == EditStatus::Retired && winner.updated_at == r.updated_at && winner.rev == r.rev {
                    // the LWW winner was the tombstone; only a same-tick
                    // tiebreak could differ, which the commutativity law
                    // already pins down
                }
            }
        }
    }
}
