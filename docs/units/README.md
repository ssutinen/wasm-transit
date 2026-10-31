# Units — the open work

One file per unit of open work. This directory is the only place
status lives: if a unit's file exists, the unit is open; when it
lands, the change that lands it DELETES the file. Git is the
archive — nothing here records what is finished.

## The rules

- **A unit file says what is still true of the work, never its
  history.** Goal (one paragraph), scope (what it touches), the
  order inside it if any, and **Done when** — the pins that must
  exist (tests, gates, e2e specs, a dogfood session on real data)
  before the file is deleted.
- **Grades are not merged.** On the bus (module + tests), the UI
  reads it, and dogfooded on real data are three claims. A unit
  whose work is on the bus but not read by a UI or not dogfooded
  stays open (or moves its remaining grade into
  [grade-debt.md](grade-debt.md)) — it is not deleted.
- **Deleting a unit is the DONE claim.** The agent that deletes a
  file reports the pins that prove it in the same change. A
  partial landing edits the file down to what is left.
- **What a landed unit built is described in the present tense in
  the architecture docs** (`docs/ARCHITECTURE.md`,
  `crates/AGENTS.md`, `docs/CONVENTIONS.md`), in the same change.
- **A finding becomes a unit only when it is picked up**: the unit
  names the `QAF-N` it closes; the finding is deleted from
  `docs/QA-FINDINGS.md` when the unit lands.
- New ops follow the extraction pattern (`crates/AGENTS.md`):
  library first, capability wrapper second.
- **A unit waiting on the user names the decision**: `Blocked:
  D-N` against the part it blocks. The question itself lives only
  in `docs/DECISIONS.md`; the rest of the unit proceeds.

## Open units

| unit | what it is |
|---|---|
| [host-discipline.md](host-discipline.md) | the composition layer: one op caller, flow maps as data, the journal, replay |
| [wire-time-assembly.md](wire-time-assembly.md) | per-user selection over the manifest; layouts as documents |
| [sync-persistence.md](sync-persistence.md) | self-describing blobs + pointers, pointer CAS, the store reflog, TLC-checked, generated schemas + `docs/FORMATS.md` |
| [cap-authz.md](cap-authz.md) | JWT verification at the server, the policy document on the store, cap-authz fail-closed |
| [runtime-cost-gate.md](runtime-cost-gate.md) | a ratchet on per-module memory and registration time |
| [module-runtime.md](module-runtime.md) | bounded registries (a kernel eviction op) and off-thread instantiation |
| [streets-model.md](streets-model.md) | streets as a model document: serving it, the bus profile, walking for riders, drawing on streets, the geography release |
| [caps-run-sessions.md](caps-run-sessions.md) | the runner across sessions; shaping the import's output |
| [grade-debt.md](grade-debt.md) | on the bus, but not yet read by a UI or dogfooded |
| [ui-ride-now.md](ui-ride-now.md) | generated UI: riding today (stops, live boards, trip plans, last bus, detours) |
| [ui-stop-display.md](ui-stop-display.md) | generated UI: the screen at a stop (next three per route and destination, three layouts, alerts) |
| [ui-disruptions.md](ui-disruptions.md) | generated UI: running today's service through disruptions (late buses, detours, coverage) |
| [ui-proposal-compare.md](ui-proposal-compare.md) | generated UI: the public comparing service proposals |
| [ui-service-design.md](ui-service-design.md) | generated UI: designing a service change in six chained views — the blocked questions and the views' remaining parts |
| [ui-runcut.md](ui-runcut.md) | generated UI: cutting a plan's work into blocks and runs |
| [ui-work-bid.md](ui-work-bid.md) | generated UI: bidding on work (waits on the runs layer) |

## Queued, not yet scoped

Ideas with a direction but no unit yet. Scope one into a file
before starting it.

- **Registry search** — a derived artifact (purpose + op docs +
  worked examples embedded; ranking carries evidence): an agent
  browses the catalog and assembles its own harness.
- **caps-run harness as data** — when a session improvisation
  recurs, crystallize it; until then improvisations are findings.
  The journal ([host-discipline.md](host-discipline.md))
  crystallizes the chained-session format first.
- **Fuel metering** in the runner, once it may point at untrusted
  registry entries (the wall-clock guard is the stopgap).

The feature direction (what the software becomes) is
`docs/ROADMAP.md`; the real-data findings are
`docs/QA-FINDINGS.md`.
