//! cap-sync — the sync capability.
//!
//! One question: **what is the converged state of this edit
//! document?** The sync protocol is deliberately LWW per record id
//! plus tombstones (the terminal authority is the agency scheduling
//! system: propose → review → accept). The ops are that document's
//! lifecycle facets: `sync_edits` merges a client document with the
//! server's and transitions drafts to submitted; `read_edits`
//! migrates a received/stored document at the receive boundary;
//! `compact_edits` is the D-57 tombstone-retention commit. The host
//! owns both documents (wholesale-replaced, JS/store-owned) and the
//! clock (`now_ms` is the only route; hosts own time), and clamps the
//! incoming clock at receive (D-56). transit-sync is the domain
//! library — the semilattice merge is property-tested there; this
//! module is the seam. Answers carry the canonical `document` bytes
//! and their `document_hash` (the module is the hash authority), so a
//! host stores exact bytes and never mints identity.
//!
//! Validation is NOT here: edit records validate against the
//! NETWORK at load_edits — every model-side module's registration
//! answer (reported, never rejected — the sync contract). A second
//! answer here would be two answers to one question on the bus.
//! The module is stateless: no registries, no persistence —
//! document storage is the host's, and the sync-persistence
//! design (blobs + pointers, pointer CAS) is the host discipline's,
//! never a module's.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One operation call. Tagged like every capability's seam.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "cap_sync/")]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Merge the client's edit document with the server's (LWW per
    /// record id — `transit_sync::merge`, the property-tested
    /// semilattice), then transition drafts to submitted (the
    /// submit bumps rev and stamps `now_ms`). The receive path clamps
    /// the client's `updated_at` to `now_ms` first (D-56), so the
    /// server clock bounds the LWW order. The answer is the converged
    /// document — as records, and as the canonical `document` bytes
    /// with their `document_hash` — and the client adopts it
    /// wholesale.
    SyncEdits {
        client: Vec<transit_sync::EditRecord>,
        server: Vec<transit_sync::EditRecord>,
        #[ts(type = "number")]
        now_ms: u64,
    },
    /// Migrate-at-receive: parse a received or stored document under
    /// the current schema — serde defaults fill what age removed,
    /// malformed records are dropped and counted, never wedging the
    /// document (`transit_sync::sanitize`).
    ReadEdits {
        #[ts(type = "any")]
        document: serde_json::Value,
    },
    /// The D-57 compaction commit: drop tombstones older than
    /// `now_ms - horizon_ms`, as an ordinary (host-CAS'd) commit.
    CompactEdits {
        records: Vec<transit_sync::EditRecord>,
        #[ts(type = "number")]
        now_ms: u64,
        #[ts(type = "number")]
        horizon_ms: u64,
    },
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "cap_sync/")]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Response {
    SyncEdits {
        records: Vec<transit_sync::EditRecord>,
        /// The canonical serialization of `records` (serde_json) —
        /// the exact bytes a host stores.
        document: String,
        /// `cap_abi::canonical_hash(records)` — the document's identity.
        document_hash: String,
    },
    ReadEdits {
        records: Vec<transit_sync::EditRecord>,
        dropped: usize,
        schema_version: u32,
        /// The canonical serialization of the migrated `records` —
        /// the exact bytes a host stores when seeding a pointer.
        document: String,
        document_hash: String,
    },
    CompactEdits {
        records: Vec<transit_sync::EditRecord>,
        document: String,
        document_hash: String,
        dropped: usize,
    },
}

/// The typed dispatch (native hosts call this directly). Failures
/// are `{"error": ...}`, never panics — a total seam.
pub fn dispatch(request: Request) -> Result<Response, String> {
    match request {
        Request::SyncEdits { client, server, now_ms } => {
            let client = transit_sync::clamp_timestamps(&client, now_ms);
            let records = transit_sync::submit(transit_sync::merge(&client, &server), now_ms);
            let document = serde_json::to_string(&records).expect("records serialize");
            let document_hash = cap_abi::canonical_hash(&records)?;
            Ok(Response::SyncEdits { records, document, document_hash })
        }
        Request::ReadEdits { document } => {
            let (records, dropped) = transit_sync::sanitize(&document);
            let canonical = serde_json::to_string(&records).expect("records serialize");
            let hash = cap_abi::canonical_hash(&records)?;
            Ok(Response::ReadEdits {
                records,
                dropped,
                schema_version: transit_sync::SCHEMA_VERSION,
                document: canonical,
                document_hash: hash,
            })
        }
        Request::CompactEdits { records, now_ms, horizon_ms } => {
            let (records, dropped) = transit_sync::compact(&records, now_ms, horizon_ms);
            let document = serde_json::to_string(&records).expect("records serialize");
            let document_hash = cap_abi::canonical_hash(&records)?;
            Ok(Response::CompactEdits { records, document, document_hash, dropped })
        }
    }
}

/// The JSON seam: `{"op": ...}` in, `{"ok": ...}` / `{"error": ...}`
/// out — byte-compatible with every other capability module.
pub fn dispatch_json(text: &str) -> String {
    let outcome = match serde_json::from_str::<Request>(text) {
        Ok(request) => match dispatch(request) {
            Ok(response) => serde_json::json!({ "ok": response }),
            Err(error) => serde_json::json!({ "error": error }),
        },
        Err(error) => serde_json::json!({ "error": format!("invalid request: {error}") }),
    };
    serde_json::to_string(&outcome).expect("dispatch outcomes serialize")
}

/// The edit document is a JSON document — no bulk-bytes op exists;
/// the seam answers honestly instead of silently ignoring one.
pub fn dispatch_bytes(op: u32, _bytes: &[u8]) -> String {
    serde_json::json!({ "error": format!("unknown bulk op {op}: the edit document is JSON (sync_edits)") })
        .to_string()
}

cap_abi::capability! {
    json: dispatch_json,
    bytes: dispatch_bytes,
}
