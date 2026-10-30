//! gen-manifest: assemble the capability manifest after the wasm
//! builds (`just caps`) and PUBLISH the registry (`just registry`).
//! Reads each capability module, pins it by content hash, serves it
//! under that hash, and writes both surfaces hosts and agents read:
//!
//! * `web/public/caps/` — the browser's manifest + hash-addressed
//!   module files (the assembly the resolver loads);
//! * `registry/` — the REGISTRY: immutable blobs, one evidence record
//!   per capability (contracts + agent context + which gates ran when
//!   these exact bytes were published), and a registry pointer. An
//!   agent (or a human dogfooding `caps run`) pulls compute from
//!   here and verifies the bytes against the record.
//!
//! Before writing, the manifest is run through the compose-time
//! validator (cap_abi::validate): a manifest this crate assembles
//! must be provably coherent, and the generation is the first
//! consumer of that proof.

use std::fs;
use std::path::Path;
use std::process::Command;

use cap_abi::manifest::{CapManifest, Capability, MigrationSpec, OpDoc, OpExample};
use cap_abi::validate::validate_assembly;
use sha2::{Digest as _, Sha256};

/// The gates `just registry` runs before publishing (it says so with
/// `CAPS_PUBLISH_GATED=1`; a plain `just caps` publishes ungated and
/// the record says that instead). The recipe is the authority; the
/// record is the honest report of it.
const PUBLISH_GATES: &[&str] = &[
    "cargo test (workspace, includes the capability law suites)",
    "node crates/cap-abi/caps-smoke.mjs (ABI + composition + context)",
];

/// The document-kernel version this build speaks.
const KERNEL: u32 = 1;

/// The capability set this deployment assembles, mirroring the crates:
/// cap-gtfs produces feed and watchlist documents (and migrates the
/// watchlist); cap-router consumes feeds. When a crate's Request enum
/// changes, this changes — and the manifest diff is the reviewable
/// assembly change.
struct ModuleSpec {
    id: &'static str,
    file: &'static str,
    ops: &'static [&'static str],
    consumes: &'static [(&'static str, &'static str)],
    speaks: &'static [(&'static str, &'static [u32])],
    migrates: &'static [(&'static str, &'static [(u32, u32)])],
    /// Agent context: what this capability is FOR.
    purpose: &'static str,
    /// Per-op agent context, with worked examples (the feed hash is
    /// the REAL hash of the fixture this manifest ships, injected at
    /// generation time so examples are always current).
    op_docs: &'static [(&'static str, &'static str)],
}

