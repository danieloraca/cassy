use std::{
    env,
    path::Path,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::{Path as AxumPath, State, WebSocketUpgrade, ws::Message as WsMessage},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use tokio::{net::TcpListener, sync::broadcast};
use tower_http::trace::TraceLayer;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

mod push;

use push::{NotificationPayload, PushService, SubscriptionRecord};

#[derive(Clone)]
struct AppState {
    db: Arc<Mutex<Connection>>,
    events: broadcast::Sender<StoredMessage>,
    push: Option<PushService>,
}

#[derive(Debug, Deserialize)]
struct NewMessage {
    conversation_id: String,
    sender_id: String,
    body: String,
}

#[derive(Clone, Debug, Serialize)]
struct StoredMessage {
    message_id: String,
    conversation_id: String,
    sender_id: String,
    sender_name: String,
    body: String,
    sent_at: i64,
}

#[derive(Debug, Deserialize)]
struct ProfileInput {
    profile_id: String,
    display_name: String,
}

#[derive(Debug, Deserialize)]
struct PushSubscriptionInput {
    user_id: String,
    conversation_id: String,
    endpoint: String,
    keys: PushSubscriptionKeys,
}

#[derive(Debug, Deserialize)]
struct PushSubscriptionKeys {
    p256dh: String,
    auth: String,
}

#[derive(Serialize)]
struct VapidPublicKey {
    public_key: String,
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
}

#[derive(Debug)]
struct AppError(String);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": self.0 })),
        )
            .into_response()
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "cassy=info".into()))
        .init();

    let database_path = env::var("DATABASE_PATH").unwrap_or_else(|_| "data/cassy.sqlite3".into());
    if let Some(parent) = Path::new(&database_path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    let connection = Connection::open(&database_path)?;
    initialise_database(&connection)?;
    let push = PushService::from_environment()?;
    info!(enabled = push.is_some(), "web push configured");

    let (events, _) = broadcast::channel(256);
    let state = AppState {
        db: Arc::new(Mutex::new(connection)),
        events,
        push,
    };

    let app = Router::new()
        .route("/", get(index))
        .route("/app.css", get(stylesheet))
        .route("/app.js", get(javascript))
        .route("/manifest.webmanifest", get(manifest))
        .route("/service-worker.js", get(service_worker))
        .route("/icon.svg", get(icon))
        .route("/health", get(health))
        .route("/api/profiles", post(register_profile))
        .route("/api/messages", post(create_message))
        .route("/api/push/vapid-public-key", get(vapid_public_key))
        .route("/api/push/subscriptions", post(register_push_subscription))
        .route(
            "/api/conversations/{conversation_id}/messages",
            get(list_messages),
        )
        .route("/ws/{conversation_id}", get(websocket))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let port = env::var("PORT").unwrap_or_else(|_| "8944".into());
    let address = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&address).await?;
    info!(%address, %database_path, "cassy listening");
    axum::serve(listener, app).await?;

    Ok(())
}

fn initialise_database(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;
        PRAGMA busy_timeout = 5000;

        CREATE TABLE IF NOT EXISTS profiles (
            profile_id TEXT PRIMARY KEY,
            display_name TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS messages (
            message_id TEXT PRIMARY KEY,
            conversation_id TEXT NOT NULL,
            sender_id TEXT NOT NULL,
            body TEXT NOT NULL,
            sent_at INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS messages_by_conversation
        ON messages(conversation_id, sent_at DESC);

        CREATE TABLE IF NOT EXISTS push_subscriptions (
            subscription_id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            endpoint TEXT NOT NULL UNIQUE,
            p256dh TEXT NOT NULL,
            auth TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS conversation_push_subscriptions (
            subscription_id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            conversation_id TEXT NOT NULL,
            endpoint TEXT NOT NULL,
            p256dh TEXT NOT NULL,
            auth TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            UNIQUE(endpoint, conversation_id)
        );

        CREATE INDEX IF NOT EXISTS subscriptions_by_conversation
        ON conversation_push_subscriptions(conversation_id);
        ",
    )
}

async fn health() -> Json<Health> {
    Json(Health { status: "ok" })
}

async fn index() -> Html<&'static str> {
    Html(include_str!("../static/index.html"))
}

async fn stylesheet() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../static/app.css"),
    )
}

