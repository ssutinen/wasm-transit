package main

// The round-trip proof: the server hosts the SYNC CAPABILITY
// (cap-sync) and gets the same converged edit document the browser
// and Node compute — the "same contracts everywhere" claim,
// executed, not asserted.

import (
	"bytes"
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"strconv"
	"strings"
	"testing"
	"time"
)

const date = "2026-10-23" // Friday: WK + HOL both run

// fixtureZip mirrors the e2e fixture (web/e2e/make_fixtures.py).
func fixtureZip(t *testing.T) []byte {
	t.Helper()
	files := map[string]string{
		"agency.txt": "agency_id,agency_name,agency_url,agency_timezone\n1,T,https://x,UTC\n",
		"routes.txt": "route_id,agency_id,route_short_name,route_type\n10,1,10,3\n20,1,20,3\n",
		"stops.txt": "stop_id,stop_name,stop_lat,stop_lon\n" +
			"A,Main & 1st,45.500,-73.500\nB,Main & 2nd,45.510,-73.510\nC,Main & 3rd,45.520,-73.520\n",
		"trips.txt": "route_id,service_id,trip_id,direction_id,block_id,shape_id\n" +
			"10,WK,t10,0,B1,sh1\n20,WK,t20,0,B1,\n10,WK,t30,0,,\n10,SUN,t11,0,,\n20,HOL,t12,0,B2,\n",
		"stop_times.txt": "trip_id,arrival_time,departure_time,stop_id,stop_sequence\n" +
			"t10,08:00:00,08:00:00,A,1\nt10,08:10:00,08:10:00,B,2\n" +
			"t20,08:20:00,08:20:00,A,1\nt20,08:30:00,08:30:00,B,2\n" +
			"t30,08:40:00,08:40:00,A,1\nt30,08:45:00,08:45:00,C,2\nt30,08:50:00,08:50:00,B,3\n" +
			"t11,09:00:00,09:00:00,A,1\nt11,09:10:00,09:10:00,B,2\n" +
			"t12,07:30:00,07:30:00,B,1\n",
		"calendar.txt": "service_id,monday,tuesday,wednesday,thursday,friday,saturday,sunday,start_date,end_date\n" +
			"WK,1,1,1,1,1,0,0,20260101,20261231\nSUN,0,0,0,0,0,0,1,20260101,20261231\n",
		"calendar_dates.txt": "service_id,date,exception_type\nHOL,20261023,1\n",
		"shapes.txt": "shape_id,shape_pt_lat,shape_pt_lon,shape_pt_sequence\n" +
			"sh1,45.400,-73.400,1\nsh1,45.500,-73.500,2\nsh1,45.510,-73.510,3\n",
		"feed_info.txt": "feed_publisher_name,feed_publisher_url,feed_lang,feed_version\n" +
			"Test Transit,https://example.com,en,2026.10.1\n",
	}
	var buf bytes.Buffer
	if err := writeZip(&buf, files); err != nil {
		t.Fatalf("fixture zip: %v", err)
	}
	return buf.Bytes()
}

func newTestHost(t *testing.T) *WasmHost {
	t.Helper()
	host, err := NewWasmHost(context.Background(), "../web/pkg/cap_sync.wasm")
	if err != nil {
		t.Fatalf("load sync module: %v", err)
	}
	return host
}

// The sync capability's round trip through the Go host: the protocol's
// one step — a draft client against an empty server converges to the
// submitted document, a newer tombstone wins LWW, and validation is
// NOT the op's business (load_edits answers it in every model-side
// module — the browser reads those reports through the bus).
func TestSyncRoundTrip(t *testing.T) {
	ctx := context.Background()
	host := newTestHost(t)
	defer host.Close(ctx)

	if !strings.HasPrefix(host.Hash(), "sha256:") {
		t.Fatalf("host hash not computed: %q", host.Hash())
	}

	record := func(status string, rev, at int64) map[string]any {
		return map[string]any{
			"id": "e1",
			"diversion": map[string]any{
				"id": "d1", "route_id": "10", "direction_id": nil,
				"first_kept_stop": "A", "last_kept_stop": "B",
				"skipped_stops": []string{"C"}, "detour": nil,
				"added_secs": 300, "start_date": date, "end_date": date,
			},
			"status": status, "rev": rev, "updated_at": at,
		}
	}

	var resp struct {
		Records []map[string]any `json:"records"`
	}
	if err := host.Invoke(ctx, map[string]any{
		"op":     "sync_edits",
		"client": []any{record("draft", 1, 1)}, "server": []any{}, "now_ms": 1000,
	}, &resp); err != nil {
		t.Fatalf("sync_edits: %v", err)
	}
	if len(resp.Records) != 1 || resp.Records[0]["status"] != "submitted" {
		t.Fatalf("sync merged and submitted: %+v", resp.Records)
	}
	if resp.Records[0]["rev"].(float64) != 2 || resp.Records[0]["updated_at"].(float64) != 1000 {
		t.Fatalf("submit bumps rev and stamps the host clock: %+v", resp.Records[0])
	}

	// LWW: a newer tombstone beats the submitted record.
	var lww struct {
		Records []map[string]any `json:"records"`
	}
	if err := host.Invoke(ctx, map[string]any{
		"op":     "sync_edits",
		"client": []any{record("accepted", 1, 1)},
		"server": []any{record("retired", 3, 2)}, "now_ms": 2000,
	}, &lww); err != nil {
		t.Fatalf("sync_edits lww: %v", err)
	}
	if len(lww.Records) != 1 || lww.Records[0]["status"] != "retired" {
		t.Fatalf("the tombstone (newer timestamp) won: %+v", lww.Records)
	}

	// Empty documents converge to empty — the protocol's zero case.
	var empty struct {
		Records []map[string]any `json:"records"`
	}
	if err := host.Invoke(ctx, map[string]any{
		"op": "sync_edits", "client": []any{}, "server": []any{}, "now_ms": 0,
	}, &empty); err != nil {
		t.Fatalf("sync_edits empty: %v", err)
	}
	if len(empty.Records) != 0 {
		t.Fatalf("empty documents converge to empty: %+v", empty.Records)
	}

	// The manifest's version field: the host reads the feed's own
	// feed_info.txt row (one boundary CSV field — the module owns
	// the format, the host owns files).
	if v := feedInfoVersion(fixtureZip(t)); v != "2026.10.1" {
		t.Fatalf("feed_info version: %q", v)
	}
}