const MODULES: &[ModuleSpec] = &[
    ModuleSpec {
        id: "gtfs",
        file: "cap_gtfs.wasm",
        ops: &["load_feed", "feed_info"],
        consumes: &[],
        speaks: &[("feed", &[1])],
        migrates: &[],
        purpose: "The FEED document: parse a GTFS static zip into a content-hash-keyed feed registry and answer the feed's own questions — what it contains at a glance (load_feed returns the summary) and its identity (feed_info.txt: version, publisher). The day views and the directory questions (pickers, names join) are NOT here: they answer over the NETWORK document (the board, runcut, calendar, and directory capabilities — one question, one answer on the bus). It has no ops that author anything.",
        op_docs: &[
            ("load_feed", "Register a GTFS zip under its content hash (idempotent). The response carries the feed's summary — stops, routes, trips, stop_times — the at-a-glance read the feed line shows. Bulk-bytes op 0; pass the raw zip to transit_invoke_bytes."),
            ("feed_info", "The feed's own metadata (version, publisher), when the feed provides feed_info.txt — the identity question: which feed is this?"),
        ],
    },
    ModuleSpec {
        id: "router",
        file: "cap_router.wasm",
        ops: &["load_network", "load_edits", "load_live", "load_walks", "plan", "plan_arrive_by", "plan_compare", "itinerary_paths"],
        consumes: &[("network", "network")],
        speaks: &[("network", &[1]), ("live", &[1]), ("walks", &[1])],
        migrates: &[],
        purpose: "Trip planning (RAPTOR) — how do I get from here to there — over the NETWORK document and the effective day (the model's materialized trips, patched with the edits in force — the same transit-dispatch composition every day view starts from), at the live document's predicted times when one is named. Its facets: leaving at a time (earliest arrival), arriving by a time (the latest departure; with no deadline, the last bus home), and the diversion delta (the unpatched day, one materialization apart, with the rider's notice). Assemble for users who need directions, not just departure times. Documents are hash-addressed; writes nothing.",
        op_docs: &[
            ("load_network", "Register the network document under its content hash (the same identity the network capability's import produces and every day view consumes; idempotent). Kernel bulk op 256 takes the bare document's JSON bytes on transit_invoke_bytes — the same answer, no request envelope; hosts send real-size networks this way."),
            ("load_edits", "Register the operator edit document against a loaded network, validated per record against it; the answer carries the reports and the document's hash. Wholesale replacement, like the host-owned store."),
            ("load_live", "Register a LIVE document (the rt capability's live answer) under its canonical content hash — the same identity rt answered; assert equality at the host. Plans name it by live_hash."),
            ("load_walks", "Register a WALK TABLE (the streets capability's walks answer: every walk between the network's stops along the streets) under its canonical content hash — the same identity streets answered; assert equality at the host. A plan naming it by walks_hash walks its transfers along the streets: its footpaths are the table's walks and nothing else. A table derived for another network is refused. Plans with no walks_hash walk straight lines (stops within 300 m, at 1.3 m/s)."),
            ("plan", "Earliest-arrival itinerary from `from` to `to` at a departure time over the materialized day. Each end is a stop id (a bare string) or a PLACE off the network — {id, name, walks: [{stop_id, walk_secs}]}, its walks to the stops near it (the streets capability's access answers them; a place id naming a stop is refused): the trip walks from the place to one of its stops, rides, and walks from one of its stops to the place, or walks straight there when `direct` ({walk_secs, path}: the walk between the two ends, when one joins them — the streets capability's access answers it) arrives as early; the itinerary and its first and last walk legs name the place by its id and name. The day: edits applied when edits_hash names a registered document; each trip instance at its PREDICTED times where the live document named by live_hash states them, schedule elsewhere; transfers along the streets when walks_hash names a registered walk table, straight lines otherwise. A null itinerary = unreachable within max_legs. The legs speak model vocabulary: the trip instance is addressed (trip, start_secs); a transit leg carries the route's short label (route_name) and long name (route_long_name, null when it has none), the headsign the bus shows at the boarding visit (the visit's own, else the trip's, else the final stop's place), and board_delay_secs / alight_delay_secs — how far a predicted time moved from the day's own (null = no prediction, the day's time)."),
            ("plan_arrive_by", "Arrive-by: the LATEST departure from `from` that reaches `to` by arrive_by_secs within max_legs, over the same day and the same ends as plan (edits_hash, live_hash, walks_hash; stops or places, direct). With no arrive_by_secs, the latest departure that reaches `to` at all on the service day — the last bus home (times past 86400 are after-midnight trips of the same day). The answer carries the deadline it answered (arrive_by_secs: the request's, else the day's last arrival) and the itinerary — the earliest-arrival plan leaving at that latest departure (its depart_secs) — or null when nothing arrives by the deadline."),
            ("plan_compare", "The diversion delta: the same plan (its ends stops or places, as plan's) over the patched day (active) and the unpatched materialization (baseline, null when no edit is in force on the date or no edits_hash is given), both at the live document's predictions when live_hash is given and walking the walk table named by walks_hash, with the rider's notice whenever the diversion changes the trip — {kind: unreachable (no active trip; the baseline has one) | no_trip (none within max_legs in either world) | later (the active trip arrives later: delay_secs, minutes_lost rounded up) | diverted (it rides a trip whose patched stop times differ from its unpatched instance), baseline_arrival_secs, routes and skipped_stops of the diverted trips the plans ride, text} — null when nothing changed. One op whether or not edits exist."),
            ("itinerary_paths", "Where an itinerary runs, one path per leg ({kind: transit | walk, path: [{lat, lon}]}) over the same effective day it was planned on (pass the same edits_hash): a transit leg's RIDDEN segment — its trip's line (trip_path's: the authored alignment, an in-force diversion's traced detour spliced between its kept stops) sliced between the board and alight stops when the trip visits both, its stops between them otherwise, a straight line as the last resort — and a walk through the stops it passes — and, given the plan's ends (from, to) and direct, a walk from or to a place along that place's walk path (between two places, direct's path); without them a place draws nothing. Send the itinerary plan answered. The map draws the paths; it computes none."),
        ],
    },
    ModuleSpec {
        id: "network",
        file: "cap_network.wasm",
        ops: &["load_feed", "import", "load_network", "validate", "export"],
        consumes: &[],
        speaks: &[("feed", &[1]), ("network", &[1]), ("boundary", &[1])],
        migrates: &[],
        purpose: "The network codec: read a GTFS feed into an authored network document (the planner's bootstrap — the agency's trips by id with their own times, patterns, service types, blocks; what cannot cross is reported) and export one back to GTFS zip bytes (the publish path). The second capability extracted from the monolith — the codec lives in the shared transit-codec library, and the monolith keeps no copy. Assemble for planners: it is how a planner's session starts and how it publishes.",
        op_docs: &[
            ("load_feed", "Register a GTFS zip under its content hash (idempotent). Bulk-bytes op 0; pass the raw zip to transit_invoke_bytes."),
            ("import", "The planner's bootstrap: read a loaded feed into an authored network document — the feed's trips ARE the document's trips (the agency's ids, every scheduled time, frequency trips as frequency trips), the feed's shapes ARE its alignments (the feed's ids, every point and published distance), patterns are stop sequences along an alignment. Deterministic; what the model cannot hold yet is reported in the notes — never silent. The boundary is the feed's serialization residue (its stop_sequence and shape_pt_sequence numbering, a frequency trip's stop_times base). The document registers itself with its boundary, so its hash answers validate/export immediately."),
            ("load_network", "Register an authored network document (the planner's JSON document) under its content hash; idempotent. The same registration every model-side capability names load_network — one op file feeds them all. Kernel bulk op 256 takes the bare document's JSON bytes on transit_invoke_bytes — the same answer, no request envelope; hosts send real-size networks this way."),
            ("validate", "Every issue of a registered document — the authoring UI's feedback loop, the whole list, never thrown. issues gate (export refuses an invalid document; a block trip its pattern trips do not run is one — the runcut and the publish path would disagree). warnings never gate: they name what validity lets through — trips no block carries (unassigned_trips, once the document has a runcut) and a versioned plan with no service in its first week (no_service_in_plan_period {plan_name, effective_from} — the plan by its name: the calendars a fork inherited end before its pick date)."),
            ("export", "The publish path: write a registered document as a GTFS zip (deterministic bytes) — every trip under its own id with its own times, the calendar as authored, with the import's boundary when it is the imported document itself — and return it base64-encoded under the content hash of those bytes, the same identity any capability returns for the same bytes. Invalid documents are refused."),
        ],
    },
    ModuleSpec {
        id: "rt",
        file: "cap_rt.wasm",
        ops: &["decode", "load_network", "load_boundary", "trip_progress", "late_trips", "live"],
        consumes: &[("network", "network")],
        // The gtfs-rt payload: an ephemeral document (polled every
        // few seconds), versioned by the protobuf schema it speaks;
        // it joins against the NETWORK (an rt trip id is a model trip
        // id), with the import's boundary for the published
        // stop_sequence numbering when there is one.
        // THE LIVE DOCUMENT is minted here and read by the board —
        // optional edges both: neither module needs the other to
        // answer its own questions, so they are spoken, not consumed.
        speaks: &[("gtfs_rt", &[1]), ("network", &[1]), ("boundary", &[1]), ("live", &[1])],
        migrates: &[],
        purpose: "Realtime decoding and the schedule join: turn a GTFS-rt protobuf payload (trip updates, vehicle positions, service alerts) into the flat view hosts show, and answer the dispatcher's joined questions — one trip's predicted times, the ranked what's-late-now view with the delayed-but-dark flag. The third capability extracted from the monolith; the domain code lives in the shared transit-rt library. It mints THE LIVE DOCUMENT: the snapshot resolved against the network into model vocabulary (delays per trip instance and stop, vehicles on their trip instances, statuses under the caller's on-time window, the service alerts in force with their informed entities resolved to routes, stops, and trip instances) — an rt trip id is a model trip id, resolved here once. Assemble for riders (live boards) and dispatchers (the operation view) alike.",
        op_docs: &[
            ("decode", "Decode a GTFS-rt protobuf payload into the overview (entity counts, trip updates with per-stop delays, vehicle positions, service alerts with their cause, effect, severity, text, periods, and informed entities in the feed's ids), and register the snapshot under its content hash — idempotent, the same bytes are the same snapshot — so trip_progress/late_trips can address it. Bulk-bytes op 0. Garbage bytes are an error outcome, never a trap."),
            ("load_network", "Register the network document the joins answer over, under its content hash; idempotent. Kernel bulk op 256 takes the bare document's JSON bytes."),
            ("load_boundary", "Register an imported network's boundary (the network capability's import answer) under its canonical content hash: the stop_sequence numbering the published feed — and so its rt updates — use. A network published from its own export needs none (1..n)."),
            ("trip_progress", "One trip's joined board: every scheduled stop with the live delay and the PREDICTED time (schedule + delay) — the dispatcher's most-asked question in one op. A stop the update does not mention carries the last stated delay upstream of it (the GTFS-rt propagation rule); stops before the first update stay null. The instance is the one the snapshot's update names (a frequency trip's by the update's start_time); updates pair with stops by the published stop_sequence (boundary_hash; 1..n without one), by stop id only when they name no sequence. Null when the snapshot has no update for the trip or the update names no instance of it."),
            ("live", "THE LIVE DOCUMENT: one rt moment resolved into model vocabulary. rt_hash (trip updates), vp_hash (vehicle positions), and/or alerts_hash (service alerts) — agencies often serve them separately; at least one — resolve against the registered network (an rt trip id is a model trip id; a frequency trip's instance is the update's start_time) into delays per trip instance ((trip, start_secs)) and stop (propagated downstream, with the predicted departure), vehicles on their trips, and statuses (early / on_time / late / very_late / no_prediction) classified under `window` — the caller's on-time policy, default: ANY earliness is early (early_secs 0 — a bus leaving before its time is a missed bus), up to 60 s late on time, past 300 s very late — which the document records so every reader classifies the same way. Trip and vehicle lateness is the trip's worst stated delay. Unresolvable rt trips are counted (unmatched_trips), never silently dropped. alerts: the service alerts in force at the snapshot's moment (the header timestamp inside a communication period, else the deprecated active period; none = always), gathered from every snapshot given, one per entity id — each informed entity resolved to a scope (route, stop, trip, start_secs; every present field must match, all absent = the whole agency; a route_type names every route published under it; direction is not matched). An alert none of whose entities name anything in the network is counted (unmatched_alerts). The response also carries the summary a status panel reads: vehicles by status and the late list worst first (late_limit, default 15) — late BUSES when positions exist, late trips (predictions) when they do not. Register the document in the board with load_live."),
            ("late_trips", "The ranked what's-late-now view: every trip whose worst stop delay reaches min_delay_secs, worst first, each carrying the located vehicle (vehicle_id, label, position) — found on the trip itself or, failing that, on another trip of its block (position_trip_id names the trip the bus is actually running: a delay propagated to a block's next trip is a late bus, not a dark one) — and the DELAYED-BUT-DARK flag (has_position false) when no vehicle of the block is visible. vp_hash names the VehiclePositions snapshot (agencies serve it as a SEPARATE feed — C-TRAN does); omit it for combined feeds."),
        ],
    },
    ModuleSpec {
        id: "analysis",
        file: "cap_analysis.wasm",
        ops: &["load_network", "load_population_layer", "network_stats", "service_types_overview", "service_groups", "service_type_detail", "service_type_compare", "stop_gaps", "hourly_trips", "schedule_texture", "route_overlap", "timed_transfers", "hub_connections", "route_frequency", "type_coverage", "unserved_points"],
        // The network document is produced by the network capability —
        // the extraction this module's own docs once called "future".
        consumes: &[("network", "network")],
        speaks: &[("network", &[1])],
        migrates: &[],
        purpose: "Planner analysis over an authored network document: network/route/stop stats, per-service-type overview and detail, population coverage. The first capability extracted from the monolith — the domain code lives in the shared transit-analysis library. Assemble for planners; riders never download it. Documents are hash-addressed (load once, query by hash — never per-op document serialization).",
        op_docs: &[
            ("load_network", "Register an authored network document (the planner's JSON document, produced or derived by the network capability) under its content hash; idempotent. Kernel bulk op 256 takes the bare document's JSON bytes on transit_invoke_bytes — the same answer, no request envelope; hosts send real-size networks this way."),
            ("load_population_layer", "Register a GeoJSON population layer under its content hash. Bulk-bytes op 0; pop_field optional (common candidates tried). The answer is the import handle — points, the picked pop_field, total population — the layer's owner never parses a second copy."),
            ("network_stats", "The network's numbers on one service date: trips, spans, revenue hours AND revenue miles, transfer points, per-route and per-stop activity, headways. Each route row carries its own revenue_hours and revenue_miles (the per-route cost basis — every cost model is per hour AND per mile; average operating speed follows from the two) and its headway as the MEDIAN gap between departures at its busiest stop (headway_stop says which) — arrivals never count. Every route of the document is a row, sorted by id: a route with no service on the date answers trips 0 and null spans (inactive routes are analysis too — filter on trips). include_routes/include_stops/include_service_types (default true) shape the response: false empties the corresponding lists — the day-shape question answers in under a KiB."),
            ("service_types_overview", "Every service type's numbers, each on its representative day — the design axis, no calendar dates in sight."),
            ("service_groups", "The service groups: one row per BASE service type with the family that rides along (every type whose whole calendar is a subset of the base's — the overlays, 1-WKDY/30 and /31 and the rest, are really part of ONE weekday service; the agency's ids say so, the calendars prove it) and the family's POOLED numbers (trips, routes, stops, revenue hours and miles) over the members' authored service, the base's representative day as the label. The axis a planner designs; drill-downs key on the base. A rider-along type never heads a row (QAF-65)."),
            ("service_type_detail", "The SERVICE GROUP's drill-down, keyed by the base: the family that rides along (same weekly pattern, calendar subset) pooled over its authored service — routes, stops, and schedules with their trips, plus the family's totals. The route rows carry the FAMILY's headways (service ids key trips, never routes: the 164 lesson). Null for an unknown type."),
            ("service_type_compare", "What a change does: the proposal (network_hash) against its baseline (base_hash) under one service type, each on its own representative day — both sides' trips, revenue hours and miles, and routes, then only the routes whose numbers changed (trips, span, typical headway, revenue hours and miles, patterns, stops), each with its baseline and proposal row (null on the side that does not run it), and the count of unchanged routes. Both documents must be loaded."),
            ("stop_gaps", "One stop's day folded into headway gaps: departures, the max gap and the departure it follows, and the per-hour profile (departures, max and median gap starting in the hour — a gap belongs to the hour it starts in). Gaps run between consecutive DISTINCT departure times — two routes leaving together is no gap at all — and final-stop visits are arrivals, never departures. The service-quality read of \\\"how long until the next bus?\\\". An unknown stop answers zero/null/empty, never an error."),
            ("hourly_trips", "The day's service profile: trips per hour bucketed by trip start (the service-day hour, 0..47 — after-midnight starts are legitimate), each hour row carrying every running service type's count, sorted by type id. Only hours with trips are rows — where service is thick, where it tapers. The planner's texture question (QAF-4), answered from the materialized day, not a jq fold over the document."),
            ("schedule_texture", "The SERVICE GROUP's schedule texture on its representative day: the base service type plus every type that rides along with it (the SAME WEEKLY PATTERN — the overlays run the same kind of days as their 1-WKDY — and never a day the base does not; a Tuesday-only or Mon+Wed service is its own family). One row per PATTERN with the group's POOLED starts — keying by service id would slice a route's day into thin, wrong headways (route 164's weekday trips split across the family). Each row: the feeding schedule count, headway uniformity (min/median/max gap between distinct starts), and clockface alignment (starts on the quarter-hour grid, start % 900 == 0). Null for an unknown type (QAF-19)."),
            ("route_overlap", "The corridor read: every pair of routes that shares at least one stop on the date, sorted by (a, b) — shared stops, shared SEGMENTS (consecutive stop pairs both run, in the same order — the corridor proper), and the shared stop where the pair's departures stack highest (stack_stop, with the combined count; ties by lowest stop id). Where the Vine Green and Red run together, answered instead of inferred."),
            ("timed_transfers", "The hub's timed-transfer view: every arrival of one route followed by a departure of another within the connection window — the caller's ({min_secs, max_secs}, for a hub with widely separated bays), else the transfer-point convention (60–900 s: a rider needs a minute to move, and beyond 15 minutes it is not a timed transfer) — pooled over the stop's walking group (bays named: arrival_stop, departure_stop), sorted by arrival then routes. Who can make what at this hub, not just pooled departures. An unknown stop answers empty, never an error."),
            ("hub_connections", "The hub as a route-pair matrix: timed_transfers (the same window, the same walking group) folded by (from_route, to_route) — how many timed connections the pair makes on the date, the shortest and longest wait, and the median wait (null below four connections), pairs in route order; a pair with no connection is no row. Who connects to whom at this hub, and how long the wait runs. An unknown stop answers empty, never an error."),
            ("route_frequency", "The SERVICE GROUP's frequency grid: every route that departs under the group (the family service_type_detail pools, route id order), read at its headway stop (the most departures, lowest id on a tie — service_type_detail's headway_stop) hour by hour: every departure in the hour, and the shortest, longest and median gap (null below four gaps) between distinct departure times starting in the hour — and the route's numbers under the group (stats: service_type_detail's row for it: trips, span, typical headway, revenue hours and miles). The gaps are the trips' own — a target adjusted through the day for traffic, layover and reliefs reads as the range it runs, never rounded to a grid. Null for an unknown type."),
            ("type_coverage", "Population coverage per service type against a hash-addressed layer: what share of the population each type serves, and serves frequently. Frequency is the whole-day average headway unless `window` ({start_secs, end_secs}) is given — then a stop is frequent when the worst wait anywhere in the window (edges included, departures only, every route pooled) fits frequent_secs: the planner's \"every 15 minutes, 07:00–19:00\". A midday or peak improvement is visible only through a window."),
            ("unserved_points", "The unserved WHERE of coverage: every layer point beyond radius_m of every stop with service on the date (the same served set type_coverage measures against), each with the nearest served stop and its distance — which part, and where, not just the share. Sorted by population descending, so the biggest unserved communities read first. Together with the coverage share they are complements: served + unserved conserves the layer total."),
        ],
    },
    ModuleSpec {
        id: "board",
        file: "cap_board.wasm",
        ops: &["load_network", "load_edits", "load_live", "stop_board", "grouped_board", "load_watchlist", "migrate_watchlist", "watchlist_board"],
        // The network document is produced by the network capability;
        // the operator edit document is host-owned (wholesale-replaced
        // — the sync contract); the watchlist is the PERSONAL document
        // this capability owns (versioned, migrated here).
        consumes: &[("network", "network")],
        speaks: &[("network", &[1]), ("operator_edits", &[1]), ("watchlist", &[1, 2]), ("live", &[1])],
        migrates: &[("watchlist", &[(1, 2)])],
        purpose: "The board: when does service leave this stop — at a stop (a station pin or bay with nothing of its own answers its pooled walking group), realtime-first over the live document the rt capability mints (its predictions and its service alerts), with what the in-force edits took away from the stop; the same stop grouped by route and destination for the display at it; or for a personal saved set (the watchlist document, the same question parameterized by a personal document). Every answer is the materialized day over the NETWORK document, patched with the in-force edits. Model-native end to end; the domain code lives in the shared transit-dispatch library. Assemble for anyone who reads a departure board.",
        op_docs: &[
            ("load_network", "Register the authored NETWORK document under its content hash (idempotent) — the same document identity the network capability's import produces and analysis consumes. Assert equality at the host: one document, one truth. Kernel bulk op 256 takes the bare document's JSON bytes on transit_invoke_bytes — the same answer, no request envelope; hosts send real-size networks this way."),
            ("load_edits", "Register the OPERATOR EDIT document against a loaded network, validated per record AGAINST THE NETWORK (model vocabulary — nothing is expanded to answer it). The response carries the reports (invalid records are REPORTED, never rejected — the host owns the document) and the document's content hash, which the day views address. Wholesale replacement: push the whole document on every change."),
            ("load_live", "Register a LIVE document (the rt capability's live answer) under its canonical content hash — the same identity rt answered; assert equality at the host."),
            ("stop_board", "THE board at one stop on a service date: the network's materialized day with the in-force edits (edits_hash), REALTIME-FIRST when live_hash names a registered live document — rows ordered by when the bus actually leaves, a late bus scheduled before from_secs that has not left is still on the board, an early one that already left is not, and each row carries expected_secs, delay_secs (against the board's own patched time), and status under the live document's window (no_prediction without live data). Each row names its route (route_name, route_long_name), the stop it leaves from (stop, stop_name — the bay, on a pooled board — and platform, the bay's own identifier, GTFS platform_code), and its destination as the sign says at that visit (the visit's headsign, else the trip's, else the final stop's station). A trip's final visit is an arrival, never a row. skipped: the scheduled departures the in-force edits took away from the stop, each with the diversion that skips it — why a detoured-away stop is empty. alerts: the live document's service alerts about the board (a row, a skipped visit, or the stop itself); each row and skipped visit names the ids about it. Each row says how long until it leaves (wait, from from_secs to when it actually leaves): due under a minute, minutes (rounded down) under an hour, else the clock time (secs). A stop with nothing of its own — nothing leaving, nothing skipped — that belongs to a walking group (a station pin) answers the group's board with pooled: true, each row naming its bay; a plain stop never pools."),
            ("grouped_board", "THE board grouped for the display at a stop: one group per route AND destination the stop is served by on the date — its whole day, so a group whose last departure has left stays on the board with departures: [] (the route has stopped here today; when it runs next has no op) — each holding its next per_group departures (default 3) by stop_board's realtime-first rule (the same rows, with wait, status, and the bay), and skipped: the visits the in-force edits took from that group, each with its diversion. Groups list in the route's display order (route_sort_order, lower first; routes without one after them, by id), then by destination, and carry the route's names and authored colors. alerts: the live document's alerts about the stop or any departure still to leave it. A stop with nothing of its own all day that belongs to a walking group (a station pin) pools the group, pooled: true, each row naming its bay. Uncut (a per_group as large as the day), the groups' rows together are stop_board's rows."),
            ("load_watchlist", "Register a watchlist document (the personal saved-set, version-tagged JSON) under its content hash."),
            ("migrate_watchlist", "Migrate a version-1 watchlist to version 2: label = note, colors dropped and reported. Pure and deterministic; the caller keeps the old bytes — migrations never destroy."),
            ("watchlist_board", "The board for a watchlist: each watched stop's board on a service date, labelled per the document's version — the same question the per-stop board answers, parameterized by the personal document, so the same answer: each stop's `board` is `stop_board` with the same edits (in force, skipped visits listed) and `live_hash` (realtime-first), and a watched station pin pools its bays."),
        ],
    },
    ModuleSpec {
        id: "runcut",
        file: "cap_runcut.wasm",
        ops: &["load_network", "load_edits", "load_live", "blocks", "fleet_summary", "block_trips", "trip_stops", "trip_path"],
        // The network document is produced by the network capability;
        // the operator edit document is host-owned (wholesale-replaced
        // — the sync contract); the live document is the rt
        // capability's, which the chain's ripple reads.
        consumes: &[("network", "network")],
        speaks: &[("network", &[1]), ("operator_edits", &[1]), ("live", &[1])],
        migrates: &[],
        purpose: "The runcut: what does this vehicle do today — the day's blocks list (which vehicles are out, with that day's running trip counts) and one block's authored chain with the layover ripple (a diverted trip ending later, or a trip the live document predicts ends late, eats into the vehicle's next layover). Blocks and layovers are model state GTFS never had. The materialized day over the NETWORK document, patched with the in-force edits; the domain code lives in the shared transit-dispatch library.",
        op_docs: &[
            ("load_network", "Register the authored NETWORK document under its content hash (idempotent) — the same document identity the network capability's import produces and analysis consumes. Assert equality at the host: one document, one truth. Kernel bulk op 256 takes the bare document's JSON bytes on transit_invoke_bytes — the same answer, no request envelope; hosts send real-size networks this way."),
            ("load_edits", "Register the OPERATOR EDIT document against a loaded network, validated per record AGAINST THE NETWORK (model vocabulary — nothing is expanded to answer it). The response carries the reports (invalid records are REPORTED, never rejected — the host owns the document) and the document's content hash, which the day views address. Wholesale replacement: push the whole document on every change."),
            ("load_live", "Register a LIVE document (the rt capability's live answer) under its canonical content hash — the same identity rt answered; assert equality at the host. block_trips reads it for the ripple."),
            ("blocks", "The day's blocks: every authored block with at least one trip running on the date, with that day's running trip counts — a block whose every trip is off is a vehicle still in the garage, not offered. With edits in force, skipped trips leave the count (and a block whose every trip is skipped leaves the list)."),
            ("fleet_summary", "The day's fleet in ONE answer: running blocks, the peak concurrent vehicles and the first time the peak is reached, and the platform / revenue / layover hours — the fold that used to take one block_trips call per block plus host-side concurrency glue. A vehicle is out from its block's first running start to its last running arrival (half-open spans: a bus arriving as another departs is one vehicle, not two). Platform = the fleet's out hours, revenue = the trips' own runtimes, layover = the rest; the peak is the dispatcher's fleet-need number. spans: each running block's outline in block order — its trips, first start, last end, span_secs, and its longest internal gap (longest_gap_secs, the wait between one trip's end and the next's start, from longest_gap_at_secs; null for a one-trip block) — the structure behind the totals and where a block might be cut."),
            ("block_trips", "One block's chain on a service date: the AUTHORED runcut (the model's blocks carry layovers GTFS never had), materialized for the day, with the in-force edits applied — a diverted trip ending later eats into the vehicle's next layover, and with live_hash so does a trip the live document predicts ends late: live_end_delay_secs is its delay at its last stop (arrival, else departure) against the published time, and a stated delay wins over the edits' added time. layover_remaining_secs is the authored layover minus that lateness, negative when the connection is eaten — will this late bus make its next trip? Each row's layover_health classifies the connection: overlap (negative — the next trip starts late), tight (under tight_layover_secs, default 300), ok. Trips of the chain that do not run on the date stay in view, marked. The answer carries the block_id it is for (many block_trips results group by the answer, never by request order). Unknown blocks answer empty, never error."),
            ("trip_stops", "One trip instance's stops ((trip, start_secs)) on a service date — the day's trip detail, EFFECTIVE (a diverted trip answers its patched sequence). The trip's line is trip_path. A trip that does not run answers empty, never an error."),
            ("trip_path", "One trip instance's LINE ((trip, start_secs)) on a service date ([{lat, lon}]), over the effective day: its pattern's authored alignment; a trip a diversion in force reroutes draws the alignment to the first kept stop, the diversion's traced detour, and the alignment from the last kept stop (a skip-only diversion keeps the alignment; with no alignment, its patched stops joined in order around the detour). A trip that does not run answers an empty path, never an error."),
        ],
    },
    ModuleSpec {
        id: "calendar",
        file: "cap_calendar.wasm",
        ops: &["load_network", "load_edits", "active_services", "service_dates"],
        // The network document is produced by the network capability;
        // the operator edit document is host-owned (wholesale-replaced
        // — the sync contract).
        consumes: &[("network", "network")],
        speaks: &[("network", &[1]), ("operator_edits", &[1])],
        migrates: &[],
        purpose: "The calendar: which service keys run on a date — often several at once (a regular pattern plus an exception-added service) — and its reverse, the dates one service type runs on. Both answers come from the EFFECTIVE day, not the calendar rules alone: a key whose every trip is skipped by edits, or was never representable in the model, is not running however its calendar reads. The materialized day over the NETWORK document; the domain code lives in the shared transit-dispatch library.",
        op_docs: &[
            ("load_network", "Register the authored NETWORK document under its content hash (idempotent) — the same document identity the network capability's import produces and analysis consumes. Assert equality at the host: one document, one truth. Kernel bulk op 256 takes the bare document's JSON bytes on transit_invoke_bytes — the same answer, no request envelope; hosts send real-size networks this way."),
            ("load_edits", "Register the OPERATOR EDIT document against a loaded network, validated per record AGAINST THE NETWORK (model vocabulary — nothing is expanded to answer it). The response carries the reports (invalid records are REPORTED, never rejected — the host owns the document) and the document's content hash, which the day views address. Wholesale replacement: push the whole document on every change."),
            ("active_services", "The service keys running on a date: the distinct service types the effective day's trips run under, sorted. Departures-only when edits_hash is omitted. The day's first question — what is running today?"),
            ("service_dates", "The dates one service type runs on within an inclusive YYYY-MM-DD window — the same question active_services answers in the other direction (\"when does 1-WKDY/31 run?\"). The effective day decides each date (edits applied when edits_hash names a registered document): a type whose every trip is skipped is not running, however its calendar reads. An unknown type or an inverted window answers empty, never an error."),
        ],
    },
    ModuleSpec {
        id: "directory",
        file: "cap_directory.wasm",
        ops: &["load_network", "stops", "stops_near", "stops_by_ids", "routes", "route_stop_sequence", "focus", "entities", "route_detail", "stop_detail", "timetable"],
        // The network document is produced by the network capability.
        consumes: &[("network", "network")],
        speaks: &[("network", &[1])],
        migrates: &[],
        purpose: "The directory: what exists in the network — the stops (with their walking-group structure; by name, or the places nearest a point), the routes (short and long names), one route's patterns with their ordered stops (list to detail). The pickers and the names join read here; they are MODEL questions, never feed reads. A route runs in directions, and each direction's stop sequences are its patterns (bay and short-turn variants), each with its own ordered stops — what a diversion editor anchors against. The planner's lists ask here too: rows of any kind narrowed by a filter, a route's composition with each pattern trips' trip count and its patterns by direction, a stop's serving routes, where an entity is on the map, and the timetable — one route, one direction, one service type, across the direction's patterns.",
        op_docs: &[
            ("load_network", "Register the authored NETWORK document under its content hash (idempotent) — the same document identity the network capability's import produces and analysis consumes. Assert equality at the host: one document, one truth. Kernel bulk op 256 takes the bare document's JSON bytes on transit_invoke_bytes — the same answer, no request envelope; hosts send real-size networks this way."),
            ("stops", "The stop list, name-sorted (the pickers' shape): every stop with its coords, its parent (the flag the station board keys off), and its transfer flag. query narrows it the way a picker's typing does — every whitespace-separated term must appear in the name or the stop id, case-insensitive (\"fisher landing\" finds Fisher's Landing and its bays). limit truncates after the sort; omit it for the whole list — a silently truncated directory would be a lie."),
            ("stops_near", "The stop list asked from a POINT ({lat, lon}): the places nearest it, by great-circle distance in meters (haversine), nearest first, ties by stop id — each row the stop's id, name, coords, parent and transfer flag (the stops row's fields) plus distance_m. With the point's walks ([{stop_id, walk_secs}] — the streets capability's access answers them, passed as they are), the places a walk reaches come first, by walk (then distance), each with walk_secs: the shortest walk to any of its members; the rest follow by distance with walk_secs null. A PLACE is what a rider walks to: a station's bays answer ONCE, as the station (the walking group the station board departs across), at the distance of its nearest member (the pin or any bay); a stop whose station the network does not hold is its own place. limit caps the rows (default 10); radius_m (meters) drops every place farther than it — omit it for the nearest limit, however far. A lat outside -90..90, a lon outside -180..180, or a negative radius is a request error."),
            ("stops_by_ids", "Stops by id, in the request's order — the names join hosts otherwise do by hand when a view (like rt's) carries ids a human reads as names. Unknown ids are omitted."),
            ("routes", "The route list, name-sorted: every route with its name (the short public name, else the long one), its long_name (\"Lincoln\"; null for a route without one), its mode (the model's closed set, not GTFS's numeric codes) and its authored color when there is one."),
            ("route_stop_sequence", "One route's patterns with their ordered stops (route_detail groups them by direction). An unknown route answers empty, never an error."),
            ("focus", "Where one entity is on the map ({kind: stop | route | pattern | pattern_trips | service_type | block, id}): its lines (each pattern's authored alignment, else its stops joined) and the stops it serves. A stop is its point; pattern trips their pattern; a route, a service type, or a block every pattern it runs. Null for an unknown id."),
            ("entities", "Every entity of one kind ({kind: stop | route | pattern | pattern_trips | service_type | block}), in id order, narrowed by an optional query the way a filter box's typing does (every whitespace-separated term in the id, the name, or a route's long name, case-insensitive — \"lin\" finds 002 Lincoln). Each row: id, name, context (a pattern's route; pattern trips' pattern · service type), transfer (stops), trips (pattern trips' materialized trip count; a block's chained trips), long_name (routes: the long name beside the short one, null without one)."),
            ("route_detail", "One route's composition: its patterns, the stops they visit (first-visit order), its pattern trips grouped by service type, each with the trips it runs (the same expansion the materialized day runs), and its patterns grouped by direction (the route's named directions in id order, then any patterns in none). Each direction also lists its VARIANTS, most trips first — the ways a rider rides it: patterns that visit the same places in the same order, a station's bays being one place (so patterns that differ only by the bay they use at a transit center are one variant); each with a name (\"first place → last place\", and \"via\" a place only it visits or \", skips\" one it does not when another variant shares its ends), its pattern ids, its places (the station or stop, and the stops used there), and its trips. Null for an unknown route."),
            ("timetable", "The planner's timetable (headed by route_name and route_long_name, null without one): one route, one `direction` (omitted or null: its patterns in no direction), one `service_type` as the base of a SERVICE FAMILY — every trip of the direction's patterns from every member of the family (service ids key trips, never routes), one row per trip by start with its identity (trip, start_secs), its pattern trips, pattern and block, its running time end to end (run_secs: the last stop's arrival less its start — each trip's own), and its departure at each column (null where its pattern does not stop); the columns are every stop those patterns visit, in route order, flagged when a timepoint; the legend lists the patterns most trips first, marking headway-band ones. Rows are the materialized day's trips. With `compare_hash` (a loaded baseline) a trip's id is its identity: a one-start trip both run at different starts carries `retimed_from_secs` (its baseline start — kept, never removed and added), a trip the baseline does not run is marked `added`, and `removed` lists the baseline's trips this one no longer runs; a frequency trip's instances share its id, so each is matched by (trip, start_secs). Null for an unknown route, direction or service type."),
            ("stop_detail", "One stop's composition: the routes whose patterns visit it (id, name and long_name rows, id order). A stop with service of its own answers its own routes — a bay never answers its siblings'; a stop no pattern visits (a station pin) answers its walking group's. Null for an unknown stop."),
        ],
    },
    ModuleSpec {
        id: "streets",
        file: "cap_streets.wasm",
        ops: &["import", "load_streets", "load_network", "walks", "access", "snap", "route"],
        // The streets document is the HOST's (read from a street
        // layer by import, answered with its hash); every query
        // addresses that hash. The walk table reads the network's
        // stops.
        consumes: &[("network", "network")],
        speaks: &[("streets", &[1]), ("network", &[1]), ("walks", &[1])],
        migrates: &[],
        purpose: "Where along the streets? Snap a point to the nearest street and route between two positions along the street graph, so a traced detour (or a pattern drawn the same way) is street geometry, never freehand chords — and how long a rider walks along them: between a network's stops (the walk table the router plans transfers over), and from any point to the stops near it (the ends of a trip from anywhere). The streets are a document: import reads a street layer (an agency GIS export, TIGER, OSM — GeoJSON lines) into the canonical streets document, whose hash is the streets' own, not the file's. The domain code lives in the shared transit-streets library; the module writes nothing.",
        op_docs: &[
            ("import", "Read a street layer — a GeoJSON FeatureCollection or Feature of LineString/MultiLineString geometries — into the canonical streets document and register it: the answer carries the document (the host keeps it), its streets_hash, and its ways, segments and nodes. Lines that are not streets (a CFCC railroad or water line, an MTFCC outside S1, an OSM railway or waterway) are left out; every street gets a class from its CFCC, MTFCC or OSM highway (unknown when none says), a name (FULLNAME, name, NAME), and its one-way (TIGER OneWay, OSM oneway — a one-way runs first point to last). The same streets in another feature order, digitized the other way, or with other properties are the same document and hash. Errors when the layer holds no street. Bulk-bytes op 0; pass the raw GeoJSON to transit_invoke_bytes."),
            ("load_streets", "Register a streets document the host holds (an import's answer, stored) under its hash — the same hash its import answered. Refused unless canonical (ways of two or more points, sorted, none twice, two-ways in canonical direction). Bulk-bytes op 1 takes the bare document's JSON bytes."),
            ("load_network", "Register the network document under its content hash (the same identity every model-side capability answers); the walk table reads its stops. Kernel bulk op 256 takes the bare document's JSON bytes."),
            ("walks", "The WALK TABLE: every walk between the network's stops along the streets, within max_walk_secs (default 600 — ten minutes) at speed_mps (default 1.3), as a document {streets_hash, network_hash, speed_mps, max_walk_secs, walks: [{from, to, walk_secs}]} in (from, to) order, symmetric — and its walks_hash (the router's load_walks answers the same). A rider walks every street class but motorway, ramp and unknown, both ways along one-way streets; a walk is each stop's walk to its nearest walkable street, the street distance, and the walk on to the other stop, rounded up to the second. A stop with no walkable street nearby walks nowhere. Which streets the document holds is the host's choice of source (walking wants one with real classes and footways)."),
            ("access", "The walks from a POINT ({lat, lon}: a tapped spot, the rider's location) to the network's stops within max_walk_secs (default 600) at speed_mps (default 1.3) — the walks a stop at the point would have in the walk table — nearest first, each {stop_id, walk_secs, path}: the path the rider walks, from the point to its street, along walkable streets, to the stop. With `to` (another point), direct is the walk from the point straight to it ({walk_secs, path}; null when none is within the limit or no `to` was given). The walks are a router PLACE's walks as they are (plan's {id, name, walks}: planning reads the seconds, itinerary_paths draws the paths), direct is plan's direct, and the walks rank the directory's stops_near. A point with no walkable street nearby walks nowhere (walks empty)."),
            ("snap", "The nearest street position to (lat, lon) — the projected point plus its graph address (segment, parameter along it), and how far the query point was. Segments are numbered through the document, way by way, so an address holds for one streets hash: a trace commits the geometry, never the address. Null when no street is nearby."),
            ("route", "The street geometry between two snapped positions: the shortest path along the graph, as points ending exactly at the target and starting AFTER the origin (traces chain legs — each leg's start is the previous leg's committed end; prepend `from` for a standalone trace), plus length_m: the path's length along the graph from the origin itself — \"how far by road?\" answered by the router, never a host-side sum. Empty points and null length when the positions sit on disconnected parts of the network — the caller falls back to a straight line."),
        ],
    },
    ModuleSpec {
        id: "sync",
        file: "cap_sync.wasm",
        ops: &["sync_edits"],
        consumes: &[],
        speaks: &[("operator_edits", &[1])],
        migrates: &[],
        purpose: "What is the converged state of this edit document? sync_edits: LWW merge per record id plus tombstones (the terminal authority is the agency scheduling system — propose, review, accept), the receive path clamping client clocks to the host-passed now_ms (D-56), then drafts transition to submitted; the answer carries the canonical document bytes and their hash, so a host stores exact bytes and never mints identity. read_edits migrates a received document at receive; compact_edits is the D-57 tombstone-retention commit. transit-sync is the domain library; the module is stateless — document storage is the host's (the -store, docs/FORMATS.md), and validation against the network is every model-side module's load_edits answer, never this one's.",
        op_docs: &[
            ("sync_edits", "Merge the client's edit document with the server's (LWW per record id — the newer updated_at wins, tombstones propagate), then transition drafts to submitted (rev bumped, now_ms stamped). The answer is the converged document; the client adopts it wholesale. The host owns both documents and the clock."),
        ],
    },
    ModuleSpec {
        id: "author",
        file: "cap_author.wasm",
        ops: &["load_network", "apply"],
        // Authoring reads and MINTS network documents — the same
        // identity the codec's import produces and every model-side
        // capability consumes.
        consumes: &[("network", "network")],
        speaks: &[("network", &[1])],
        migrates: &[],
        purpose: "Authoring: what does this network become after these edits? A change — retime a trip, add a stop, extend a calendar, fork the plan into the next pick — is a list of typed edits applied atomically to a registered NETWORK document. The answer is the next document under its content hash (registered: chain the next change on it), with its validation issues and warnings — the authoring loop in one op. The last document is never touched (the host wholesale-replaces its copy); removals refuse while anything references the entity. The edit vocabulary lives in transit-network. Assemble for anyone who changes a network: a planner's UI, a script, an agent.",
        op_docs: &[
            ("load_network", "Register the authored NETWORK document under its content hash (idempotent) — the same identity every model-side capability returns. Kernel bulk op 256 takes the bare document's JSON bytes on transit_invoke_bytes — the same answer, no request envelope; hosts send real-size networks this way."),
            ("apply", "Apply `edits` in order, atomically, to the registered document: set_plan (fork / stamp a pick — {plan:{id,name,effective_from}}), set_name, retime_trip {trip, to_secs} (move a one-start trip: it keeps its id, its running times, and its blocks — they name it by id), add_trip {pattern_trips, start_secs, id?} (a new trip timed by its pattern's profile covering the start; id minted {pattern_trips}@{start} when omitted, reported in `created`), remove_trip {trip} (gone everywhere: its pattern trips, emptied, go; its block closes up and goes when emptied), upsert_* / remove_* for stop, route, pattern, service_type, pattern_trips, block (upserts replace the whole entity under its id: read it from the document, change it, send it back), and the GESTURES — one edit per thing a planner does, each carrying its coherence rule: add_stop / add_route / add_pattern / add_service_type {from} / add_pattern_trips / add_block (id minted when omitted — first free s1, r1, pat1, cal1, svc1, b1 — and reported in `created`); update_stop / update_route / update_pattern / update_service_type / set_pattern_trips_refs (field-wise, references checked); set_direction {route, id, name} (add or rename one of a route's directions) / remove_direction (refused while a pattern runs in it), update_pattern {direction} (one of its route's directions, \"\" for none; a route move leaves it in none unless the same edit places it); insert_pattern_stop {index?} (before visit index, default the last stop, halfway along that leg, non-timepoint, unscheduled on every trip), remove_pattern_stop / toggle_timepoint (middle visits only; every timing profile and every trip's times stay aligned — a new timepoint takes its neighbours' midpoint in each profile, and each trip is scheduled there where it ran), set_pattern_stop, add_timing_profile / remove_timing_profile (one always stays) / set_timing_profile / set_timepoint_time; toggle_service_day {day: mon..sun}, add_service_range / set_service_range / remove_service_range, set_service_exception (one per date, date order) / remove_service_exception; set_band / add_band / remove_band {trip} (a frequency trip's bands; one always stays); add_block_trip {block, trip:{trip}} (in order of first start; a layover stays only while its two trips stay adjacent) / remove_block_trip / set_layover {after_trip, secs | null} (only between two trips). The TIMETABLE gestures address a route + direction (+ service type) — the directory's `timetable` — and apply across every pattern of the direction: set_headway {route, direction, service_type, from_secs, to_secs, headway_secs, pattern?} (frequency by period: a trip every headway from from_secs until before to_secs; one-start trips already in the window move to the nearest slots in order, keeping their ids, times, and blocks; open slots are new trips with fresh ids on the pattern the trip before runs, timed by its profile, unassigned; surplus trips go everywhere; frequency trips are left alone), set_span {route, direction, service_type, first_secs?, last_secs?, headway_secs?} (trim past a bound; extend to it at headway_secs, else the edge's median gap, on the edge trip's pattern), adjust_running_time {route, direction, service_type, from_secs, to_secs, delta_secs} (the direction's one-start trips under that service type departing in the window run delta_secs longer, negative shorter, in proportion along each trip — each keeps its id and start; other service types, frequency trips, and the profiles are untouched), insert_direction_stop {route, direction, stop, after} / remove_direction_stop {route, direction, stop} (by stop id on every pattern of the direction; endpoints stay). direction null or omitted means the route's patterns in no direction; pattern trips a gesture empties are removed and new ones minted ({service_type}:{pattern}) are reported in `created`; only one-start trips are moved. An out-of-range index or a missing reference is a named error, never a silent no-op. One edit that cannot apply (an unknown id; a removal of something still referenced — a stop a pattern visits, pattern trips a block carries) rejects the whole change as an error naming the edit's position. The answer: the next network_hash (already registered), the document itself, its issues (gate export) and warnings (never gate: unassigned trips, a pick with no service in its period), and `created` ({kind, id} per entity the change made)."),
        ],
    },
    ModuleSpec {
        id: "divert",
        file: "cap_divert.wasm",
        ops: &["load_network", "draft", "list", "propose", "retire", "restore", "sanitize"],
        // A draft is answered over the network the codec's import
        // produces; the answers are next states of the host-owned
        // operator edit document.
        consumes: &[("network", "network")],
        speaks: &[("network", &[1]), ("operator_edits", &[1])],
        migrates: &[],
        purpose: "Diversion authoring: what does this diversion draft amount to? A dispatcher's draft — a route, the pattern anchored against, two kept stops, the interior stops the detour still serves, a traced path, added minutes, dates — is answered over the NETWORK document (the route's patterns and the default, each stop's role, the skips in effect, the alignment the detour is drawn against, the traced path and its length, and the problems keeping it from being a record). propose, retire, and restore answer the edit document's next state under the envelope rules (rev bumped, host clock stamped; retirement is the tombstone); sanitize reads a stored edit document under the current schema. The host owns the documents, the clock, and the record ids. Converging two documents is the sync capability's question; validating records against the network is every day view's load_edits. Assemble for anyone who authors diversions: a dispatcher's UI, a script, an agent.",
        op_docs: &[
            ("load_network", "Register the authored NETWORK document under its content hash (idempotent) — the same identity every model-side capability returns. Kernel bulk op 256 takes the bare document's JSON bytes on transit_invoke_bytes — the same answer, no request envelope; hosts send real-size networks this way."),
            ("draft", "The editor's view of a draft: the route's patterns ({pattern, name, stop_count}), the pattern in effect (the draft's when it is the route's, else the longest), every visit with its role (first_kept / last_kept / interior / outside — the anchors are the first visit of the first kept stop and the first visit of the last kept stop after it) and whether it is skipped (an interior stop not in served_stops), the skips the record would carry, the pattern's alignment (authored path, else stop-to-stop), the traced slices flattened (repeated points dropped) with their length in metres, and problems (empty = ready). Total: a draft naming what the network lacks answers problems, never an error."),
            ("list", "The edit document's rows as the dispatcher's list shows them, in document order: route, anchor, and skipped-stop names resolved over the network (an id the network lacks shows as itself), status and in_force (retired is the tombstone), added seconds, whether a detour is drawn, the dates."),
            ("propose", "The edit document with the draft proposed as record_id: a new id appends a draft at rev 1, a known id is replaced whole with its rev bumped (its domain id kept); now_ms stamped. The record: the anchors, the skips in effect, the detour when the trace has two points or more, added_mins rounded to seconds, the dates. A draft with problems is refused, naming them."),
            ("retire", "The edit document with record_id retired — the tombstone (the record stays so sync carries the deletion), rev bumped, now_ms stamped. Retiring a retired record answers the same document. An unknown id is an error."),
            ("restore", "The edit document with record_id back in force as a draft (the next sync re-submits it), rev bumped, now_ms stamped. Restoring an in-force record answers the same document."),
            ("sanitize", "Read a stored edit document (a cache, a server response, a file): the records that parse under the current schema — defaults filled for fields added since they were written — and how many were dropped. Anything but an array is an empty document."),
        ],
    },
    ModuleSpec {
        id: "plans",
        file: "cap_plans.wasm",
        ops: &["resolve", "validate", "periods"],
        consumes: &[],
        speaks: &[("plan_set", &[1])],
        migrates: &[],
        purpose: "The service-plan chain: which plan governs a date. Service changes are PICKS — a few a year at defined dates — and the chain over the plan set's metadata is DERIVED, never stored: a pick's date is a target that moves while the pick is in development. resolve answers which pick is in force; periods windows every member; validate reports chain invariant violations. Unversioned plans are standalone: they never resolve, never flag. Pure ops over the metadata the host pushes per call.",
        op_docs: &[
            ("resolve", "Which plan is in force on the date: the latest pick at or before it, ties broken by id (deterministic, order-insensitive). Unversioned plans never resolve; no pick yet answers null."),
            ("validate", "Chain validity: unique plan ids, every versioned plan effective-dated, pick dates unique. Every violation is reported (kind-tagged); unversioned plans are standalone and never flagged."),
            ("periods", "Every chain member's governing period — [effective_from, until the next pick) — derived and sorted by pick date. The until end is computed, never stored."),
        ],
    },
];

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
    let target = root.join("target/wasm32-unknown-unknown/release");
    let dist = root.join("web/public/caps");

    // Hash-addressed files: the filename carries the content hash, so
    // the HTTP cache holds each capability build forever.
    let _ = fs::remove_dir_all(&dist);
    fs::create_dir_all(&dist).expect("create caps dir");

    // The demo feed: the e2e fixture, served statically so dev and
    // e2e run the identical demo — and hashed now, so the worked
    // examples in op_docs carry the REAL document identity.
    let fixture = root.join("web/e2e/fixtures/sample-gtfs.zip");
    let feed_bytes = fs::read(&fixture).expect("fixture exists");
    let feed_hex = format!("{:x}", Sha256::digest(&feed_bytes));
    fs::copy(&fixture, dist.join("sample-gtfs.zip")).expect("copy demo feed");
    let feed_hash = format!("sha256:{feed_hex}");

    // The network capability's worked examples are computed from the
    // REAL fixture: import derives the document, export produces the
    // zip — both through the shared transit-codec library, so the
    // examples are exactly what the module returns and cannot drift.
    let net_ctx = network_example_context(&feed_bytes);

    // The day-view runner gates' document: the load_network request
    // for the REAL fixture-derived network (the same bytes the
    // examples and the smoke use — the document cannot drift from
    // the module that owns it). Every JSON-document capability's
    // gate (board, runcut, calendar) chains from it.
    if let Some(ctx) = net_ctx.as_ref() {
        let request = serde_json::json!({ "op": "load_network", "network": ctx.derived });
        let json = serde_json::to_string_pretty(&request).expect("request serializes");
        fs::write(dist.join("sample-network-load.json"), json + "\n")
            .expect("write the dispatch sample document");
    }

    let mut capabilities = Vec::with_capacity(MODULES.len());
    for module in MODULES {
        let bytes = fs::read(target.join(module.file)).expect("wasm module (run the build first)");
        let hex = format!("{:x}", Sha256::digest(&bytes));
        fs::write(dist.join(format!("{}-{hex}.wasm", module.id)), &bytes).expect("write module");
        capabilities.push(Capability {
            id: module.id.into(),
            hash: format!("sha256:{hex}"),
            url: format!("caps/{}-{hex}.wasm", module.id),
            bytes: bytes.len() as u64,
            ops: module.ops.iter().map(|s| s.to_string()).collect(),
            consumes: module
                .consumes
                .iter()
                .map(|(kind, id)| ((*kind).to_string(), (*id).to_string()))
                .collect(),
            speaks: module
                .speaks
                .iter()
                .map(|(kind, versions)| {
                    ((*kind).to_string(), versions.iter().copied().collect::<Vec<_>>())
                })
                .collect(),
            migrates: module
                .migrates
                .iter()
                .map(|(kind, steps)| {
                    (
                        (*kind).to_string(),
                        steps
                            .iter()
                            .map(|&(from, to)| MigrationSpec { from, to })
                            .collect::<Vec<_>>(),
                    )
                })
                .collect(),
            purpose: module.purpose.into(),
            op_docs: module
                .op_docs
                .iter()
                .map(|(op, description)| {
                    let example = example_for(module.id, op, &feed_hash, net_ctx.as_ref());
                    let kinds = kinds_for(module.id, op, net_ctx.as_ref());
                    (
                        (*op).to_string(),
                        OpDoc { description: (*description).into(), example, kinds },
                    )
                })
                .collect(),
        });
        println!("cap {}: sha256:{hex} ({:.1} KiB)", module.id, bytes.len() as f64 / 1024.0);
    }

    let manifest = CapManifest { kernel: KERNEL, capabilities };

    // The compose-time proof, applied to our own assembly before it
    // is written: generation is the validator's first consumer.
    if let Err(issues) = validate_assembly(&manifest) {
        eprintln!("generated assembly is NOT valid:");
        for issue in issues {
            eprintln!("  [{}] {}: {}", issue.capability, issue.check, issue.detail);
        }
        std::process::exit(1);
    }

    let json = serde_json::to_string_pretty(&manifest).expect("manifest serializes");
    fs::write(dist.join("manifest.json"), json + "\n").expect("write manifest");

    publish_registry(&root, &manifest);

    println!("manifest: {} capabilities, kernel v{KERNEL}, assembly valid", MODULES.len());
}

