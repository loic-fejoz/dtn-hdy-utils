# ADR 1: Use of High-Level Application API for Bundle Receiver (`dtnprint`)

## Status
Accepted

## Context
We need to implement a receiver utility (`dtnprint`) that registers on a Hardy BPA endpoint, listens for incoming bundles, and prints their textual payloads to `stdout`.

The Hardy BPA framework offers two main interfaces for application/service integration:
1. **Application API**: Exposes a high-level trait `Application` where the BPA handles all bundle decoding, canonicalization, and validation. The application only receives the decoded payload (ADU) and metadata.
2. **Service API**: Exposes a low-level trait `Service` where the application receives raw CBOR-encoded Bundle Protocol version 7 (BPv7) bundles and is responsible for parsing them.

## Decision
We chose the high-level **Application API** to implement `dtnprint`. It implements the `hardy_bpa::services::Application` trait to receive plain ADU payloads and log events.

## Consequences
- The implementation of `dtnprint` remains simple, lightweight, and focused purely on printing incoming payloads.
- `dtnprint` does not need to depend on block-parsing or CBOR decoding logic.
- It is constrained to endpoints representing standard application destinations, and cannot access raw extension blocks or custom canonical headers directly.
