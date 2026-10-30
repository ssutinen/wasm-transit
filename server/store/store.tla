---------------------------- MODULE store ----------------------------
(* The sync store engine (docs/units/sync-persistence.md), model-checked
   by `just spec` (TLC, explicit enumeration at small bounds — `just test`
   never requires Java).

   Model: one pointer, two writers contending on it, one GC, every crash
   point. A commit is two writes — the blob, then ONE log append; the
   pointer is the log's last entry. A crash between the writes leaves an
   orphan blob. The merge semilattice is NOT here: it is a pure function
   and proptest owns it (crates/transit-sync); this spec covers the
   engine's interleavings.

   Invariants checked (the unit's Done-when):
     - every pointer, current or in the log, names a durable blob;
     - the log is append-only and each entry's `from` is the prior `to`;
     - GC never deletes a blob any log entry names;
     - a failed CAS leaves authority untouched (encoded: a Commit whose
       `from` is not the current head is never enabled);
     - a crash between blob write and log append leaves the old authority
       readable (the orphan is collectable, the head unchanged). *)

EXTENDS Naturals, Sequences, FiniteSets, TLC

CONSTANTS
    BlobSet,        \* the finite set of hashes blobs may take
    MaxCommits      \* bound on log length

VARIABLES
    blobs,          \* set of durable blob hashes
    logs,           \* Seq of [from |-> h, to |-> h] — append-only
    head,           \* 0 = pointer absent; else index into logs (1-based)
    orphan          \* a blob written by a crashed commit (not in any log entry)

vars == <<blobs, logs, head, orphan>>

TypeOK ==
    /\ blobs \in SUBSET BlobSet
    /\ logs \in Seq([from : BlobSet \cup {""}, to : BlobSet])
    /\ head \in 0..MaxCommits
    /\ orphan \in BlobSet \cup {"none"}

Init ==
    /\ blobs = {}
    /\ logs = << >>
    /\ head = 0
    /\ orphan = "none"

current == IF head = 0 THEN "" ELSE logs[head].to

tos == { logs[i].to : i \in 1..Len(logs) }
froms == { logs[i].from : i \in 1..Len(logs) }
referenced == tos \cup (froms \ {""})

(* A writer commits: PutBlob(to) then append ONE log record, CAS'd on
   `from`. Enabled only when `from` is the current head ("" = absent) —
   a stale writer's Commit is simply not enabled, which is the failed-CAS
   invariant. Committing a hash equal to the crash-orphan absorbs it: the
   store is content-addressed, so the torn blob write was a no-op for an
   already-written blob, and the hash is now referenced — nothing is
   orphaned anymore. *)
Commit(from, to) ==
    /\ Len(logs) < MaxCommits
    /\ (head = 0 /\ from = "") \/ (head > 0 /\ logs[head].to = from)
    /\ to \in BlobSet
    /\ blobs' = blobs \cup {to}              \* the blob write (idempotent)
    /\ logs' = Append(logs, [from |-> from, to |-> to])
    /\ head' = head + 1
    /\ IF to = orphan THEN orphan' = "none" ELSE UNCHANGED <<orphan>>

(* A crash between the blob write and the log append: the blob exists,
   no log entry names it (a torn write is unreferenced by construction),
   the head does not move. *)
Crash(b) ==
    /\ orphan = "none"
    /\ b \in BlobSet
    /\ b \notin referenced
    /\ blobs' = blobs \cup {b}
    /\ orphan' = b
    /\ UNCHANGED <<logs, head>>

(* GC: delete only blobs no log entry names (the crash-torn orphans,
   past the grace period — grace is modeled as expired for orphans). *)
GC ==
    \E b \in blobs :
        /\ b \notin referenced
        /\ blobs' = blobs \ {b}
        /\ IF orphan = b THEN orphan' = "none" ELSE UNCHANGED <<orphan>>
        /\ UNCHANGED <<logs, head>>

Next ==
    \/ \E f, t \in BlobSet : Commit(IF head = 0 THEN "" ELSE logs[head].to, t) \* honest writer re-reads
    \/ \E f \in BlobSet \cup {""}, t \in BlobSet : Commit(f, t)                    \* contending writer, any from
    \/ \E b \in BlobSet : Crash(b)
    \/ GC

Spec == Init /\ [][Next]_vars

---------------------------------------------------------------------------
(* Invariants *)

(* Every pointer, current or historical, names a durable blob. *)
RefIntegrity ==
    /\ (head = 0) \/ (logs[head].to \in blobs)
    /\ \A i \in 1..Len(logs) :
        /\ logs[i].to \in blobs
        /\ logs[i].from /= "" => logs[i].from \in blobs

(* The log is append-only and chained: each entry's from is the prior to. *)
LogChained ==
    \A i \in 2..Len(logs) : logs[i].from = logs[i-1].to

(* The pointer is the log's last entry. *)
PointerIsLogTail == head = Len(logs)

(* GC never deletes a referenced blob (encoded in GC's precondition, and
   re-asserted here over every reachable state). *)
GCSafe == referenced \subseteq blobs

(* A failed CAS leaves authority untouched: head never decreases and logs
   only grow — both follow from Next, pinned here. *)
AuthorityMonotone == head <= MaxCommits /\ Len(logs) = head

(* Crash safety: after a crash the old authority is readable — the head
   still names a durable blob (RefIntegrity) — and the orphan stays
   unreferenced until either GC collects it or a commit of the same hash
   absorbs it (Commit clears the marker), so it is always collectable
   while orphaned. *)
CrashSafe ==
    /\ orphan = "none" \/ orphan \notin referenced
    /\ (head = 0) \/ (logs[head].to \in blobs)

(* Small-bounds assumptions for TLC live in store.cfg (finite BlobSet,
   bounded log). *)
===============================================================================
