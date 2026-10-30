// Package main is the sync server: the Go host for the capability
// modules.
//
// The division of labor is deliberate (see the repo README): the
// modules own all domain logic — the server loads the exact bytes
// the browser runs (sha256 in the manifest proves it) and calls the
// flat ABI. This host is scaffolding only: HTTP routing, static
// serving with Range support, the CORS proxy, the geocoder proxy
// (geocode.go), the published network
// (network.go) and streets (streets.go), and the manifest that pins
// their hashes and the sync module's. No
// domain decision is ever made here.
package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"log"
	"mime"
	"net/http"
	"net/url"
	"os"
	"path"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"rustwasm/server/store"
)

func main() {
	addr := flag.String("addr", "127.0.0.1:8080", "listen address")
	dist := flag.String("dist", "../web/dist", "directory of static files to serve")
	wasmPath := flag.String("wasm", "../web/pkg/cap_sync.wasm", "the sync capability module to host and pin")
	feedPath := flag.String("feed", "", "path to the GTFS zip: imported once and published as the network (enables /network and the manifest's schedule section)")
	networkWasm := flag.String("network-wasm", "../web/pkg/cap_network.wasm", "the network capability module that imports the feed")
	streetsPath := flag.String("streets", "", "path to a street layer (GeoJSON lines): imported once and published as the streets document (enables /streets, and with -feed the walk table at /walks)")
	streetsWasm := flag.String("streets-wasm", "../web/pkg/cap_streets.wasm", "the streets capability module that imports the layer and derives the walk table")
	publisher := flag.String("publisher", "", "manifest provenance (who publishes this schedule)")
	editsPath := flag.String("edits", "", "path to persist the edit document (JSON); empty = in-memory only (legacy mode; with -store it seeds the edits pointer once)")
	storeDir := flag.String("store", "", "directory of the durable sync store (blobs + pointers, docs/FORMATS.md): enables /pointers, /blobs and store-backed /edits and /manifest; empty = legacy mode")
	retentionDays := flag.Int("retention-days", 90, "tombstone retention horizon (days) for the store's D-57 compaction commits")
	geocoderURL := flag.String("geocoder", "", "base URL of a Pelias geocoder (e.g. http://127.0.0.1:4000): enables /geocode/{autocomplete,search,reverse} and lists it in the manifest's services")
	geocoderPolicy := flag.String("geocoder-policy", "geocoder.json", "the deployment's geocoder query policy (Pelias parameters per op), applied by the proxy")
	flag.Parse()

	mime.AddExtensionType(".wasm", "application/wasm")
	mime.AddExtensionType(".mjs", "text/javascript")
	mime.AddExtensionType(".pmtiles", "application/vnd.pmtiles")

	ctx := context.Background()
	host, err := NewWasmHost(ctx, *wasmPath)
	if err != nil {
		log.Fatalf("wasm host: %v", err)
	}
	log.Printf("sync module loaded: %s (%s)", *wasmPath, host.Hash())

	s := &server{
		dist:          *dist,
		wasm:          host,
		publisher:     *publisher,
		editsPath:     *editsPath,
		retentionDays: *retentionDays,
	}
	if *storeDir != "" {
		st, err := store.Open(*storeDir)
		if err != nil {
			log.Fatalf("store: %v", err)
		}
		s.store = st
		s.seedStore(*editsPath)
		s.maybeCompactEdits()
		log.Printf("sync store open: %s", *storeDir)
	}
	if *geocoderURL != "" {
		if s.geocoder, err = loadGeocoder(*geocoderURL, *geocoderPolicy); err != nil {
			log.Fatalf("geocoder: %v", err)
		}
	}
	if *feedPath != "" {
		s.feeder = &fileFeed{path: *feedPath, networkWasm: *networkWasm}
		if _, err := s.feeder.Load(); err != nil {
			log.Fatalf("feed: %v", err)
		}
	}
	if *streetsPath != "" {
		s.streets = &fileStreets{path: *streetsPath, streetsWasm: *streetsWasm}
		if _, err := s.publishedStreets(); err != nil {
			log.Fatalf("streets: %v", err)
		}
	}
	s.edits = loadEdits(*editsPath)

	mux := http.NewServeMux()
	mux.HandleFunc("/rt", s.handleProxy)
	mux.HandleFunc("/manifest", s.handleManifest)
	mux.HandleFunc("/network", s.handleNetwork)
	mux.HandleFunc("/streets", s.handleStreets)
	mux.HandleFunc("/walks", s.handleWalks)
	mux.HandleFunc("/edits", s.handleEdits)
	mux.HandleFunc("/pointers/", s.handlePointers)
	mux.HandleFunc("/blobs/", s.handleBlob)
	mux.HandleFunc("/geocode/", s.handleGeocode)
	mux.HandleFunc("/", s.handleStatic)

	log.Printf("listening on http://%s (dist: %s)", *addr, *dist)
	log.Fatal(http.ListenAndServe(*addr, mux))
}

