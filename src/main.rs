use axum::{
    extract::State,
    routing::{get, post},
    Router,
    response::{IntoResponse, Json},
    extract::ws::{WebSocket, WebSocketUpgrade, Message},
    body::Body,
};
use futures::{stream::StreamExt, SinkExt};
use std::{collections::HashMap, sync::Arc, net::SocketAddr, time::SystemTime};
use tokio::sync::{RwLock, mpsc::{self, UnboundedSender, UnboundedReceiver}, broadcast};
use tracing::{info, warn, error};
use uuid::Uuid;
use serde::{Serialize, Deserialize};
use serde_json;
use axum::response::Response;
use hyper::StatusCode;

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

// Connection details
struct ConnectionDetails {
    connected_at: u64,
    tunnel_id: Option<String>,
    sender: UnboundedSender<Message>,
    response_handler: Option<mpsc::Sender<serde_json::Value>>,
}

// Shared state between all connections
struct AppState {
    connections: RwLock<HashMap<String, ConnectionDetails>>,
}

// Validate tunnel ID format
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

// Sequence 1: Gateway Startup and Initialisation
// ----------------------------------------------
// 1.1. Initialise logging and shutdown channel.
// 1.2. Create shared state (AppState) to track active agent connections.
// 1.3. Build HTTP routes:
//      - /health for health check,
//      - /ws for upgrading to WebSocket (agent connections),
//      - /connections to list active connections,
//      - /forward and catch‑all GET for request forwarding.
// 1.4. Bind to a TCP listener and serve with graceful shutdown.
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
        .route("/health", get(handle_health_check))
        .route("/ws", get(handle_websocket))
        .route("/connections", get(handle_list_connections))
        .route("/forward", post(handle_forward_request))
        .route("/*path", get(handle_direct_request))
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

// Handle health check
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

// Handle listing active connections
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

