// Package store is the content-addressed document store the sync
// server keeps when run with -store (docs/units/sync-persistence.md,
// docs/FORMATS.md).
//
// Layout:
//
//	<dir>/blobs/<hex>            one immutable blob — the document's exact bytes
//	<dir>/blobs/<hex>.meta.json  its typed envelope sidecar
//	<dir>/pointers/<name>.log    the pointer's append-only JSONL log
//	<dir>/lock                   flock: one writer process per store
//
// The pointer is the log's last entry — there is no separate pointer
// file. A commit is two fsync'd writes: the blob (+ sidecar), then one
// log record, appended only when the log's last `to` equals the
// writer's `from` (pointer CAS). A crash between the writes orphans the
// blob; GC deletes blobs no log record names and older than the grace
// period. The store never mints document identity for bus documents:
// callers pass the module-produced hash and the store asserts
// sha256(bytes) == hash, failing loudly on mismatch.
//
// The engine is model-checked (store.tla, `just spec`): every pointer,
// current or historical, names a durable blob; the log is append-only
// and each entry's `from` is the prior `to`; GC never deletes a
// referenced blob; a failed CAS leaves authority untouched.
package store

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"syscall"
	"time"
)

// OrphanGraceMs is how long an unreferenced blob survives before GC
// collects it — long enough that a crash mid-commit is never data
// loss, short enough that orphans do not pile up.
const OrphanGraceMs = 24 * 3600 * 1000

// Envelope is the typed sidecar stored next to every blob: the hash
// names the bytes, the envelope says how to read them.
type Envelope struct {
	Kind          string `json:"kind"` // network_document, edit_records, diversion_draft, manifest, plans
	SchemaVersion uint32 `json:"schema_version"`
	ProducedBy    string `json:"produced_by"` // "cap-sync@sha256:<hex>", or "server" for host-owned documents
}

// LogEntry is one commit record in a pointer's append-only log.
type LogEntry struct {
	Seq           int64   `json:"seq"`
	From          string  `json:"from"` // the prior `to`; "" on create
	To            string  `json:"to"`
	Kind          string  `json:"kind"`
	SchemaVersion uint32  `json:"schema_version"`
	ProducedBy    string  `json:"produced_by"`
	Writer        string  `json:"writer"` // reserved for cap-authz
	Policy        *string `json:"policy"` // reserved for cap-authz
	AtMs          int64   `json:"at_ms"`  // host clock, Unix epoch ms
}

// Head is a pointer's current value: the log's last entry.
type Head struct {
	Name     string   `json:"name"`
	Hash     string   `json:"hash"` // "" when the pointer does not exist
	Seq      int64    `json:"seq"`
	Envelope Envelope `json:"envelope"`
}

// ConflictError is a failed CAS: the pointer moved since the writer
// last saw it. Current is the hash to re-read and retry against.
type ConflictError struct {
	Pointer string
	Current string
	Seq     int64
}

func (e *ConflictError) Error() string {
	return fmt.Sprintf("store: pointer %q moved (current %s, seq %d)", e.Pointer, e.Current, e.Seq)
}

// ErrNotFound is a missing blob or pointer.
var ErrNotFound = errors.New("store: not found")

type headState struct {
	seq int64
	to  string
	env Envelope
	log []LogEntry
}

// Store is one open store directory. Safe for concurrent use; the
// flock makes it safe across processes too.
type Store struct {
	dir  string
	mu   sync.Mutex
	lock *os.File
	logs map[string]*headState
}

