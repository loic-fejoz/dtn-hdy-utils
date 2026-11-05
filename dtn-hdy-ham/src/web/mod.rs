//! Axum web server and REST API for `dtn-hdy-ham`.

pub mod injector;
pub mod routes;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tracing::{info, warn};

use crate::config::HamConfig;
use crate::web::injector::WebBundleInjector;

/// Shared application state for all web handlers.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<HamConfig>,
    pub injector: WebBundleInjector,
    pub service_tracker: crate::service_tracker::ServiceTracker,
    pub cla_manager: crate::cla_manager::ClaManager,
    pub chat_manager: crate::chat_manager::ChatManager,
    pub internal_storage: crate::internal_storage::InternalStorage,
}

/// Builds the Axum router with API endpoints and SPA static asset fallback.
pub fn create_router(state: AppState) -> Router {
    let mut static_dir = state
        .config
        .web
        .as_ref()
        .map(|w| w.static_dir.clone())
        .unwrap_or_else(crate::config::default_web_static_dir);

    // If configured path does not exist directly, check relative to executable directory
    if !static_dir.exists()
        && let Ok(exe) = std::env::current_exe()
    {
        let exe_dir = exe.parent().unwrap_or_else(|| std::path::Path::new("."));
        let candidate = exe_dir.join(&static_dir);
        if candidate.exists() {
            static_dir = candidate;
        } else {
            let alt = exe_dir.join("web").join("dist");
            if alt.exists() {
                static_dir = alt;
            }
        }
    }

    let index_file = static_dir.join("index.html");

    let api_routes = Router::new()
        .route("/config", get(routes::config::get_config))
        .route("/services", get(routes::services::get_services))
        .route("/events", get(routes::services::sse_events))
        .route(
            "/bundles",
            get(routes::bundles::list_bundles)
                .post(routes::bundles::send_bundle)
                .delete(routes::bundles::delete_bundle_handler),
        )
        .route(
            "/bundles/payload",
            get(routes::bundles::get_bundle_payload_handler),
        )
        .route("/clas/connect", post(routes::clas::connect_peer_handler))
        .route(
            "/chat/conversations",
            get(routes::chat::list_conversations).post(routes::chat::create_conversation),
        )
        .route(
            "/chat/conversations/{peer}",
            get(routes::chat::get_conversation_messages).post(routes::chat::send_chat_message),
        )
        .route(
            "/chat/conversations/{peer}/read",
            post(routes::chat::mark_conversation_read),
        )
        .route(
            "/chat/messages",
            get(routes::chat::get_conversation_messages_query)
                .post(routes::chat::send_chat_message_body),
        )
        .route(
            "/chat/read",
            post(routes::chat::mark_conversation_read_body),
        )
        .route(
            "/chat/messages/{id}/sticky",
            post(routes::chat::toggle_chat_sticky),
        )
        .route("/files", get(routes::files::list_files))
        .route(
            "/files/{id}",
            get(routes::files::get_file_details).delete(routes::files::delete_file_bundle),
        )
        .route(
            "/files/{id}/payload",
            get(routes::files::download_file_payload),
        )
        .route(
            "/files/{id}/sticky",
            post(routes::files::toggle_file_sticky),
        )
        .route("/files/{id}/read", post(routes::files::mark_file_read))
        .route("/files/unread", get(routes::files::get_unread_count));

    if static_dir.exists() {
        info!(
            "serving web frontend from directory {}",
            static_dir.display()
        );
    } else {
        warn!(
            "web static-dir '{}' does not exist; SPA assets will not be served",
            static_dir.display()
        );
    }
    let serve_dir = ServeDir::new(&static_dir).fallback(ServeFile::new(index_file));

    Router::new()
        .nest("/api/v1", api_routes)
        .fallback_service(serve_dir)
        .layer(CorsLayer::permissive())
        .with_state(state)
}

/// Spawns the Axum web server in the background.
pub async fn start_web_server(
    state: AppState,
    listen_addr: SocketAddr,
) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    let app = create_router(state);
    let listener = tokio::net::TcpListener::bind(listen_addr).await?;
    info!("Axum web server listening on http://{listen_addr}");

    let handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!("Axum web server error: {e}");
        }
    });

    Ok(handle)
}
