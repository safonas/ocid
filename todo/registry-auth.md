# Loopback Token & Basic Authentication

## Context
The embedded OCI registry binds to loopback (`127.0.0.1:5050`) with no authentication, relying entirely on host-level OS boundaries. Any local unprivileged user or compromised process on the host can push images to the registry, effectively publishing them under the node's Ed25519 identity.

## Proposal
- Support opt-in or default local authentication:
  - Generate a secure ephemeral token on daemon startup stored in `~/.ocid/auth.token` (0600 permissions).
  - Support HTTP Basic Auth with user credentials matching standard Docker config format (`~/.docker/config.json`).
  - Require the `Authorization: Bearer <token>` or Basic header on mutating operations (`POST`, `PUT`, `PATCH`, `DELETE`).
- Provide seamless CLI login via `podman login localhost:5050 --authfile ...` or `ocictl auth`.
