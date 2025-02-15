# Gateway Service

A standalone Rust-based gateway service that:
- Accepts WebSocket connections from agents
- Validates agent tunnel IDs
- Maintains persistent connections
- Forwards HTTP requests to connected agents and returns responses

## Technical Story & Sequence of Events

The Gateway Service orchestrates communication between HTTP clients and agents through a series of well-defined sequences. Here's how the story unfolds:

### Core Sequences

#### Sequence 1: Gateway Startup and Initialisation
The gateway begins its life by setting up the foundation for all future operations:
1. Initialises logging system for operational visibility
2. Creates a shutdown channel for graceful termination
3. Establishes shared state (AppState) to track agent connections
4. Configures HTTP routes:
   - `/health` for system status
   - `/ws` for WebSocket connections
   - `/connections` for active connection listing
   - `/forward` for explicit request forwarding
   - `/*path` for direct request handling
5. Binds to port 3000 and begins serving requests

#### Sequence 2: WebSocket Connection Upgrade
When an agent attempts to connect:
1. Agent sends HTTP request to `/ws`
2. Gateway validates the connection request
3. Connection is upgraded to WebSocket protocol
4. Control is passed to the WebSocket handler

#### Sequence 3: WebSocket Communication Lifecycle
Once a WebSocket connection is established:
1. Gateway generates a unique connection ID
2. Records connection timestamp
3. Adds connection to shared state
4. Sends connection ID to agent
5. Splits communication into parallel tasks:
   - Sender: Handles outbound messages
   - Receiver: Processes inbound messages
6. Maintains connection until closure/error

#### Sequence 4: HTTP Request Forwarding (POST /forward)
For explicit forwarding requests:
1. Receives POST request with forwarding details
2. Creates response channel for agent reply
3. Selects available agent with valid tunnel ID
4. Configures response handler
5. Forwards request via WebSocket
6. Awaits response (5-second timeout)
7. Returns response to client

#### Sequence 5: Direct GET Request Handling
For direct browser/client requests:
1. Captures any GET request not matching other routes
2. Sets up response channel
3. Identifies available agent
4. Wraps and forwards request
5. Awaits response (30-second timeout)
6. Returns formatted HTTP response

### Supporting Methods

The gateway employs several supporting methods that enhance these core sequences:

#### Tunnel ID Validation
- **Method**: `validate_tunnel_id`
- **Supports**: Sequence 3 (WebSocket Communication)
- **Purpose**: Ensures agent identification follows the format `agent_{uuid}_{purpose}`
- **Validation Rules**:
  - Must have exactly 3 parts
  - First part must be "agent"
  - Second part must be valid UUID
  - Third part must be alphanumeric with underscores
- **Importance**: Critical for security and agent identification

#### Health Check Handler
- **Method**: `handle_health_check`
- **Supports**: Sequence 1 (Gateway Operation)
- **Purpose**: Provides system status and version information
- **Usage**: Monitoring and system verification

#### Connection Listing
- **Method**: `handle_list_connections`
- **Supports**: All sequences (Operational Monitoring)
- **Purpose**: Provides visibility into active connections
- **Usage**: System monitoring and debugging

## Architecture

```
                         Gateway Service
                        (127.0.0.1:3000)
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
   - `GET /health` - Health check endpoint
   - `GET /ws` - WebSocket endpoint for agents
   - `GET /connections` - List active connections
   - `POST /forward` - Forward requests to agents
   - `GET /*path` - Direct request forwarding to agents

3. Request Forwarding:
   - Forwards HTTP requests to available agents
   - Waits for agent responses (with timeouts)
   - Returns responses to clients with appropriate status codes
   - Supports both direct GET requests and POST forwarding

### Testing Locally

1. Start the gateway:
```bash
RUST_LOG=info cargo run --bin gateway
```

2. Test endpoints:
```bash
# Health check
curl http://127.0.0.1:3000/health

# List connections
curl http://127.0.0.1:3000/connections

# Forward request to agent
curl -X POST http://127.0.0.1:3000/forward \
  -H "Content-Type: application/json" \
  -d '{"message": "Hello from client!"}'

# Direct GET request (forwarded to agent)
curl http://127.0.0.1:3000/about
```

3. Start an agent:
```bash
RUST_LOG=info cargo run --bin agent -- --tunnel-id agent_550e8400-e29b-41d4-a716-446655440000_prod
```

### Known Limitations
1. Single response handler per agent connection (potential race condition with concurrent requests)
2. No agent selection mechanism (uses first available agent)
3. No authentication for HTTP endpoints
4. No TLS support yet
5. Limited error handling for concurrent requests

## Next Steps
1. Implement concurrent request handling per agent
2. Add agent selection mechanism
3. Add authentication and TLS
4. Add request/response timeout configuration
5. Implement proper error handling for concurrent scenarios
6. Add metrics collection and monitoring