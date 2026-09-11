//! Embedded OCI Distribution (v2) registry on localhost, plus the `/_ocid/`
//! control API used by the CLI.
//!
//! The registry is the bridge to podman/docker/crane: pushes become signed
//! releases announced to the swarm; pulls of unknown images are fetched from
//! peers on demand.

use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{anyhow, bail, Context, Result};
use axum::{
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use n0_future::StreamExt;
use ocid_core::{
    api::{
        AddPeerReq, AddPeerResp, AnnounceReq, AnnounceResp, DaemonEvent, ErrorResp, GcReq, OkResp,
        RefReq, ReleaseInfo, RmReq, RmResp, SyncReq, SyncResp,
    },
    identity::PublisherId,
    oci::{self, Digest, ImageRef, Manifest},
    release::{BlobRef, Referrer, Release, ReleasePayload, ReleaseSummary},
};
use serde::Serialize;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
};
use tokio_stream::wrappers::BroadcastStream;
use tokio_util::io::ReaderStream;

use crate::{
    metrics::{self, RequestLabels},
    node::Node,
};

const MAX_MANIFEST: usize = 4 * 1024 * 1024;

#[derive(Clone)]
struct App {
    node: Arc<Node>,
    uploads: Arc<Mutex<HashMap<String, Upload>>>,
}

#[derive(Debug)]
struct Upload {
    path: std::path::PathBuf,
    size: u64,
}

pub async fn serve(node: Arc<Node>, listener: tokio::net::TcpListener) {
    let app = App {
        node,
        uploads: Arc::new(Mutex::new(HashMap::new())),
    };
    let router = Router::new()
        .route("/v2", get(v2_root))
        .route("/v2/", get(v2_root))
        .route("/v2/{*path}", axum::routing::any(v2_dispatch))
        .route("/_ocid/status", get(ctl_status))
        .route("/_ocid/peers", get(ctl_peers).post(ctl_add_peer))
        .route("/_ocid/releases", get(ctl_releases))
        .route("/_ocid/pull", post(ctl_pull))
        .route("/_ocid/announce", post(ctl_announce))
        .route("/_ocid/sync", post(ctl_sync))
        .route("/_ocid/policy/reload", post(ctl_reload))
        .route("/_ocid/gc", post(ctl_gc))
        .route("/_ocid/rm", post(ctl_rm))
        .route("/_ocid/events", get(ctl_events))
        .route("/metrics", get(metrics_get))
        .layer(DefaultBodyLimit::disable())
        .layer(middleware::from_fn_with_state(app.clone(), count_requests))
        .with_state(app);
    if let Err(e) = axum::serve(listener, router).await {
        tracing::error!("registry server failed: {e}");
    }
}

/// Count every request by method / route family / status.
async fn count_requests(State(app): State<App>, req: Request, next: Next) -> Response {
    let method = req.method().to_string();
    let route = metrics::route_family(req.uri().path());
    // Only pay for the path copy when someone is listening on /_ocid/events,
    // and never echo the event stream request itself.
    let path = (app.node.events.receiver_count() > 0
        && !req.uri().path().starts_with("/_ocid/events"))
    .then(|| req.uri().path().to_string());
    let resp = next.run(req).await;
    let status = resp.status().as_u16();
    app.node
        .metrics
        .http_requests
        .get_or_create(&RequestLabels {
            method: method.clone(),
            route,
            status,
        })
        .inc();
    if let Some(path) = path {
        app.node.emit(DaemonEvent::HttpRequest {
            method,
            path,
            status,
        });
    }
    resp
}

/// `GET /metrics` — OpenMetrics text exposition.
async fn metrics_get(State(app): State<App>) -> Response {
    let Some(exporter) = &app.node.exporter else {
        return (StatusCode::NOT_FOUND, "metrics disabled").into_response();
    };
    app.node.refresh_gauges().await;
    match exporter.encode() {
        Ok(body) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, metrics::CONTENT_TYPE)
            .body(Body::from(body))
            .unwrap(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------------
// OCI error responses
// ---------------------------------------------------------------------------

struct OciError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl OciError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }
    fn not_found(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, code, message)
    }
    fn bad(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "UNSUPPORTED", message)
    }
    fn internal(e: anyhow::Error) -> Self {
        tracing::error!("registry internal error: {e:#}");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "UNKNOWN",
            format!("{e:#}"),
        )
    }
}

