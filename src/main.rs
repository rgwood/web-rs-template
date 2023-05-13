use std::{eprintln, net::SocketAddr, thread, time::Duration};

use anyhow::Result;
use axum::{
    body::{boxed, BoxBody, Full},
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    http::{header, Response, StatusCode, Uri},
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use clap::Parser;
use rust_embed::RustEmbed;
use tokio::sync::broadcast;

#[derive(clap::Parser, Clone)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// The port for the web server
    #[arg(short, long, default_value = "3000")]
    port: u16,
}

fn main() -> Result<()> {
    initialize_environment();

    let cli = Cli::parse();

    let (tx, _rx) = broadcast::channel::<String>(10000); // capacity arbitrarily chosen
    let state = AppState { tx: tx.clone() };

    // start web server and attempt to open it in browser
    let rt = tokio::runtime::Runtime::new()?;
    let _webserver = rt.spawn(async move {
        let app = Router::new()
            .route("/", get(root))
            .route("/events", get(events_websocket))
            .route("/*file", get(static_handler))
            .with_state(state);

        let url = format!("http://localhost:{}", cli.port);
        let _ = open::that(url);

        let addr = SocketAddr::from(([127, 0, 0, 1], cli.port));
        axum::Server::bind(&addr)
            .serve(app.into_make_service())
            .await
            .expect(
                "Failed to bind to socket. Maybe another service is already using the same port",
            );
    });

    loop {
        tx.send("foo".to_string())?;
        thread::sleep(Duration::from_millis(1000));
    }
}

fn initialize_environment() {
    std::env::set_var("RUST_BACKTRACE", "1");
}

#[derive(Clone)]
struct AppState {
    // TODO: replace String with whatever type you want to send to the UI
    tx: broadcast::Sender<String>,
}

#[axum::debug_handler]
async fn root() -> impl IntoResponse {
    Html(include_str!("../embed/index.html"))
}

#[axum::debug_handler]
async fn static_handler(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/').to_string();
    StaticFile(path)
}

#[derive(RustEmbed)]
#[folder = "embed/"]
struct Asset;

#[axum::debug_handler]
async fn events_websocket(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(|ws: WebSocket| async { stream_events(state, ws).await })
}

async fn stream_events(app_state: AppState, mut ws: WebSocket) {
    let mut rx = app_state.tx.subscribe();

    loop {
        let event = rx.recv().await.unwrap();
        // serialization is an example; don't need to do this if you're sending a string
        let serialized = serde_json::to_string(&event).unwrap();

        if let Err(e) = ws.send(Message::Text(serialized)).await {
            eprintln!("failed to send websocket message: {}", e);
            return;
        }
    }
}

pub struct StaticFile<T>(pub T);

impl<T> IntoResponse for StaticFile<T>
where
    T: Into<String>,
{
    fn into_response(self) -> Response<BoxBody> {
        let path = self.0.into();

        match Asset::get(path.as_str()) {
            Some(content) => {
                let body = boxed(Full::from(content.data));
                let mime = mime_guess::from_path(path).first_or_octet_stream();
                Response::builder()
                    .header(header::CONTENT_TYPE, mime.as_ref())
                    .body(body)
                    .unwrap()
            }
            None => Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(boxed(Full::from("404")))
                .unwrap(),
        }
    }
}
