//! cap-authz — the authorization capability.
//!
//! One question: **may this identity perform this action on this
//! pointer?** Authentication is the IdP's — the host verifies the JWT
//! (signature via JWKS or a pinned key, iss/aud/exp, D-60) and reads
//! the roles claim (D-59). This module owns only the decision:
//! `authorize` evaluates the policy document the host passes per call
//! (the store's `policy` pointer, D-61) and answers fail-closed — no
//! policy, no subject, or no matching rule is a deny, never an allow.
//!
//! The module is stateless: no registries, no persistence — like
//! cap-plans, the host owns the document and pushes it per call.
//! Authorization is never cached across calls; the policy document's
//! hash is the cache key if a host wants one.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One operation call. Tagged like every capability's seam.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "cap_authz/")]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// May `identity` perform `action` ("read" | "write") on `pointer`
    /// under `policy`? Fail-closed; the verdict never depends on rule
    /// order (transit_authz::authorize, property-tested).
    Authorize {
        identity: transit_authz::Identity,
        action: String,
        pointer: String,
        policy: transit_authz::Policy,
    },
    /// The policy AS IT APPLIES TO THIS IDENTITY — the client's view:
    /// the caller's roles and merged capabilities only, never other
    /// roles or rules. What `GET /permissions` serves.
    Effective {
        identity: transit_authz::Identity,
        policy: transit_authz::Policy,
    },
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "cap_authz/")]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Response {
    Authorize { decision: transit_authz::Decision },
    Effective { effective: transit_authz::EffectivePolicy },
}

/// The typed dispatch (native hosts call this directly). Failures
/// are `{"error": ...}`, never panics — a total seam.
pub fn dispatch(request: Request) -> Result<Response, String> {
    match request {
        Request::Authorize { identity, action, pointer, policy } => {
            let action = match action.as_str() {
                "read" => transit_authz::Action::Read,
                "write" => transit_authz::Action::Write,
                "audit" => transit_authz::Action::Audit,
                other => return Err(format!("unknown action {other:?}: want \"read\", \"write\" or \"audit\"")),
            };
            Ok(Response::Authorize {
                decision: transit_authz::authorize(&policy, &identity, action, &pointer),
            })
        }
        Request::Effective { identity, policy } => Ok(Response::Effective {
            effective: transit_authz::effective(&policy, &identity),
        }),
    }
}

/// The JSON seam: `{"op": ...}` in, `{"ok": ...}` / `{"error": ...}`
/// out — byte-compatible with every other capability module.
pub fn dispatch_json(text: &str) -> String {
    let outcome = match serde_json::from_str::<Request>(text) {
        Ok(request) => match dispatch(request) {
            Ok(response) => serde_json::json!({ "ok": response }),
            Err(error) => serde_json::json!({ "error": error }),
        },
        Err(error) => serde_json::json!({ "error": format!("invalid request: {error}") }),
    };
    serde_json::to_string(&outcome).expect("dispatch outcomes serialize")
}

/// No bulk-bytes op exists — the policy is a JSON document.
pub fn dispatch_bytes(op: u32, _bytes: &[u8]) -> String {
    serde_json::json!({ "error": format!("unknown bulk op {op}: the policy document is JSON (authorize)") })
        .to_string()
}

cap_abi::capability! {
    json: dispatch_json,
    bytes: dispatch_bytes,
}
