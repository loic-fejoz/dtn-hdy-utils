# ADR 3: Register `dtnsend` as CLA to Bypass Source EID Spoofing Checks

## Status
Accepted

## Context
To enable the loopback/echo integration scenario (where `dtnsend` sends a bundle with source `dtn://f4jxq/incoming` and `dtnprint` listens on the same EID), we need a way to dispatch a bundle with a spoofed/custom source EID. 

The BPA's dispatcher rejects raw bundles from registered services if the source EID does not match the service's own registered EID. However, the BPA dispatcher does not perform this source check on bundles received from the network convergence layer adapters (CLAs), as CLAs naturally receive bundles from other nodes with arbitrary source EIDs.

## Decision
We re-architected `dtnsend` to implement the `hardy_bpa::cla::Cla` trait and register itself dynamically as a temporary, mock convergence layer adapter (CLA) over gRPC using `RemoteBpa::register_cla`. The constructed raw bundle is then injected into the BPA's ingress routing pipeline via the CLA `sink.dispatch` call.

## Consequences
- `dtnsend` can now send bundles with any custom source EID (including `dtn://f4jxq/incoming`).
- The BPA processes the injected bundle as if it had been received from a peer node over a convergence layer, passing it through the normal ingress routing and delivery pipeline.
- This fully enables the user's loopback and echo-testing scenario without requiring `dtnprint` to stop listening.
- Registering a CLA requires implementing the `Cla` trait (specifically `on_register`, `on_unregister`, and a dummy `forward` method), which is slightly more complex than a standard application client but keeps the codebase clean.
