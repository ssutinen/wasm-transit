// Capability-module ABI smoke: instantiates each capability module
// directly in Node (zero imports, the flat ABI) and verifies the
// module contract plus the composition — the same bytes into both
// modules, hashes equal, ops answering through their typed seams.
//
// The manifest is verified too: every capability it advertises is
// present, its bytes hash to the pinned hash, and the bulk op loads
// the fixture. Run from anywhere: `node crates/cap-abi/caps-smoke.mjs`.
// Also `just test`.

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
const DIST = path.join(here, "../../web/public/caps");
const DATE = "2026-10-23";

const manifest = JSON.parse(readFileSync(path.join(DIST, "manifest.json"), "utf8"));
const feed = readFileSync(path.join(DIST, "sample-gtfs.zip"));

const assert = (cond, msg) => {
  if (!cond) throw new Error("FAIL: " + msg);
  console.log("ok - " + msg);
};

function loadModule(cap) {
  const bytes = readFileSync(path.join(DIST, "..", cap.url));
  const hash = "sha256:" + createHash("sha256").update(bytes).digest("hex");
  assert(hash === cap.hash, `capability ${cap.id} bytes match the pinned hash`);
  const mod = new WebAssembly.Module(bytes);
  assert(
    WebAssembly.Module.imports(mod).length === 0,
    `capability ${cap.id} has zero imports`,
  );
  assert(
    WebAssembly.Module.exports(mod).some((e) => e.name === "transit_invoke"),
    `capability ${cap.id} exports transit_invoke`,
  );
  return new WebAssembly.Instance(mod, {}).exports;
}

function shell(exports) {
  const raw = (request) => {
    const payload = Buffer.from(JSON.stringify(request));
    const ptr = exports.transit_alloc(payload.length);
    new Uint8Array(exports.memory.buffer, ptr, payload.length).set(payload);
    const outPtr = exports.transit_invoke(ptr, payload.length);
    exports.transit_free(ptr, payload.length);
    const len = new DataView(exports.memory.buffer).getUint32(outPtr, true);
    const outcome = JSON.parse(
      Buffer.from(exports.memory.buffer, outPtr + 4, len).toString("utf8"),
    );
    exports.transit_free(outPtr, len + 4);
    return outcome;
  };
  const read = (outPtr) => {
    const len = new DataView(exports.memory.buffer).getUint32(outPtr, true);
    const outcome = JSON.parse(
      Buffer.from(exports.memory.buffer, outPtr + 4, len).toString("utf8"),
    );
    exports.transit_free(outPtr, len + 4);
    if (outcome.error) throw new Error("op error: " + outcome.error);
    return outcome.ok;
  };
  const invoke = (request) => {
    const outcome = raw(request);
    if (outcome.error) throw new Error("op error: " + outcome.error);
    return outcome.ok;
  };
  const invokeBytes = (op, bytes) => {
    const ptr = exports.transit_alloc(bytes.length);
    new Uint8Array(exports.memory.buffer, ptr, bytes.length).set(bytes);
    const outPtr = exports.transit_invoke_bytes(op, ptr, bytes.length);
    exports.transit_free(ptr, bytes.length);
    return read(outPtr);
  };
  return { invoke, invokeBytes, raw };
}

const byId = Object.fromEntries(manifest.capabilities.map((c) => [c.id, c]));
assert(manifest.kernel === 1, "manifest speaks kernel v1");
assert(byId.gtfs && byId.router, "manifest advertises gtfs and router");
assert(byId.network, "manifest advertises the network capability");
assert(byId.router.consumes.network === "network", "router declares its registry edge: it consumes network documents");
assert(byId.analysis.consumes.network === "network", "analysis consumes networks from the network capability");

// Agent context: every capability states its purpose and documents
// every op — the discovery surface an agent assembles from.
for (const cap of manifest.capabilities) {
  assert(
    typeof cap.purpose === "string" && cap.purpose.length > 40,
    `capability ${cap.id} carries agent context (purpose)`,
  );
  for (const op of cap.ops) {
    assert(cap.op_docs[op], `capability ${cap.id} documents op ${op}`);
  }
  assert(Object.keys(cap.speaks).length > 0, `capability ${cap.id} declares the documents it speaks`);
}

// Compatibility documentation: the board capability speaks both
// watchlist versions and provides the 1→2 migration; the validator
// accepted it.
assert(
  byId.board.speaks.watchlist.includes(1) && byId.board.speaks.watchlist.includes(2),
  "board speaks watchlist v1 and v2",
);
assert(
  byId.board.migrates.watchlist.some((m) => m.from === 1 && m.to === 2),
  "board migrates watchlist 1→2",
);

// The feed document capability — parse + identity alone. The day
// views and planning are NOT here (one question, one answer on the
// bus; planning joins the network-document composition below); the
// retired ops must answer as request errors, never silently serve.
const gtfs = shell(loadModule(byId.gtfs));

const loadedGtfs = gtfs.invokeBytes(0, feed);
assert(loadedGtfs.op === "load_feed", "gtfs load_feed answers on the bulk-bytes seam");

// The feed's identity and its names — the feed document capability's
// whole surface. The day views are NOT here (one question, one
// answer on the bus); the retired ops must answer as request
// errors, never silently serve.
const info = gtfs.invoke({ op: "feed_info", feed_hash: loadedGtfs.feed_hash });
assert(
  info.info && info.info.feed_version === "2026.10.1",
  "the feed answers its own identity (version, publisher)",
);
const retiredOps = [
  "departures",
  "station_departures",
  "active_services",
  "blocks",
  "block_trips",
  "stops",
  "stops_by_ids",
];
for (const op of retiredOps) {
  const gone = gtfs.raw({ op, feed_hash: loadedGtfs.feed_hash });
  assert(gone.error && gone.error.includes("invalid request"), `${op} is retired on gtfs`);
}

const bad = gtfs.raw({ op: "nope" });
assert(bad.error && bad.error.includes("invalid request"), "unknown ops error, never panic");
const missing = gtfs.raw({
  op: "feed_info",
  feed_hash: "sha256:missing",
});
assert(missing.error && missing.error.includes("unknown feed"), "unknown hashes error, never panic");

// The analysis flow — the first REAL capability extracted from the
// monolith: a network document in by content hash, planner numbers
// out by hash.
const analysis = shell(loadModule(byId.analysis));
const NETWORK = {
  name: "Cap Analysis Net",
  plan: { id: "", name: "" },
  anchor: "noon_minus_twelve",
  stops: {
    A: { name: "Main & 1st", lat: 45.5, lon: -73.5, transfer: false },
    B: { name: "Main & 2nd", lat: 45.501, lon: -73.501, transfer: false },
  },
  routes: { "10": { name: "10", mode: "bus" } },
  patterns: {
    "pat-10-0-0": {
      route: "10",
      name: "10 → Main & 2nd",
      stops: [
        { stop: "A", timepoint: true, distance_m: 0.0 },
        { stop: "B", timepoint: true, distance_m: 500.0 },
      ],
      timing: [{ start_secs: 0, end_secs: 108000, at_timepoints: [0, 600] }],
    },
  },
  service_types: {
    WK: {
      name: "WK",
      rule: {
        days: { mon: true, tue: true, wed: true, thu: true, fri: true, sat: false, sun: false },
        ranges: [{ start: "2026-01-01", end: "2026-12-31" }],
        exceptions: [],
      },
    },
  },
  pattern_trips: {
    "WK:pat-10-0-0": {
      pattern: "pat-10-0-0",
      service_type: "WK",
      departures: { kind: "exact_times", starts: [28800] },
    },
  },
  blocks: {},
  notes: [],
};
const loadedNet = analysis.invoke({ op: "load_network", network: NETWORK });
assert(loadedNet.network_hash.startsWith("sha256:"), "analysis registers the network by content hash");
const stats = analysis.invoke({
  op: "network_stats",
  network_hash: loadedNet.network_hash,
  date: "2026-01-05", // a Monday: WK runs
});
assert(stats.stats.trips === 1, `analysis computes the document's weekday trips: ${stats.stats.trips}`);
const overview = analysis.invoke({ op: "service_types_overview", network_hash: loadedNet.network_hash });
assert(overview.service_types.length === 1, "the design axis answers");
// The stop's day as gaps (QAF-25): one departure is no gap — the
// honest answer is null, and the terminal stop is an arrival, never
// a departure.
const gaps = analysis.invoke({
  op: "stop_gaps",
  network_hash: loadedNet.network_hash,
  date: "2026-01-05",
  stop_id: "A",
});
assert(gaps.gaps.departures === 1, "one weekday departure at A");
assert(gaps.gaps.max_gap_secs == null, "a one-departion day has no gap to read");
assert(gaps.gaps.hourly.length === 1 && gaps.gaps.hourly[0].hour === 8, "28800 s is hour 8");
assert(gaps.gaps.hourly[0].max_gap_secs == null, "no gap starts in hour 8");
const terminalGaps = analysis.invoke({
  op: "stop_gaps",
  network_hash: loadedNet.network_hash,
  date: "2026-01-05",
  stop_id: "B",
});
assert(terminalGaps.gaps.departures === 0, "final-stop visits are arrivals, never departures");
const unknownGaps = analysis.invoke({
  op: "stop_gaps",
  network_hash: loadedNet.network_hash,
  date: "2026-01-05",
  stop_id: "nope",
});
assert(unknownGaps.gaps.departures === 0, "an unknown stop answers empty, never errors");
// The day's hourly profile (QAF-4): one weekday trip at 28800 s is
// one hour row; a weekend-only day asked on a Monday answers empty.
const hours = analysis.invoke({
  op: "hourly_trips",
  network_hash: loadedNet.network_hash,
  date: "2026-01-05",
});
assert(hours.hours.length === 1 && hours.hours[0].hour === 8, "28800 s is hour 8");
assert(hours.hours[0].trips === 1, "one trip starts in hour 8");
assert(hours.hours[0].service_types.length === 1 && hours.hours[0].service_types[0].service_type === "WK",
  "the hour row carries the running service type");
