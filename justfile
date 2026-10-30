# rustwasm — task runner. Run `just` (or `just --list`) to see all
# recipes. The project has four layers, one recipe group each:
#
#   Rust  — the transit-* libraries and cap-* capability modules (cargo)
#   Go    — server/ (the sync server: wazero host over cap-sync, proxy)
#   Node  — crates/cap-abi/caps-smoke.mjs (runs the modules directly)
#   Web   — web/ (Svelte + Vite) and Playwright e2e
#
# There is no monolith artifact: every
# host instantiates the capability modules it needs — the browser
# fetches them from web/public/caps/, the sync server hosts
# cap-sync, cap-network and cap-streets from web/pkg/.

default:
    @just --list

# ---------------------------------------------------------------
# build
# ---------------------------------------------------------------

# Build the capability modules (the capability bus: horizontal
# capabilities assembled per user) and regenerate the capability
# manifest — hash-pinned modules in web/public/caps/, served under
# their content hash so the HTTP cache holds them forever. The sync
# server's modules also land at stable paths in web/pkg/ (cap_sync,
# cap_network, cap_streets — the server's default -wasm,
# -network-wasm and -streets-wasm flags; rebuilt with the modules,
# never by hand).
caps:
    #!/usr/bin/env bash
    set -e
    cargo build --release -p cap-gtfs -p cap-router -p cap-analysis -p cap-network -p cap-rt -p cap-board -p cap-runcut -p cap-calendar -p cap-directory -p cap-streets -p cap-sync -p cap-plans -p cap-author -p cap-divert --target wasm32-unknown-unknown
    cargo run --quiet -p cap-abi --bin gen-manifest
    mkdir -p web/pkg
    cp target/wasm32-unknown-unknown/release/cap_sync.wasm web/pkg/cap_sync.wasm
    cp target/wasm32-unknown-unknown/release/cap_network.wasm web/pkg/cap_network.wasm
    cp target/wasm32-unknown-unknown/release/cap_streets.wasm web/pkg/cap_streets.wasm

# Publish the registry: gates first (the records report them), then
# build and publish — immutable blobs, evidence records per
# capability, and the pointer. This is the artifact an agent (or a
# human dogfooding `caps run`) pulls compute from.
registry:
    #!/usr/bin/env bash
    set -e
    export TS_RS_EXPORT_DIR="$PWD/web/src/types"
    cargo test
    node crates/cap-abi/caps-smoke.mjs
    CAPS_PUBLISH_GATED=1 just caps

# Build everything the server serves: artifact + frontend (web/dist).
# Regenerates the TS bindings, then type-checks — the TS layer is
# part of the gate, checked against the contract as the Rust stands.
build: types
    cd web && npm run typecheck
    cd web && npm run build

# Regenerate web/src/types/*.ts from the Rust model (ts-rs). The TS
# contract is generated, never hand-written, and never committed
# (gitignored). The directory is wiped first: a type deleted from the
# Rust must not linger as a stale file the frontend still compiles
# against.
types:
    #!/usr/bin/env bash
    set -e
    rm -rf web/src/types
    export TS_RS_EXPORT_DIR="$PWD/web/src/types"
    cargo test --workspace export_bindings >/dev/null
    echo "$(ls web/src/types | wc -l) type files in web/src/types"

# ---------------------------------------------------------------
# test
# ---------------------------------------------------------------

