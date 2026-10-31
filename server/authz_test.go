package main

// Authorization round trips: the IdP authenticates (here: a test JWKS
// over httptest, RS256 and ES256), the server verifies the JWT and
// asks cap-authz over the store's policy pointer — fail-closed, reads
// public, writes role-gated (D-58..D-61), and the verified subject
// lands in the store log as `writer`.

import (
	"context"
	"crypto"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/rsa"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"io"
	"math/big"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"rustwasm/server/store"
)

func b64enc(b []byte) string { return base64.RawURLEncoding.EncodeToString(b) }

func signRS256(t *testing.T, key *rsa.PrivateKey, kid string, claims map[string]any) string {
	t.Helper()
	h, _ := json.Marshal(map[string]any{"alg": "RS256", "typ": "JWT", "kid": kid})
	c, _ := json.Marshal(claims)
	input := b64enc(h) + "." + b64enc(c)
	digest := sha256.Sum256([]byte(input))
	sig, err := rsa.SignPKCS1v15(rand.Reader, key, crypto.SHA256, digest[:])
	if err != nil {
		t.Fatal(err)
	}
	return input + "." + b64enc(sig)
}

func signES256(t *testing.T, key *ecdsa.PrivateKey, kid string, claims map[string]any) string {
	t.Helper()
	h, _ := json.Marshal(map[string]any{"alg": "ES256", "typ": "JWT", "kid": kid})
	c, _ := json.Marshal(claims)
	input := b64enc(h) + "." + b64enc(c)
	digest := sha256.Sum256([]byte(input))
	r, s, err := ecdsa.Sign(rand.Reader, key, digest[:])
	if err != nil {
		t.Fatal(err)
	}
	sig := make([]byte, 64)
	r.FillBytes(sig[:32])
	s.FillBytes(sig[32:])
	return input + "." + b64enc(sig)
}

func rsaJWK(kid string, key *rsa.PublicKey) map[string]any {
	e := big.NewInt(int64(key.E)).Bytes()
	return map[string]any{
		"kid": kid, "kty": "RSA", "alg": "RS256", "use": "sig",
		"n": b64enc(key.N.Bytes()), "e": b64enc(e),
	}
}

func ecJWK(kid string, key *ecdsa.PublicKey) map[string]any {
	x := make([]byte, 32)
	y := make([]byte, 32)
	key.X.FillBytes(x)
	key.Y.FillBytes(y)
	return map[string]any{
		"kid": kid, "kty": "EC", "alg": "ES256", "use": "sig",
		"crv": "P-256", "x": b64enc(x), "y": b64enc(y),
	}
}

// jwksServer serves a mutable JWKS so the test can rotate keys.
type jwksServer struct {
	mu   sync.Mutex
	keys []map[string]any
	srv  *httptest.Server
}

func newJWKSServer(initial []map[string]any) *jwksServer {
	j := &jwksServer{keys: initial}
	j.srv = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		j.mu.Lock()
		defer j.mu.Unlock()
		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(map[string]any{"keys": j.keys})
	}))
	return j
}

func (j *jwksServer) add(k map[string]any) {
	j.mu.Lock()
	defer j.mu.Unlock()
	j.keys = append(j.keys, k)
}

func (j *jwksServer) url() string { return j.srv.URL }

func testPolicy() []byte {
	return []byte(`{"schema_version":1,"rules":[
		{"name":"ops","roles":["dispatcher"],"pointers":["edits"],"actions":["read","write"]},
		{"name":"auditors","roles":["auditor"],"pointers":["*"],"actions":["audit"]},
		{"name":"admin-all","roles":["admin"],"pointers":["*"],"actions":["*"]}
	]}`)
}

func newAuthzTestHost(t *testing.T) *WasmHost {
	t.Helper()
	host, err := NewWasmHost(context.Background(), "../web/pkg/cap_authz.wasm")
	if err != nil {
		t.Fatalf("load authz module: %v", err)
	}
	return host
}

