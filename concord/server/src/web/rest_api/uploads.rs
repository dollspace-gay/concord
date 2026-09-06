use super::{
    AppState, Arc, AuthUser, Body, Deserialize, HeaderMap, IntoResponse, Json, Multipart, Path,
    Query, ReaderStream, SeekFrom, Serialize, State, StatusCode, error, header,
    organization_error_response,
};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

#[derive(Serialize)]
pub struct UploadResponse {
    pub id: String,
    pub filename: String,
    pub content_type: String,
    pub file_size: i64,
    pub url: String,
}

/// Check whether a Content-Type is allowed for file uploads.
/// Rejects types that could execute scripts when rendered inline by a browser
/// (e.g., text/html, application/javascript, SVG) and malformed MIME strings.
pub(super) fn is_allowed_upload_content_type(content_type: &str) -> bool {
    // Must look like a MIME type (contains '/') and be reasonably short
    if !content_type.contains('/') || content_type.len() > 255 {
        return false;
    }

    // Blocklist: types that browsers may execute as active content
    const BLOCKED: &[&str] = &[
        "text/html",
        "text/javascript",
        "application/javascript",
        "application/xhtml+xml",
        "image/svg+xml",
        "text/xml",
        "application/xml",
    ];

    // Compare only the base type (strip parameters like "; charset=utf-8")
    let base = content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
        .to_ascii_lowercase();

    !BLOCKED.contains(&base.as_str())
}

/// POST /api/uploads — upload a file (multipart form data).
#[derive(Deserialize)]
pub struct UploadQuery {
    pub purpose: Option<String>,
    pub conversation_id: Option<String>,
    pub server_id: Option<String>,
    pub channel: Option<String>,
}

pub async fn upload_file(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Query(target): Query<UploadQuery>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let _upload_permit = match state.upload_admission.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return (StatusCode::TOO_MANY_REQUESTS, "Upload capacity is busy").into_response();
        }
    };
    let upload_deadline = tokio::time::Instant::now() + state.upload_total_timeout;
    let purpose = target.purpose.as_deref().unwrap_or("message");
    let mut upload_plan = match state
        .engine
        .authorize_media_upload(
            &auth.actor,
            crate::engine::media_service::UploadTarget {
                purpose,
                conversation_id: target.conversation_id.as_deref(),
                server_id: target.server_id.as_deref(),
                channel: target.channel.as_deref(),
            },
            state.max_file_size,
        )
        .await
    {
        Ok(plan) => Some(plan),
        Err(error) => return organization_error_response(error),
    };

    loop {
        let remaining = upload_deadline.saturating_duration_since(tokio::time::Instant::now());
        let field = match tokio::time::timeout(
            state.upload_idle_timeout.min(remaining),
            multipart.next_field(),
        )
        .await
        {
            Ok(Ok(Some(field))) => field,
            Ok(Ok(None)) => break,
            Ok(Err(error)) => {
                error!(error=%error, "Upload multipart stream failed");
                return (StatusCode::BAD_REQUEST, "Failed to read file data").into_response();
            }
            Err(_) => {
                return (StatusCode::REQUEST_TIMEOUT, "Upload timed out").into_response();
            }
        };
        let mut field = field;
        if field.name() == Some("file") {
            let filename = field
                .file_name()
                .unwrap_or("unnamed")
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or("file")
                .to_string();
            let content_type = field
                .content_type()
                .unwrap_or("application/octet-stream")
                .to_string();
            if !is_allowed_upload_content_type(&content_type) {
                return (StatusCode::BAD_REQUEST, "File type not allowed").into_response();
            }
            if upload_plan.as_ref().is_some_and(|plan| plan.images_only)
                && !matches!(
                    content_type.as_str(),
                    "image/jpeg" | "image/png" | "image/gif" | "image/webp"
                )
            {
                return (
                    StatusCode::BAD_REQUEST,
                    "Managed media must be a safe image type",
                )
                    .into_response();
            }
            let mut upload = match state
                .engine
                .reserve_media_upload(
                    &auth.actor,
                    upload_plan
                        .take()
                        .expect("upload plan is consumed only when returning a response"),
                    crate::engine::media_service::UploadReservation {
                        media_root: &state.media_dir,
                        filename: &filename,
                        content_type: &content_type,
                        per_user_bytes: state.max_media_per_user,
                        total_bytes: state.max_media_total,
                    },
                )
                .await
            {
                Ok(upload) => upload,
                Err(error) => {
                    error!(error=%error, "Failed to start private upload");
                    return (StatusCode::SERVICE_UNAVAILABLE, "Media storage unavailable")
                        .into_response();
                }
            };
            loop {
                let remaining =
                    upload_deadline.saturating_duration_since(tokio::time::Instant::now());
                match tokio::time::timeout(state.upload_idle_timeout.min(remaining), field.chunk())
                    .await
                {
                    Ok(Ok(Some(chunk))) => {
                        if let Err(error) = upload.write_chunk(&chunk).await {
                            let too_large = matches!(error, crate::media::MediaError::TooLarge);
                            upload.abort().await;
                            return (
                                if too_large {
                                    StatusCode::PAYLOAD_TOO_LARGE
                                } else {
                                    StatusCode::SERVICE_UNAVAILABLE
                                },
                                if too_large {
                                    "File too large"
                                } else {
                                    "Media storage unavailable"
                                },
                            )
                                .into_response();
                        }
                    }
                    Ok(Ok(None)) => break,
                    Ok(Err(error)) => {
                        error!(error=%error, "Upload stream failed");
                        upload.abort().await;
                        return (StatusCode::BAD_REQUEST, "Failed to read file data")
                            .into_response();
                    }
                    Err(_) => {
                        upload.abort().await;
                        return (StatusCode::REQUEST_TIMEOUT, "Upload timed out").into_response();
                    }
                }
            }
            return match upload.finish().await {
                Ok(ready) => (
                    StatusCode::CREATED,
                    Json(UploadResponse {
                        url: format!("/api/uploads/{}", ready.id),
                        id: ready.id,
                        filename,
                        content_type,
                        file_size: ready.file_size as i64,
                    }),
                )
                    .into_response(),
                Err(error) => {
                    error!(error=%error, "Failed to finalize private upload");
                    (StatusCode::SERVICE_UNAVAILABLE, "Media storage unavailable").into_response()
                }
            };
        }
    }
    (StatusCode::BAD_REQUEST, "No file field in upload").into_response()
}

