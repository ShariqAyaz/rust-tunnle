# Agent Service (Reverse Tunnel)

A standalone Rust-based agent that:
- Creates reverse tunnel to gateway
- Uses secure tunnel ID for identification
- Maintains persistent WebSocket connection
- Handles and responds to forwarded requests

## Architecture

```
Private Network                     Public Network
┌─────────────────┐               ┌─────────────┐
│                 │   WebSocket   │             │
│  Agent Service  ├──connection──►│   Gateway   │
│    (No Port)    │   tunnel_id   │   :3000     │
└─────────────────┘               └─────────────┘
     Behind NAT                    Public Server

Connection Flow:
1. Agent initiates WebSocket connection
2. Gateway assigns connection ID
3. Agent sends tunnel ID handshake
4. Connection maintained with ping/pong
```

## Current Implementation

### Features
1. Connection Management:
   - Automatic connection to gateway
   - Secure tunnel ID validation
   - Automatic reconnection with backoff
   - Connection health monitoring

2. Request Handling:
   - Receives forwarded HTTP requests
   - Processes request data
   - Returns structured responses
   - Includes request metadata and timestamps

3. Error Handling:
   - Connection retry with exponential backoff
   - Maximum retry attempts (5)
   - Detailed error logging
   - Graceful connection cleanup

### Testing Locally

1. Start the agent:
```bash
RUST_LOG=info cargo run --bin agent -- --tunnel-id YOUR_TUNNEL_ID
```

Example tunnel ID:
```
agent_550e8400-e29b-41d4-a716-446655440000_prod
```

2. Current Response Format:
```json
{
  "status": "success",
  "message": "Request processed successfully",
  "data": {
    "request": {
      "method": "POST",
      "path": "/",
      "headers": [["content-type", "application/json"]],
      "body": {"message": "Hello from client!"}
    },
    "timestamp": "2025-02-13T22:45:40.418279+00:00",
    "agent_version": "0.1.0"
  }
}
```

### Configuration
- `GATEWAY_URL`: WebSocket gateway URL (default: ws://localhost:3000/ws)
- `RUST_LOG`: Logging level (recommended: info)
- `--tunnel-id`: Required command-line argument for identification

### Known Limitations
1. No request validation
2. No custom request processing
3. Only echoes request data
4. No TLS support yet

## Next Steps
1. Add custom request processing
2. Implement request validation
3. Add TLS support
4. Add metrics collection
5. Enhance error recovery 