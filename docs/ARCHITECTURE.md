# Architecture — the authoritative reference

This is the ONE authoritative architecture doc. The compact tree
and ownership rules live in the root `AGENTS.md`; per-crate,
server, web, and bus detail lives here and nowhere else. The
three architecture laws (the network is the core data model;
nobody reinvents the monolith inside a module; one NEED per
module, personas assemble at wire time) are spelled out in
`crates/AGENTS.md` — that file and this one are updated together.
This doc describes the tree as it is; open work lives in
`docs/units/`, one file per unit.

## Layering (bottom up)

```
transit-network The authored network model (planner authority):
                routes and their DIRECTIONS (`Route.directions`,
                each named; a pattern runs in at most one —
                `Pattern.direction`, validated against its route;
                bay and short-turn variants share one),
                patterns, timing profiles, service types (the
                service identifier, linked to days by its calendar
                rule), pattern trips (`PatternTrips`: one pattern's
                trips under one service type — NOT a planner's
                schedule; the timetable a planner reads is a route's
                direction under one service type, across its
                patterns), runcut. A TRIP is a stored record: its
                own id, its own departure (`Departs::At` one start,
                or `Departs::Every` a frequency — one instance per
                band slot, sharing the trip's id and times), and its
                own time at every pattern stop (`StopOffset`s from
                its start; a timepoint stop carries both times, an
                unscheduled one materializes by distance between its
                scheduled neighbours). A pattern's timing profiles
                are the planner's tool for PRODUCING a new trip's
                times, never the schedule. A block references trips
                by id and runs each whole (a frequency trip's
                instances back to back); it may hold service types
                that never share a day, and is judged day by day.
                An ALIGNMENT is a line on the ground, its own record
                (`Network::alignments`: its id, its points, the
                distances published along it); a pattern runs one by
                reference (`Pattern::alignment`), so a pattern is its
                stops along its line. Patterns may share one, and one
                no pattern runs still stands.
                `materialize` answers a day's instances of the stored
                trips; `read_network` also reads documents written
                before trips were stored (each old departure becomes
                a trip `{pattern_trips}@{start}`) or before
                alignments stood alone (a pattern's inline `path`
                becomes an alignment under the pattern's id).
                `trip_instance` answers one instance date-free: a
                one-start trip at its start, an exact frequency trip
                on its grid, a non-exact one at any start inside a
                band. A trip instance is addressed (trip, start)
                everywhere. WHERE A BUS IS GOING is one rule
                (`signs.rs`, `visit_destination`): the visit's own
                headsign (it overrides the trip's at that visit
                only, as GTFS's stop_headsign does), else the trip's,
                else the final stop named as its place (the parent
                station, never the bay) — every board row and
                itinerary leg reads it. The network models everything the GTFS
                Schedule spec describes, in model vocabulary, each
                family in its own module: per pattern stop its
                headsign, pickup / drop-off and continuous-stopping
                rules (`StopRule`), booking rules, and whether its
                distance was published; per trip its headsign,
                public number, and wheelchair / bike / car
                accommodation; per route its long name, description,
                URL, text color, sort order, continuous stopping,
                operator, fare network, contactless acceptance, and
                a published vehicle type where the mode does not say
                it (the short name is `Route::name`, possibly empty;
                display by `Route::label`); per stop its kind (stop
                or station) and `StopInfo` (code, spoken name,
                description, URL, fare zone, wheelchair boarding,
                platform, level, timezone, street access).
                `operators.rs`: operators, the publisher,
                attributions, translations. `places.rs`: station
                parts (entrances, nodes, boarding areas), levels,
                pathways, transfer rules. `fares.rs`: zone fares and
                fare products (areas, fare networks, timeframes,
                rider categories, media, offers, leg / join /
                transfer rules). `demand.rs`: demand-responsive
                service — location groups, zones (a GeoJSON
                FeatureCollection), booking rules, and trips that
                visit stops, groups, or zones within windows. GTFS is a codec at
                the boundary, not the model; no GTFS types leak in
                here. Transfer points are authored flags on
                stops, composed over their WALKING GROUP (the stop +
                siblings sharing a parent, the parent, children —
                transfers happen across a street, not at one pin).
                Every document is a SERVICE PLAN (`PlanMeta`: id,
                name, effective_from) — agencies implement a few
                picks a year at defined dates; empty identity =
                unversioned (a fresh import). AUTHORING lives here
                too (`edit.rs`): typed, atomic `NetworkEdit`s
                answer the NEXT document (never mutating the last);
                removals refuse while referenced. Two levels:
                entity upserts/removes, and GESTURES — one edit per
                thing a planner does, each carrying its coherence
                rule (timing aligned with timepoints, every trip's
                times aligned with its pattern's stops, layovers only
                between trips, ids minted and reported, the plan
                frame merged field-wise) —
                and the TIMETABLE's gestures (`timetable_edit.rs`:
                headway, span, running time, and stops, stated
                for a route + direction (+ service type) and
                applied across its patterns; a trip that stays is
                retimed and keeps its id, so its blocks keep it; one
                that goes leaves them; a new one gets a fresh id and
                is unassigned; a running-time change names its
                service type). Validation answers issues (gate: a
                block naming a trip the network does not have is
                one) and WARNINGS that never gate
                (unassigned trips; a pick with no service in its
                first week). GEOMETRY lives here too
                (`geometry.rs`): a pattern's line (the alignment it
                runs, else its stops joined — the model's own
                contract) and where an entity is (`entity_focus`: a
                stop's point; the lines and stops of every pattern a
                route, pattern trips, service type, or block runs). The
                planner's LISTS live here too (`listing.rs`): rows
                of any kind narrowed by every filter term (id, name,
                and a route's long name), the places near a point
                (`stops_near`; `stops_near_walked` ranks them by the
                point's walks), and a route's composition
                (patterns, the stops they visit, pattern trips by
                service type, patterns by direction, and each
                direction's VARIANTS — the ways a rider rides it:
                patterns over the same places in order, a station's
                bays one place, named by their ends and by what sets
                them apart, most trips first) — each entry's trip
                count is its trips' instances, the materialized
                day's own expansion — a stop's serving routes, and the
                planner's TIMETABLE (`timetable`: one route, one
                direction, one service type — every trip of the
                direction's patterns by start, one column per stop
                in route order, a variant's stops slotted where it
                leaves the others; rows are `materialize_service`'s
                trips, never a second derivation, each with its own
                end-to-end running time, `run_secs`).
transit-router  RAPTOR planning over the network model's
                materialized day; pure (no I/O, clock, or calendar),
                GTFS-free. `RouterNetwork::build(network, day, date,
                live)` groups the caller's day (plain or effective)
                into patterns by actual stop sequence — an edited
                trip rides its own pattern. With a live document,
                each trip instance rides at its predicted times where
                the document states them (delays against the
                published schedule, clamped so a trip never runs
                backwards), the day's own times elsewhere.
                A plan's ends (`End`) are stops or PLACES off the
                network (`Place`: an id never a stop's, a name, its
                walks to the stops near it): from a place the rounds
                start at every stop it walks to, each at its walk's
                time, and to one they end at the best of its stops
                plus its walk — or at the direct walk between the
                ends, which wins ties (D-33). The itinerary and its
                first and last walks name the place.
                `earliest_arrival_between` answers leave-at;
                `latest_departure_between` answers ARRIVE-BY — the
                same rounds run backward from the destination, and
                with no deadline the service day's last arrival: the
                last bus home; its itinerary is the forward plan from
                that departure (`earliest_arrival` /
                `latest_departure` are the stop-to-stop forms). A
                transit leg names its trip instance,
                the route's label and long name, the sign at the
                boarding visit (`visit_destination`), and how far
                each predicted time moved (board/alight_delay_secs,
                null without a prediction). `notice::plan_notice` is
                the one place a compared plan's notice is computed:
                structured (no_trip / unreachable / later / diverted,
                minutes lost, routes, skipped stops) plus a text
                line, firing whenever the diverted world arrives
                later, has no trip, or rides a trip instance whose
                patched times differ from its unpatched one.
                Transfers are footpaths: straight lines (stop pairs
                within 300 m at 1.3 m/s) until `with_walks` replaces
                them with a walk table's walks — exactly the table's,
                between the network's stops (D-32). A plan's legs
                bound its rides, never its walks: the walk to the
                first boarding and from the last alighting settle
                before any round, so a one-leg plan walks to and
                from its bus. Its arrival and rides fixed, a plan
                then walks the least it can: the first ride boarded,
                each transfer made, and the last ride left at the
                stops whose walk is shortest and still connects
                (D-35). `walk_path`
                draws a walk leg: through its stops, and along a
                place's walk path where it walks from or to a place
                (a `PlaceWalk` carries the path `access` answered;
                between two places, the direct walk's).
transit-codec   The network codec, both directions, over the whole
                GTFS Schedule spec: `Feed` (src/feed.rs) reads and
                writes every spec file and column as the exact text
                the feed carried — lossless by construction,
                deterministic zip bytes — and `import` / `export`
                map a feed to an authored network document and back.
                A feed's trips ARE the document's trips: the
                agency's ids, every scheduled time exactly
                (offsets from the trip's first arrival), frequency
                trips as frequency trips, blocks by trip. A feed's
                shapes ARE the document's alignments: the feed's ids,
                every point and published distance, shapes no trip
                runs included. Patterns are stop sequences along an
                alignment — one per (route, direction, stops,
                exactness flags, shape, and what each stop row says:
                headsign, pickup / drop-off, continuous stopping,
                booking rules, published distance); how long a trip
                takes is its own. A trip with a row that is not a
                stop at a time (a location group, a zone, a window)
                is a demand-responsive trip. Everything else a feed
                carries reads into its model home and writes back
                value for value (src/extras.rs; text ⇄ values in
                src/values.rs). A row is exact (the pattern's
                timepoint) when it carries both times and does not
                say timepoint=0. The `Boundary` is the import's
                serialization residue (stop_sequence and
                shape_pt_sequence numbering, a frequency trip's
                stop_times base, the id given an agency published
                without one, and which file names a route's fare
                network); an authored
                document exports with the default. Service ids are
                service types, written back as authored (the
                calendar row, exceptions as calendar_dates); the
                import keeps every calendar the feed names. Each
                pattern gets one all-day timing profile from its
                earliest trip (the production tool), and each block
                layover is the observed gap between consecutive
                one-start trips of one service type (the planner
                re-authors from there). GTFS direction_id becomes
                the route's directions ("0"/"1"), named by the
                trips' most common headsign (else "To" the commonest
                terminal); the export writes direction_id back and
                each trip's own headsign (none invented). What
                the model cannot hold yet is reported as an
                ImportNote (stops without coordinates and the trips
                serving them, single-stop trips, a transfer point
                where fewer than two routes meet) — never silent.
                The laws: the import is valid by construction over
                spec-valid feeds (tests/import_prop_test), keeps the
                trips (tests/trip_import_law_test), and
                `import ∘ export` is a fixpoint
                (tests/codec_prop_test); tests/spec_round_trip_test
                holds GTFS → network → GTFS against the whole spec,
                one law per file, oracle read as raw tables. The
                feed layer also answers a feed's summary and its
                feed_info identity, and base64 for the JSON seam.
                A LIBRARY under cap-network — the extraction
                pattern: crate boundary first, capability wrapper
                second. The C-TRAN import regressions live in its
                own test suite (tests/import_regression_test).
transit-analysis Planner analysis over an authored network document:
                stats, service-type overview/detail, schedule
                texture (hourly profile, headway gaps, clockface),
                route corridors (shared stops and segments), the
                frequency grid (route_frequency: each route of a
                service group hour by hour at its headway stop —
                the trips' own shortest, longest and median gaps,
                never regularized — with the route's numbers), a
                hub's connections by route pair (hub_connections:
                timed_transfers folded), population coverage — the share AND the unserved
                where (unserved_points: the complement, named).
                A LIBRARY under the
                cap-analysis module; the planner's regression
                numbers live in its own
                test suite (tests/regression_test).
transit-dispatch The materialized day's views, model-native: patch_day
                (skip/shift over MATERIALIZED stop times — the day,
                never the calendar), edit validation against the
                NETWORK, THE board (stop_board: walking-group
                pooling for a stop with nothing of its own,
                realtime-first over the live document — rows
                ordered by when the bus actually leaves, each with
                its expected time, delay, status, and wait (due,
                minutes rounded down under an hour, then the clock
                time — `wait`, the one rule), its route's short and
                long names, its bay's name and platform code, and its
                destination as the sign says there; the departures
                the day's edits took away (`skipped`, each with the
                diversion that skips it — day_edits holds the
                unpatched day beside the patched one); the live
                document's alerts about the board, each row naming
                its own), the grouped board (grouped_board: THE
                board's rows by route and destination for a stop's
                display — every group the stop is served by all day,
                one that has stopped kept with none, its next
                per_group, its skipped visits; in route_sort_order,
                then id, then destination), the day's
                blocks list, the fleet summary (peak concurrent
                vehicles and when, platform / revenue / layover
                hours), the service keys
                running on a date, and the AUTHORED block chain with
                the layover ripple (layover_remaining_secs goes
                negative when a connection is eaten), and the
                DIVERSION EDITOR over the network (divert.rs: a
                draft's view — the route's patterns and the longest
                as default, each visit's role, the interior skipped
                unless served, the alignment, the flattened trace
                and its length, the problems — the record a ready
                draft becomes, and the edit document's named rows),
                and WHERE SERVICE RUNS TODAY (geometry.rs: a trip's
                line over the effective day — the authored alignment,
                and for a diverted trip the alignment to the first
                kept stop, the traced detour, the alignment from the
                last (a skip-only diversion keeps the alignment) —
                and the ridden segment, that line sliced between two
                stops).
                A LIBRARY shared by the cap-board, cap-runcut,
                cap-calendar, and cap-divert modules; the UI reads
                them here. Holds no GTFS
                layer (the justfile arch gate enforces it).
transit-rt      prost types from the official GTFS-rt proto, the
                OVERVIEW view (delays, vehicles, each trip
                descriptor's start_time) over them, and the joins
                over the NETWORK: an rt trip id IS a model trip id,
                the instance its start (a frequency trip's, the
                descriptor's start_time — `resolve_instance`). The
                PROGRESS join (one trip's predicted times; the
                ranked late view with the DELAYED-BUT-DARK flag)
                pairs stop updates by the published stop_sequence
                (the import's `Boundary`; 1..n without one), by stop
                id only when an update names none. The BOUNDARY
                RESOLUTION (live_day): snapshots (trip updates,
                vehicle positions, service alerts) plus the network
                (and its boundary) → THE live document, the one
                place rt trip ids are resolved (tests/rt_join_law_test)
                and the one place an alert's informed entities become
                routes, stops, and trip instances — kept when in
                force at the snapshot's moment (tests/alert_test).
transit-live    THE live document: one rt moment in model vocabulary
                (delays per trip instance (trip, start) and stop,
                vehicles on their trip instances, every status classified under the
                on-time window the document records — one window,
                never a threshold per view; by default ANY earliness
                is early, because a bus leaving early is a missed
                bus — and the service alerts in force, each a set of
                scopes: route / stop / trip instance, fields ANDed,
                scopes ORed, all absent = the agency), the
                classification and alert-matching rules, and the
                status summary. Pure data; no GTFS
                layer (arch gate). Minted by cap-rt, read by
                cap-board — no host computes a delay.
transit-sync   THE sync document: EditRecord, FeedManifest, EditReport,
                the Diversion edit document (THE DISPATCHER'S OWN —
                it lives in the sync contract, not the GTFS codec),
                pure validation/merge logic (merge is a
                property-tested semilattice; submit transitions
                drafts with the host-passed clock), and the record
                envelope every writer follows (propose: rev 1 or
                bumped; set_retired: the tombstone and restore;
                sanitize: a stored document read under the current
                schema, the unparseable dropped and counted). Shared
                by every host — cap-sync is the seam the Go server
                calls, cap-divert the authoring seam.
transit-plans  THE service-plan chain: resolve (which pick is in
                force), periods (each member's governing window),
                validate (chain invariants) over the plan set's
                metadata — DERIVED, never stored, deterministic and
                order-insensitive. Cross-document logic, the
                counterpart of transit-sync's cross-record merge;
                cap-plans is the seam.
transit-streets THE streets model. `model.rs`: the streets document
                (`Streets`: ways — points as [lat, lon] microdegrees,
                one-way, class, name), CANONICAL so its hash is the
                streets' own, not a file's: the same streets in
                another feature order, digitized the other way, drawn
                twice, or carrying other properties are one document
                (two-ways in canonical direction, a one-way running
                first point to last, ways sorted and unique;
                `validate` holds a stored document to it).
                `geojson.rs`: the boundary — a street layer (an
                agency GIS export such as C-TRAN's Trapeze roads,
                TIGER, OSM; GeoJSON lines) read into the document:
                CFCC, MTFCC, or OSM `highway` classify each street
                and leave out lines that are not streets (railroads,
                water — D-30), `FULLNAME` / `name` / `NAME` name it,
                TIGER `OneWay` and OSM `oneway` set its direction; a
                malformed vertex breaks a line. No other code reads a
                layer's properties. `lib.rs`: the routing graph built
                from the document — segments numbered way by way, the
                address a snapped position names (valid for one
                streets hash, so a trace commits GEOMETRY, never an
                address: a network never depends on a streets
                document) — snap (nearest street position) and
                shortest-path route (one-way aware; turn restrictions
                are a non-goal until a restriction table ships in the
                data). `walking.rs`: how long a rider walks — the
                walking profile is every class but motorway, ramp and
                unknown, both ways along one-way streets; a walk is
                each stop's walk to its nearest walkable street, the
                street distance, and the walk on to the other stop,
                at 1.3 m/s by default, kept within ten minutes by
                default, symmetric by construction. The WALK TABLE
                (`WalkTable`) is a document derived from a streets
                document and a network — every walk between its stops,
                naming both hashes, the speed and the limit — that
                the router plans transfers over. A POINT walks as a
                stop there would (`walks_from`): its walks to the
                stops within the limit, nearest first, each with the
                path the rider walks (the point, along walkable
                streets, the stop) — a router place's walks, the ends
                of a trip from anywhere. Walking quality is
                the streets document's: a source with real classes
                and footways (OSM), not one whose classes cannot tell
                a freeway from an arterial (QAF-85). Open work:
                `docs/units/streets-model.md`.
cap-authz       The authorization capability: may this identity
                perform this action on this pointer? One op
                (`authorize`), stateless like cap-plans — the host
                verifies the IdP's JWT and pushes the policy document
                per call; the verdict is fail-closed and never depends
                on rule order (transit-authz is the library, property-
                tested). The policy is a store document (kind `policy`,
                D-61).
cap-sync        The sync capability: the converged state of an edit
                document — its lifecycle facets: sync_edits (LWW merge
                + the submit transition, with the receive path
                clamping client clocks to the host's, D-56; the
                answer carries the canonical document bytes and their
                hash — the op behind POST /edits), read_edits
                (migrate-at-receive: typed parse, defaults fill what
                age removed, malformed dropped and counted), and
                compact_edits (the D-57 tombstone-retention commit).
                Stateless; validation is NOT here (load_edits
                answers it against the network in every model-side
                module — one question, one answer).
cap-plans       The plan-chain capability: which service plan
                governs a date — resolve / periods / validate over
                the plan set's metadata (transit-plans is the
                library). Pure ops, no registries: the host owns
                the plan set and pushes its metadata per call.
cap-author      The authoring capability: what does this network
                become after these edits? apply (typed edits,
                atomic) answers the next document under its hash,
                registered for the next change, with its issues
                and warnings. transit-network's edit module is the
                library; `created` names the entities a change made.
                Every host authors through it — the browser's
                planner included (web/src/planner.ts sends edits and
                adopts the answer; it never edits a document).
cap-divert      The diversion-authoring capability: what does this
                diversion draft amount to over the network (draft —
                the editor's whole view; list — the edit document's
                rows, names resolved), and what is the edit document
                after propose / retire / restore (the envelope rules)
                — plus sanitize for stored copies. A separate need
                from cap-sync's converge step (the Go server hosts
                that one without a network). The host passes the
                clock and the record ids; the browser's diversion
                editor is a render component over these answers.

Every module is the raw rustc output with the same four flat
exports (no wasm-bindgen — do not reintroduce host glue).
```

