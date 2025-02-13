use axum::{
    extract::State,
    routing::{get, post},
    Router,
    response::{IntoResponse, Json},
    extract::ws::{WebSocket, WebSocketUpgrade, Message},
};
use futures::{stream::StreamExt, SinkExt};
use std::{collections::HashMap, sync::Arc, net::SocketAddr, time::SystemTime};
use tokio::sync::{RwLock, mpsc::{self, UnboundedSender, UnboundedReceiver}, broadcast};
use tracing::{info, warn, error};
use uuid::Uuid;
use serde::{Serialize, Deserialize};
use serde_json;

#[derive(Serialize)]
struct ApiResponse<T> {
    status: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<T>,
}

#[derive(Serialize)]
struct HealthResponse {
    version: &'static str,
    status: &'static str,
}

#[derive(Serialize)]
struct ConnectionInfo {
    connection_id: String,
    connected_at: u64,
    tunnel_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WebSocketMessage {
    message_type: String,
    payload: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ForwardedRequest {
    method: String,
    path: String,
    body: String,
    headers: Vec<(String, String)>,
}

#[derive(Debug, Deserialize)]
struct AgentHandshake {
    tunnel_id: String,
    agent_version: String,
}

// Connection information
struct ConnectionDetails {
    connected_at: u64,
    tunnel_id: Option<String>,
    sender: UnboundedSender<Message>,
}

// Shared state between all connections
struct AppState {
    connections: RwLock<HashMap<String, ConnectionDetails>>,
}

fn validate_tunnel_id(tunnel_id: &str) -> bool {
    // Format: agent_{uuid}_{purpose}
    let parts: Vec<&str> = tunnel_id.split('_').collect();
    if parts.len() != 3 {
        return false;
    }

    if parts[0] != "agent" {
        return false;
    }

    // Validate UUID part
    if let Err(_) = uuid::Uuid::parse_str(parts[1]) {
        return false;
    }

    // Validate purpose (only allow alphanumeric and underscore)
    parts[2].chars().all(|c| c.is_alphanumeric() || c == '_')
}

#[tokio::main]
async fn main() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // Create shutdown channel
    let (shutdown_tx, _) = broadcast::channel(1);
    let shutdown_tx_clone = shutdown_tx.clone();

    // Create shared state
    let state = Arc::new(AppState {
        connections: RwLock::new(HashMap::new()),
    });

    // Build our application with routes
    let app = Router::new()
        .route("/", get(handle_health_check))
        .route("/ws", get(handle_websocket))
        .route("/connections", get(handle_list_connections))
        .route("/forward", post(handle_forward_request))
        .with_state(Arc::clone(&state));

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    info!("Starting gateway server on {}", addr);
    info!("Available endpoints:");
    info!("  GET    / - Health check");
    info!("  GET    /ws - WebSocket endpoint");
    info!("  GET    /connections - List active connections");
    info!("  POST   /forward - Forward HTTP request");

    // Handle shutdown signal
    tokio::spawn(async move {
        if let Ok(()) = tokio::signal::ctrl_c().await {
            info!("Shutdown signal received...");
            let connections = state.connections.read().await;
            info!("Notifying {} connected agents...", connections.len());
            
            // Send close message to all connected agents
            for (id, details) in connections.iter() {
                if let Err(e) = details.sender.send(Message::Close(None)) {
                    error!("Failed to send close message to agent {}: {}", id, e);
                } else {
                    info!("Close message sent to agent {}", id);
                }
            }
            
            // Give agents a moment to process close messages
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            info!("Initiating shutdown...");
            let _ = shutdown_tx_clone.send(());
        }
    });

    // Run the server with shutdown signal
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown_tx.subscribe().recv().await;
            info!("Gateway shutdown complete");
        })
        .await
        .unwrap();
}

async fn handle_health_check() -> Json<ApiResponse<HealthResponse>> {
    Json(ApiResponse {
        status: "success".to_string(),
        message: "Gateway is running".to_string(),
        data: Some(HealthResponse {
            version: env!("CARGO_PKG_VERSION"),
            status: "operational",
        }),
    })
}

async fn handle_list_connections(State(state): State<Arc<AppState>>) -> Json<ApiResponse<Vec<ConnectionInfo>>> {
    let connections = state.connections.read().await;
    let connection_list: Vec<ConnectionInfo> = connections
        .iter()
        .map(|(id, details)| ConnectionInfo {
            connection_id: id.clone(),
            connected_at: details.connected_at,
            tunnel_id: details.tunnel_id.clone(),
        })
        .collect();

    Json(ApiResponse {
        status: "success".to_string(),
        message: format!("Found {} active connections", connection_list.len()),
        data: Some(connection_list),
    })
}