/// Publish the REGISTRY: immutable blobs, one evidence record per
/// capability, and the pointer. The record says what these exact
/// bytes are, what they answer, what they speak — and WHICH GATES
/// the publish recipe ran against them, at which commit. That
/// record is what makes "known good" a checkable claim for an agent
/// (or a human dogfooding `caps run`) rather than a marketing one.
fn publish_registry(root: &Path, manifest: &CapManifest) {
    let registry = root.join("registry");
    let blobs = registry.join("blobs");
    let caps_dir = registry.join("capabilities");
    let _ = fs::remove_dir_all(&registry);
    fs::create_dir_all(&blobs).expect("create registry blobs");
    fs::create_dir_all(&caps_dir).expect("create registry records");

    // Provenance-style evidence: what built these bytes, from where.
    let run = |args: &[&str]| {
        Command::new(args[0])
            .args(&args[1..])
            .current_dir(root)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    };
    let commit = run(&["git", "rev-parse", "HEAD"]);
    let dirty = !run(&["git", "status", "--porcelain"]).is_empty();
    let rustc = run(&["rustc", "--version"]);
    let gated = std::env::var("CAPS_PUBLISH_GATED").is_ok_and(|v| v == "1");
    let evidence = serde_json::json!({
        "gates": if gated { PUBLISH_GATES } else { &[] },
        "recipe": if gated { "just registry" } else { "just caps" },
        "source_commit": commit,
        "dirty_tree": dirty,
        "source_digest": source_digest(root),
        "rustc": rustc,
    });

    for cap in &manifest.capabilities {
        let bytes = fs::read(root.join("web/public/caps").join(Path::new(&cap.url).file_name().unwrap()))
            .expect("module bytes (the manifest step wrote them)");
        let hex = cap.hash.strip_prefix("sha256:").expect("hash format");
        fs::write(blobs.join(format!("{hex}.wasm")), &bytes).expect("write blob");

        let record = serde_json::json!({
            "id": cap.id,
            "hash": cap.hash,
            "bytes": cap.bytes,
            "kernel": manifest.kernel,
            "ops": cap.ops,
            "consumes": cap.consumes,
            "speaks": cap.speaks,
            "migrates": cap.migrates,
            "purpose": cap.purpose,
            "op_docs": cap.op_docs,
            "blob": format!("blobs/{hex}.wasm"),
            "evidence": evidence,
        });
        let json = serde_json::to_string_pretty(&record).expect("record serializes");
        fs::write(caps_dir.join(format!("{}.json", cap.id)), json + "\n")
            .expect("write record");
    }

    // The pointer: which capabilities (at which hashes) this registry
    // currently names. Blobs are immutable; this file moves.
    let pointer = serde_json::json!({
        "kernel": manifest.kernel,
        "capabilities": manifest
            .capabilities
            .iter()
            .map(|c| serde_json::json!({ "id": c.id, "hash": c.hash }))
            .collect::<Vec<_>>(),
        "evidence": evidence,
    });
    let json = serde_json::to_string_pretty(&pointer).expect("pointer serializes");
    fs::write(registry.join("registry.json"), json + "\n").expect("write pointer");

    println!(
        "registry: {} capabilities published (blobs + records + pointer)",
        manifest.capabilities.len()
    );
}