impl From<anyhow::Error> for OciError {
    fn from(e: anyhow::Error) -> Self {
        Self::internal(e)
    }
}

impl IntoResponse for OciError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({
            "errors": [{ "code": self.code, "message": self.message }]
        });
        let mut resp = (self.status, Json(body)).into_response();
        resp.headers_mut().insert(
            "Docker-Distribution-API-Version",
            HeaderValue::from_static("registry/2.0"),
        );
        resp
    }
}

type OciResult = std::result::Result<Response, OciError>;

fn with_api_version(mut resp: Response) -> Response {
    resp.headers_mut().insert(
        "Docker-Distribution-API-Version",
        HeaderValue::from_static("registry/2.0"),
    );
    resp
}

async fn v2_root() -> Response {
    with_api_version((StatusCode::OK, Json(serde_json::json!({}))).into_response())
}

// ---------------------------------------------------------------------------
// /v2/{*path} dispatcher
// ---------------------------------------------------------------------------

enum Route<'a> {
    Catalog,
    Tags { name: &'a str },
    Manifest { name: &'a str, reference: &'a str },
    UploadStart { name: &'a str },
    Upload { name: &'a str, id: &'a str },
    Blob { name: &'a str, digest: &'a str },
    Referrers { name: &'a str, digest: &'a str },
}

fn route(path: &str) -> Option<Route<'_>> {
    let path = path.trim_end_matches('/');
    if path == "_catalog" {
        return Some(Route::Catalog);
    }
    if let Some(name) = path.strip_suffix("/tags/list") {
        return Some(Route::Tags { name });
    }
    if let Some(i) = path.rfind("/blobs/uploads") {
        let name = &path[..i];
        let rest = &path[i + "/blobs/uploads".len()..];
        return Some(match rest.strip_prefix('/') {
            None | Some("") => Route::UploadStart { name },
            Some(id) => Route::Upload { name, id },
        });
    }
    if let Some(i) = path.rfind("/manifests/") {
        return Some(Route::Manifest {
            name: &path[..i],
            reference: &path[i + "/manifests/".len()..],
        });
    }
    if let Some(i) = path.rfind("/referrers/") {
        return Some(Route::Referrers {
            name: &path[..i],
            digest: &path[i + "/referrers/".len()..],
        });
    }
    if let Some(i) = path.rfind("/blobs/") {
        return Some(Route::Blob {
            name: &path[..i],
            digest: &path[i + "/blobs/".len()..],
        });
    }
    None
}

async fn v2_dispatch(
    State(app): State<App>,
    method: Method,
    Path(path): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let res = match route(&path) {
        Some(Route::Catalog) if method == Method::GET => catalog(&app).await,
        Some(Route::Tags { name }) if method == Method::GET => tags_list(&app, name).await,
        Some(Route::Manifest { name, reference }) => match method {
            Method::GET | Method::HEAD => {
                manifest_get(&app, name, reference, method == Method::HEAD).await
            }
            Method::PUT => manifest_put(&app, name, reference, &headers, body).await,
            Method::DELETE => manifest_delete(&app, name, reference).await,
            _ => Err(OciError::new(
                StatusCode::METHOD_NOT_ALLOWED,
                "UNSUPPORTED",
                "method",
            )),
        },
        Some(Route::UploadStart { name }) if method == Method::POST => {
            upload_start(&app, name, &query, body).await
        }
        Some(Route::Upload { name, id }) => match method {
            Method::PATCH => upload_patch(&app, name, id, body).await,
            Method::PUT => upload_put(&app, name, id, &query, body).await,
            Method::GET => upload_status(&app, name, id).await,
            Method::DELETE => upload_cancel(&app, id).await,
            _ => Err(OciError::new(
                StatusCode::METHOD_NOT_ALLOWED,
                "UNSUPPORTED",
                "method",
            )),
        },
        Some(Route::Blob { name, digest }) => match method {
            Method::GET | Method::HEAD => {
                blob_get(&app, name, digest, &headers, method == Method::HEAD).await
            }
            // Blobs are shared between images; they are reclaimed by GC once
            // no release references them.
            Method::DELETE => Err(OciError::new(
                StatusCode::METHOD_NOT_ALLOWED,
                "UNSUPPORTED",
                "blobs are garbage-collected; delete manifests instead",
            )),
            _ => Err(OciError::new(
                StatusCode::METHOD_NOT_ALLOWED,
                "UNSUPPORTED",
                "method",
            )),
        },
        Some(Route::Referrers { name, digest }) if method == Method::GET => {
            referrers_get(&app, name, digest, &query).await
        }
        _ => Err(OciError::not_found(
            "NAME_UNKNOWN",
            format!("no route for /v2/{path}"),
        )),
    };
    match res {
        Ok(r) => with_api_version(r),
        Err(e) => e.into_response(),
    }
}

