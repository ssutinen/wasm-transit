# Testing expectations

Every change lands with tests at the layer that owns the logic.

## Laws first — from the intent, never from the code

Every behavior change STARTS with the property tests (laws) that
define it, written before the implementation and run red. This repo
has shipped wrong behavior whose prop tests were written to match
the implementation, so the tests certified the bug. The rules:

- **Derive each law from the stated intent** — the user's directive,
  the GTFS spec, the architecture laws — never from what the code
  currently does. Write down, in the law's doc comment, the intent
  it encodes.
- **Independent oracles.** A law's expected answer is recomputed
  from the generated input with code that shares nothing with the
  implementation (`docs/CONVENTIONS.md`, native tests first).
- **Generators produce only valid input** for the domain the law
  claims (a spec-valid feed is valid against the GTFS spec, not
  against our parser). An invalid generator makes a law demand the
  impossible.
- **Audit the existing laws when intent changes.** A law that
  encodes the old behavior is rewritten or deleted in the same
  change that adds the new ones — never left to pass by accident.
- **A red law is fixed in the implementation.** A law is weakened
  or deleted only when the user's intent says the law is wrong, and
  the change says so.
- **Ambiguity is asked, not assumed.** When the intent does not
  settle a behavior the law must pin (an edge case, a spec
  conflict, two readings of a directive), stop and ask the user;
  list the decisions a new law encodes so they can be checked
  before the implementation starts. Every open question is an
  entry in `docs/DECISIONS.md`, which names the work it blocks; a
  law landed on an unconfirmed answer is entered there as
  *Encoded provisionally* and names its pins.

