#!/bin/bash
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Check if running as root
if [ "$EUID" -ne 0 ]; then
    echo -e "${RED}Error: This script must be run as root${NC}"
    echo "Please run: sudo ./install.sh"
    exit 1
fi

echo -e "${GREEN}Installing Cyan Skillfish Governor...${NC}"

# Installation directory
INSTALL_DIR="/etc/cyan-skillfish-governor-smu"
BIN_PATH="$INSTALL_DIR/cyan-skillfish-governor-smu"
CONFIG_PATH="$INSTALL_DIR/config.toml"
SERVICE_FILE="/etc/systemd/system/cyan-skillfish-governor-smu.service"
PERFORMANCE_MODE_PATH="/usr/local/bin/cyan-skillfish-performance-mode"
DBUS_POLICY_PATH="/etc/dbus-1/system.d/com.cyanskillfish.Governor.conf"

# Create installation directory
echo "Creating installation directory..."
mkdir -p "$INSTALL_DIR"

# Copy binary
echo "Installing binary..."
cp cyan-skillfish-governor-smu "$BIN_PATH"
chmod +x "$BIN_PATH"

echo "Installing performance mode script..."
cp scripts/cyan-skillfish-performance-mode "$PERFORMANCE_MODE_PATH"
chmod +x "$PERFORMANCE_MODE_PATH"

echo "Installing D-Bus policy..."
cp com.cyanskillfish.Governor.conf "$DBUS_POLICY_PATH"

echo "Reloading D-Bus policy..."
if command -v busctl >/dev/null 2>&1; then
    busctl --system call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus ReloadConfig || true
fi

# Copy config if it doesn't exist
if [ -f "$CONFIG_PATH" ]; then
    echo -e "${YELLOW}Config file already exists, backing up to config.toml.bak${NC}"
    cp "$CONFIG_PATH" "$CONFIG_PATH.bak"
fi
cp config.toml "$CONFIG_PATH"

# Create systemd service
echo "Creating systemd service..."
cat > "$SERVICE_FILE" << EOF
[Unit]
Description=Cyan Skillfish GPU Governor
After=multi-user.target

[Service]
Type=simple
ExecStart=$BIN_PATH $CONFIG_PATH
Restart=on-failure
RestartSec=5s
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
EOF

# Reload systemd
echo "Reloading systemd..."
systemctl daemon-reload

echo -e "${GREEN}Installation complete!${NC}"
echo ""
echo "To start the service:"
echo "  sudo systemctl start cyan-skillfish-governor-smu"
echo ""
echo "To enable at boot:"
echo "  sudo systemctl enable cyan-skillfish-governor-smu"
echo ""
echo "To check status:"
echo "  sudo systemctl status cyan-skillfish-governor-smu"
echo ""
echo "D-Bus policy installed at:"
echo "  $DBUS_POLICY_PATH"
echo ""
echo "Steam launch option example:"
echo "  cyan-skillfish-performance-mode %command%"
echo ""
echo "Performance mode command:"
echo "  cyan-skillfish-performance-mode --on   # enable"
echo "  cyan-skillfish-performance-mode --off  # disable"
echo "  cyan-skillfish-performance-mode --status # check status"
echo ""
echo "To view logs:"
echo "  sudo journalctl -u cyan-skillfish-governor-smu -f"
echo ""
echo -e "${YELLOW}Note: Edit $CONFIG_PATH to customize settings${NC}"