// newAuthServer builds a store-mode server with auth on: JWKS over
// httptest, policy seeded from a file into the store's policy pointer.
func newAuthServer(t *testing.T, jwks *jwksServer) (*server, string) {
	t.Helper()
	dir := t.TempDir()
	s := &server{wasm: newTestHost(t), retentionDays: 90}
	st, err := store.Open(dir)
	if err != nil {
		t.Fatal(err)
	}
	s.store = st
	s.seedStore("")
	policyPath := filepath.Join(dir, "policy.json")
	if err := os.WriteFile(policyPath, testPolicy(), 0o644); err != nil {
		t.Fatal(err)
	}
	s.seedPolicy(policyPath)

	a := &authzState{
		cfg: oidcConfig{
			issuer: "https://idp.example", jwksURL: jwks.url(),
			audience: "transit-server", rolesClaim: "roles",
		},
		wasm:            newAuthzTestHost(t),
		jwksKeys:        map[string]crypto.PublicKey{},
		jwksMinInterval: 0, // tests rotate keys immediately
	}
	s.authz = a
	t.Cleanup(func() { st.Close(); a.wasm.Close(context.Background()) })

	mux := http.NewServeMux()
	mux.HandleFunc("/edits", s.protectWrite(func(*http.Request) string { return "edits" }, s.handleEdits))
	mux.HandleFunc("/pointers/", s.protectWrite(pointerNameOf, s.handlePointers))
	mux.HandleFunc("/blobs/", s.handleBlob)
	base := httptest.NewServer(mux).URL
	return s, base
}

func bearerPost(t *testing.T, url, token, body string) (int, string) {
	t.Helper()
	req, _ := http.NewRequest("POST", url, strings.NewReader(body))
	req.Header.Set("Content-Type", "application/json")
	if token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	b, _ := io.ReadAll(resp.Body)
	return resp.StatusCode, string(b)
}

func TestAuthzPublicReadAuthedWrite(t *testing.T) {
	rsaKey, _ := rsa.GenerateKey(rand.Reader, 2048)
	jwks := newJWKSServer([]map[string]any{rsaJWK("k1", &rsaKey.PublicKey)})
	defer jwks.srv.Close()
	_, base := newAuthServer(t, jwks)

	now := time.Now().Unix()
	claims := func(roles []string, extra map[string]any) map[string]any {
		c := map[string]any{
			"iss": "https://idp.example", "aud": "transit-server",
			"sub": "dispatcher-7", "roles": roles,
			"exp": now + 3600, "nbf": now - 10,
		}
		for k, v := range extra {
			c[k] = v
		}
		return c
	}

	// D-58: GET is public.
	code, body := func() (int, string) {
		resp, err := http.Get(base + "/edits")
		if err != nil {
			t.Fatal(err)
		}
		defer resp.Body.Close()
		b, _ := io.ReadAll(resp.Body)
		return resp.StatusCode, string(b)
	}()
	if code != 200 {
		t.Fatalf("GET /edits must be public: %d %s", code, body)
	}

	// No token → 401.
	if code, _ := bearerPost(t, base+"/edits", "", `[{"id":"e1"}]`); code != 401 {
		t.Fatalf("no token: want 401, got %d", code)
	}
	// alg=none → 401.
	if code, _ := bearerPost(t, base+"/edits", "eyJhbGciOiJub25lIn0.e30.", `[]`); code != 401 {
		t.Fatalf("alg=none must be rejected: %d", code)
	}
	// Expired → 401.
	expired := signRS256(t, rsaKey, "k1", claims([]string{"dispatcher"}, map[string]any{"exp": now - 3600}))
	if code, _ := bearerPost(t, base+"/edits", expired, `[]`); code != 401 {
		t.Fatalf("expired token: want 401, got %d", code)
	}
	// Wrong audience → 401.
	wrongAud := signRS256(t, rsaKey, "k1", claims([]string{"dispatcher"}, map[string]any{"aud": "other"}))
	if code, _ := bearerPost(t, base+"/edits", wrongAud, `[]`); code != 401 {
		t.Fatalf("wrong aud: want 401, got %d", code)
	}
	// Role without a matching rule → 403.
	rider := signRS256(t, rsaKey, "k1", claims([]string{"rider"}, nil))
	if code, _ := bearerPost(t, base+"/edits", rider, editDoc("e1", "draft", 1, 100)); code != 403 {
		t.Fatalf("rider writing edits: want 403, got %d", code)
	}
	// Dispatcher with write on edits → 200, and the log's writer is the sub.
	dispatcher := signRS256(t, rsaKey, "k1", claims([]string{"dispatcher"}, nil))
	code, body = bearerPost(t, base+"/edits", dispatcher, editDoc("e1", "draft", 1, 100))
	if code != 200 || !strings.Contains(body, `"submitted"`) {
		t.Fatalf("dispatcher post: %d %s", code, body)
	}
}

