//! Native dispatch tests for the sync capability — the seam without
//! wasm, over the e2e fixture's edit-record shape. The laws (the
//! merge's own laws are property-tested in transit-sync; here the
//! seam pins them through the module):
//!
//! 1. the union of ids, sorted; LWW per record id — the newer
//!    `updated_at` wins, a tombstone propagates over a stale live
//!    record;
//! 2. drafts submit (rev bumped, the host clock stamped); records
//!    already past draft pass through untouched;
//! 3. the answer is byte-deterministic — the same pair merges to
//!    the same document in any input order (client and server are
//!    roles, not order);
//! 4. garbage and bulk bytes are honest errors, never panics;
//! 5. the receive path clamps the client clock to `now_ms` (D-56),
//!    and the answer carries the canonical document bytes plus their
//!    hash, so a host stores exact bytes and never mints identity;
//! 6. `read_edits` migrates a received document at receive (defaults
//!    fill what age removed, malformed records are counted, never
//!    wedging it), and `compact_edits` is the D-57 tombstone commit.

use cap_sync::{dispatch, dispatch_bytes, dispatch_json, Request, Response};
use transit_sync::{Diversion, EditRecord, EditStatus};

const DATE: &str = "2026-10-23";

fn record(id: &str, status: EditStatus, rev: u64, updated_at: u64) -> EditRecord {
    EditRecord {
        id: id.into(),
        diversion: Diversion {
            id: format!("d-{id}"),
            route_id: "10".into(),
            direction_id: None,
            first_kept_stop: "A".into(),
            last_kept_stop: "B".into(),
            skipped_stops: vec!["C".into()],
            detour: None,
            added_secs: 300,
            start_date: chrono::NaiveDate::parse_from_str(DATE, "%Y-%m-%d").unwrap(),
            end_date: chrono::NaiveDate::parse_from_str(DATE, "%Y-%m-%d").unwrap(),
        },
        status,
        rev,
        updated_at,
    }
}

fn synced(client: Vec<EditRecord>, server: Vec<EditRecord>, now_ms: u64) -> Vec<EditRecord> {
    match dispatch(Request::SyncEdits { client, server, now_ms }).unwrap() {
        Response::SyncEdits { records, .. } => records,
        _ => panic!("sync_edits"),
    }
}

#[test]
fn merges_lww_and_submits() {
    // Client arrives with a draft; the server already holds a newer
    // tombstone for one record and nothing for the other.
    let draft = record("e-new", EditStatus::Draft, 1, 1);
    let mut retired = record("e-old", EditStatus::Accepted, 3, 2);
    retired.status = EditStatus::Retired;

    let mut stale_local = retired.clone();
    stale_local.status = EditStatus::Accepted;
    stale_local.updated_at = 1;

    let records = synced(vec![draft, stale_local], vec![retired], 1_000);
    let ids: Vec<&str> = records.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec!["e-new", "e-old"], "union of ids, sorted");
    assert_eq!(records[0].status, EditStatus::Submitted, "drafts submit");
    assert_eq!(records[0].rev, 2, "submit bumps rev");
    assert_eq!(records[0].updated_at, 1_000, "submit stamps the host clock");
    assert_eq!(records[1].status, EditStatus::Retired, "the tombstone (newer timestamp) won");
    assert_eq!(records[1].rev, 3, "past-draft records pass through untouched");
}

#[test]
fn the_pair_is_order_insensitive_and_deterministic() {
    // client and server are roles, not order: the same two documents
    // converge to the same answer either way — the semilattice the
    // library property-tests, pinned through the seam.
    let a = record("e1", EditStatus::Accepted, 2, 7);
    let mut b = record("e2", EditStatus::Draft, 1, 3);

    let one = synced(vec![a.clone(), b.clone()], vec![], 100);
    let two = synced(vec![], vec![b.clone(), a], 100);
    assert_eq!(one, two);
    assert_eq!(serde_json::to_string(&one).unwrap(), serde_json::to_string(&two).unwrap());

    // The submit transition is idempotent given the same clock: a
    // document already submitted does not bump again.
    b.status = EditStatus::Submitted;
    b.rev = 2;
    let once = synced(vec![b.clone()], vec![], 100);
    assert_eq!(once[0].rev, 2, "submitted records stay put");
}

#[test]
fn empty_documents_converge_to_empty() {
    // The server's first day: both sides empty, the answer is empty
    // — the protocol's zero case (the Go round-trip pins the same).
    assert!(synced(vec![], vec![], 0).is_empty());
}