/// The digest of the source the modules are built from — every file
/// git sees (tracked or not, ignored ones excluded) under `crates/`
/// plus the workspace `Cargo.toml` and `Cargo.lock`: sha256 over
/// `"{path}\n{sha256 hex of its bytes}\n"` per file, in path order.
/// `caps-run` recomputes it the same way and says when the registry
/// was published from another tree (QAF-77). `None` without git.
fn source_digest(root: &Path) -> Option<String> {
    let listing = Command::new("git")
        .args(["ls-files", "-co", "--exclude-standard", "-z", "--", "crates", "Cargo.toml", "Cargo.lock"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let mut paths: Vec<&str> =
        std::str::from_utf8(&listing.stdout).ok()?.split('\0').filter(|p| !p.is_empty()).collect();
    paths.sort_unstable();
    paths.dedup();
    let mut digest = Sha256::new();
    for path in paths {
        let Ok(bytes) = fs::read(root.join(path)) else { continue };
        digest.update(format!("{path}\n{:x}\n", Sha256::digest(&bytes)).as_bytes());
    }
    Some(format!("sha256:{:x}", digest.finalize()))
}

/// The network examples' real context: the fixture's derived
/// document (canonical hash — the module's formula), and its
/// exported zip (content hash of the deterministic bytes).
struct NetworkExampleContext {
    /// The typed derived document — the analysis examples compute
    /// real answers over it.
    network: transit_network::Network,
    derived: serde_json::Value,
    boundary: serde_json::Value,
    notes: serde_json::Value,
    network_hash: String,
    export_feed_hash: String,
    export_summary: serde_json::Value,
    zip_b64: String,
}

impl NetworkExampleContext {
    fn derived_network(&self) -> &transit_network::Network {
        &self.network
    }
}

fn network_example_context(feed_bytes: &[u8]) -> Option<NetworkExampleContext> {
    let outcome = transit_codec::import(&transit_codec::Feed::from_zip_bytes(feed_bytes).ok()?);
    let canonical = serde_json::to_string(&outcome.network).ok()?;
    let network_hash = format!("sha256:{:x}", Sha256::digest(canonical.as_bytes()));
    let derived = serde_json::to_value(&outcome.network).ok()?;
    let boundary = serde_json::to_value(&outcome.boundary).ok()?;
    let notes = serde_json::to_value(&outcome.notes).ok()?;
    let exported = transit_codec::export(&outcome.network, &outcome.boundary).ok()?;
    let export_summary = serde_json::to_value(transit_codec::FeedSummary {
        agencies: exported.agency.len(),
        routes: exported.routes.len(),
        stops: exported.stops.len(),
        trips: exported.trips.len(),
        stop_times: exported.stop_times.len(),
    })
    .ok()?;
    let zip = exported.to_zip_bytes();
    let export_feed_hash = format!("sha256:{:x}", Sha256::digest(&zip));
    let zip_b64 = transit_codec::base64_encode(&zip);
    Some(NetworkExampleContext {
        network: outcome.network,
        derived,
        boundary,
        notes,
        network_hash,
        export_feed_hash,
        export_summary,
        zip_b64,
    })
}

/// The parsed e2e fixture feed (for the gtfs capability's dispatch
/// read examples — real answers, same library the module uses).
fn fixture_gtfs() -> Option<transit_codec::Feed> {
    let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/../../web/e2e/fixtures/sample-gtfs.zip");
    let bytes = std::fs::read(fixture).ok()?;
    transit_codec::Feed::from_zip_bytes(&bytes).ok()
}

/// The street capability's examples' context: the e2e street
/// fixture imported through the shared library, with the Elbow
/// corner snap/route answers computed over the real graph.
struct StreetsExampleContext {
    streets: transit_streets::Streets,
    streets_hash: String,
    segments: u32,
    nodes: u32,
    pos_a: transit_streets::StreetPosition,
    pos_b: transit_streets::StreetPosition,
    points: Vec<transit_streets::StreetPoint>,
    length_m: Option<f64>,
}

fn streets_example_context() -> Option<StreetsExampleContext> {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../web/e2e/fixtures/streets.geojson"
    ))
    .ok()?;
    let streets = transit_streets::geojson::import(&bytes).ok()?;
    let streets_hash = cap_abi::canonical_hash(&streets).ok()?;
    let net = transit_streets::StreetNetwork::build(&streets);
    let (segments, nodes) = (net.segment_count(), net.node_count());
    let pos_a = net.snap(45.5040, -73.5030)?;
    let pos_b = net.snap(45.5080, -73.5080)?;
    let (points, length_m) = match net.route_with_length(pos_a, pos_b) {
        Some((points, length)) => (points, Some(length)),
        None => (vec![], None),
    };
    Some(StreetsExampleContext { streets, streets_hash, segments, nodes, pos_a, pos_b, points, length_m })
}

