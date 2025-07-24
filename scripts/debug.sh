#!/bin/bash

# Debug helper script for Tauri v2 + Vue3 project
# This script helps coordinate frontend and backend debugging

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}🚀 Tauri Debug Helper${NC}"
echo -e "${BLUE}===================${NC}"

# Function to check if port is in use
check_port() {
    local port=$1
    if lsof -Pi :$port -sTCP:LISTEN -t >/dev/null 2>&1; then
        return 0
    else
        return 1
    fi
}

# Function to kill process on port
kill_port() {
    local port=$1
    local pid=$(lsof -ti:$port)
    if [ ! -z "$pid" ]; then
        echo -e "${YELLOW}Killing process on port $port (PID: $pid)${NC}"
        kill -9 $pid
        sleep 1
    fi
}

# Function to start frontend dev server
start_frontend() {
    echo -e "${GREEN}Starting frontend dev server...${NC}"
    yarn dev &
    FRONTEND_PID=$!

    echo -e "${YELLOW}Waiting for frontend server to start...${NC}"
    local max_attempts=30
    local attempt=0

    while [ $attempt -lt $max_attempts ]; do
        if check_port 1420; then
            echo -e "${GREEN}✅ Frontend server is running on http://localhost:1420${NC}"
            return 0
        fi
        sleep 1
        attempt=$((attempt + 1))
        echo -n "."
    done

    echo -e "${RED}❌ Frontend server failed to start after 30 seconds${NC}"
    return 1
}

# Function to cleanup on exit
cleanup() {
    echo -e "\n${YELLOW}Cleaning up...${NC}"
    if [ ! -z "$FRONTEND_PID" ]; then
        kill $FRONTEND_PID 2>/dev/null || true
    fi
    kill_port 1420
    exit 0
}

# Function to check if we're in the right directory
check_project_directory() {
    # Get the directory where this script is located
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

    # Change to project root
    cd "$PROJECT_ROOT"

    # Verify we're in a Tauri project
    if [ ! -f "package.json" ] || [ ! -d "src-tauri" ]; then
        echo -e "${RED}❌ This doesn't appear to be a Tauri project directory${NC}"
        echo -e "${YELLOW}Script location: $SCRIPT_DIR${NC}"
        echo -e "${YELLOW}Expected project root: $PROJECT_ROOT${NC}"
        exit 1
    fi

    echo -e "${GREEN}✅ Running from project root: $PROJECT_ROOT${NC}"
}

# Trap to cleanup on script exit
trap cleanup EXIT INT TERM

# Check and change to project directory
check_project_directory

# Main menu
echo -e "${BLUE}Choose debug mode:${NC}"
echo "1) Full Stack Debug (Frontend + Backend)"
echo "2) Backend Only Debug"
echo "3) Frontend Only (Dev Server)"
echo "4) Clean ports and exit"

read -p "Enter your choice (1-4): " choice

case $choice in
    1)
        echo -e "${BLUE}🔧 Starting Full Stack Debug Mode${NC}"

        # Check if frontend port is already in use
        if check_port 1420; then
            echo -e "${YELLOW}Port 1420 is already in use. Kill existing process? (y/n)${NC}"
            read -p "" kill_existing
            if [ "$kill_existing" = "y" ] || [ "$kill_existing" = "Y" ]; then
                kill_port 1420
            else
                echo -e "${RED}Cannot proceed with port 1420 in use${NC}"
                exit 1
            fi
        fi

        # Start frontend
        if start_frontend; then
            echo -e "${GREEN}🎯 Frontend is ready! Now start Zed debugger with 'Debug Tauri App' configuration${NC}"
            echo -e "${YELLOW}Press Ctrl+C to stop both frontend and backend${NC}"

            # Keep the script running to maintain the frontend server
            wait $FRONTEND_PID
        else
            echo -e "${RED}Failed to start frontend server${NC}"
            exit 1
        fi
        ;;

    2)
        echo -e "${BLUE}🔧 Backend Only Debug Mode${NC}"
        echo -e "${YELLOW}Make sure frontend dev server is running separately!${NC}"
        echo -e "${GREEN}Now start Zed debugger with 'Debug Rust Backend Only' configuration${NC}"
        echo -e "${BLUE}To start frontend separately, run: yarn dev${NC}"
        ;;

    3)
        echo -e "${BLUE}🔧 Frontend Only Mode${NC}"
        if check_port 1420; then
            echo -e "${YELLOW}Port 1420 is already in use. Kill existing process? (y/n)${NC}"
            read -p "" kill_existing
            if [ "$kill_existing" = "y" ] || [ "$kill_existing" = "Y" ]; then
                kill_port 1420
            else
                echo -e "${RED}Cannot proceed with port 1420 in use${NC}"
                exit 1
            fi
        fi

        if start_frontend; then
            echo -e "${GREEN}✅ Frontend dev server is running${NC}"
            echo -e "${YELLOW}Press Ctrl+C to stop${NC}"
            wait $FRONTEND_PID
        fi
        ;;

    4)
        echo -e "${BLUE}🧹 Cleaning up ports${NC}"
        kill_port 1420
        echo -e "${GREEN}✅ Ports cleaned${NC}"
        exit 0
        ;;

    *)
        echo -e "${RED}Invalid choice${NC}"
        exit 1
        ;;
esac