async fn javascript() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../static/app.js"),
    )
}

async fn manifest() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/manifest+json")],
        include_str!("../static/manifest.webmanifest"),
    )
}

async fn service_worker() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("../static/service-worker.js"),
    )
}

async fn icon() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/svg+xml")],
        include_str!("../static/icon.svg"),
    )
}

async fn register_profile(
    State(state): State<AppState>,
    Json(input): Json<ProfileInput>,
) -> Result<StatusCode, AppError> {
    let profile_id = input.profile_id.trim().to_string();
    let display_name = input.display_name.trim().to_string();
    if profile_id.is_empty() || display_name.is_empty() || display_name.len() > 60 {
        return Err(AppError("invalid profile".into()));
    }

    let db = Arc::clone(&state.db);
    tokio::task::spawn_blocking(move || {
        let connection = db.lock().map_err(|error| error.to_string())?;
        connection
            .execute(
                "INSERT INTO profiles (profile_id, display_name, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(profile_id) DO UPDATE SET
                    display_name = excluded.display_name,
                    updated_at = excluded.updated_at",
                params![profile_id, display_name, unix_timestamp_millis()],
            )
            .map_err(|error| error.to_string())?;
        Ok::<_, String>(())
    })
    .await
    .map_err(|error| AppError(error.to_string()))?
    .map_err(AppError)?;

    Ok(StatusCode::NO_CONTENT)
}

async fn vapid_public_key(State(state): State<AppState>) -> Result<Json<VapidPublicKey>, AppError> {
    let push = state
        .push
        .as_ref()
        .ok_or_else(|| AppError("web push is not configured".into()))?;
    Ok(Json(VapidPublicKey {
        public_key: push.public_key().to_string(),
    }))
}