async fn handle_websocket(
    State(state): State<Arc<AppState>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let connection_id = Uuid::new_v4().to_string();
    let connected_at = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let (sender, mut receiver) = mpsc::unbounded_channel();
    
    // Add connection to state
    state.connections.write().await.insert(connection_id.clone(), ConnectionDetails {
        connected_at,
        tunnel_id: None,
        sender,
    });
    
    info!("New WebSocket connection established: {}", connection_id);

    let (mut ws_sender, mut ws_receiver) = socket.split();

    // Send connection ID to the client
    if let Err(e) = ws_sender.send(Message::Text(connection_id.clone())).await {
        error!("Failed to send connection ID to client: {}", e);
        state.connections.write().await.remove(&connection_id);
        return;
    }

    // Create a channel for sending pong responses
    let (pong_tx, mut pong_rx) = mpsc::unbounded_channel();
    let pong_sender = pong_tx.clone();

    // Handle incoming messages from other parts of the application
    let send_task = {
        let connection_id = connection_id.clone();
        let mut ws_sender = ws_sender;
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(message) = receiver.recv() => {
                        if let Err(e) = ws_sender.send(message).await {
                            error!("Failed to send message to WebSocket: {}", e);
                            break;
                        }
                    }
                    Some(pong_data) = pong_rx.recv() => {
                        if let Err(e) = ws_sender.send(Message::Pong(pong_data)).await {
                            error!("Failed to send pong: {}", e);
                            break;
                        }
                    }
                }
            }
            info!("Send task ended for connection: {}", connection_id);
        })
    };

    // Handle incoming WebSocket messages
    let recv_task = {
        let connection_id = connection_id.clone();
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            while let Some(Ok(msg)) = ws_receiver.next().await {
                match msg {
                    Message::Close(_) => {
                        info!("WebSocket connection closed: {}", connection_id);
                        break;
                    }
                    Message::Text(text) => {
                        info!("Received message from {}: {}", connection_id, text);
                        
                        if let Ok(handshake) = serde_json::from_str::<AgentHandshake>(&text) {
                            if !validate_tunnel_id(&handshake.tunnel_id) {
                                warn!("Invalid tunnel ID format from {}: {}", connection_id, handshake.tunnel_id);
                                break;
                            }
                            info!("Valid handshake from {} with tunnel ID: {}", connection_id, handshake.tunnel_id);
                            
                            // Update connection with tunnel ID
                            if let Some(details) = state.connections.write().await.get_mut(&connection_id) {
                                details.tunnel_id = Some(handshake.tunnel_id);
                            }
                        } else if let Ok(msg) = serde_json::from_str::<WebSocketMessage>(&text) {
                            if msg.message_type == "response" {
                                info!("Received response from agent {}: {}", connection_id, msg.payload);
                            }
                        }
                    }
                    Message::Ping(data) => {
                        if let Err(e) = pong_sender.send(data) {
                            error!("Failed to queue pong: {}", e);
                            break;
                        }
                    }
                    Message::Pong(_) => {
                        // Pong received, connection is alive
                    }
                    _ => {}
                }
            }
            info!("Receive task ended for connection: {}", connection_id);
        })
    };

    // Wait for either task to finish
    let _ = tokio::select! {
        res = send_task => {
            info!("Send task finished first");
            res
        }
        res = recv_task => {
            info!("Receive task finished first");
            res
        }
    };

    // Clean up connection
    state.connections.write().await.remove(&connection_id);
    info!("Connection cleaned up: {}", connection_id);
}

async fn handle_forward_request(
    State(state): State<Arc<AppState>>,
    axum::extract::Json(body): axum::extract::Json<serde_json::Value>,
) -> impl IntoResponse {
    let connections = state.connections.read().await;
    
    // Find an agent connection
    let agent = connections.iter().find(|(_, details)| details.tunnel_id.is_some());
    
    match agent {
        Some((connection_id, details)) => {
            let forward_msg = WebSocketMessage {
                message_type: "request".to_string(),
                payload: serde_json::to_string(&ForwardedRequest {
                    method: "POST".to_string(),
                    path: "/".to_string(),
                    body: serde_json::to_string(&body).unwrap(),
                    headers: vec![("content-type".to_string(), "application/json".to_string())],
                }).unwrap(),
            };

            match details.sender.send(Message::Text(serde_json::to_string(&forward_msg).unwrap())) {
                Ok(_) => {
                    info!("Request forwarded to agent: {}", connection_id);
                    Json(ApiResponse {
                        status: "success".to_string(),
                        message: format!("Request forwarded to agent {}", connection_id),
                        data: None::<()>,
                    })
                }
                Err(e) => {
                    error!("Failed to forward request: {}", e);
                    Json(ApiResponse {
                        status: "error".to_string(),
                        message: "Failed to forward request".to_string(),
                        data: None::<()>,
                    })
                }
            }
        }
        None => Json(ApiResponse {
            status: "error".to_string(),
            message: "No agents available".to_string(),
            data: None::<()>,
        }),
    }
} 