/// Resolve a registry repository path via the hybrid scheme.
async fn resolve(app: &App, name: &str) -> std::result::Result<(PublisherId, String), OciError> {
    let policy = app.node.policy().await;
    oci::resolve_repo(name, &policy, &app.node.id())
        .map_err(|e| OciError::new(StatusCode::BAD_REQUEST, "NAME_INVALID", e.to_string()))
}

// ---------------------------------------------------------------------------
// catalog / tags
// ---------------------------------------------------------------------------

async fn catalog(app: &App) -> OciResult {
    let mut repos: Vec<String> = app
        .node
        .store
        .list_releases()?
        .iter()
        .map(|r| format!("{}/{}", r.publisher(), r.name()))
        .collect();
    repos.dedup();
    Ok(Json(serde_json::json!({ "repositories": repos })).into_response())
}

async fn tags_list(app: &App, name: &str) -> OciResult {
    let (publisher, image) = resolve(app, name).await?;
    let tags = app.node.store.list_tags(&publisher, &image)?;
    if tags.is_empty() {
        return Err(OciError::not_found(
            "NAME_UNKNOWN",
            format!("unknown repository {name}"),
        ));
    }
    Ok(Json(serde_json::json!({ "name": name, "tags": tags })).into_response())
}

// ---------------------------------------------------------------------------
// manifests
// ---------------------------------------------------------------------------

async fn manifest_get(app: &App, name: &str, reference: &str, head: bool) -> OciResult {
    let (publisher, image) = resolve(app, name).await?;

    let blob: BlobRef = if let Ok(digest) = reference.parse::<Digest>() {
        // By digest: any manifest we know.
        match app.node.store.blob_ref(&digest)? {
            Some(b) if app.node.store.has_hash(b.hash).await? => b,
            _ => {
                return Err(OciError::not_found(
                    "MANIFEST_UNKNOWN",
                    format!("manifest {digest} not found"),
                ))
            }
        }
    } else {
        oci::validate_tag(reference)
            .map_err(|e| OciError::new(StatusCode::BAD_REQUEST, "TAG_INVALID", e.to_string()))?;
        let timeout = Duration::from_secs(app.node.config.fetch_timeout_secs);
        let rel = tokio::time::timeout(
            timeout,
            app.node.get_or_fetch(&publisher, &image, reference),
        )
        .await
        .map_err(|_| {
            OciError::not_found(
                "MANIFEST_UNKNOWN",
                format!("timed out fetching {name}:{reference} from peers"),
            )
        })??;
        match rel {
            Some(r) => r.payload.manifest,
            None => {
                return Err(OciError::not_found(
                    "MANIFEST_UNKNOWN",
                    format!("{name}:{reference} not found locally or on any peer"),
                ))
            }
        }
    };

    let media_type = blob
        .media_type
        .clone()
        .unwrap_or_else(|| oci::MT_OCI_MANIFEST.to_string());
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, media_type)
        .header(header::CONTENT_LENGTH, blob.size)
        .header("Docker-Content-Digest", blob.digest.as_str());
    if head {
        return Ok(builder.body(Body::empty()).unwrap());
    }
    let bytes = app.node.store.read_hash(blob.hash).await?;
    app.node
        .metrics
        .http_bytes_served
        .inc_by(bytes.len() as u64);
    builder = builder.header(header::ETAG, format!("\"{}\"", blob.digest));
    Ok(builder.body(Body::from(bytes)).unwrap())
}

