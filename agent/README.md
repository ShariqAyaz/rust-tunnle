# Agent Service (Reverse Tunnel)

A standalone Rust-based agent that:
- Creates reverse tunnel to gateway
- Uses secure tunnel ID for identification
- Maintains persistent WebSocket connection
- Forwards requests to local HTTP server and returns responses

## Architecture

```
Private Network                     Public Network
┌─────────────────┐               ┌─────────────┐
│  Local Server   │               │             │
│    :8000    ◄───┤   WebSocket   │             │
│                 │   connection  │   Gateway   │
│  Agent Service  ├──────────────►│   :3000     │
│    (No Port)    │   tunnel_id   │             │
└─────────────────┘               └─────────────┘
     Behind NAT                    Public Server

Connection Flow:
1. Agent initiates WebSocket connection
2. Gateway assigns connection ID
3. Agent sends tunnel ID handshake
4. Connection maintained with ping/pong
5. Requests forwarded to local server (127.0.0.1:8000)
```

## Current Implementation

### Features
1. Connection Management:
   - Automatic connection to gateway
   - Secure tunnel ID validation
   - Automatic reconnection with exponential backoff
   - Connection health monitoring with ping/pong
   - Maximum 10 retry attempts

2. Request Handling:
   - Receives forwarded HTTP requests from gateway
   - Forwards requests to local server (127.0.0.1:8000)
   - Supports GET and POST methods
   - Preserves headers and request body
   - Returns structured responses with metadata

3. Error Handling:
   - Connection retry with exponential backoff (1-30 seconds)
   - Maximum retry attempts (10)
   - Detailed error logging
   - Graceful connection cleanup
   - Local server error handling

### Testing Locally

1. Start your local server (e.g., Laravel) on port 8000:
```bash
# Example: Laravel server
php artisan serve
```

2. Start the agent:
```bash
RUST_LOG=info cargo run --bin agent -- --tunnel-id YOUR_TUNNEL_ID
```

Example tunnel ID:
```
agent_550e8400-e29b-41d4-a716-446655440000_prod
```

3. Current Response Format:
```json
{
  "status": "success",
  "message": "Request processed successfully",
  "data": {
    "status_code": 200,
    "headers": [
      ["content-type", "text/html"],
      ["connection", "close"]
    ],
    "body": "Response from local server",
    "timestamp": "2025-02-13T22:45:40.418279+00:00",
    "agent_version": "0.1.0"
  }
}
```

### Configuration
- `GATEWAY_URL`: WebSocket gateway URL (default: ws://127.0.0.1:3000/ws)
- `RUST_LOG`: Logging level (recommended: info)
- `--tunnel-id`: Required command-line argument for identification
- Local server URL: http://127.0.0.1:8000 (currently hardcoded)

### Known Limitations
1. Hardcoded local server URL
2. No TLS support yet
3. No request validation or filtering
4. Single-threaded request handling
5. No request queueing or rate limiting

## Next Steps
1. Make local server URL configurable
2. Add TLS support for both gateway and local connections
3. Add request validation and filtering
4. Implement concurrent request handling
5. Add metrics collection
6. Add rate limiting and request queueing
7. Enhance error recovery and circuit breaking 