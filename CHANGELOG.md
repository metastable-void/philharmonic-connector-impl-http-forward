# Changelog

All notable changes to this crate are documented in this file.

The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
this crate adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
- `build_client` now constructs its mhc `Client` with
  `pool_max_idle_per_host(0)`, disabling idle TCP / H2 / H3
  connection reuse for the upstream forwarded hop. The
  previous default reuse policy meant a second forwarded
  request to the same upstream could pick up a stale
  keep-alive connection the peer had already half-closed,
  surfacing as `Error::Cancelled` on the second call. For
  HTTP-forward connector traffic the reconnect cost is small
  relative to the harm of returning a transport error to a
  workflow run; reconnect now happens per-request.

## [0.2.1] - 2026-05-14

### Changed
- Internal Cargo.toml audit: `default-features = false` set on
  direct dependencies with explicit feature lists for what the
  crate actually uses. No behaviour change. (D24)

## [0.2.0] - 2026-05-13

Changed (breaking): outbound HTTP is now driven by `mechanics-http-client`
(hyper-rustls + webpki-roots + aws-lc-rs) instead of `reqwest`.

- `HttpForward::with_client(reqwest::Client)` →
  `HttpForward::with_client(mechanics_http_client::Client)`.
- Removed: the `reqwest` dependency.
- Trust posture: TLS root store is the bundled Mozilla CA bundle
  (`webpki-roots`) only — no OS-native trust, no
  `rustls-platform-verifier`. Crypto provider is `aws-lc-rs`;
  `ring` is no longer in the dep graph.

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
