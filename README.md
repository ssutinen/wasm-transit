# wasm-transit — local-first transit software

GTFS static and GTFS-rt, read by Rust and shipped as **capability
modules**: small WebAssembly modules with one flat, import-free ABI,
each answering one question over hash-addressed documents. The same
modules, the same documents, and the same bytes run in the browser,
in Node, and in a Go server — the browser is the main consumer, and
nothing is uploaded.

The core data model is the authored **network** (routes, patterns,
trips with their own ids and times, service types, blocks, fares,
stations, demand-responsive service). GTFS is how it is shared: a
feed imports into a network and exports back, losslessly over the
whole GTFS Schedule spec.

## Where things are

- `AGENTS.md` — the crate tree, the commands, the ownership rules,
  and the conventions. Start here, human or agent.
- `docs/ARCHITECTURE.md` — per-crate detail, the server, the web
  app, and the capability bus.
- `docs/TESTING.md` — what every change is tested with, and the
  real-data dogfood rules.
- `docs/units/` — the open work. `docs/DECISIONS.md` — the questions
  waiting on a decision. `docs/QA-FINDINGS.md` — what real-data
  sessions found. `docs/ROADMAP.md` — where it is going.

```
crates/   Rust: the domain libraries (transit-*) and the capability
          modules (cap-*) over them
server/   Go + wazero: the sync server — static files, the rt proxy,
          the manifest, the edit document
web/      Svelte 5 + Vite: a picker over the generated UIs, and the
          original app (/app.html)
testdata/ the authored-network fixture, real-data assets
```

## Getting started

Toolchains: Rust (with the `wasm32-unknown-unknown` target), Go
1.22+, Node, and `just`. Then:

```
cd web && npm install && cd ..
just build                                   # capability modules + frontend
just serve                                   # the server on :8080
just serve path/to/gtfs.zip                  # + /network (the zip, imported once) and the manifest
just serve path/to/gtfs.zip roads.geojson    # + /streets and /walks (the layer imported
                                             # once, the walk table derived from both)
just serve path/to/gtfs.zip "" "Test Transit"   # + the manifest's publisher
STORE=./store just serve path/to/gtfs.zip      # + the durable sync store: content-addressed
                                               # blobs + CAS'd pointers under ./store
                                               # (/pointers, /blobs; docs/FORMATS.md)
just dev                                     # vite hot reload + the server
```

Open http://127.0.0.1:8080. `just test` is the host-side gate,
`just e2e` the browser runner, `just check` both; the full command
list is in `AGENTS.md`.

The agent runner drives the same modules from the command line —
`node tools/caps-run.mjs list` shows the capabilities,
`node tools/caps-run.mjs tools <capability>` their ops.

## Basemap

The map prefers `basemap.pmtiles` in `web/public/` (Vite copies it
into the build; the server serves it with Range requests) and falls
back to OpenFreeMap (free vector tiles, no API key).

`just basemap "<bbox>" [maxzoom]` extracts a region from the daily
Protomaps planet build over HTTP range requests, so the planet is
never downloaded (needs the `pmtiles` CLI from
github.com/protomaps/go-pmtiles; a metro area at z15 is about 94 MB):

```
just basemap "-123.20,45.25,-122.10,45.95"
```

To build a region from OpenStreetMap instead (Java 21+):

```
curl -LO https://github.com/onthegomap/planetiler/releases/latest/download/planetiler.jar
curl -LO https://download.geofabrik.de/<your-region>-latest.osm.pbf
java -jar planetiler.jar --osm-path=<your-region>-latest.osm.pbf --output=web/public/basemap.pmtiles
```

Rebuild the frontend afterwards (`just build`). A metro extract is
small enough to keep next to the GTFS zip, which makes the whole app
— feed, queries, basemap — work offline.

## Address search

Stop search is the network's; addresses and places come from a
geocoder the server proxies at `/geocode` — a service, not data
the app holds, so without one the search finds stops only. To run
a local Pelias for the Portland–Vancouver area (Docker or podman
with podman-compose; several GB of data, about an hour to build):

```
just geocoder-build                          # once: download and import
just geocoder                                # start it on 127.0.0.1:4000
GEOCODER=http://127.0.0.1:4000 just serve path/to/gtfs.zip roads.geojson
just geocoder-stop
```

What the server asks Pelias for (which layers name a dropped pin,
how far, how many results) is the deployment's policy,
`server/geocoder.json` (`-geocoder-policy`); the manifest lists the
geocoder and its policy under `services`.

## Live feeds

The ride-now UI (`/ride-now.html`) and the stop display
(`/stop-display.html?stop=<id>&layout=center|route|routes`) read the
agency's GTFS-rt feeds
named in `web/public/live-feeds.json` (`trips`, `vehicles`,
`alerts`; an empty URL is a feed the agency does not have), fetched
through the server's `/rt` proxy (every 30 seconds for ride-now,
20 for the display). A rider can set their own from ride-now's live
pill. Rebuild the frontend after changing
the file.
