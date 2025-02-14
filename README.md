# Gateway Service

A standalone Rust-based gateway service that:
- Accepts WebSocket connections from agents
- Validates agent tunnel IDs
- Maintains persistent connections
- Forwards HTTP requests to connected agents

## Architecture

```
                         Gateway Service
                        (209.38.153.137)
                        ┌──────────────┐
                        │              │
HTTP Clients            │   Gateway    │           Agents
┌──────────┐            │   :3000      │        ┌──────────┐
│          │────/──────►│   - State    │◄───────┤ Agent 1  │
│ Client 1 │    HTTP    │   - Routes   │   WS   └──────────┘
└──────────┘            │              │
                        │              │        ┌──────────┐
┌──────────┐            │              │◄───────┤ Agent 2  │
│ Client 2 │────/───-──►│              │   WS   └──────────┘
└──────────┘    HTTP    └──────────────┘

WS = WebSocket Connection with tunnel_id
```

## Current Implementation

### Features
1. WebSocket Connections:
   - Accepts agent connections via `/ws`
   - Validates tunnel ID format: `agent_{uuid}_{purpose}`
   - Maintains persistent connections with ping/pong
   - Tracks connection state

2. HTTP Endpoints:
   - `GET /` - Health check
   - `GET /ws` - WebSocket endpoint for agents
   - `GET /connections` - List active connections
   - `POST /forward` - Forward requests to agents

3. Request Forwarding:
   - Forwards HTTP requests to any available agent
   - Currently returns success on forwarding, not actual response
   - Logs agent responses but doesn't return them to clients

### Testing Locally

1. Start the gateway:
```bash
RUST_LOG=info cargo run --bin gateway
```

2. Test endpoints:
```bash
# Health check
curl http://209.38.153.137:3000/

# List connections
curl http://209.38.153.137:3000/connections

# Forward request to agent
curl -X POST http://209.38.153.137:3000/forward \
  -H "Content-Type: application/json" \
  -d '{"message": "Hello from client!"}'
```

3. Start an agent:
```bash
RUST_LOG=info cargo run --bin agent -- --tunnel-id agent_550e8400-e29b-41d4-a716-446655440000_prod
```

### Known Limitations
1. No response forwarding from agent back to HTTP client
2. No agent selection (uses first available agent)
3. No authentication for HTTP endpoints
4. No TLS support yet

## Next Steps
1. Implement response forwarding from agent to client
2. Add agent selection mechanism
3. Add authentication and TLS
4. Add request/response timeout handling
5. Implement proper error handling