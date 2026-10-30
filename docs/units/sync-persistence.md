# Sync persistence

The durable sync store. **Landed (first slice):** the Go store engine
(`server/store/`) — content-addressed blobs with typed envelope
sidecars, one append-only JSONL log per pointer (the pointer is the
log's last entry; a commit is blob-write + one fsync'd append, CAS'd
on the `from` hash, flock'd across processes), GC of crash-torn
orphans, and idempotent redelivery (re-committing the same bytes is a
no-op: same hash, same seq). The HTTP surface (`-store <dir>`):
`/pointers/{name}`, `/pointers/{name}/log?since=`, `POST
/pointers/{name}` (CAS; `edit_records` runs cap-sync's merge under the
hood), `/blobs/{hash}`; `/edits` is the `edit_records` specialization
with an internal CAS retry loop (legacy whole-document protocol
unchanged), and `/manifest` serves the `manifest` ROOT pointer —
publishing is one CAS on it. cap-sync answers carry the canonical
`document` + `document_hash` (the module is the hash authority; the
store asserts `sha256(bytes) == hash`). D-56 is encoded: the receive
path clamps `updated_at` to the server clock. D-57 is encoded:
`compact_edits` drops retired records past the horizon (`-retention-days`,
default 90), committed as an ordinary logged CAS (at startup today).
Formats: `docs/FORMATS.md`. Engine spec: `server/store/store.tla`
(`just spec`). The legacy mode (no `-store`: in-memory, or `-edits`
flat file, seeded into the store on first use) still runs.

## What remains

- **Generated schemas + registry evidence.** JSON Schema emitted from
  the serde types (never hand-written) for every document kind,
  published with the registry's evidence; a Go test validates a stored
  blob against the schema and reads it with no Rust/wasm — plus a
  reader outside the wasm interpreting a blob through the envelope,
  the schema, and `docs/FORMATS.md` alone.
- **The plans pointer + publish → serve → resolve** (roadmap #1): plan
  documents on the store, the manifest growing plan pointers, clients
  resolving "which pick is in force" through cap-plans and re-pinning.
  Needs this store (landed) — the manifest-as-root makes the publish
  one CAS.
- **cap-authz**: enforce the reserved `writer`/`policy` log fields
  (fail-closed policy document, server-side assembly); today the
  fields are recorded, not enforced, and `/edits` is unauthenticated —
  fine on localhost, not beyond it.
- **Per-device journal unification** ([host-discipline.md](host-discipline.md)):
  the browser journal reuses the envelope + hash-chained log shape, so
  a device journal replays against the server store.
- **GC cadence**: orphans are collected at `Open`; add a periodic
  in-process sweep (the log is append-only forever by design — the
  delta window is the whole history; document growth is compaction's
  job, D-57).

## Done when (the remaining pins)

- The registry publishes the generated schemas and worked examples as
  evidence; a Go test validates a stored blob against the generated
  JSON Schema with no Rust/wasm and reads the plan chain from it;
  `docs/FORMATS.md` covers the rest (it landed with the store).
- A C-TRAN session syncs an edit document through a RESTARTED server
  (`-store`) and resolves a real plan chain through cap-plans — this
  also pays the sync/plans line of [grade-debt.md](grade-debt.md).

Already pinned by the landed slice (kept as regression gates, not
re-listed here): `go test -race ./...` covers CAS conflicts, log
replay, idempotent redelivery (same hash AND seq), crash-between-blob-
and-append (old authority readable, orphan collected at next `Open`),
delta sync (`/log?since=`, unknown since → whole document), the
`/blobs` ETag/envelope headers, and `just spec` model-checking
`server/store/store.tla` clean (TLC 2.19, 2026-10-30, in a toolbox
container: 1207 states generated, 214 distinct, no invariant violations,
fingerprint-collision estimate 1.2E-14; `CHECK_DEADLOCK FALSE` for the
artificial `MaxCommits` bound), plus the D-56 clock clamp through the
receive path, and edits surviving a server restart; `cargo test`
pins the transit-sync laws (clamp, compact, envelope) and the cap-sync
seam (canonical document + hash in every answer); caps-smoke pins the
hash formula end to end.
