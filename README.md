# philharmonic-connector-impl-http-forward

Part of the Philharmonic workspace: https://github.com/metastable-void/philharmonic-workspace

`philharmonic-connector-impl-http-forward` provides the `http_forward`
connector implementation: it deserializes a `mechanics-config::HttpEndpoint`
configuration, validates and executes outbound HTTP calls with `reqwest`,
applies endpoint retry policy and response-size limits, and returns a
normalized response payload through the shared
`philharmonic-connector-impl-api` contract.

## Contributing

This crate is developed as a submodule of the Philharmonic
workspace. Workspace-wide development conventions — git workflow,
script wrappers, Rust code rules, versioning, terminology — live
in the workspace meta-repo at
[metastable-void/philharmonic-workspace](https://github.com/metastable-void/philharmonic-workspace),
authoritatively in its
[`CONTRIBUTING.md`](https://github.com/metastable-void/philharmonic-workspace/blob/main/CONTRIBUTING.md).

SPDX-License-Identifier: Apache-2.0 OR MPL-2.0