async fn manifest_put(
    app: &App,
    name: &str,
    reference: &str,
    headers: &HeaderMap,
    body: Body,
) -> OciResult {
    let (publisher, image) = resolve(app, name).await?;
    if publisher != app.node.id() {
        return Err(OciError::new(
            StatusCode::FORBIDDEN,
            "DENIED",
            format!(
                "cannot push into another publisher's namespace ({}); push to {}/{image} or just {image}",
                publisher.fmt_short(),
                app.node.id()
            ),
        ));
    }
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or(s).trim().to_string());
    let bytes = axum::body::to_bytes(body, MAX_MANIFEST)
        .await
        .map_err(|e| OciError::bad(format!("reading manifest: {e}")))?;
    let manifest = Manifest::parse(&bytes, content_type.as_deref())
        .map_err(|e| OciError::new(StatusCode::BAD_REQUEST, "MANIFEST_INVALID", e.to_string()))?;
    let media_type = content_type.unwrap_or_else(|| manifest.media_type().to_string());

    // Collect referenced blobs transitively; all must be present.
    let mut blobs: Vec<BlobRef> = Vec::new();
    collect_refs(app, &manifest, &mut blobs).await?;

    let digest = Digest::sha256(&bytes);
    if let Ok(d) = reference.parse::<Digest>() {
        if d != digest {
            return Err(OciError::new(
                StatusCode::BAD_REQUEST,
                "DIGEST_INVALID",
                "manifest digest does not match reference",
            ));
        }
    } else {
        oci::validate_tag(reference)
            .map_err(|e| OciError::new(StatusCode::BAD_REQUEST, "TAG_INVALID", e.to_string()))?;
    }

    let manifest_ref = app
        .node
        .store
        .put_blob_bytes(bytes, Some(&digest), Some(media_type.clone()))
        .await?;

    // OCI 1.1 referrers: remember the relation and attach the artifact to our
    // release(s) of the subject so it replicates together with the image.
    let subject = manifest.subject().map(|d| d.digest.clone());
    if let Some(subject) = &subject {
        let referrer = Referrer {
            digest: manifest_ref.digest.clone(),
            media_type: media_type.clone(),
            size: manifest_ref.size,
            artifact_type: manifest.artifact_type(),
            annotations: manifest.annotations().cloned().unwrap_or_default(),
        };
        app.node.store.put_referrer(subject, &referrer)?;
        let mut referrer_blobs = vec![manifest_ref.clone()];
        referrer_blobs.extend(blobs.iter().cloned());
        let attached = app
            .node
            .attach_referrer(subject, referrer, referrer_blobs)
            .await?;
        tracing::info!(
            "referrer {} -> subject {} (attached to {attached} release(s))",
            manifest_ref.digest,
            subject
        );
    }

    if reference.parse::<Digest>().is_err() {
        let release = Release::sign(
            &app.node.identity,
            ReleasePayload {
                version: 0,
                publisher,
                name: image.clone(),
                tag: reference.to_string(),
                manifest: manifest_ref.clone(),
                blobs,
                timestamp: 0,
                referrers: vec![],
            },
        )?;
        app.node.publish(&release).await?;
        tracing::info!("published {}", release.reference());
    }

    let mut resp = Response::builder()
        .status(StatusCode::CREATED)
        .header(
            header::LOCATION,
            format!("/v2/{name}/manifests/{}", manifest_ref.digest),
        )
        .header("Docker-Content-Digest", manifest_ref.digest.as_str());
    if let Some(subject) = subject {
        resp = resp.header("OCI-Subject", subject.as_str());
    }
    Ok(resp.body(Body::empty()).unwrap())
}

/// `GET /v2/<name>/referrers/<digest>[?artifactType=..]` (OCI distribution 1.1).
async fn referrers_get(
    app: &App,
    name: &str,
    digest: &str,
    query: &HashMap<String, String>,
) -> OciResult {
    let _ = resolve(app, name).await?;
    let digest: Digest = digest.parse().map_err(|e: anyhow::Error| {
        OciError::new(StatusCode::BAD_REQUEST, "DIGEST_INVALID", e.to_string())
    })?;
    let filter = query.get("artifactType");
    let manifests: Vec<serde_json::Value> = app
        .node
        .store
        .list_referrers(&digest)?
        .into_iter()
        .filter(|r| filter.is_none_or(|f| r.artifact_type.as_deref() == Some(f.as_str())))
        .map(|r| {
            let mut d = serde_json::json!({
                "mediaType": r.media_type,
                "digest": r.digest,
                "size": r.size,
            });
            if let Some(at) = r.artifact_type {
                d["artifactType"] = serde_json::Value::String(at);
            }
            if !r.annotations.is_empty() {
                d["annotations"] = serde_json::to_value(r.annotations).unwrap_or_default();
            }
            d
        })
        .collect();
    let body = serde_json::json!({
        "schemaVersion": 2,
        "mediaType": oci::MT_OCI_INDEX,
        "manifests": manifests,
    });
    let mut resp = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, oci::MT_OCI_INDEX);
    if filter.is_some() {
        resp = resp.header("OCI-Filters-Applied", "artifactType");
    }
    Ok(resp
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap())
}

