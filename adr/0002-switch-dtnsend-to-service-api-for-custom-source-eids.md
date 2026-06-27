# ADR 2: Switch `dtnsend` to Low-Level Service API for Raw Bundle Construction

## Status
Superceded by [ADR 3](0003-register-dtnsend-as-cla-to-bypass-source-eid-checks.md)

## Context
In testing loopback and echo service scenarios, `dtnsend` needs to construct and send bundles with a custom source EID (e.g. `dtn://f4jxq/incoming` where `dtnprint` is listening) so that the echo responder swaps EIDs and replies back to the listener. 

Under the high-level `Application` API, the BPA automatically overrides the source EID of outgoing bundles to match the registered EID of the sender client. Consequently, `dtnsend` could not specify a custom source EID.

## Decision
We switched `dtnsend` to use the low-level **Service API** (`hardy_bpa::services::Service`). This allowed the utility to use `hardy_bpv7::builder::Builder` to construct raw BPv7 bundles locally, populate them with arbitrary source EIDs, and dispatch the completed raw CBOR bytes via `ServiceSink::send`.

## Consequences
- `dtnsend` had to assume responsibility for building complete, valid CBOR bundles, utilizing the `hardy-bpv7` crate's builder abstractions.
- During integration testing, we discovered that the BPA's dispatcher verifies the source EID of raw bundles dispatched by registered services (`&bundle.id.source == expected_source` in `local_dispatch_raw`). If they do not match, the BPA returns an `InvalidDestination` error to prevent EID spoofing.
- Because endpoint/service registrations on a BPA are exclusive, `dtnsend` could not register under `dtn://f4jxq/incoming` while `dtnprint` was already running and listening on it. This made it impossible to run this loopback scenario using the Service API.
