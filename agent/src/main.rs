use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use url::Url;
use tracing::{info, error, warn};
use serde::{Serialize, Deserialize};
use std::{env, time::Duration, sync::Arc};
use tokio::{time::sleep, sync::broadcast};

const MAX_RETRIES: u32 = 10;
const INITIAL_RETRY_DELAY_MS: u64 = 1000;
const MAX_RETRY_DELAY_MS: u64 = 30000;
const PING_INTERVAL_SECS: u64 = 30;
const GATEWAY_UNREACHABLE_EXIT_CODE: i32 = 1;
const SHUTDOWN_EXIT_CODE: i32 = 0;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, required = true)]
    tunnel_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct AgentHandshake {
    tunnel_id: String,
    agent_version: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GatewayMessage {
    message_type: String,
    payload: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ForwardedRequest {
    method: String,
    path: String,
    body: String,
    headers: Vec<(String, String)>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AgentResponse {
    status: String,
    message: String,
    data: Option<serde_json::Value>,
}

#[derive(Debug)]
struct AgentError(String);

impl std::fmt::Display for AgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for AgentError {}

async fn handle_forwarded_request(request: ForwardedRequest) -> Result<String, Box<dyn std::error::Error>> {
    info!("Processing request: {} {}", request.method, request.path);
    
    // Parse the request body
    let body: serde_json::Value = serde_json::from_str(&request.body)
        .map_err(|e| AgentError(format!("Failed to parse request body: {}", e)))?;
    
    // Create response
    let response = AgentResponse {
        status: "success".to_string(),
        message: "Request processed successfully".to_string(),
        data: Some(serde_json::json!({
            "request": {
                "method": request.method,
                "path": request.path,
                "headers": request.headers,
                "body": body,
            },
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "agent_version": env!("CARGO_PKG_VERSION"),
        })),
    };

    // Serialize response
    serde_json::to_string(&response)
        .map_err(|e| AgentError(format!("Failed to serialize response: {}", e)).into())
}

async fn connect_to_gateway(
    tunnel_id: String,
    shutdown_rx: broadcast::Receiver<()>
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway_url = env::var("GATEWAY_URL")
        .unwrap_or_else(|_| "ws://localhost:3000".to_string());
    let ws_url = format!("{}/ws", gateway_url);
    
    let url = Url::parse(&ws_url)
        .map_err(|e| AgentError(format!("Invalid gateway URL: {}", e)))?;
    
    info!("Connecting to gateway at: {}", url);
    
    let (ws_stream, _) = connect_async(url).await
        .map_err(|e| AgentError(format!("Failed to connect: {}", e)))?;
    
    info!("WebSocket connection established");
    let (mut write, mut read) = ws_stream.split();

    // Send handshake
    let handshake = AgentHandshake {
        tunnel_id: tunnel_id.clone(),
        agent_version: env!("CARGO_PKG_VERSION").to_string(),
    };

    let handshake_msg = serde_json::to_string(&handshake)
        .map_err(|e| AgentError(format!("Failed to serialize handshake: {}", e)))?;

    write.send(Message::Text(handshake_msg)).await
        .map_err(|e| AgentError(format!("Failed to send handshake: {}", e)))?;

    info!("Handshake sent, awaiting response");

    let mut ping_interval = tokio::time::interval(Duration::from_secs(PING_INTERVAL_SECS));
    let mut received_connection_id = false;
    let mut shutdown_rx = shutdown_rx;

    loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if !received_connection_id {
                            info!("Received connection ID: {}", text);
                            received_connection_id = true;
                            continue;
                        }

                        if let Ok(msg) = serde_json::from_str::<GatewayMessage>(&text) {
                            match msg.message_type.as_str() {
                                "request" => {
                                    info!("Received request from gateway");
                                    if let Ok(request) = serde_json::from_str::<ForwardedRequest>(&msg.payload) {
                                        match handle_forwarded_request(request).await {
                                            Ok(response) => {
                                                let response_msg = GatewayMessage {
                                                    message_type: "response".to_string(),
                                                    payload: response,
                                                };
                                                if let Err(e) = write.send(Message::Text(serde_json::to_string(&response_msg)?)).await {
                                                    error!("Failed to send response: {}", e);
                                                    return Err(e.into());
                                                }
                                                info!("Response sent to gateway");
                                            }
                                            Err(e) => {
                                                error!("Failed to handle request: {}", e);
                                                let error_msg = GatewayMessage {
                                                    message_type: "error".to_string(),
                                                    payload: e.to_string(),
                                                };
                                                if let Err(e) = write.send(Message::Text(serde_json::to_string(&error_msg)?)).await {
                                                    error!("Failed to send error response: {}", e);
                                                    return Err(e.into());
                                                }
                                            }
                                        }
                                    }
                                }
                                "error" => {
                                    let error_msg = format!("Gateway error: {}", msg.payload);
                                    error!("{}", error_msg);
                                    return Err(AgentError(error_msg).into());
                                }
                                _ => {
                                    info!("Received message: {}", text);
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        info!("Gateway closed connection gracefully");
                        return Ok(());
                    }
                    Some(Ok(Message::Ping(data))) => {
                        if let Err(e) = write.send(Message::Pong(data)).await {
                            error!("Failed to send pong: {}", e);
                            return Err(e.into());
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {
                        // Pong received, connection is alive
                    }
                    Some(Err(e)) => {
                        let error_msg = format!("WebSocket error: {}", e);
                        error!("{}", error_msg);
                        return Err(AgentError(error_msg).into());
                    }
                    None => {
                        warn!("WebSocket stream ended unexpectedly");
                        return Ok(());
                    }
                    _ => {}
                }
            }
            _ = ping_interval.tick() => {
                if let Err(e) = write.send(Message::Ping(vec![])).await {
                    error!("Failed to send ping: {}", e);
                    return Err(AgentError(format!("Failed to send ping: {}", e)).into());
                }
            }
            _ = shutdown_rx.recv() => {
                info!("Shutdown signal received, closing connection...");
                if let Err(e) = write.send(Message::Close(None)).await {
                    warn!("Failed to send close message: {}", e);
                }
                return Ok(());
            }
        }
    }
}

async fn connect_with_retry(tunnel_id: String, shutdown_rx: broadcast::Receiver<()>) -> i32 {
    let mut retry_count = 0;
    let mut delay_ms = INITIAL_RETRY_DELAY_MS;
    let mut shutdown_rx = shutdown_rx;

    loop {
        info!("Connection attempt {} of {}", retry_count + 1, MAX_RETRIES);
        
        match connect_to_gateway(tunnel_id.clone(), shutdown_rx.resubscribe()).await {
            Ok(_) => {
                info!("Connection closed gracefully, attempting to reconnect...");
                retry_count = 0;
                delay_ms = INITIAL_RETRY_DELAY_MS;
            }
            Err(e) => {
                error!("Connection error: {}", e);
                retry_count += 1;
                
                if retry_count >= MAX_RETRIES {
                    error!("Max retries ({}) reached, exiting...", MAX_RETRIES);
                    return GATEWAY_UNREACHABLE_EXIT_CODE;
                }
                
                delay_ms = std::cmp::min(delay_ms * 2, MAX_RETRY_DELAY_MS);
                info!("Retrying in {} ms...", delay_ms);

                // Add shutdown check during retry delay
                tokio::select! {
                    _ = sleep(Duration::from_millis(delay_ms)) => {}
                    _ = shutdown_rx.recv() => {
                        info!("Shutdown signal received during retry delay");
                        return SHUTDOWN_EXIT_CODE;
                    }
                }
            }
        }
    }
}

#[tokio::main]
async fn main() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // Parse command line arguments
    let args = Args::parse();
    let tunnel_id = args.tunnel_id;

    info!("Starting agent with tunnel_id: {}", tunnel_id);

    // Create shutdown channel
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let shutdown_tx = Arc::new(shutdown_tx);

    // Handle Ctrl+C
    let shutdown_tx_clone = Arc::clone(&shutdown_tx);
    tokio::spawn(async move {
        if let Ok(()) = tokio::signal::ctrl_c().await {
            info!("Received Ctrl+C, initiating shutdown...");
            let _ = shutdown_tx_clone.send(());
        }
    });

    // Start connection loop
    let exit_code = connect_with_retry(tunnel_id, shutdown_rx).await;
    std::process::exit(exit_code);
} 