const quietHours = analysis.invoke({
  op: "hourly_trips",
  network_hash: loadedNet.network_hash,
  date: "2026-01-10", // a Saturday: WK does not run
});
assert(quietHours.hours.length === 0, "a day nothing runs answers empty, never errors");
// The schedule texture (QAF-16): the one schedule's uniformity (one
// trip — no gaps to read) and clockface alignment (28800 s is on
// the quarter-hour grid).
const texture = analysis.invoke({
  op: "schedule_texture",
  network_hash: loadedNet.network_hash,
  service_type: "WK",
});
assert(texture.texture.pattern_trips.length === 1, "one schedule under WK");
const row = texture.texture.pattern_trips[0];
assert(row.trips === 1 && row.min_gap_secs == null && row.max_gap_secs == null,
  "a one-trip schedule has no gap to read");
assert(texture.texture.on_clockface === 1, "28800 s is on the quarter-hour grid");
const unknownTexture = analysis.invoke({
  op: "schedule_texture",
  network_hash: loadedNet.network_hash,
  service_type: "nope",
});
assert(unknownTexture.texture == null, "an unknown service type answers null, never errors");
// The corridor read (QAF-22): a one-route document has no pairs —
// the honest answer is empty, never an error.
const overlaps = analysis.invoke({
  op: "route_overlap",
  network_hash: loadedNet.network_hash,
  date: "2026-01-05",
});
assert(Array.isArray(overlaps.overlaps) && overlaps.overlaps.length === 0,
  "a one-route document has no pairs to join");
// The timed-transfer view (QAF-27): one route alone has nobody to
// meet — the honest answer is empty, never an error.
const transfers = analysis.invoke({
  op: "timed_transfers",
  network_hash: loadedNet.network_hash,
  date: "2026-01-05",
  stop_id: "A",
});
assert(Array.isArray(transfers.transfers) && transfers.transfers.length === 0,
  "a one-route hub has no connections to pair");
const layer = analysis.invokeBytes(0, Buffer.from(JSON.stringify({
  type: "FeatureCollection",
  features: [
    { type: "Feature", properties: { pop: 500 }, geometry: { type: "Point", coordinates: [-73.5, 45.5] } },
    { type: "Feature", properties: { pop: 500 }, geometry: { type: "Point", coordinates: [-74.0, 46.0] } },
  ],
})));
assert(layer.points === 2, "analysis registers a population layer by content hash");
// The registration answer IS the import handle (U4: the monolith's
// parse-only op is retired): the field the parse picked, the people
// it found.
assert(layer.pop_field === "pop", "the layer handle names the population field");
assert(layer.total_pop === 1000, "the layer handle sums the population: " + layer.total_pop);
const coverage = analysis.invoke({
  op: "type_coverage",
  network_hash: loadedNet.network_hash,
  layer_hash: layer.layer_hash,
  radius_m: 500,
  frequent_secs: 3600,
  min_departures: 2,
});
assert(coverage.coverage.length === 1, "type coverage answers by hash pair");
assert(
  coverage.coverage[0].served_pct === 50,
  `half the layer is served (one point at a stop, one far away): ${coverage.coverage[0].served_pct}`,
);
// The windowed form crosses the seam (QAF-42): frequency judged as
// the worst wait inside a window; the whole day is the widest one.
const windowed = analysis.invoke({
  op: "type_coverage",
  network_hash: loadedNet.network_hash,
  layer_hash: layer.layer_hash,
  radius_m: 500,
  frequent_secs: 3600,
  min_departures: 2,
  window: { start_secs: 0, end_secs: 86400 },
});
assert(
  windowed.coverage.length === 1 && windowed.coverage[0].served_pct === 50,
  "the window changes frequency, never what is served",
);
const unknownNet = analysis.raw({ op: "network_stats", network_hash: "sha256:missing", date: "2026-01-05" });
assert(unknownNet.error && unknownNet.error.includes("unknown network"), "unknown hashes error, never panic");

// The network flow — the second REAL capability extracted from the
// monolith: the planner's whole bootstrap → author → publish path.
// The same feed bytes into a THIRD module, identity asserted; import
// derives the document (hash returned, resolutions reported);
// export produces a GTFS zip whose identity any module agrees on.
const network = shell(loadModule(byId.network));
const loadedNetCap = network.invokeBytes(0, feed);
assert(loadedNetCap.op === "load_feed", "network load_feed answers on the bulk-bytes seam");
assert(
  loadedNetCap.feed_hash === loadedGtfs.feed_hash,
  "three capabilities agree on the feed identity",
);
const imported = network.invoke({ op: "import", feed_hash: loadedNetCap.feed_hash });
assert(
  imported.network && Object.keys(imported.network.stops).length === 6,
  "import infers the fixture feed (6 stops, station pin included)",
);
assert(
  imported.network_hash.startsWith("sha256:"),
  "import registers the derived document and returns its hash",
);
assert(
  Array.isArray(imported.notes) && imported.notes.length > 0,
  "the fixture's dropped single-stop trip is reported — never silent",
);
const validated = network.invoke({ op: "validate", network_hash: imported.network_hash });
assert(validated.issues.length === 0, "the derived document validates clean");
const loadedDoc = network.invoke({ op: "load_network", network: imported.network });
assert(
  loadedDoc.network_hash === imported.network_hash,
  "load_network is idempotent by content hash",
);
const exported = network.invoke({ op: "export", network_hash: imported.network_hash });
const zip = Buffer.from(exported.zip_b64, "base64");
assert(
  "sha256:" + createHash("sha256").update(zip).digest("hex") === exported.feed_hash,
  "the export's hash is the content hash of the decoded bytes",
);
assert(
  zip[0] === 0x50 && zip[1] === 0x4b,
  "the export payload is zip bytes",
);
// The composition moment of the publish path: the exported artifact is
// a first-class feed — cap-gtfs loads the SAME bytes and returns the
// SAME identity the export advertised.
const published = gtfs.invokeBytes(0, zip);
assert(
  published.feed_hash === exported.feed_hash,
  "the exported zip loads into cap-gtfs under the advertised hash",
);
// Broken documents: validate reports every issue, export refuses.
const broken = JSON.parse(JSON.stringify(imported.network));
broken.patterns[Object.keys(broken.patterns)[0]].stops[1].stop = "ghost";
const brokenHash = network.invoke({ op: "load_network", network: broken }).network_hash;
const brokenIssues = network.invoke({ op: "validate", network_hash: brokenHash });
assert(brokenIssues.issues.length > 0, "a referential break is reported");
const refused = network.raw({ op: "export", network_hash: brokenHash });
assert(refused.error && refused.error.includes("invalid network"), "export refuses invalid documents");
const unknownFeed = network.raw({ op: "import", feed_hash: "sha256:missing" });
assert(unknownFeed.error && unknownFeed.error.includes("unknown feed"), "unknown hashes error, never panic");

