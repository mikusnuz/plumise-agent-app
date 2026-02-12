#!/usr/bin/env bash
# Sets up the plumise-agent sidecar for local Tauri development.
# Requires: plumise-agent repo cloned alongside this repo, Python 3.11+

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
AGENT_DIR="${PROJECT_DIR}/../plumise-agent"
BINARIES_DIR="${PROJECT_DIR}/src-tauri/binaries"

# Detect target triple
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)  TARGET="aarch64-apple-darwin" ;;
  Darwin-x86_64) TARGET="x86_64-apple-darwin" ;;
  Linux-x86_64)  TARGET="x86_64-unknown-linux-gnu" ;;
  MINGW*|MSYS*)  TARGET="x86_64-pc-windows-msvc" ;;
  *)             echo "Unsupported platform"; exit 1 ;;
esac

echo "Building plumise-agent for ${TARGET}..."

if [ ! -d "$AGENT_DIR" ]; then
  echo "Error: plumise-agent repo not found at $AGENT_DIR"
  echo "Clone it: git clone https://github.com/mikusnuz/plumise-agent.git $AGENT_DIR"
  exit 1
fi

cd "$AGENT_DIR"
pip install -e ".[dev]" pyinstaller 2>/dev/null || pip install -r requirements.txt pyinstaller

if [ -f "plumise-agent.spec" ]; then
  pyinstaller plumise-agent.spec --noconfirm
else
  pyinstaller --onefile --name plumise-agent src/plumise_agent/cli/main.py
fi

mkdir -p "$BINARIES_DIR"

EXT=""
if [[ "$TARGET" == *"windows"* ]]; then
  EXT=".exe"
fi

cp "dist/plumise-agent${EXT}" "${BINARIES_DIR}/plumise-agent-${TARGET}${EXT}"
echo "Sidecar binary placed at: ${BINARIES_DIR}/plumise-agent-${TARGET}${EXT}"
echo "You can now run: npm run tauri:dev"
