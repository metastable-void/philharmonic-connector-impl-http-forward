# Changelog

All notable changes to this crate are documented in this file.

The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
this crate adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-04-24

### Added

- Initial substantive `http_forward` implementation of the
  `philharmonic-connector-impl-api` `Implementation` trait.
- Config/request/response/runtime modules covering:
  - `mechanics-config::HttpEndpoint`-backed config validation and runtime preparation.
  - camelCase request deserialization and endpoint body-mode request encoding.
  - streamed response-size enforcement, response-body decoding, and exposed-header filtering.
  - full-jitter retry loop with Retry-After handling and overall retry deadline enforcement.
  - internal error taxonomy with explicit mapping to `ImplementationError`.
- Unit tests for config validation, request decoding, response header normalization,
  retry primitives, and error mapping.
- Wiremock-backed integration tests for happy paths, error cases,
  retry behavior, and fixed outbound request vectors.

