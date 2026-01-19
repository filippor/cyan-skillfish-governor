#!/bin/bash
set -e

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

if [ "$EUID" -ne 0 ]; then 
    echo -e "${RED}Error: This script must be run as root${NC}"
    exit 1
fi

INSTALL_DIR="/etc/cyan-skillfish-governor-smu"
SERVICE_FILE="/etc/systemd/system/cyan-skillfish-governor-smu.service"

echo -e "${YELLOW}Uninstalling Cyan Skillfish Governor...${NC}"

# Stop and disable service
if systemctl is-active --quiet cyan-skillfish-governor-smu; then
    echo "Stopping service..."
    systemctl stop cyan-skillfish-governor-smu
fi

if systemctl is-enabled --quiet cyan-skillfish-governor-smu 2>/dev/null; then
    echo "Disabling service..."
    systemctl disable cyan-skillfish-governor-smu
fi

# Remove service file
if [ -f "$SERVICE_FILE" ]; then
    echo "Removing service file..."
    rm "$SERVICE_FILE"
    systemctl daemon-reload
fi

# Remove installation directory
if [ -d "$INSTALL_DIR" ]; then
    echo "Removing installation directory..."
    rm -rf "$INSTALL_DIR"
fi

echo -e "${GREEN}Uninstall complete!${NC}"