/// The rt join examples' context: the fixture network (imported
/// through the codec, with its boundary) and the e2e rt payload, both
/// through the shared libraries.
struct RtExampleContext {
    network: transit_network::Network,
    network_hash: String,
    boundary: transit_codec::Boundary,
    boundary_hash: String,
    overview: transit_rt::RtOverview,
    rt_hash: String,
}

fn rt_example_context() -> Option<RtExampleContext> {
    let outcome = transit_codec::import(&fixture_gtfs()?);
    let payload = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../web/e2e/fixtures/sample-rt.pb"
    ))
    .ok()?;
    let overview = transit_rt::decode_overview(&payload).ok()?;
    let hash = |v: String| format!("sha256:{:x}", Sha256::digest(v.as_bytes()));
    Some(RtExampleContext {
        network_hash: hash(serde_json::to_string(&outcome.network).ok()?),
        boundary_hash: hash(serde_json::to_string(&outcome.boundary).ok()?),
        network: outcome.network,
        boundary: outcome.boundary,
        overview,
        rt_hash: format!("sha256:{:x}", Sha256::digest(&payload)),
    })
}

/// The live document over the fixture pair, the way the rt module
/// mints it: the document and its hash.
fn live_example() -> Option<(transit_live::LiveDay, String)> {
    let ctx = rt_example_context()?;
    let live = transit_rt::live_day(&ctx.network, &ctx.boundary, Some(&ctx.overview), None, None, transit_live::OnTimeWindow::default());
    let live_hash = format!("sha256:{:x}", Sha256::digest(serde_json::to_string(&live).ok()?.as_bytes()));
    Some((live, live_hash))
}

