# Testing Guidelines

This document provides a guide on how to write, mock, and run tests for the `dtn-hdy-utils` project.

## Integration Testing
- Integration tests can verify end-to-end delivery by launching `dtnprint` and `dtnsend` against a local running Hardy BPA instance.
- To confirm successful registration and dynamic endpoint generation, launch `dtnprint` and parse the generated EID. An example setup is shown in [dtnprint.rs:L107-L113](../src/bin/dtnprint.rs#L107-L113).
- To send bundles to a specific endpoint, invoke `dtnsend` with a file or piped `stdin` payload. See [dtnsend.rs:L88-L100](../src/bin/dtnsend.rs#L88-L100) for input ingestion.

## Dry-run & Serialization Verification
- Offline tests of bundle assembly can be validated using the dry-run path which creates a bundle via `hardy_bpv7::builder::Builder`.
- Test assertions should verify that:
  - The generated EIDs are parsed correctly using the EID parsing methods. Refer to [dtnsend.rs:L108-L125](../src/bin/dtnsend.rs#L108-L125) for EID construction.
  - The hexified CBOR output is valid according to the BPv7 specification (RFC 9171). See the hex printing logic in [dtnsend.rs:L134-L135](../src/bin/dtnsend.rs#L134-L135).

## Unit Testing
- For `dtnping`, verify duration parsing and CBOR encoding/decoding via unit tests defined in `src/bin/dtnping.rs`.

## Running Tests
Always run the validation suite before delivering modifications:
- Run `cargo test` to execute all defined test cases.