/// GET /api/uploads/:id — serve an uploaded file.
pub async fn get_upload(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(attachment_id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let attachment = match state
        .engine
        .authorized_media_download(&auth.actor, &attachment_id)
        .await
    {
        Ok(attachment) => attachment,
        Err(_) => return (StatusCode::NOT_FOUND, "File not found").into_response(),
    };
    let Some((start, end)) = parse_single_range(
        headers.get(header::RANGE).and_then(|v| v.to_str().ok()),
        attachment.file_size as u64,
    ) else {
        return (StatusCode::RANGE_NOT_SATISFIABLE, "Invalid range").into_response();
    };
    let mut file =
        match crate::media::open_rooted_media(&state.media_dir, &attachment.storage_key).await {
            Ok(file) => file,
            Err(error) => {
                error!(error=%error,attachment_id=%attachment_id,"Private media bytes missing");
                return (StatusCode::NOT_FOUND, "File not found").into_response();
            }
        };
    if !state
        .engine
        .media_download_is_still_authorized(&auth.actor, &attachment_id)
        .await
    {
        return (StatusCode::NOT_FOUND, "File not found").into_response();
    }
    if file.seek(SeekFrom::Start(start)).await.is_err() {
        return (StatusCode::SERVICE_UNAVAILABLE, "Media storage unavailable").into_response();
    }
    let safe_filename: String = attachment
        .original_filename
        .chars()
        .filter(|c| !c.is_control() && *c != '"' && *c != ';' && *c != '\\')
        .collect();
    let safe_filename = if safe_filename.is_empty() {
        "download".to_string()
    } else {
        safe_filename
    };
    // Only allow inline rendering for safe media types to prevent stored XSS
    // (e.g., a file with content_type: text/html containing <script> tags)
    let is_safe_inline = safe_inline_content_type(&attachment.content_type);
    let content_disposition = if is_safe_inline {
        format!("inline; filename=\"{safe_filename}\"")
    } else {
        format!("attachment; filename=\"{safe_filename}\"")
    };
    let length = end - start + 1;
    let partial = start != 0 || end + 1 != attachment.file_size as u64;
    let mut response = Body::from_stream(ReaderStream::new(file.take(length))).into_response();
    *response.status_mut() = if partial {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };
    let out = response.headers_mut();
    out.insert(
        header::CONTENT_TYPE,
        attachment
            .content_type
            .parse()
            .unwrap_or_else(|_| header::HeaderValue::from_static("application/octet-stream")),
    );
    out.insert(
        header::CONTENT_DISPOSITION,
        content_disposition
            .parse()
            .unwrap_or_else(|_| header::HeaderValue::from_static("attachment")),
    );
    out.insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("private, no-store"),
    );
    out.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        header::HeaderValue::from_static("nosniff"),
    );
    out.insert(
        header::CONTENT_SECURITY_POLICY,
        header::HeaderValue::from_static("sandbox; default-src 'none'"),
    );
    out.insert(
        header::ACCEPT_RANGES,
        header::HeaderValue::from_static("bytes"),
    );
    out.insert(header::CONTENT_LENGTH, length.into());
    if partial {
        out.insert(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{}", attachment.file_size)
                .parse()
                .unwrap(),
        );
    }
    response
}

pub(super) fn parse_single_range(value: Option<&str>, size: u64) -> Option<(u64, u64)> {
    if size == 0 {
        return None;
    }
    let Some(value) = value else {
        return Some((0, size - 1));
    };
    let spec = value.strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (start, end) = spec.split_once('-')?;
    if start.is_empty() {
        let suffix: u64 = end.parse().ok()?;
        if suffix == 0 {
            return None;
        }
        return Some((size.saturating_sub(suffix), size - 1));
    }
    let start: u64 = start.parse().ok()?;
    if start >= size {
        return None;
    }
    let end = if end.is_empty() {
        size - 1
    } else {
        end.parse::<u64>().ok()?.min(size - 1)
    };
    (start <= end).then_some((start, end))
}

pub(super) fn safe_inline_content_type(content_type: &str) -> bool {
    matches!(
        content_type,
        "image/jpeg"
            | "image/png"
            | "image/gif"
            | "image/webp"
            | "video/mp4"
            | "video/webm"
            | "audio/mpeg"
            | "audio/ogg"
            | "audio/webm"
    )
}

pub async fn delete_upload(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(attachment_id): Path<String>,
) -> impl IntoResponse {
    match state
        .engine
        .delete_unattached_upload_for_actor(&auth.actor, &attachment_id)
        .await
    {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "File not found").into_response(),
        Err(error) => organization_error_response(error),
    }
}
