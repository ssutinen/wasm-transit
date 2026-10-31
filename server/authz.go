package main

// Authentication + authorization (docs/units/cap-authz.md,
// D-58..D-61).
//
// Authentication is the IdP's: this file only VERIFIES the bearer JWT —
// signature against the IdP's JWKS (cached by kid, refetched on an
// unknown kid) or a pinned PEM key (D-60), RS256/ES256 only (alg=none
// and HS* are rejected — the IdP signs with a private key), iss/aud
// exact, exp/nbf with 60 s leeway — and extracts the identity: `sub`
// plus the roles at the configured claim path (D-59). Authorization is
// a domain decision, so it runs in the cap-authz MODULE over the
// policy document (the store's `policy` pointer, seeded from a -policy
// file at first boot — D-61): fail-closed, every write denied when
// there is no valid token or no matching rule (D-58: reads stay
// public, writes are authenticated).
//
// The host stays scaffolding: it owns the token, the clock and the
// HTTP; the module owns the verdict. A verified subject becomes the
// store log's `writer` — the audit trail ties commits to identities.

import (
	"context"
	"crypto"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rsa"
	"crypto/sha256"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"errors"
	"fmt"
	"io"
	"log"
	"math/big"
	"net/http"
	"os"
	"strings"
	"sync"
	"time"

	"rustwasm/server/store"
)

// oidcConfig is the deployment's IdP wiring (D-60).
type oidcConfig struct {
	issuer     string
	jwksURL    string
	audience   string
	keyFile    string // pinned PEM public key (used when jwksURL is empty, or as bootstrap)
	rolesClaim string // dotted claim path, default "roles" (D-59)
}

// authzState is the auth-enabled server state: nil means auth is OFF
// (dev default — logged loudly at startup, writes unauthenticated).
type authzState struct {
	cfg    oidcConfig
	wasm   *WasmHost // cap-authz
	pinned crypto.PublicKey

	jwksMu          sync.Mutex
	jwksKeys        map[string]crypto.PublicKey // by kid
	jwksFetched     time.Time
	jwksMinInterval time.Duration // refetch rate limit; loadAuthz sets 30s, tests 0

	// policyBytes is the policy document used when there is no store
	// (legacy mode); with -store it is read from the `policy` pointer
	// per request (changes are audited CAS commits).
	policyMu    sync.RWMutex
	policyBytes []byte
}

func loadAuthz(ctx context.Context, cfg oidcConfig, authzWasmPath string, policyFile string) (*authzState, error) {
	host, err := NewWasmHost(ctx, authzWasmPath)
	if err != nil {
		return nil, fmt.Errorf("authz module: %w", err)
	}
	a := &authzState{cfg: cfg, wasm: host, jwksKeys: map[string]crypto.PublicKey{}, jwksMinInterval: 30 * time.Second}
	if a.cfg.rolesClaim == "" {
		a.cfg.rolesClaim = "roles"
	}
	if cfg.keyFile != "" {
		pemBytes, err := os.ReadFile(cfg.keyFile)
		if err != nil {
			return nil, fmt.Errorf("oidc key: %w", err)
		}
		key, err := parsePublicPEM(pemBytes)
		if err != nil {
			return nil, fmt.Errorf("oidc key: %w", err)
		}
		a.pinned = key
	}
	if policyFile != "" {
		b, err := os.ReadFile(policyFile)
		if err != nil {
			return nil, fmt.Errorf("policy file: %w", err)
		}
		a.policyBytes = b
	}
	return a, nil
}

func parsePublicPEM(b []byte) (crypto.PublicKey, error) {
	block, _ := pem.Decode(b)
	if block == nil {
		return nil, errors.New("no PEM block")
	}
	if key, err := x509.ParsePKCS1PublicKey(block.Bytes); err == nil {
		return key, nil
	}
	key, err := x509.ParsePKIXPublicKey(block.Bytes)
	if err != nil {
		return nil, err
	}
	switch k := key.(type) {
	case *rsa.PublicKey, *ecdsa.PublicKey:
		return k, nil
	}
	return nil, fmt.Errorf("unsupported key type %T", key)
}