func TestAuthzWriterIsTheVerifiedSubject(t *testing.T) {
	rsaKey, _ := rsa.GenerateKey(rand.Reader, 2048)
	jwks := newJWKSServer([]map[string]any{rsaJWK("k1", &rsaKey.PublicKey)})
	defer jwks.srv.Close()
	s, base := newAuthServer(t, jwks)

	now := time.Now().Unix()
	tok := signRS256(t, rsaKey, "k1", map[string]any{
		"iss": "https://idp.example", "aud": "transit-server",
		"sub": "dispatcher-7", "roles": []string{"dispatcher"},
		"exp": now + 3600,
	})
	if code, _ := bearerPost(t, base+"/edits", tok, editDoc("e1", "draft", 1, 100)); code != 200 {
		t.Fatalf("post: %d", code)
	}
	entries, found, err := s.store.Log("edits", "")
	if err != nil || !found {
		t.Fatalf("log: %v %v", found, err)
	}
	last := entries[len(entries)-1]
	if last.Writer != "dispatcher-7" {
		t.Fatalf("log writer = %q, want the verified sub", last.Writer)
	}
}

func TestAuthzES256AndKidRotation(t *testing.T) {
	ecKey, _ := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	jwks := newJWKSServer([]map[string]any{ecJWK("ec1", &ecKey.PublicKey)})
	defer jwks.srv.Close()
	_, base := newAuthServer(t, jwks)

	now := time.Now().Unix()
	claims := map[string]any{
		"iss": "https://idp.example", "aud": "transit-server",
		"sub": "dispatcher-9", "roles": []string{"dispatcher"},
		"exp": now + 3600,
	}
	// ES256 token → 200.
	tok := signES256(t, ecKey, "ec1", claims)
	if code, _ := bearerPost(t, base+"/edits", tok, editDoc("e9", "draft", 1, 100)); code != 200 {
		t.Fatalf("ES256 post: want 200, got %d", code)
	}
	// Unknown kid → 401; after the JWKS gains the key, the same token works.
	rsaKey, _ := rsa.GenerateKey(rand.Reader, 2048)
	rotated := signRS256(t, rsaKey, "k2", claims)
	if code, _ := bearerPost(t, base+"/edits", rotated, `[]`); code != 401 {
		t.Fatalf("unknown kid: want 401, got %d", code)
	}
	jwks.add(rsaJWK("k2", &rsaKey.PublicKey))
	if code, body := bearerPost(t, base+"/edits", rotated, editDoc("e10", "draft", 1, 100)); code != 200 {
		t.Fatalf("after kid rotation: want 200, got %d %s", code, body)
	}
}

