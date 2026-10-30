package store

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

func env(kind string) Envelope {
	return Envelope{Kind: kind, SchemaVersion: 1, ProducedBy: "cap-sync@sha256:" + strings.Repeat("ab", 32)}
}

func open(t *testing.T, dir string) *Store {
	t.Helper()
	s, err := Open(dir)
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	t.Cleanup(func() { s.Close() })
	return s
}

func TestCASCreateMoveAndConflict(t *testing.T) {
	s := open(t, t.TempDir())

	h1, seq, err := s.CAS("edits", "", []byte(`[{"id":"e1"}]`), env("edit_records"), "", "alice", 1000)
	if err != nil || seq != 1 {
		t.Fatalf("create: %v seq=%d", err, seq)
	}
	head, found, err := s.Head("edits")
	if err != nil || !found || head.Hash != h1 || head.Seq != 1 {
		t.Fatalf("head: %+v found=%v err=%v", head, found, err)
	}
	if head.Envelope.Kind != "edit_records" || head.Envelope.SchemaVersion != 1 {
		t.Fatalf("head envelope: %+v", head.Envelope)
	}

	// A second writer that saw h1 commits; one that saw nothing conflicts.
	h2, seq, err := s.CAS("edits", h1, []byte(`[{"id":"e1"},{"id":"e2"}]`), env("edit_records"), "", "bob", 2000)
	if err != nil || seq != 2 {
		t.Fatalf("move: %v seq=%d", err, seq)
	}
	_, _, err = s.CAS("edits", "", []byte(`[]`), env("edit_records"), "", "carol", 3000)
	var ce *ConflictError
	if !errors.As(err, &ce) || ce.Current != h2 {
		t.Fatalf("create-over-existing must conflict with current h2: %v", err)
	}
	_, _, err = s.CAS("edits", h1, []byte(`[]`), env("edit_records"), "", "carol", 3000)
	if !errors.As(err, &ce) || ce.Current != h2 {
		t.Fatalf("stale from must conflict: %v", err)
	}
	// A failed CAS left authority untouched.
	head, _, _ = s.Head("edits")
	if head.Hash != h2 || head.Seq != 2 {
		t.Fatalf("failed CAS moved the pointer: %+v", head)
	}
}

func TestRedeliveryIsIdempotentAndLogReplays(t *testing.T) {
	s := open(t, t.TempDir())
	doc := []byte(`[{"id":"e1","status":"submitted"}]`)
	h1, _, err := s.CAS("edits", "", doc, env("edit_records"), "", "alice", 1000)
	if err != nil {
		t.Fatal(err)
	}
	// At-least-once redelivery: committing the SAME bytes on top of the
	// same head is a no-op — same hash, same seq, no log growth.
	h1again, seqAgain, err := s.CAS("edits", h1, doc, env("edit_records"), "", "alice", 1001)
	if err != nil || h1again != h1 || seqAgain != 1 {
		t.Fatalf("same-bytes re-commit must be a no-op: %v %s seq=%d", err, h1again, seqAgain)
	}
	head, _, _ := s.Head("edits")
	entries, found, err := s.Log("edits", "")
	if err != nil || !found || len(entries) != 1 {
		t.Fatalf("log: %v found=%v entries=%d", err, found, len(entries))
	}
	if entries[0].From != "" || entries[0].To != head.Hash {
		t.Fatalf("log not chained: %+v", entries)
	}
	// A real change appends; delta since the first hash answers it.
	h2, _, err := s.CAS("edits", h1, []byte(`[{"id":"e1","x":1}]`), env("edit_records"), "", "bob", 2000)
	if err != nil {
		t.Fatal(err)
	}
	delta, found, err := s.Log("edits", entries[0].To)
	if err != nil || !found || len(delta) != 1 || delta[0].To != h2 || delta[0].From != h1 {
		t.Fatalf("delta: %v found=%v %+v", err, found, delta)
	}
	// An unknown since answers not-found (the client falls back to the
	// whole document).
	_, found, err = s.Log("edits", "sha256:"+strings.Repeat("00", 32))
	if err != nil || found {
		t.Fatalf("unknown since must be found=false: %v %v", found, err)
	}
	_ = h1
}