## server/

Go + wazero host: scaffolding only (HTTP, manifest, proxies). It must
never make a domain decision; the host reads the feed's own
feed_info.txt version field (one boundary CSV field — I/O, not
domain). It instantiates three modules, all rebuilt by `just caps`
into web/pkg/: cap-sync (the -wasm flag, cap_sync.wasm), the op
behind POST /edits, held for the server's lifetime; cap-network
(the -network-wasm flag, cap_network.wasm), which imports the
configured GTFS zip (`load_feed` + `import`) in a fresh instance
that is closed afterwards — the import's working memory goes with
it; and cap-streets (the -streets-wasm flag, cap_streets.wasm),
which imports the configured street layer (`import`) and derives
the walk table over it and the published network (`load_network`
+ `walks`) in a fresh instance, closed the same way. Machine code
is cached per module bytes for the process, so a re-import does
not recompile. The manifest pins the network it publishes, the zip
it was imported from, the streets and walk table it publishes, and
the sync module's hash. (Reentrancy and the Go-host serialization live in
`docs/CONVENTIONS.md`.)

**The deployment publishes the network, not the feed.** Browsers
never parse GTFS a server has: the import runs once per published
zip, on the server, through the module the browser runs (same
bytes, same `network_hash`), and every client registers the
answer. On the real C-TRAN feed that is the difference between
~2.7 s of import and a ~187 MB network module on every rider's
page, and ~0.4 s and ~17 MB (the import's high-water mark never
shrinks; the parsed feed stays registered). A zip loaded by hand
(the UIs' file input) still imports in the browser.

**The deployment publishes the streets the same way.** The street
layer is read once, on the server (`-streets`), and the walk table
— every walk between the network's stops, 13 s of work on C-TRAN —
is derived there too: a browser registers the canonical document
by its BYTES (cap-streets' `load_streets` bulk op parses it in the
module; the page never parses it) and the table in the router. On
C-TRAN's Trapeze roads (`sha256:48fe6c89…`) the server's startup
spends 11.8 s on the import and the table; the page fetches 4.5 MB
(the document, gzipped) and 43 KB (the table), and registers the
document in 0.47 s; the first `access` builds the walking graph in
0.32 s, later ones answer in ~14 ms.

With `-store <dir>` the server keeps a content-addressed store
(`docs/units/sync-persistence.md`, `docs/FORMATS.md`): immutable blobs
under `blobs/<hex>` with a typed envelope sidecar, and one append-only
log per pointer under `pointers/<name>.log` — the pointer is the log's
last entry, and a commit is blob-write + one log append, CAS'd on the
`from` hash. The `manifest` pointer is the root (publishing is one CAS
on it); the `edits` pointer is the multi-writer LWW document. Without
`-store`, `-edits <path>` persists the edit document as one flat file
and everything else is in process.

Its HTTP surface:

- `/` — `web/dist` (stdlib `http.ServeContent`, so the Range
  requests PMTiles needs work).
- `/rt?url=<encoded>` — a proxy that fetches an agency's realtime
  feed and returns it with CORS headers, so feeds without
  `Access-Control-Allow-Origin` can be polled from the page (the
  server binds 127.0.0.1 by default; `-addr` changes it).
- `/geocode/{autocomplete,search,reverse}` — a proxy to the Pelias
  geocoder named by `-geocoder` (`<url>/v1/<op>`): it forwards the
  user's input only (`text`, the focus point, the reverse point) and
  adds the deployment's query policy (`-geocoder-policy`, default
  `server/geocoder.json`); 404 when none is configured. The geocoder
  is an oracle, never a document on the bus (D-36, D-38; web
  section, "External services"). `just geocoder-build` builds a local Pelias
  (`github.com/pelias/docker`'s portland-metro project, in the
  gitignored `.pelias/`, its transit importer not run — stops are
  the network's), `just geocoder` starts it on :4000, and
  `GEOCODER=http://127.0.0.1:4000 just serve …` (or `just dev`)
  proxies it.
- `/manifest` — `{network_hash, feed_hash, feed_version,
  streets_hash, walks_hash, wasm_hash, generated_at, publisher,
  services}`: which network is authoritative, the
  GTFS zip it was imported from, which sync module bytes the server
  runs, who says so, and the external services it proxies
  (`services.geocoder: {path, query}` with `-geocoder`).
- `/streets` — the canonical streets document imported from the
  configured layer (`-streets`), bare (the bytes `load_streets`
  takes); gzipped, its ETag the streets hash, re-imported when the
  file changes (mtime-checked). A client refuses it unless the
  module answers the ETag's hash (`servedStreets` in
  `src/uis/kit.ts`).