// server holds the host's few pieces of state.
type server struct {
	dist      string
	wasm      *WasmHost
	feeder    *fileFeed    // nil when no feed is configured
	streets   *fileStreets // nil when no street layer is configured
	publisher string
	geocoder  *geocoder // nil when no geocoder is configured

	// The edit document. Opaque JSON: the artifact owns the schema
	// and every domain decision about it (merge, submit, validation)
	// — this host only stores and ferries it.
	editsMu   sync.Mutex
	edits     json.RawMessage // []EditRecord
	editsPath string          // empty = in-memory only (legacy mode)

	// The durable sync store (store mode, -store): blobs + CAS'd
	// pointers. nil = legacy mode.
	store         *store.Store
	retentionDays int

	// The manifest pointer's last committed key (store mode): the
	// manifest is re-committed when the published set changes.
	manifestMu  sync.Mutex
	manifestKey string
}

// handleStatic serves the built frontend. http.ServeContent gives
// Range requests (PMTiles needs them), If-Modified-Since, and content
// types by extension — no hand-rolled 206s.
func (s *server) handleStatic(w http.ResponseWriter, r *http.Request) {
	upath := r.URL.Path
	if strings.Contains(upath, "..") {
		http.Error(w, "bad path", http.StatusBadRequest)
		return
	}
	name := path.Clean(strings.TrimPrefix(upath, "/"))
	if name == "" || name == "." {
		name = "index.html"
	}
	file, err := os.Open(filepath.Join(s.dist, name))
	if err != nil {
		http.NotFound(w, r)
		return
	}
	defer file.Close()
	info, err := file.Stat()
	if err != nil || info.IsDir() {
		http.NotFound(w, r)
		return
	}
	// ServeContent sniffs the content type by extension.
	http.ServeContent(w, r, info.Name(), info.ModTime(), file)
}

// handleProxy is the CORS proxy for agency feeds the browser can't
// fetch directly (no Access-Control-Allow-Origin upstream). The
// server is expected to bind localhost in dev; production puts it
// behind the agency's own reverse proxy.
func (s *server) handleProxy(w http.ResponseWriter, r *http.Request) {
	raw := r.URL.Query().Get("url")
	if raw == "" {
		http.Error(w, "missing url parameter", http.StatusBadRequest)
		return
	}
	target, err := url.Parse(raw)
	if err != nil || (target.Scheme != "http" && target.Scheme != "https") {
		http.Error(w, "url must be http(s)", http.StatusBadRequest)
		return
	}
	resp, err := http.Get(raw) //nolint:noctx // dev proxy; upstream timeout via client defaults
	if err != nil {
		http.Error(w, fmt.Sprintf("upstream fetch failed: %v", err), http.StatusBadGateway)
		return
	}
	defer resp.Body.Close()
	if ct := resp.Header.Get("Content-Type"); ct != "" {
		w.Header().Set("Content-Type", ct)
	}
	w.Header().Set("Access-Control-Allow-Origin", "*")
	w.WriteHeader(resp.StatusCode)
	io.Copy(w, resp.Body)
}

// Manifest pins the published network, the GTFS zip it was imported
// from, the published streets and walk table, and the sync module
// this server runs, and lists the external services it proxies. `feed_version` is read
// from the feed's own feed_info.txt by the host (the host owns files
// and I/O; the module owns the format's domain).
type Manifest struct {
	NetworkHash string  `json:"network_hash"` // the network GET /network serves
	FeedHash    string  `json:"feed_hash"`    // "sha256:<hex>" of the GTFS zip it was imported from
	FeedVersion *string `json:"feed_version,omitempty"`
	StreetsHash string  `json:"streets_hash,omitempty"` // the streets document GET /streets serves
	WalksHash   string  `json:"walks_hash,omitempty"`   // the walk table GET /walks serves
	WasmHash    string  `json:"wasm_hash"`              // sha256 of the sync module this server runs
	GeneratedAt int64   `json:"generated_at"`
	Publisher   *string `json:"publisher,omitempty"`
	// The external services this deployment offers, by name — a
	// service it does not list is off, and the frames that need it
	// say so (docs/ARCHITECTURE.md, web section, "External services").
	Services map[string]Service `json:"services,omitempty"`
}

