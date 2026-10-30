package main

// Store-mode round trips (server with -store): the edit document lives
// in the content-addressed store as the edits pointer, /edits is its
// specialization, the generic /pointers and /blobs surface serves CAS
// and delta sync, and the manifest is the root pointer. The pins are
// docs/units/sync-persistence.md's Done-when: persistence through a
// restart, CAS conflict answering the current hash, idempotent
// redelivery, delta sync over the log, and the D-56 clock clamp
// through the receive path.

import (
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strconv"
	"strings"
	"testing"
	"time"

	"rustwasm/server/store"
)

func newStoreServer(t *testing.T, dir string) *server {
	t.Helper()
	s := &server{wasm: newTestHost(t), retentionDays: 90}
	st, err := store.Open(dir)
	if err != nil {
		t.Fatalf("store open: %v", err)
	}
	s.store = st
	s.seedStore("")
	t.Cleanup(func() { st.Close() })
	return s
}

func storeMux(s *server) *http.ServeMux {
	mux := http.NewServeMux()
	mux.HandleFunc("/edits", s.handleEdits)
	mux.HandleFunc("/pointers/", s.handlePointers)
	mux.HandleFunc("/blobs/", s.handleBlob)
	mux.HandleFunc("/manifest", s.handleManifest)
	return mux
}

func editDoc(id, status string, rev, at int64) string {
	diversion := `{"id":"d-` + id + `","route_id":"10","direction_id":null,` +
		`"first_kept_stop":"A","last_kept_stop":"B","skipped_stops":["C"],` +
		`"detour":null,"added_secs":300,"start_date":"` + date + `","end_date":"` + date + `"}`
	return `[{"id":"` + id + `","diversion":` + diversion +
		`,"status":"` + status + `","rev":` + strconv.FormatInt(rev, 10) +
		`,"updated_at":` + strconv.FormatInt(at, 10) + `}]`
}

func httpPost(t *testing.T, url, body string) (int, http.Header, string) {
	t.Helper()
	resp, err := http.Post(url, "application/json", strings.NewReader(body))
	if err != nil {
		t.Fatalf("post %s: %v", url, err)
	}
	defer resp.Body.Close()
	b, _ := io.ReadAll(resp.Body)
	return resp.StatusCode, resp.Header, string(b)
}

func httpGet(t *testing.T, url string) (int, http.Header, string) {
	t.Helper()
	resp, err := http.Get(url)
	if err != nil {
		t.Fatalf("get %s: %v", url, err)
	}
	defer resp.Body.Close()
	b, _ := io.ReadAll(resp.Body)
	return resp.StatusCode, resp.Header, string(b)
}

// The unit's headline pin: an edit document syncs through a RESTARTED
// server (a fresh process over the same store directory) and the
// document — tombstone included — is intact.
func TestStoreEditsSurviveRestart(t *testing.T) {
	dir := t.TempDir()
	s1 := newStoreServer(t, dir)
	base := httptest.NewServer(storeMux(s1)).URL

	code, hdr, body := httpPost(t, base+"/edits", editDoc("e1", "draft", 1, 100))
	if code != 200 || !strings.Contains(body, `"status":"submitted"`) {
		t.Fatalf("post draft: %d %s", code, body)
	}
	etag := strings.Trim(hdr.Get("ETag"), `"`)

	code, _, body = httpPost(t, base+"/edits", editDoc("e1", "retired", 2, time.Now().UnixMilli()))
	if code != 200 || !strings.Contains(body, `"status":"retired"`) {
		t.Fatalf("post tombstone: %d %s", code, body)
	}

	// Restart: a new server (new store handle) over the same directory.
	s1.store.Close()
	s2 := newStoreServer(t, dir)
	base2 := httptest.NewServer(storeMux(s2)).URL

	code, hdr2, body := httpGet(t, base2+"/edits")
	if code != 200 || !strings.Contains(body, `"retired"`) {
		t.Fatalf("after restart: %d %s", code, body)
	}
	if hdr2.Get("ETag") == `""`+etag+`""` {
		t.Fatal("etag must move on a new commit")
	}

	// The pointer log is the audit: two commits, chained.
	code, _, logBody := httpGet(t, base2+"/pointers/edits/log")
	if code != 200 {
		t.Fatalf("log: %d %s", code, logBody)
	}
	var logResp struct {
		Entries []store.LogEntry `json:"entries"`
	}
	if err := json.Unmarshal([]byte(logBody), &logResp); err != nil {
		t.Fatalf("log json: %v", err)
	}
	if len(logResp.Entries) != 3 || logResp.Entries[0].From != "" || logResp.Entries[1].From != logResp.Entries[0].To {
		t.Fatalf("log not append-only+chained: %+v", logResp.Entries)
	}
	if logResp.Entries[0].Kind != "edit_records" || !strings.HasPrefix(logResp.Entries[0].ProducedBy, "cap-sync@sha256:") {
		t.Fatalf("envelope in the log: %+v", logResp.Entries[0])
	}
}

