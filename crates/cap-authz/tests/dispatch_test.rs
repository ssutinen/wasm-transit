//! Native dispatch tests for the authorization capability — the seam
//! without wasm. The laws (fail-closed, order-insensitive verdict,
//! role/action/pointer matching) are property-tested in
//! transit-authz; here the seam pins them through the module:
//!
//! 1. a matching rule allows; anything else denies — empty policy,
//!    unknown action, unknown pointer, missing role all deny;
//! 2. an unknown action string is an error, never a silent allow;
//! 3. garbage and bulk bytes are honest errors, never panics.

use cap_authz::{dispatch, dispatch_bytes, dispatch_json, Request, Response};
use transit_authz::{Identity, Policy, Rule};

fn policy() -> Policy {
    Policy::current(vec![
        Rule {
            name: "ops-write".into(),
            roles: vec!["dispatcher".into()],
            pointers: vec!["edits".into()],
            actions: vec!["write".into()],
        },
        Rule {
            name: "admin-all".into(),
            roles: vec!["admin".into()],
            pointers: vec!["*".into()],
            actions: vec!["*".into()],
        },
    ])
}

fn identity(roles: &[&str]) -> Identity {
    Identity { sub: "user-1".into(), roles: roles.iter().map(|r| r.to_string()).collect() }
}

fn ask(id: &Identity, action: &str, pointer: &str) -> bool {
    match dispatch(Request::Authorize {
        identity: id.clone(),
        action: action.into(),
        pointer: pointer.into(),
        policy: policy(),
    })
    .unwrap()
    {
        Response::Authorize { decision } => decision.allow,
        _ => panic!("authorize"),
    }
}

#[test]
fn matching_rule_allows_and_everything_else_denies() {
    assert!(ask(&identity(&["dispatcher"]), "write", "edits"), "rule ops-write");
    assert!(!ask(&identity(&["dispatcher"]), "read", "edits"), "write does not imply read");
    assert!(!ask(&identity(&["rider"]), "write", "edits"), "role not held");
    assert!(!ask(&identity(&["dispatcher"]), "write", "policy"), "pointer not in the rule");
    assert!(ask(&identity(&["admin"]), "write", "policy"), "admin wildcard");
    assert!(ask(&identity(&["admin"]), "read", "manifest"), "admin read anywhere");
}

#[test]
fn empty_policy_is_fail_closed() {
    match dispatch(Request::Authorize {
        identity: identity(&["admin"]),
        action: "write".into(),
        pointer: "edits".into(),
        policy: Policy::current(vec![]),
    })
    .unwrap()
    {
        Response::Authorize { decision } => {
            assert!(!decision.allow);
            assert_eq!(decision.matched_rule, None);
            assert!(!decision.reason.is_empty());
        }
        _ => panic!("authorize"),
    }
}

#[test]
fn effective_projects_only_the_callers_own_capabilities() {
    match dispatch(Request::Effective { identity: identity(&["dispatcher"]), policy: policy() }).unwrap() {
        Response::Effective { effective } => {
            assert_eq!(effective.roles, vec!["dispatcher".to_string()]);
            // Its own merged capabilities — and nothing about admin.
            assert_eq!(effective.capabilities.len(), 1);
            assert_eq!(effective.capabilities[0].pointer, "edits");
            assert_eq!(effective.capabilities[0].actions, vec!["write".to_string()]);
            let json = serde_json::to_string(&effective).unwrap();
            assert!(!json.contains("admin"), "no other roles leak: {json}");
        }
        _ => panic!("effective"),
    }
    // Empty policy → empty projection (fail-closed).
    match dispatch(Request::Effective { identity: identity(&["admin"]), policy: Policy::current(vec![]) }).unwrap() {
        Response::Effective { effective } => assert!(effective.capabilities.is_empty()),
        _ => panic!("effective"),
    }
}

#[test]
fn unknown_action_is_an_error_not_an_allow() {
    let err = dispatch(Request::Authorize {
        identity: identity(&["admin"]),
        action: "delete-everything".into(),
        pointer: "edits".into(),
        policy: policy(),
    })
    .unwrap_err();
    assert!(err.contains("unknown action"), "{err}");

    let bad = dispatch_json(r#"{"op":"authorize","identity":{"sub":"u","roles":["admin"]},"action":"write","pointer":"edits","policy":{"schema_version":1,"rules":[]}}"#);
    assert!(bad.contains("\"allow\":false"), "empty rules deny through the wire: {bad}");
    let bad = dispatch_json(r#"{"op":"nope"}"#);
    assert!(bad.contains("invalid request"), "{bad}");
    let bad = dispatch_bytes(0, b"x");
    assert!(bad.contains("unknown bulk op"), "{bad}");
}