async fn register_push_subscription(
    State(state): State<AppState>,
    Json(input): Json<PushSubscriptionInput>,
) -> Result<StatusCode, AppError> {
    if state.push.is_none() {
        return Err(AppError("web push is not configured".into()));
    }

    let db = Arc::clone(&state.db);
    tokio::task::spawn_blocking(move || {
        let connection = db.lock().map_err(|error| error.to_string())?;
        connection
            .execute(
                "INSERT INTO conversation_push_subscriptions
                 (subscription_id, user_id, conversation_id, endpoint, p256dh, auth, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(endpoint, conversation_id) DO UPDATE SET
                    user_id = excluded.user_id,
                    p256dh = excluded.p256dh,
                    auth = excluded.auth,
                    created_at = excluded.created_at",
                params![
                    Uuid::new_v4().to_string(),
                    input.user_id,
                    input.conversation_id,
                    input.endpoint,
                    input.keys.p256dh,
                    input.keys.auth,
                    unix_timestamp_millis()
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok::<_, String>(())
    })
    .await
    .map_err(|error| AppError(error.to_string()))?
    .map_err(AppError)?;

    Ok(StatusCode::NO_CONTENT)
}

async fn create_message(
    State(state): State<AppState>,
    Json(input): Json<NewMessage>,
) -> Result<(StatusCode, Json<StoredMessage>), AppError> {
    if input.conversation_id.trim().is_empty()
        || input.sender_id.trim().is_empty()
        || input.body.trim().is_empty()
        || input.body.len() > 2_000
    {
        return Err(AppError("invalid message".into()));
    }

    let db = Arc::clone(&state.db);
    let message = tokio::task::spawn_blocking(move || {
        let connection = db.lock().map_err(|error| error.to_string())?;
        let sender_name = connection
            .query_row(
                "SELECT display_name FROM profiles WHERE profile_id = ?1",
                [&input.sender_id],
                |row| row.get(0),
            )
            .map_err(|_| "register a profile before sending messages".to_string())?;

        let message = StoredMessage {
            message_id: Uuid::new_v4().to_string(),
            conversation_id: input.conversation_id,
            sender_id: input.sender_id,
            sender_name,
            body: input.body,
            sent_at: unix_timestamp_millis(),
        };

        connection
            .execute(
                "INSERT INTO messages
                 (message_id, conversation_id, sender_id, body, sent_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    &message.message_id,
                    &message.conversation_id,
                    &message.sender_id,
                    &message.body,
                    message.sent_at
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok::<_, String>(message)
    })
    .await
    .map_err(|error| AppError(error.to_string()))?
    .map_err(AppError)?;

    let _ = state.events.send(message.clone());
    if state.push.is_some() {
        let push_state = state.clone();
        let push_message = message.clone();
        tokio::spawn(async move {
            if let Err(error) = send_push_notifications(push_state, push_message).await {
                warn!(%error, "push delivery failed");
            }
        });
    }

    Ok((StatusCode::CREATED, Json(message)))
}

async fn list_messages(
    State(state): State<AppState>,
    AxumPath(conversation_id): AxumPath<String>,
) -> Result<Json<Vec<StoredMessage>>, AppError> {
    let db = Arc::clone(&state.db);
    let messages = tokio::task::spawn_blocking(move || {
        let connection = db.lock().map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare(
                "SELECT m.message_id, m.conversation_id, m.sender_id,
                        COALESCE(p.display_name, m.sender_id), m.body, m.sent_at
                 FROM messages m
                 LEFT JOIN profiles p ON p.profile_id = m.sender_id
                 WHERE m.conversation_id = ?1
                 ORDER BY m.sent_at DESC
                 LIMIT 100",
            )
            .map_err(|error| error.to_string())?;

        let rows = statement
            .query_map([conversation_id], |row| {
                Ok(StoredMessage {
                    message_id: row.get(0)?,
                    conversation_id: row.get(1)?,
                    sender_id: row.get(2)?,
                    sender_name: row.get(3)?,
                    body: row.get(4)?,
                    sent_at: row.get(5)?,
                })
            })
            .map_err(|error| error.to_string())?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| AppError(error.to_string()))?
    .map_err(AppError)?;

    Ok(Json(messages))
}

async fn send_push_notifications(state: AppState, message: StoredMessage) -> Result<(), String> {
    let Some(push) = state.push else {
        return Ok(());
    };

    let db = Arc::clone(&state.db);
    let conversation_id = message.conversation_id.clone();
    let sender_id = message.sender_id.clone();
    let subscriptions = tokio::task::spawn_blocking(move || {
        let connection = db.lock().map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare(
                "SELECT endpoint, p256dh, auth
                 FROM conversation_push_subscriptions
                 WHERE conversation_id = ?1 AND user_id != ?2",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![conversation_id, sender_id], |row| {
                Ok(SubscriptionRecord {
                    endpoint: row.get(0)?,
                    p256dh: row.get(1)?,
                    auth: row.get(2)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())??;

    let payload = NotificationPayload {
        title: &message.sender_name,
        body: &message.body,
        conversation_id: &message.conversation_id,
        message_id: &message.message_id,
    };

    for subscription in subscriptions {
        match tokio::time::timeout(
            std::time::Duration::from_secs(10),
            push.send(&subscription, &payload),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                warn!(%error, endpoint = %subscription.endpoint, "push subscription failed");
            }
            Err(_) => {
                warn!(endpoint = %subscription.endpoint, "push subscription timed out");
            }
        }
    }

    Ok(())
}

async fn websocket(
    websocket: WebSocketUpgrade,
    State(state): State<AppState>,
    AxumPath(conversation_id): AxumPath<String>,
) -> impl IntoResponse {
    websocket.on_upgrade(move |socket| async move {
        let mut socket = socket;
        let mut events = state.events.subscribe();

        loop {
            tokio::select! {
                incoming = socket.recv() => {
                    match incoming {
                        Some(Ok(WsMessage::Close(_))) | None => break,
                        Some(Err(error)) => {
                            warn!(%error, "websocket receive failed");
                            break;
                        }
                        _ => {}
                    }
                }
                event = events.recv() => {
                    match event {
                        Ok(message) if message.conversation_id == conversation_id => {
                            let Ok(json) = serde_json::to_string(&message) else { continue };
                            if socket.send(WsMessage::Text(json.into())).await.is_err() {
                                break;
                            }
                        }
                        Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        }
    })
}

fn unix_timestamp_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