// CAS conflict over HTTP: a POST /pointers/edits with a stale from
// answers 409 with the current hash; the client re-reads and retries.
// Idempotent redelivery: re-POSTing the same document converges to the
// same head hash.
func TestStoreCASConflictAndRedelivery(t *testing.T) {
	s := newStoreServer(t, t.TempDir())
	base := httptest.NewServer(storeMux(s)).URL

	code, _, body := httpPost(t, base+"/edits", editDoc("e1", "draft", 1, 100))
	if code != 200 {
		t.Fatalf("post: %d %s", code, body)
	}
	_, _, headBody := httpGet(t, base+"/pointers/edits")
	var head struct {
		Hash string `json:"hash"`
	}
	if err := json.Unmarshal([]byte(headBody), &head); err != nil {
		t.Fatal(err)
	}

	// A stale from ("" = must not exist, but it does) conflicts.
	req, _ := json.Marshal(map[string]any{"from": "", "client": json.RawMessage(editDoc("e2", "draft", 1, 100))})
	resp, err := http.Post(base+"/pointers/edits", "application/json", strings.NewReader(string(req)))
	if err != nil {
		t.Fatal(err)
	}
	if resp.StatusCode != http.StatusConflict {
		b, _ := io.ReadAll(resp.Body)
		resp.Body.Close()
		t.Fatalf("stale from must 409, got %d: %s", resp.StatusCode, b)
	}
	var ce struct {
		Current string `json:"current"`
	}
	json.NewDecoder(resp.Body).Decode(&ce)
	resp.Body.Close()
	if ce.Current != head.Hash {
		t.Fatalf("409 must carry the current hash: %q vs %q", ce.Current, head.Hash)
	}

	// Redelivery of the same client document: converges (submit is
	// idempotent given the same clock domain — the head may move once
	// for the new record, but re-POSTing the identical document again
	// must not change the head hash).
	_, _, b1 := httpPost(t, base+"/edits", editDoc("e2", "draft", 1, 100))
	_, _, head1 := httpGet(t, base+"/pointers/edits")
	_, _, b2 := httpPost(t, base+"/edits", editDoc("e2", "draft", 1, 100))
	_, _, head2 := httpGet(t, base+"/pointers/edits")
	_ = b1
	_ = b2
	if head1 != head2 { // same document re-POSTed: same hash AND same seq
		t.Fatalf("redelivering the same document changed the head: %s -> %s", head1, head2)
	}
}

// Delta sync over the log: a client holding `since` gets only the later
// entries; an unknown since answers whole=true (fall back to the whole
// current document). Blobs serve with the hash as ETag and the envelope
// in headers.
func TestStoreDeltaSyncAndBlobs(t *testing.T) {
	s := newStoreServer(t, t.TempDir())
	base := httptest.NewServer(storeMux(s)).URL

	httpPost(t, base+"/edits", editDoc("e1", "draft", 1, 100))
	httpPost(t, base+"/edits", editDoc("e2", "draft", 1, 100))
	httpPost(t, base+"/edits", editDoc("e3", "draft", 1, 100))

	_, _, logBody := httpGet(t, base+"/pointers/edits/log")
	var full struct {
		Entries []store.LogEntry `json:"entries"`
		Current string           `json:"current"`
	}
	json.Unmarshal([]byte(logBody), &full)
	if len(full.Entries) != 4 {
		t.Fatalf("entries: %d", len(full.Entries))
	}

	_, _, deltaBody := httpGet(t, base+"/pointers/edits/log?since="+full.Entries[0].To)
	var delta struct {
		Entries    []store.LogEntry `json:"entries"`
		SinceFound bool             `json:"since_found"`
	}
	json.Unmarshal([]byte(deltaBody), &delta)
	if !delta.SinceFound || len(delta.Entries) != 3 || delta.Entries[0].To != full.Entries[1].To {
		t.Fatalf("delta: %+v", delta)
	}

	_, _, wholeBody := httpGet(t, base+"/pointers/edits/log?since=sha256:"+strings.Repeat("00", 32))
	var whole struct {
		Whole bool `json:"whole"`
	}
	json.Unmarshal([]byte(wholeBody), &whole)
	if !whole.Whole {
		t.Fatalf("unknown since must answer whole=true: %s", wholeBody)
	}

	// The head blob serves raw with ETag + envelope headers, and 304s.
	hash := full.Current
	code, hdr, blob := httpGet(t, base+"/blobs/"+hash)
	if code != 200 || hdr.Get("ETag") != `"`+hash+`"` || hdr.Get("X-Document-Kind") != "edit_records" {
		t.Fatalf("blob: %d %v %s", code, hdr, blob)
	}
	if !strings.Contains(blob, `"id":"e3"`) {
		t.Fatalf("blob body: %s", blob)
	}
	req, _ := http.NewRequest("GET", base+"/blobs/"+hash, nil)
	req.Header.Set("If-None-Match", `"`+hash+`"`)
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	if resp.StatusCode != http.StatusNotModified {
		t.Fatalf("etag revalidation: %d", resp.StatusCode)
	}
}

