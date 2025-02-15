# Gateway Service

A standalone Rust-based gateway service that:
- Accepts WebSocket connections from agents
- Validates agent tunnel IDs
- Maintains persistent connections using DashMap for concurrent access
- Forwards HTTP requests to connected agents and returns responses

## Technical Story & Sequence of Events

The Gateway Service orchestrates communication between HTTP clients and agents through a series of well-defined sequences:

### Core Sequences

#### Sequence 1: Gateway Startup and Initialisation
The gateway begins its life by setting up the foundation for all future operations:
1. Initialises logging system for operational visibility
2. Creates a shutdown channel for graceful termination
3. Establishes shared state (AppState) using DashMap for concurrent connection tracking
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
3. Adds connection to DashMap state
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

### Prerequisites

Before starting the gateway, ensure:
1. Port 3000 is available (will get "Address already in use" error otherwise)
2. At least one agent is ready to connect
3. The local server (e.g., Laravel on port 8000) is running for the agent to forward requests to

### Testing Locally

1. Start the gateway:
```bash
# Kill any existing process on port 3000 if needed
lsof -ti:3000 | xargs kill -9

# Start the gateway
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

### Common Issues and Solutions

1. **"Address already in use" Error**
   - Symptom: Gateway fails to start with error code 48
   - Solution: Kill existing process using port 3000
   ```bash
   lsof -ti:3000 | xargs kill -9
   ```

2. **"Connection refused" Error on Requests**
   - Symptom: Requests fail with "tcp connect error: Connection refused (os error 61)"
   - Cause: Local server (e.g., Laravel) not running on port 8000
   - Solution: Start your local server before making requests

3. **No Agents Available**
   - Symptom: Requests return "No agents available"
   - Solution: Ensure at least one agent is connected and check `/connections` endpoint

### Known Limitations
1. Single response handler per agent connection (potential race condition with concurrent requests)
2. No agent selection mechanism (uses first available agent)
3. No authentication for HTTP endpoints
4. No TLS support yet
5. Limited error handling for concurrent requests
6. Requires manual port management
7. No automatic reconnection for lost agent connections

## Next Steps
1. Implement concurrent request handling per agent
2. Add agent selection mechanism
3. Add authentication and TLS
4. Add request/response timeout configuration
5. Implement proper error handling for concurrent scenarios
6. Add metrics collection and monitoring
7. Add automatic port conflict resolution
8. Implement agent connection health checks