// Service is where a page asks an external service on this origin,
// and the query policy the path applies.
type Service struct {
	Path  string                    `json:"path"`
	Query map[string]map[string]any `json:"query,omitempty"`
}

// buildManifest assembles the manifest from the published set. The
// generated_at stamp is the publish time, not the request time, so a
// store-mode manifest blob is immutable between republishes.
func (s *server) buildManifest() (Manifest, error) {
	manifest := Manifest{
		WasmHash:    s.wasm.Hash(),
		GeneratedAt: time.Now().Unix(),
	}
	if s.feeder != nil {
		feed, err := s.feeder.Load()
		if err != nil {
			return manifest, fmt.Errorf("feed unavailable: %w", err)
		}
		manifest.NetworkHash = feed.networkHash
		manifest.FeedHash = feed.hash
		if v := feed.version; v != "" {
			manifest.FeedVersion = &v
		}
	}
	if s.streets != nil {
		streets, err := s.publishedStreets()
		if err != nil {
			return manifest, fmt.Errorf("streets unavailable: %w", err)
		}
		manifest.StreetsHash = streets.streets.hash
		manifest.WalksHash = streets.walks.hash
	}
	if s.publisher != "" {
		manifest.Publisher = &s.publisher
	}
	if s.geocoder != nil {
		manifest.Services = map[string]Service{"geocoder": s.geocoder.service()}
	}
	return manifest, nil
}

// manifestKeyOf identifies the published set a manifest pins (everything
// but the generated_at stamp): a change re-commits the manifest pointer.
func manifestKeyOf(m Manifest) string {
	services, _ := json.Marshal(m.Services)
	sum := sha256.Sum256([]byte(strings.Join([]string{
		m.WasmHash, m.NetworkHash, m.FeedHash, orEmpty(m.FeedVersion),
		m.StreetsHash, m.WalksHash, orEmpty(m.Publisher), string(services),
	}, "|")))
	return hex.EncodeToString(sum[:])
}

func orEmpty(s *string) string {
	if s == nil {
		return ""
	}
	return *s
}

func (s *server) handleManifest(w http.ResponseWriter, r *http.Request) {
	manifest, err := s.buildManifest()
	if err != nil {
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	if s.store == nil {
		// Legacy mode: the manifest is ephemeral, served as built.
		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(manifest)
		return
	}
	// Store mode: the manifest is the ROOT pointer — publishing is one
	// CAS on it, and clients resolve the published set through it
	// (docs/units/sync-persistence.md). GET serves the pointer's blob.
	s.manifestMu.Lock()
	key := manifestKeyOf(manifest)
	if key != s.manifestKey {
		body, _ := json.Marshal(manifest)
		env := store.Envelope{Kind: "manifest", SchemaVersion: 1, ProducedBy: "server"}
		for attempt := 0; attempt < 5; attempt++ {
			head, found, err := s.store.Head("manifest")
			if err != nil {
				s.manifestMu.Unlock()
				http.Error(w, err.Error(), http.StatusInternalServerError)
				return
			}
			from := ""
			if found {
				from = head.Hash
			}
			_, _, err = s.store.CAS("manifest", from, body, env, "", "server", time.Now().UnixMilli())
			if err != nil {
				var ce *store.ConflictError
				if errors.As(err, &ce) && attempt < 4 {
					continue
				}
				s.manifestMu.Unlock()
				http.Error(w, err.Error(), http.StatusInternalServerError)
				return
			}
			s.manifestKey = key
			break
		}
	}
	s.manifestMu.Unlock()

	head, found, err := s.store.Head("manifest")
	if err != nil || !found {
		http.Error(w, "manifest pointer unavailable", http.StatusInternalServerError)
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
}

// seedStore creates the edits pointer when the store is fresh: from the
// legacy -edits file (migrated at receive through the module — canonical
// bytes and hash come from the module, never minted here), else empty.
func (s *server) seedStore(editsPath string) {
	nowMs := time.Now().UnixMilli()
	if _, found, err := s.store.Head("edits"); err == nil && !found {
		raw := []byte("[]")
		if editsPath != "" {
			if body, err := os.ReadFile(editsPath); err == nil && len(body) > 0 {
				raw = body
			}
		}
		_, document, hash, dropped, err := s.readEdits(raw)
		if err != nil {
			log.Printf("seed edits: read_edits: %v", err)
			return
		}
		if _, _, err := s.store.CAS("edits", "", []byte(document), s.syncEnvelope(), hash, "migrate:-edits", nowMs); err != nil {
			log.Printf("seed edits: %v", err)
			return
		}
		if dropped > 0 {
			log.Printf("seed edits: migrated, %d malformed record(s) dropped", dropped)
		}
	}
}