func b64url(s string) ([]byte, error) {
	return base64.RawURLEncoding.DecodeString(s)
}

// jwk is one JWKS key (RSA or EC P-256).
type jwk struct {
	Kid string `json:"kid"`
	Kty string `json:"kty"`
	Alg string `json:"alg"`
	Use string `json:"use"`
	N   string `json:"n"`   // RSA
	E   string `json:"e"`   // RSA
	Crv string `json:"crv"` // EC
	X   string `json:"x"`   // EC
	Y   string `json:"y"`   // EC
}

func (k jwk) publicKey() (crypto.PublicKey, error) {
	switch k.Kty {
	case "RSA":
		nBytes, err := b64url(k.N)
		if err != nil {
			return nil, err
		}
		eBytes, err := b64url(k.E)
		if err != nil {
			return nil, err
		}
		e := 0
		for _, b := range eBytes {
			e = e<<8 | int(b)
		}
		return &rsa.PublicKey{N: new(big.Int).SetBytes(nBytes), E: e}, nil
	case "EC":
		if k.Crv != "P-256" {
			return nil, fmt.Errorf("unsupported crv %q", k.Crv)
		}
		xBytes, err := b64url(k.X)
		if err != nil {
			return nil, err
		}
		yBytes, err := b64url(k.Y)
		if err != nil {
			return nil, err
		}
		return &ecdsa.PublicKey{
			Curve: elliptic.P256(),
			X:     new(big.Int).SetBytes(xBytes),
			Y:     new(big.Int).SetBytes(yBytes),
		}, nil
	}
	return nil, fmt.Errorf("unsupported kty %q", k.Kty)
}

var jwksHTTP = &http.Client{Timeout: 10 * time.Second}

