# Provider Routing & Swarm Bloom Filters

## Context
When pulling an unseeded image on demand (`podman pull localhost:5050/<pub>/app:tag`), the local daemon sequentially probes candidate peers (`candidate_peers`) with `GetRelease` requests until one responds with the data. In large swarms, this introduces latency and unnecessary network hops.

## Proposal
- Nodes periodically exchange compressed summaries or bloom filters during `Inventory` sync or connection handshakes.
- A node querying for an unseeded release looks up which connected peers have reported holding matching digests or namespaces.
- Fall back to broadcast / candidate queries only if bloom filter routing fails or yields a false positive.