// The realtime flow — the third capability extracted from the
// monolith: protobuf bytes in on the bulk seam, delays and vehicles
// out. A pure decoder; garbage bytes error, never trap.
const rt = shell(loadModule(byId.rt));
const rtPayload = readFileSync(path.join(here, "../../web/e2e/fixtures/sample-rt.pb"));
const decoded = rt.invokeBytes(0, rtPayload);
assert(decoded.op === "decode", "rt decode answers on the bulk-bytes seam");
assert(decoded.overview.trip_updates === 1, "one trip update in the fixture");
assert(decoded.overview.vehicles === 1, "one vehicle in the fixture");
const trip = decoded.overview.trips[0];
assert(trip.trip_id === "t10", "the delayed trip is t10");
assert(trip.stop_updates[0].departure_delay === 120, "t10 is +120 s at A");
const vehicle = decoded.overview.vehicle_positions[0];
assert(vehicle.label === "4201", "the vehicle label (the bus number) survives decode");
const garbage = rt.raw({ op: "decode", bytes: [0xff, 0xff] });
assert(garbage.error && /decode/.test(garbage.error), "garbage bytes are an error outcome, never a trap");

// The join: the imported NETWORK into cap-rt must agree with the
// network capability on the identity, with the import's boundary for
// the published stop_sequence numbering — and the joined ops answer
// over the fixture pair (t10 +120/+180, one vehicle): an rt trip id is
// a model trip id.
const loadedRt = rt.invoke({ op: "load_network", network: imported.network });
assert(
  loadedRt.network_hash === imported.network_hash,
  "cap-rt agrees with the network capability on the document's identity",
);
const loadedBoundary = rt.invoke({ op: "load_boundary", boundary: imported.boundary });
assert(loadedBoundary.boundary_hash.startsWith("sha256:"), "the boundary registers under its content hash");
const snapshot = rt.invokeBytes(0, readFileSync(path.join(here, "../../web/e2e/fixtures/sample-rt.pb")));
assert(snapshot.rt_hash.startsWith("sha256:"), "decode registers the snapshot under its content hash");
const progress = rt.invoke({
  op: "trip_progress",
  network_hash: loadedRt.network_hash,
  boundary_hash: loadedBoundary.boundary_hash,
  rt_hash: snapshot.rt_hash,
  trip_id: "t10",
});
assert(progress.progress && progress.progress.stops.length === 2, "t10's joined board has both stops");
assert(
  progress.progress.stops[0].predicted_departure === 28800 + 120,
  "t10's predicted departure at A is 08:02 (schedule + delay)",
);
const late = rt.invoke({
  op: "late_trips",
  network_hash: loadedRt.network_hash,
  rt_hash: snapshot.rt_hash,
  min_delay_secs: 60,
});
assert(late.trips.length === 1 && late.trips[0].trip_id === "t10", "the ranked view finds the late trip");
assert(late.trips[0].vehicle_label === "4201" && late.trips[0].has_position, "its bus is named and visible");
const unknownJoin = rt.raw({ op: "trip_progress", network_hash: "sha256:missing", rt_hash: decoded.rt_hash, trip_id: "t10" });
assert(unknownJoin.error && unknownJoin.error.includes("unknown network"), "unknown hashes error, never panic");

// The names join (stops_by_ids) moved to the directory with the
// stop list it belongs to — the flow below pins it; the retired gtfs
// op is pinned in the retirement loop above.

// The day-view capabilities — the need cut of the day views over
// the SAME network document the import produced (model-native end
// to end; the arch gate in `just test` enforces that). The
// composition law: one document into every capability that needs
// it, identities agree — one document, one truth, no expansion.
assert(byId.board && byId.runcut && byId.calendar, "manifest advertises board, runcut, and calendar");
assert(
  byId.board.consumes.network === "network",
  "board declares its registry edge: it consumes network documents",
);
assert(
  byId.runcut.consumes.network === "network",
  "runcut declares its registry edge: it consumes network documents",
);
assert(
  byId.calendar.consumes.network === "network",
  "calendar declares its registry edge: it consumes network documents",
);
const board = shell(loadModule(byId.board));
const runcut = shell(loadModule(byId.runcut));
const calendar = shell(loadModule(byId.calendar));
const loadedBoardNet = board.invoke({ op: "load_network", network: imported.network });
const loadedRuncutNet = runcut.invoke({ op: "load_network", network: imported.network });
const loadedCalendarNet = calendar.invoke({ op: "load_network", network: imported.network });
assert(
  loadedBoardNet.network_hash === loadedRuncutNet.network_hash &&
    loadedRuncutNet.network_hash === loadedCalendarNet.network_hash &&
    loadedCalendarNet.network_hash === imported.network_hash,
  "board, runcut, and calendar agree with the network capability on the document identity",
);

// THE ROUTER — planning over the same document (the planner left the
// feed behind: the graph is the materialized day, and the diversion
// delta is the unpatched day one materialization apart).
const router = shell(loadModule(byId.router));
const loadedRouterNet = router.invoke({ op: "load_network", network: imported.network });
assert(
  loadedRouterNet.network_hash === imported.network_hash,
  "router agrees with the network capability on the document identity",
);
const plan = router.invoke({
  op: "plan",
  network_hash: loadedRouterNet.network_hash,
  from: "A",
  to: "B",
  date: DATE,
  depart_secs: 7 * 3600,
  max_legs: 4,
});
assert(
  plan.itinerary && plan.itinerary.arrival_secs === 8 * 3600 + 600,
  "router plans A → B over the materialized day: arrive 08:10",
);
const planPaths = router.invoke({
  op: "itinerary_paths",
  network_hash: loadedRouterNet.network_hash,
  date: DATE,
  itinerary: plan.itinerary,
}).paths;
assert(
  planPaths.length === plan.itinerary.legs.length && planPaths[0].kind === "transit" && planPaths[0].path.length >= 2,
  "an itinerary draws one path per leg, the ridden segment",
);
const routerEdits = router.invoke({
  op: "load_edits",
  network_hash: loadedRouterNet.network_hash,
  records: [
    {
      id: "e1",
      diversion: {
        id: "d1",
        route_id: "10",
        direction_id: null,
        first_kept_stop: "A",
        last_kept_stop: "B",
        skipped_stops: ["C"],
        detour: null,
        added_secs: 300,
        start_date: DATE,
        end_date: DATE,
      },
      status: "accepted",
      rev: 1,
      updated_at: 1,
    },
  ],
});
assert(
  routerEdits.reports.length === 1 && routerEdits.reports[0].valid,
  "the edit validates against the network (reported, never rejected)",
);
const compare = router.invoke({
  op: "plan_compare",
  network_hash: loadedRouterNet.network_hash,
  edits_hash: routerEdits.edits_hash,
  from: "A",
  to: "B",
  date: DATE,
  depart_secs: 7 * 3600,
  max_legs: 4,
});
assert(
  compare.active && compare.active.arrival_hhmmss === "08:15:00" &&
    compare.baseline && compare.baseline.arrival_hhmmss === "08:10:00",
  "the diversion delta: 08:15 against the unpatched 08:10",
);
assert(
  compare.notice && compare.notice.kind === "later" && compare.notice.minutes_lost === 5 &&
    compare.notice.text.includes("5 min later"),
  "a later arrival carries the rider's notice: 5 min later on the detour",
);
const tooLate = router.invoke({
  op: "plan_compare",
  network_hash: loadedRouterNet.network_hash,
  edits_hash: null,
  from: "A",
  to: "B",
  date: DATE,
  depart_secs: 40 * 3600,
  max_legs: 6,
});
assert(
  tooLate.active === null && tooLate.baseline === null && tooLate.notice.kind === "no_trip" &&
    tooLate.notice.text.includes("within 6 legs"),
  "no trip and no second world: the notice names the legs searched",
);
const lastBus = router.invoke({
  op: "plan_arrive_by",
  network_hash: loadedRouterNet.network_hash,
  from: "A",
  to: "B",
  date: DATE,
  max_legs: 4,
});
assert(
  lastBus.arrive_by_secs === 8 * 3600 + 3000 && lastBus.itinerary && lastBus.itinerary.depart_secs === 8 * 3600 + 2400,
  "arrive-by with no deadline answers the last bus home: t30 leaves 08:40",
);
const badRouter = router.raw({ op: "nope" });
assert(badRouter.error && badRouter.error.includes("invalid request"), "unknown ops error, never panic");