# The host-side runner: Rust suites, Go -race, artifact smoke in
# Node, capability-module smoke (needs `caps` built — hash-pinned
# modules and the manifest the smoke verifies against), runner
# gates, web parity ratchets. No browser — the gate for logic
# changes under a stable op surface; pair with a real-data caps-run
# session (docs/TESTING.md, dogfood QA). Browser cover is `e2e`.
# TS_RS_EXPORT_DIR keeps the ts-rs bindings regenerating into
# web/src/types on every run (.cargo/config.toml pins the same path
# for plain cargo runs; without either, ts-rs scatters them into
# crates/*/bindings).
test: caps
    #!/usr/bin/env bash
    set -e
    export TS_RS_EXPORT_DIR="$PWD/web/src/types"
    cargo test
    node crates/cap-abi/caps-smoke.mjs
    # The runner knows which tree its registry was built from
    # (QAF-77): just published, the registry's source digest (Rust,
    # gen-manifest) equals the tree's (JS, caps-run) — nothing on
    # stderr.
    test -z "$(node tools/caps-run.mjs list 2>&1 >/dev/null)"
    # The agent runner, gated with chained sessions — every op
    # references its documents through $load/$op result references
    # (no hand-copied hashes; the runner resolves them in-session).
    # The directory capability's gate: the pickers' one question over
    # the network document, one chained session — stops, the names
    # join, the places nearest a point (a bay answers as its
    # station), routes, and route 10's patterns. grep reads the whole
    # answer (no -q: quitting early breaks the runner's pipe).
    node tools/caps-run.mjs directory \
      --op @web/public/caps/sample-network-load.json \
      --op '{"op":"stops","network_hash":"$load_network.network_hash","limit":2}' \
      --op '{"op":"stops_by_ids","network_hash":"$load_network.network_hash","ids":["A","ghost","STN"]}' \
      --op '{"op":"stops_near","network_hash":"$load_network.network_hash","lat":45.515,"lon":-73.5155,"limit":3}' \
      --op '{"op":"routes","network_hash":"$load_network.network_hash"}' \
      --op '{"op":"route_stop_sequence","network_hash":"$load_network.network_hash","route_id":"10"}' \
      | grep '"stop_id":"A"' > /dev/null
    # An oracle's answer chains into the ops (--ask; a data: URL
    # stands in for the deployment's geocoder): the rider typed an
    # address, picked the first, and asks what is near it.
    node tools/caps-run.mjs directory \
      --op @web/public/caps/sample-network-load.json \
      --ask 'data:application/json,{"features":[{"geometry":{"coordinates":[-73.5155,45.515]},"properties":{"name":"200 Station Rd"}}]}' \
      --op '{"op":"stops_near","network_hash":"$load_network.network_hash","lat":"$ask.features.0.geometry.coordinates.1","lon":"$ask.features.0.geometry.coordinates.0","limit":1}' \
      | grep '"stop_id":"STN"' > /dev/null
    # The board capability's gate: the station pin answers the union
    # of its bays (walking-group pooling), and the personal board
    # chains its watchlist document — all references chained, no
    # hand-copied hashes.
    node tools/caps-run.mjs board \
      --op @web/public/caps/sample-network-load.json \
      --op '{"op":"stop_board","network_hash":"$load_network.network_hash","stop_id":"STN","date":"2026-10-23","from_secs":25200,"limit":10}' \
      --op '{"op":"load_watchlist","document":{"version":"v2","stops":[{"id":"A","label":"home"}]}}' \
      --op '{"op":"watchlist_board","watchlist_hash":"$load_watchlist.watchlist_hash","network_hash":"$load_network.network_hash","date":"2026-10-23","from_secs":25200,"limit":3}' \
      | grep -q '"label":"home"'
    # The network capability's gate: load → import, chained; the
    # dropped single-stop trip must be REPORTED (never silent).
    node tools/caps-run.mjs network --load web/public/caps/sample-gtfs.zip \
      --op '{"op":"import","feed_hash":"$load.feed_hash"}' | grep -q '"kind":"dropped_trips"'
    # The edit vocabulary reads through the runner: `tools author`
    # carries one worked example per edit kind (QAF-52), each applied
    # to the fixture when the manifest was generated.
    node tools/caps-run.mjs tools author \
      | node -e 'const apply = JSON.parse(require("fs").readFileSync(0, "utf8")).tools.apply; if (!apply.kinds?.some((k) => k.kind === "add_trip" && k.example.edit === "add_trip")) process.exit(1)'
    # The feed's identity — the feed document capability's one
    # question.
    node tools/caps-run.mjs gtfs --load web/public/caps/sample-gtfs.zip \
      --op '{"op":"feed_info","feed_hash":"$load.feed_hash"}' \
      | grep -q '"feed_version":"2026.10.1"'
    # The realtime capability's gate: the dispatch leg as a JOIN — the
    # imported network and its boundary, the fixture payload on op 0,
    # and the dispatcher's most-asked question answered in one op
    # (t10's predicted departure at A: 08:00 + 120 s).
    node tools/caps-run.mjs network --load web/public/caps/sample-gtfs.zip \
      --as net --op '{"op":"import","feed_hash":"$load.feed_hash"}' \
      --cap rt --as rnet --op '{"op":"load_network","network":"$net.network"}' \
      --as bnd --op '{"op":"load_boundary","boundary":"$net.boundary"}' \
      --as snap --load web/e2e/fixtures/sample-rt.pb \
      --op '{"op":"trip_progress","network_hash":"$rnet.network_hash","boundary_hash":"$bnd.boundary_hash","rt_hash":"$snap.rt_hash","trip_id":"t10"}' \
      | grep -q '"predicted_departure":28920'
    # The clock: $TODAY resolves host-side (the runner is the host) —
    # a valid date must answer, whatever day the gate runs on.
    node tools/caps-run.mjs calendar \
      --op @web/public/caps/sample-network-load.json \
      --op '{"op":"active_services","network_hash":"$load_network.network_hash","date":"$TODAY"}' \
      | grep -q '"op":"active_services"'
    # The calendar read backwards: WK's fixture dates across a window
    # that crosses the feed's calendar exception (removed 10-29) —
    # the reverse read must carry it.
    node tools/caps-run.mjs calendar \
      --op @web/public/caps/sample-network-load.json \
      --op '{"op":"service_dates","network_hash":"$load_network.network_hash","service_type":"WK","from":"2026-10-23","to":"2026-11-03"}' \
      | grep -q '"dates":\["2026-10-23","2026-10-26","2026-10-27","2026-10-28","2026-10-30","2026-11-02","2026-11-03"\]'
    # The stop's day as gaps (QAF-25): stop A's three fixture
    # departures (08:00, 08:20, 08:40) fold into two 1200 s gaps —
    # the answer a jq fold over the board used to approximate.
    node tools/caps-run.mjs analysis \
      --op @web/public/caps/sample-network-load.json \
      --op '{"op":"stop_gaps","network_hash":"$load_network.network_hash","date":"2026-10-23","stop_id":"A"}' \
      | grep -q '"departures":3,"first_departure_secs":28800'
    # The day's hourly profile (QAF-4): all five fixture trips start
    # in hour 8 under WK — one hour row answers what the jq fold
    # used to approximate.
    node tools/caps-run.mjs analysis \
      --op @web/public/caps/sample-network-load.json \
      --op '{"op":"hourly_trips","network_hash":"$load_network.network_hash","date":"2026-10-23"}' \
      | grep -q '"hour":8,"service_types":\[{"service_type":"WK","trips":5}\],"trips":5'
    # The schedule texture (QAF-16): WK's five fixture schedules —
    # each a single trip (no gaps to read), two starts on the
    # quarter-hour grid (28800, 29700), three off it.
    node tools/caps-run.mjs analysis \
      --op @web/public/caps/sample-network-load.json \
      --op '{"op":"schedule_texture","network_hash":"$load_network.network_hash","service_type":"WK"}' \
      | grep -q '"on_clockface":2,"pattern_trips":\['
    # The corridor read (QAF-22): routes 10 and 20 share the A→B
    # segment and stack three departures at A — the pair answered,
    # not inferred from stop data.
    node tools/caps-run.mjs analysis \
      --op @web/public/caps/sample-network-load.json \
      --op '{"op":"route_overlap","network_hash":"$load_network.network_hash","date":"2026-10-23"}' \
      | grep -q '"a":"10","b":"20","shared_segments":1,"shared_stops":2,"stack_departures":3,"stack_stop":"A"'
    # The timed-transfer view (QAF-27): route 10's 08:00 arrival at
    # A meets route 20's 08:20 departure under a caller-brought
    # 25-minute window (the default convention caps at 15) — the
    # hub's pairs, not just pooled departures.
    node tools/caps-run.mjs analysis \
      --op @web/public/caps/sample-network-load.json \
      --op '{"op":"timed_transfers","network_hash":"$load_network.network_hash","date":"2026-10-23","stop_id":"A","window":{"min_secs":60,"max_secs":1500}}' \
      | grep -q '"from_route":"10","to_route":"20","wait_secs":1200'
    # The cross-capability session: one session imports the fixture
    # feed, hands the DOCUMENT to the board by reference (no file in
    # between), addresses results by --as name, and feeds the numeric
    # clock to a seconds field — the identities must agree across the
    # two modules (one document, one truth).
    node tools/caps-run.mjs network --load web/public/caps/sample-gtfs.zip \
      --as net --op '{"op":"import","feed_hash":"$load.feed_hash"}' \
      --cap board --as board_net --op '{"op":"load_network","network":"$net.network"}' \
      --op '{"op":"stop_board","network_hash":"$board_net.network_hash","stop_id":"A","date":"$TODAY","from_secs":"$NOW_SECS","limit":1}' \
      | node -e 'const r = require("fs").readFileSync(0, "utf8").trim().split("\n").map(JSON.parse); const id = (n) => r.find((x) => x.as === n).ok.network_hash; if (id("net") !== id("board_net") || r.at(-1).ok?.op !== "stop_board") process.exit(1)'
    # The live board — the parity chain every host runs (browser,
    # runner, server): import → cap-rt resolves the rt snapshot against
    # the network into THE live document → the board reads it by hash.
    # 08:01 at A: t10 (08:00, +120 s) is still to come, expected 08:02.
    node tools/caps-run.mjs network --as feed --load web/public/caps/sample-gtfs.zip \
      --as net --op '{"op":"import","feed_hash":"$feed.feed_hash"}' \
      --cap rt --as rnet --op '{"op":"load_network","network":"$net.network"}' \
      --as bnd --op '{"op":"load_boundary","boundary":"$net.boundary"}' \
      --as snap --load web/e2e/fixtures/sample-rt.pb \
      --as live --op '{"op":"live","network_hash":"$rnet.network_hash","boundary_hash":"$bnd.boundary_hash","rt_hash":"$snap.rt_hash"}' \
      --cap board --as bnet --op '{"op":"load_network","network":"$net.network"}' \
      --as blive --op '{"op":"load_live","live":"$live.live"}' \
      --op '{"op":"stop_board","network_hash":"$bnet.network_hash","stop_id":"A","date":"2026-10-23","from_secs":28860,"limit":3,"live_hash":"$blive.live_hash"}' \
      | node -e 'const r = require("fs").readFileSync(0, "utf8").trim().split("\n").map(JSON.parse); const ok = (n) => r.find((x) => x.as === n).ok; const row = r.at(-1).ok.board.rows.find((x) => x.expected_secs != null); if (ok("live").live_hash !== ok("blive").live_hash || !row || row.expected_secs !== 28920 || row.status !== "late") process.exit(1)'
    # The stop display's board: the same live chain into grouped_board.
    # 08:01 at A: the 10 → Downtown group leads with t10 (+120 s,
    # expected 08:02) — a minute away, "1 min" by the op's wait.
    node tools/caps-run.mjs network --as feed --load web/public/caps/sample-gtfs.zip \
      --as net --op '{"op":"import","feed_hash":"$feed.feed_hash"}' \
      --cap rt --as rnet --op '{"op":"load_network","network":"$net.network"}' \
      --as bnd --op '{"op":"load_boundary","boundary":"$net.boundary"}' \
      --as snap --load web/e2e/fixtures/sample-rt.pb \
      --as live --op '{"op":"live","network_hash":"$rnet.network_hash","boundary_hash":"$bnd.boundary_hash","rt_hash":"$snap.rt_hash"}' \
      --cap board --as bnet --op '{"op":"load_network","network":"$net.network"}' \
      --as blive --op '{"op":"load_live","live":"$live.live"}' \
      --op '{"op":"grouped_board","network_hash":"$bnet.network_hash","stop_id":"A","date":"2026-10-23","from_secs":28860,"live_hash":"$blive.live_hash"}' \
      | node -e 'const r = require("fs").readFileSync(0, "utf8").trim().split("\n").map(JSON.parse); const g = r.at(-1).ok?.board.groups.find((x) => x.route === "10"); const d = g?.departures[0]; if (!d || d.trip !== "t10" || d.wait.kind !== "minutes" || d.wait.minutes !== 1 || g.departures.length > 3) process.exit(1)'
    # The runcut capability's gate: the day's blocks (the index) into
    # one block's chain (the detail), with the edit ripple chained in
    # session (the connection-health read: 600 − 300).
    node tools/caps-run.mjs runcut \
      --op @web/public/caps/sample-network-load.json \
      --op '{"op":"blocks","network_hash":"$load_network.network_hash","date":"2026-10-23"}' \
      --op '{"op":"load_edits","network_hash":"$load_network.network_hash","records":[{"id":"e1","diversion":{"id":"d1","route_id":"10","direction_id":null,"first_kept_stop":"A","last_kept_stop":"B","skipped_stops":["C"],"detour":null,"added_secs":300,"start_date":"2026-10-23","end_date":"2026-10-23"},"status":"accepted","rev":1,"updated_at":1}]}' \
      --op '{"op":"block_trips","network_hash":"$load_network.network_hash","edits_hash":"$load_edits.edits_hash","block_id":"B1","date":"2026-10-23"}' \
      | grep -q '"layover_remaining_secs":300'
    # The map's geometry is answered, never computed by a host (W4):
    # a plan's ridden segment (router), the diverted 08:40 trip drawn
    # along the diversion's traced detour from A (runcut), and a route
    # framed by its two patterns (directory) — one chained session.
    node tools/caps-run.mjs router \
      --op @web/public/caps/sample-network-load.json \
      --as plan --op '{"op":"plan","network_hash":"$load_network.network_hash","from":"A","to":"B","date":"2026-10-23","depart_secs":25200,"max_legs":4}' \
      --op '{"op":"itinerary_paths","network_hash":"$load_network.network_hash","date":"2026-10-23","itinerary":"$plan.itinerary"}' \
      --cap runcut --op @web/public/caps/sample-network-load.json \
      --as edits --op '{"op":"load_edits","network_hash":"$load_network.network_hash","records":[{"id":"e1","diversion":{"id":"d1","route_id":"10","direction_id":null,"first_kept_stop":"A","last_kept_stop":"B","skipped_stops":["C"],"detour":[{"lat":45.6,"lon":-73.6}],"added_secs":300,"start_date":"2026-10-23","end_date":"2026-10-23"},"status":"accepted","rev":1,"updated_at":1}]}' \
      --as trip --op '{"op":"trip_path","network_hash":"$load_network.network_hash","edits_hash":"$edits.edits_hash","trip":"t30","start_secs":31200,"date":"2026-10-23"}' \
      --cap directory --op @web/public/caps/sample-network-load.json \
      --as focus --op '{"op":"focus","network_hash":"$load_network.network_hash","kind":"route","id":"10"}' \
      | node -e 'const r = require("fs").readFileSync(0, "utf8").trim().split("\n").map(JSON.parse); const ok = (n) => r.find((x) => x.as === n).ok; const legs = r.find((x) => x.ok?.op === "itinerary_paths").ok.paths; if (legs[0]?.kind !== "transit" || legs[0].path.length < 2 || !ok("trip").path.some((p) => p.lat === 45.6 && p.lon === -73.6) || ok("focus").focus.lines.length !== 3) process.exit(1)'
    # The planner's numbers and search are answered (W5): route 10's
    # composition with each pattern trips' count, the stop list
    # narrowed by every term, a stop's serving routes, route 10's
    # timetable in direction 0 on WK (directory),
    # and a plan with no trip carries the rider's notice (router) —
    # one chained session.
    node tools/caps-run.mjs directory \
      --op @web/public/caps/sample-network-load.json \
      --as detail --op '{"op":"route_detail","network_hash":"$load_network.network_hash","route_id":"10"}' \
      --as stops --op '{"op":"entities","network_hash":"$load_network.network_hash","kind":"stop","query":"main 2nd"}' \
      --as serving --op '{"op":"stop_detail","network_hash":"$load_network.network_hash","stop_id":"A"}' \
      --as timetable --op '{"op":"timetable","network_hash":"$load_network.network_hash","route_id":"10","direction":"0","service_type":"WK"}' \
      --cap router --op @web/public/caps/sample-network-load.json \
      --as late --op '{"op":"plan_compare","network_hash":"$load_network.network_hash","from":"A","to":"B","date":"2026-10-23","depart_secs":144000,"max_legs":6}' \
      | node -e 'const r = require("fs").readFileSync(0, "utf8").trim().split("\n").map(JSON.parse); const ok = (n) => r.find((x) => x.as === n).ok; const d = ok("detail").detail; if (d.pattern_trips_by_type.map((g) => g.service_type).join() !== "SUN,WK" || !d.pattern_trips_by_type.every((g) => g.pattern_trips.every((s) => s.trips > 0)) || ok("stops").rows.map((s) => s.id).join() !== "B" || !ok("serving").detail.routes.some((r) => r.id === "10") || !(ok("timetable").timetable?.trips.length > 0) || !ok("late").notice?.text?.includes("within 6 legs")) process.exit(1)'
    # The router capability's gate: planning over the network document
    # — the plan, then the diversion delta (compare) chained on the
    # same edits document (08:15 against the unpatched 08:10: the
    # notice says 5 min later), and arrive-by with no deadline (the
    # last bus home: t30 leaves 08:40).
    node tools/caps-run.mjs router \
      --op @web/public/caps/sample-network-load.json \
      --op '{"op":"plan","network_hash":"$load_network.network_hash","from":"A","to":"B","date":"2026-10-23","depart_secs":25200,"max_legs":4}' \
      --op '{"op":"load_edits","network_hash":"$load_network.network_hash","records":[{"id":"e1","diversion":{"id":"d1","route_id":"10","direction_id":null,"first_kept_stop":"A","last_kept_stop":"B","skipped_stops":["C"],"detour":null,"added_secs":300,"start_date":"2026-10-23","end_date":"2026-10-23"},"status":"accepted","rev":1,"updated_at":1}]}' \
      --as cmp --op '{"op":"plan_compare","network_hash":"$load_network.network_hash","edits_hash":"$load_edits.edits_hash","from":"A","to":"B","date":"2026-10-23","depart_secs":25200,"max_legs":4}' \
      --as last --op '{"op":"plan_arrive_by","network_hash":"$load_network.network_hash","from":"A","to":"B","date":"2026-10-23","max_legs":4}' \
      | node -e 'const r = require("fs").readFileSync(0, "utf8").trim().split("\n").map(JSON.parse); const ok = (n) => r.find((x) => x.as === n).ok; const c = ok("cmp"); if (c.active?.arrival_hhmmss !== "08:15:00" || c.notice?.kind !== "later" || c.notice.minutes_lost !== 5 || ok("last").itinerary?.depart_secs !== 31200) process.exit(1)'
    # The map's click-through, on the bus (U6): the trip detail — the
    # patched day's t30 keeps A→B, C skipped (the answer carries no
    # geometry; the host's view code draws).
    node tools/caps-run.mjs runcut \
      --op @web/public/caps/sample-network-load.json \
      --op '{"op":"load_edits","network_hash":"$load_network.network_hash","records":[{"id":"e1","diversion":{"id":"d1","route_id":"10","direction_id":null,"first_kept_stop":"A","last_kept_stop":"B","skipped_stops":["C"],"detour":null,"added_secs":300,"start_date":"2026-10-23","end_date":"2026-10-23"},"status":"accepted","rev":1,"updated_at":1}]}' \
      --op '{"op":"trip_stops","network_hash":"$load_network.network_hash","edits_hash":"$load_edits.edits_hash","trip":"t30","start_secs":31200,"date":"2026-10-23"}' \
      | grep -q '"stop_id":"B"'
    # The street capability's gate: where along the streets, one
    # chained session — the street fixture imported on bulk op 0, the
    # document it answered registered back (load_streets: the hash
    # the queries address), two snaps, and the route between them
    # (the Elbow corner, not the diagonal: the committed trace
    # follows street geometry). grep reads the whole answer (no -q:
    # quitting early breaks the runner's pipe).
    node tools/caps-run.mjs streets --load web/e2e/fixtures/streets.geojson \
      --op '{"op":"load_streets","streets":"$load.streets"}' \
      --op '{"op":"snap","streets_hash":"$2.streets_hash","lat":45.504,"lon":-73.503}' \
      --op '{"op":"snap","streets_hash":"$2.streets_hash","lat":45.508,"lon":-73.508}' \
      --op '{"op":"route","streets_hash":"$2.streets_hash","from":"$3.position","to":"$4.position"}' \
      | grep '"lat":45.504,"lon":-73.508' > /dev/null
    # The walk gate: a plan's transfers along the streets, composed
    # by documents across two capabilities — streets mints the walk
    # table from the walking fixture (a residential street through
    # the station's bays; a motorway no rider walks) and the network,
    # the router registers it and walks bay A to bay B on it (61 s
    # along Station Rd, before the day's first bus).
    node tools/caps-run.mjs streets --load web/e2e/fixtures/walk-streets.geojson \
      --op @web/public/caps/sample-network-load.json \
      --op '{"op":"walks","streets_hash":"$load.streets_hash","network_hash":"$load_network.network_hash"}' \
      --cap router --op @web/public/caps/sample-network-load.json \
      --op '{"op":"load_walks","walks":"$walks.walks"}' \
      --op '{"op":"plan","network_hash":"$load_network.network_hash","from":"SA","to":"SB","date":"2026-10-23","depart_secs":18000,"max_legs":2,"walks_hash":"$load_walks.walks_hash"}' \
      | grep '"arrival_secs":18061' > /dev/null
    # The place gate: a trip from any point — streets answers a
    # dropped pin's walks to the stops near it (with their paths),
    # the router plans from the pin as a place, the walks passed as
    # they are (138 s along Station Rd to bay B).
    node tools/caps-run.mjs streets --load web/e2e/fixtures/walk-streets.geojson \
      --op @web/public/caps/sample-network-load.json \
      --op '{"op":"access","streets_hash":"$load.streets_hash","network_hash":"$load_network.network_hash","point":{"lat":45.5152,"lon":-73.5165}}' \
      --cap router --op @web/public/caps/sample-network-load.json \
      --op '{"op":"plan","network_hash":"$load_network.network_hash","from":{"id":"pin","name":"Dropped pin","walks":"$access.walks"},"to":"SB","date":"2026-10-23","depart_secs":18000,"max_legs":2}' \
      | grep '"arrival_secs":18138,"depart_secs":18000,"from_stop_id":"pin"' > /dev/null
    # The sync capability's gate: the protocol's one step — a draft
    # client against an empty server converges to the submitted
    # document (rev bumped, host clock stamped).
    node tools/caps-run.mjs sync \
      --op '{"op":"sync_edits","client":[{"id":"e1","diversion":{"id":"d1","route_id":"10","direction_id":null,"first_kept_stop":"A","last_kept_stop":"B","skipped_stops":["C"],"detour":null,"added_secs":300,"start_date":"2026-10-23","end_date":"2026-10-23"},"status":"draft","rev":1,"updated_at":1}],"server":[],"now_ms":1000}' \
      | grep -q '"status":"submitted"'
    # The plan-chain capability's gate: which pick governs — the
    # spring pick in force, the fall pick taking over after it, and
    # the derived windows.
    node tools/caps-run.mjs plans \
      --op '{"op":"resolve","plans":[{"id":"spring","name":"Spring 2026","effective_from":"2026-03-01"},{"id":"fall","name":"Fall 2026","effective_from":"2026-09-07"}],"date":"2026-10-23"}' \
      --op '{"op":"periods","plans":[{"id":"spring","name":"Spring 2026","effective_from":"2026-03-01"},{"id":"fall","name":"Fall 2026","effective_from":"2026-09-07"}]}' \
      | grep -q '"plan":"fall"'
    # The authoring capability's gate: the fixture network stamped as
    # a pick in one atomic change, then the answered document read by
    # the day views in the same session — chained by the hash the
    # author answered (the next change's address), never re-uploaded.
    node tools/caps-run.mjs author \
      --op @web/public/caps/sample-network-load.json \
      --as pick --op '{"op":"apply","network_hash":"$load_network.network_hash","edits":[{"edit":"set_plan","plan":{"id":"fall","name":"Fall 2026","effective_from":"2026-10-19"}}]}' \
      --cap calendar --op '{"op":"load_network","network":"$pick.network"}' \
      --op '{"op":"active_services","network_hash":"$pick.network_hash","date":"2026-10-23"}' \
      | grep -q '"services":\["WK"\]'
    # The diversion-authoring capability's gate: a draft anchored end
    # to end on route 10's default pattern, proposed into an empty
    # edit document, and the answered document validated by the board
    # in the same session — the record the browser's editor writes.
    node tools/caps-run.mjs divert \
      --op @web/public/caps/sample-network-load.json \
      --as prop --op '{"op":"propose","network_hash":"$load_network.network_hash","draft":{"route_id":"10","pattern":"pat-10-0-1","first_kept_stop":"A","last_kept_stop":"B","served_stops":[],"slices":[],"added_mins":5,"start_date":"2026-10-23","end_date":"2026-10-23"},"records":[],"record_id":"e1","now_ms":1000}' \
      --cap board --op @web/public/caps/sample-network-load.json \
      --op '{"op":"load_edits","network_hash":"$load_network.network_hash","records":"$prop.records"}' \
      | grep -q '"valid":true'
    # ARCHITECTURE GATES — the laws as assertions (spelled out in
    # crates/AGENTS.md), because prose does
    # not enforce:
    #   1. the NETWORK is the core data model; GTFS is a serialization
    #      spec for external data sharing. Model-side capabilities do
    #      not depend on the GTFS layer, and the boundary codec's
    #      whole-feed write never leaves the codec's homes.
    #   2. nobody reinvents the monolith inside a module.
    # Law 1a: model-side capabilities (and their shared libraries) do
    # not depend on the GTFS layer (transit-codec, the boundary codec)
    # in [dependencies].
    # Dev-dependencies may build fixture documents through the codec —
    # tests compose over real documents; production code does not.
    # Sanctioned consumers: cap-gtfs, cap-network, cap-rt and
    # transit-rt (rt updates number stops as the published feed does —
    # the import's Boundary). The router left the feed in U5 — it
    # plans over the network document and the effective day.
    for crate in cap-analysis cap-board cap-runcut cap-calendar cap-directory cap-sync cap-plans cap-author cap-divert transit-analysis transit-dispatch transit-live; do
        for dep in transit-codec; do
            if awk '/^\[dev-dependencies\]/{exit} /^'"$dep"' =/{found=1} END{exit !found}' "crates/$crate/Cargo.toml" 2>/dev/null; then
                echo "ARCH FAIL: $crate depends on $dep — the network is the core model; compose over the network document (crates/AGENTS.md, Law 1)"; exit 1
            fi
        done
    done
    # Law 1b: the whole-feed write (transit_codec::export) is the
    # codec's publish path — sanctioned only in the codec's homes. A
    # hit anywhere else is the monolith rebuilt with new clothes.
    violations=$(grep -rlE "transit_codec::export|codec::\{[^}]*\bexport\b" crates --include="*.rs" \
        | grep -v "crates/transit-codec/" \
        | grep -v "crates/cap-network/" \
        | grep -v "crates/cap-abi/" || true)
    if [ -n "$violations" ]; then
        echo "ARCH FAIL: transit_codec::export outside the codec's homes (Law 2 — no whole-feed writes in capabilities):"
        echo "$violations"
        exit 1
    fi
    # WEB GATES — parity (docs/ARCHITECTURE.md, web section, "Parity"): the
    # browser runs the same ops as caps-run and the Go server, so
    # web/ holds host glue and render components, never answers.
    # Ratchets: each allowlist names today's violators and may only
    # SHRINK — a new violator fails, and so does a listed file that
    # no longer violates (take it off the list in the same change).
    ratchet() { # name, allowlist (space-separated), violators
        local name="$1" allowed=" $2 " found="$3" f
        for f in $found; do
            if [[ "$allowed" != *" $f "* ]]; then
                echo "WEB GATE FAIL ($name): $f — $4 (docs/ARCHITECTURE.md, Parity)"; exit 1
            fi
        done
        for f in $2; do
            if [[ " $found " != *" $f "* ]]; then
                echo "WEB GATE FAIL ($name): $f no longer violates — remove it from the allowlist"; exit 1
            fi
        done
    }
    # W0a: components never call the bus — the host layer does (the
    # allowlist reached empty with W6: a hard gate now). Every shell's
    # layout is held to it too (App.svelte, each generated UI, the picker).
    ratchet resolver-in-components \
        "" \
        "$(grep -l 'capabilities\.svelte' web/src/App.svelte web/src/components/*.svelte web/src/uis/*/*.svelte web/src/picker/*.svelte | tr '\n' ' ')" \
        "a component imports the capability resolver; take node values as props"
    # W0b: no host-side rt join — delays come from cap-rt's live
    # document (the allowlist reached empty with W1: a hard gate now).
    ratchet rt-join-in-web \
        "" \
        "$(grep -rl 'stop_updates' web/src --include='*.ts' --include='*.svelte' | grep -v '^web/src/types/' | sort | tr '\n' ' ')" \
        "reads raw rt stop_updates; the live join is cap-rt's"
    # W0c: no clone-and-mutate of documents in web/ — every change to
    # a network document is a NetworkEdit through cap-author (W2).
    ratchet clone-and-mutate \
        "" \
        "$(grep -rl 'structuredClone' web/src --include='*.ts' --include='*.svelte' | grep -v '^web/src/types/' | sort | tr '\n' ' ')" \
        "clones a document to mutate it; send NetworkEdits through cap-author"
    # GENERATED UIs (docs/ARCHITECTURE.md, "Generated UIs"): each
    # src/uis/<id>/ carries its descriptor, its page, and its e2e spec,
    # and composes THIN components only — a component is thin iff it
    # imports none of the app shell's host modules (state.svelte's
    # `app`, planner, actions, capdemo). The rule is checked on what a
    # UI imports, so a thin component that grows a coupling fails here.
    coupled='from "\.\./(state\.svelte|planner|actions|capdemo\.svelte)"'
    for dir in web/src/uis/*/; do
        id=$(basename "$dir")
        for need in "web/src/uis/$id/ui.json" "web/$id.html" "web/e2e/ui-$id.spec.mjs"; do
            [ -f "$need" ] || { echo "UI GATE FAIL: generated UI $id is missing $need"; exit 1; }
        done
        node -e 'const j = require(process.argv[1]); for (const k of ["id","title","summary","questions","components"]) if (j[k] === undefined) { console.error("missing " + k); process.exit(1) } if (j.id !== process.argv[2]) { console.error("id " + j.id + " != directory"); process.exit(1) }' "./web/src/uis/$id/ui.json" "$id" \
            || { echo "UI GATE FAIL: web/src/uis/$id/ui.json"; exit 1; }
    done
    for f in web/src/uis/*.ts web/src/uis/*/* web/src/picker/*; do
        if grep -qE 'from "(\.\./)+(planner|actions|capdemo\.svelte|App\.svelte)"' "$f" \
            || grep -qE 'import \{[^}]*\bapp\b[^}]*\} from "(\.\./)+state\.svelte"' "$f"; then
            echo "UI GATE FAIL: $f imports the app shell — a generated UI has its own host layer (src/uis/<id>/host.svelte.ts)"; exit 1
        fi
        for c in $(grep -oE 'components/[A-Za-z]+\.svelte' "$f" | sort -u); do
            if grep -qE "$coupled" "web/src/$c"; then
                echo "UI GATE FAIL: $f uses $c, which is coupled to the app shell — compose thin components (or build a thin one)"; exit 1
            fi
        done
    done
    # -race: the artifact is one non-reentrant instance; the host must
    # serialize invokes (see server/race_test.go — the gate watches it).
    cd server && go test -race ./...

# Browser e2e (Chromium via Playwright, against the Go server).
# Builds first; the runner starts the server unless :8080 is taken.
# For UI-visible changes: user-visible flows, module surfaces, the
# capability-bus spec (web/e2e/caps.spec.mjs).
e2e: caps build
    cd web && npx playwright test

# Both runners — for changes that touch both sides.
check: test e2e

# Browser performance baselines (docs/TESTING.md, "Performance
# baselines"): the real feed and recorded live feeds through a UI,
# measured over the DevTools protocol — reports, never gates. The
# profiling build keeps the capability modules' name sections (so a
# CPU profile names Rust functions) and serves an unminified frontend
# from web/dist-perf; the next `just caps` (or `just test`) restores
# the stripped modules. PERF_RT picks the recording
# (`node web/perf/record-rt.mjs`), PERF_FEED the feed.
perf *args:
    #!/usr/bin/env bash
    set -e
    CARGO_PROFILE_RELEASE_STRIP=debuginfo just caps
    just types
    cd web && npx vite build --minify false --outDir dist-perf --emptyOutDir
    npx playwright test -c perf/playwright.config.mjs {{args}}

# Real-data soak: transit-streets against the downloaded county
# export. Hermetic tests skip it; point COUNTY_GEOJSON at the file to
# run it (e.g. the CTRAN Trapeze roads geojson from the open data
# portal).
soak:
    #!/usr/bin/env bash
    if [ -z "$COUNTY_GEOJSON" ]; then
        echo "set COUNTY_GEOJSON to the downloaded county roads geojson"
        exit 1
    fi
    cargo test --release -p transit-streets --test county_data -- --nocapture

# Regenerate the e2e GTFS fixture (web/e2e/fixtures/sample-gtfs.zip).
fixtures:
    cd web/e2e && python3 make_fixtures.py

# Regenerate the Clark County population layer
# (testdata/clark-population.geojson, from public-domain census
# sources — the real-data dogfood asset for analysis.type_coverage).
population:
    node testdata/generate-population.mjs

# ---------------------------------------------------------------
# run
# ---------------------------------------------------------------

# Serve the built app and APIs on :8080. Arguments are positional:
#
#   just serve                                   # static + /rt only
#   just serve path/to/gtfs.zip                  # + /network (imported once), /manifest
#   just serve path/to/gtfs.zip roads.geojson    # + /streets, /walks (the layer
#                                                # imported once, the walk table
#                                                # derived from both)
#   just serve path/to/gtfs.zip "" "Test Transit"   # + manifest publisher
#   just serve path/to/gtfs.zip "" "" 127.0.0.1:8081
#
# The feed and street layer paths are resolved relative to the repo
# root, wherever you run just from.
serve feed="" streets="" publisher="" addr="127.0.0.1:8080":
    #!/usr/bin/env bash
    set -e
    cd server
    args=(-addr "{{ addr }}")
    if [ -n "{{ feed }}" ]; then args+=(-feed "{{ absolute_path(feed) }}"); fi
    if [ -n "{{ streets }}" ]; then args+=(-streets "{{ absolute_path(streets) }}"); fi
    if [ -n "{{ publisher }}" ]; then args+=(-publisher "{{ publisher }}"); fi
    if [ -n "${GEOCODER:-}" ]; then args+=(-geocoder "$GEOCODER"); fi
    if [ -n "${STORE:-}" ]; then args+=(-store "$STORE"); fi
    exec go run . "${args[@]}"

# Frontend dev loop: vite with hot reload on :5173 (proxies /rt,
# /geocode), with the sync server alongside on :8080 (logs:
# /tmp/transit-server.log). GEOCODER=<url> passes -geocoder.
dev: caps types
    #!/usr/bin/env bash
    set -e
    geo=(); if [ -n "${GEOCODER:-}" ]; then geo=(-geocoder "$GEOCODER"); fi
    store=(); if [ -n "${STORE:-}" ]; then store=(-store "$STORE"); fi
    (cd server && go build -o /tmp/transit-server . && \
      /tmp/transit-server "${geo[@]}" "${store[@]}" >/tmp/transit-server.log 2>&1 & echo $! >/tmp/transit-server.pid)
    trap 'kill "$(cat /tmp/transit-server.pid)" 2>/dev/null || true' EXIT
    cd web && npx vite

# ---------------------------------------------------------------
# basemap
# ---------------------------------------------------------------

# Extract a regional basemap from the Protomaps daily planet build
# (needs the pmtiles CLI; ~94 MB for a metro area at z15).
#
#   just basemap "-123.20,45.25,-122.10,45.95"
# just basemap "-123.20,45.25,-122.10,45.95" 14
basemap bbox maxzoom="15":
    pmtiles extract "https://build.protomaps.com/$(date +%Y%m%d).pmtiles" \
        web/public/basemap.pmtiles --bbox={{ bbox }} --maxzoom={{ maxzoom }}

# ---------------------------------------------------------------
# geocoder
# ---------------------------------------------------------------

# Address search is an external service, not a document: a local
# Pelias (github.com/pelias/docker, its portland-metro project —
# OSM, OpenAddresses, Who's On First, TIGER interpolation) in
# .pelias/ (gitignored: pelias/docker at a pinned commit in docker/,
# never written to; data/ ~6.5 GB). The server proxies it at /geocode when started with
# GEOCODER set:
#
#   just geocoder-build      # once: fetch, download, import (~1 h)
#   just geocoder            # start it on 127.0.0.1:4000
#   GEOCODER=http://127.0.0.1:4000 just serve feed.zip roads.geojson
#   just geocoder-stop       # stop every container
#
# Needs docker compose (or podman + podman-compose; rootless podman
# gets a keep-id compose override and a docker.io short-name config
# here). The
# project's transit importer is skipped: stops come from the
# network, never the geocoder.
pelias_commit := "3dfa07d580416edd7a27c2d4ff5976c8c1cc6ebc"

[private]
pelias *args:
    #!/usr/bin/env bash
    set -e
    # .pelias/ holds pelias/docker at the pinned commit (docker/, a
    # plain tarball extract — no nested git repository — never written
    # to), its data (data/), and the files this recipe generates
    # (podman/).
    root="{{ justfile_directory() }}/.pelias"
    clone="$root/docker"
    if [ "$(cat "$clone/.commit" 2>/dev/null)" != "{{ pelias_commit }}" ]; then
        rm -rf "$clone" && mkdir -p "$clone"
        curl -fsSL "https://codeload.github.com/pelias/docker/tar.gz/{{ pelias_commit }}" \
            | tar -xz -C "$clone" --strip-components=1
        echo "{{ pelias_commit }}" > "$clone/.commit"
    fi
    project="$clone/projects/portland-metro"
    mkdir -p "$root/data"
    # The environment wins over the project's .env, in the pelias CLI
    # and in compose alike.
    export DATA_DIR="$root/data" COMPOSE_PROJECT_NAME=pelias
    if command -v podman >/dev/null; then
        # Rootless podman: unqualified images resolve on docker.io, and
        # every container runs as the invoking user (keep-id, outside a
        # pod — podman refuses keep-id in one) so the data bind mount
        # is writable.
        mkdir -p "$root/podman"
        printf 'unqualified-search-registries = ["docker.io"]\nshort-name-mode = "permissive"\n' > "$root/podman/registries.conf"
        { echo 'x-podman: { in_pod: false }'; echo 'services:'
          awk '/^services:/ { on = 1; next } /^[^ ]/ { on = 0 }
               on && /^  [a-z-]+:$/ { print $0 " { userns_mode: keep-id }" }' "$project/docker-compose.yml"
        } > "$root/podman/compose.override.yml"
        export CONTAINERS_REGISTRIES_CONF="$root/podman/registries.conf"
        export COMPOSE_FILE="$project/docker-compose.yml:$root/podman/compose.override.yml"
    fi
    cd "$project" && "$clone/pelias" {{ args }}

geocoder-build:
    just pelias compose pull
    just pelias elastic start
    just pelias elastic wait
    just pelias elastic create
    just pelias download wof
    just pelias download oa
    just pelias download osm
    just pelias download tiger
    just pelias prepare polylines
    just pelias prepare placeholder
    just pelias prepare interpolation
    just pelias import wof
    just pelias import oa
    just pelias import osm
    just pelias import polylines
    just pelias compose down

geocoder:
    just pelias compose up -d elasticsearch api libpostal placeholder pip interpolation

geocoder-stop:
    just pelias compose down

# Supply-chain audit: runs whatever is installed, reports what is
# missing. Recommended installs:
#   cargo install cargo-audit cargo-mutants
# go install golang.org/x/vuln/cmd/govulncheck@latest
audit:
    #!/usr/bin/env bash
    set +e
    status=0
    if command -v cargo-audit >/dev/null; then
        cargo audit || status=1
    else
        echo "SKIP: cargo-audit not installed (cargo install cargo-audit)"
    fi
    if command -v govulncheck >/dev/null; then
        (cd server && govulncheck ./...) || status=1
    else
        echo "SKIP: govulncheck not installed (go install golang.org/x/vuln/cmd/govulncheck@latest)"
    fi
    (cd web && npm audit --audit-level=high) || status=1
    exit $status

# Mutation-test the domain crates: every surviving mutant is a place
# where "tests passed" is a lie. Generated code (transit-rt) is
# excluded; needs cargo-mutants:
# cargo install cargo-mutants
mutants:
    cargo mutants -p transit-codec -p transit-router -p transit-sync -p transit-plans

# Model-check the sync store engine (server/store/store.tla, run with
# TLC over store.cfg) — the engine's invariants: ref integrity, an
# append-only chained log, GC safety, CAS authority, crash safety
# (docs/units/sync-persistence.md). OPTIONAL: run it only when a change
# touches the sync engine (server/store/, the /pointers + /blobs +
# /edits store surface, or the spec itself) — nothing else needs Java.
# Needs Java 11+ (current tla2tools) and tla2tools.jar from
# github.com/tlaplus/tlaplus/releases (the user's install; TLA_JAR or
# ~/bin/tla2tools.jar). Deliberately NOT part of `just test`.
spec:
    #!/usr/bin/env bash
    set -e
    jar="${TLA_JAR:-$HOME/bin/tla2tools.jar}"
    if ! command -v java >/dev/null 2>&1 || [ ! -f "$jar" ]; then
        echo "just spec is OPTIONAL (only for sync-engine changes) and needs Java 11+ and tla2tools.jar"
        echo "(set TLA_JAR=/path/to/tla2tools.jar; get it from github.com/tlaplus/tlaplus/releases)"
        echo "nothing else needs Java: 'just test' runs without it"
        exit 1
    fi
    java -XX:+UseParallelGC -cp "$jar" tlc2.TLC -config server/store/store.cfg server/store/store.tla
