import socket

BROADCAST_PORT = 8080

# Create a UDP socket
sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)

# Allow reuse of address, important for binding to broadcast address on some systems
sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)

# Bind to the broadcast address and port
# Use 0.0.0.0 to listen on all available network interfaces
sock.bind(('', BROADCAST_PORT))

print(f"Listening for UDP broadcasts on port {BROADCAST_PORT}...")

while True:
    data, addr = sock.recvfrom(1024) # Buffer size 1024 bytes
    print(f"Received message: '{data.decode()}' from {addr}")
