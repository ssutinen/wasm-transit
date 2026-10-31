package main

// Edit sync: GET/POST /edits. The host treats documents as opaque
// JSON — the merge and the submit transition happen in the SYNC
// CAPABILITY (cap-sync) via its one op, so client and server agree
// by construction (same bytes, same input, same output).
//
// POST /edits  body: the client's document ([]EditRecord).
//              response: the converged document — the client
//              adopts it wholesale.
// GET  /edits  returns the server's document.
//
// With -store, /edits is the edit_records pointer of the content-
// addressed store (docs/FORMATS.md): POST runs the CAS retry loop over
// the merge, GET serves the head blob (ETag = its hash). Without it,
// the legacy flat-file (-edits) or in-memory path runs unchanged.

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"strings"
	"time"
)

func (s *server) handleEdits(w http.ResponseWriter, r *http.Request) {
	if s.store != nil {
		s.handleStoreEdits(w, r)
		return
	}
	switch r.Method {
	case http.MethodGet:
		s.handleGetEdits(w, r)
	case http.MethodPost:
		s.handlePostEdits(w, r)
	default:
		http.Error(w, "use GET or POST", http.StatusMethodNotAllowed)
	}
}

// handleStoreEdits is /edits over the store: the pointer's head blob is
// the authority, and a POST commits the merged document through a CAS
// retry loop (a contending writer is re-read and re-merged, never
// clobbered).
func (s *server) handleStoreEdits(w http.ResponseWriter, r *http.Request) {
	switch r.Method {
	case http.MethodGet:
		head, found, err := s.store.Head("edits")
		if err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}
		if !found {
			w.Header().Set("Content-Type", "application/json")
			w.Write([]byte("[]"))
			return
		}
		data, _, err := s.store.GetBlob(head.Hash)
		if err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}
		etag := `"` + head.Hash + `"`
		w.Header().Set("ETag", etag)
		w.Header().Set("Cache-Control", "no-cache")
		if r.Header.Get("If-None-Match") == etag {
			w.WriteHeader(http.StatusNotModified)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		w.Write(data)
	case http.MethodPost:
		body, err := io.ReadAll(io.LimitReader(r.Body, 32<<20))
		if err != nil {
			http.Error(w, "read body: "+err.Error(), http.StatusBadRequest)
			return
		}
		document, hash, err := s.commitEdits(json.RawMessage(body), writerOf(r, "anonymous"))
		if err != nil {
			if strings.Contains(err.Error(), "moved") {
				http.Error(w, "conflict: the edit document moved, retry", http.StatusConflict)
				return
			}
			http.Error(w, err.Error(), http.StatusBadRequest)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		w.Header().Set("ETag", `"`+hash+`"`)
		w.Write([]byte(document))
	default:
		http.Error(w, "use GET or POST", http.StatusMethodNotAllowed)
	}
}

// maybeCompactEdits runs the D-57 tombstone-retention commit at startup
// (and is reusable after): retired records older than the horizon are
// dropped by cap-sync's compact_edits, committed as an ordinary CAS'd
// log entry, so the compaction is audited like any other commit.
func (s *server) maybeCompactEdits() {
	if s.store == nil {
		return
	}
	nowMs := time.Now().UnixMilli()
	horizonMs := int64(s.retentionDays) * 24 * 3600 * 1000
	head, found, err := s.store.Head("edits")
	if err != nil || !found {
		return
	}
	body, _, err := s.store.GetBlob(head.Hash)
	if err != nil {
		return
	}
	records, _, _, _, err := s.readEdits(body)
	if err != nil {
		return
	}
	var resp struct {
		Document     string `json:"document"`
		DocumentHash string `json:"document_hash"`
		Dropped      int    `json:"dropped"`
	}
	if err := s.syncOp(map[string]any{
		"op": "compact_edits", "records": records, "now_ms": nowMs, "horizon_ms": horizonMs,
	}, &resp); err != nil || resp.Dropped == 0 {
		return
	}
	if _, _, err := s.store.CAS("edits", head.Hash, []byte(resp.Document), s.syncEnvelope(), resp.DocumentHash, "gc", nowMs); err == nil {
		fmt.Printf("compacted %d tombstone(s) past the %d-day horizon\n", resp.Dropped, s.retentionDays)
	}
}

func (s *server) handleGetEdits(w http.ResponseWriter, r *http.Request) {
	s.editsMu.Lock()
	defer s.editsMu.Unlock()
	doc := s.edits
	if len(doc) == 0 {
		doc = json.RawMessage("[]")
	}
	w.Header().Set("Content-Type", "application/json")
	w.Write(doc)
}

func (s *server) handlePostEdits(w http.ResponseWriter, r *http.Request) {
	body, err := io.ReadAll(io.LimitReader(r.Body, 32<<20))
	if err != nil {
		http.Error(w, "read body: "+err.Error(), http.StatusBadRequest)
		return
	}

	s.editsMu.Lock()
	defer s.editsMu.Unlock()

	serverDoc := s.edits
	if len(serverDoc) == 0 {
		serverDoc = json.RawMessage("[]")
	}

	// One trip into the sync capability: merge + submit. The host
	// passes the clock; the module owns every rule. Validation is
	// NOT here — edit records validate against the network at
	// load_edits in every model-side module (the client reads those
	// reports through the bus).
	var resp struct {
		Records json.RawMessage `json:"records"`
	}
	err = s.wasm.Invoke(r.Context(), map[string]any{
		"op":     "sync_edits",
		"client": json.RawMessage(body),
		"server": serverDoc,
		"now_ms": time.Now().UnixMilli(),
	}, &resp)
	if err != nil {
		http.Error(w, "sync op failed: "+err.Error(), http.StatusInternalServerError)
		return
	}

	if err := s.persistEdits(resp.Records); err != nil {
		http.Error(w, "persist edits: "+err.Error(), http.StatusInternalServerError)
		return
	}
	s.edits = resp.Records

	w.Header().Set("Content-Type", "application/json")
	w.Write(resp.Records)
}

// persistEdits writes the document atomically (temp + rename) when a
// path is configured; in-memory mode is a no-op.
func (s *server) persistEdits(doc json.RawMessage) error {
	if s.editsPath == "" {
		return nil
	}
	tmp := s.editsPath + ".tmp"
	if err := os.WriteFile(tmp, doc, 0o644); err != nil {
		return fmt.Errorf("write %s: %w", tmp, err)
	}
	return os.Rename(tmp, s.editsPath)
}

// loadEdits restores a persisted document at startup.
func loadEdits(path string) json.RawMessage {
	if path == "" {
		return nil
	}
	body, err := os.ReadFile(path)
	if err != nil {
		if !os.IsNotExist(err) {
			fmt.Fprintf(os.Stderr, "warning: read %s: %v\n", path, err)
		}
		return nil
	}
	return json.RawMessage(body)
}