func TestCrashBetweenBlobAndLogLeavesOrphanAndOldAuthority(t *testing.T) {
	dir := t.TempDir()
	s := open(t, dir)
	h1, _, err := s.CAS("edits", "", []byte(`[1]`), env("edit_records"), "", "alice", 1000)
	if err != nil {
		t.Fatal(err)
	}
	// Simulate a crash after the blob write but before the log append:
	// the blob exists, no log entry names it.
	orphan, err := s.PutBlob([]byte(`[2]`), env("edit_records"), "")
	if err != nil {
		t.Fatal(err)
	}
	s.Close()

	// Reopen: the old authority is intact, the orphan still there
	// (within grace), then GC past the grace collects only the orphan.
	s2, err := Open(dir)
	if err != nil {
		t.Fatal(err)
	}
	head, found, _ := s2.Head("edits")
	if !found || head.Hash != h1 || head.Seq != 1 {
		t.Fatalf("crash must leave the old authority readable: %+v", head)
	}
	if _, _, err := s2.GetBlob(orphan); err != nil {
		t.Fatalf("orphan blob should still exist within grace: %v", err)
	}
	// Age the orphan past the grace period and GC.
	old := time.Now().Add(-48 * time.Hour)
	h, _ := ParseHash(orphan)
	os.Chtimes(filepath.Join(dir, "blobs", h), old, old)
	os.Chtimes(filepath.Join(dir, "blobs", h+".meta.json"), old, old)
	removed, err := s2.GC(OrphanGraceMs)
	if err != nil || removed != 2 { // blob + its sidecar
		t.Fatalf("gc: removed=%d err=%v", removed, err)
	}
	if _, _, err := s2.GetBlob(orphan); !errors.Is(err, ErrNotFound) {
		t.Fatalf("orphan must be collected: %v", err)
	}
	if _, _, err := s2.GetBlob(h1); err != nil {
		t.Fatalf("referenced blob must survive GC: %v", err)
	}
	s2.Close()
}

func TestHashAssertionFailsLoudly(t *testing.T) {
	s := open(t, t.TempDir())
	_, err := s.PutBlob([]byte(`[]`), env("edit_records"), "sha256:"+strings.Repeat("ff", 32))
	if err == nil || !strings.Contains(err.Error(), "hash mismatch") {
		t.Fatalf("a wrong module hash must fail loudly: %v", err)
	}
}

func TestConcurrentWritersConvergeOnOneHead(t *testing.T) {
	s := open(t, t.TempDir())
	if _, _, err := s.CAS("edits", "", []byte(`[]`), env("edit_records"), "", "seed", 0); err != nil {
		t.Fatal(err)
	}
	var wg sync.WaitGroup
	conflicts := make(chan *ConflictError, 64)
	for w := 0; w < 8; w++ {
		wg.Add(1)
		go func(w int) {
			defer wg.Done()
			for i := 0; i < 10; i++ {
				head, _, err := s.Head("edits")
				if err != nil {
					t.Error(err)
					return
				}
				doc := []byte(`[{"w":` + string(rune('0'+w)) + `,"i":` + string(rune('0'+i%10)) + `}]`)
				_, _, err = s.CAS("edits", head.Hash, doc, env("edit_records"), "", "w", int64(i))
				if err != nil {
					var ce *ConflictError
					if errors.As(err, &ce) {
						conflicts <- ce
					} else {
						t.Error(err)
						return
					}
				}
			}
		}(w)
	}
	wg.Wait()
	close(conflicts)
	head, _, _ := s.Head("edits")
	entries, _, _ := s.Log("edits", "")
	// Every commit appended exactly one chained entry; the log is
	// append-only and consistent.
	if int64(len(entries)) != head.Seq {
		t.Fatalf("log length %d != head seq %d", len(entries), head.Seq)
	}
	for i, e := range entries {
		if e.Seq != int64(i+1) {
			t.Fatalf("seq broken at %d: %+v", i, e)
		}
		if i > 0 && e.From != entries[i-1].To {
			t.Fatalf("chain broken at %d", i)
		}
	}
	// The head names a durable blob.
	if _, _, err := s.GetBlob(head.Hash); err != nil {
		t.Fatalf("head blob missing: %v", err)
	}
	t.Logf("%d conflicts, %d commits, head %s", len(conflicts), head.Seq, head.Hash)
}

func TestSecondProcessCannotOpenLockedStore(t *testing.T) {
	dir := t.TempDir()
	s := open(t, dir)
	if _, err := Open(dir); err == nil {
		t.Fatal("a second Open on the locked store must fail")
	}
	s.Close()
	if _, err := Open(dir); err != nil {
		t.Fatalf("after Close the store reopens: %v", err)
	}
}

func TestGetBlobRoundTrip(t *testing.T) {
	s := open(t, t.TempDir())
	h, err := s.PutBlob([]byte(`{"a":1}`), env("manifest"), "")
	if err != nil {
		t.Fatal(err)
	}
	data, gotEnv, err := s.GetBlob(h)
	if err != nil || string(data) != `{"a":1}` || gotEnv.Kind != "manifest" {
		t.Fatalf("getblob: %q %+v %v", data, gotEnv, err)
	}
	// Idempotent put returns the same hash.
	h2, err := s.PutBlob([]byte(`{"a":1}`), env("manifest"), "")
	if err != nil || h2 != h {
		t.Fatalf("put idempotent: %s vs %s (%v)", h, h2, err)
	}
	if _, _, err := s.GetBlob("sha256:" + strings.Repeat("00", 32)); !errors.Is(err, ErrNotFound) {
		t.Fatalf("missing blob: %v", err)
	}
}