/// `DELETE /v2/<name>/manifests/<tag|digest>`: remove the release(s). By
/// digest, every tag of this repository pointing at that manifest goes.
async fn manifest_delete(app: &App, name: &str, reference: &str) -> OciResult {
    let (publisher, image) = resolve(app, name).await?;
    let tags: Vec<String> = if let Ok(digest) = reference.parse::<Digest>() {
        let mut v = Vec::new();
        for t in app.node.store.list_tags(&publisher, &image)? {
            if let Some(r) = app.node.store.get_release(&publisher, &image, &t)? {
                if r.payload.manifest.digest == digest {
                    v.push(t);
                }
            }
        }
        if v.is_empty() {
            return Err(OciError::not_found(
                "MANIFEST_UNKNOWN",
                format!("no tag of {name} points at {digest}"),
            ));
        }
        v
    } else {
        oci::validate_tag(reference)
            .map_err(|e| OciError::new(StatusCode::BAD_REQUEST, "TAG_INVALID", e.to_string()))?;
        if app
            .node
            .store
            .get_release(&publisher, &image, reference)?
            .is_none()
        {
            return Err(OciError::not_found(
                "MANIFEST_UNKNOWN",
                format!("{name}:{reference} not found"),
            ));
        }
        vec![reference.to_string()]
    };
    for t in tags {
        app.node
            .remove_release(&publisher, &image, Some(&t))
            .await?;
    }
    Ok(StatusCode::ACCEPTED.into_response())
}

