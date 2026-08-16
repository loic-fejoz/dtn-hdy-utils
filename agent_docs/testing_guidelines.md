# Testing Guidelines

This document provides a guide on how to write, mock, and run tests for the `dtn-hdy-utils` project.

## Integration Testing
- Integration tests can verify end-to-end delivery by launching receiver utilities (`dtnprint`, `dtntrigger`, `dtnfiles`) and sender/diagnostic utilities (`dtnsend`, `dtnping`) against a local running Hardy BPA instance.
- To confirm successful registration and dynamic endpoint generation, launch `dtnprint`, `dtntrigger`, or `dtnfiles` and parse the generated EID.
- To send bundles to a specific endpoint, invoke `dtnsend` with a file or piped `stdin` payload.
- To verify BPSec integrity checking, configure a local keystore file and invoke the utilities with the `--keystore` and `--verify-policy` options to ensure signature validity/failure drop behaviors match expectation.
- Complete end-to-end integration tests are provided in the `examples/` directory:
  - [`test_basket.py`](file:///home/loic/projets/dtn-hdy-utils/examples/test_basket.py): Verifies the basket service request/response cycle.
  - [`test_forward.py`](file:///home/loic/projets/dtn-hdy-utils/examples/test_forward.py): Verifies the forwarding service (BIBE encapsulation) using `dtnforward`, `dtnsend`, and `dtntrigger`.

## Dry-run & Serialization Verification
- Offline tests of bundle assembly can be validated using the dry-run path which creates a bundle via `hardy_bpv7::builder::Builder`.
- Test assertions should verify that:
- The generated EIDs are parsed correctly using the EID parsing methods.
- The hexified CBOR output is valid according to the BPv7 specification (RFC 9171).

## Unit Testing
- For BPSec security (`src/security.rs`), verify key format auto-detection, pattern matching, signing, and verification under different key scenarios (wrong key, missing key, unsigned).
- For `dtnping`, verify duration parsing and CBOR encoding/decoding via unit tests defined in `src/bin/dtnping.rs`.
- For `dtntrigger`, verify temporary file generation and execution of external commands.
- For `dtnfiles`, verify auto file extension detection logic via unit tests in `src/bin/dtnfiles.rs`.

## Running Tests
Always run the validation suite before delivering modifications:
- Run `cargo test` to execute all defined test cases.
- Run `cargo clippy && cargo fmt --check` to ensure the codebase remains clean and warning-free.
- Ensure the `hardy` checkout `ref` in `.github/workflows/ci.yml` is updated whenever `hardy` dependencies, trait signatures, or patches evolve.