/// Per-kind worked examples for an op whose argument is a tagged
/// vocabulary. cap-author `apply`: one edit per kind, applied IN ORDER
/// to the fixture network through the same `apply_edits` the module
/// runs — an example that stops applying fails the build, and a kind
/// with no example (read off the type's own declaration) does too.
fn kinds_for(capability: &str, op: &str, net: Option<&NetworkExampleContext>) -> Vec<cap_abi::manifest::KindExample> {
    if (capability, op) != ("author", "apply") {
        return Vec::new();
    }
    let ctx = net.expect("the authoring kinds need the fixture network");
    let chain = edit_kind_examples();
    let mut doc = ctx.derived_network().clone();
    let mut kinds = Vec::with_capacity(chain.len());
    for (i, example) in chain.into_iter().enumerate() {
        let edit: transit_network::NetworkEdit = serde_json::from_value(example.clone())
            .unwrap_or_else(|e| panic!("edit example {i} ({example}) is not an edit: {e}"));
        doc = transit_network::apply_edits(&doc, std::slice::from_ref(&edit))
            .unwrap_or_else(|e| panic!("edit example {i} ({example}) does not apply: {e}"));
        let kind = example["edit"].as_str().expect("a tagged edit").to_string();
        kinds.push(cap_abi::manifest::KindExample { kind, example });
    }
    let declared = <transit_network::NetworkEdit as ts_rs::TS>::decl(&ts_rs::Config::default());
    let tags: std::collections::BTreeSet<&str> = declared
        .split("\"edit\": \"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .collect();
    let covered: std::collections::BTreeSet<&str> = kinds.iter().map(|k| k.kind.as_str()).collect();
    assert!(!tags.is_empty(), "the edit kinds are read off NetworkEdit's declaration");
    assert_eq!(covered, tags, "every edit kind has exactly one worked example (gen_manifest edit_kind_examples)");
    assert_eq!(covered.len(), kinds.len(), "one example per edit kind");
    kinds
}

/// The edit kinds, one each, in an order that applies to the fixture
/// network (routes 10/20/40, stops A B C and the station STN with bays
/// SA SB, service types WK SUN HOL, trips t10 t11 t20 t30 t40 t41,
/// block B1 = t10 then t20): each later edit may name what an earlier
/// one made.
fn edit_kind_examples() -> Vec<serde_json::Value> {
    use serde_json::json;
    vec![
        // the plan
        json!({ "edit": "set_plan", "plan": { "id": "fall-2026", "name": "Fall 2026", "effective_from": "2026-10-19" } }),
        json!({ "edit": "update_plan", "name": "Fall 2026 service" }),
        json!({ "edit": "set_name", "name": "Sample transit" }),
        // creation gestures (an omitted id is minted and reported in `created`)
        json!({ "edit": "add_stop", "id": "D", "lat": 45.53, "lon": -73.53 }),
        json!({ "edit": "add_route", "id": "30" }),
        json!({ "edit": "add_pattern", "id": "pat-30", "route": "30" }),
        json!({ "edit": "add_service_type", "id": "SAT", "from": "2026-10-19" }),
        json!({ "edit": "add_pattern_trips", "id": "SAT:pat-10-0-1", "pattern": "pat-10-0-1", "service_type": "SAT" }),
        // field gestures
        json!({ "edit": "update_stop", "id": "D", "name": "Main & 4th", "transfer": true }),
        json!({ "edit": "update_route", "id": "30", "name": "30", "mode": "bus" }),
        json!({ "edit": "set_direction", "route": "30", "id": "0", "name": "Uptown" }),
        json!({ "edit": "update_pattern", "id": "pat-30", "name": "30 via Main", "direction": "0" }),
        json!({ "edit": "update_service_type", "id": "SAT", "name": "Saturday" }),
        json!({ "edit": "set_pattern_trips_refs", "id": "SAT:pat-10-0-1", "service_type": "SAT" }),
        // a pattern's stops and timing
        json!({ "edit": "insert_pattern_stop", "pattern": "pat-10-0-2", "stop": "D", "index": 1 }),
        json!({ "edit": "set_pattern_stop", "pattern": "pat-10-0-2", "index": 1, "distance_m": 400.0 }),
        json!({ "edit": "toggle_timepoint", "pattern": "pat-10-0-2", "index": 1 }),
        json!({ "edit": "add_timing_profile", "pattern": "pat-10-0-2" }),
        json!({ "edit": "set_timing_profile", "pattern": "pat-10-0-2", "index": 1, "start_secs": 25200, "end_secs": 32400 }),
        json!({ "edit": "set_timepoint_time", "pattern": "pat-10-0-2", "profile": 1, "timepoint": 1, "secs": 300 }),
        json!({ "edit": "remove_timing_profile", "pattern": "pat-10-0-2", "index": 1 }),
        json!({ "edit": "remove_pattern_stop", "pattern": "pat-10-0-2", "index": 1 }),
        // a service type's calendar
        json!({ "edit": "toggle_service_day", "id": "SAT", "day": "sat" }),
        json!({ "edit": "add_service_range", "id": "SAT" }),
        json!({ "edit": "set_service_range", "id": "SAT", "index": 1, "start": "2027-01-01", "end": "2027-06-30" }),
        json!({ "edit": "remove_service_range", "id": "SAT", "index": 1 }),
        json!({ "edit": "set_service_exception", "id": "SAT", "date": "2026-12-26", "state": "removed" }),
        json!({ "edit": "remove_service_exception", "id": "SAT", "date": "2026-12-26" }),
        // trips
        json!({ "edit": "retime_trip", "trip": "t30", "to_secs": 31500 }),
        json!({ "edit": "add_trip", "pattern_trips": "WK:pat-10-0-1", "id": "t12", "start_secs": 34200 }),
        json!({ "edit": "remove_trip", "trip": "t12" }),
        // a frequency trip's bands
        json!({ "edit": "upsert_pattern_trips", "id": "WK:pat-20-0-0", "pattern_trips": {
            "pattern": "pat-20-0-0", "service_type": "WK", "trips": [
                { "id": "t20", "departs": { "kind": "at", "start_secs": 30000 }, "times": [
                    { "arrival_secs": 0, "departure_secs": 0 }, { "arrival_secs": 600, "departure_secs": 600 } ] },
                { "id": "t21", "departs": { "kind": "every", "exact": false, "bands": [
                    { "start_secs": 36000, "end_secs": 43200, "headway_secs": 1200 } ] }, "times": [
                    { "arrival_secs": 0, "departure_secs": 0 }, { "arrival_secs": 600, "departure_secs": 600 } ] } ] } }),
        json!({ "edit": "add_band", "trip": "t21" }),
        json!({ "edit": "set_band", "trip": "t21", "index": 1, "start_secs": 43200, "end_secs": 50400, "headway_secs": 1800 }),
        json!({ "edit": "remove_band", "trip": "t21", "index": 1 }),
        // the timetable gestures (route + direction + service type)
        json!({ "edit": "set_headway", "route": "10", "direction": "0", "service_type": "WK",
            "from_secs": 28800, "to_secs": 32400, "headway_secs": 1200 }),
        json!({ "edit": "set_span", "route": "10", "direction": "0", "service_type": "WK", "last_secs": 36000, "headway_secs": 1800 }),
        json!({ "edit": "adjust_running_time", "route": "10", "direction": "0", "service_type": "WK",
            "from_secs": 28800, "to_secs": 32400, "delta_secs": 120 }),
        json!({ "edit": "insert_direction_stop", "route": "10", "direction": "0", "stop": "D", "after": "A" }),
        json!({ "edit": "remove_direction_stop", "route": "10", "direction": "0", "stop": "D" }),
        // the runcut (an emptied block goes)
        json!({ "edit": "add_block", "id": "B2" }),
        json!({ "edit": "add_block_trip", "block": "B2", "trip": { "trip": "t40" } }),
        json!({ "edit": "set_layover", "block": "B1", "after_trip": 0, "secs": 600 }),
        json!({ "edit": "remove_block_trip", "block": "B2", "index": 0 }),
        // whole entities: read one from the document, change it, send it back
        json!({ "edit": "upsert_stop", "id": "E", "stop": { "name": "Main & 5th", "lat": 45.54, "lon": -73.54 } }),
        json!({ "edit": "upsert_route", "id": "50", "route": { "name": "50", "mode": "bus" } }),
        json!({ "edit": "upsert_pattern", "id": "pat-50", "pattern": { "route": "50", "name": "50 Main", "stops": [
            { "stop": "A", "distance_m": 0.0, "timepoint": true }, { "stop": "B", "distance_m": 800.0, "timepoint": true } ],
            "timing": [ { "start_secs": 0, "end_secs": 86400, "at_timepoints": [0, 240] } ] } }),
        json!({ "edit": "upsert_service_type", "id": "XMAS", "service_type": { "name": "Christmas", "rule": {
            "days": { "mon": false, "tue": false, "wed": false, "thu": false, "fri": false, "sat": false, "sun": false },
            "ranges": [], "exceptions": [ { "date": "2026-12-25", "state": "added" } ] } } }),
        json!({ "edit": "upsert_block", "id": "B3", "block": { "trips": [ { "trip": "t11" } ], "layovers": [] } }),
        // removals refuse while referenced; these name what nothing references
        json!({ "edit": "remove_block", "id": "B3" }),
        json!({ "edit": "remove_service_type", "id": "XMAS" }),
        json!({ "edit": "remove_stop", "id": "E" }),
        json!({ "edit": "remove_pattern_trips", "id": "SAT:pat-10-0-1" }),
        json!({ "edit": "remove_pattern", "id": "pat-30" }),
        json!({ "edit": "remove_direction", "route": "30", "id": "0" }),
        json!({ "edit": "remove_route", "id": "30" }),
    ]
}

/// Worked examples: an agent reads these to learn the shapes.
/// Requests carry the REAL feed hash computed above; responses
/// carry the fixture's known answers.
fn example_for(
    capability: &str,
    op: &str,
    feed_hash: &str,
    net: Option<&NetworkExampleContext>,
) -> Option<OpExample> {
    use serde_json::json;
    let (request, response) = match (capability, op) {
        ("gtfs", "feed_info") => {
            let gtfs = fixture_gtfs()?;
            (
                json!({ "op": "feed_info", "feed_hash": feed_hash }),
                json!({ "ok": { "op": "feed_info", "info": gtfs.info() } }),
            )
        }
        // The router's examples compute REAL answers over the
        // fixture-derived document — the same composition the module
        // dispatches through (materialize, patch, build, query).
        ("router", "load_live") => {
            let (live, live_hash) = live_example()?;
            (
                json!({ "op": "load_live", "live": live }),
                json!({ "ok": { "op": "load_live", "live_hash": live_hash } }),
            )
        }
        ("router", "load_walks") => {
            // A two-walk table over the fixture's station bays.
            let net = net?;
            let walk = |from: &str, to: &str| transit_streets::walking::Walk { from: from.into(), to: to.into(), walk_secs: 95 };
            let table = transit_streets::walking::WalkTable {
                streets_hash: "sha256:…".into(),
                network_hash: net.network_hash.clone(),
                speed_mps: transit_streets::walking::WALK_SPEED_MPS,
                max_walk_secs: transit_streets::walking::MAX_WALK_SECS,
                walks: vec![walk("SA", "SB"), walk("SB", "SA")],
            };
            let walks_hash = cap_abi::canonical_hash(&table).ok()?;
            (
                json!({ "op": "load_walks", "walks": table }),
                json!({ "ok": { "op": "load_walks", "walks_hash": walks_hash } }),
            )
        }
        ("router", "plan") => {
            // Over the fixture's live document: the legs carry the
            // predicted times and say how far they moved.
            let ctx = net?;
            let (live, live_hash) = live_example()?;
            let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 23).expect("fixture date");
            let network = ctx.derived_network();
            let day = transit_network::materialize(network, date).expect("fixture materializes");
            let graph = transit_router::RouterNetwork::build(network, &day, date, Some(&live));
            let itinerary = transit_router::earliest_arrival(&graph, "A", "B", 25200, 4)
                .expect("query")
                .expect("A→B reachable on the fixture Friday");
            (
                json!({ "op": "plan", "network_hash": ctx.network_hash, "from": "A", "to": "B",
                        "date": "2026-10-23", "depart_secs": 25200, "max_legs": 4, "live_hash": live_hash }),
                json!({ "ok": { "op": "plan", "itinerary": itinerary } }),
            )
        }
        ("router", "plan_arrive_by") => {
            // No deadline: the last bus home from A to B on the
            // fixture Friday.
            let ctx = net?;
            let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 23).expect("fixture date");
            let network = ctx.derived_network();
            let day = transit_network::materialize(network, date).expect("fixture materializes");
            let graph = transit_router::RouterNetwork::build_from_day(network, &day, date);
            let answer = transit_router::latest_departure(&graph, "A", "B", None, 4).expect("query");
            (
                json!({ "op": "plan_arrive_by", "network_hash": ctx.network_hash, "from": "A", "to": "B",
                        "date": "2026-10-23", "max_legs": 4 }),
                json!({ "ok": { "op": "plan_arrive_by", "arrive_by_secs": answer.arrive_by_secs,
                                "itinerary": answer.itinerary } }),
            )
        }
        ("router", "plan_compare") => {
            let ctx = net?;
            let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 23).expect("fixture date");
            let network = ctx.derived_network();
            let records = dispatch_fixture_edits();
            // The edits document's hash, computed the way the module
            // does: sha256 of the canonical serialization.
            let canonical = serde_json::to_string(&records).expect("records serialize");
            let edits_hash = format!("sha256:{:x}", Sha256::digest(canonical.as_bytes()));
            let patched =
                transit_dispatch::effective_day(network, date, &records).expect("patched day");
            let active_graph = transit_router::RouterNetwork::build_from_day(network, &patched, date);
            let active = transit_router::earliest_arrival(&active_graph, "A", "B", 25200, 4)
                .expect("query")
                .expect("A→B reachable while the diversion runs");
            let base_day = transit_network::materialize(network, date).expect("baseline day");
            let base_graph = transit_router::RouterNetwork::build_from_day(network, &base_day, date);
            let baseline = transit_router::earliest_arrival(&base_graph, "A", "B", 25200, 4)
                .expect("query")
                .expect("A→B reachable on the unpatched day");
            // The diversion costs the rider 5 minutes: the notice says so.
            let diverted = transit_router::diverted_instances(&base_day, &patched);
            let notice = transit_router::plan_notice(network, Some(&active), Some(&baseline), 4, &diverted);
            (
                json!({ "op": "plan_compare", "network_hash": ctx.network_hash,
                        "edits_hash": edits_hash, "from": "A", "to": "B",
                        "date": "2026-10-23", "depart_secs": 25200, "max_legs": 4 }),
                json!({ "ok": { "op": "plan_compare", "active": active, "baseline": baseline, "notice": notice } }),
            )
        }
        ("router", "itinerary_paths") => {
            let ctx = net?;
            let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 23).expect("fixture date");
            let network = ctx.derived_network();
            let day = transit_network::materialize(network, date).expect("fixture materializes");
            let graph = transit_router::RouterNetwork::build_from_day(network, &day, date);
            let itinerary = transit_router::earliest_arrival(&graph, "A", "B", 25200, 4).ok()??;
            let paths: Vec<_> = itinerary
                .legs
                .iter()
                .map(|leg| match leg {
                    transit_router::Leg::Transit { trip, start_secs, board_stop_id, alight_stop_id, .. } => {
                        json!({ "kind": "transit", "path": transit_dispatch::trip_segment(network, &day, &[], trip, *start_secs, board_stop_id, alight_stop_id) })
                    }
                    transit_router::Leg::Walk { from_stop_id, via_stop_ids, to_stop_id, .. } => {
                        let stops: Vec<&str> = std::iter::once(from_stop_id).chain(via_stop_ids).chain(std::iter::once(to_stop_id)).map(String::as_str).collect();
                        json!({ "kind": "walk", "path": transit_network::stops_line(network, stops) })
                    }
                })
                .collect();
            (
                json!({ "op": "itinerary_paths", "network_hash": ctx.network_hash, "date": "2026-10-23", "itinerary": itinerary }),
                json!({ "ok": { "op": "itinerary_paths", "paths": paths } }),
            )
        }
        // The network capability's examples carry the REAL derived
        // document and exported zip of the fixture — computed by the
        // same shared library the module dispatches through.
        ("network", "import") => {
            let ctx = net?;
            (
                json!({ "op": "import", "feed_hash": feed_hash }),
                json!({ "ok": { "op": "import", "network": ctx.derived, "boundary": ctx.boundary,
                    "notes": ctx.notes, "network_hash": ctx.network_hash } }),
            )
        }
        ("network", "load_network") => {
            let ctx = net?;
            (
                json!({ "op": "load_network", "network": ctx.derived }),
                json!({ "ok": { "op": "load_network", "network_hash": ctx.network_hash } }),
            )
        }
        ("network", "validate") => {
            let ctx = net?;
            (
                json!({ "op": "validate", "network_hash": ctx.network_hash }),
                json!({ "ok": { "op": "validate",
                    "issues": transit_network::validation_issues(ctx.derived_network()),
                    "warnings": transit_network::validation_warnings(ctx.derived_network()) } }),
            )
        }
        ("network", "export") => {
            let ctx = net?;
            (
                json!({ "op": "export", "network_hash": ctx.network_hash }),
                json!({ "ok": { "op": "export", "feed_hash": ctx.export_feed_hash,
                    "summary": ctx.export_summary, "zip_b64": ctx.zip_b64 } }),
            )
        }
        // The realtime example decodes the REAL e2e fixture payload —
        // the same bytes the browser tests replay.
        ("rt", "decode") => {
            let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/../../web/e2e/fixtures/sample-rt.pb");
            let bytes = std::fs::read(fixture).expect("rt fixture exists");
            let overview = transit_rt::decode_overview(&bytes).expect("fixture decodes");
            (
                json!({ "op": "decode", "bytes": "<the raw GTFS-rt protobuf — bulk-bytes op 0>" }),
                json!({ "ok": { "op": "decode", "overview": overview } }),
            )
        }
        // The analysis examples carry REAL answers over the fixture-
        // derived document (the same shared library the module
        // dispatches through), so the worked examples cannot drift.
        ("analysis", "load_network") => {
            let ctx = net?;
            (
                json!({ "op": "load_network", "network": ctx.derived }),
                json!({ "ok": { "op": "load_network", "network_hash": ctx.network_hash } }),
            )
        }
        ("analysis", "network_stats") => {
            let ctx = net?;
            let date = chrono::NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
            let mut stats = transit_analysis::network_stats(&ctx.derived_network(), date);
            // The example ASKS the day-shape question: no routes, no
            // stops, no service types inline — the query-shaping
            // flags in the flesh.
            let routes = std::mem::take(&mut stats.routes);
            let stops = std::mem::take(&mut stats.stops);
            let service_types = std::mem::take(&mut stats.service_types);
            let response = json!({ "ok": { "op": "network_stats", "stats": stats } });
            let _ = (routes, stops, service_types);
            (
                json!({ "op": "network_stats", "network_hash": ctx.network_hash,
                    "date": "2026-01-05", "include_routes": false, "include_stops": false, "include_service_types": false }),
                response,
            )
        }
        ("analysis", "service_types_overview") => {
            let ctx = net?;
            (
                json!({ "op": "service_types_overview", "network_hash": ctx.network_hash }),
                json!({ "ok": { "op": "service_types_overview",
                    "service_types": transit_analysis::service_type_overview(&ctx.derived_network()) } }),
            )
        }
        // The service groups over the same fixture document.
        ("analysis", "service_groups") => {
            let ctx = net?;
            (
                json!({ "op": "service_groups", "network_hash": ctx.network_hash }),
                json!({ "ok": { "op": "service_groups",
                    "groups": transit_analysis::service_groups(&ctx.derived_network()) } }),
            )
        }
        ("analysis", "service_type_detail") => {
            let ctx = net?;
            let detail = transit_analysis::service_type_detail(&ctx.derived_network(), "WK")
                .expect("WK exists in the fixture-derived document");
            (
                json!({ "op": "service_type_detail", "network_hash": ctx.network_hash, "service_type": "WK" }),
                json!({ "ok": { "op": "service_type_detail", "detail": detail } }),
            )
        }
        ("analysis", "service_type_compare") => {
            let ctx = net?;
            let comparison = transit_analysis::service_type_compare(&ctx.derived_network(), &ctx.derived_network(), "WK")
                .expect("WK exists in the fixture-derived document");
            (
                json!({ "op": "service_type_compare", "base_hash": ctx.network_hash, "network_hash": ctx.network_hash, "service_type": "WK" }),
                json!({ "ok": { "op": "service_type_compare", "comparison": comparison } }),
            )
        }
        // The stop's day as gaps, computed over the same fixture
        // document — a real weekday answer, not a hand-written one.
        ("analysis", "stop_gaps") => {
            let ctx = net?;
            let date = chrono::NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
            let gaps = transit_analysis::stop_gaps(&ctx.derived_network(), date, "A");
            (
                json!({ "op": "stop_gaps", "network_hash": ctx.network_hash, "date": "2026-01-05", "stop_id": "A" }),
                json!({ "ok": { "op": "stop_gaps", "gaps": gaps } }),
            )
        }
        // The day's hourly profile over the same fixture document.
        ("analysis", "hourly_trips") => {
            let ctx = net?;
            let date = chrono::NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
            (
                json!({ "op": "hourly_trips", "network_hash": ctx.network_hash, "date": "2026-01-05" }),
                json!({ "ok": { "op": "hourly_trips",
                    "hours": transit_analysis::hourly_trips(&ctx.derived_network(), date) } }),
            )
        }
        // The schedule texture over the same fixture document.
        ("analysis", "schedule_texture") => {
            let ctx = net?;
            (
                json!({ "op": "schedule_texture", "network_hash": ctx.network_hash, "service_type": "WK" }),
                json!({ "ok": { "op": "schedule_texture",
                    "texture": transit_analysis::schedule_texture(&ctx.derived_network(), "WK") } }),
            )
        }
        // The corridor read over the same fixture document.
        ("analysis", "route_overlap") => {
            let ctx = net?;
            let date = chrono::NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
            (
                json!({ "op": "route_overlap", "network_hash": ctx.network_hash, "date": "2026-01-05" }),
                json!({ "ok": { "op": "route_overlap",
                    "overlaps": transit_analysis::route_overlap(&ctx.derived_network(), date) } }),
            )
        }
        // The hub's timed transfers over the same fixture document —
        // the window the caller brings widens the default 60–900 s
        // connection bounds to catch the fixture's 20-minute meeting.
        ("analysis", "timed_transfers") => {
            let ctx = net?;
            let date = chrono::NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
            (
                json!({ "op": "timed_transfers", "network_hash": ctx.network_hash,
                    "date": "2026-01-05", "stop_id": "A",
                    "window": { "min_secs": 60, "max_secs": 1500 } }),
                json!({ "ok": { "op": "timed_transfers",
                    "transfers": transit_analysis::timed_transfers(
                        &ctx.derived_network(), date, "A",
                        Some(transit_analysis::TransferWindow { min_secs: 60, max_secs: 1500 })) } }),
            )
        }
        // The hub's route pairs: the same connections, folded.
        ("analysis", "hub_connections") => {
            let ctx = net?;
            let date = chrono::NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
            (
                json!({ "op": "hub_connections", "network_hash": ctx.network_hash,
                    "date": "2026-01-05", "stop_id": "A",
                    "window": { "min_secs": 60, "max_secs": 1500 } }),
                json!({ "ok": { "op": "hub_connections",
                    "connections": transit_analysis::hub_connections(
                        &ctx.derived_network(), date, "A",
                        Some(transit_analysis::TransferWindow { min_secs: 60, max_secs: 1500 })) } }),
            )
        }
        ("analysis", "route_frequency") => {
            let ctx = net?;
            (
                json!({ "op": "route_frequency", "network_hash": ctx.network_hash, "service_type": "WK" }),
                json!({ "ok": { "op": "route_frequency",
                    "frequency": transit_analysis::route_frequency(&ctx.derived_network(), "WK") } }),
            )
        }
        ("analysis", "type_coverage") => {
            let ctx = net?;
            // The same two-point layer the caps smoke uses: one point
            // at a stop (served), one far away — coverage is 50%.
            let layer_json = r#"{"type":"FeatureCollection","features":[
                {"type":"Feature","properties":{"pop":500},"geometry":{"type":"Point","coordinates":[-73.5,45.5]}},
                {"type":"Feature","properties":{"pop":500},"geometry":{"type":"Point","coordinates":[-74.0,46.0]}}
            ]}"#;
            let layer = transit_analysis::parse_pop_layer(layer_json.as_bytes(), None)
                .expect("layer parses");
            let layer_hash = format!("sha256:{:x}", Sha256::digest(layer_json.as_bytes()));
            (
                json!({ "op": "type_coverage", "network_hash": ctx.network_hash,
                    "layer_hash": layer_hash, "radius_m": 500, "frequent_secs": 3600, "min_departures": 2 }),
                json!({ "ok": { "op": "type_coverage",
                    "coverage": transit_analysis::type_coverage(&ctx.derived_network(), &layer, 500.0, 3600, 2, None) } }),
            )
        }
        // The unserved WHERE over the same fixture document and the
        // same two-point layer: the far point, named, with the
        // nearest served stop.
        ("analysis", "unserved_points") => {
            let ctx = net?;
            let layer_json = r#"{"type":"FeatureCollection","features":[
                {"type":"Feature","properties":{"pop":500},"geometry":{"type":"Point","coordinates":[-73.5,45.5]}},
                {"type":"Feature","properties":{"pop":500},"geometry":{"type":"Point","coordinates":[-74.0,46.0]}}
            ]}"#;
            let layer = transit_analysis::parse_pop_layer(layer_json.as_bytes(), None)
                .expect("layer parses");
            let layer_hash = format!("sha256:{:x}", Sha256::digest(layer_json.as_bytes()));
            let date = chrono::NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
            (
                json!({ "op": "unserved_points", "network_hash": ctx.network_hash,
                    "layer_hash": layer_hash, "radius_m": 500, "date": "2026-01-05" }),
                json!({ "ok": { "op": "unserved_points",
                    "points": transit_analysis::unserved_points(&ctx.derived_network(), date, &layer, 500.0) } }),
            )
        }
        // The rt join examples compute REAL answers over the fixture
        // pair (sample-gtfs.zip + sample-rt.pb): t10 +120 s at A /
        // +180 s at B, one vehicle on t10 — the joined board and the
        // ranked view through the same shared library the module
        // dispatches through.
        ("rt", "load_network") => {
            let ctx = rt_example_context()?;
            (
                json!({ "op": "load_network", "network": ctx.network }),
                json!({ "ok": { "op": "load_network", "network_hash": ctx.network_hash } }),
            )
        }
        ("rt", "load_boundary") => {
            let ctx = rt_example_context()?;
            (
                json!({ "op": "load_boundary", "boundary": ctx.boundary }),
                json!({ "ok": { "op": "load_boundary", "boundary_hash": ctx.boundary_hash } }),
            )
        }
        ("rt", "trip_progress") => {
            let ctx = rt_example_context()?;
            (
                json!({ "op": "trip_progress", "network_hash": ctx.network_hash,
                    "boundary_hash": ctx.boundary_hash, "rt_hash": ctx.rt_hash, "trip_id": "t10" }),
                json!({ "ok": { "op": "trip_progress",
                    "progress": transit_rt::trip_progress(&ctx.network, &ctx.boundary, &ctx.overview, "t10") } }),
            )
        }
        ("rt", "live") => {
            let ctx = rt_example_context()?;
            let (live, live_hash) = live_example()?;
            (
                json!({ "op": "live", "network_hash": ctx.network_hash, "boundary_hash": ctx.boundary_hash,
                    "rt_hash": ctx.rt_hash,
                    "window": { "late_secs": 60, "very_late_secs": 300 } }),
                json!({ "ok": { "op": "live", "live_hash": live_hash,
                    "summary": transit_live::summarize(&live, 15), "live": live } }),
            )
        }
        ("rt", "late_trips") => {
            let ctx = rt_example_context()?;
            (
                json!({ "op": "late_trips", "network_hash": ctx.network_hash,
                    "rt_hash": ctx.rt_hash, "min_delay_secs": 60,
                    "vp_hash": null }),
                json!({ "ok": { "op": "late_trips",
                    "trips": transit_rt::late_trips(&ctx.network, &ctx.overview, None, 60) } }),
            )
        }
        ("board", "load_network") => {
            let ctx = net?;
            (
                json!({ "op": "load_network", "network": ctx.derived }),
                json!({ "ok": { "op": "load_network", "network_hash": ctx.network_hash } }),
            )
        }
        ("board", "load_edits") => {
            let ctx = net?;
            let records = dispatch_fixture_edits();
            let reports = transit_dispatch::report_against_network(ctx.derived_network(), &records);
            let edits_hash = format!("sha256:{:x}", Sha256::digest(
                serde_json::to_string(&records).expect("records serialize").as_bytes(),
            ));
            (
                json!({ "op": "load_edits", "network_hash": ctx.network_hash, "records": records }),
                json!({ "ok": { "op": "load_edits", "edits_hash": edits_hash, "reports": reports } }),
            )
        }
        ("board", "load_live") => {
            let (live, live_hash) = live_example()?;
            (
                json!({ "op": "load_live", "live": live }),
                json!({ "ok": { "op": "load_live", "live_hash": live_hash } }),
            )
        }
        ("board", "stop_board") => {
            let ctx = net?;
            let (live, live_hash) = live_example()?;
            let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
            let day = transit_network::materialize(ctx.derived_network(), date).expect("fixture day");
            // 08:01 at A: t10 (08:00, +120 s) has not left yet.
            (
                json!({ "op": "stop_board", "network_hash": ctx.network_hash,
                    "stop_id": "A", "date": "2026-10-23", "from_secs": 28860, "limit": 5,
                    "live_hash": live_hash }),
                json!({ "ok": { "op": "stop_board",
                    "board": transit_dispatch::stop_board(ctx.derived_network(), &day, "A", 28860, 5, Some(&live), None) } }),
            )
        }
        ("board", "grouped_board") => {
            let ctx = net?;
            let (live, live_hash) = live_example()?;
            let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
            let day = transit_network::materialize(ctx.derived_network(), date).expect("fixture day");
            // 08:01 at A: the 10's next three (t10 late, +120 s), the 20's one.
            (
                json!({ "op": "grouped_board", "network_hash": ctx.network_hash,
                    "stop_id": "A", "date": "2026-10-23", "from_secs": 28860, "per_group": 3,
                    "live_hash": live_hash }),
                json!({ "ok": { "op": "grouped_board",
                    "board": transit_dispatch::grouped_board(ctx.derived_network(), &day, "A", 28860, 3, Some(&live), None) } }),
            )
        }
        ("board", "load_watchlist") => (
            json!({ "op": "load_watchlist", "document": { "version": "v2", "stops": [
                { "id": "A", "label": "home" }, { "id": "B", "label": "work" }
            ] } }),
            json!({ "ok": { "op": "load_watchlist", "version": 2, "watchlist_hash": "sha256:<of the document>" } }),
        ),
        ("board", "migrate_watchlist") => (
            json!({ "op": "migrate_watchlist", "document": { "version": "v1", "stops": [
                { "id": "A", "note": "home", "color": "blue" }
            ] } }),
            json!({ "ok": { "op": "migrated_watchlist", "document": { "version": "v2", "stops": [
                { "id": "A", "label": "home" }
            ] }, "notes": [ { "kind": "dropped_field", "field": "color", "count": 1 } ] } }),
        ),
        ("board", "watchlist_board") => {
            let ctx = net?;
            let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
            let day = transit_network::materialize(ctx.derived_network(), date).expect("fixture day");
            let network = ctx.derived_network();
            let board = |stop: &str| transit_dispatch::stop_board(&network, &day, stop, 25200, 3, None, None);
            (
                json!({ "op": "watchlist_board", "watchlist_hash": "sha256:<of the document>",
                    "network_hash": ctx.network_hash, "date": "2026-10-23", "from_secs": 25200, "limit": 3 }),
                json!({ "ok": { "op": "watchlist_board", "stops": [
                    { "id": "A", "label": "home", "board": board("A") },
                    { "id": "B", "label": "work", "board": board("B") }
                ] } }),
            )
        }
        ("runcut", "load_network") => {
            let ctx = net?;
            (
                json!({ "op": "load_network", "network": ctx.derived }),
                json!({ "ok": { "op": "load_network", "network_hash": ctx.network_hash } }),
            )
        }
        ("runcut", "load_edits") => {
            let ctx = net?;
            let records = dispatch_fixture_edits();
            let reports = transit_dispatch::report_against_network(ctx.derived_network(), &records);
            let edits_hash = format!("sha256:{:x}", Sha256::digest(
                serde_json::to_string(&records).expect("records serialize").as_bytes(),
            ));
            (
                json!({ "op": "load_edits", "network_hash": ctx.network_hash, "records": records }),
                json!({ "ok": { "op": "load_edits", "edits_hash": edits_hash, "reports": reports } }),
            )
        }
        ("runcut", "blocks") => {
            let ctx = net?;
            let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
            let day = transit_network::materialize(ctx.derived_network(), date).expect("fixture day");
            (
                json!({ "op": "blocks", "network_hash": ctx.network_hash, "date": "2026-10-23" }),
                json!({ "ok": { "op": "blocks",
                    "blocks": transit_dispatch::day_blocks(ctx.derived_network(), &day) } }),
            )
        }
        ("runcut", "fleet_summary") => {
            let ctx = net?;
            let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
            let day = transit_network::materialize(ctx.derived_network(), date).expect("fixture day");
            (
                json!({ "op": "fleet_summary", "network_hash": ctx.network_hash, "date": "2026-10-23" }),
                json!({ "ok": { "op": "fleet_summary",
                    "summary": transit_dispatch::fleet_summary(ctx.derived_network(), &day) } }),
            )
        }
        ("runcut", "load_live") => {
            let (live, live_hash) = live_example()?;
            (
                json!({ "op": "load_live", "live": live }),
                json!({ "ok": { "op": "load_live", "live_hash": live_hash } }),
            )
        }
        ("runcut", "block_trips") => {
            let ctx = net?;
            let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
            let records = dispatch_fixture_edits();
            let day = transit_dispatch::effective_day(ctx.derived_network(), date, &records)
                .expect("fixture day");
            (
                json!({ "op": "block_trips", "network_hash": ctx.network_hash,
                    "edits_hash": "<of the loaded edit document>", "block_id": "B1", "date": "2026-10-23" }),
                json!({ "ok": { "op": "block_trips", "block_id": "B1",
                    "trips": transit_dispatch::block_chain(ctx.derived_network(), &day, "B1", transit_dispatch::TIGHT_LAYOVER_SECS, None) } }),
            )
        }
        ("runcut", "trip_stops") => {
            let ctx = net?;
            let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
            let records = dispatch_fixture_edits();
            let day = transit_dispatch::effective_day(ctx.derived_network(), date, &records)
                .expect("fixture day");
            // t30: the via-C run at 08:40 — the
            // patched day keeps A→B (C skipped, +300 s).
            let trip = day
                .iter()
                .find(|t| t.stop_times.len() == 2 && t.start_secs == 8 * 3600 + 40 * 60)
                .expect("the diverted trip is in the day");
            (
                json!({ "op": "trip_stops", "network_hash": ctx.network_hash,
                    "edits_hash": "<of the loaded edit document>",
                    "trip": trip.trip, "start_secs": trip.start_secs, "date": "2026-10-23" }),
                json!({ "ok": { "op": "trip_stops",
                    "stops": transit_dispatch::trip_stops(
                        ctx.derived_network(), &day, &trip.trip, trip.start_secs) } }),
            )
        }
        ("runcut", "trip_path") => {
            let ctx = net?;
            let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
            let records = dispatch_fixture_edits();
            let day = transit_dispatch::effective_day(ctx.derived_network(), date, &records).ok()?;
            let diversions = transit_sync::in_force_on(&records, date);
            // The diverted 08:40 trip draws the diversion's line (A→B).
            let trip = day.iter().find(|t| t.stop_times.len() == 2 && t.start_secs == 8 * 3600 + 40 * 60)?;
            (
                json!({ "op": "trip_path", "network_hash": ctx.network_hash,
                    "edits_hash": "<of the loaded edit document>",
                    "trip": trip.trip, "start_secs": trip.start_secs, "date": "2026-10-23" }),
                json!({ "ok": { "op": "trip_path",
                    "path": transit_dispatch::trip_path(ctx.derived_network(), &day, &diversions, &trip.trip, trip.start_secs) } }),
            )
        }
        ("calendar", "load_network") => {
            let ctx = net?;
            (
                json!({ "op": "load_network", "network": ctx.derived }),
                json!({ "ok": { "op": "load_network", "network_hash": ctx.network_hash } }),
            )
        }
        ("calendar", "load_edits") => {
            let ctx = net?;
            let records = dispatch_fixture_edits();
            let reports = transit_dispatch::report_against_network(ctx.derived_network(), &records);
            let edits_hash = format!("sha256:{:x}", Sha256::digest(
                serde_json::to_string(&records).expect("records serialize").as_bytes(),
            ));
            (
                json!({ "op": "load_edits", "network_hash": ctx.network_hash, "records": records }),
                json!({ "ok": { "op": "load_edits", "edits_hash": edits_hash, "reports": reports } }),
            )
        }
        ("calendar", "active_services") => {
            let ctx = net?;
            let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
            let day = transit_network::materialize(ctx.derived_network(), date).expect("fixture day");
            (
                json!({ "op": "active_services", "network_hash": ctx.network_hash, "date": "2026-10-23" }),
                json!({ "ok": { "op": "active_services",
                    "services": transit_dispatch::active_services(ctx.derived_network(), &day) } }),
            )
        }
        // The reverse read, computed over the same fixture document
        // — the window crosses the fixture's calendar exception (WK
        // removed on 2026-10-29), so the example cannot drift.
        ("calendar", "service_dates") => {
            let ctx = net?;
            let from = chrono::NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
            let to = chrono::NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
            let dates = transit_dispatch::service_dates(ctx.derived_network(), &[], "WK", from, to)
                .expect("fixture window");
            (
                json!({ "op": "service_dates", "network_hash": ctx.network_hash,
                    "service_type": "WK", "from": "2026-10-23", "to": "2026-11-03" }),
                json!({ "ok": { "op": "service_dates",
                    "dates": dates.iter().map(|d| d.format("%Y-%m-%d").to_string()).collect::<Vec<_>>() } }),
            )
        }
        // The directory's examples compute REAL answers over the
        // fixture-derived document — the stop list with its walking
        // group, the route list, and route 10's patterns (the direct
        // run and the via-C run) — through the same document the
        // module addresses, so the worked examples cannot drift.
        ("directory", "load_network") => {
            let ctx = net?;
            (
                json!({ "op": "load_network", "network": ctx.derived }),
                json!({ "ok": { "op": "load_network", "network_hash": ctx.network_hash } }),
            )
        }
        ("directory", "stops") => {
            let ctx = net?;
            let network = ctx.derived_network();
            let mut stops: Vec<serde_json::Value> = network
                .stops
                .keys()
                .map(|id| {
                    let s = &network.stops[id];
                    json!({ "stop_id": id, "name": s.name, "lat": s.lat, "lon": s.lon,
                        "parent": s.parent, "transfer": s.transfer })
                })
                .collect();
            stops.sort_by(|a, b| {
                a["name"].as_str().cmp(&b["name"].as_str())
                    .then(a["stop_id"].as_str().cmp(&b["stop_id"].as_str()))
            });
            (
                json!({ "op": "stops", "network_hash": ctx.network_hash, "limit": 3 }),
                json!({ "ok": { "op": "stops", "stops": stops.into_iter().take(3).collect::<Vec<_>>() } }),
            )
        }
        ("directory", "stops_near") => {
            // Standing on the station's Bay A: the station answers once
            // (0 m — its nearest member), then the plain stops by distance.
            let ctx = net?;
            (
                json!({ "op": "stops_near", "network_hash": ctx.network_hash, "lat": 45.515, "lon": -73.5155, "limit": 3 }),
                json!({ "ok": { "op": "stops_near",
                    "stops": transit_network::listing::stops_near(ctx.derived_network(), 45.515, -73.5155, 3, None) } }),
            )
        }
        ("directory", "stops_by_ids") => {
            let ctx = net?;
            let network = ctx.derived_network();
            let pick = |id: &str| network.stops.get(id).map(|s| json!({
                "stop_id": id, "name": s.name, "lat": s.lat, "lon": s.lon,
                "parent": s.parent, "transfer": s.transfer }));
            (
                json!({ "op": "stops_by_ids", "network_hash": ctx.network_hash, "ids": ["A", "ghost", "STN"] }),
                json!({ "ok": { "op": "stops_by_ids", "stops": [
                    pick("A").expect("A exists"), pick("STN").expect("STN exists"),
                ] } }),
            )
        }
        ("directory", "routes") => {
            let ctx = net?;
            let network = ctx.derived_network();
            let mut routes: Vec<serde_json::Value> = network
                .routes
                .iter()
                .map(|(id, r)| json!({ "route_id": id, "name": r.label(), "long_name": r.long_name, "mode": r.mode, "color": r.color }))
                .collect();
            routes.sort_by(|a, b| {
                a["name"].as_str().cmp(&b["name"].as_str())
                    .then(a["route_id"].as_str().cmp(&b["route_id"].as_str()))
            });
            (
                json!({ "op": "routes", "network_hash": ctx.network_hash }),
                json!({ "ok": { "op": "routes", "routes": routes } }),
            )
        }
        ("directory", "focus") => {
            let ctx = net?;
            (
                json!({ "op": "focus", "network_hash": ctx.network_hash, "kind": "route", "id": "10" }),
                json!({ "ok": { "op": "focus",
                    "focus": transit_network::entity_focus(ctx.derived_network(), transit_network::EntityKind::Route, "10") } }),
            )
        }
        ("directory", "entities") => {
            let ctx = net?;
            (
                json!({ "op": "entities", "network_hash": ctx.network_hash, "kind": "pattern_trips", "query": "wk" }),
                json!({ "ok": { "op": "entities",
                    "rows": transit_network::entity_rows(ctx.derived_network(), transit_network::EntityKind::PatternTrips, "wk") } }),
            )
        }
        ("directory", "route_detail") => {
            let ctx = net?;
            (
                json!({ "op": "route_detail", "network_hash": ctx.network_hash, "route_id": "10" }),
                json!({ "ok": { "op": "route_detail",
                    "detail": transit_network::route_detail(ctx.derived_network(), "10") } }),
            )
        }
        ("directory", "timetable") => {
            let ctx = net?;
            (
                json!({ "op": "timetable", "network_hash": ctx.network_hash, "route_id": "10", "direction": "0", "service_type": "WK" }),
                json!({ "ok": { "op": "timetable",
                    "timetable": transit_network::timetable(ctx.derived_network(), "10", Some("0"), "WK") } }),
            )
        }
        ("directory", "stop_detail") => {
            let ctx = net?;
            (
                json!({ "op": "stop_detail", "network_hash": ctx.network_hash, "stop_id": "B" }),
                json!({ "ok": { "op": "stop_detail",
                    "detail": transit_network::stop_detail(ctx.derived_network(), "B") } }),
            )
        }
        ("directory", "route_stop_sequence") => {
            let ctx = net?;
            let network = ctx.derived_network();
            let patterns: Vec<serde_json::Value> = network
                .patterns
                .iter()
                .filter(|(_, p)| p.route == "10")
                .map(|(id, p)| json!({ "pattern": id, "name": p.name, "stops": p.stops.iter().map(|ps| json!({
                    "stop_id": ps.stop,
                    "name": network.stops.get(&ps.stop).map(|s| s.name.clone()).unwrap_or_else(|| ps.stop.clone()),
                })).collect::<Vec<_>>() }))
                .collect();
            (
                json!({ "op": "route_stop_sequence", "network_hash": ctx.network_hash, "route_id": "10" }),
                json!({ "ok": { "op": "route_stop_sequence", "patterns": patterns } }),
            )
        }
        ("streets", "walks") => {
            // The walking fixture: a residential street through the
            // station's bays, a motorway across it no rider walks.
            let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../web/e2e/fixtures/walk-streets.geojson")).ok()?;
            let streets = transit_streets::geojson::import(&bytes).ok()?;
            let streets_hash = cap_abi::canonical_hash(&streets).ok()?;
            let net = net?;
            let stops: Vec<(String, f64, f64)> =
                net.derived_network().stops.iter().map(|(id, s)| (id.clone(), s.lat, s.lon)).collect();
            let table = transit_streets::walking::WalkTable {
                streets_hash: streets_hash.clone(),
                network_hash: net.network_hash.clone(),
                speed_mps: transit_streets::walking::WALK_SPEED_MPS,
                max_walk_secs: transit_streets::walking::MAX_WALK_SECS,
                walks: transit_streets::walking::walks(
                    &streets,
                    &stops,
                    transit_streets::walking::WALK_SPEED_MPS,
                    transit_streets::walking::MAX_WALK_SECS,
                ),
            };
            let walks_hash = cap_abi::canonical_hash(&table).ok()?;
            (
                json!({ "op": "walks", "streets_hash": streets_hash, "network_hash": net.network_hash }),
                json!({ "ok": { "op": "walks", "walks_hash": walks_hash, "walks": table } }),
            )
        }
        ("streets", "access") => {
            // From a dropped pin ~22 m off Station Rd, west of the
            // bays, with a destination east of them.
            let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../web/e2e/fixtures/walk-streets.geojson")).ok()?;
            let streets = transit_streets::geojson::import(&bytes).ok()?;
            let streets_hash = cap_abi::canonical_hash(&streets).ok()?;
            let net = net?;
            let stops: Vec<(String, f64, f64)> =
                net.derived_network().stops.iter().map(|(id, s)| (id.clone(), s.lat, s.lon)).collect();
            let graph = transit_streets::walking::WalkGraph::new(&streets);
            let (speed, max) = (transit_streets::walking::WALK_SPEED_MPS, transit_streets::walking::MAX_WALK_SECS);
            let walks = graph.walks_from(45.5152, -73.5165, &stops, speed, max);
            let direct = graph.walks_from(45.5152, -73.5165, &[(String::new(), 45.5148, -73.5135)], speed, max).pop()?;
            (
                json!({ "op": "access", "streets_hash": streets_hash, "network_hash": net.network_hash,
                        "point": { "lat": 45.5152, "lon": -73.5165 }, "to": { "lat": 45.5148, "lon": -73.5135 } }),
                json!({ "ok": { "op": "access", "walks": walks,
                                "direct": { "walk_secs": direct.walk_secs, "path": direct.path } } }),
            )
        }
        ("streets", "load_streets") => {
            let ctx = streets_example_context()?;
            (
                json!({ "op": "load_streets", "streets": ctx.streets }),
                json!({ "ok": { "op": "load_streets", "streets_hash": ctx.streets_hash,
                    "ways": ctx.streets.ways.len(), "segments": ctx.segments, "nodes": ctx.nodes } }),
            )
        }
        ("streets", "snap") => {
            let ctx = streets_example_context()?;
            (
                json!({ "op": "snap", "streets_hash": ctx.streets_hash, "lat": 45.5040, "lon": -73.5030 }),
                json!({ "ok": { "op": "snap", "position": ctx.pos_a } }),
            )
        }
        ("streets", "route") => {
            let ctx = streets_example_context()?;
            (
                json!({ "op": "route", "streets_hash": ctx.streets_hash, "from": ctx.pos_a, "to": ctx.pos_b }),
                json!({ "ok": { "op": "route", "points": ctx.points, "length_m": ctx.length_m } }),
            )
        }
        // The sync capability's example computes the REAL converged
        // answer over the fixture edit document (draft client vs
        // empty server) through the same library the module
        // dispatches through.
        ("sync", "sync_edits") => {
            let mut draft = dispatch_fixture_edits();
            draft[0].status = transit_sync::EditStatus::Draft;
            let now_ms: u64 = 1_770_000_000_000;
            let records = transit_sync::submit(transit_sync::merge(&draft, &[]), now_ms);
            (
                json!({ "op": "sync_edits", "client": draft, "server": [], "now_ms": now_ms }),
                json!({ "ok": { "op": "sync_edits", "records": records } }),
            )
        }
        // The plan-chain capability's examples compute REAL answers
        // through the same shared library the module dispatches
        // through — a two-pick chain over the fixture's dates.
        // Authoring examples run the REAL edit through the shared
        // library: the fixture network stamped as a pick, the answer's
        // hash computed the way every module computes it.
        ("author", "load_network") => {
            let ctx = net?;
            (
                json!({ "op": "load_network", "network": ctx.derived }),
                json!({ "ok": { "op": "load_network", "network_hash": ctx.network_hash } }),
            )
        }
        ("author", "apply") => {
            let ctx = net?;
            let edits = vec![transit_network::NetworkEdit::SetPlan {
                plan: transit_network::PlanMeta {
                    id: "fall-2026".into(),
                    name: "Fall 2026".into(),
                    effective_from: chrono::NaiveDate::from_ymd_opt(2026, 10, 19),
                },
            }, transit_network::NetworkEdit::AddStop { id: None, lat: Some(45.5), lon: Some(-73.5) }];
            let (next, created) = transit_network::apply_edits_reporting(ctx.derived_network(), &edits).ok()?;
            let canonical = serde_json::to_string(&next).ok()?;
            (
                json!({ "op": "apply", "network_hash": ctx.network_hash, "edits": edits }),
                json!({ "ok": { "op": "apply",
                    "network_hash": format!("sha256:{:x}", Sha256::digest(canonical.as_bytes())),
                    "network": next,
                    "issues": transit_network::validation_issues(&next),
                    "warnings": transit_network::validation_warnings(&next),
                    "created": created } }),
            )
        }
        // Diversion-authoring examples run the REAL draft through the
        // shared libraries over the fixture network: route 10 anchored
        // end to end on its default pattern.
        ("divert", "load_network") => {
            let ctx = net?;
            (
                json!({ "op": "load_network", "network": ctx.derived }),
                json!({ "ok": { "op": "load_network", "network_hash": ctx.network_hash } }),
            )
        }
        ("divert", "draft") => {
            let ctx = net?;
            let draft = divert_example_draft(ctx.derived_network())?;
            (
                json!({ "op": "draft", "network_hash": ctx.network_hash, "draft": draft }),
                json!({ "ok": { "op": "draft", "view": transit_dispatch::draft_view(ctx.derived_network(), &draft) } }),
            )
        }
        ("divert", "list") => {
            let ctx = net?;
            let records = dispatch_fixture_edits();
            (
                json!({ "op": "list", "network_hash": ctx.network_hash, "records": records }),
                json!({ "ok": { "op": "list", "rows": transit_dispatch::diversion_rows(ctx.derived_network(), &records) } }),
            )
        }
        ("divert", "propose") => {
            let ctx = net?;
            let draft = divert_example_draft(ctx.derived_network())?;
            let now_ms: u64 = 1_770_000_000_000;
            let diversion = transit_dispatch::diversion_of(ctx.derived_network(), &draft, &format!("div-10-{now_ms}")).ok()?;
            (
                json!({ "op": "propose", "network_hash": ctx.network_hash, "draft": draft, "records": [], "record_id": "e2", "now_ms": now_ms }),
                json!({ "ok": { "op": "propose", "records": transit_sync::propose(&[], "e2", diversion, now_ms) } }),
            )
        }
        ("divert", op @ ("retire" | "restore")) => {
            let mut records = dispatch_fixture_edits();
            if op == "restore" {
                records = transit_sync::set_retired(&records, "e1", true, 1_770_000_000_000).ok()?;
            }
            let now_ms: u64 = 1_770_000_060_000;
            let next = transit_sync::set_retired(&records, "e1", op == "retire", now_ms).ok()?;
            (
                json!({ "op": op, "records": records, "record_id": "e1", "now_ms": now_ms }),
                json!({ "ok": { "op": op, "records": next } }),
            )
        }
        ("divert", "sanitize") => {
            let mut old = serde_json::to_value(&dispatch_fixture_edits()[0]).ok()?;
            for field in ["direction_id", "skipped_stops", "detour", "added_secs"] {
                old["diversion"].as_object_mut()?.remove(field);
            }
            let stored = json!([old, { "id": "e3", "status": "draft" }]);
            let (records, dropped) = transit_sync::sanitize(&stored);
            (
                json!({ "op": "sanitize", "records": stored }),
                json!({ "ok": { "op": "sanitize", "records": records, "dropped": dropped } }),
            )
        }
        ("plans", "resolve") => {
            let plan_set = fixture_plan_set();
            let plan = transit_plans::resolve(
                &plan_set,
                chrono::NaiveDate::from_ymd_opt(2026, 10, 23).unwrap(),
            )
            .expect("fixture dates parse");
            (
                json!({ "op": "resolve", "plans": plan_set, "date": "2026-10-23" }),
                json!({ "ok": { "op": "resolve", "plan": plan } }),
            )
        }
        ("plans", "validate") => {
            let plan_set = fixture_plan_set();
            (
                json!({ "op": "validate", "plans": plan_set }),
                json!({ "ok": { "op": "validate", "issues": transit_plans::validate(&plan_set) } }),
            )
        }
        ("plans", "periods") => {
            let plan_set = fixture_plan_set();
            (
                json!({ "op": "periods", "plans": plan_set }),
                json!({ "ok": { "op": "periods", "periods": transit_plans::periods(&plan_set) } }),
            )
        }
        // Bulk-bytes ops have no JSON request example (the payload is
        // the raw zip/protobuf); the description carries the contract.
        (_, "load_feed") => return None,
        (_, "load_population_layer") => return None,
        (_, "load_streets") => return None,
        _ => return None,
    };
    Some(OpExample { request, response })
}

