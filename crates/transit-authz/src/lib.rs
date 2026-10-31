//! transit-authz — the authorization policy document and the pure
//! authorize decision.
//!
//! Authentication is the IdP's (the host verifies the JWT: signature,
//! iss/aud/exp — D-60). Authorization is a DOCUMENT on the bus: a
//! policy names rules, each rule names roles, pointers and actions,
//! and `authorize` answers one question — **may this identity perform
//! this action on this pointer?** — fail-closed: no policy, no
//! matching rule, or a malformed request is a deny, never an allow.
//!
//! Everything here is pure: types plus functions over them. The host
//! owns the token and the clock; the policy document is stored like
//! any other (the store's `policy` pointer, seeded from a file at
//! first boot — D-61) and passed to the module per call, the way
//! cap-plans takes the plan set.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The current schema version of the policy document.
pub const SCHEMA_VERSION: u32 = 1;

/// What is being asked about: a read (GET), a write (POST/CAS), or an
/// audit read (the pointer log WITH writer identities — the only
/// place a user identity leaves the store). `"*"` in a rule matches
/// all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Read,
    Write,
    Audit,
}

impl Action {
    /// The string form used in requests and rules: "read", "write",
    /// "audit", "*".
    pub fn as_str(self) -> &'static str {
        match self {
            Action::Read => "read",
            Action::Write => "write",
            Action::Audit => "audit",
        }
    }
}

/// The verified identity a host hands over after authenticating the
/// token: the subject and the roles read from the configured claim
/// path (D-59). Nothing else is trusted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Identity {
    pub sub: String,
    pub roles: Vec<String>,
}

/// One policy rule: identities holding any of `roles` may perform any
/// of `actions` on any of `pointers`. `"*"` matches all actions /
/// pointers. A rule with EMPTY `roles` never matches — an empty
/// intersection is not an intersection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Rule {
    pub name: String,
    pub roles: Vec<String>,
    #[serde(default)]
    pub pointers: Vec<String>,
    #[serde(default)]
    pub actions: Vec<String>,
}

impl Rule {
    fn matches(&self, action: Action, pointer: &str) -> bool {
        let action_ok = self.actions.is_empty()
            || self.actions.iter().any(|a| a == "*" || a == action.as_str());
        let pointer_ok = self.pointers.is_empty()
            || self.pointers.iter().any(|p| p == "*" || p == pointer);
        action_ok && pointer_ok
    }
}

/// The policy document: a versioned rule list, evaluated in order.
/// Fail-closed by construction — see `authorize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Policy {
    pub schema_version: u32,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

impl Policy {
    pub fn current(rules: Vec<Rule>) -> Self {
        Policy { schema_version: SCHEMA_VERSION, rules }
    }
}

/// The answer to one authorize question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Decision {
    pub allow: bool,
    /// The first rule that allowed it, when one did.
    pub matched_rule: Option<String>,
    /// Why — human-readable, safe to log (never echoes the token).
    pub reason: String,
}

/// May `identity` perform `action` on `pointer` under `policy`?
///
/// Laws (property-tested below):
/// - fail-closed: empty policy, no rules, or no matching rule → deny;
/// - a rule matches only when the identity holds at least one of the
///   rule's roles AND the rule's actions/pointers match (empty
///   actions/pointers in a rule mean "all"; empty ROLES mean "never");
/// - the allow/deny outcome is independent of rule ORDER (the first
///   match's name is reported, so the name may vary — the verdict
///   never does);
/// - an unknown pointer is denied unless a rule says "*".
pub fn authorize(policy: &Policy, identity: &Identity, action: Action, pointer: &str) -> Decision {
    if identity.sub.is_empty() {
        return Decision { allow: false, matched_rule: None, reason: "no subject".into() };
    }
    for rule in &policy.rules {
        let role_ok = !rule.roles.is_empty()
            && rule.roles.iter().any(|r| identity.roles.contains(r));
        if role_ok && rule.matches(action, pointer) {
            return Decision {
                allow: true,
                matched_rule: Some(rule.name.clone()),
                reason: format!("rule {:?} grants {} on {} to a held role", rule.name, action.as_str(), pointer),
            };
        }
    }
    Decision {
        allow: false,
        matched_rule: None,
        reason: format!("no rule grants {} on {} to the identity's roles", action.as_str(), pointer),
    }
}

/// One effective capability entry: the actions an identity may perform
/// on one pointer (`"*"` = all pointers).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Capability {
    pub pointer: String,
    pub actions: Vec<String>,
}

/// The policy AS IT APPLIES TO ONE IDENTITY — the client's view: the
/// caller's own roles and the merged capabilities of every matching
/// rule. It never contains other roles, other rules, or the policy's
/// structure beyond what applies to this identity, so a client can
/// drive its UI from it without learning anything about anyone else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EffectivePolicy {
    pub roles: Vec<String>,
    pub capabilities: Vec<Capability>,
}