func TestAuthzPolicyChangesAreAuditedCommits(t *testing.T) {
	rsaKey, _ := rsa.GenerateKey(rand.Reader, 2048)
	jwks := newJWKSServer([]map[string]any{rsaJWK("k1", &rsaKey.PublicKey)})
	defer jwks.srv.Close()
	s, base := newAuthServer(t, jwks)

	now := time.Now().Unix()
	admin := signRS256(t, rsaKey, "k1", map[string]any{
		"iss": "https://idp.example", "aud": "transit-server",
		"sub": "admin-1", "roles": []string{"admin"}, "exp": now + 3600,
	})
	dispatcher := signRS256(t, rsaKey, "k1", map[string]any{
		"iss": "https://idp.example", "aud": "transit-server",
		"sub": "dispatcher-7", "roles": []string{"dispatcher"}, "exp": now + 3600,
	})

	// The admin rewrites the policy (write on pointer "policy"), dropping
	// the dispatcher rule — a CAS'd, audited commit.
	newPolicy := `{"schema_version":1,"rules":[{"name":"admin-all","roles":["admin"],"pointers":["*"],"actions":["*"]}]}`
	req, _ := json.Marshal(map[string]any{"from": mustHead(t, s, "policy"), "kind": "policy", "document": newPolicy})
	code, body := bearerPost(t, base+"/pointers/policy", admin, string(req))
	if code != 200 {
		t.Fatalf("policy rewrite: %d %s", code, body)
	}
	// Now the dispatcher is denied (fail-closed against the new policy).
	if code, _ := bearerPost(t, base+"/edits", dispatcher, editDoc("e1", "draft", 1, 100)); code != 403 {
		t.Fatalf("dispatcher after policy change: want 403, got %d", code)
	}
	// And the policy pointer's log has the seed + the rewrite, chained.
	entries, _, err := s.store.Log("policy", "")
	if err != nil || len(entries) != 2 || entries[1].Writer != "admin-1" || entries[1].Kind != "policy" {
		t.Fatalf("policy log: %v %v", entries, err)
	}
	// The manifest pins the new policy hash in store mode.
	if head, found, _ := s.store.Head("policy"); !found || head.Hash != entries[1].To {
		t.Fatalf("policy head: %+v", head)
	}
}

func TestAuthzFailClosedWithoutPolicy(t *testing.T) {
	rsaKey, _ := rsa.GenerateKey(rand.Reader, 2048)
	jwks := newJWKSServer([]map[string]any{rsaJWK("k1", &rsaKey.PublicKey)})
	defer jwks.srv.Close()

	// Auth on, but no policy seeded (no -policy file): writes deny.
	dir := t.TempDir()
	s := &server{wasm: newTestHost(t), retentionDays: 90}
	st, _ := store.Open(dir)
	s.store = st
	s.seedStore("")
	s.authz = &authzState{
		cfg:             oidcConfig{issuer: "https://idp.example", jwksURL: jwks.url(), audience: "transit-server", rolesClaim: "roles"},
		wasm:            newAuthzTestHost(t),
		jwksKeys:        map[string]crypto.PublicKey{},
		jwksMinInterval: 0,
	}
	defer st.Close()
	mux := http.NewServeMux()
	mux.HandleFunc("/edits", s.protectWrite(func(*http.Request) string { return "edits" }, s.handleEdits))
	base := httptest.NewServer(mux).URL

	now := time.Now().Unix()
	tok := signRS256(t, rsaKey, "k1", map[string]any{
		"iss": "https://idp.example", "aud": "transit-server",
		"sub": "admin-1", "roles": []string{"admin"}, "exp": now + 3600,
	})
	if code, body := bearerPost(t, base+"/edits", tok, `[]`); code != 403 || !strings.Contains(body, "no policy") {
		t.Fatalf("no policy must fail closed: %d %s", code, body)
	}
}