Two runners: `just test` is the host-side gate (no browser) — run
it before declaring any change done; `just e2e` adds the browser
runner, for changes that touch user-visible flows; `just check`
runs both, for changes that touch both sides. A third runner is
optional: `just spec` model-checks the sync store engine
(`server/store/store.tla`) and needs Java 11+ plus tla2tools.jar —
run it only when a change touches the sync engine (`server/store/`,
the store's HTTP surface, or the spec itself); nothing else requires
Java, and `just test` never does. Default bounds are the small pinned
`store.cfg`; the exhaustive tier is
`SPEC_CFG=server/store/store-exhaustive.cfg just spec` (blob-hash
symmetry, larger bounds, still exhaustive; `TLC_WORKERS`/`TLC_XMX`
size the run).

1. **Unit/property tests in the owning crate** (oracle style for
   laws — e.g. `transit-network`'s `trip_law_test` holds that trips
   are stored records (ids, times, edits that keep them, documents
   written before trips were stored); `transit-codec`'s
   `import_prop_test` holds that the imported document is valid by
   construction over every spec-valid feed and deterministic, its
   `trip_import_law_test` that the feed's trips are the document's,
   its `codec_prop_test` the export round trip, and its
   `spec_round_trip_test` that
   GTFS → network → GTFS loses nothing in the GTFS Schedule spec,
   one law per file, against a raw-table oracle independent of the
   parser; `transit-plans`'s chain tests hold the
   service-plan chain laws (resolution, windows, validity,
   order-insensitivity); transit-dispatch's `dispatch_laws_test`
   holds the diversion/ripple laws).
2. **A dispatch test if a new op exists** (native, JSON round-trip)
   — the module's `tests/dispatch_test.rs`, plus a flow in
   `crates/cap-abi/caps-smoke.mjs` and a chained runner gate in the
   justfile (`just test` runs both; `just test` and `just e2e`
   build `caps` first). The manifest is generated with REAL
   computed examples — extend `gen_manifest`'s `example_for` so an
   op's docs cannot drift from the module.
3. **A Go round-trip test** if server-visible behavior changes.
4. **Playwright e2e for user-visible flows** (Svelte components
   should not need changes when only the ABI underneath changes).
   External services are answered by the spec (`page.route`): the
   live feeds behind `/rt`, the edit document, and the geocoder
   (the manifest's `services` entry and `/geocode`) — no spec needs
   a running Pelias, and the e2e servers list none, so the
   no-geocoder path is the default. A runner gate stands in for an
   oracle with a `data:` URL (`--ask`).
   The capability bus keeps its own spec (`web/e2e/caps.spec.mjs`:
   assembly, composition, degraded state, migrate/rollback) —
   extend it for bus-visible changes; capability law tests
   (migration laws, validator mutation law, the dispatch
   diversion laws) follow expectation 1.
5. **Web parity ratchets** (`just test`, "WEB GATES"): components
   never import the capability resolver, no host code reads raw
   rt `stop_updates`, and nothing clones a document to mutate it
   — each against an allowlist that only shrinks, all empty
   (`docs/ARCHITECTURE.md`, web section, "Parity"). Logic a view needs
   lands as an op with expectations 1–2, never in TypeScript; a
   unit that empties an allowlist entry removes it in the same
   change.
6. **A generated UI** (`src/uis/<id>/`) lands with
   `web/e2e/ui-<id>.spec.mjs` on the fixture feed, which walks its
   questions end to end, and with a dogfood session (below) in which
   its numbers on the real feed match the runner's. That session
   runs BEFORE the UI is built: the questions' op gaps become
   findings and the UI's blockers (`docs/ARCHITECTURE.md`,
   "Generated UIs", the procedure). The GENERATED
   UIs gate in `just test` refuses one without its spec, and
   `e2e/picker.spec.mjs` covers its card with no edit.
7. **Dogfood on real data**: capability work touching feeds or
   payload size runs a `caps-run` session against the C-TRAN feed
   before it is done — fixture-scale gates do not catch real-data
   defects.

## Dogfood QA — the agent interface mirrors the human UI

`caps-run` exercises the same capabilities, bytes, and laws the app
runs; a human asking planner questions through it is doing real QA.
The rules this paradigm earned (the founding C-TRAN session found
four real defects fixture gates missed — a 64 KiB output truncation
among them):

- **All capability work is dogfooded on real data before it is
  done.** Anything touching feeds or payload size runs against
  `testdata/ctran-gtfs.zip` (the real C-TRAN GTFS, NOT COMMITTED —
  licensed data; fetch from c-tran.com) through `caps-run`. The
  reference hashes are in `docs/QA-FINDINGS.md`'s header.
- **A one-off script is never tooling — it is a finding.** If a QA
  session needs bespoke glue (a bootstrap script, a summarizer),
  that is a missing op or runner feature; log it in
  `docs/QA-FINDINGS.md`, don't ship the script.
- **The mirror cuts both ways.** A question that is one op via
  the CLI but absent from the UI is a UI gap; a question awkward
  via the CLI is an op gap. Dogfood sessions generate both
  signals; both land in `docs/QA-FINDINGS.md`.
- **Findings are open work, not a log.** A session adds new
  findings (and re-confirmation lines on old ones) to
  `docs/QA-FINDINGS.md`; the change that fixes a finding deletes it
  and names its pin. The session narrative itself is not kept —
  what it proved lands as tests, and what it found lands as
  findings.

## Performance baselines — measured, never gated

`just perf` drives a generated UI in Chromium against the real feed
and reports what it costs the browser; it fails only when the UI
cannot be driven, never on a number (timings on a dev box are noisy).
Its spec is `web/perf/<ui>.perf.mjs`, its probe `web/perf/probe.mjs`
(the DevTools protocol, test-side only — the page is the shipped page,
observed).

- **What it measures, per step:** wall time; main-thread, script,
  layout and style time; long tasks and long animation frames (their
  scripts named); a sampled CPU profile, summarized by self time per
  bucket — a capability module's crate (`wasm:transit_router`), a JS
  bundle file, the engine (GC, compile) — and saved as a
  `.cpuprofile` for DevTools' Performance panel. Memory, after a
  forced collection: the JS heap, the array buffers outside it, each
  capability module's linear memory by manifest id (the JS heap
  counters do not see it, and it never shrinks), DOM nodes and
  listeners.
- **The build it measures** is a profiling build: the capability
  modules keep their name sections (`CARGO_PROFILE_RELEASE_STRIP=
  debuginfo`; the code is the shipped code) and the frontend is
  unminified (`web/dist-perf`). The next `just caps` restores the
  stripped modules.
- **The live feeds are recorded, then replayed** one snapshot per
  poll, with the browser clock installed at the recording's first
  header timestamp and fast-forwarded a poll at a time — a 45-minute
  soak takes minutes and every run sees the same data.
  `node web/perf/record-rt.mjs [count] [dir] [interval]` records the
  feeds `web/public/live-feeds.json` names into `testdata/perf-rt*/`
  (gitignored — agency data); `PERF_RT=testdata/perf-rt-45m just perf`
  picks one.
- **The soak** leaves the UI open while every remaining snapshot
  arrives, sampling memory every 10 polls: growth across the samples
  that does not level off is a leak to chase with a heap snapshot.
- **Reports** land in `web/perf-results/<stamp>/` (`summary.md`,
  `report.json`, `profiles/`, gitignored). Headless Chromium renders
  the map with software WebGL, so map rendering costs more there than
  on a device with a GPU — compare runs with each other, not with a
  phone.