// THE BOARD — when does service leave this stop?
const stopBoard = (stop_id, from_secs, live_hash) =>
  board.invoke({
    op: "stop_board",
    network_hash: loadedBoardNet.network_hash,
    stop_id,
    date: DATE,
    from_secs,
    limit: 10,
    ...(live_hash ? { live_hash } : {}),
  }).board;
const atA = stopBoard("A", 25200);
assert(atA.rows.length >= 3 && !atA.pooled && !atA.live, "the materialized board answers at A, schedule-only");
assert(
  atA.rows.every((r) => r.trip && r.route_name && r.stop === "A" && r.status === "no_prediction"),
  "board rows speak model vocabulary (trip, start, concrete stop); no live data, no prediction",
);
const atStation = stopBoard("STN", 0);
assert(
  atStation.pooled && atStation.rows.map((r) => r.stop).join() === "SA,SB",
  "the station pin answers its bays' pooled board (t40 leaves SA, t41 leaves SB), rows tagged with the bay",
);
// An arrival is not a departure: every fixture trip through B ends
// there, so B's board is empty (QAF-29) — and B is a plain stop, so
// it never pools.
const atB = stopBoard("B", 0);
assert(atB.rows.length === 0 && !atB.pooled, "trips terminating at B never appear on B's board");

// THE LIVE DOCUMENT — rt → board by hash: the snapshot resolves
// against the network into trip instances in cap-rt; the board reads
// it. No host joins rt (the parity rule).
const live = rt.invoke({
  op: "live",
  network_hash: loadedRt.network_hash,
  boundary_hash: loadedBoundary.boundary_hash,
  rt_hash: snapshot.rt_hash,
});
assert(
  live.live.unmatched_trips === 0 && live.live.trips.length === 1 &&
    live.live.trips[0].trip === "t10" && live.live.trips[0].start_secs === 28800,
  "t10 resolves into its trip instance (t10, 08:00)",
);
assert(
  live.summary.counts.late === 1 && live.summary.late[0].label === "4201" && live.summary.late[0].delay_secs === 180,
  "the summary names the late bus (+180 s is late under the default window)",
);
const loadedLive = board.invoke({ op: "load_live", live: live.live });
assert(loadedLive.live_hash === live.live_hash, "board and rt agree on the live document's identity");
// 08:01 at A: t10 left the schedule at 08:00 but is +120 s — still to come.
const liveA = stopBoard("A", 28860, loadedLive.live_hash);
const t10Row = liveA.rows.find((r) => r.expected_secs != null);
assert(
  liveA.live && t10Row && t10Row.expected_secs === 28800 + 120 && t10Row.delay_secs === 120 && t10Row.status === "late",
  "the realtime-first board keeps the late bus, expected 08:02",
);
const noSnapshot = rt.raw({ op: "live", network_hash: loadedRt.network_hash });
assert(noSnapshot.error && noSnapshot.error.includes("needs a snapshot"), "live without a snapshot is a request error");

// The personal board — the watchlist document, migrated with its
// loss reported, both versions querying over the network.
const v1 = {
  version: "v1",
  stops: [
    { id: "A", note: "home", color: "blue" },
    { id: "B", note: "work" },
  ],
};
const migrated = board.invoke({ op: "migrate_watchlist", document: v1 });
assert(migrated.document.version === "v2", "migration produces v2");
assert(migrated.document.stops[0].label === "home", "label = note");
assert(
  migrated.notes.length === 1 &&
    migrated.notes[0].kind === "dropped_field" &&
    migrated.notes[0].field === "color" &&
    migrated.notes[0].count === 1,
  "the dropped color is reported — never silent",
);
const againMigrate = board.raw({ op: "migrate_watchlist", document: migrated.document });
assert(againMigrate.error && againMigrate.error.includes("already version 2"), "re-migrating v2 errors, not panics");
for (const document of [v1, migrated.document]) {
  const loaded = board.invoke({ op: "load_watchlist", document });
  const watch = board.invoke({
    op: "watchlist_board",
    watchlist_hash: loaded.watchlist_hash,
    network_hash: loadedBoardNet.network_hash,
    date: DATE,
    from_secs: 25200,
    limit: 3,
  });
  assert(watch.stops.length === 2, `the personal board answers at ${document.version}`);
  assert(
    watch.stops.every((s) => s.board.rows.every((r) => r.trip)),
    `the personal board's rows are model-native at ${document.version}`,
  );
}

// THE RUNCUT — what does this vehicle do today? The day's blocks
// list (the index) into one block's chain (the detail), with the
// edit ripple.
const daysBlocks = runcut.invoke({
  op: "blocks",
  network_hash: loadedRuncutNet.network_hash,
  date: DATE,
});
assert(
  daysBlocks.blocks.length === 1 && daysBlocks.blocks[0].block_id === "B1",
  "the day's blocks: B1 alone (B2's only trip is single-stop — the import drops it; the honest hole)",
);
const parkedDay = runcut.raw({ op: "blocks", network_hash: loadedRuncutNet.network_hash, date: "2026-10-24" });
assert(parkedDay.ok && parkedDay.ok.blocks.length === 0, "a no-service day has no blocks out");
const runcutChain = runcut.invoke({
  op: "block_trips",
  network_hash: loadedRuncutNet.network_hash,
  block_id: "B1",
  date: DATE,
});
assert(runcutChain.trips.length === 2, "the authored chain answers");
assert(runcutChain.trips[0].layover_after_secs === 600, "the layover is model state (600 s)");
assert(runcutChain.trips[0].layover_remaining_secs === 600, "no edits: the layover stands");
const runcutEdits = runcut.invoke({
  op: "load_edits",
  network_hash: loadedRuncutNet.network_hash,
  records: [
    {
      id: "e1",
      diversion: {
        id: "d1",
        route_id: "10",
        direction_id: null,
        first_kept_stop: "A",
        last_kept_stop: "B",
        skipped_stops: ["C"],
        detour: null,
        added_secs: 300,
        start_date: DATE,
        end_date: DATE,
      },
      status: "accepted",
      rev: 1,
      updated_at: 1,
    },
  ],
});
assert(
  runcutEdits.reports.length === 1 && runcutEdits.reports[0].valid,
  "the edit validates against the network (reported, never rejected)",
);
const rippedChain = runcut.invoke({
  op: "block_trips",
  network_hash: loadedRuncutNet.network_hash,
  edits_hash: runcutEdits.edits_hash,
  block_id: "B1",
  date: DATE,
});
assert(
  rippedChain.trips[0].layover_remaining_secs === 300,
  "the diverted end eats the layover (600 - 300) — the connection-health ripple",
);
const tightChain = runcut.invoke({
  op: "block_trips",
  network_hash: loadedRuncutNet.network_hash,
  edits_hash: runcutEdits.edits_hash,
  block_id: "B1",
  date: DATE,
  tight_layover_secs: 600,
});
assert(
  rippedChain.trips[0].layover_health === "ok" && tightChain.trips[0].layover_health === "tight",
  "the module classifies the connection under the caller's tight threshold",
);
// The live ripple (QAF-31): the snapshot predicts t10 +180 s at B, its
// last stop — 420 s of the 600 s layover before t20 remain.
const runcutLive = runcut.invoke({ op: "load_live", live: live.live });
assert(runcutLive.live_hash === live.live_hash, "runcut and rt agree on the live document's identity");
const liveChain = runcut.invoke({
  op: "block_trips",
  network_hash: loadedRuncutNet.network_hash,
  block_id: "B1",
  date: DATE,
  live_hash: runcutLive.live_hash,
});
assert(
  liveChain.trips[0].live_end_delay_secs === 180 && liveChain.trips[0].layover_remaining_secs === 420,
  "the live delay at t10's last stop eats the layover (600 - 180)",
);
// What the radio calls those trips: each row carries its trip's own
// id (the agency's, for an imported trip).
assert(rippedChain.trips[0].trip === "t10", "the chain names its trips by id (B1 starts with t10)");
// The trip detail — the map's click-through question: one trip
// instance's EFFECTIVE stops, patched by the same edits (its line is
// trip_path, below).
const chainTrip = runcutChain.trips[0];
const tripAuthored = runcut.invoke({
  op: "trip_stops",
  network_hash: loadedRuncutNet.network_hash,
  trip: chainTrip.trip,
  start_secs: chainTrip.start_secs,
  date: DATE,
});
assert(
  tripAuthored.stops.length === 2 &&
    tripAuthored.stops[0].stop_id === "A" &&
    tripAuthored.stops[1].stop_id === "B" &&
    tripAuthored.stops[1].arrival_secs === 8 * 3600 + 10 * 60,
  "the authored day's trip detail: A→B, B at 08:10",
);
const tripPatched = runcut.invoke({
  op: "trip_stops",
  network_hash: loadedRuncutNet.network_hash,
  edits_hash: runcutEdits.edits_hash,
  trip: chainTrip.trip,
  start_secs: chainTrip.start_secs,
  date: DATE,
});
assert(
  tripPatched.stops.length === 2 &&
    tripPatched.stops[1].arrival_secs === 8 * 3600 + 15 * 60,
  "the patched day's trip detail: the kept segment carries the added 300 s (B at 08:15)",
);
const tripAbsent = runcut.invoke({
  op: "trip_stops",
  network_hash: loadedRuncutNet.network_hash,
  trip: chainTrip.trip,
  start_secs: 123,
  date: DATE,
});
assert(tripAbsent.stops.length === 0, "a trip not in the day answers empty, never an error");
const tripLine = runcut.invoke({
  op: "trip_path",
  network_hash: loadedRuncutNet.network_hash,
  edits_hash: runcutEdits.edits_hash,
  trip: chainTrip.trip,
  start_secs: chainTrip.start_secs,
  date: DATE,
});
const chainAlignment =
  imported.network.alignments[imported.network.patterns[imported.network.pattern_trips[chainTrip.pattern_trips].pattern].alignment];
