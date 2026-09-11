# Multi-Writer / Delegated Publishers

## Context
Currently, an image namespace is tied strictly 1:1 to a single Ed25519 keypair (`PublisherId`). Teams, organizations, and automated CI pipelines cannot share a publisher namespace without sharing a private key.

## Proposal
- Model after Radicle's delegate / quorum architecture.
- Root identity signs delegation certificates:
  ```json
  {
    "root_publisher": "<root-pubkey>",
    "delegate": "<worker-pubkey>",
    "scope": "app/*",
    "expires_at": 1750000000,
    "signature": "<root-sig>"
  }
  ```
- Release records include the delegation certificate when signed by a delegate key.
- Verifiers ensure the root identity authorized the delegate to sign releases for that namespace within the specified time window.