// The policy document never leaves the server: the manifest may pin its
// hash (opaque), but the blob itself is 404 even for an authenticated
// admin — the audited path to policy changes is the pointer log.
func TestAuthzPolicyBlobNeverServed(t *testing.T) {
	rsaKey, _ := rsa.GenerateKey(rand.Reader, 2048)
	jwks := newJWKSServer([]map[string]any{rsaJWK("k1", &rsaKey.PublicKey)})
	defer jwks.srv.Close()
	s, base := newAuthServer(t, jwks)

	policyHash := mustHead(t, s, "policy")

	// Anonymous: 404. The blob exists (the store has it) but is withheld.
	resp, err := http.Get(base + "/blobs/" + policyHash)
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	if resp.StatusCode != 404 {
		t.Fatalf("policy blob anonymous: want 404, got %d", resp.StatusCode)
	}

	// Authenticated admin: still 404 (even admins get the policy only
	// through the audited log, never the blob).
	now := time.Now().Unix()
	admin := signRS256(t, rsaKey, "k1", map[string]any{
		"iss": "https://idp.example", "aud": "transit-server",
		"sub": "admin-1", "roles": []string{"admin"}, "exp": now + 3600,
	})
	req, _ := http.NewRequest("GET", base+"/blobs/"+policyHash, nil)
	req.Header.Set("Authorization", "Bearer "+admin)
	resp2, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	resp2.Body.Close()
	if resp2.StatusCode != 404 {
		t.Fatalf("policy blob as admin: want 404, got %d", resp2.StatusCode)
	}

	// A normal blob (the edits document) still serves fine, publicly.
	editsHash := mustHead(t, s, "edits")
	resp3, err := http.Get(base + "/blobs/" + editsHash)
	if err != nil {
		t.Fatal(err)
	}
	body, _ := io.ReadAll(resp3.Body)
	resp3.Body.Close()
	if resp3.StatusCode != 200 || !strings.Contains(string(body), "[") {
		t.Fatalf("edits blob must stay public: %d %s", resp3.StatusCode, body)
	}
}

// The pointer log carries writer identities, so it is authenticated:
// no token → 401; a read rule without an audit rule → 200 with writers
// redacted; an audit rule → full writers.
func TestAuthzLogWriterRedaction(t *testing.T) {
	rsaKey, _ := rsa.GenerateKey(rand.Reader, 2048)
	jwks := newJWKSServer([]map[string]any{rsaJWK("k1", &rsaKey.PublicKey)})
	defer jwks.srv.Close()
	s, base := newAuthServer(t, jwks)

	now := time.Now().Unix()
	mk := func(sub string, roles []string) string {
		return signRS256(t, rsaKey, "k1", map[string]any{
			"iss": "https://idp.example", "aud": "transit-server",
			"sub": sub, "roles": roles, "exp": now + 3600,
		})
	}
	bearerGet := func(url, token string) (int, string) {
		req, _ := http.NewRequest("GET", url, nil)
		if token != "" {
			req.Header.Set("Authorization", "Bearer "+token)
		}
		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		defer resp.Body.Close()
		b, _ := io.ReadAll(resp.Body)
		return resp.StatusCode, string(b)
	}

	// One write as the dispatcher, so the log has a writer to leak.
	if code, _ := bearerPost(t, base+"/edits", mk("dispatcher-7", []string{"dispatcher"}), editDoc("e1", "draft", 1, 100)); code != 200 {
		t.Fatalf("post: %d", code)
	}
	logURL := base + "/pointers/edits/log"

	// No token → 401 (the log is not public when auth is on).
	if code, _ := bearerGet(logURL, ""); code != 401 {
		t.Fatalf("log anonymous: want 401, got %d", code)
	}
	// Read on edits but no audit → 200, writers redacted.
	code, body := bearerGet(logURL, mk("dispatcher-7", []string{"dispatcher"}))
	if code != 200 || strings.Contains(body, "dispatcher-7") {
		t.Fatalf("log for reader without audit: want 200 redacted, got %d %s", code, body)
	}
	if !strings.Contains(body, `"writer":""`) {
		t.Fatalf("writer must be redacted, got %s", body)
	}
	// Audit rule → full writers.
	if _, body := bearerGet(logURL, mk("aud-1", []string{"auditor"})); !strings.Contains(body, "dispatcher-7") {
		t.Fatalf("auditor must see writers, got %s", body)
	}
	// Admin wildcard covers audit too.
	if _, body := bearerGet(logURL, mk("admin-1", []string{"admin"})); !strings.Contains(body, "dispatcher-7") {
		t.Fatalf("admin must see writers, got %s", body)
	}
	// The head endpoint stays public and carries no identity.
	if code, _ := bearerGet(base+"/pointers/edits", ""); code != 200 {
		t.Fatalf("head must stay public: %d", code)
	}
	_ = s
}