- `/walks` — the walk table over the published streets and network
  (both `-streets` and `-feed`), derived again when either changes;
  gzipped, its ETag the walks hash, refused by a client the same
  way (`servedWalks`).
- `/network` — the import's answer for the configured zip
  (`-feed`), served as-is: `{network, boundary, notes,
  network_hash}`. Gzipped when the client takes it; its ETag is the
  network hash, so a client holding it revalidates for a 304.
  Republishing the zip is re-imported on the next request
  (mtime-checked). A client registers the network with
  `load_network` and refuses it unless the module answers the
  pinned hash (`servedNetwork` in `src/uis/kit.ts`).
- **Authentication/authorization** (D-58..D-61): with any `-oidc-*`
  flag set, the server verifies bearer JWTs — JWKS (`-oidc-jwks-url`,
  cached by `kid`, refetched on unknown kid) or a pinned PEM
  (`-oidc-key`), RS256/ES256 only, iss/aud exact, exp/nbf with leeway —
  and asks the hosted cap-authz module (`-authz-wasm`) over the policy
  document before any write: reads stay public (D-58), writes
  (`POST /edits`, `POST /pointers/*`) need a role the policy grants
  write on that pointer. 401 bad/missing token, 403 denied; the
  verified `sub` is the store log's `writer`. The policy is the store's
  `policy` pointer, seeded from `-policy <file>` at first boot (D-61);
  later changes are authenticated CAS commits. No `-oidc-*` = auth off,
  logged loudly (dev only). `STORE`/`OIDC_*`/`POLICY` env vars pass the
  flags through `just serve`/`just dev`.
