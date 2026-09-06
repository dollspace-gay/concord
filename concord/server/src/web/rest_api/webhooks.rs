use super::{
    AppState, Arc, AuthUser, Deserialize, IntoResponse, Json, Path, Query, State, StatusCode,
};

#[derive(Deserialize)]
pub struct WebhookExecuteRequest {
    pub content: String,
    pub idempotency_key: String,
    pub username: Option<String>,
    pub avatar_url: Option<String>,
}

/// POST /api/webhooks/{id}/{token} — execute an incoming webhook (public, no session auth).
pub async fn execute_webhook(
    State(state): State<Arc<AppState>>,
    Path((webhook_id, token)): Path<(String, String)>,
    Json(body): Json<WebhookExecuteRequest>,
) -> impl IntoResponse {
    match state
        .engine
        .execute_incoming_webhook(
            &webhook_id,
            &token,
            &body.content,
            &body.idempotency_key,
            body.username.as_deref(),
            body.avatar_url.as_deref(),
        )
        .await
    {
        Ok(receipt) => (StatusCode::CREATED, Json(receipt)).into_response(),
        Err(e) => {
            if e.contains("Invalid webhook token") {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({"error": e})),
                )
                    .into_response()
            } else {
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": e})),
                )
                    .into_response()
            }
        }
    }
}

#[derive(Deserialize)]
pub struct WebhookDeliveryListParams {
    pub limit: Option<i64>,
}

pub async fn list_webhook_deliveries(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(webhook_id): Path<String>,
    Query(params): Query<WebhookDeliveryListParams>,
) -> impl IntoResponse {
    match state
        .engine
        .list_webhook_deliveries(&auth.actor, &webhook_id, params.limit.unwrap_or(50))
        .await
    {
        Ok(deliveries) => Json(serde_json::json!({"deliveries": deliveries})).into_response(),
        Err(error) => integration_http_error(&error, "Webhook unavailable"),
    }
}

pub async fn test_outgoing_webhook(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(webhook_id): Path<String>,
) -> impl IntoResponse {
    match state
        .engine
        .enqueue_webhook_test(&auth.actor, &webhook_id)
        .await
    {
        Ok(delivery_id) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({"delivery_id":delivery_id,"status":"pending"})),
        )
            .into_response(),
        Err(error) => integration_http_error(&error, "Webhook unavailable"),
    }
}

pub async fn retry_webhook_delivery(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(delivery_id): Path<String>,
) -> impl IntoResponse {
    match state
        .engine
        .retry_webhook_delivery(&auth.actor, &delivery_id)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => integration_http_error(&error, "Delivery unavailable"),
    }
}

pub(super) fn integration_http_error(
    error: &str,
    hidden_message: &'static str,
) -> axum::response::Response {
    if error.starts_with("DEPENDENCY_UNAVAILABLE:") {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Integration dependency unavailable",
        )
            .into_response()
    } else if error.starts_with("AUTHENTICATION_REQUIRED:") {
        (StatusCode::UNAUTHORIZED, "Authentication required").into_response()
    } else if error.starts_with("FORBIDDEN:") {
        (StatusCode::FORBIDDEN, hidden_message).into_response()
    } else if error.starts_with("INVALID_INPUT:") {
        (StatusCode::BAD_REQUEST, "Invalid integration request").into_response()
    } else {
        (StatusCode::NOT_FOUND, hidden_message).into_response()
    }
}
