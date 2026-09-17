# Resumable Chunked Uploads (OCI v2)

> **Priority 11 · Backlog:** CI reliability polish for multi-GB layers over flaky links.

## Context
The registry implements basic chunked and monolithic layer uploads (`POST /v2/<name>/blobs/uploads/`), but does not fully preserve byte offsets across network interruptions or client retry cycles. If a multi-gigabyte layer upload drops midway, standard tools may have to restart the transfer from byte 0.

## Proposal
- Track upload session progress on disk using temporary state files under `uploads/<session_id>`.
- Properly handle `GET /v2/<name>/blobs/uploads/<session_id>` returning `Range: 0-<current_bytes>`.
- Allow chunked `PATCH` requests resuming from `<current_bytes>` using the `Content-Range: <start>-<end>/<total>` header per OCI distribution specification.