- `/edits` — the edit document: `POST` a client document, receive
  the LWW-merged result (cap-sync's `sync_edits`, with the receive
  path clamping client timestamps to the server clock, D-56); `GET`
  reads the server's. With `-store` this is the `edit_records`
  pointer, CAS'd through the store (a CAS conflict retries against
  the re-read document); without it, `-edits <path>` persists it as
  one flat file across restarts, else in memory.
- Store mode also exposes the generic surface (`-store` only):
  `GET /pointers/{name}`, `GET /pointers/{name}/log?since=<hash>`
  (delta sync over the append-only log), `POST /pointers/{name}`
  (`{from, …}` → 200 + new head, 409 + current on conflict), and
  `GET /blobs/{hash}` (raw bytes, ETag the hash, envelope in headers —
  except kind `policy` blobs, which are never served: 404). `GET
  /manifest` serves the `manifest` pointer's blob, re-committed when the
  published feed, streets, module, or services change. With auth on, the
  log endpoint requires a read (an `audit` rule implies it) and redacts
  `writer` unless audit is granted; `GET /permissions` (any valid token)
  returns the caller's effective-permissions projection (cap-authz
  `effective`) — the only client-readable form of the policy.

`go test ./...` in `server/` proves the server runs the exact
module bytes the manifest pins and answers what the browser does.

## web/

Svelte 5 runes + Vite, TypeScript end to end, as a multi-page
build: `/` is the PICKER (`index.html` → `src/picker/`), one card
per generated UI (below) and a link to the original app. Every page
lives at the web root because asset fetches (`caps/manifest.json`,
`basemap.pmtiles`) are relative.

**The original app** (`app.html` → `src/main.ts`, `App.svelte`) is
ONE shell: a sidebar
(persona tabs + the active workflow) beside a PERSISTENT map canvas
that never remounts on a persona switch; persona-owned layers
DERIVE from app.persona. The map is A VIEW, not a fixture: data
screens hide it and the rail takes the full width
(`.map-side.hidden`; entity and service-type screens frame things
on it). Data sources (feed, rt, streets, population layer) is a
MODAL, not a floating panel. The Planner persona is a WORKSPACE of
screens with a back stack, ROOTED AT SERVICE PLANS (the chain: in
force / upcoming / superseded, fork = the service-change workflow);
below that the network overview (per-service-type analysis —
planners design types, not calendar dates) and drill-downs per
entity. There is no planner drawer.

### Parity

An agent (caps-run), the Go server, and the browser a human uses
run the SAME ops and get the SAME bytes. Any answer the browser
computes in TypeScript is an answer the other two hosts cannot
give — so `web/` holds host glue (I/O, fetch and pagination,
IndexedDB, the clock, settings, polling, the sync POST, the
capability resolver) and RENDER components (props in, events out,
formatting), and nothing else. A join, fallback, threshold, record
builder, or geometry rule in TypeScript is a parity break, however
small: it lands as an op instead.

Two rulings that follow from it:

- **The live document.** cap-rt answers "what is live" in MODEL
  vocabulary: an rt snapshot plus the network resolve into a
  kernel document of delays per trip instance and stop and of the
  service alerts in force, addressed by hash; cap-board (and the
  router) consume it by hash. Which alert touches which departure
  is the board's answer, never the host's. rt trip ids resolve
  INSIDE the op, never in a host. Thresholds (the on-time
  window) are op parameters.
- **Geometry is a model answer.** "What path does this trip take
  today" (diversions, the authored alignment, an itinerary leg's
  ridden segment) answers in an op; `web/` only draws the GeoJSON
  it is handed.

The WEB GATES in `just test` hold it: no component imports the
resolver, no host code reads raw rt `stop_updates`, and nothing
clones a document to mutate it (every change is a NetworkEdit
through cap-author). Each gate's allowlist only shrinks, and all
are empty.

File ownership:

- `src/mapview.ts` only shapes GeoJSON: every line the map draws
  is an answer from the bus (cap-runcut `trip_path`, cap-router
  `itinerary_paths`, cap-directory `focus`, cap-divert's draft
  alignment); it turns `[{lat, lon}]` into `[lon, lat]` and
  computes no geometry.
- Components are RENDER-ONLY: props in, events out, or reads of
  `app` state plus calls into the host layer (actions.ts,
  planner.ts, capdemo.svelte.ts). No component imports the
  resolver (the WEB GATES in `just test`).
- `src/capabilities.svelte.ts` is the bus resolver — the ONLY wasm
  loader (components never see the ABI): manifest
  validation, demand-driven load, byte verification against pinned
  hashes, per-capability degradation, and the IMMUTABLE document
  store (every version hash-chained by produced_by; `current` is a
  pointer; migrations move it forward, rollback moves it back;
  IndexedDB-persisted). CapabilityDemo.svelte is the visible story
  (assembly, hash handoff, degraded state, migrate/rollback) in the
  DataSources modal; capdemo.svelte.ts runs it.
- `src/planner.ts` sends the planner's changes to cap-author as
  NetworkEdits (in order, each landing on the plan it was made
  against) and adopts the answered document; it also owns the
  screen stack, and asks cap-directory for the list screens' rows,
  the selected route's and stop's composition, and every kind's
  rows for the pickers (`app.plannerRows`, `app.routeDetail`,
  `app.stopDetail` — each tagged with what it answers, so a late
  answer never renders under another screen — and `app.pickers`).
