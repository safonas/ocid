# Containerd / CRI Remote Snapshotter Integration

## Context
Container runtimes (containerd, CRI-O, k3s) pull images by unpacking tar layers through the local HTTP registry shim on loopback (`127.0.0.1:5050`). This causes redundant layer decompression, duplicate disk usage between `iroh-blobs` store and containerd graph drivers, and loopback socket overhead.

## Proposal
- Develop a containerd remote snapshotter or transfer service plugin for `ocid`.
- Allow containerd to directly mount or unpack blobs stored inside `iroh-blobs` content-addressed storage.
- Enables instant container cold-starts on Kubernetes/k3s edge clusters by lazily streaming container rootfs chunks on demand (stargz / overlayfs integration).
