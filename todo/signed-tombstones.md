# Signed Tombstones (Cryptographic Revocation)

## Context
Currently, deleting an image release or tag (`ocictl rm` or registry `DELETE`) is local to the node. A publisher has no cryptographic mechanism to deprecate, recall, or yank a compromised or broken release across peers in the swarm.

## Proposal
- Introduce a signed `Tombstone` record:
  ```json
  {
    "publisher": "<publisher-id>",
    "name": "<image-name>",
    "tag": "<tag>",
    "timestamp": 1726012800,
    "manifest_digest": "sha256:...",
    "reason": "Security vulnerability CVE-...",
    "signature": "<hex-sig>"
  }
  ```
- Broadcast tombstones on the publisher's gossip topic.
- When nodes receive a verified tombstone:
  - If the timestamp is newer than the local release, mark the release as revoked.
  - Exclude revoked releases from serving, or quarantine them based on policy.
  - Optionally purge referenced blobs if no other release uses them.