// GET /permissions serves the policy AS IT APPLIES TO THE CALLER: the
// projection (own roles + merged capabilities), never the policy
// document — no other roles, no rule list. The policy blob stays 404.
func TestAuthzPermissionsProjection(t *testing.T) {
	rsaKey, _ := rsa.GenerateKey(rand.Reader, 2048)
	jwks := newJWKSServer([]map[string]any{rsaJWK("k1", &rsaKey.PublicKey)})
	defer jwks.srv.Close()
	s, _ := newAuthServer(t, jwks)

	mux := http.NewServeMux()
	mux.HandleFunc("/permissions", s.requireAuth(s.handlePermissions))
	mux.HandleFunc("/blobs/", s.handleBlob)
	permBase := httptest.NewServer(mux).URL

	now := time.Now().Unix()
	mk := func(sub string, roles []string) string {
		return signRS256(t, rsaKey, "k1", map[string]any{
			"iss": "https://idp.example", "aud": "transit-server",
			"sub": sub, "roles": roles, "exp": now + 3600,
		})
	}
	get := func(token string) (int, string) {
		req, _ := http.NewRequest("GET", permBase+"/permissions", nil)
		if token != "" {
			req.Header.Set("Authorization", "Bearer "+token)
		}
		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		defer resp.Body.Close()
		b, _ := io.ReadAll(resp.Body)
		return resp.StatusCode, string(b)
	}

	// No token → 401.
	if code, _ := get(""); code != 401 {
		t.Fatalf("permissions anonymous: want 401, got %d", code)
	}
	// Dispatcher: sees its own capabilities on edits — and NOT the other
	// roles or the rule structure.
	code, body := get(mk("dispatcher-7", []string{"dispatcher"}))
	if code != 200 {
		t.Fatalf("permissions: %d %s", code, body)
	}
	if !strings.Contains(body, `"edits"`) || !strings.Contains(body, `"write"`) || !strings.Contains(body, `"dispatcher"`) {
		t.Fatalf("own capabilities missing: %s", body)
	}
	for _, leak := range []string{"admin", "auditor", "auditors", "admin-all", "rules"} {
		if strings.Contains(body, leak) {
			t.Fatalf("policy internals leaked to the client (%q): %s", leak, body)
		}
	}
	// Auditor: sees its wildcard audit capability, nothing else.
	_, body = get(mk("aud-1", []string{"auditor"}))
	if !strings.Contains(body, `"audit"`) || strings.Contains(body, "edits") {
		t.Fatalf("auditor projection: %s", body)
	}
	// The policy blob itself is still withheld even for the admin.
	policyHash := mustHead(t, s, "policy")
	req, _ := http.NewRequest("GET", permBase+"/blobs/"+policyHash, nil)
	req.Header.Set("Authorization", "Bearer "+mk("admin-1", []string{"admin"}))
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	if resp.StatusCode != 404 {
		t.Fatalf("policy blob must stay 404 for admin too: %d", resp.StatusCode)
	}
}

func mustHead(t *testing.T, s *server, pointer string) string {
	t.Helper()
	head, found, err := s.store.Head(pointer)
	if err != nil || !found {
		t.Fatalf("head %s: %v %v", pointer, found, err)
	}
	return head.Hash
}
