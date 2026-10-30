# Decisions

This is the durable record of the questions put to the user and
their answers. When
an agent hits a decision that the user's intent, the GTFS spec,
and the architecture laws leave open, the agent writes it down here
and does not build on an answer it picked (AGENTS.md, "Clarify,
don't assume"). The entry names the work the decision blocks, so
the rest of the work can go ahead while the decision is reviewed.

## The rules

- **One entry per question**, in this format:

  ```
  ### D-N — the question
  State: Unanswered | Encoded provisionally | Decided
  Answer: (Decided only) the user's answer, dated, in their words.
  Context: what the intent, the spec, and the code say, briefly.
  Options: each answer and what it costs; mark the one the code
    encodes (if any).
  Blocks: the units, findings, or laws that cannot proceed or be
    deleted until it is decided. "Nothing" is a valid answer.
  ```

- **State.** *Unanswered* means nothing is built on any answer, and
  the work it blocks waits. *Encoded provisionally* means an agent
  had to pick an answer to land a law. The laws that pin that
  answer are named, and a different decision rewrites them.
- **Blocked work names its decision.** A unit or finding that
  waits on a decision says `Blocked: D-N`. Work that does not name
  one is free to start.
- **A decided entry stays — decisions are a durable record, never
  deleted.** Its state becomes *Decided* and its Answer line records
  what the user said and when. In the same change, the answer goes
  into the law that pins it and the doc that owns its topic
  (`docs/ARCHITECTURE.md`, `docs/units/`, `docs/QA-FINDINGS.md`),
  and the `Blocked: D-N` lines are removed. A later answer that
  reverses one is a new entry naming the entry it replaces; the old
  entry's state says so.
- IDs are never reused; the next new decision is **D-58**.

## Summary

| id | question | state | blocks |
|---|---|---|---|
| D-1 | Is a trip's start its first arrival or its first departure? | encoded | nothing |
| D-2 | What does a running-time change touch besides fixed-start trips? | encoded | nothing |
| D-3 | Which timing profile does the import give a pattern? | encoded; re-opened | QAF-90 |
| D-5 | Does the import keep a block that breaks the spec? | encoded | nothing |
| D-6 | Does "frequent" count terminal arrivals? | unanswered | QAF-78 |
| D-8 | Do timetable edits act on the whole service family? | unanswered | QAF-65 gestures, QAF-44 |
| D-9 | Where does required recovery time come from? | unanswered | QAF-62, QAF-26 |
| D-10 | Agency questions on C-TRAN's data | unanswered | QAF-9, QAF-48 |
| D-11 | Does editing a shared alignment change every pattern on it? | unanswered | an alignment-authoring edit |
| D-12 | What headsign does a new trip get? | encoded | nothing |
| D-14 | Does the proposal comparison collect public responses? | unanswered | `ui-proposal-compare` question 6 |
| D-15 | What makes a trip "covered"? | unanswered | `ui-disruptions` question 6 |
| D-16 | Where does the runs layer live? | encoded | nothing |
| D-17 | What does "the last bus" mean with no deadline? | encoded | nothing |
| D-18 | How do live delays meet a diverted trip? | encoded | nothing |
| D-19 | Does an alert for one direction reach both? | encoded | nothing |
| D-20 | Which language does an alert speak? | encoded | nothing |
| D-21 | Where is a trip going when nothing signs it? | encoded | nothing |
| D-22 | How is a route's rider variant named and ordered? | encoded | nothing |
| D-23 | In what order does a stop display list its groups? | encoded | nothing |
| D-24 | How does a departure's countdown round? | encoded | nothing |
| D-25 | Which streets are the source of truth? | decided | nothing |
| D-26 | Which streets can each mode use? | decided (walking); unanswered (a bus) | drawing on streets |
| D-27 | What does place search find? | decided | nothing |
| D-28 | Where do stop and address results meet? | encoded | nothing |
| D-29 | What area does a geography release cover? | unanswered | streets-model geography release |
| D-30 | What does the streets import call a street, and what class is it? | encoded | nothing |
| D-31 | Can a plan add a street the data does not have? | unanswered | streets-model planned streets |
| D-32 | How does the router walk with no walk table? | encoded | nothing |
| D-33 | How does a trip from a place walk? | encoded | nothing |
| D-34 | Where does a stop no walk reaches go in the nearby list? | encoded | nothing |
| D-35 | Among plans arriving together, where does the rider board and get off? | encoded | nothing |
| D-36 | Is geocoding a document on the bus or a service the host calls? | decided | nothing |
| D-37 | What names a spot dropped on the map? | encoded | nothing |
| D-38 | What may a host do with an external service's answer? | decided | nothing |
| D-39 | Do the live feeds move into the manifest's services? | unanswered | live-feed discovery |
| D-40 | How long does a planned trip take, and from when? | unanswered | QAF-81 |
| D-41 | What carries a service type's calendar into a fork's period? | unanswered | QAF-54 |
| D-42 | When a route has stopped for the day, how far ahead does a stop look? | unanswered | QAF-82 |
| D-43 | What does a coverage comparison report? | unanswered | QAF-20 |
| D-44 | Is a branch of a direction its own thing? | unanswered | QAF-63 |
| D-45 | Which dispatcher edits does the edit document carry beyond the diversion? | unanswered | QAF-36, D-15 |
| D-46 | How many more vehicles does a proposal need? | unanswered | QAF-3 |
| D-47 | Which layers does coverage take? | unanswered | QAF-18, QAF-28 |
| D-48 | Does a stop answer to a name its sign does not carry? | unanswered | QAF-70 |
| D-49 | Where do the basemap's glyphs and sprites come from? | unanswered | QAF-84 |
| D-50 | What are the work rules a run is cut under? | unanswered | QAF-11, QAF-12, QAF-14, QAF-15 |
| D-51 | What does a revenue hour, a revenue mile, and a vehicle cost? | unanswered | `ui-service-design` questions 2, 18 (cost) |
| D-52 | How is a route-hour's frequency judged and drawn? | encoded | nothing |
| D-53 | What ridership data does the planner bring? | unanswered | `ui-service-design` question 10 |
| D-54 | What does a frequency change keep of the trips already there? | unanswered | QAF-90, `ui-service-design` question 16 |
| D-55 | Which members does a service group count on a day? | unanswered | QAF-91 |
| D-56 | Whose clock is the LWW clock? | encoded | the receive path of sync-persistence |
| D-57 | How long does a tombstone live? | encoded | the GC/retention section of sync-persistence |

## The trip model

### D-1 — Is a trip's start its first arrival or its first departure?
State: Encoded provisionally.
Context: a stored trip's times are offsets from its start, and the
trip laws require the first stop's arrival offset to be 0.
`frequencies.txt` defines `start_time` as the time the first
vehicle *departs* the first stop. A trip whose first stop has a
dwell (arrival before departure) therefore has two candidate
starts. The feed's own frequency base is kept in
`Boundary::frequency_bases`, so export is lossless under either
answer.
Options:
- **First arrival (encoded).** `RetimeTrip`, `AddTrip`, headway
  bands, and the timetable all read the arrival at the first stop.
- First departure. This matches the spec's frequency wording, and
  a rider's "leaves at". The first arrival then becomes a negative
  offset or is dropped.
Pinned by: `trip_law_test.rs`, and `FirstOffsetNotZero` in
`transit-network` validation (`network_test.rs`).
Blocks: nothing. A change rewrites those laws and the codec's
frequency export.

### D-2 — What does a running-time change touch besides fixed-start trips?
State: Encoded provisionally.
Context: `adjust_running_time` names a route, a direction, a
service type, and a window. It rescales the stored times of each
fixed-start trip that starts in the window, keeping the first
stop's dwell.
Options:
- **Fixed-start trips only (encoded).** The pattern's timing
  profiles and any frequency trips are left alone. A trip added
  later with `add_trip` gets the profile's old times, and a
  frequency trip that runs through the window keeps its old times.
- Also update the profile in the window, so that new trips inherit
  the change.
- Also rescale the frequency trips that overlap the window. A
  frequency trip has one set of times, so one that only partly
  overlaps has to be split into two trips, and the new trip needs
  an id.
Pinned by: `running_time_changes_only_the_named_trips`
(`trip_law_test.rs`) and `timetable_edit_test.rs`.
Blocks: nothing.

### D-3 — Which timing profile does the import give a pattern?
State: Encoded provisionally.
Context: profiles only produce times for new trips; they never
govern stored trips. GTFS has no profiles, so the import has to
derive one.
Options:
- **One all-day profile from the pattern's earliest trip
  (encoded).** A trip added at 17:00 gets the 05:00 run time.
- One profile per time band, taken from clusters of observed run
  times. This is closer to a scheduler's practice, but it needs a
  clustering rule, which is a further decision.
- No profile. `add_trip` on an imported pattern then has to name
  its times or copy them from a neighbouring trip.
Pinned by: `trip_import_law_test.rs` and `codec_prop_test.rs`.
Re-opened 2026-10-30 by QAF-90: on 071 an evening trip added by
`set_headway` runs 20 min beside trips that run 28. The user, the
same day: "traffic pattern[s] change throughout the day and layover
needs, reliefs require flexibility. so please don't make a rigid
timetable setup." That points to profiles per time band, which is
the second option. It stays provisional until the user confirms it
and names the band rule.
Blocks: QAF-90.

### D-11 — Does editing a shared alignment change every pattern on it?
State: Unanswered.
Context: patterns run alignments by reference (`Pattern::alignment`),
and several patterns may run one: a feed may give one shape to
trips with different stops. No edit authors an alignment yet. The
first edit that redraws a pattern's line has to say what happens to
the other patterns on the same alignment.
Options:
- The edit changes the alignment, so every pattern on it is
  redrawn. This is one place to fix a line, but a change meant for
  one variant moves the others silently.
- The edit copies the alignment under a new id for the edited
  pattern, and the others keep the old line. This follows "new
  entities get new ids", but a fix meant for all of them must be
  made once per pattern.
- The edit names which it does, and the gesture reports the
  patterns it touched.
Blocks: an alignment-authoring edit (none exists; `UpsertPattern`
can point a pattern at an existing alignment, but no edit adds or
redraws one).

### D-5 — Does the import keep a block that breaks the spec?
State: Encoded provisionally.
Context: GTFS forbids trips in one block that overlap on the same
service day. A real feed can still carry them.
Options:
- **Import as written (encoded).** Validation reports
  `BlockInfeasible`, nothing is dropped, and export reproduces the
  feed.
- Drop or split the offending trips, with an import note. This
  loses data, which conflicts with the lossless law.
Pinned by: the spec-valid generators in `codec_prop_test.rs` and
`import_prop_test.rs` (they never produce such a block, so no law
pins the invalid case).
Blocks: nothing. C-TRAN has no such block.

### D-12 — What headsign does a new trip get?
State: Encoded provisionally.
Context: a trip's headsign is its own (`Trip::headsign`), and the
export writes it as is — a feed trip published without one stays
without one (the whole-spec law forbids inventing it). A trip the
planner creates (`add_trip`, a headway or span gesture) has no
headsign, so an authored network publishes none for it; riders see
no sign text.
Options:
- **None (encoded).** The planner signs trips explicitly; there is
  no edit for it yet.
- Copy the headsign of a trip on the same pattern trips (or the
  pattern), when one exists.
- The direction's name, as the export did before trips carried
  their own headsign.
Pinned by: `spec_round_trip_test.rs` (`trips_survive`) pins only
that nothing is invented on export.
Blocks: nothing.

### D-44 — Is a branch of a direction its own thing?
State: Unanswered.
Context: QAF-63. GTFS direction_id groups branches, not only
variants: C-TRAN's 006 direction 1 holds a Fruit Valley pattern and
a VA Medical Center pattern leaving at the same minutes all day,
and the import puts both in one direction, named by the majority
headsign. A planner reads them as two branches; the timetable shows
one direction with two variants (D-22).
Options:
- The model stays as it is: a planner who wants two branches
  authors them (set_direction + update_pattern). The finding becomes
  documentation.
- The import splits a direction whose patterns end at different
  terminals into one direction per terminal.
- The model names branches inside a direction (a pattern group with
  its own name and timetable), and the timetable can show one branch.
Blocks: QAF-63.

## Analysis and live

### D-6 — Does "frequent" count terminal arrivals?
State: Unanswered. The current answer is pinned as a finding, not
as a law.
Context: QAF-78. The whole-day "frequent" test counts every visit
to a stop, arrivals included, so a terminal where nobody boards can
read as frequent. The windowed test counts departures only.
Options: departures only, for both tests (the finding reads this
way); or keep counting arrivals and document the difference.
Blocks: the fix for QAF-78, and the `frequent_pct` number in
`transit-analysis` `regression_test`.

### D-43 — What does a coverage comparison report?
State: Unanswered.
Context: QAF-20. Comparing two plans' coverage takes two
type_coverage answers and a host subtraction, and a net change hides
churn: a proposal can serve 2,000 new people and drop 1,900 others
for a net +100.
Options:
- Per service type (matched by id across the two documents): both
  documents' served and frequent shares, and the population gained
  and lost — served and frequent separately.
- The same per representative DATE instead of per service type
  (a fork that renames or splits a type still compares).
- Net change only (the two shares and their difference).
Blocks: QAF-20 (the coverage half).

### D-47 — Which layers does coverage take?
State: Unanswered.
Context: QAF-18 and QAF-28. Coverage takes one weighted POINT layer
and calls the weight population. A tract's people sit at its
centroid, so a rural tract reads unserved where its developed part
has service, and the radius swings the answer by 20 points (28.9%
at 500 m, 49.1% at 800 m). Jobs, seniors, and zero-car households
are the usual companions; no such layer exists in the repo.
Options:
- Areal layers: polygons with a weight, served share by the share of
  the area within the radius (block groups or blocks for population).
- Point layers only, at a finer resolution (block centroids), with
  the resolution stated in the answer.
- Either, and the vocabulary becomes "a weighted layer" (any field:
  population, jobs, households).
Blocks: QAF-18, QAF-28.

## Service types

### D-8 — Do timetable edits act on the whole service family?
State: Unanswered.
Context: QAF-65 and QAF-44. The reads pool a service family (a
base service and the siblings that run on its days, such as 1-WKDY
with /30 and /31), but the timetable gestures key on one service
type. A `set_headway` under 1-WKDY retimes only the base's trips.
Options:
- Gestures act on the family. The trips stay under their own
  service ids.
- Gestures stay per service type, and the UI warns when the family
  has siblings.
- An authored "same as, except" relation between service types
  (QAF-44), which the gestures follow. This is a model change.
Blocks: the remaining QAF-65 work (the gestures) and QAF-44.

### D-41 — What carries a service type's calendar into a fork's period?
State: Unanswered.
Context: QAF-54. A fork picked after the import's calendar end
(2027-01-09) runs nothing until every service type's date range is
extended by hand — 13 set_service_range edits on C-TRAN. Holiday
types (4-NYE and others) are dated exceptions, not ranges.
Options:
- A gesture, extend_service_ranges {through}: every weekly service
  type whose last range ends before `through` ends at `through`;
  dated-only types are untouched.
- The fork itself (set_plan with a period end) extends every weekly
  type to the period's end.
- No gesture: the warning names the problem, the planner extends by
  hand.
Open inside the first two: which date the period ends on, and
whether next year's holidays (new dated exceptions) are part of it.
Blocks: QAF-54.

## Service design

### D-51 — What does a revenue hour, a revenue mile, and a vehicle cost?
State: Unanswered.
Context: `docs/units/ui-service-design.md` questions 2 and 18. The
ops answer revenue hours and miles, and the fleet answers peak
vehicles. A proposal's cost in dollars needs the agency's rates, and
the feed carries none. A rate the UI multiplies in is a policy value
in the host.
Options:
- A cost-model document on the bus (rates per revenue hour, per
  revenue mile, per peak vehicle, optionally per service group),
  read by the ops that answer cost.
- Rates as op parameters with no default: the host passes what the
  planner types, and no dollar figure appears until they do.
- No dollars: the UI answers hours, miles, and vehicles only.
Blocks: the cost line in the service-design questions 2 and 18.

### D-52 — How is a route-hour's frequency judged and drawn?
State: Encoded provisionally — the third option: the grid shades
continuously by the longest gap (full at 10 min or less, none at 75
or more — drawing, not a standard) and prints the longest and
shortest gap in the cell; no boundary judges an hour. Pinned by
`web/src/components/FrequencyGrid.svelte`; the op (`route_frequency`)
answers the gaps and carries no threshold, so a decided standard
becomes an op parameter without changing the answer.
Context: `docs/units/ui-service-design.md` question 4. The frequency
matrix shades each route-hour. The trips vary around a target on
purpose (traffic, layover, reliefs: the user, 2026-10-30), so an
hour has a typical gap, a shortest, and a longest, and no one
number. Which of them the shading reads, and any boundary on it,
is a threshold, so the op owns it, as a parameter with a default.
Coverage already takes `frequent_secs` (default 900).
Options:
- Shade by the longest gap (what a rider can wait), with standards
  as parameters (the frequent-network and base headways).
- Shade by the typical gap, and mark an hour whose longest gap
  breaks the standard.
- No boundaries: continuous shading of the longest gap, with the
  range printed in the cell.
Blocks: nothing (the grid's shading is provisional).

### D-53 — What ridership data does the planner bring?
State: Unanswered.
Context: `docs/units/ui-service-design.md` question 10. Productivity
(boardings per revenue hour) is a planner's first measure, and
nothing on the bus carries a boarding. The GTFS-Ride extension and
APC exports carry boardings per stop and per trip. Adherence history
is `docs/ROADMAP.md` 3.
Options:
- A ridership document on the bus (boardings and alightings per trip
  and stop, by date or averaged per service group), imported from
  GTFS-Ride or an APC export.
- A weighted point layer (boardings per stop), read the way coverage
  reads population (D-47).
- None for now: the question stays open as a data gap.
Blocks: the service-design question 10.

### D-54 — What does a frequency change keep of the trips already there?
State: Unanswered.
Context: QAF-90 and the user's direction of 2026-10-30, against rigid
timetables: a route's trips are adjusted around a target through the
day, for traffic, layover, and reliefs. `set_headway` moves every
one-start trip in the window to the nearest slot of an exact grid,
which erases those adjustments, and times new trips by the pattern's
one profile (D-3).
Options:
- A target with a tolerance: trips whose gaps already fall within
  the tolerance keep their times. Trips are added only where a gap
  exceeds target + tolerance and removed only where gaps fall short
  of target − tolerance, and a new trip goes in the middle of the
  gap it fills.
- Add and remove only: existing trips never move, and new trips
  split the longest gaps until the target is met.
- Keep the exact grid as one option among these, chosen per change.
Open in every option: which running time a new trip takes (D-3),
and whether a hand-nudged trip is ever moved by a later target.
Blocks: QAF-90, the service-design question 16 (the frequency
gesture).

### D-55 — Which members does a service group count on a day?
State: Unanswered.
Context: QAF-91 and QAF-65. A group pools a base service type and
the types whose calendars are subsets of it. C-TRAN's 1-WKDY/30 and
1-WKDY/31 are both subsets of 1-WKDY, but they run in successive
periods and never share a day, so pooling both on the representative
day counts trips that never run together (2130 against 1774).
Options:
- Count only the members that run on the group's representative day
  (a real day's service). A member that runs in another period shows
  as the group's other period.
- Split a group into periods wherever its members' date ranges
  change, each period with its own representative day and numbers.
- Count every member, as now, and say so in the answer (a total over
  the members' authored service, not a day).
Blocks: QAF-91 (every pooled read: service_groups,
service_type_detail, schedule_texture, the directory timetable).

## Blocks and runcutting

### D-9 — Where does required recovery time come from?
State: Unanswered.
Context: QAF-62. Imported layovers are the whole observed gap
between two trips, so any delay reads as infeasible. Separating
required recovery from slack needs a rule the feed does not carry.
Options: a policy document (a minimum in minutes, a percentage of
the run time, or both, per route or per service type); a value
derived from the feed (for example, the smallest layover seen at
that terminal); or both, with the policy winning.
Blocks: QAF-62 and QAF-26 (the recovery view is meaningless until
recovery is separated from slack).

### D-16 — Where does the runs layer live?
State: Encoded provisionally (in the docs; no code yet).
Context: runcutting grows into runs and bidding (the user's
decision; `docs/ROADMAP.md` 7). A run is cut from blocks, so the
layer could hang off `Block` or stand beside the network.
Options:
- **Beside the network (encoded).** Runs, the work-rule policy,
  and bids are their own documents and modules, reading the
  network by hash. This follows the laws: the network stays the
  core model, and one need per module.
- A `Block` extension: runs as fields on blocks in the network
  document. Every network consumer then carries operator work.
Pinned by: `docs/ROADMAP.md` 7, `docs/units/ui-runcut.md`.
Blocks: nothing.

### D-46 — How many more vehicles does a proposal need?
State: Unanswered.
Context: QAF-3. A proposal's added trips are unassigned (no block),
so fleet_summary cannot say how many more vehicles it needs. A
vehicle need is a block assignment, which the model does not make,
and an automatic one needs a minimum turn time (D-9).
Options:
- An automatic blocking op over a proposal (chain trips terminal to
  terminal under a minimum layover), answering the blocks it would
  make; the planner adopts them or not.
- A peak-vehicle estimate from concurrent trips plus a recovery
  allowance, with no blocks made.
- The planner blocks by hand (add_block_trip) and fleet_summary
  answers; the finding becomes a UI affordance.
Blocks: QAF-3 (and depends on D-9 for any automatic answer).

### D-50 — What are the work rules a run is cut under?
State: Unanswered.
Context: QAF-11, QAF-12, QAF-14, and the rest of QAF-15. The runs
layer (D-16: beside the network) needs a work-rule policy document,
and its shape is the agency's rules: maximum platform and spread,
report and pull times, minimum and maximum piece, meal breaks,
relief points, pay (guarantee, spread premium, overtime). The
hand-cut menu in QAF-11 assumed 8 h maximum platform, 15 + 15
report and pull, and a 2 h minimum piece.
Options:
- The user supplies C-TRAN's rules (or the labor agreement's
  section), and the policy document is shaped to them.
- A generic policy document (the common rules above, each optional)
  lands first, and C-TRAN's values are data in it.
Blocks: QAF-11, QAF-12, QAF-14, QAF-15 (the cut candidates).

## Riding today

### D-17 — What does "the last bus" mean with no deadline?
State: Encoded provisionally.
Context: the last bus home. `plan_arrive_by` without `arrive_by_secs` answers
the latest departure that still reaches the destination. Something
has to stand in for the missing deadline.
Options:
- **The service day's last arrival (encoded).** A pair a rider can
  walk between answers a walk that leaves at that moment minus the
  walk.
- Transit legs only: a walk never counts as the last way home.
Pinned by: transit-router `arrive_by_law_test` (law 4), cap-router
`arrive_by_answers_the_latest_departure`.
Blocks: nothing.

### D-18 — How do live delays meet a diverted trip?
State: Encoded provisionally.
Context: a live plan reads each trip at its predicted
times. The rt feed states delays against the PUBLISHED schedule, and
a diversion's `added_secs` also shifts the trip.
Options:
- **The rt delay wins (encoded).** A stated visit is the published
  time plus the delay, not also shifted by `added_secs`. A visit is
  matched by its scheduled departure, else by stop when the trip
  visits it once. A missing arrival (departure) delay takes the
  other's, and predictions are clamped so a trip never runs
  backwards.
- Both apply: the delay is read against the patched time.
Pinned by: transit-router `live_law_test`, cap-router
`a_compared_plan_reads_live_in_both_worlds`.
Blocks: nothing.

### D-19 — Does an alert for one direction reach both?
State: Encoded provisionally.
Context: a GTFS-rt informed entity may name a `direction_id`. The
model matches patterns, not GTFS directions (the diversion record
keeps `direction_id` for the boundary path only).
Options:
- **Not matched (encoded):** the alert reaches both directions of
  the route. Telling too many riders beats telling none.
- Resolve the GTFS direction to the route's model directions at the
  boundary and scope the alert to them.
Pinned by: transit-rt `alert_test` (`resolve_scope` ignores the
direction).
Blocks: nothing.

### D-20 — Which language does an alert speak?
State: Encoded provisionally.
Context: alert text is a `TranslatedString`. The live document
carries one string per field.
Options:
- **The untagged translation, else the first (encoded).**
- Carry every translation and let the reader's language choose.
  That choice would be a parameter on the board, never host logic.
Pinned by: transit-rt `alert_test`
(`an_alert_decodes_and_resolves_into_model_vocabulary`).
Blocks: nothing.

### D-21 — Where is a trip going when nothing signs it?
State: Encoded provisionally.
Context: a board row's or leg's destination. `visit_destination` reads the visit's headsign, then
the trip's. When neither is set, the rule falls back to the final
stop, and that stop may be a bay ("Van Mall Transit Center: Bay D").
Options:
- **The final stop's parent station, else its own name (encoded).**
- The final stop's own name, bay included.
Pinned by: transit-network `signs_law_test`
(`a_final_bay_is_named_by_its_station`), transit-dispatch
`a_row_is_going_where_its_sign_says`.
Blocks: nothing.

### D-22 — How is a route's rider variant named and ordered?
State: Encoded provisionally.
Context: `route_detail` groups a direction's patterns into the
variants a rider rides: the same places in order, a station's bays
one place (the user's rule — the per-bay patterns at a transit
center read as one route). Real differences stay apart (006's two
branches; 060's time-of-day stops). Two variants can share their
ends, and they need names a rider can tell apart.
Options:
- **Ordered by trips, every service type summed; named "first →
  last", then " via" the first place only it visits against the
  most-trips variant with the same ends, else ", skips" the first
  place of that variant it misses (encoded).**
- Ordered by today's trips, named by when they run ("weekday
  mornings") — needs the service day; a rider reads time of day.
Pinned by: transit-network `listing_test`
(`bay_variants_of_a_direction_read_as_one`).
Blocks: nothing.

### D-23 — In what order does a stop display list its groups?
State: Encoded provisionally.
Context: `grouped_board` answers one group per route and
destination. GTFS `route_sort_order` says routes with lower values
display first; it says nothing about routes without one, or about
one route's several destinations. C-TRAN sets it on every route.
Options:
- **`route_sort_order` ascending, routes without one after them by
  route id, then destination by name (encoded).** Stable: a group
  keeps its place as the day goes on, and an ended group stays put.
- Soonest departure first. The next bus is always on top, but the
  sign reshuffles every poll and a rider loses their line.
- Routes without a sort order first, or by public name instead of id.
Pinned by: `the_groups_list_in_display_order`
(`grouped_board_prop_test.rs`).
Blocks: nothing.

### D-24 — How does a departure's countdown round?
State: Encoded provisionally.
Context: the user set the display's time format: "Due" under a
minute, "N min" up to 59, then the clock time. Whether 3 min 50 s
reads "3 min" or "4 min" was left open. Every board row carries it
(`wait`).
Options:
- **Round down (encoded).** 3 min 50 s reads "3 min" — a rider
  hurries rather than misses a bus (an early departure is a missed
  bus).
- Round to the nearest minute. Closer on average; a rider can read
  "4 min" for a bus 3 min 31 s away.
Pinned by: `every_row_says_how_long_until_it_leaves` and
`the_wait_turns_at_a_minute_and_an_hour`
(`grouped_board_prop_test.rs`).
Blocks: nothing.

### D-40 — How long does a planned trip take, and from when?
State: Unanswered.
Context: QAF-81. An itinerary carries depart_secs (the moment the
plan was asked for) and arrival_secs; a walk leg carries only its
seconds, so when the rider must leave is host arithmetic, and so is
"takes 23 min". A board row already says how long until it leaves
(`wait`, D-24).
Options:
- Door to door: `leave_secs` (the first ride's boarding minus the
  walk before it — when the rider must set out) and `duration_secs`
  (arrival minus leave), plus the board's `wait` rule for "leave in
  N min".
- From the moment asked: one `duration_secs` = arrival minus
  depart_secs (it counts the wait before leaving).
- Both readings, named.
Blocks: QAF-81.

### D-42 — When a route has stopped for the day, how far ahead does a stop look?
State: Unanswered.
Context: QAF-82. grouped_board keeps a route and destination whose
last departure has left, with no departures. The display says "No
more service today" where a rider wants "Next: 5:12 am tomorrow" or
"Next: Monday". The next one is on another service day, which the
board does not ask.
Options:
- Look ahead day by day for up to a week (one weekly cycle), with
  each day's in-force edits; say nothing past a week.
- Tomorrow only.
- Until the calendar ends.
Open inside each: whether a later day's answer reads that day's
edits (yes, if they are in the document) and never the live feed.
Blocks: QAF-82.

### D-48 — Does a stop answer to a name its sign does not carry?
State: Unanswered.
Context: QAF-70. directory `stops` matches the feed's names, so
"vancouver mall" finds nothing (the sign says "Van Mall"). Where the
server runs a geocoder (D-36), the everyday name finds the place and
the stops near it; without one it has no answer.
Options:
- Stops carry authored aliases in the network (a stop's other
  names), and `stops` matches them.
- The geocoder is the answer; a deployment without one says how
  stops are named (as the UI does now). The finding closes as
  documentation.
- Fuzzy matching on the stop names (typos, abbreviations like
  "Van" for "Vancouver").
Blocks: QAF-70.

## Public comparison

### D-14 — Does the proposal comparison collect public responses?
State: Unanswered.
Context: the comparison UI reads published proposals. Collecting
comments or votes on them would add a new shared document, synced
through the server, with its own retention and moderation
questions.
Options:
- Read-only. Responses go through the agency's existing channels,
  and the UI links to them.
- A response document on the bus (a new document kind, synced by
  LWW per record like the edit document).
Blocks: question 6 of `docs/units/ui-proposal-compare.md`.

## Live operations

### D-15 — What makes a trip "covered"?
State: Unanswered.
Context: the disruptions UI asks which trips are covered and which
are uncovered. The bus knows vehicles only through blocks and live
positions. It holds no operator assignments (QAF-17), and the edit
document cannot cancel or cover a trip (QAF-36).
Options:
- A trip is covered when a vehicle is on its block: from the
  blocks and the live positions, with cancel and cover as new edit
  kinds (QAF-36).
- A trip is covered when an operator holds its run. This needs the
  runs layer (`docs/ROADMAP.md` 7) and workforce data.
Blocks: question 6 of `docs/units/ui-disruptions.md`.

### D-45 — Which dispatcher edits does the edit document carry beyond the diversion?
State: Unanswered.
Context: QAF-36. The edit document holds diversions only; a
dispatcher's first answer to a 24-minute-late trip (cancel it,
cover it, hold a connection, swap the vehicle) has no vocabulary. A
new edit kind is a sync-document change, and each kind needs a
meaning on every reader: does a cancelled trip leave the board or
show as cancelled, does the router skip it, does the live feed's
own cancellation (SCHEDULED/CANCELED in GTFS-rt) agree with it.
D-15 ("covered") depends on the answer.
Options:
- Cancel (whole trip) first; a cancelled trip shows on boards as
  cancelled and the router never rides it.
- Cancel and partial cancel (from a stop on).
- Cancel, cover (another block or vehicle takes it), hold, swap —
  the full set, landed together.
Blocks: QAF-36, and with it D-15.

## Geography

### D-25 — Which streets are the source of truth?
State: Decided.
Answer: 2026-10-29 — "it should use the streets document model you
just built. that model can get data from multiple sources." Walking
and every other street op read a streets document by hash; which
source fills it is the host's choice. On C-TRAN's Trapeze roads not
carrying walking: "that is something to be solved in the data, not
in the model or the logic … a problem for me/the user to solve" —
walking wants an OSM-sourced document (QAF-85).
Context: the streets document (`docs/units/streets-model.md`) is
imported from a street layer; the import reads the agency's
Trapeze roads export, TIGER, and OSM GeoJSON alike. C-TRAN's
Trapeze roads (118,860 features) are what the agency's scheduling
system routes on, so deadheads computed over them agree with it.
They carry no sidewalks, paths, or stairs (no CFCC A7x feature),
so walking over them treats every road as walkable and misses
every cut-through. OSM carries footways and crossings, at a
licence cost: the ODbL's share-alike applies to a streets document
derived from OSM and distributed.
Options:
- One source for every mode: the Trapeze roads (agency parity,
  no walking network), or OSM (walking detail, no parity).
- Two documents: the Trapeze roads for driving (drawing, detours,
  deadheads) and OSM for walking (access, transfers, walksheds).
  Every op that takes streets then names which.
- **Decided:** the model takes any source; the source is data.
Blocks: nothing.

### D-26 — Which streets can each mode use?
State: Decided for walking; unanswered for a bus.
Answer: 2026-10-29 — walking: "Also refuse unknown" (a rider walks
every class but motorway, ramp, and unknown — strict); a rider walks
a one-way street both ways ("agreed"); a walk is at most ten minutes
by default, at 1.3 m/s, both op parameters. Pins:
`crates/transit-streets/tests/walk_law_test.rs`.
Context: the import classifies every street (D-30). A rider cannot
walk a motorway or a ramp; a bus cannot be drawn down a path,
stairs, or a 4WD track, and a planner may not want one drawn down an
alley or a private drive. C-TRAN's data holds 1,701 streets with no
class (626 with a null CFCC, among them SE Powell Blvd, and 1,075
unnamed `X10` features), so a rule must also say what an unknown
class allows.
Options: a mode profile per class, owned by the op as a parameter
with a default (a walking profile, a bus profile). The defaults
are the decision: which classes each mode may use, and whether an
unknown class is allowed (permissive) or refused (strict).
- **Decided (walking):** strict — every class but motorway, ramp,
  and unknown; one-way does not bind a rider.
- Open (a bus): which classes a bus may be drawn on, and whether an
  unknown class is allowed.
Blocks: drawing on streets in `docs/units/streets-model.md`.

### D-27 — What does place search find?
State: Decided.
Answer: 2026-10-29 — "i can run a pelias container for this project
and have it wire to ride-now" … "wire up pelias with the portland
metro example". Search finds stops (directory `stops`) and whatever
the geocoder answers: Pelias's portland-metro project — addresses
(OpenAddresses, TIGER interpolation), streets and venues (OSM),
and places (Who's On First). Which sources is the geocoder's
configuration, not the model's (D-36).
Context: search found stop names only (directory `stops`). A rider
types addresses ("1400 NE 136th Ave"), intersections ("4th Plain
& Andresen"), and places ("PeaceHealth", "WSU Vancouver"). The
Trapeze roads carry TIGER-style address ranges per side
(`L_F_ADD`…`R_T_ADD`) and street names, so addresses and
intersections need no new source; places do (OSM, or the county's
data), and they are the hardest to keep current.
Options: stops only; + intersections; + addresses
(interpolated along the ranges, or the county's address points);
+ places. **Decided:** stops, and every kind the geocoder answers.
Blocks: nothing.

### D-28 — Where do stop and address results meet?
State: Encoded provisionally.
Context: a rider wants one search box. The host may not merge two
answers (`docs/ARCHITECTURE.md`, "Generated UIs", step 6). With the
geocoder a service (D-36), the stops are an op's answer and the
addresses the geocoder's; neither knows the other.
Options:
- **Encoded:** one box, two lists — the directory's stops, then the
  geocoder's addresses and places under their own heading, each in
  the order its source gave; the arrow keys move through both. The
  "no stop is named" hint shows only when neither answered anything.
- Rank them together: the geocoder's candidates go into an op as
  input (like an rt snapshot) and a capability orders both — the
  oracle promoted to a source (D-38).
- Two boxes (stops, addresses).
Blocks: nothing. Pins: `web/e2e/published-network.spec.mjs`, "ride
now searches addresses through the geocoder".

### D-29 — What area does a geography release cover?
State: Unanswered.
Context: a geography release pins the basemap and the streets
document, built from one source snapshot
(`docs/units/streets-model.md`). C-TRAN runs into Portland. The
Trapeze export already reaches Portland (its first feature is NE
21st Ave, Portland); the local basemap is one extract.
Options: the service area plus a buffer (how far?); the counties
served (Clark, plus Multnomah for the Portland trips); whatever
the agency's GIS exports.
Blocks: the geography release in `docs/units/streets-model.md`.

### D-30 — What does the streets import call a street, and what class is it?
State: Encoded provisionally.
Context: street layers mix roads with other lines. C-TRAN's
Trapeze roads hold 1,673 + 1,388 railroads (CFCC `B10`, `B20`) and
1,160 + 1,898 + 81 water lines (`H10`, `H20`, `H30`: shorelines,
the Washougal River). Before the streets document, every one of
them was welded into the routing graph. The import must say which
lines are streets, and give each a class the walking profile and the bus profile (D-26)
can read. The class field is read from the layer's own scheme:
TIGER's CFCC (Trapeze and older TIGER), TIGER's MTFCC, OSM's
`highway`.
Options:
- **Encoded:** a line whose class field names something other than
  a road is not a street: a CFCC in a non-road category (`B`
  railroads, `C` other transport, `D` landmarks, `E` physical
  features, `F` boundaries, `H` water), an MTFCC outside `S1`, an
  OSM line with no `highway` but a `railway`, `waterway`,
  `natural`, `power`, or `boundary` tag. A line with no class
  field, or with a road class the import does not know (CFCC `X10`
  "not yet classified", a null CFCC, CFCC `A65` ferries, MTFCC
  `S1750`, OSM `construction` and `proposed`), is a street of
  class `unknown`, and the walking and bus profiles (D-26) decide what it
  allows. The class field is read in the order CFCC, MTFCC,
  `highway`; the name in the order `FULLNAME`, `name`, `NAME` (the
  Trapeze `NAME` is the bare street name, "21ST" for NE 21st
  Ave). The classes: `motorway` (CFCC A1, MTFCC S1100, OSM
  motorway), `primary` (A2, S1200, trunk, primary), `secondary`
  (A3, secondary, tertiary), `local` (A4, A60–A62, S1400,
  residential, unclassified, living_street, road), `ramp` (A63,
  S1630, any `*_link`), `service` (A64, A73, A74, S1640, S1730,
  S1740, S1780, service), `track` (A5, S1500, track), `path` (A70,
  A71, S1710, S1820, S1830, footway, path, pedestrian, cycleway,
  bridleway), `steps` (A72, S1720, steps).
- Keep every line, classed `other` — the graph then needs the mode
  profiles (D-26) before it is safe to route on.
Blocks: nothing. Pins: `crates/transit-streets/tests/import_law_test.rs`.

### D-31 — Can a plan add a street the data does not have?
State: Unanswered.
Context: a service change often serves streets that do not exist
yet (a subdivision under construction, a new connector). Drawing a
pattern on streets (`docs/units/streets-model.md`) can only follow
streets the streets document has. A pattern's committed alignment
is geometry and never references the streets document, so a
pattern can always be drawn freehand.
Options:
- No: a planned street is drawn freehand on the pattern.
- A streets edit document beside the plan, applied to the imported
  streets the way network edits answer the next network. The
  streets document's ways then need identities that survive a
  re-import.
Blocks: planned streets in `docs/units/streets-model.md`.

### D-32 — How does the router walk with no walk table?
State: Encoded provisionally.
Context: the router's transfers walk the streets when a plan names
a walk table (`walks_hash`, minted by cap-streets `walks` from a
streets document and the network). A plan with none still needs
transfers: the fixture feeds, a host with no streets, a plan
document drawn where no streets are loaded.
Options:
- **Encoded:** with no walk table the router keeps its straight-line
  transfers (every stop pair within 300 m, at 1.3 m/s) — two answers
  to "how long is this walk" on the bus, one of them crow-flies.
- Require a walk table on every plan: one answer, and every host
  that plans must assemble cap-streets and a streets document.
- No transfers on foot without a walk table.
Blocks: nothing. Pins: `crates/transit-router/tests/walks_law_test.rs`.

### D-33 — How does a trip from a place walk?
State: Encoded provisionally.
Context: a plan's end can be a place off the network (a tapped
spot, the rider's location), named by its walks to the stops near
it: cap-streets `access` answers them along the streets, each
within the walk limit (ten minutes, D-26), and the walk straight to
a second point (the plan's `direct`). The router plans between ends. Three things the
intent leaves open:
- Whether the walk to the first stop may continue on foot to
  another stop before boarding (and the walk from the last stop be
  preceded by one). A stop plan already chains transfer walks.
- When walking straight there arrives as early as riding, which the
  rider is told to do.
- Whether a destination past the walk limit on foot is ever walked
  to directly.
Options:
- **Encoded:** a trip from a place is the place's walk to one of its
  stops, any stop plan (which may begin or end with transfer walks:
  a walk chain is one walk leg), and a stop's walk to the
  destination place — or the direct walk, which wins ties. Every
  single walk is within the limit; a chain of them may total more,
  as a stop plan's chains already do. The direct walk exists only
  within the limit.
- The place's walk is the whole walk at each end (no chain): a
  rider never walks more than the limit to reach the first bus.
- Ties go to riding.
- The direct walk is unbounded: a destination 20 minutes away on
  foot with no bus is still a plan.
Blocks: nothing. Pins:
`crates/transit-router/tests/places_law_test.rs` (laws 1–3 compose
the stop plan with chains; law 6 the tie), `walks_from` in
`crates/transit-streets/tests/walk_law_test.rs` (the limit).

### D-34 — Where does a stop no walk reaches go in the nearby list?
State: Encoded provisionally.
Context: with the point's walks along the streets (cap-streets
`access`, within the ten-minute limit), the directory's
`stops_near` ranks the places near the point by walk. Some places
are near as the crow flies and reached by no walk within the
limit — across a freeway, past the limit, or off every walkable
street.
Options:
- **Encoded:** the places a walk reaches come first, by walk (then
  distance, then id); the rest follow by distance, with no walk
  time. The walks reorder the places and nothing else: the same
  places answer within the radius and the limit.
- Leave the places no walk reaches out: the list says only where
  the rider can walk, and is empty off the streets.
- Rank every place by distance and show the walk where one exists.
Blocks: nothing. Pins:
`crates/transit-network/tests/stops_near_law_test.rs` (laws 6–8).

### D-35 — Among plans arriving together, where does the rider board and get off?
State: Encoded provisionally.
Context: the search answers the earliest arrival within the leg
bound, and its rides; it is indifferent to how far the rider walks.
On C-TRAN (2026-10-29, Jantzen Beach to Van Mall Dr & 72nd at 14:15)
it walked the rider 13 minutes up Broadway to route 032's first stop
at 19th St, when the same bus passes Evergreen & Broadway, a
four-minute walk, three minutes later — the same arrival. The user
called it out: closer stops serve the transfer.
Options:
- **Encoded:** the arrival and the rides are the search's; then the
  first ride is boarded, each transfer made, and the last ride left
  at the stops that walk least while still connecting (a tie keeps
  the search's stop). Each walk is the least for the stops around
  it — not a search for the least walking over every plan with that
  arrival.
- Least total walking over every earliest plan (a second criterion
  in the search itself: costlier, and it can trade a ride for a walk
  the encoded rule never does).
- Fewest minutes waiting instead: board as late on the trip as the
  rider can reach.
Blocks: nothing. Pins:
`crates/transit-router/tests/walk_least_law_test.rs` (laws 1–4).

### D-36 — Is geocoding a document on the bus or a service the host calls?
State: Decided.
Answer: 2026-10-29 — "basemap tiles are outside the data model, but
streets are a part of it. what about geocoding/reverse geocoding?
this feels like something that should just be a service i call to"
… "i can run a pelias container for this project and have it wire
to ride-now". The geocoder is an external service: the host asks
it, through the server's `/geocode` proxy, and every op takes the
point it answers.
Context: data belongs on the bus when an op's answer depends on it
— the streets do (walks, the walk table); basemap tiles do not.
A geocoder turns text into a point and a point into a name; from
the point on, every op answers the same whichever way the point
came (typed, tapped, located). The gazetteer that was queued in
`docs/units/streets-model.md` (a document read from the street
layers' names and address ranges) would have rebuilt a geocoder —
interpolation, fuzzy matching, ranking, places — inside a
capability.
Options:
- A gazetteer document on the bus: offline, the same answer on
  every host, and a geocoder to build and keep current.
- **Decided:** an external service (Pelias, `just geocoder`). Costs:
  no address search offline or where the server runs no geocoder
  (stop search, the map, and the location still answer); a rider's
  typed addresses reach whoever runs it (the agency's server, here);
  its answers drift as its data does, so nothing durable keeps a
  query — a trip end keeps the point and the name it was shown with.
  Stops stay the network's: the Pelias project's transit importer
  is not run.
Blocks: nothing. Pins: `server/geocode_test.go` (the proxy forwards
the query as-is, answers 404 with no geocoder),
`web/e2e/published-network.spec.mjs` ("ride now searches addresses
through the geocoder").

### D-37 — What names a spot dropped on the map?
State: Encoded provisionally.
Context: a spot tapped on the map (as a trip's end, or to see what
is near it) has a point and no name; the itinerary repeats its
name ("min walk to …"). The geocoder's reverse lookup names a
point; nearby, it may answer a street, an address, a venue, or a
town far off.
Options:
- **Encoded:** the nearest address, venue, or street the geocoder
  answers within 250 m, else "Dropped pin" (and with no geocoder).
  The name rides into the router's place as its label — an echo no
  op branches on (D-38). On the local
  Pelias (2026-10-29) a venue names a spot well at a transit center
  ("Vancouver Mall Transit Center", 2 m) and poorly elsewhere: a
  sign by the I-5 bridge ("Welcome to Washington", 39 m, where the
  street is East Fourth Plain Boulevard, 71 m) and a store
  ("Advance Auto Parts", where NE Andresen Road is 39 m).
- Always "Dropped pin": the itinerary never depends on the geocoder.
- The nearest of anything, at any distance.
- The nearest address or street only: always a street name, never
  a venue's — and never "Vancouver Mall Transit Center".
Blocks: nothing. Pins: `web/e2e/published-network.spec.mjs` ("an
address typed as a trip's end, and a dropped pin named by the
geocoder"; the no-geocoder specs keep "Dropped pin").

### D-38 — What may a host do with an external service's answer?
State: Decided.
Answer: 2026-10-29 — "makes sense to me", to the proposal: external
things are sources (their answer becomes a document on the bus),
presentation (pixels only), or oracles (an input the user could have
given by hand). A host asks an oracle with the user's input only,
renders its answer as given, and turns the pick into the user's
input (a point, the label shown); it never ranks, filters, merges,
thresholds, or joins the answers — answers that must meet model
data promote the oracle to a source. Ops echo an oracle's label,
never branch on it. The query policy is the deployment's (the
server's policy file, applied by its proxy); the manifest's
`services` lists what a deployment offers; nothing durable keeps a
query or an oracle's id. The rule's home is `docs/ARCHITECTURE.md`,
web section, "External services".
Context: the geocoder (D-36) was the first time a host showed an
answer that is not an op's. Its query policy (layers, radius, size)
was chosen in ride-now's TypeScript — a threshold in TypeScript,
which "Parity" forbids — and the page found it by probing for a
404, a third way of discovering a deployment's offer beside fixed
endpoints and `live-feeds.json`.
Options:
- Leave external services to each host's judgment.
- **Decided:** the three kinds and the oracle contract above.
Blocks: nothing. Pins: `server/geocode_test.go` (the page's own
policy is dropped and the deployment's applied; the manifest lists
the service exactly when configured), `web/e2e/published-network.
spec.mjs` ("ride now searches addresses through the geocoder": the
page sends the text and the focus point only), the `--ask` runner
gate in `just test`.

### D-39 — Do the live feeds move into the manifest's services?
State: Unanswered.
Context: D-38 makes the manifest's `services` the way a deployment
says what it offers. The deployment's GTFS-rt feeds are still named
in a static `web/public/live-feeds.json` beside the page (read by
ride-now and the stop display; the perf harness and its recorder
read the file too), and a rider may set their own. Moving them
moves where a deployment configures its feeds — server flags or a
file the server reads — and the page would read them from
`/manifest`, so a page served without the sync server has no
default feeds.
Options:
- Move them now: `services.live: {trips, vehicles, alerts}` from
  the server's configuration; `live-feeds.json` is retired.
- Wait for a second deployment to need it; `live-feeds.json` stays.
Blocks: live-feed discovery (ride-now's and the stop display's
`restoreUrls`, `web/perf/record-rt.mjs`).

### D-49 — Where do the basemap's glyphs and sprites come from?
State: Unanswered.
Context: QAF-84. With `basemap.pmtiles` served, the style still
fetches its fonts (glyphs) and icons (sprites) from
protomaps.github.io, so a stop display that loses its uplink loses
every map label. Installs and downloads are the user's job.
Options:
- Vendor them into the repo (the Protomaps font PBFs and sprite
  sheets, a few MB, OFL/BSD licensed), served beside the tiles.
- A recipe that fetches them once into web/public (like `just
  geocoder-build`), gitignored; the style reads them locally when
  present and falls back to the network.
- Keep the network fetch; the finding is documented as a known
  offline limit.
Blocks: QAF-84.

## Sync and persistence

### D-56 — Whose clock is the LWW clock?
State: Encoded provisionally.
Context: `transit_sync::merge` orders records by `updated_at`, stamped
by the writer — the client's clock for local edits, the server's
`now_ms` only for the submit transition. A device whose clock runs
fast mints a record that beats every honest record until real time
passes it, and skew can resurrect a retired record. Convergence needs
one clock, and the fix must stay idempotent: at-least-once redelivery
of the same document has to converge to the same hash.
Options:
- Client clocks as they are (the old behavior): broken under skew —
  a future-dated record wins indefinitely.
- **Clamp at receive (encoded).** The receive path sets
  `updated_at = min(updated_at, now_ms)` on every incoming record
  before merging; submit still stamps `now_ms`. A future-dated writer
  is treated as written now, client order survives within honest
  skew, and clamping a clamped value is a no-op, so redelivery stays
  idempotent.
- Re-stamp every record with the server clock at receive: strongest,
  but not idempotent — redelivering the same document would mint a
  new hash, breaking the converge-to-same-hash invariant.
Pinned by: `transit-sync` law test
`a_future_dated_record_is_clamped_at_receive`, cap-sync dispatch test
`sync_edits_clamps_the_client_clock`, and the store's clock-skew
session through a restarted server.
Blocks: the receive path of `docs/units/sync-persistence.md` (being
built on the encoded answer).

### D-57 — How long does a tombstone live?
State: Encoded provisionally.
Context: the edit document grows monotonically — a retirement is a
tombstone kept so deletions propagate through sync, so whole-document
sync never shrinks and every new device receives every retired
diversion. Dropping a tombstone can resurrect a record redelivered
after it, so the horizon must exceed the longest offline redelivery.
Options:
- Never compact (the old behavior): the document grows forever.
- **Compaction commit with a horizon (encoded).** The kind's
  `compact_edits` op drops records that are retired AND whose
  `updated_at` is older than `now_ms - horizon_ms`; the host runs it
  as an ordinary CAS'd commit, audited in the store log. Default
  horizon 90 days (`-retention-days`). With D-56's clamp a
  redelivered record can carry at most "now" as its stamp, so a
  resurrection needs a record older than the horizon arriving after
  compaction — accepted and documented.
- Compaction as a derived view (readers skip old tombstones): no
  resurrection risk, but the stored document and the delta log never
  shrink.
Pinned by: `transit-sync` law test
`compact_drops_only_retired_records_past_the_horizon`, cap-sync
`compact_edits` dispatch test, and the store GC test.
Blocks: the GC/retention section of `docs/units/sync-persistence.md`.

## Agency data

### D-10 — Agency questions on C-TRAN's data
State: Unanswered.
Context: two findings wait on facts only the agency has, not on a
design choice.
- QAF-9: should transfer inference fire on C-TRAN (station 106 and
  its bays all read `transfer: false`)? Or does C-TRAN mean its
  stations only as walking groups?
- QAF-48: vehicle labels are empty, so the late list shows ids
  like "CTRAN7003". Is that id what a dispatcher calls the bus? If
  not, what is?
Blocks: QAF-9 and QAF-48.