// Sequence 2: WebSocket Connection Upgrade
// -----------------------------------------
// 2.1. Accept an HTTP connection on /ws and upgrade it to a WebSocket.
// 2.2. Hand over the established socket to handle_socket for the full WebSocket lifecycle.
async fn handle_websocket(
    State(state): State<Arc<AppState>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

// Sequence 3: WebSocket Communication Lifecycle (Agent Connection)
// -----------------------------------------------------------------
// 3.1. Generate a unique connection ID and record the timestamp.
// 3.2. Insert the new connection into shared state with initial details.
// 3.3. Send the connection ID to the agent to initiate the handshake.
// 3.4. Split the WebSocket into two parallel tasks:
//      - Sender Task: Listens for messages queued for the agent (or pong responses).
//      - Receiver Task: Processes incoming messages (handshake, responses, ping/pong).
// 3.5. On connection closure or error, remove the connection from state.
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
        response_handler: None,
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
                                // Parse the response payload
                                if let Ok(response) = serde_json::from_str::<serde_json::Value>(&msg.payload) {
                                    // Get the response handler and send the response
                                    let mut connections = state.connections.write().await;
                                    if let Some(details) = connections.get_mut(&connection_id) {
                                        if let Some(handler) = details.response_handler.take() {
                                            let _ = handler.send(response).await;
                                        }
                                    }
                                }
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

// Sequence 4: Forward HTTP Request via Agent (POST /forward)
// -----------------------------------------------------------
// 4.1. Receive a POST HTTP request to forward.
// 4.2. Create a one-shot response channel to receive the agent’s reply.
// 4.3. Select an available agent that has completed the handshake (has a valid tunnel_id).
// 4.4. Set the agent connection's response_handler to the response channel.
// 4.5. Construct and send the forward message (containing method, path, body, headers) over WebSocket.
// 4.6. Wait for the agent’s response with a timeout and return it to the HTTP client.
async fn handle_forward_request(
    State(state): State<Arc<AppState>>,
    axum::extract::Json(body): axum::extract::Json<serde_json::Value>,
) -> Json<ApiResponse<serde_json::Value>> {
    // Create channel for response
    let (response_tx, mut response_rx) = mpsc::channel(1);
    
    // Find an agent and set up response handler atomically
    let send_result = {
        let mut connections = state.connections.write().await;
        
        if let Some((connection_id, details)) = connections.iter_mut().find(|(_, details)| details.tunnel_id.is_some()) {
            // Create the forward message
            let forward_msg = WebSocketMessage {
                message_type: "request".to_string(),
                payload: serde_json::to_string(&ForwardedRequest {
                    method: "POST".to_string(),
                    path: "/".to_string(),
                    body: body.to_string(), // Use direct JSON string representation
                    headers: vec![("content-type".to_string(), "application/json".to_string())],
                }).unwrap(),
            };

            // Set response handler
            details.response_handler = Some(response_tx);
            
            // Send message while still holding the lock
            info!("Forwarding request to agent: {}", connection_id);
            details.sender.send(Message::Text(serde_json::to_string(&forward_msg).unwrap()))
        } else {
            info!("No agents available for forwarding");
            return Json(ApiResponse {
                status: "error".to_string(),
                message: "No agents available".to_string(),
                data: None,
            });
        }
    };

    // Handle send result
    match send_result {
        Ok(_) => {
            // Wait for response with timeout
            match tokio::time::timeout(std::time::Duration::from_secs(5), response_rx.recv()).await {
                Ok(Some(response)) => {
                    info!("Received and forwarding agent response to client");
                    // The response here is already parsed by the WebSocket handler
                    Json(ApiResponse {
                        status: "success".to_string(),
                        message: "Request processed by agent".to_string(),
                        data: Some(response),
                    })
                }
                Ok(None) => {
                    error!("Response channel closed without response");
                    Json(ApiResponse {
                        status: "error".to_string(),
                        message: "Agent connection lost".to_string(),
                        data: None,
                    })
                }
                Err(_) => {
                    error!("Timeout waiting for agent response");
                    Json(ApiResponse {
                        status: "error".to_string(),
                        message: "Timeout waiting for agent response".to_string(),
                        data: None,
                    })
                }
            }
        }
        Err(e) => {
            error!("Failed to send request to agent: {}", e);
            Json(ApiResponse {
                status: "error".to_string(),
                message: format!("Failed to send request to agent: {}", e),
                data: None,
            })
        }
    }
}

// Sequence 5: Direct GET Request Handling via Agent (Catch-All GET)
// ---------------------------------------------------------------
// 5.1. Capture any GET request not matching other routes.
// 5.2. Set up a response channel similar to the POST forward process.
// 5.3. Identify an available agent to handle the request.
// 5.4. Wrap and forward the GET request with appropriate headers and the requested path.
// 5.5. Wait (with an extended timeout) for the agent response.
// 5.6. Build and return the final HTTP response to the client.
async fn handle_direct_request(
    State(state): State<Arc<AppState>>,
    uri: axum::http::Uri,
) -> Response<Body> {
    let path = uri.path().to_string();
    info!("Received direct GET request for path: {}", path);

    // Create channel for response
    let (response_tx, mut response_rx) = mpsc::channel(1);
    
    // Find an agent and set up response handler
    let send_result = {
        let mut connections = state.connections.write().await;
        
        if let Some((connection_id, details)) = connections.iter_mut().find(|(_, details)| details.tunnel_id.is_some()) {
            // Create the forward message
            let forward_msg = WebSocketMessage {
                message_type: "request".to_string(),
                payload: serde_json::to_string(&ForwardedRequest {
                    method: "GET".to_string(),
                    path: path.clone(),
                    body: "".to_string(),
                    headers: vec![
                        ("accept".to_string(), "text/html,application/xhtml+xml".to_string()),
                        ("user-agent".to_string(), "Mozilla/5.0".to_string()),
                    ],
                }).unwrap(),
            };

            // Set response handler
            details.response_handler = Some(response_tx);
            
            // Send message while still holding the lock
            info!("Forwarding request to agent: {}", connection_id);
            details.sender.send(Message::Text(serde_json::to_string(&forward_msg).unwrap()))
        } else {
            info!("No agents available for forwarding");
            return Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .body(Body::from("No agents available"))
                .unwrap();
        }
    };

    // Handle send result
    match send_result {
        Ok(_) => {
            // Wait for response with timeout (increased to 30 seconds)
            match tokio::time::timeout(std::time::Duration::from_secs(30), response_rx.recv()).await {
                Ok(Some(response)) => {
                    info!("Received response from agent");
                    if let Some(data) = response.get("data") {
                        if let Some(body) = data.get("body") {
                            if let Some(body_str) = body.as_str() {
                                return Response::builder()
                                    .status(StatusCode::OK)
                                    .header("Content-Type", "text/html")
                                    .header("Connection", "close") // Add this to prevent keep-alive
                                    .body(Body::from(body_str.to_string()))
                                    .unwrap();
                            }
                        }
                        // If we got a response but couldn't extract the body
                        error!("Invalid response format from agent: {:?}", data);
                    }
                    Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .header("Connection", "close")
                        .body(Body::from("Invalid response format"))
                        .unwrap()
                }
                Ok(None) => {
                    error!("Agent connection lost while waiting for response");
                    Response::builder()
                        .status(StatusCode::BAD_GATEWAY)
                        .header("Connection", "close")
                        .body(Body::from("Agent connection lost"))
                        .unwrap()
                }
                Err(_) => {
                    error!("Request timed out after 30 seconds");
                    Response::builder()
                        .status(StatusCode::GATEWAY_TIMEOUT)
                        .header("Connection", "close")
                        .body(Body::from("Request timed out after 30 seconds"))
                        .unwrap()
                }
            }
        }
        Err(e) => {
            error!("Failed to send request to agent: {}", e);
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("Connection", "close")
                .body(Body::from(format!("Failed to send request: {}", e)))
                .unwrap()
        }
    }
} 