// keyFor returns the verification key for kid, fetching the JWKS on a
// miss (rate-limited: at most one refetch per 30 s).
func (a *authzState) keyFor(ctx context.Context, kid string) (crypto.PublicKey, error) {
	if a.pinned != nil {
		return a.pinned, nil
	}
	a.jwksMu.Lock()
	key, ok := a.jwksKeys[kid]
	canFetch := time.Since(a.jwksFetched) > a.jwksMinInterval
	a.jwksMu.Unlock()
	if ok {
		return key, nil
	}
	if a.cfg.jwksURL == "" {
		return nil, errors.New("no JWKS URL and no pinned key configured")
	}
	if !canFetch {
		return nil, fmt.Errorf("unknown kid %q (JWKS refetch rate-limited)", kid)
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, a.cfg.jwksURL, nil)
	if err != nil {
		return nil, err
	}
	resp, err := jwksHTTP.Do(req)
	if err != nil {
		return nil, fmt.Errorf("jwks fetch: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != 200 {
		return nil, fmt.Errorf("jwks fetch: %s", resp.Status)
	}
	body, err := io.ReadAll(io.LimitReader(resp.Body, 4<<20))
	if err != nil {
		return nil, err
	}
	var set struct {
		Keys []jwk `json:"keys"`
	}
	if err := json.Unmarshal(body, &set); err != nil {
		return nil, fmt.Errorf("jwks parse: %w", err)
	}
	a.jwksMu.Lock()
	a.jwksFetched = time.Now()
	for _, k := range set.Keys {
		if k.Use != "" && k.Use != "sig" {
			continue
		}
		if key, err := k.publicKey(); err == nil {
			a.jwksKeys[k.Kid] = key
		}
	}
	key, ok = a.jwksKeys[kid]
	a.jwksMu.Unlock()
	if !ok {
		return nil, fmt.Errorf("kid %q not in the JWKS", kid)
	}
	return key, nil
}

// identity is the verified subject + roles (D-59).
type identity struct {
	Sub   string   `json:"sub"`
	Roles []string `json:"roles"`
}

const clockLeeway = 60 * time.Second

// verifyJWT authenticates one bearer token (D-60): signature, alg
// allowlist, iss/aud, exp/nbf, then the roles claim path.
func (a *authzState) verifyJWT(ctx context.Context, token string) (*identity, error) {
	parts := strings.Split(token, ".")
	if len(parts) != 3 {
		return nil, errors.New("not a JWT")
	}
	var header struct {
		Alg string `json:"alg"`
		Kid string `json:"kid"`
	}
	hBytes, err := b64url(parts[0])
	if err != nil || json.Unmarshal(hBytes, &header) != nil {
		return nil, errors.New("bad header")
	}
	if header.Alg != "RS256" && header.Alg != "ES256" {
		return nil, fmt.Errorf("alg %q rejected (RS256/ES256 only)", header.Alg)
	}
	key, err := a.keyFor(ctx, header.Kid)
	if err != nil {
		return nil, err
	}
	sig, err := b64url(parts[2])
	if err != nil {
		return nil, errors.New("bad signature encoding")
	}
	signingInput := parts[0] + "." + parts[1]
	digest := sha256.Sum256([]byte(signingInput))
	switch k := key.(type) {
	case *rsa.PublicKey:
		if header.Alg != "RS256" {
			return nil, fmt.Errorf("alg %q with an RSA key", header.Alg)
		}
		if err := rsa.VerifyPKCS1v15(k, crypto.SHA256, digest[:], sig); err != nil {
			return nil, errors.New("bad signature")
		}
	case *ecdsa.PublicKey:
		if header.Alg != "ES256" {
			return nil, fmt.Errorf("alg %q with an EC key", header.Alg)
		}
		if len(sig) != 64 {
			return nil, errors.New("bad ES256 signature length")
		}
		r := new(big.Int).SetBytes(sig[:32])
		s := new(big.Int).SetBytes(sig[32:])
		if !ecdsa.Verify(k, digest[:], r, s) {
			return nil, errors.New("bad signature")
		}
	default:
		return nil, fmt.Errorf("unsupported key type %T", key)
	}

	pBytes, err := b64url(parts[1])
	if err != nil {
		return nil, errors.New("bad payload encoding")
	}
	var claims map[string]any
	if err := json.Unmarshal(pBytes, &claims); err != nil {
		return nil, errors.New("bad claims")
	}
	now := time.Now()
	if a.cfg.issuer != "" {
		if claims["iss"] != a.cfg.issuer {
			return nil, fmt.Errorf("iss %v != %q", claims["iss"], a.cfg.issuer)
		}
	}
	if a.cfg.audience != "" {
		audOK := claims["aud"] == a.cfg.audience
		if !audOK {
			if arr, ok := claims["aud"].([]any); ok {
				for _, v := range arr {
					if v == a.cfg.audience {
						audOK = true
					}
				}
			}
		}
		if !audOK {
			return nil, fmt.Errorf("aud %v does not contain %q", claims["aud"], a.cfg.audience)
		}
	}
	if exp, ok := claims["exp"].(float64); ok {
		if now.After(time.Unix(int64(exp), 0).Add(clockLeeway)) {
			return nil, errors.New("token expired")
		}
	}
	if nbf, ok := claims["nbf"].(float64); ok {
		if now.Add(clockLeeway).Before(time.Unix(int64(nbf), 0)) {
			return nil, errors.New("token not yet valid")
		}
	}
	sub, _ := claims["sub"].(string)
	if sub == "" {
		return nil, errors.New("no sub")
	}
	return &identity{Sub: sub, Roles: rolesAtClaim(claims, a.cfg.rolesClaim)}, nil
}

// rolesAtClaim reads a dotted claim path; the leaf must be a string
// array (missing or wrong-typed → no roles, and no roles means no rule
// matches — fail-closed).
func rolesAtClaim(claims map[string]any, path string) []string {
	cur := any(claims)
	for _, seg := range strings.Split(path, ".") {
		m, ok := cur.(map[string]any)
		if !ok {
			return nil
		}
		cur, ok = m[seg]
		if !ok {
			return nil
		}
	}
	arr, ok := cur.([]any)
	if !ok {
		return nil
	}
	roles := make([]string, 0, len(arr))
	for _, v := range arr {
		if s, ok := v.(string); ok {
			roles = append(roles, s)
		}
	}
	return roles
}

type ctxKey string

const (
	writerKey   ctxKey = "writer"
	identityKey ctxKey = "identity" // the verified *identity (sub + roles)
	auditKey    ctxKey = "audit"    // may the caller see writer identities in the log?
)

// policyDocument returns the current policy document bytes: the
// store's `policy` pointer in store mode (audited), else the -policy
// file's bytes (legacy mode). Absent → nil (fail-closed at the op).
func (s *server) policyDocument() []byte {
	if s.store != nil {
		head, found, err := s.store.Head("policy")
		if err != nil || !found {
			return nil
		}
		b, _, err := s.store.GetBlob(head.Hash)
		if err != nil {
			return nil
		}
		return b
	}
	if s.authz == nil {
		return nil
	}
	s.authz.policyMu.RLock()
	defer s.authz.policyMu.RUnlock()
	return s.authz.policyBytes
}

// authorize asks cap-authz the one question. Fail-closed: any error —
// no policy, module error, malformed answer — is a deny.
func (s *server) authorize(r *http.Request, id *identity, action, pointer string) (bool, string) {
	policy := s.policyDocument()
	if len(policy) == 0 {
		return false, "no policy document (fail-closed)"
	}
	var resp struct {
		Decision struct {
			Allow       bool    `json:"allow"`
			MatchedRule *string `json:"matched_rule"`
			Reason      string  `json:"reason"`
		} `json:"decision"`
	}
	err := s.authz.wasm.Invoke(r.Context(), map[string]any{
		"op":       "authorize",
		"identity": map[string]any{"sub": id.Sub, "roles": id.Roles},
		"action":   action,
		"pointer":  pointer,
		"policy":   json.RawMessage(policy),
	}, &resp)
	if err != nil {
		return false, "authorize op failed (fail-closed): " + err.Error()
	}
	if !resp.Decision.Allow {
		return false, resp.Decision.Reason
	}
	return true, resp.Decision.Reason
}

// protectWrite wraps a write endpoint (D-58: reads stay public —
// EXCEPT the pointer log, which carries writer identities and is
// authenticated: a read on the pointer, with writers redacted unless
// the identity also holds an "audit" rule). pointerOf names the
// pointer ("edits" for /edits, the path segment for /pointers/{name}).
// With auth disabled (no -oidc flags) everything passes through — dev
// mode, logged loudly at startup.
func (s *server) protectWrite(pointerOf func(*http.Request) string, next http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		isLog := r.Method == http.MethodGet && strings.HasSuffix(r.URL.Path, "/log")
		if s.authz == nil || ((r.Method == http.MethodGet || r.Method == http.MethodHead) && !isLog) {
			next(w, r)
			return
		}
		auth := r.Header.Get("Authorization")
		if !strings.HasPrefix(auth, "Bearer ") {
			http.Error(w, "missing bearer token", http.StatusUnauthorized)
			return
		}
		id, err := s.authz.verifyJWT(r.Context(), strings.TrimPrefix(auth, "Bearer "))
		if err != nil {
			http.Error(w, "invalid token: "+err.Error(), http.StatusUnauthorized)
			return
		}
		pointer := pointerOf(r)
		ctx := context.WithValue(r.Context(), identityKey, id)
		ctx = context.WithValue(ctx, writerKey, id.Sub)
		if isLog {
			// The log endpoint needs read on the pointer — and an "audit"
			// rule implies it. The log's `writer` fields are user
			// identities: visible only to an audit rule, redacted
			// otherwise, so no client ever sees another user's sub.
			allowRead, reason := s.authorize(r, id, "read", pointer)
			canAudit, _ := s.authorize(r, id, "audit", pointer)
			if !allowRead && !canAudit {
				log.Printf("authz deny: sub=%s read %s: %s", id.Sub, pointer, reason)
				http.Error(w, "forbidden: "+reason, http.StatusForbidden)
				return
			}
			ctx = context.WithValue(ctx, auditKey, canAudit)
			next(w, r.WithContext(ctx))
			return
		}
		allow, reason := s.authorize(r, id, "write", pointer)
		if !allow {
			log.Printf("authz deny: sub=%s write %s: %s", id.Sub, pointer, reason)
			http.Error(w, "forbidden: "+reason, http.StatusForbidden)
			return
		}
		next(w, r.WithContext(ctx))
	}
}