#[test]
fn garbage_and_bulk_bytes_are_errors_not_panics() {
    let bad = dispatch_json(r#"{"op":"nope"}"#);
    assert!(bad.contains("invalid request"), "{bad}");
    let bad = dispatch_json(r#"{"op":"sync_edits","client":"not a list","server":[],"now_ms":0}"#);
    assert!(bad.contains("invalid request"), "{bad}");
    let bad = dispatch_bytes(0, b"whatever");
    assert!(bad.contains("unknown bulk op"), "the edit document is JSON: {bad}");

    // The tagged wire form round-trips.
    let ok = dispatch_json(r#"{"op":"sync_edits","client":[],"server":[],"now_ms":0}"#);
    assert!(ok.contains(r#""records":[]"#), "{ok}");
}

#[test]
fn sync_edits_clamps_the_client_clock() {
    // D-56: a client whose clock runs fast cannot mint a record that
    // beats every honest write — the receive path clamps to now_ms.
    let future = record("e1", EditStatus::Accepted, 5, 10_000);
    let mut tombstone = record("e1", EditStatus::Accepted, 3, 2);
    tombstone.status = EditStatus::Retired;
    tombstone.updated_at = 6_000;

    let out = match dispatch(Request::SyncEdits {
        client: vec![future],
        server: vec![tombstone.clone()],
        now_ms: 5_000,
    })
    .unwrap()
    {
        Response::SyncEdits { records, .. } => records,
        _ => panic!("sync_edits"),
    };
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].status, EditStatus::Retired, "the clamped stamp (5000) loses to the honest 6000");
    assert_eq!(out[0].updated_at, 6_000);

    // Without the tombstone the clamped record stands at now_ms.
    let out = synced(vec![record("e1", EditStatus::Draft, 1, 10_000)], vec![], 5_000);
    assert_eq!(out[0].updated_at, 5_000, "submit re-stamps drafts at the host clock anyway");
}

#[test]
fn the_answer_carries_the_canonical_document_and_its_hash() {
    let client = vec![record("e1", EditStatus::Draft, 1, 1)];
    match dispatch(Request::SyncEdits { client, server: vec![], now_ms: 1_000 }).unwrap() {
        Response::SyncEdits { records, document, document_hash } => {
            assert_eq!(document, serde_json::to_string(&records).unwrap(), "canonical bytes");
            assert_eq!(document_hash, cap_abi::canonical_hash(&records).unwrap(), "the module is the hash authority");
            assert!(document_hash.starts_with("sha256:"), "{document_hash}");
        }
        _ => panic!("sync_edits"),
    }
}

#[test]
fn read_edits_migrates_a_received_document_at_receive() {
    let good = serde_json::to_value(record("e1", EditStatus::Submitted, 2, 100)).unwrap();
    let mut old = good.clone();
    let d = old["diversion"].as_object_mut().unwrap();
    for f in ["direction_id", "skipped_stops", "detour", "added_secs"] {
        d.remove(f);
    }
    let mut bad_date = good.clone();
    bad_date["diversion"]["start_date"] = "soon".into();
    let doc = serde_json::json!([good, old, bad_date, {"id": "x"}, 7]);

    match dispatch(Request::ReadEdits { document: doc }).unwrap() {
        Response::ReadEdits { records, dropped, schema_version, document, document_hash } => {
            assert_eq!(records.len(), 2, "defaults fill what age removed");
            assert_eq!(dropped, 3, "malformed records are counted, never wedging the document");
            assert_eq!(records[1].diversion.added_secs, 0);
            assert_eq!(schema_version, transit_sync::SCHEMA_VERSION);
            assert_eq!(document, serde_json::to_string(&records).unwrap());
            assert_eq!(document_hash, cap_abi::canonical_hash(&records).unwrap());
        }
        _ => panic!("read_edits"),
    }
}

#[test]
fn compact_edits_is_the_d57_tombstone_commit() {
    let now = 1_000_000u64;
    let horizon = 100_000u64;
    let records = vec![
        record("old-retired", EditStatus::Retired, 2, now - 200_000),
        record("live", EditStatus::Accepted, 9, now - 900_000),
    ];
    match dispatch(Request::CompactEdits { records, now_ms: now, horizon_ms: horizon }).unwrap() {
        Response::CompactEdits { records, document, document_hash, dropped } => {
            assert_eq!(dropped, 1);
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].id, "live", "in-force records are never dropped");
            assert_eq!(document, serde_json::to_string(&records).unwrap());
            assert_eq!(document_hash, cap_abi::canonical_hash(&records).unwrap());
        }
        _ => panic!("compact_edits"),
    }
}