assert(
  chainAlignment &&
    JSON.stringify(tripLine.path) === JSON.stringify(chainAlignment.points.map(({ lat, lon }) => ({ lat, lon }))),
  "a trip that follows its pattern draws the alignment it runs",
);
const brokenEdit = runcut.invoke({
  op: "load_edits",
  network_hash: loadedRuncutNet.network_hash,
  records: [
    {
      id: "e2",
      diversion: {
        id: "d2",
        route_id: "ghost",
        direction_id: null,
        first_kept_stop: "A",
        last_kept_stop: "B",
        skipped_stops: [],
        detour: null,
        added_secs: 60,
        start_date: DATE,
        end_date: DATE,
      },
      status: "accepted",
      rev: 1,
      updated_at: 1,
    },
  ],
});
assert(
  brokenEdit.reports[0].valid === false,
  "an edit against an unknown route is reported invalid, never rejected",
);

// THE CALENDAR — which service keys run today? From the effective
// day: HOL's calendar exception runs, but its only trip never made
// it into the model — the honest answer is WK alone.
const keys = calendar.invoke({
  op: "active_services",
  network_hash: loadedCalendarNet.network_hash,
  date: DATE,
});
assert(
  keys.services.length === 1 && keys.services[0] === "WK",
  "the effective day answers: WK runs, HOL's unrepresentable trip does not resurrect it",
);
const keysEmpty = calendar.invoke({
  op: "active_services",
  network_hash: loadedCalendarNet.network_hash,
  date: "2026-10-24",
});
assert(keysEmpty.services.length === 0, "a no-service day answers empty, never errors");
// The reverse read: WK's fixture dates across a window that crosses
// the calendar exception (removed 2026-10-29) — the two directions
// must agree on the same relation.
const wkDates = calendar.invoke({
  op: "service_dates",
  network_hash: loadedCalendarNet.network_hash,
  service_type: "WK",
  from: "2026-10-23",
  to: "2026-11-03",
});
assert(
  JSON.stringify(wkDates.dates) ===
    JSON.stringify([
      "2026-10-23",
      "2026-10-26",
      "2026-10-27",
      "2026-10-28",
      "2026-10-30",
      "2026-11-02",
      "2026-11-03",
    ]),
  "the reverse read carries the calendar exception: no 2026-10-29",
);
const wkNone = calendar.invoke({
  op: "service_dates",
  network_hash: loadedCalendarNet.network_hash,
  service_type: "NOPE",
  from: "2026-10-23",
  to: "2026-11-03",
});
assert(wkNone.dates.length === 0, "an unknown service type answers empty, never errors");

// THE DIRECTORY — what exists in the network. The pickers and the
// names join are MODEL questions: the stop list (name-sorted, with
// the walking-group structure), the route list, and a route's
// PATTERNS with their ordered stops (what a diversion editor anchors
// against), grouped by the route's directions.
assert(byId.directory, "manifest advertises the directory capability");
assert(
  byId.directory.consumes.network === "network",
  "directory declares its registry edge: it consumes network documents",
);
const dir = shell(loadModule(byId.directory));
const loadedDirNet = dir.invoke({ op: "load_network", network: imported.network });
assert(
  loadedDirNet.network_hash === imported.network_hash,
  "directory agrees with the network capability on the document identity",
);
const dirStops = dir.invoke({ op: "stops", network_hash: loadedDirNet.network_hash, limit: null });
assert(dirStops.stops.length === 6, "the directory lists every stop");
const byStop = Object.fromEntries(dirStops.stops.map((st) => [st.stop_id, st]));
assert(byStop.SA.parent === "STN" && byStop.SB.parent === "STN", "the bays carry their parent");
assert(byStop.STN.parent == null, "the station pin carries none");
const names = dirStops.stops.map((st) => st.name);
assert(
  names.every((n, i) => i === 0 || names[i - 1] <= n),
  "the stop list is name-sorted (the pickers' shape)",
);
const dirNames = dir.invoke({
  op: "stops_by_ids",
  network_hash: loadedDirNet.network_hash,
  ids: ["B", "ghost", "STN"],
});
assert(
  dirNames.stops.map((st) => st.stop_id).join(",") === "B,STN",
  "the names join answers in request order, unknown ids omitted",
);
const dirRoutes = dir.invoke({ op: "routes", network_hash: loadedDirNet.network_hash });
assert(dirRoutes.routes.some((r) => r.route_id === "10"), "the route list carries route 10");
assert(
  dirRoutes.routes.every((r) => r.mode === "bus"),
  "the model's mode set, not GTFS's numeric codes",
);
const dirPatterns = dir.invoke({
  op: "route_stop_sequence",
  network_hash: loadedDirNet.network_hash,
  route_id: "10",
});
assert(
  dirPatterns.patterns.length === 3,
  "route 10's stop sequences are its patterns: the direct run with its shape and without, and the via-C run",
);
const longest = dirPatterns.patterns.reduce((a, b) => (b.stops.length > a.stops.length ? b : a));
assert(
  longest.stops.map((st) => st.stop_id).join(",") === "A,C,B",
  "the longest pattern carries the skippable interior (A→C→B)",
);
const ghostRoute = dir.invoke({
  op: "route_stop_sequence",
  network_hash: loadedDirNet.network_hash,
  route_id: "missing",
});
assert(ghostRoute.patterns.length === 0, "an unknown route answers empty, never an error");
const routeFocus = dir.invoke({ op: "focus", network_hash: loadedDirNet.network_hash, kind: "route", id: "10" }).focus;
assert(routeFocus.lines.length === 3 && routeFocus.points.length === 7, "a route frames every pattern it runs");
assert(dir.invoke({ op: "focus", network_hash: loadedDirNet.network_hash, kind: "stop", id: "missing" }).focus === null, "an unknown entity frames nothing");
const entities = (kind, query) => dir.invoke({ op: "entities", network_hash: loadedDirNet.network_hash, kind, query }).rows;
assert(entities("stop").length === 6, "the planner's stop list is every stop");
assert(entities("stop", "MAIN 2nd").map((r) => r.id).join(",") === "B", "every term narrows, case-insensitive");
assert(entities("pattern", "pat-10").every((r) => r.context === "10"), "a pattern is placed by its route");
const routeDetail = dir.invoke({ op: "route_detail", network_hash: loadedDirNet.network_hash, route_id: "10" }).detail;
assert(routeDetail.patterns.length === 3, "route 10's composition lists its three patterns");
assert(
  routeDetail.stops.map((r) => r.id).join(",") === "A,B,C",
  "the stops its patterns visit, first-visit order",
);
assert(
  routeDetail.pattern_trips_by_type.map((g) => g.service_type).join(",") === "SUN,WK" &&
    routeDetail.pattern_trips_by_type.flatMap((g) => g.pattern_trips).every((sc) => sc.trips > 0),
  "schedules grouped by service type, each with the trips it runs",
);
assert(dir.invoke({ op: "route_detail", network_hash: loadedDirNet.network_hash, route_id: "missing" }).detail === null, "an unknown route has no composition");
// The import keeps GTFS direction_id: route 10's patterns all run in
// direction "0", and its timetable on WK is every trip of them, by
// start, one time per column (null where a pattern does not stop).
assert(
  routeDetail.directions.length === 1 && routeDetail.directions[0].direction === "0" &&
    routeDetail.directions[0].patterns.length === routeDetail.patterns.length,
  "a route lists its patterns by direction",
);
const table = dir.invoke({ op: "timetable", network_hash: loadedDirNet.network_hash, route_id: "10", direction: "0", service_type: "WK" }).timetable;
assert(
  table.trips.length > 0 && table.trips.every((t) => t.times.length === table.stops.length) &&
    table.trips.every((t, i, all) => i === 0 || all[i - 1].start_secs <= t.start_secs),
  "a timetable is every trip of one direction on one service type, by start",
);
assert(
  dir.invoke({ op: "timetable", network_hash: loadedDirNet.network_hash, route_id: "10", direction: "9", service_type: "WK" }).timetable === null,
  "an unknown direction has no timetable",
);
const stopDetail = dir.invoke({ op: "stop_detail", network_hash: loadedDirNet.network_hash, stop_id: "A" }).detail;
assert(stopDetail.routes.some((r) => r.id === "10"), "a stop lists the routes whose patterns visit it");
assert(dir.invoke({ op: "stop_detail", network_hash: loadedDirNet.network_hash, stop_id: "missing" }).detail === null, "an unknown stop has no composition");