/// The projection of `policy` onto `identity`: every rule whose roles
/// intersect the identity's, merged per pointer (empty actions/pointers
/// in a rule mean "all"; empty roles never match). Deterministic:
/// capabilities sorted by pointer, actions sorted and deduped.
pub fn effective(policy: &Policy, identity: &Identity) -> EffectivePolicy {
    let mut caps: Vec<Capability> = Vec::new();
    for rule in &policy.rules {
        if rule.roles.is_empty() || !rule.roles.iter().any(|r| identity.roles.contains(r)) {
            continue;
        }
        let actions = if rule.actions.is_empty() { vec!["*".to_string()] } else { rule.actions.clone() };
        let pointers = if rule.pointers.is_empty() { vec!["*".to_string()] } else { rule.pointers.clone() };
        for p in pointers {
            match caps.iter_mut().find(|c| c.pointer == p) {
                Some(c) => {
                    for a in &actions {
                        if !c.actions.contains(a) {
                            c.actions.push(a.clone());
                        }
                    }
                }
                None => caps.push(Capability { pointer: p, actions: actions.clone() }),
            }
        }
    }
    for c in &mut caps {
        c.actions.sort();
        c.actions.dedup();
    }
    caps.sort_by(|a, b| a.pointer.cmp(&b.pointer));
    EffectivePolicy { roles: identity.roles.clone(), capabilities: caps }
}

#[cfg(test)]
mod laws {
    use super::*;

    fn id(roles: &[&str]) -> Identity {
        Identity { sub: "user-1".into(), roles: roles.iter().map(|r| r.to_string()).collect() }
    }

    fn rule(name: &str, roles: &[&str], pointers: &[&str], actions: &[&str]) -> Rule {
        Rule {
            name: name.into(),
            roles: roles.iter().map(|r| r.to_string()).collect(),
            pointers: pointers.iter().map(|p| p.to_string()).collect(),
            actions: actions.iter().map(|a| a.to_string()).collect(),
        }
    }

    #[test]
    fn empty_policy_denies_everything() {
        let policy = Policy::current(vec![]);
        for action in [Action::Read, Action::Write] {
            let d = authorize(&policy, &id(&["admin"]), action, "edits");
            assert!(!d.allow, "fail-closed: {d:?}");
        }
    }

    #[test]
    fn a_rule_matches_only_on_role_action_and_pointer() {
        let policy = Policy::current(vec![rule("ops", &["dispatcher"], &["edits"], &["write"])]);
        assert!(authorize(&policy, &id(&["dispatcher"]), Action::Write, "edits").allow);
        assert!(!authorize(&policy, &id(&["dispatcher"]), Action::Read, "edits").allow, "write does not imply read");
        assert!(!authorize(&policy, &id(&["dispatcher"]), Action::Write, "policy").allow, "other pointer");
        assert!(!authorize(&policy, &id(&["rider"]), Action::Write, "edits").allow, "role not held");
    }

    #[test]
    fn empty_roles_in_a_rule_never_match_and_wildcards_do() {
        let policy = Policy::current(vec![
            rule("nobody", &[], &["*"], &["*"]),
            rule("all-read", &["rider"], &["*"], &["read"]),
            rule("admin-all", &["admin"], &["*"], &["*"]),
        ]);
        assert!(!authorize(&policy, &id(&["rider"]), Action::Write, "edits").allow);
        assert!(authorize(&policy, &id(&["rider"]), Action::Read, "anything").allow, "* pointer, read action");
        assert!(authorize(&policy, &id(&["admin"]), Action::Write, "policy").allow, "*/* for admin");
        assert!(!authorize(&policy, &id(&[]), Action::Read, "x").allow, "identity with no roles");
    }

    #[test]
    fn no_subject_is_a_deny() {
        let policy = Policy::current(vec![rule("all", &["admin"], &["*"], &["*"])]);
        let anon = Identity { sub: "".into(), roles: vec!["admin".into()] };
        assert!(!authorize(&policy, &anon, Action::Write, "edits").allow);
    }

    #[test]
    fn the_verdict_is_rule_order_insensitive() {
        let rules = vec![
            rule("a", &["ops"], &["edits"], &["write"]),
            rule("b", &["ops", "admin"], &["*"], &["*"]),
        ];
        let p1 = Policy::current(rules.clone());
        let mut p2 = Policy::current(rules.clone());
        p2.rules.reverse();
        for action in [Action::Read, Action::Write] {
            for pointer in ["edits", "policy", "manifest"] {
                for roles in [vec!["ops"], vec!["admin"], vec!["rider"]] {
                    let i = id(&roles.iter().map(|s| *s).collect::<Vec<_>>());
                    let d1 = authorize(&p1, &i, action, pointer);
                    let d2 = authorize(&p2, &i, action, pointer);
                    assert_eq!(d1.allow, d2.allow, "order changed the verdict for {roles:?} {action:?} {pointer}");
                }
            }
        }
    }

    #[test]
    fn policy_round_trips_through_json() {
        let policy = Policy::current(vec![rule("ops", &["dispatcher"], &["edits"], &["write", "read"])]);
        let json = serde_json::to_string(&policy).unwrap();
        assert!(json.contains("\"schema_version\":1"), "{json}");
        assert_eq!(serde_json::from_str::<Policy>(&json).unwrap(), policy);
    }