/// The plan-chain examples' plan set: a two-pick chain around the
/// fixture's dates — the spring pick in force on the fixture Friday,
/// the fall pick taking over after it.
fn fixture_plan_set() -> Vec<transit_plans::PlanInfo> {
    vec![
        transit_plans::PlanInfo {
            id: "spring".into(),
            name: "Spring 2026".into(),
            effective_from: Some("2026-03-01".into()),
        },
        transit_plans::PlanInfo {
            id: "fall".into(),
            name: "Fall 2026".into(),
            effective_from: Some("2026-09-07".into()),
        },
    ]
}

/// The dispatch examples' edit document: the fixture diversion
/// (route 10 keeps A→B, skips C, +300 s on the fixture Friday).
/// The divert examples' draft: route 10 kept end to end on its
/// default pattern, five minutes added, on the fixture Friday.
fn divert_example_draft(network: &transit_network::Network) -> Option<transit_dispatch::DiversionDraft> {
    let mut draft = transit_dispatch::DiversionDraft {
        route_id: "10".into(),
        pattern: None,
        first_kept_stop: None,
        last_kept_stop: None,
        served_stops: vec![],
        slices: vec![],
        added_mins: 5.0,
        start_date: "2026-10-23".into(),
        end_date: "2026-10-23".into(),
    };
    let stops = transit_dispatch::draft_view(network, &draft).stops;
    draft.first_kept_stop = Some(stops.first()?.stop_id.clone());
    draft.last_kept_stop = Some(stops.last()?.stop_id.clone());
    Some(draft)
}

fn dispatch_fixture_edits() -> Vec<transit_sync::EditRecord> {
    let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
    vec![transit_sync::EditRecord {
        id: "e1".into(),
        diversion: transit_sync::Diversion {
            id: "d1".into(),
            route_id: "10".into(),
            direction_id: None,
            first_kept_stop: "A".into(),
            last_kept_stop: "B".into(),
            skipped_stops: vec!["C".into()],
            detour: None,
            added_secs: 300,
            start_date: date,
            end_date: date,
        },
        status: transit_sync::EditStatus::Accepted,
        rev: 1,
        updated_at: 1,
    }]
}