// TestModuleIsTheDeployedBytes pins the module the tests run to the
// file the server hosts — same bytes, by construction of this test
// reading that exact path.
func TestModuleIsTheDeployedBytes(t *testing.T) {
	host := newTestHost(t)
	sum := sha256File(t, "../web/pkg/cap_sync.wasm")
	if host.Hash() != sum {
		t.Fatalf("host hash %s != file hash %s", host.Hash(), sum)
	}
}

// TestEditsSyncEndpoint exercises POST/GET /edits through real HTTP:
// the client document merges with the server's (LWW in the artifact),
// drafts submit, tombstones propagate, and persistence round-trips.
func TestEditsSyncEndpoint(t *testing.T) {
	host := newTestHost(t)

	s := &server{wasm: host, editsPath: t.TempDir() + "/edits.json"}
	mux := http.NewServeMux()
	mux.HandleFunc("/edits", s.handleEdits)
	base := httptest.NewServer(mux).URL

	editJSON := func(id, status string, rev, at int64) string {
		diversion := `{"id":"d-` + id + `","route_id":"10","direction_id":null,` +
			`"first_kept_stop":"A","last_kept_stop":"B","skipped_stops":["C"],` +
			`"detour":null,"added_secs":300,"start_date":"` + date + `","end_date":"` + date + `"}`
		return `[{"id":"` + id + `","diversion":` + diversion +
			`,"status":"` + status + `","rev":` + strconv.FormatInt(rev, 10) +
			`,"updated_at":` + strconv.FormatInt(at, 10) + `}]`
	}
	post := func(doc string) (int, string) {
		resp, err := http.Post(base+"/edits", "application/json", strings.NewReader(doc))
		if err != nil {
			t.Fatalf("post: %v", err)
		}
		defer resp.Body.Close()
		body, _ := io.ReadAll(resp.Body)
		return resp.StatusCode, string(body)
	}

	// Push a draft; the server must submit it.
	code, body := post(editJSON("e1", "draft", 1, 100))
	if code != 200 {
		t.Fatalf("post: %d %s", code, body)
	}
	if !strings.Contains(body, `"status":"submitted"`) {
		t.Fatalf("draft not submitted: %s", body)
	}

	// A second push with a newer tombstone for the same id: LWW. The
	// timestamp must beat the server's submit stamp (real epoch ms),
	// so date it in the future.
	code, body = post(editJSON("e1", "retired", 2, time.Now().UnixMilli()+60_000))
	if code != 200 {
		t.Fatalf("post 2: %d %s", code, body)
	}
	if !strings.Contains(body, `"status":"retired"`) {
		t.Fatalf("tombstone did not win: %s", body)
	}

	// GET returns the merged document.
	resp, err := http.Get(base + "/edits")
	if err != nil {
		t.Fatalf("get: %v", err)
	}
	got, _ := io.ReadAll(resp.Body)
	resp.Body.Close()
	if !strings.Contains(string(got), `"id":"e1"`) || !strings.Contains(string(got), `"retired"`) {
		t.Fatalf("get: %s", got)
	}

	// Persistence: a fresh server with the same path restores the doc.
	s2 := &server{wasm: host, edits: loadEdits(s.editsPath), editsPath: s.editsPath}
	mux2 := http.NewServeMux()
	mux2.HandleFunc("/edits", s2.handleEdits)
	base2 := httptest.NewServer(mux2).URL
	resp2, err := http.Get(base2 + "/edits")
	if err != nil {
		t.Fatalf("get 2: %v", err)
	}
	body2, _ := io.ReadAll(resp2.Body)
	resp2.Body.Close()
	if !strings.Contains(string(body2), `"retired"`) {
		t.Fatalf("persisted document lost the tombstone: %s", body2)
	}
}