// Unknown hashes error, never panic — every day-view seam.
const unknownBoard = board.raw({ op: "stop_board", network_hash: "sha256:missing", stop_id: "A", date: DATE, from_secs: 0, limit: 5 });
assert(unknownBoard.error && unknownBoard.error.includes("unknown network"), "unknown hashes error, never panic");

// The street flow — where along the streets: a street layer in on
// the bulk seam (the e2e street fixture), read into the canonical
// streets document and registered under its hash; the document
// handed back on bulk op 1 is the same hash; then pure queries by
// hash. The Elbow route follows street geometry, not the diagonal;
// garbage bytes are an error outcome, never a trap.
assert(byId.streets, "manifest advertises the streets capability");
const streets = shell(loadModule(byId.streets));
const streetsBytes = readFileSync(path.join(here, "../../web/e2e/fixtures/streets.geojson"));
const loadedStreets = streets.invokeBytes(0, streetsBytes);
assert(loadedStreets.op === "import", "streets import answers on the bulk-bytes seam");
assert(loadedStreets.ways === 3 && loadedStreets.segments === 5 && loadedStreets.nodes === 8, "the fixture imports: 3 ways, 5 segments, 8 nodes");
assert(loadedStreets.streets_hash.startsWith("sha256:"), "the document registers under its content hash");
assert(loadedStreets.streets.ways.length === 3, "the import answers the document the host keeps");
const againStreets = streets.invokeBytes(1, new TextEncoder().encode(JSON.stringify(loadedStreets.streets)));
assert(againStreets.op === "load_streets", "load_streets answers on bulk op 1");
assert(againStreets.streets_hash === loadedStreets.streets_hash, "the document handed back is the same hash");
const snappedA = streets.invoke({
  op: "snap",
  streets_hash: loadedStreets.streets_hash,
  lat: 45.5040,
  lon: -73.5030,
});
assert(snappedA.position && snappedA.position.dist_m < 1e-6, "a click on the street snaps to it (dist 0)");
const snappedB = streets.invoke({
  op: "snap",
  streets_hash: loadedStreets.streets_hash,
  lat: 45.5080,
  lon: -73.5080,
});
const routed = streets.invoke({
  op: "route",
  streets_hash: loadedStreets.streets_hash,
  from: snappedA.position,
  to: snappedB.position,
});
assert(
  routed.points.length === 2 &&
    Math.abs(routed.points[0].lat - 45.5040) < 1e-9 &&
    Math.abs(routed.points[0].lon + 73.5080) < 1e-9,
  "the route bends at the Elbow corner, not the diagonal",
);
const nowhere = streets.invoke({ op: "snap", streets_hash: loadedStreets.streets_hash, lat: 44.0, lon: -70.0 });
assert(nowhere.position === null, "no street nearby answers null, never an error");
const unknownStreets = streets.raw({ op: "snap", streets_hash: "sha256:missing", lat: 0, lon: 0 });
assert(unknownStreets.error && unknownStreets.error.includes("unknown street network"), "unknown hashes error, never panic");
const garbageStreets = streets.raw({ op: "import", bytes: [0xff, 0xfe] });
assert(garbageStreets.error && /street layer/.test(garbageStreets.error), "garbage bytes are an error outcome, never a trap");

// The walk flow — a plan's transfers along the streets, composed by
// documents only: streets mints the walk table from a streets
// document and the network (the walking fixture: a residential
// street through the station's bays, a motorway across it no rider
// walks), the router registers it under the same hash and plans over
// it. Two capabilities, one document between them.
const walkStreets = streets.invokeBytes(0, readFileSync(path.join(here, "../../web/e2e/fixtures/walk-streets.geojson")));
const streetsNet = streets.invoke({ op: "load_network", network: imported.network });
assert(streetsNet.network_hash === imported.network_hash, "streets agrees with the network capability on the document identity");
const walked = streets.invoke({ op: "walks", streets_hash: walkStreets.streets_hash, network_hash: streetsNet.network_hash });
const walkPairs = walked.walks.walks.map((w) => `${w.from}>${w.to}`).join(" ");
assert(walkPairs === "SA>SB SA>STN SB>SA SB>STN STN>SA STN>SB", `the bays and their station walk Station Rd (${walkPairs})`);
assert(walked.walks.max_walk_secs === 600 && walked.walks.speed_mps === 1.3, "the walk defaults: ten minutes at 1.3 m/s");
const routerWalks = router.invoke({ op: "load_walks", walks: walked.walks });
assert(routerWalks.walks_hash === walked.walks_hash, "router agrees with streets on the walk table's identity");
const bayWalk = router.invoke({
  op: "plan",
  network_hash: loadedRouterNet.network_hash,
  from: "SA",
  to: "SB",
  date: DATE,
  depart_secs: 5 * 3600,
  max_legs: 2,
  walks_hash: routerWalks.walks_hash,
});
const sabWalk = walked.walks.walks.find((w) => w.from === "SA" && w.to === "SB").walk_secs;
assert(
  bayWalk.itinerary?.legs.length === 1 && bayWalk.itinerary.legs[0].type === "walk" && bayWalk.itinerary.arrival_secs === 5 * 3600 + sabWalk,
  "the plan walks the table's walk between the bays",
);
const foreignWalks = router.raw({ op: "plan", network_hash: loadedRouterNet.network_hash, from: "SA", to: "SB", date: DATE, depart_secs: 0, max_legs: 2, walks_hash: "sha256:missing" });
assert(foreignWalks.error && foreignWalks.error.includes("load_walks first"), "an unknown walk table is an error, never a panic");
// A trip from any point: streets answers a dropped pin's walks to
// the stops near it, the router plans from the pin as a place —
// the walks pass as they are.
const pin = { lat: 45.5152, lon: -73.5165 };
const access = streets.invoke({ op: "access", streets_hash: walkStreets.streets_hash, network_hash: streetsNet.network_hash, point: pin });
const reached = access.walks.map((w) => w.stop_id).join(" ");
assert(reached === "SA STN SB", `the pin walks to the bays and the station, nearest first (${reached})`);
assert(access.walks.every((w) => w.path[0].lat === pin.lat && w.path[0].lon === pin.lon), "each walk's path starts at the pin");
const fromPin = router.invoke({
  op: "plan",
  network_hash: loadedRouterNet.network_hash,
  from: { id: "pin", name: "Dropped pin", walks: access.walks },
  to: "SB",
  date: DATE,
  depart_secs: 5 * 3600,
  max_legs: 2,
  walks_hash: routerWalks.walks_hash,
});
const pinWalk = access.walks.find((w) => w.stop_id === "SB").walk_secs;
assert(
  fromPin.itinerary?.from_stop_id === "pin" && fromPin.itinerary.legs[0].from_stop_id === "pin" && fromPin.itinerary.arrival_secs === 5 * 3600 + pinWalk,
  "the plan walks from the pin, along the streets",
);