- `src/state.svelte.ts` owns the documents (edit document, plan
  set — both IndexedDB-persisted, wholesale-replaced) and the
  loaded feed's directory store (stops, routes — written by the
  resolver's `ensureDirectory`). Every next
  edit document is an answer (cap-divert propose / retire /
  restore, cap-sync's converge); a stored or synced copy is read
  through cap-divert's `sanitize` (`actions.readEdits`). Stored
  plans restore the same way: `restorePlans(readStoredNetwork)`
  reads each through cap-author (an `apply` with no edits answers
  the canonical document), so a plan saved under older field names
  comes back in the current shape; a bare `restorePlans()` restores
  them as stored, and `readPlan(id, readStoredNetwork)` reads one on
  first use. The host never sniffs a shape.
- MapView owns all map state (`window.__map`). `src/basemap.ts` is
  the basemap choice every map view shares (local pmtiles, else
  OpenFreeMap) and the MapLibre worker wiring.

### External services

Parity holds the bus's answers; it does not say what a host may do
with something OFF the bus. External things are sorted by what
their answer BECOMES, not by how they are fetched (`/rt` and
`/geocode` are the same proxy and different kinds):

| kind | its answer becomes | examples | rule |
|---|---|---|---|
| source | a document on the bus, by hash | the GTFS zip, a street layer, the GTFS-rt feeds | a module registers its bytes; ops read it by hash |
| presentation | pixels — no op reads it | basemap tiles, glyphs, sprites | the map's; nothing else touches it |
| oracle | an input the user could have given by hand | the geocoder | below |

**The oracle test: could the user have done it themselves?** A
geocoded point is a tap on the map; a geocoded name is a label the
rider could have typed. An oracle only makes an input easier to
give, so every op stays deterministic in its inputs and cannot tell
where a point came from. A service whose answer no user could give
by hand — another agency's itineraries, say — is not an oracle: it
is a source (its answer becomes a document an op reads), or it is
not used.

The contract:

- **A host may** ask the oracle with the user's input only (the
  text typed, the point tapped, where the map looks), render the
  answer exactly as given, in the oracle's order, and turn the item
  the user picks into the input the user could have given: a point,
  and the label it was shown with.
- **A host may not** rank, filter, merge, or threshold the answers
  (that is policy), or join them with a document (that is an op).
  When the answers must meet model data — D-28's "rank stops and
  addresses together" — the oracle is PROMOTED: its candidates
  become an input document, like an rt snapshot, and an op joins
  them.
- **Ops may echo an oracle's label, never branch on it.** The
  router repeats a geocoded pin's name in the itinerary (D-37); no
  op decides anything by it.
- **The query policy is the deployment's** (which layers, how far,
  how many): the server's policy file (`server/geocoder.json`,
  `-geocoder-policy`), applied by its proxy, which forwards only the
  user's input from the page. Every host that asks through the path
  asks the same question.
- **Discovery is the manifest's.** `/manifest`'s `services` lists
  each service the deployment offers (`{geocoder: {path, query}}`);
  one it does not list is off, and the frames that need it say so —
  the host never probes.
- **Transport is the deployment's origin**, so a user's queries go
  no further than the service the deployment chose.
