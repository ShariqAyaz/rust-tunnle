# Agent Service (Reverse Tunnel)

A standalone Rust-based agent that:
- Creates reverse tunnel to gateway
- Uses secure tunnel ID for identification
- Maintains persistent WebSocket connection
- Forwards requests to local HTTP server and returns responses

## Technical Story & Implementation

The Agent Service acts as a bridge between the gateway and your local services, enabling secure access to services behind NAT:

### Core Functionality

#### 1. Connection Management
- Establishes WebSocket connection to gateway
- Performs handshake with tunnel ID
- Maintains connection with ping/pong
- Handles reconnection with exponential backoff
- Validates responses and manages errors

#### 2. Request Handling
- Receives forwarded requests from gateway
- Forwards to local HTTP server (default: http://127.0.0.1:8000)
- Supports multiple HTTP methods (GET, POST)
- Preserves headers and request body
- Returns structured responses with metadata

#### 3. Error Handling
- Connection retry with exponential backoff (1-30 seconds)
- Maximum 10 retry attempts
- Detailed error logging
- Graceful connection cleanup
- Local server error handling

## Prerequisites

Before starting the agent, ensure:
1. Gateway service is running and accessible
2. Local server (e.g., Laravel) is running on port 8000
3. Valid tunnel ID is available
4. No firewall blocking WebSocket connections

## Testing Locally

1. Start your local server first:
```bash
# Example: Laravel server
php artisan serve
```

2. Start the agent (from the agent directory):
```bash
cd agent && RUST_LOG=info cargo run --bin agent -- --tunnel-id agent_550e8400-e29b-41d4-a716-446655440000_prod
```

### Common Issues and Solutions

1. **"No bin target named 'agent'" Error**
   - Symptom: Error when trying to run agent from main directory
   - Solution: Change to agent directory first
   ```bash
   cd agent
   ```

2. **"Connection refused" to Local Server**
   - Symptom: Error connecting to http://127.0.0.1:8000
   - Solution: Start your local server before making requests
   - Check: Ensure local server is running and port is correct

3. **Gateway Connection Issues**
   - Symptom: Cannot connect to gateway
   - Check: Gateway URL (default: ws://127.0.0.1:3000/ws)
   - Check: Firewall settings
   - Check: Gateway service is running

### Configuration

- `GATEWAY_URL`: WebSocket gateway URL (default: ws://127.0.0.1:3000/ws)
- `RUST_LOG`: Logging level (recommended: info)
- `--tunnel-id`: Required command-line argument (format: agent_{uuid}_{purpose})
- Local server URL: http://127.0.0.1:8000 (currently hardcoded)

### Response Format

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
    "timestamp": "2025-02-15T20:08:49.472112Z",
    "agent_version": "0.1.0"
  }
}
```

### Error Response Format

```json
{
  "message_type": "error",
  "payload": "Failed to forward request to local server: Connection refused (os error 61)"
}
```

### Known Limitations
1. Hardcoded local server URL
2. No TLS support yet
3. No request validation or filtering
4. Single-threaded request handling
5. No request queueing or rate limiting
6. No automatic local server health checks
7. Limited error recovery options

## Next Steps
1. Make local server URL configurable
2. Add TLS support for both gateway and local connections
3. Add request validation and filtering
4. Implement concurrent request handling
5. Add metrics collection
6. Add rate limiting and request queueing
7. Enhance error recovery and circuit breaking
8. Add local server health monitoring
9. Implement automatic service discovery 