    #[test]
    fn effective_projects_only_the_identitys_own_rules() {
        let policy = Policy::current(vec![
            rule("ops", &["dispatcher"], &["edits"], &["write", "read"]),
            rule("auditors", &["auditor"], &["*"], &["audit"]),
            rule("admin-all", &["admin"], &["*"], &["*"]),
        ]);
        // A dispatcher sees its own rule merged — and nothing about
        // auditor or admin.
        let eff = effective(&policy, &id(&["dispatcher"]));
        assert_eq!(eff.roles, vec!["dispatcher".to_string()]);
        assert_eq!(eff.capabilities, vec![Capability {
            pointer: "edits".into(),
            actions: vec!["read".into(), "write".into()],
        }]);
        // An auditor sees the wildcard audit capability only.
        let eff = effective(&policy, &id(&["auditor"]));
        assert_eq!(eff.capabilities, vec![Capability { pointer: "*".into(), actions: vec!["audit".into()] }]);
        // Empty policy → no capabilities (fail-closed projection).
        assert!(effective(&Policy::current(vec![]), &id(&["admin"])).capabilities.is_empty());
        // Empty roles on the identity → nothing, even with wildcard rules.
        assert!(effective(&policy, &id(&[])).capabilities.is_empty());
    }

    #[test]
    fn effective_merges_wildcards_and_is_deterministic() {
        let policy = Policy::current(vec![
            rule("a", &["ops"], &["edits", "*"], &["write"]),
            rule("b", &["ops"], &["edits"], &["read", "write"]),
        ]);
        let eff = effective(&policy, &id(&["ops"]));
        assert_eq!(eff.capabilities.len(), 2, "edits merged, * separate");
        let edits = eff.capabilities.iter().find(|c| c.pointer == "edits").unwrap();
        assert_eq!(edits.actions, vec!["read".to_string(), "write".to_string()], "sorted, deduped");
        let star = eff.capabilities.iter().find(|c| c.pointer == "*").unwrap();
        assert_eq!(star.actions, vec!["write".to_string()]);
    }
}

#[cfg(test)]
mod prop {
    use super::*;
    use proptest::prelude::*;
    use proptest::collection::vec;

    fn role_name() -> impl Strategy<Value = String> {
        proptest::sample::select(vec!["admin".to_string(), "dispatcher".to_string(), "rider".to_string()])
    }

    fn rule_strat() -> impl Strategy<Value = Rule> {
        (
            "[a-z]{1,6}",
            vec(role_name(), 0..3),
            vec(proptest::sample::select(vec!["*".to_string(), "edits".to_string(), "policy".to_string(), "manifest".to_string()]), 0..3),
            vec(proptest::sample::select(vec!["*".to_string(), "read".to_string(), "write".to_string()]), 0..3),
        )
            .prop_map(|(name, roles, pointers, actions)| Rule { name, roles, pointers, actions })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// Fail-closed everywhere: a deny is the only possible answer
        /// when no rule's role set intersects the identity's.
        #[test]
        fn deny_when_no_role_intersects(
            rules in vec(rule_strat(), 0..5),
            identity_roles in vec(role_name(), 0..3),
            action in proptest::sample::select(vec![Action::Read, Action::Write, Action::Audit]),
            pointer in proptest::sample::select(vec!["edits", "policy", "manifest", "plans"]),
        ) {
            let policy = Policy::current(rules);
            let identity = Identity { sub: "u".into(), roles: identity_roles };
            let d = authorize(&policy, &identity, action, pointer);
            let any_role_overlap = policy.rules.iter().any(|r| {
                !r.roles.is_empty() && r.roles.iter().any(|x| identity.roles.contains(x))
            });
            if !any_role_overlap {
                prop_assert!(!d.allow, "allowed with no role overlap: {d:?}");
            }
        }

        /// Verdict independent of rule order.
        #[test]
        fn verdict_is_order_insensitive(
            rules in vec(rule_strat(), 0..5),
            identity_roles in vec(role_name(), 0..3),
            action in proptest::sample::select(vec![Action::Read, Action::Write, Action::Audit]),
            pointer in "[a-z]+",
        ) {
            let mut reversed = rules.clone();
            reversed.reverse();
            let p1 = Policy::current(rules);
            let p2 = Policy::current(reversed);
            let identity = Identity { sub: "u".into(), roles: identity_roles };
            prop_assert_eq!(
                authorize(&p1, &identity, action, &pointer).allow,
                authorize(&p2, &identity, action, &pointer).allow,
            );
        }

        /// The projection is deterministic under rule permutation
        /// (output is sorted), so clients see a stable answer.
        #[test]
        fn effective_is_order_insensitive(
            rules in vec(rule_strat(), 0..5),
            identity_roles in vec(role_name(), 0..3),
        ) {
            let mut reversed = rules.clone();
            reversed.reverse();
            let identity = Identity { sub: "u".into(), roles: identity_roles };
            prop_assert_eq!(
                effective(&Policy::current(rules), &identity),
                effective(&Policy::current(reversed), &identity),
            );
        }
    }
}