- **Durability:** nothing durable keeps a query or an oracle's id
  for a result (Pelias's `gid`) — only the user-equivalent input
  (the point and its label). The oracle's data drifts; a saved trip
  end must not drift with it.
- **Failure:** an oracle only makes input easier; down or slow,
  the user types a stop, taps the map, or shares a location, and
  nothing waits on it.
- **Parity:** caps-run asks an oracle the way a page does (`--ask`
  through the deployment's path) and chains its answer into the
  ops; the answers themselves drift with the oracle's data, so
  sessions record them as reference, never as fixtures.

### Generated UIs

A generated UI is a layout built for a set of questions, made from
THIN components over its own host layer. It never builds on the
original app. Each one is a directory, and the build, the picker,
and the gates discover it. Nothing central is edited to add one.

```
web/<id>.html                  the page (copy service-design.html; change the script src)
web/src/uis/<id>/ui.json       the descriptor the picker renders: id (= the directory),
                               title, summary, questions, ops, components, session (the
                               dogfood session: date, feed, open findings), spec, generated
web/src/uis/<id>/main.ts       imports ../base.css (+ maplibre css if it maps), mounts
                               Shell, then awaits the host's boot (the layout paints first)
web/src/uis/<id>/Shell.svelte  PLACEMENT ONLY: reads the host's answers state, passes
                               props, routes events to host functions
web/src/uis/<id>/host.svelte.ts the ONLY file of the UI that calls the resolver; owns
                               I/O, the selection, and one $state object of answers
web/e2e/ui-<id>.spec.mjs       the UI's e2e spec (fixture feed: e2e/fixtures/)
```

Shared by every generated UI:

- `src/uis/base.css`: color tokens (`--surface --text --muted
  --line --hover --accent --accent-soft --bad --warn --ok`) in light and
  dark, the page base, `.wb-scroll` (a table that scrolls in its own
  box with an opaque sticky header), and the class names the shared
  components render with (`.muted`, `.table-scroll`, `.chip`,
  `.picker-row`). A generated UI NEVER loads `app.css`: that file's
  card and table rules belong to the app shell, and they broke the
  first generated UI's layout.
- `src/uis/kit.ts` (host layer only): `hashIn(module, doc)` gives a
  document's hash in one module, registered once per module per
  document version; `rememberHash` adopts a hash cap-author's
  `apply` already answered; `today()` is the host's date. Host
  layers call the resolver's `*Cap(module)` wrappers and
  `*Module()` getters (`src/capabilities.svelte.ts`) directly.
  The answer graph is TRANSIENT — hand-built ahead of the engine
  `docs/units/host-discipline.md` designs, which replaces it: a host
  declares its answers as `AnswerNode`s (the inputs each reads, the
  frame that renders it, when that frame is on screen), and
  `answerGraph` answers the nodes downstream of a changed input in
  declared order, yielding between nodes so each frame paints as
  its answer lands; a request made mid-pass supersedes the pass,
  and a hidden frame's node waits until it shows. `answer(module,
  op, args, run)` caches an answer by (module bytes, op, argument
  hashes) — the journal's key. Neither carries domain knowledge.
- `src/state.svelte.ts`'s document store (`getAllPlans`,
  `putPlan`, `setActivePlan`, `restorePlans`; the edit document)
  is shared with the original app. JS owns the documents. A host
  reads every plan it uses through cap-author before using it — at
  boot with `restorePlans(readStoredNetwork)`, or on first use with a
  bare `restorePlans()` and `readPlan(id, readStoredNetwork)` (the
  service-design host: a workspace of many forks reads the two it
  opens). The UI reads documents in JavaScript, which has no serde
  aliases; only the `plan` frame, which every stored shape spells
  alike, may be read before. The `app`
  object is the original app's screen state, and generated UIs
  never import it.
- `src/basemap.ts`: the basemap style choice and the MapLibre
  worker wiring, for any map view.

The component catalog. A component is THIN if and only if it
imports none of the app shell's host modules (`state.svelte`,
`planner`, `actions`, `capdemo.svelte`). The gate checks that rule
on every component a generated UI imports.

- Thin, usable by any generated UI: Unavailable (a frame with no
  answer, said plainly: what is missing and why — an op or decision
  it waits on, or a capability that failed, with a retry), the
  service-design set: DesignRail (the views, keyed 1–6),
  ContextBar (plan, baseline, group, route, stop, hours, health, a
  route jump), PickTimeline (plans `periods` as bars, `resolve`'s
  plan named), PlanList (the plan set, fork, feed import),
  GroupTable (`service_groups`), GroupLedger
  (`service_type_compare` totals), DayProfile (`hourly_trips`, a
  drag sets the hour window), FrequencyGrid (`route_frequency`:
  hours as cells, shaded continuously by the longest wait),
  CoveragePanel (`type_coverage` with its parameters and window,
  `unserved_points` listed), CoverageMap (`unserved_points` drawn),
  HubMatrix (`hub_connections`), CorridorTable (`route_overlap`),
  StopWaits (`stop_gaps`), RouteHeader (a route's `route_frequency`
  numbers, before → after), StringLine (directory `timetable` as
  time against stop; a dragged trip is a nudge event), RunTimes
  (`run_secs` by start), HourGaps (a route's `route_frequency`
  hours), GesturePanel (span, running time, one trip, stops — one
  event each), ChangeLedger (`service_type_compare` routes, undo and
  redo events), ProposalScore and Redline
  (`service_type_compare`), Readiness (issues, warnings, the export
  event); RouteCard (`route_detail`), ScheduleChange (one
  pattern trips' starts with add and remove events), DirectionTabs
  (`route_detail` directions), TimetableGrid (directory `timetable`,
  added, retimed, and removed trips when asked against a baseline; a stop
  column click is an event), FocusMap (a
  directory `focus` drawn and framed), BlocksPanel (runcut
  `blocks` / `block_trips`), DepartureBoard (`stop_board`), RtPanel
  (the live summary), TripPlanner (from, to, depart → `onplan`),
  Combobox (a searchable picker), and the rider's set: RideMap (every
  stop, the live buses by status, a route's `focus`, a trip's
  `itinerary_paths`, the rider's location; a tapped stop or spot is
  an event), PlaceSearch (a debounced type-ahead over directory
  `stops`, and under them the geocoder's addresses when the host
  passes them — two lists, one box, D-28 — keyboard-driven), DepartureRows (a board's rows —
  `stop_board`, or a watched stop's from `watchlist_board` — with
  their expected time and status),
  StopCard (a stop at a glance), RiderBoard (one stop's board, its
  alerts and skipped visits), AlertList (the live document's
  alerts), TripPlan (the trip's ends, when, and the router's answer)
  with Itinerary (the answer as a timeline), RouteList (directory
  `entities` for routes, filtered as typed), RouteView (one route's
  directions and their variants, as line diagrams), BusView (one
  live bus and its trip's `trip_progress`, stop by stop), and
  LiveFeeds (whether
  the times are live, and the feed URLs); and the stop display's set:
  DisplayHeader (the stop's name and id, the host's clock, live or
  scheduled), DisplayGroup (one `grouped_board` group as a station
  list's row, one route large, or a band across the screen; each
  time its `wait`, its status, and at a station its bay; a group with none left says it
  is detoured or has stopped for the day), DisplayAlerts (the alerts
  slot, always on screen), and DisplaySetup (the three layouts and
  the stop type-ahead). `rideFormat.ts` holds their formatting
  (clock times, a row's wait, status words and tones). Each
  frame whose capability can fail takes an `off` prop: a capability
  that did not assemble turns its frame off, and the frame says so.
- Coupled to the original app (do not compose; build a thin one
  instead): MapView, PlannerRail, PersonaTabs, DataSources,
  CapabilityDemo, PlanOverlay, DiversionPanel, and the entity forms
  (RouteForm, StopForm, PatternForm, ScheduleForm,
  ServiceTypeForm, BlockForm).

The procedure. Every generated UI follows it in order, and a step
that is not done blocks the next. A UI's open work is its unit,
`docs/units/ui-<id>.md` (`docs/units/README.md`); the
unit is written before any code and deleted when the UI lands.

1. **Name the need and its questions.** Open the unit: the need the
   UI serves (named for the need, never a persona — a persona is
   one example of who has the need), and the questions it answers,
   in the order they are asked.
2. **Inventory the ops, from the manifest.** For each question:
   the op that answers it (`caps-run tools <cap>`), or the open
   finding that says none does (`docs/QA-FINDINGS.md`). This is
   the unit's first draft. It is a list of what to test, not yet
   a list of what works.
3. **Dogfood the questions on real data** through `caps-run`
   (`docs/TESTING.md`, the dogfood rules), one op per question.
   Each question that needs anything besides passing one op's
   answer through to the screen is an op gap, and it goes into
   `docs/QA-FINDINGS.md` as a finding. That covers a host join, a
   threshold, a derived number, a second op stitched to the first,
   bisecting inputs until an answer changes, or a rule the
   rendering would have to apply. A session re-confirms the
   findings it hits. The unit's inventory then names, per question,
   either its op (as a dogfooded fact) or its blocker (`QAF-N`,
   and `Blocked: D-N` when the fix waits on the user).
4. **Build only what is unblocked.** No host, shell, or component
   is written while the session in step 3 has not run. A blocked
   question is unblocked by an op (laws first, library first, the
   capability wrapper second — `crates/AGENTS.md`), never by host
   code. The UI may land answering only its unblocked questions:
   `ui.json` lists the questions it answers, and the unit keeps
   the blocked ones with their findings.
5. **Draw the answer graph before the layout.** In the unit, one
   row per node: the op (or a run of ops over the same inputs),
   the inputs it reads (documents, selection keys, the clock, a
   live snapshot, a visibility flag), the frame that renders it,
   and when that frame is on screen. The table is the UI's first
   draft of its flow map (`docs/units/host-discipline.md`): the
   engine will be designed from these tables, so a question the
   table cannot express (a timer, a stream, a node whose inputs
   depend on another node's answer in a way `needs` cannot say) is
   written into host-discipline.md as an engine requirement, and
   `kit.ts`'s transient runner is NOT extended to cover it.
6. **Write the host from the table.** `host.svelte.ts` declares the
   table's nodes as `AnswerNode`s and requests inputs from event
   handlers. The only things a host may do: read and write I/O
   (files, fetch, IndexedDB), read the clock and pass it as an op
   argument, hold the selection (including repairing a selection
   the new answer no longer holds), track which frames are on
   screen, and adopt answers as-is. It never joins, filters,
   sorts, counts, thresholds, or falls back by a domain rule. A
   policy value is an op parameter whose default the op owns. The
   test: if `caps-run` or the Go server would have to re-implement
   it to get the same answer, it belongs in an op.
7. **Then the components, then the shell.** Thin components in
   `src/components/` (props in, events out, formatting only), then
   `Shell.svelte` (placement only).
8. **Land it.** Add `ui.json`, the page, and
   `e2e/ui-<id>.spec.mjs`. Run the UI in the browser on the real
   feed (`just serve testdata/ctran-gtfs.zip`) and check its numbers
   against the runner session's. Record the session in the
   descriptor's `session` field, and describe the UI below. Delete
   the unit if no question is left blocked, naming its pins;
   otherwise edit it down to the blocked questions.

Gates (`just test`, GENERATED UIs and the WEB GATES): every `src/uis/<id>/`
has its `ui.json` (with fields complete, and id equal to the
directory), its page, and its spec. No generated UI or picker file
imports the resolver from a `.svelte` file, or imports planner,
actions, capdemo, App, or the `app` object. Every component a UI
imports is thin. The build also refuses a UI directory without
its page (`vite.config.js`), and the picker's spec
(`e2e/picker.spec.mjs`) checks one card per `ui.json`.

The UIs today: `service-design` (`src/uis/service-design/`): six
views over one selection — a rail of views (keys 1–6) and a context
bar (plan, baseline, service group, route, stop, hour window) frame
whichever is open. Plans: the pick timeline, the plan set (fork,
baseline, feed import), the service groups. Network: the group's
numbers against the baseline, trips per hour (a drag sets the hour
window every view reads), and the frequency grid; a row opens the
route. Coverage: the parameters and window, shares per service id,
the unserved points listed and mapped. Hubs: a stop's connection
matrix and waits, and the shared corridors. Route: the string-line
workbench — directions, the string-line or the timetable grid,
gaps by hour, running time by start, the change ledger with undo
and redo, the route's map, and (edit mode, E) the gestures: span,
running time, one trip moved alone (picked or dragged), stops. A
frequency target is shown as waiting on D-54. Proposal: the
scorecard, the redline, readiness, and the export. The selection
lives in the URL hash. With no stored plan it opens the server's
`/network` as the base plan. Its host answers through the answer
graph; a view's nodes answer only while it is open, each frame
records its own failure (`sd.failed`) and renders Unavailable with
a retry while the rest answers, and the plan's health adopts
cap-author's issues for a document it authored. Undo restores an
earlier document version the host keeps; nothing is inverted.
`ride-now` (`src/uis/ride-now/`): a full-bleed map with a panel over
it (a card at the left on a wide screen, a sheet over the bottom of
the map on a phone), the live pill and the alerts button in the
corner. The map draws every stop and every live bus (colored by its
status, pointing its bearing, labeled with its route); a tapped bus,
stop, or spot opens it. The panel follows the rider: home (the
type-ahead — stops, and addresses and places when the server runs
a geocoder: a picked address is a spot, "Near 1400 NE 136th Ave" —,
"Get directions" and "Routes", the places near the rider
or a tapped spot with each one's next departures, saved stops,
recent stops — nearest by the walk along the streets when the
server publishes them), one stop (its live board; a picked departure draws
its route, marks its bus, and offers to track it), one bus (its
trip stop by stop, the stop it heads to marked, followed on the
map), the routes and one route (its directions' variants as line
diagrams; its buses stay bright on the map, the rest dim), getting
there (the ends as type-aheads, or the rider's location, or chosen
on the map — "Choose on map" per end: the next tap sets it, a stop,
or with the streets published any spot, a dropped pin named by the
geocoder's nearest address (D-37); an address typed in an end's
field is a point end the same way; "Directions
here" on a tapped spot does the same; a point end plans
as a router place over cap-streets `access`, its walks drawn along
the streets; leave now, leave at, arrive by, or the last bus; the
itinerary as a timeline, drawn on the map), and the alerts in
force. It boots from the server's `/network` (or a zip the rider
loads), then loads the server's `/streets` and `/walks` in the
background (`servedStreets`, `servedWalks`; until they land, or
where none is published, walks are straight lines and trips run
stop to stop; a hand-loaded zip walks no streets — the walk table
walks the published network), reads the edit document from
`GET /edits`, reads the live feeds the deployment names in
`public/live-feeds.json` (the rider may set their own) through
`/rt`, and polls both and the clock every 30 s. A trip asked for
"now" is re-asked on each poll. Addresses come from the geocoder
the manifest lists (read once on boot; "External services"): the
host sends the typed text and the near point or the rider's, shows
Pelias's answer as given, and a point it answers keeps the name it
was shown with; nothing keeps the query. Its open work is
`docs/units/ui-ride-now.md`.

`stop-display` (`src/uis/stop-display/`): the screen at a stop, a
passenger information display, in three full-screen layouts over
one `grouped_board` answer — a transit center (the groups as a list,
each time naming its bay, the alerts beside it), a single route (one
group filling the screen, its type scaled to it, for the stop one
route serves), and several routes (a
street stop's two or three, each a full-width band sharing the
screen's height, its type scaled to the band); the alerts slot is
always on screen. Any layout shows any stop. The stop and the layout live in the page's URL
(`?stop=<id>&layout=center|route|routes`), so a screen reopens on its
stop; without a stop, the setup picks both. It boots from the
server's `/network` (or a zip), reads `GET /edits` and the deployment's
live feeds (`public/live-feeds.json`, through `/rt`), and polls them
and the clock every 20 s. Its open work is
`docs/units/ui-stop-display.md`.

Layouts are hand-placed Svelte; layouts as documents are
`docs/units/wire-time-assembly.md`.

Frontend conventions and gotchas: `docs/CONVENTIONS.md`.

## The capability bus

The architecture: horizontal capability modules, one NEED per
module, assembled per user at WIRE time from a manifest —
capability bytes differ per user while the DOCUMENT and the
contracts stay identical everywhere. Every host instantiates the
modules it needs — the browser fetches them from the manifest,
the sync server hosts cap-sync.

```
cap-abi         The seam crate: the flat C ABI as a macro (every
                capability module is the same four exports), the
                capability-manifest contract (speaks/migrates/
                consumes + agent context), the ASSEMBLY VALIDATOR
                (compose-time proof: closure, version coherence,
                migration-path reachability — total and property-
                tested; gen-manifest self-validizes before writing),
                and gen-manifest (the capability set as code; the
                manifest diff IS the reviewable assembly change; its
                worked examples are computed, and cap-author apply's
                carry one example per edit kind, applied to the
                fixture in order — a kind without one, or one that
                stops applying, fails the build).
                The kernel registration lives here too: bulk op 256
                (OP_LOAD_NETWORK — the bare network document on
                transit_invoke_bytes, parsed straight into the
                model; the JSON load_network routes to the same
                parse via load_network_json) and canonical_hash,
                the one identity formula (streamed into the
                digest, never a second copy of the document).
                Also caps-smoke.mjs: Node ABI smoke + the law that
                every advertised op is documented (undocumented ops
                fail the build — agent context cannot drift).
cap-gtfs        The feed-document capability: GTFS zip →
                content-hash feed registry, and the feed's OWN
                questions only — what it contains at a glance
                (load_feed's summary) and its identity (feed_info).
                The day views, the watchlist, the pickers, and the
                names join are NOT here: they answer over the NETWORK
                document (cap-board / cap-runcut / cap-calendar /
                cap-directory — one question, one answer on the bus).
                Parse + identity alone, over transit-codec's whole-
                spec `Feed`. CHARTER
                CLOSED: parse, export, primitive boundary queries —
                nothing more goes "in GTFS."
cap-router      The router capability — ONE QUESTION: how do I get
                from here to there? — over the NETWORK document and
                the effective day — the model's materialized trips,
                patched with the edits in force (the same
                transit-dispatch composition every day view starts
                from), read at the live document's predictions when
                live_hash names one (load_live), its transfers
                walking the streets when walks_hash names a walk table
                (load_walks — cap-streets' `walks` answer, refused
                when derived for another network; straight lines
                without one), the RAPTOR graph cached per (network,
                edits, live, date, walks; only the newest live
                document's graphs stay). A plan's `from` and `to`
                are stop ids or places (`{id, name, walks}` —
                cap-streets' `access` walks pass as they are) with
                `direct` the walk between them (`access`'s). plan
                leaves at a time; plan_arrive_by answers the latest
                departure that arrives by a deadline (none: the last
                bus home). itinerary_paths, given the plan's ends
                and `direct`, draws a place's walks along their
                street paths (`transit_router::walk_path`); without
                them a place draws nothing.
                The compare facet is the diversion delta:
                the unpatched day read at the same predictions, one
                materialization apart, with its notice; the
                itinerary_paths facet draws a plan's legs over the
                same day (a transit leg's ridden segment, a walk
                through its via stops). The browser plans through
                plan_compare alone (no edits hash: no baseline) and
                shows its notice's text. A pure-function capability; it
                writes nothing.
cap-analysis    The analysis capability: planner numbers (stats,
                service types, the frequency grid, hub connections,
                coverage) over hash-keyed network and
                population-layer registries. The domain code lives
                in the shared transit-analysis library. Consumes
                network documents from cap-network (the registry
                edge the manifest carries).
cap-network     The network codec capability: load_feed (zip bytes →
                content-hash feed registry), import (the planner's
                bootstrap — the document registers itself with its
                boundary residue, and its hash is in the response),
                load_network (the
                same registration name every model-side module uses),
                validate (the authoring feedback loop: issues that
                gate export, warnings that never gate), export (the
                publish path — deterministic GTFS zip bytes under
                their content hash, base64 across the JSON seam, with
                the import's boundary when it is the imported
                document itself). The domain code lives in the
                shared transit-codec library.
cap-rt          The realtime capability: decode — a GTFS-rt protobuf
                payload (bulk-bytes op 0, the ephemeral bytes an
                agency serves every few seconds) into the overview
                view (delays per trip/stop, live vehicles, service
                alerts in the feed's ids), each
                snapshot registered under its content hash; and the
                SCHEDULE JOIN the dispatcher actually asks for, over
                the NETWORK: load_network (the identity every
                network consumer holds) and load_boundary (an
                imported network's published stop_sequence
                numbering; optional), trip_progress (one trip's scheduled/delay/predicted
                board; a stop the update skips carries the last
                stated delay, GTFS-rt's propagation rule), late_trips (ranked, with the DELAYED-BUT-
                DARK flag and vp_hash for agencies that serve the
                two feeds separately — C-TRAN does). Dark is judged
                per BLOCK: a delay propagated to a block's next trip
                while its bus runs the trip before is a located
                vehicle (position_trip_id names the trip it is on;
                vehicle_id rides beside the label). live mints THE
                LIVE DOCUMENT (transit-live) from the
                trip-updates, vehicle-positions, and/or service-alerts
                snapshots (alerts_hash; every snapshot's own alerts
                count, one per entity id), with
                the summary a status panel reads (vehicles by
                status, the late list) — the browser's live views
                render it; none joins rt.
cap-board       The board capability — ONE QUESTION: when does
                service leave this stop? (at a stop, a station's
                pooled bays, or a personal saved set — the WATCHLIST
                document, v1/v2, migrated here: a personal
                multi-stop board is the same question parameterized
                by a personal document). load_network / load_edits
                (validated per record against the network, invalid
                records REPORTED never rejected) / load_live (cap-rt's
                live document, by hash) then stop_board (THE board:
                pooling, realtime-first order, row status, the
                route's long name, the bay's name, the sign's
                destination, the visits the in-force edits skip and
                the diversion that skips each, the live alerts about
                the board — the module's answer, never the host's) /
                grouped_board (the same rows by route and
                destination, the next three of each by default — the
                stop display's answer) / load_watchlist /
                migrate_watchlist / watchlist_board (each watched
                stop's stop_board, the same edits and live document;
                a trip's FINAL
                visit is an arrival, never a row) over ONE
                MATERIALIZED DAY (materialize(net, date), patched
                with the in-force edits — no derived registry, no
                GTFS expansion, no effective cache; a day is computed
                per call and a day is cheap). The domain code lives
                in the shared transit-dispatch library; no GTFS
                layer (arch gate). The UI's departure board renders
                stop_board's answer; rows carry the trip instance's
                address (trip, start), so the click-through needs no
                join.
cap-runcut      The runcut capability — ONE QUESTION: what does
                this vehicle do today? The day's blocks LIST (the
                index facet: vehicles out, with that day's running
                trip counts; a block whose every trip is
                unrepresentable or skipped is not offered — the
                honest hole), the day's FLEET SUMMARY (peak
                concurrent vehicles and when, platform / revenue /
                layover hours, and each running block's outline —
                span, trips, longest internal gap — the fold that
                took a block_trips call per block), and one block's
                chain (the detail
                facet: the AUTHORED runcut with the layover ripple —
                the in-force edits' added time, or with a live
                document (load_live) a trip's predicted delay at its
                last stop, which wins (D-18) —
                layover_remaining_secs going negative when a
                connection is eaten, and layover_health classifying
                it — overlap / tight / ok under the caller's
                tight_layover_secs; each row names its trip by id, a
                frequency trip once per instance) and one trip
                instance's EFFECTIVE
                stops on the date (the trip detail facet) and its
                LINE (trip_path — what the map's click-through draws:
                the authored alignment, with an in-force diversion's
                traced detour spliced between its kept stops). Same
                document set as
                cap-board
                (load_network / load_edits / load_live + the day
                views); the
                domain code lives in the shared transit-dispatch
                library; no GTFS layer (arch gate). The UI's blocks
                panel and trip lines answer through this module.
cap-calendar    The calendar capability — TWO READS of one relation:
                which service keys run today (active_services), and
                the reverse, the dates one service type runs
                (service_dates over an inclusive window). Both over
                the EFFECTIVE day (a key whose every trip is skipped
                or unrepresentable is not running, however its
                calendar reads — the same honesty as the runcut
                view's empty hole). Same document set (load_network
                / load_edits plumbing); the domain code lives in the
                shared transit-dispatch library; no GTFS layer (arch
                gate).
cap-directory   The directory capability — ONE QUESTION: what
                exists in the network? The stop list (name-sorted,
                with the walking-group structure the station board
                keys on; an optional query narrows it the way a
                picker's typing does), the names join (stops_by_ids),
                the PLACES nearest a point (stops_near: great-circle
                meters, nearest first, ties by id, at most `limit`,
                within `radius_m` when given; a station's bays answer
                once as the station, at its nearest member's
                distance — the walking group the board pools; given
                the point's `walks` (cap-streets `access`), the
                places a walk reaches first, by walk — the shortest
                to any member — and the rest by distance, D-34; names
                are matched exactly, never by alias), the route list
                (the model's closed mode set; every row that names a
                route carries its long name — routes, entities,
                stop_detail's routes, the timetable header), and one route's
                PATTERNS with their ordered stops — what the
                diversion editor anchors against (new edit records
                carry direction_id null — the documented
                both-directions case). The pickers are
                model questions, never feed reads; the UI's
                comboboxes and the map's name lookups read this
                module (the feed line reads cap-gtfs's load_feed
                summary + feed_info). focus answers where an entity
                is on the map — the planner's drill-down frames it.
                entities, route_detail, and stop_detail answer the
                planner's lists (filtered rows per kind — the forms'
                pickers and the Document counts too; a route's
                composition with trip counts and its patterns and
                rider variants by direction; a stop's serving
                routes — its own, or a station pin's bays', the
                board's rule), and timetable
                answers the planner's timetable (route + direction
                + service type, across the direction's patterns) —
                the planner computes no number.
cap-streets    The street-graph capability: where along the
                streets. import (bulk-bytes op 0) reads a street
                layer into the streets document and registers it,
                answering the document (the host keeps it) with its
                hash and size; load_streets (bulk op 1, or JSON)
                registers a document the host holds under the same
                hash, refused unless canonical; snap answers the
                nearest street position (the graph address a trace's
                waypoint commits to); route answers the street
                geometry between two positions (shortest path,
                starting after `from` — trace legs chain — with
                its length along the graph; empty on disconnected
                parts — the caller falls back to a straight line);
                walks answers the walk table between a registered
                network's stops (load_network, kernel op 256 — the
                module keeps only the stops) with its hash, the
                router's load_walks answering the same; access
                answers a point's walks to those stops, each with
                its path, nearest first, and the direct walk to a
                second point — the router's place. The walking
                graph is built on first walks, the street graph on
                first snap — a host that only walks never builds the
                other. The domain code lives in the shared
                transit-streets library; the module writes nothing. Its consumer
                today is the detour editor (the original app's
                DataSources import; the map draws the document, so it
                shows exactly what routes); drawing a pattern on
                streets is the next (`docs/units/streets-model.md`).
```

### Bus rules (load-bearing)

- Capabilities compose through DOCUMENTS addressed by content hash,
  never module-to-module — the host loads the same bytes into each
  module and ASSERTS the returned hashes agree (`assertSameDocument`
  is the law).
- The MODULE is the hash authority — hosts never mint document
  identity (JS JSON.stringify differs from the module's canonical
  serde bytes in float formatting; query with the hash the module
  returned).
- Bulk op numbers: a capability's own number from 0 (feeds,
  layers, rt payloads); kernel ops number from 256 and mean the same
  thing in every module that answers them. Every network consumer
  answers 256, and the browser host registers networks only that
  way (`CapModule.loadNetwork`, the document encoded once).
- One Request enum per capability is the whole surface (networks
  are minted only by cap-author's apply and cap-network's import —
  readers have no authoring ops by construction).
- `#[no_mangle]` ABI exports collide if one binary links two cap
  crates (tests assert against the hash formula instead); cap
  crates need `crate-type = ["cdylib", "rlib"]` for native tests to
  link.
- Do not let a capability grow a private document format — shared
  data crosses modules as kernel documents addressed by content hash
  only.

### The registry and the runner

The registry (`registry/`, published by every `just caps`; `just
registry` runs the gates first): immutable blobs + one evidence
record per capability (contracts, agent context, which gates ran —
none for a plain `just caps` — at which commit, and the digest of
the source tree that built it) + a pointer. `caps-run` recomputes
the source digest on every run and says on stderr when the registry
was published from another tree (`just test` gates the agreement).
It is the
artifact agents and humans pull SANDBOXED COMPUTE from —
`tools/caps-run.mjs` verifies blob bytes against the pinned hash and
runs ops behind a worker-termination wall-clock guard (`--timeout`).
A session runs its steps in command-line order and may span
capabilities (`--cap` switches; each module verified and
instantiated once): documents cross between modules BY REFERENCE
(`"network":"$net.network"`), the way a host hands the same bytes to
each module, and `--as` names results so references survive
reordering. A session holds each result as the module's JSON text
and parses one only when a reference reaches into it — a parsed
real-size answer is millions of objects every later collection
re-marks. Sessions do not persist — a later session re-imports.
Registry records and the browser manifest are BOTH generated by
`cargo run -p cap-abi --bin gen-manifest` — one source of contracts,
two surfaces. `just test` gates the runner itself (the fixture feed
hash is stable, so registry/fixture drift fails the gate).