// The sync flow — the protocol's one step: the client's draft
// diversion against an empty server converges to the submitted
// document (LWW union, rev bumped, host clock stamped), and the
// pair order never matters (client and server are roles, not
// order). Validation is NOT here: load_edits answers it against
// the network in every model-side module.
assert(byId.sync, "manifest advertises the sync capability");
const sync = shell(loadModule(byId.sync));
const draft = {
  id: "e1",
  diversion: {
    id: "d1", route_id: "10", direction_id: null,
    first_kept_stop: "A", last_kept_stop: "B",
    skipped_stops: ["C"], detour: null,
    added_secs: 300, start_date: DATE, end_date: DATE,
  },
  status: "draft", rev: 1, updated_at: 1,
};
const converged = sync.invoke({ op: "sync_edits", client: [draft], server: [], now_ms: 1000 });
assert(converged.op === "sync_edits" && converged.records.length === 1, "the pair converges to one document");
assert(converged.records[0].status === "submitted", "drafts submit");
assert(converged.records[0].rev === 2, "submit bumps rev");
assert(converged.records[0].updated_at === 1000, "submit stamps the host clock");
const swapped = sync.invoke({ op: "sync_edits", client: [], server: [draft], now_ms: 1000 });
assert(
  JSON.stringify(swapped.records) === JSON.stringify(converged.records),
  "the pair is order-insensitive (the same documents converge the same way)",
);
const empty = sync.invoke({ op: "sync_edits", client: [], server: [], now_ms: 0 });
assert(empty.records.length === 0, "empty documents converge to empty");
const garbageSync = sync.raw({ op: "sync_edits", client: "not a list", server: [], now_ms: 0 });
assert(garbageSync.error && garbageSync.error.includes("invalid request"), "garbage requests are an error outcome, never a trap");

// The answer carries the canonical document bytes and their hash — the
// module is the hash authority, so a host stores exact bytes and never
// mints document identity (docs/FORMATS.md).
assert(
  typeof converged.document === "string" &&
    converged.document_hash === "sha256:" + createHash("sha256").update(converged.document).digest("hex"),
  "document_hash is sha256 of the canonical document bytes",
);

// D-56: the receive path clamps the client clock — a record stamped far
// in the future is treated as written at now_ms (and clamping is
// idempotent, so redelivery converges).
const future = { ...draft, status: "accepted", rev: 9, updated_at: 1e15 };
const clamped = sync.invoke({ op: "sync_edits", client: [future], server: [], now_ms: 2000 });
assert(clamped.records[0].updated_at === 2000, "a future-dated record is clamped at receive");
const reclamped = sync.invoke({ op: "sync_edits", client: [clamped.records[0]], server: [], now_ms: 3000 });
assert(reclamped.records[0].updated_at === 2000, "an already-clamped stamp is untouched (idempotent)");

// The authorization flow — the one question: may this identity do this
// on this pointer? Fail-closed: a matching rule allows, anything else
// denies (empty policy, missing role, wrong action/pointer).
assert(byId.authz, "manifest advertises the authz capability");
const authz = shell(loadModule(byId.authz));
const policy = {
  schema_version: 1,
  rules: [
    { name: "ops-write-edits", roles: ["dispatcher"], pointers: ["edits"], actions: ["write"] },
    { name: "admin-all", roles: ["admin"], pointers: ["*"], actions: ["*"] },
  ],
};
const allowed = authz.invoke({
  op: "authorize",
  identity: { sub: "dispatcher-7", roles: ["dispatcher"] },
  action: "write", pointer: "edits", policy,
});
assert(allowed.decision.allow === true && allowed.decision.matched_rule === "ops-write-edits", "the matching rule allows");
const denied = authz.invoke({
  op: "authorize",
  identity: { sub: "rider-1", roles: ["rider"] },
  action: "write", pointer: "edits", policy,
});
assert(denied.decision.allow === false, "no matching rule denies");
const noPolicy = authz.invoke({
  op: "authorize",
  identity: { sub: "admin-1", roles: ["admin"] },
  action: "write", pointer: "edits", policy: { schema_version: 1, rules: [] },
});
assert(noPolicy.decision.allow === false, "empty policy is fail-closed");
const badAction = authz.raw({
  op: "authorize",
  identity: { sub: "a", roles: ["admin"] },
  action: "drop", pointer: "edits", policy,
});
assert(badAction.error && badAction.error.includes("unknown action"), "an unknown action is an error, never an allow");

// The projection: the policy as it applies to ONE identity — its roles
// and merged capabilities only, never other roles or rules.
const eff = authz.invoke({
  op: "effective",
  identity: { sub: "dispatcher-7", roles: ["dispatcher"] },
  policy,
});
assert(
  eff.effective.roles.length === 1 &&
    eff.effective.capabilities.length === 1 &&
    eff.effective.capabilities[0].pointer === "edits" &&
    !JSON.stringify(eff.effective).includes("admin"),
  "effective projects only the caller's own capabilities",
);

// The plan-chain flow — which pick governs: the spring pick in
// force on the fixture Friday, the fall pick after it, the windows
// derived (never stored), and the chain validating clean.
assert(byId.plans, "manifest advertises the plans capability");
const plans = shell(loadModule(byId.plans));
const PLAN_SET = [
  { id: "spring", name: "Spring 2026", effective_from: "2026-03-01" },
  { id: "fall", name: "Fall 2026", effective_from: "2026-09-07" },
];
const resolved = plans.invoke({ op: "resolve", plans: PLAN_SET, date: DATE });
assert(resolved.plan === "fall", "the latest pick at or before the date governs");
const before = plans.invoke({ op: "resolve", plans: PLAN_SET, date: "2026-01-01" });
assert(before.plan === null, "no pick yet answers null");
const unversioned = plans.invoke({
  op: "resolve",
  plans: [{ id: "", name: "imported", effective_from: null }],
  date: DATE,
});
assert(unversioned.plan === null, "unversioned plans never resolve");
const windows = plans.invoke({ op: "periods", plans: [PLAN_SET[1], PLAN_SET[0]] });
assert(windows.periods.length === 2, "unversioned excluded, duplicates are validate's business");
assert(windows.periods[0].id === "spring" && windows.periods[0].until === "2026-09-07", "windows are derived and sorted by pick date");
const valid = plans.invoke({ op: "validate", plans: PLAN_SET });
assert(valid.issues.length === 0, "a valid chain has no issues");
const chainIssues = plans.invoke({
  op: "validate",
  plans: [...PLAN_SET, { id: "spring", name: "dup", effective_from: "2026-05-01" }],
});
assert(
  chainIssues.issues.some((i) => i.kind === "duplicate_plan_id"),
  "every invariant violation is reported, never silent",
);
const garbagePlans = plans.raw({ op: "resolve", plans: [], date: "not a date" });
assert(garbagePlans.error && garbagePlans.error.includes("invalid date"), "garbage dates are an error outcome, never a trap");

