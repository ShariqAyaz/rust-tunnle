# test_server.py
from http.server import HTTPServer, BaseHTTPRequestHandler
import json
import logging
import sys
import socket
import threading
import asyncio
import time
import random
import urllib.parse
from socketserver import ThreadingMixIn
from concurrent.futures import ThreadPoolExecutor, Future
from queue import Queue, Empty

logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(levelname)s - %(message)s',
    stream=sys.stdout
)
logger = logging.getLogger(__name__)

class RequestHandler(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    
    # Create a thread pool for handling long-running requests
    executor = ThreadPoolExecutor(max_workers=10)
    response_queues = {}
    response_lock = threading.Lock()

    def setup(self):
        super().setup()
        self.request.setsockopt(socket.SOL_SOCKET, socket.SO_KEEPALIVE, 1)
        if hasattr(socket, 'TCP_KEEPIDLE'):
            self.request.setsockopt(socket.SOL_TCP, socket.TCP_KEEPIDLE, 60)
        if hasattr(socket, 'TCP_KEEPINTVL'):
            self.request.setsockopt(socket.SOL_TCP, socket.TCP_KEEPINTVL, 60)
        if hasattr(socket, 'TCP_KEEPCNT'):
            self.request.setsockopt(socket.SOL_TCP, socket.TCP_KEEPCNT, 5)

    def generate_random_rows(self):
        rows = []
        for i in range(10):
            # Generate a row with 8 random digits
            row = ''.join([str(random.randint(0, 9)) for _ in range(8)])
            rows.append(f"Row {i+1}: {row}")
        return "\n".join(rows)

    def process_long_request(self, request_id):
        """Handle long-running request processing in a separate thread"""
        try:
            logger.info(f"[{request_id}] Processing request...")
            
            # Check if client is still connected before each update
            if not self.is_client_connected(request_id):
                logger.info(f"[{request_id}] Client disconnected, stopping processing")
                return

            time.sleep(10)
            logger.info(f"[{request_id}] Stage 1 complete...")
            
            if not self.is_client_connected(request_id):
                logger.info(f"[{request_id}] Client disconnected, stopping processing")
                return

            logger.info(f"[{request_id}] Stage 2 complete...")
            logger.info(f"[{request_id}] Stage 3 complete...")
            response = self.generate_random_rows()
            
            # Only send response if client is still connected
            if self.is_client_connected(request_id):
                with self.response_lock:
                    if request_id in self.response_queues:
                        self.response_queues[request_id].put(("final", response))
        except Exception as e:
            logger.error(f"[{request_id}] Error in long request: {e}")
            if self.is_client_connected(request_id):
                with self.response_lock:
                    if request_id in self.response_queues:
                        self.response_queues[request_id].put(("error", f"Error: {e}"))
        finally:
            # Clean up resources
            self.cleanup_request(request_id)

    def is_client_connected(self, request_id):
        """Check if client is still connected"""
        with self.response_lock:
            return request_id in self.response_queues

    def cleanup_request(self, request_id):
        """Clean up resources for a request"""
        with self.response_lock:
            if request_id in self.response_queues:
                del self.response_queues[request_id]
                logger.info(f"[{request_id}] Cleaned up resources")

    def serve_about_page(self):
        html_content = """
        <!DOCTYPE html>
        <html lang="en">
        <head>
            <meta charset="UTF-8">
            <meta name="viewport" content="width=device-width, initial-scale=1.0">
            <title>About Page</title>
            <style>
                body {
                    font-family: Arial, sans-serif;
                    max-width: 800px;
                    margin: 0 auto;
                    padding: 20px;
                    background-color: #f5f5f5;
                }
                header {
                    background-color: #333;
                    color: white;
                    padding: 20px;
                }
                main {
                    background-color: white;
                    padding: 20px;
                    margin-top: 20px;
                }
                h1 {
                    margin: 0;
                }
                p {
                    line-height: 1.6;
                }
            </style>
            <script>
                // Immediately set cursor to default and stop any loading indicators
                document.documentElement.style.cursor = 'default';
            </script>
        </head>
        <body>
            <header>
                <h1>About Our Service</h1>
            </header>
            <main>
                <h2>Welcome to Our Test Server</h2>
                <p>This is a simple test server that demonstrates the following features:</p>
                <ul>
                    <li>HTTP request handling</li>
                    <li>WebSocket connections</li>
                    <li>Request forwarding</li>
                    <li>Basic routing</li>
                </ul>
                <p>The server is part of a larger system that includes:</p>
                <ul>
                    <li>A Gateway service for managing connections</li>
                    <li>Agent services for handling requests</li>
                    <li>Reverse tunneling capabilities</li>
                </ul>
            </main>
            <script>
                // Ensure page is marked as complete
                if (document.readyState === 'loading') {
                    document.addEventListener('DOMContentLoaded', function() {
                        document.documentElement.style.cursor = 'default';
                        window.stop();  // Stop any pending loads
                    });
                } else {
                    document.documentElement.style.cursor = 'default';
                    window.stop();  // Stop any pending loads
                }
            </script>
        </body>
        </html>
        """
        try:
            # Convert content to bytes once
            content_bytes = html_content.encode('utf-8')
            content_length = len(content_bytes)

            # Send headers
            self.send_response(200)
            self.send_header('Content-Type', 'text/html; charset=utf-8')
            self.send_header('Content-Length', str(content_length))
            self.send_header('Connection', 'close')
            self.send_header('Cache-Control', 'no-cache, no-store, must-revalidate')
            self.send_header('Pragma', 'no-cache')
            self.send_header('Expires', '0')
            self.end_headers()

            # Send content in one go
            self.wfile.write(content_bytes)
            self.wfile.flush()

            # Force connection close
            try:
                self.close_connection = True
                self.connection.shutdown(socket.SHUT_WR)
            except Exception as e:
                logger.debug(f"Connection already closed: {e}")

            logger.info("About page served successfully")
        except Exception as e:
            logger.error(f"Error serving about page: {e}")
            # Try to send error response if headers haven't been sent
            if not self.headers_sent:
                self.send_error(500, f"Internal error: {str(e)}")
            raise

    def do_GET(self):
        client_ip = self.client_address[0]
        logger.info(f"Received GET request from {client_ip} for path: {self.path}")
        logger.info(f"Server address: {self.server.server_address}")
        logger.info(f"Headers: {self.headers}")
        
        if self.path == '/about':
            self.serve_about_page()
            logger.info(f"Served about page to {client_ip}")
            return

        response = "Hello from test server!" + self.generate_random_rows()
        self.send_response(200)
        self.send_header('Content-type', 'text/plain')
        self.end_headers()
        self.wfile.write(response.encode('utf-8'))
        logger.info(f"Response sent to {client_ip}")

    def do_POST(self):
        logger.info(f"Received POST request from {self.client_address}")
        logger.info(f"Headers: {self.headers}")
        
        try:
            content_length = int(self.headers.get('Content-Length', 0))
            post_data = self.rfile.read(content_length)
            logger.info(f"Received POST data: {post_data.decode('utf-8')}")
            
            self.send_response(200)
            self.send_header('Content-type', 'text/plain')
            self.send_header('Connection', 'close')
            self.end_headers()
            
            response = f"Received POST data: {post_data.decode('utf-8')}"
            logger.info(f"Sending response: {response}")
            self.wfile.write(response.encode('utf-8'))
            self.wfile.flush()
            logger.info("POST response sent successfully")
            sys.stdout.flush()
        except Exception as e:
            logger.error(f"Error handling POST request: {e}")
            self.send_error(500, f"Internal error: {str(e)}")

# Add new ThreadedHTTPServer class
class ThreadedHTTPServer(ThreadingMixIn, HTTPServer):
    """Handle requests in a separate thread."""
    daemon_threads = True

# Update the run_server function
def run_server():
    server_address = ('127.0.0.1', 8000)
    retries = 3
    
    for attempt in range(retries):
        try:
            httpd = ThreadedHTTPServer(server_address, RequestHandler)
            httpd.socket.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            
            # Get the actual IP address
            hostname = socket.gethostname()
            ip_address = socket.gethostbyname(hostname)
            
            logger.info(f"Server socket created with options:")
            logger.info(f"Hostname: {hostname}")
            logger.info(f"IP Address: {ip_address}")
            logger.info(f"Listening Address: {httpd.server_address}")
            logger.info(f"Socket family: {httpd.socket.family}")
            logger.info(f"Socket type: {httpd.socket.type}")
            
            logger.info(f"Server running on http://{ip_address}:8000")
            try:
                httpd.serve_forever()
            except KeyboardInterrupt:
                logger.info("Shutting down the server...")
                httpd.server_close()
                break
            except Exception as e:
                logger.error(f"Server error: {e}")
                httpd.server_close()
                break
        except OSError as e:
            if e.errno == 48:  # Address already in use
                if attempt < retries - 1:
                    logger.warning(f"Port 8000 is in use, waiting 5 seconds before retry {attempt + 1}/{retries}")
                    time.sleep(5)
                    continue
                else:
                    logger.error("Could not bind to port 8000 after multiple attempts")
                    raise
            else:
                raise

if __name__ == '__main__':
    run_server() 