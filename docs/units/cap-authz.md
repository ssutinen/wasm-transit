# cap-authz — authorization on the sync server

Landed (first slice): authentication is the IdP's — the Go server
verifies the bearer JWT and nothing more; authorization is a document
on the bus plus a module that answers the one question fail-closed.
Decisions: D-58 (public read, auth'd write), D-59 (configurable roles
claim path), D-60 (JWKS preferred, pinned-key fallback, RS256/ES256
only), D-61 (policy = store pointer seeded from `-policy`).

## What landed

- **transit-authz** (library): the `Policy` document (versioned rule
  list; a rule = roles × pointers × actions, `"*"` wildcards, empty
  roles never match) and the pure `authorize` — fail-closed, verdict
  independent of rule order. Law + property tests.
- **cap-authz** (module): one op, `authorize`, stateless like cap-plans
  (the host pushes the policy per call). Registered in the manifest
  with a computed worked example; smoke-pinned.
- **Go server** (`server/authz.go`): JWT verification — JWKS fetched
  and cached by `kid` (refetch on unknown kid, rate-limited 30 s), or a
  pinned PEM key; `alg: none`/HS* rejected; iss/aud exact, exp/nbf with
  60 s leeway; roles read at `-oidc-roles-claim` (dotted path, default
  `roles`). `protectWrite` wraps the write endpoints (`/edits`,
  `/pointers/*`): 401 without a valid token, 403 when cap-authz denies,
  and the verified `sub` becomes the store log's `writer`. With no
  `-oidc-*` flags auth is OFF and the server logs that loudly (dev only).
  Policy lives at the store's `policy` pointer, seeded from `-policy`
  at first boot; changes are authenticated, audited CAS commits
  (`POST /pointers/policy` needs write on `policy`). The manifest pins
  `authz_hash` and `policy_hash` when present.

## What remains

- **JWKS bootstrap trust**: with `-oidc-jwks-url` the first fetch is
  unauthenticated HTTPS to the IdP (standard); pin the IdP's TLS cert
  in a production runbook if you want more.
- **Rate limiting / lockout** on 401s (the fail-closed path is
  cheap, but a brute-force probe should be throttled at the reverse
  proxy, not here — the server stays scaffolding).
- **Per-device tokens**: the browser holds the IdP token (its own
  concern) and presents it on writes; the read path needs none (D-58).

## Client isolation (landed)

A client learns only what applies to the authenticated user:

- The policy document is never served: `/blobs/{hash}` 404s any blob
  whose envelope kind is `policy` (even for an admin). The manifest may
  pin `policy_hash` — a hash is opaque and lets clients detect rotation.
- `GET /permissions` (any valid token) returns the **projection** of the
  policy onto the caller — cap-authz's `effective` op: its own roles and
  merged `{pointer, actions}` capabilities, sorted, wildcards symbolic.
  No other roles, no rule names beyond matched ones, no structure. Pinned
  by `TestAuthzPermissionsProjection` (asserts the response contains no
  other role/rule names) and the dispatch/smoke tests.
- The pointer log (`GET /pointers/{name}/log`) is authenticated (read
  on the pointer; an `audit` rule implies it) and its `writer` fields —
  the only user identities in the store — are redacted unless the caller
  holds an `audit` rule on that pointer. Pinned by
  `TestAuthzLogWriterRedaction`.
- Edit/plan/network documents carry no author field at all
  (content-addressed, shared); per-user data (watchlists) stays
  client-side and is never synced.

## Done when (remaining pins)

- A dogfood session against the C-TRAN deployment shape: token from a
  real IdP (or the test JWKS), a dispatcher writing a diversion, an
  admin rotating the policy, restart → policy pointer intact, log
  writers = subs. (Go tests cover all of this except the real IdP.)