/// Gather every blob a manifest references (recursing into indexes).
async fn collect_refs(
    app: &App,
    manifest: &Manifest,
    out: &mut Vec<BlobRef>,
) -> std::result::Result<(), OciError> {
    for d in manifest.referenced() {
        let mut r = match app.node.store.blob_ref(&d.digest)? {
            Some(r) if app.node.store.has_hash(r.hash).await? => r,
            _ => {
                return Err(OciError::new(
                    StatusCode::BAD_REQUEST,
                    "MANIFEST_BLOB_UNKNOWN",
                    format!("referenced blob {} has not been uploaded", d.digest),
                ))
            }
        };
        if r.media_type.is_none() {
            r.media_type = Some(d.media_type.clone());
            app.node.store.record_blob(&r).await?;
        }
        if out.iter().any(|b| b.digest == r.digest) {
            continue;
        }
        if oci::is_manifest_media_type(&d.media_type) {
            let bytes = app.node.store.read_hash(r.hash).await?;
            let child = Manifest::parse(&bytes, Some(&d.media_type)).map_err(|e| {
                OciError::new(StatusCode::BAD_REQUEST, "MANIFEST_INVALID", e.to_string())
            })?;
            out.push(r);
            Box::pin(collect_refs(app, &child, out)).await?;
        } else {
            out.push(r);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// blobs
// ---------------------------------------------------------------------------

async fn blob_get(
    app: &App,
    name: &str,
    digest: &str,
    headers: &HeaderMap,
    head: bool,
) -> OciResult {
    let _ = resolve(app, name).await?;
    let digest: Digest = digest.parse().map_err(|e: anyhow::Error| {
        OciError::new(StatusCode::BAD_REQUEST, "DIGEST_INVALID", e.to_string())
    })?;
    let blob = match app.node.store.blob_ref(&digest)? {
        Some(b) if app.node.store.has_hash(b.hash).await? => b,
        _ => {
            return Err(OciError::not_found(
                "BLOB_UNKNOWN",
                format!("blob {digest} not found"),
            ))
        }
    };

    // Optional single byte range (RFC 9110): `bytes=start-end`, `bytes=start-`, `bytes=-suffix`.
    let range = match headers.get(header::RANGE).and_then(|v| v.to_str().ok()) {
        Some(spec) => match parse_range(spec, blob.size) {
            Some(r) => Some(r),
            None => {
                return Ok(Response::builder()
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header(header::CONTENT_RANGE, format!("bytes */{}", blob.size))
                    .body(Body::empty())
                    .unwrap())
            }
        },
        None => None,
    };

    let (status, start, len) = match range {
        Some((start, end)) => (StatusCode::PARTIAL_CONTENT, start, end - start + 1),
        None => (StatusCode::OK, 0, blob.size),
    };
    let mut builder = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, len)
        .header(header::ACCEPT_RANGES, "bytes")
        .header("Docker-Content-Digest", blob.digest.as_str());
    if let Some((s, e)) = range {
        builder = builder.header(
            header::CONTENT_RANGE,
            format!("bytes {s}-{e}/{}", blob.size),
        );
    }
    if head {
        return Ok(builder.body(Body::empty()).unwrap());
    }
    app.node.metrics.http_bytes_served.inc_by(len);
    let mut reader = app.node.store.blob_reader(blob.hash);
    if start > 0 {
        use tokio::io::AsyncSeekExt;
        reader
            .seek(std::io::SeekFrom::Start(start))
            .await
            .map_err(anyhow::Error::from)?;
    }
    let stream = ReaderStream::with_capacity(reader.take(len), 1 << 16);
    Ok(builder.body(Body::from_stream(stream)).unwrap())
}

/// Parse a single `bytes=` range into an inclusive `(start, end)`.
fn parse_range(spec: &str, size: u64) -> Option<(u64, u64)> {
    let spec = spec.strip_prefix("bytes=")?.trim();
    if spec.contains(',') || size == 0 {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let (start, end) = match (a.trim(), b.trim()) {
        ("", suffix) => {
            let n: u64 = suffix.parse().ok()?;
            if n == 0 {
                return None;
            }
            (size.saturating_sub(n), size - 1)
        }
        (s, "") => (s.parse().ok()?, size - 1),
        (s, e) => (s.parse().ok()?, e.parse::<u64>().ok()?.min(size - 1)),
    };
    (start <= end && start < size).then_some((start, end))
}

// ---------------------------------------------------------------------------
// uploads
// ---------------------------------------------------------------------------

fn upload_location(name: &str, id: &str) -> String {
    format!("/v2/{name}/blobs/uploads/{id}")
}

async fn upload_start(
    app: &App,
    name: &str,
    query: &HashMap<String, String>,
    body: Body,
) -> OciResult {
    let (publisher, _) = resolve(app, name).await?;
    if publisher != app.node.id() {
        return Err(OciError::new(
            StatusCode::FORBIDDEN,
            "DENIED",
            "cannot push into another publisher's namespace",
        ));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let path = app.node.paths.uploads().join(&id);
    tokio::fs::File::create(&path)
        .await
        .with_context(|| format!("creating {}", path.display()))?;
    app.uploads
        .lock()
        .await
        .insert(id.clone(), Upload { path, size: 0 });

    if let Some(d) = query.get("digest") {
        // monolithic upload
        return upload_put(
            app,
            name,
            &id,
            &HashMap::from([("digest".to_string(), d.clone())]),
            body,
        )
        .await;
    }

    Ok(Response::builder()
        .status(StatusCode::ACCEPTED)
        .header(header::LOCATION, upload_location(name, &id))
        .header("Docker-Upload-UUID", &id)
        .header(header::RANGE, "0-0")
        .header(header::CONTENT_LENGTH, 0)
        .body(Body::empty())
        .unwrap())
}

/// Append the request body to the upload file; returns the new size.
async fn append_body(app: &App, id: &str, body: Body) -> std::result::Result<u64, OciError> {
    let path = {
        let uploads = app.uploads.lock().await;
        uploads.get(id).map(|u| u.path.clone()).ok_or_else(|| {
            OciError::not_found("BLOB_UPLOAD_UNKNOWN", format!("upload {id} unknown"))
        })?
    };
    let mut file = tokio::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .await
        .with_context(|| format!("opening {}", path.display()))?;
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk: Bytes = chunk.map_err(|e| OciError::bad(format!("reading body: {e}")))?;
        app.node
            .metrics
            .http_bytes_received
            .inc_by(chunk.len() as u64);
        file.write_all(&chunk).await.map_err(anyhow::Error::from)?;
    }
    file.flush().await.map_err(anyhow::Error::from)?;
    let size = file.metadata().await.map_err(anyhow::Error::from)?.len();
    if let Some(u) = app.uploads.lock().await.get_mut(id) {
        u.size = size;
    }
    Ok(size)
}

async fn upload_patch(app: &App, name: &str, id: &str, body: Body) -> OciResult {
    let size = append_body(app, id, body).await?;
    Ok(Response::builder()
        .status(StatusCode::ACCEPTED)
        .header(header::LOCATION, upload_location(name, id))
        .header("Docker-Upload-UUID", id)
        .header(header::RANGE, format!("0-{}", size.saturating_sub(1)))
        .header(header::CONTENT_LENGTH, 0)
        .body(Body::empty())
        .unwrap())
}

async fn upload_put(
    app: &App,
    name: &str,
    id: &str,
    query: &HashMap<String, String>,
    body: Body,
) -> OciResult {
    let digest: Digest = query
        .get("digest")
        .ok_or_else(|| OciError::new(StatusCode::BAD_REQUEST, "DIGEST_INVALID", "missing digest"))?
        .parse()
        .map_err(|e: anyhow::Error| {
            OciError::new(StatusCode::BAD_REQUEST, "DIGEST_INVALID", e.to_string())
        })?;
    append_body(app, id, body).await?;
    let upload = app.uploads.lock().await.remove(id).ok_or_else(|| {
        OciError::not_found("BLOB_UPLOAD_UNKNOWN", format!("upload {id} unknown"))
    })?;
    let blob = app
        .node
        .store
        .put_blob_file(&upload.path, Some(&digest), None)
        .await
        .map_err(|e| {
            let _ = std::fs::remove_file(&upload.path);
            OciError::new(StatusCode::BAD_REQUEST, "DIGEST_INVALID", e.to_string())
        })?;
    tracing::debug!("blob uploaded {} ({} bytes)", blob.digest, blob.size);
    Ok(Response::builder()
        .status(StatusCode::CREATED)
        .header(
            header::LOCATION,
            format!("/v2/{name}/blobs/{}", blob.digest),
        )
        .header("Docker-Content-Digest", blob.digest.as_str())
        .header(header::CONTENT_LENGTH, 0)
        .body(Body::empty())
        .unwrap())
}

async fn upload_status(app: &App, name: &str, id: &str) -> OciResult {
    let size = app
        .uploads
        .lock()
        .await
        .get(id)
        .map(|u| u.size)
        .ok_or_else(|| {
            OciError::not_found("BLOB_UPLOAD_UNKNOWN", format!("upload {id} unknown"))
        })?;
    Ok(Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header(header::LOCATION, upload_location(name, id))
        .header("Docker-Upload-UUID", id)
        .header(header::RANGE, format!("0-{}", size.saturating_sub(1)))
        .body(Body::empty())
        .unwrap())
}

async fn upload_cancel(app: &App, id: &str) -> OciResult {
    if let Some(u) = app.uploads.lock().await.remove(id) {
        let _ = tokio::fs::remove_file(u.path).await;
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---------------------------------------------------------------------------
// control API (CLI <-> daemon)
// ---------------------------------------------------------------------------

fn ctl_err(status: StatusCode, e: impl std::fmt::Display) -> Response {
    (
        status,
        Json(ErrorResp {
            error: e.to_string(),
        }),
    )
        .into_response()
}

fn ctl_result<T: Serialize>(r: Result<T>) -> Response {
    match r {
        Ok(v) => Json(v).into_response(),
        Err(e) => ctl_err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
    }
}

async fn ctl_status(State(app): State<App>) -> Response {
    ctl_result(app.node.status().await)
}

async fn ctl_peers(State(app): State<App>) -> Response {
    Json(app.node.peer_infos().await).into_response()
}

async fn ctl_add_peer(State(app): State<App>, Json(req): Json<AddPeerReq>) -> Response {
    ctl_result(
        app.node
            .add_peer_ticket(&req.ticket)
            .await
            .map(|id| AddPeerResp { id }),
    )
}

async fn ctl_releases(State(app): State<App>) -> Response {
    let me = app.node.id();
    let res: Result<Vec<ReleaseInfo>> = async {
        let mut out = Vec::new();
        for r in app.node.store.list_releases()? {
            out.push(ReleaseInfo {
                summary: ReleaseSummary::from(&r),
                complete: app.node.store.is_complete(&r).await?,
                size: r.total_size(),
                blobs: r.all_blobs().count(),
                mine: r.publisher() == &me,
            });
        }
        Ok(out)
    }
    .await;
    ctl_result(res)
}

async fn ctl_pull(State(app): State<App>, Json(req): Json<RefReq>) -> Response {
    let policy = app.node.policy().await;
    let r = match ImageRef::parse(&req.reference, &policy, &app.node.id()) {
        Ok(r) => r,
        Err(e) => return ctl_err(StatusCode::BAD_REQUEST, e),
    };
    let tag = r.tag_or_latest();
    match app.node.get_or_fetch(&r.publisher, &r.name, tag).await {
        Ok(Some(rel)) => Json(ReleaseSummary::from(&rel)).into_response(),
        Ok(None) => ctl_err(
            StatusCode::NOT_FOUND,
            format!(
                "{}/{}:{tag} not found locally or on any peer",
                r.publisher, r.name
            ),
        ),
        Err(e) => ctl_err(StatusCode::BAD_GATEWAY, format!("{e:#}")),
    }
}

async fn ctl_announce(State(app): State<App>, Json(req): Json<AnnounceReq>) -> Response {
    let res: Result<AnnounceResp> = async {
        match req.reference {
            None => Ok(AnnounceResp {
                announced: app.node.announce_all_local().await?,
            }),
            Some(s) => {
                let policy = app.node.policy().await;
                let r = ImageRef::parse(&s, &policy, &app.node.id())?;
                let rel = app
                    .node
                    .store
                    .get_release(&r.publisher, &r.name, r.tag_or_latest())?
                    .ok_or_else(|| anyhow!("{r} is not a local release"))?;
                app.node.announce(&rel).await?;
                Ok(AnnounceResp { announced: 1 })
            }
        }
    }
    .await;
    ctl_result(res)
}

async fn ctl_sync(State(app): State<App>, Json(req): Json<SyncReq>) -> Response {
    let res: Result<SyncResp> = async {
        let peers: Vec<PublisherId> = match req.peer {
            Some(p) => vec![ocid_core::identity::parse_publisher(&p)?],
            None => app
                .node
                .peer_infos()
                .await
                .into_iter()
                .map(|p| p.id)
                .collect(),
        };
        let mut synced = 0;
        let mut failed = Vec::new();
        for p in peers {
            match app.node.sync_with(p).await {
                Ok(()) => synced += 1,
                Err(e) => failed.push(format!("{}: {e}", p.fmt_short())),
            }
        }
        Ok(SyncResp { synced, failed })
    }
    .await;
    ctl_result(res)
}

async fn ctl_gc(State(app): State<App>, Json(req): Json<GcReq>) -> Response {
    ctl_result(app.node.gc(req).await)
}

async fn ctl_rm(State(app): State<App>, Json(req): Json<RmReq>) -> Response {
    let res: Result<RmResp> = async {
        let policy = app.node.policy().await;
        let r = ImageRef::parse(&req.reference, &policy, &app.node.id())?;
        match (&r.tag, req.all_tags) {
            (Some(_), true) => bail!("--all cannot be combined with a tag"),
            (None, false) => bail!("{r} has no tag; give one or pass --all to remove every tag"),
            _ => {}
        }
        app.node
            .remove_release(&r.publisher, &r.name, r.tag.as_deref())
            .await
    }
    .await;
    ctl_result(res)
}

async fn ctl_reload(State(app): State<App>) -> Response {
    ctl_result(app.node.reload_policy().await.map(|_| OkResp { ok: true }))
}

/// `GET /_ocid/events` — live daemon events as Server-Sent Events, one JSON
/// `DaemonEvent` per `data:` line. Consumers that lag behind the broadcast
/// buffer simply miss events; there is no backpressure onto the daemon.
async fn ctl_events(
    State(app): State<App>,
) -> Sse<impl n0_future::Stream<Item = Result<SseEvent, std::convert::Infallible>>> {
    let stream = BroadcastStream::new(app.node.events.subscribe()).filter_map(|item| {
        item.ok()
            .and_then(|ev| serde_json::to_string(&ev).ok())
            .map(|json| Ok(SseEvent::default().data(json)))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}