// pointerNameOf extracts the pointer name from /pointers/{name}[/log].
func pointerNameOf(r *http.Request) string {
	rest := strings.TrimPrefix(r.URL.Path, "/pointers/")
	return strings.Split(rest, "/")[0]
}

// requireAuth wraps an endpoint that needs any valid token (no pointer
// decision): the identity is verified and becomes the log's writer.
func (s *server) requireAuth(next http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		if s.authz == nil {
			http.Error(w, "auth is not configured", http.StatusNotFound)
			return
		}
		auth := r.Header.Get("Authorization")
		if !strings.HasPrefix(auth, "Bearer ") {
			http.Error(w, "missing bearer token", http.StatusUnauthorized)
			return
		}
		id, err := s.authz.verifyJWT(r.Context(), strings.TrimPrefix(auth, "Bearer "))
		if err != nil {
			http.Error(w, "invalid token: "+err.Error(), http.StatusUnauthorized)
			return
		}
		ctx := context.WithValue(r.Context(), identityKey, id)
		ctx = context.WithValue(ctx, writerKey, id.Sub)
		next(w, r.WithContext(ctx))
	}
}

// handlePermissions serves the client's view of the policy: GET
// /permissions (authenticated) → the projection of the policy onto the
// caller — its roles and merged capabilities, nothing else. The policy
// document itself is never served (its blob is withheld, /blobs 404s
// kind "policy"); this endpoint is the only client-readable form.
func (s *server) handlePermissions(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "use GET", http.StatusMethodNotAllowed)
		return
	}
	var id *identity
	if v := r.Context().Value(identityKey); v != nil {
		id = v.(*identity)
	}
	sub := ""
	var roles []string
	if id != nil {
		sub = id.Sub
		roles = id.Roles
	}
	policy := s.policyDocument()
	var resp struct {
		Effective struct {
			Roles        []string `json:"roles"`
			Capabilities []struct {
				Pointer string   `json:"pointer"`
				Actions []string `json:"actions"`
			} `json:"capabilities"`
		} `json:"effective"`
	}
	rawPolicy := json.RawMessage(policy)
	if len(rawPolicy) == 0 {
		rawPolicy = json.RawMessage(`{"schema_version":1,"rules":[]}`)
	}
	err := s.authz.wasm.Invoke(r.Context(), map[string]any{
		"op":       "effective",
		"identity": map[string]any{"sub": sub, "roles": roles},
		"policy":   rawPolicy,
	}, &resp)
	if err != nil {
		http.Error(w, "effective op failed", http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(resp.Effective)
}

// seedPolicy commits the -policy file as the store's `policy` pointer
// when it does not exist yet (D-61 bootstrap). The blob is the file's
// bytes; the hash is the shared formula (the policy is host-authored
// at seed — later changes are normal CAS commits).
func (s *server) seedPolicy(policyFile string) {
	if s.store == nil || policyFile == "" {
		return
	}
	if _, found, err := s.store.Head("policy"); err == nil && found {
		return
	}
	raw, err := os.ReadFile(policyFile)
	if err != nil || len(raw) == 0 {
		return
	}
	env := store.Envelope{Kind: "policy", SchemaVersion: 1, ProducedBy: "server"}
	if _, _, err := s.store.CAS("policy", "", raw, env, "", "bootstrap:-policy", time.Now().UnixMilli()); err != nil {
		log.Printf("seed policy: %v", err)
		return
	}
	log.Printf("policy seeded into the store from %s", policyFile)
}

// writerOf returns the verified subject when auth ran, else the
// caller's label — this becomes the store log's `writer`.
func writerOf(r *http.Request, fallback string) string {
	if v := r.Context().Value(writerKey); v != nil {
		if s, ok := v.(string); ok && s != "" {
			return s
		}
	}
	if h := r.Header.Get("X-Writer"); h != "" {
		return h
	}
	return fallback
}