// Open creates (if needed) and locks the store, loads every pointer's
// log, and collects orphan blobs.
func Open(dir string) (*Store, error) {
	for _, sub := range []string{"blobs", "pointers"} {
		if err := os.MkdirAll(filepath.Join(dir, sub), 0o755); err != nil {
			return nil, err
		}
	}
	lock, err := os.OpenFile(filepath.Join(dir, "lock"), os.O_CREATE|os.O_RDWR, 0o644)
	if err != nil {
		return nil, err
	}
	if err := syscall.Flock(int(lock.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		lock.Close()
		return nil, fmt.Errorf("store %s is locked by another process: %w", dir, err)
	}
	s := &Store{dir: dir, lock: lock, logs: map[string]*headState{}}
	if err := s.loadLogs(); err != nil {
		lock.Close()
		return nil, err
	}
	if _, err := s.GC(OrphanGraceMs); err != nil {
		lock.Close()
		return nil, fmt.Errorf("gc at open: %w", err)
	}
	return s, nil
}

// Close releases the flock.
func (s *Store) Close() error {
	s.mu.Lock()
	defer s.mu.Unlock()
	syscall.Flock(int(s.lock.Fd()), syscall.LOCK_UN)
	return s.lock.Close()
}

// loadLogs reads every pointers/<name>.log into memory (the pointer is
// the last entry; the in-memory state is rebuilt from the log, never
// from a side file).
func (s *Store) loadLogs() error {
	entries, err := os.ReadDir(filepath.Join(s.dir, "pointers"))
	if err != nil {
		return err
	}
	for _, e := range entries {
		if !strings.HasSuffix(e.Name(), ".log") {
			continue
		}
		name := strings.TrimSuffix(e.Name(), ".log")
		data, err := os.ReadFile(filepath.Join(s.dir, "pointers", e.Name()))
		if err != nil {
			return err
		}
		st := &headState{}
		for _, line := range strings.Split(strings.TrimSpace(string(data)), "\n") {
			if line == "" {
				continue
			}
			var entry LogEntry
			if err := json.Unmarshal([]byte(line), &entry); err != nil {
				return fmt.Errorf("pointer %s log line: %w", name, err)
			}
			// Append-only and chained: each entry's from is the prior to.
			if entry.Seq != st.seq+1 || entry.From != st.to {
				return fmt.Errorf("pointer %s log broken at seq %d", name, entry.Seq)
			}
			st.log = append(st.log, entry)
			st.seq = entry.Seq
			st.to = entry.To
			st.env = Envelope{Kind: entry.Kind, SchemaVersion: entry.SchemaVersion, ProducedBy: entry.ProducedBy}
		}
		s.logs[name] = st
	}
	return nil
}

// ParseHash validates "sha256:<hex>" and returns the hex.
func ParseHash(hash string) (string, error) {
	if !strings.HasPrefix(hash, "sha256:") {
		return "", fmt.Errorf("not a content hash: %q", hash)
	}
	hexPart := strings.TrimPrefix(hash, "sha256:")
	if len(hexPart) != 64 {
		return "", fmt.Errorf("not a sha256 hex: %q", hash)
	}
	if _, err := hex.DecodeString(hexPart); err != nil {
		return "", fmt.Errorf("not a sha256 hex: %q", hash)
	}
	return hexPart, nil
}

func hashOf(data []byte) string {
	sum := sha256.Sum256(data)
	return "sha256:" + hex.EncodeToString(sum[:])
}

func (s *Store) blobPath(h string) string   { return filepath.Join(s.dir, "blobs", h) }
func (s *Store) metaPath(h string) string   { return s.blobPath(h) + ".meta.json" }
func (s *Store) logPath(name string) string { return filepath.Join(s.dir, "pointers", name+".log") }

// writeFileDurably writes tmp + fsync + rename (atomic on POSIX), so a
// torn write is an orphan, never a partial blob.
func writeFileDurably(path string, data []byte) error {
	tmp := path + ".tmp"
	f, err := os.OpenFile(tmp, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0o644)
	if err != nil {
		return err
	}
	if _, err := f.Write(data); err != nil {
		f.Close()
		return err
	}
	if err := f.Sync(); err != nil {
		f.Close()
		return err
	}
	if err := f.Close(); err != nil {
		return err
	}
	return os.Rename(tmp, path)
}

// PutBlob stores data under its content hash with its envelope. It is
// idempotent: an existing blob is returned as-is. wantHash, when
// non-empty, is the module-produced document hash the caller asserts
// against sha256(data) — the store never mints bus-document identity.
func (s *Store) PutBlob(data []byte, env Envelope, wantHash string) (string, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.putBlobLocked(data, env, wantHash)
}

func (s *Store) putBlobLocked(data []byte, env Envelope, wantHash string) (string, error) {
	hash := hashOf(data)
	if wantHash != "" && wantHash != hash {
		return "", fmt.Errorf("hash mismatch: module says %s, bytes are %s", wantHash, hash)
	}
	h, err := ParseHash(hash)
	if err != nil {
		return "", err
	}
	if _, err := os.Stat(s.blobPath(h)); err == nil {
		return hash, nil // idempotent
	}
	if err := writeFileDurably(s.blobPath(h), data); err != nil {
		return "", err
	}
	meta, err := json.Marshal(env)
	if err != nil {
		return "", err
	}
	if err := writeFileDurably(s.metaPath(h), append(meta, '\n')); err != nil {
		return "", err
	}
	return hash, nil
}

// GetBlob reads a blob and its envelope.
func (s *Store) GetBlob(hash string) ([]byte, Envelope, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	h, err := ParseHash(hash)
	if err != nil {
		return nil, Envelope{}, err
	}
	data, err := os.ReadFile(s.blobPath(h))
	if err != nil {
		if os.IsNotExist(err) {
			return nil, Envelope{}, ErrNotFound
		}
		return nil, Envelope{}, err
	}
	metaRaw, err := os.ReadFile(s.metaPath(h))
	if err != nil {
		if os.IsNotExist(err) {
			return nil, Envelope{}, ErrNotFound
		}
		return nil, Envelope{}, err
	}
	var env Envelope
	if err := json.Unmarshal(metaRaw, &env); err != nil {
		return nil, Envelope{}, fmt.Errorf("blob %s envelope: %w", hash, err)
	}
	return data, env, nil
}

// Head returns a pointer's current value. found is false when the
// pointer has no commits yet.
func (s *Store) Head(name string) (Head, bool, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	st, ok := s.logs[name]
	if !ok || st.seq == 0 {
		return Head{Name: name}, false, nil
	}
	return Head{Name: name, Hash: st.to, Seq: st.seq, Envelope: st.env}, true, nil
}

// CAS commits data as the pointer's next value. from is the hash the
// writer last saw ("" = the pointer must not exist). On a moved
// pointer it returns *ConflictError with the current hash; the caller
// re-reads, re-merges (the kind's module op), and retries. The blob is
// written first, then ONE log record is appended (the commit); the
// in-memory head follows.
func (s *Store) CAS(name, from string, data []byte, env Envelope, wantHash, writer string, atMs int64) (string, int64, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	st, exists := s.logs[name]
	if !exists {
		st = &headState{}
		s.logs[name] = st
	}
	current := st.to
	if from == "" {
		if exists && st.seq > 0 {
			return "", 0, &ConflictError{Pointer: name, Current: current, Seq: st.seq}
		}
	} else if current != from {
		return "", 0, &ConflictError{Pointer: name, Current: current, Seq: st.seq}
	}
	hash, err := s.putBlobLocked(data, env, wantHash)
	if err != nil {
		return "", 0, err
	}
	if exists && st.to == hash {
		// Idempotent redelivery: the same document is already the head —
		// no log entry, no seq bump (at-least-once converges to the same
		// hash AND the same seq).
		return hash, st.seq, nil
	}
	entry := LogEntry{
		Seq: st.seq + 1, From: current, To: hash,
		Kind: env.Kind, SchemaVersion: env.SchemaVersion, ProducedBy: env.ProducedBy,
		Writer: writer, AtMs: atMs,
	}
	line, err := json.Marshal(entry)
	if err != nil {
		return "", 0, err
	}
	f, err := os.OpenFile(s.logPath(name), os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o644)
	if err != nil {
		return "", 0, err
	}
	if _, err := f.Write(append(line, '\n')); err != nil {
		f.Close()
		return "", 0, err
	}
	if err := f.Sync(); err != nil { // the commit: one fsync'd append
		f.Close()
		return "", 0, err
	}
	f.Close()
	st.log = append(st.log, entry)
	st.seq = entry.Seq
	st.to = hash
	st.env = env
	return hash, entry.Seq, nil
}

// Log returns the entries after the entry whose To is since ("" = all
// entries). found is false when since is not in the log — the caller
// then falls back to the whole current document (delta sync outside
// the log's history).
func (s *Store) Log(name, since string) (entries []LogEntry, found bool, err error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	st, ok := s.logs[name]
	if !ok {
		return nil, false, nil
	}
	if since == "" {
		return append([]LogEntry{}, st.log...), true, nil
	}
	for i, e := range st.log {
		if e.To == since {
			return append([]LogEntry{}, st.log[i+1:]...), true, nil
		}
	}
	return nil, false, nil
}

// GC deletes blobs no log record names (neither `to` nor `from`) and
// older than graceMs — the crash-torn orphans. It never touches a
// referenced blob, and never trims a log.
func (s *Store) GC(graceMs int64) (int, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	referenced := map[string]bool{}
	for _, st := range s.logs {
		for _, e := range st.log {
			if e.To != "" {
				if h, err := ParseHash(e.To); err == nil {
					referenced[h] = true
				}
			}
			if e.From != "" {
				if h, err := ParseHash(e.From); err == nil {
					referenced[h] = true
				}
			}
		}
	}
	entries, err := os.ReadDir(filepath.Join(s.dir, "blobs"))
	if err != nil {
		return 0, err
	}
	now := time.Now().UnixMilli()
	removed := 0
	for _, e := range entries {
		name := e.Name()
		base := strings.TrimSuffix(strings.TrimSuffix(name, ".meta.json"), ".tmp")
		if referenced[base] {
			continue
		}
		info, err := e.Info()
		if err != nil {
			continue
		}
		if now-info.ModTime().UnixMilli() < graceMs {
			continue
		}
		if err := os.Remove(filepath.Join(s.dir, "blobs", name)); err != nil {
			return removed, err
		}
		removed++
	}
	return removed, nil
}