// The authoring flow — what does this network become after these
// edits? The imported document stamped as a pick and renamed in one
// atomic change; the answer is a NEW identity every module agrees on
// (the codec module re-registers the answered document and returns
// the same hash), and a refused removal names its edit.
assert(byId.author, "manifest advertises the author capability");
const author = shell(loadModule(byId.author));
const authorNet = author.invoke({ op: "load_network", network: imported.network });
assert(authorNet.network_hash === imported.network_hash, "author agrees on the imported document's identity");
const changed = author.invoke({
  op: "apply",
  network_hash: authorNet.network_hash,
  edits: [
    { edit: "set_plan", plan: { id: "fall", name: "Fall 2026", effective_from: DATE } },
    { edit: "set_name", name: "Fall pick" },
  ],
});
assert(changed.network_hash !== authorNet.network_hash, "a change is a new document");
assert(changed.network.plan.id === "fall" && changed.network.name === "Fall pick", "the edits applied in order");
assert(changed.issues.length === 0, "the stamped pick stays valid");
const recount = network.invoke({ op: "load_network", network: changed.network });
assert(recount.network_hash === changed.network_hash, "the codec module agrees on the authored identity (one document, one truth)");
const firstStop = Object.keys(imported.network.stops).find((id) =>
  Object.values(imported.network.patterns).some((p) => p.stops.some((s) => s.stop === id)),
);
const refusedEdit = author.raw({ op: "apply", network_hash: authorNet.network_hash, edits: [{ edit: "remove_stop", id: firstStop }] });
assert(refusedEdit.error && refusedEdit.error.includes("edit 0"), "a referenced stop is never hard-deleted out from under a pattern");
// The gestures carry the model's rules: a minted id comes back in
// `created`, and a new timepoint keeps every timing profile aligned.
const gestured = author.invoke({
  op: "apply",
  network_hash: authorNet.network_hash,
  edits: [{ edit: "add_stop", lat: 45.5, lon: -73.5 }, { edit: "add_route" }],
});
assert(
  gestured.created.map((c) => `${c.kind}:${c.id}`).join() === "stop:s1,route:r1",
  "creation mints the first free ids and reports them",
);
// Insert a visit (a non-timepoint, before the last stop), then make
// it a timepoint: every profile gains its time.
const [patternId, pattern] = Object.entries(gestured.network.patterns)[0];
const inserted = pattern.stops.length - 1; // where the new visit lands
const toggled = author.invoke({
  op: "apply",
  network_hash: gestured.network_hash,
  edits: [
    { edit: "insert_pattern_stop", pattern: patternId, stop: "s1" },
    { edit: "toggle_timepoint", pattern: patternId, index: inserted },
  ],
}).network.patterns[patternId];
const timepoints = toggled.stops.filter((st) => st.timepoint).length;
assert(
  toggled.stops[inserted].stop === "s1" && toggled.stops[inserted].timepoint &&
    toggled.timing.every((t) => t.at_timepoints.length === timepoints),
  "a new timepoint keeps every timing profile aligned",
);
// The timetable's gestures address route + direction + service type:
// route 10 toward Main & 2nd every 20 min 08:00–09:00 keeps the 08:00
// and 08:40 trips and adds 08:20 on the pattern before it — and the
// directory's timetable of the answer has all three.
const frequent = author.invoke({
  op: "apply",
  network_hash: authorNet.network_hash,
  edits: [{ edit: "set_headway", route: "10", direction: "0", service_type: "WK", from_secs: 28800, to_secs: 32400, headway_secs: 1200 }],
});
assert(frequent.issues.length === 0, "a higher frequency keeps the runcut valid");
const frequentDir = dir.invoke({ op: "load_network", network: frequent.network });
const frequentTable = dir.invoke({ op: "timetable", network_hash: frequentDir.network_hash, route_id: "10", direction: "0", service_type: "WK" }).timetable;
assert(
  frequentTable.trips.map((t) => `${t.start_secs}:${t.pattern}`).join() === "28800:pat-10-0-1,30000:pat-10-0-1,31200:pat-10-0-2",
  "set_headway fills the window on the neighbour's pattern, and the timetable reads it",
);
const marked = dir.invoke({ op: "timetable", network_hash: frequentDir.network_hash, route_id: "10", direction: "0", service_type: "WK", compare_hash: loadedDirNet.network_hash }).timetable;
assert(
  marked.trips.map((t) => t.added).join() === "false,true,false" && marked.removed.length === 0,
  "against its baseline, the timetable marks the added trip",
);
analysis.invoke({ op: "load_network", network: imported.network });
const analysisFrequent = analysis.invoke({ op: "load_network", network: frequent.network });
const compared = analysis.invoke({ op: "service_type_compare", base_hash: imported.network_hash, network_hash: analysisFrequent.network_hash, service_type: "WK" }).comparison;
assert(
  compared.proposal.trips === compared.base.trips + 1 && compared.routes.length === 1 && compared.routes[0].route_id === "10",
  "the comparison names the one route the change touched",
);

// The diversion-authoring flow — what does this draft amount to:
// the default pattern, the interior skipped unless served, the
// record proposed then retired then restored (the envelope rules),
// and a stored document read under the current schema.
assert(byId.divert, "manifest advertises the divert capability");
const divert = shell(loadModule(byId.divert));
const divertNet = divert.invoke({ op: "load_network", network: imported.network });
assert(divertNet.network_hash === imported.network_hash, "divert agrees on the imported document's identity");
const divertDraft = { route_id: "10", served_stops: [], slices: [], added_mins: 5, start_date: DATE, end_date: DATE };
const blank = divert.invoke({ op: "draft", network_hash: divertNet.network_hash, draft: divertDraft }).view;
assert(blank.pattern && blank.stops.length >= 3 && blank.problems.length === 1, "a route alone answers the default pattern and what is missing");
const ends = { first_kept_stop: blank.stops[0].stop_id, last_kept_stop: blank.stops.at(-1).stop_id };
const anchored = divert.invoke({ op: "draft", network_hash: divertNet.network_hash, draft: { ...divertDraft, ...ends } }).view;
assert(
  anchored.problems.length === 0 && anchored.skipped_stops.length === blank.stops.length - 2,
  "the stops between the anchors are skipped unless served",
);
const proposed = divert.invoke({
  op: "propose", network_hash: divertNet.network_hash, draft: { ...divertDraft, ...ends },
  records: [], record_id: "e9", now_ms: 5000,
}).records;
assert(proposed.length === 1 && proposed[0].status === "draft" && proposed[0].diversion.added_secs === 300, "propose builds the record");
const rows = divert.invoke({ op: "list", network_hash: divertNet.network_hash, records: proposed }).rows;
assert(rows.length === 1 && rows[0].in_force && rows[0].skipped_names.length === anchored.skipped_stops.length, "the list names the records");
const retiredDoc = divert.invoke({ op: "retire", records: proposed, record_id: "e9", now_ms: 6000 }).records;
assert(retiredDoc[0].status === "retired" && retiredDoc[0].rev === 2, "retire is the tombstone, rev bumped");
const restoredDoc = divert.invoke({ op: "restore", records: retiredDoc, record_id: "e9", now_ms: 7000 }).records;
assert(restoredDoc[0].status === "draft" && restoredDoc[0].rev === 3, "restore puts it back in force");
const stored = divert.invoke({ op: "sanitize", records: [restoredDoc[0], { id: "junk" }] });
assert(stored.records.length === 1 && stored.dropped === 1, "a stored document drops what does not parse");
const unready = divert.raw({ op: "propose", network_hash: divertNet.network_hash, draft: divertDraft, records: [], record_id: "x", now_ms: 1 });
assert(unready.error && unready.error.includes("first stop"), "an unready draft is refused, naming why");


// THE KERNEL BULK OP — every network consumer registers the bare
// document on transit_invoke_bytes op 256 (no request envelope),
// answering the import's identity; garbage is an error, never a trap.
const OP_LOAD_NETWORK = 256;
const networkBytes = Buffer.from(JSON.stringify(imported.network));
for (const id of ["network", "analysis", "board", "runcut", "calendar", "router", "directory", "author", "divert"]) {
  const fresh = shell(loadModule(byId[id]));
  const viaBytes = fresh.invokeBytes(OP_LOAD_NETWORK, networkBytes);
  assert(viaBytes.network_hash === imported.network_hash, `${id} answers the kernel's load_network bulk op with the import's identity`);
  let refused = null;
  try {
    fresh.invokeBytes(OP_LOAD_NETWORK, Buffer.from("{not a network"));
  } catch (error) {
    refused = String(error);
  }
  assert(refused?.includes("invalid document"), `${id} refuses a garbage document on the bulk op`);
}

console.log("capability modules: ABI smoke ok");
