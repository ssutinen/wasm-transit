# Formats — what the store persists

The at-rest contract of the sync store
(`docs/units/sync-persistence.md`). Everything here is readable
without any wasm: the blobs are plain bytes, the envelopes and logs
plain JSON, and the hash is sha256. The JSON Schemas generated from
the serde types (published with the registry's evidence) pin the
*document payloads*; this file pins the *storage envelope* and the
invariants the schema cannot say.

## Addressing and hash

- A hash is `sha256:<hex>` of the exact bytes —
  `cap_abi::content_hash`, the formula every module and host shares.
- A blob lives at `<store>/blobs/<hex>` (raw bytes, no extension).
  Its envelope sidecar is `<store>/blobs/<hex>.meta.json`.
- **The module is the hash authority for bus documents** (network,
  edit records, plans, walks): the producing module returns the
  canonical bytes and their hash, and the store asserts
  `sha256(blob) == hash`, failing loudly on mismatch. Host-owned
  documents (the manifest) are canonicalized and hashed by the host
  with the same formula; no module ever claims their identity.
- Canonical serialization of a document is its serde_json bytes
  (struct field order), so `sha256(canonical bytes)` is the document's
  identity on every host.

## The envelope sidecar

```json
{"kind": "edit_records", "schema_version": 1, "produced_by": "cap-sync@sha256:<hex>"}
```

- `kind` — one of `network_document`, `edit_records`,
  `diversion_draft`, `manifest`, `plans`, `policy`.
- `schema_version` — the payload schema's version; a reader pins a
  version and ignores newer ones, and a writer bumps it deliberately.
  Migrations happen at receive, never at read.
- `produced_by` — the producing module's hash (`cap-sync@sha256:…`),
  or `server` for host-owned documents.

## The pointer log

`<store>/pointers/<name>.log` is append-only JSONL, one record per
commit:

```json
{"seq": 2, "from": "sha256:aaa…", "to": "sha256:bbb…", "kind": "edit_records", "schema_version": 1, "produced_by": "cap-sync@sha256:…", "writer": "anonymous", "policy": null, "at_ms": 1798761600000}
```

- The pointer's current value is the last record's `to` — there is no
  separate pointer file.
- `from` is the hash the writer last saw; a commit appends only when
  the last `to` equals it (empty `from` = the pointer must not exist
  yet). A failed CAS changes nothing.
- `writer` is the verified JWT `sub` when authentication is on (D-58),
  else the caller's label (`X-Writer`, default `anonymous`). It leaves
  the server only through the log endpoint, and only to `audit`-granted
  callers (redacted otherwise). `policy` (the field) stays null — the
  policy itself is the `policy` pointer's blob (D-61), and that blob is
  never served over HTTP (`/blobs` 404s kind `policy`); clients get the
  caller's projection from `GET /permissions` instead.
- `<store>/lock` is an flock held for the process's lifetime, so two
  server processes on one store cannot interleave a commit.

A commit is two fsync'd writes: the blob (+ sidecar), then one log
record. A crash between them orphans the blob; GC (at `Open` and
after commits) deletes blobs no log record names and older than the
grace period (24 h).

## The HTTP surface (store mode, `-store <dir>`)

- `GET /pointers/{name}` → `{"name","hash","seq","envelope"}`.
- `GET /pointers/{name}/log?since=<hash>` → `{"entries":[…]}` — the
  records after the entry whose `to` is `since`; an unknown `since`
  answers `{"entries": [], "current": <head>, "whole": true}` and the
  client takes the whole current document.
- `POST /pointers/{name}` body `{"from": "<hash>", …}` → `200` with the
  new head, or `409` with `{"current": "<hash>"}`. For
  `edit_records` the body is `{"from", "client": <document>}` and the
  server runs cap-sync's `sync_edits` (receive-clamped, D-56) and
  stores the canonical converged bytes; other kinds take
  `{"from", "document": <value>}` verbatim.
- `GET /blobs/{hash}` → the raw bytes; ETag is the full `sha256:…`
  hash (304 on match); `X-Document-Kind` and `X-Schema-Version` carry
  the envelope.
- `/edits` is the `edit_records` specialization: `GET` reads the head
  blob, `POST` runs the merge with a CAS retry loop, so the legacy
  whole-document protocol is unchanged.

## Invariants the schema cannot say

- Dates cross every boundary as `"YYYY-MM-DD"` strings; times as u32
  seconds since service-day midnight (values past 86400 are
  legitimate — after-midnight trips); the store's own timestamps are
  `at_ms`, Unix epoch milliseconds from the host clock.
- The edit document's LWW order is a strict total order: later
  `updated_at` wins, then higher `rev`, then the lexicographically
  greater canonical record. The receive path clamps
  `updated_at = min(updated_at, now_ms)` (D-56), so the server clock
  bounds the order and redelivery is idempotent.
- `status: "retired"` is the only tombstone: the record stays in the
  document, is no longer in force, and deletions propagate through
  sync. Compaction (D-57) drops retired records older than the horizon
  as an ordinary logged commit.
- Nothing compressed is ever the durable format; compression is
  transport only.

## Worked examples

An `edit_records` blob is the canonical JSON array of records:

```json
[{"id":"e1","diversion":{"id":"d1","route_id":"10","direction_id":null,"first_kept_stop":"A","last_kept_stop":"B","skipped_stops":["C"],"detour":null,"added_secs":300,"start_date":"2026-10-23","end_date":"2026-10-23"},"status":"submitted","rev":2,"updated_at":1000}]
```

with sidecar
`{"kind":"edit_records","schema_version":1,"produced_by":"cap-sync@sha256:<hex>"}`
and, after a second commit, log record
`{"seq":2,"from":"sha256:<first>","to":"sha256:<this>","kind":"edit_records","schema_version":1,"produced_by":"cap-sync@sha256:<hex>","writer":"anonymous","policy":null,"at_ms":2000}`.

A `manifest` blob is the manifest object itself
(`{"network_hash","feed_hash","feed_version","streets_hash","walks_hash","wasm_hash","generated_at","publisher","services"}`),
sidecar `{"kind":"manifest","schema_version":1,"produced_by":"server"}`.