// D-56 through the receive path: a client whose clock runs 10 h fast
// cannot mint a record that wins past now — the stored stamp is clamped
// to the server clock.
func TestStoreReceiveClampsTheClock(t *testing.T) {
	s := newStoreServer(t, t.TempDir())
	base := httptest.NewServer(storeMux(s)).URL

	before := time.Now().UnixMilli()
	future := time.Now().Add(10 * time.Hour).UnixMilli()
	code, _, body := httpPost(t, base+"/edits", editDoc("e1", "accepted", 9, future))
	if code != 200 {
		t.Fatalf("post: %d %s", code, body)
	}
	var doc []struct {
		ID        string `json:"id"`
		UpdatedAt int64  `json:"updated_at"`
	}
	if err := json.Unmarshal([]byte(body), &doc); err != nil || len(doc) != 1 {
		t.Fatalf("doc: %v %s", err, body)
	}
	after := time.Now().UnixMilli()
	if doc[0].UpdatedAt < before || doc[0].UpdatedAt > after {
		t.Fatalf("stored updated_at %d outside [%d, %d]: the future stamp was not clamped", doc[0].UpdatedAt, before, after)
	}
}

// The manifest is the root pointer in store mode: GET /manifest commits
// it once and serves the pointer's blob (ETag = its hash); a second GET
// revalidates for 304 and does not append a log entry.
func TestStoreManifestPointer(t *testing.T) {
	s := newStoreServer(t, t.TempDir())
	base := httptest.NewServer(storeMux(s)).URL

	code, hdr, body := httpGet(t, base+"/manifest")
	if code != 200 || !strings.Contains(body, `"wasm_hash"`) {
		t.Fatalf("manifest: %d %s", code, body)
	}
	etag := hdr.Get("ETag")
	if etag == "" {
		t.Fatal("manifest must carry an ETag in store mode")
	}

	req, _ := http.NewRequest("GET", base+"/manifest", nil)
	req.Header.Set("If-None-Match", etag)
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	if resp.StatusCode != http.StatusNotModified {
		t.Fatalf("manifest 304: %d", resp.StatusCode)
	}

	_, _, logBody := httpGet(t, base+"/pointers/manifest/log")
	var logResp struct {
		Entries []store.LogEntry `json:"entries"`
	}
	json.Unmarshal([]byte(logBody), &logResp)
	if len(logResp.Entries) != 1 || logResp.Entries[0].Kind != "manifest" || logResp.Entries[0].ProducedBy != "server" {
		t.Fatalf("manifest pointer: %+v", logResp.Entries)
	}
	if `"`+logResp.Entries[0].To+`"` != etag {
		t.Fatalf("manifest ETag %s != pointer %s", etag, logResp.Entries[0].To)
	}
}

// The generic raw-commit path: a host-owned document kind (plans) posts
// verbatim with a from-CAS, and a wrong document_hash is rejected.
func TestStoreRawPointerCommit(t *testing.T) {
	s := newStoreServer(t, t.TempDir())
	base := httptest.NewServer(storeMux(s)).URL

	mk := func(from, doc, hash string) (int, string) {
		req, _ := json.Marshal(map[string]any{"from": from, "kind": "plans", "document": doc, "document_hash": hash})
		code, _, body := httpPost(t, base+"/pointers/plans", string(req))
		return code, body
	}
	code, body := mk("", `[{"id":"spring"}]`, "")
	if code != 200 {
		t.Fatalf("create plans: %d %s", code, body)
	}
	// A wrong asserted hash fails loudly (the store never mints identity).
	code, body = mk("", `[]`, "sha256:"+strings.Repeat("ff", 32))
	if code == 200 {
		t.Fatalf("wrong document_hash must fail: %s", body)
	}
	// Move with the right from: create answered the head hash.
	_, _, headBody := httpGet(t, base+"/pointers/plans")
	var head struct {
		Hash string `json:"hash"`
	}
	json.Unmarshal([]byte(headBody), &head)
	code, body = mk(head.Hash, `[{"id":"spring"},{"id":"fall"}]`, "")
	if code != 200 {
		t.Fatalf("move plans: %d %s", code, body)
	}
	_, _, blob := httpGet(t, base+"/blobs/"+head.Hash)
	_ = blob
	_, _, head2 := httpGet(t, base+"/pointers/plans")
	var head3 struct {
		Hash string `json:"hash"`
		Seq  int64  `json:"seq"`
	}
	json.Unmarshal([]byte(head2), &head3)
	if head3.Seq != 2 || head3.Hash == head.Hash {
		t.Fatalf("plans pointer: %+v", head3)
	}
}
