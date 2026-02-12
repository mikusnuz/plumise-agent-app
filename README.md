# Plumise Agent

Desktop application for running a Plumise network inference agent. One-click setup to contribute your GPU/CPU computing power to the Plumise decentralized inference network and earn PLM rewards.

## Features
- One-click agent management (start/stop)
- Real-time system monitoring (CPU, RAM, VRAM)
- Live inference metrics (requests, tokens, latency)
- Automatic pre-flight checks before launch
- Real-time log viewer with search, filter, and export
- Auto-updates via built-in updater
- Modern glassmorphism UI

## Download
Download the latest installer from [GitHub Releases](https://github.com/mikusnuz/plumise-agent-app/releases).
- Windows: `.msi` or `.exe` (NSIS) installer

## Quick Start
1. Download and install the latest release
2. Open Plumise Agent
3. Go to Settings → Enter your private key (0x...)
4. Click "Start" on the Dashboard
5. Monitor your agent's performance and rewards

## Building from Source

### Prerequisites
- Node.js 20+
- Rust 1.77+
- Python 3.11+ (for building the agent sidecar)

### Steps
```bash
# Clone the repository
git clone https://github.com/mikusnuz/plumise-agent-app.git
cd plumise-agent-app

# Install frontend dependencies
npm install

# Development mode (requires plumise-agent on system PATH)
npm run tauri:dev

# Production build (requires sidecar binary)
npm run tauri:build
```

### Building the Sidecar
```bash
git clone https://github.com/mikusnuz/plumise-agent.git
cd plumise-agent
pip install -r requirements.txt pyinstaller
pyinstaller plumise-agent.spec
# Copy dist/plumise-agent.exe to plumise-agent-app/src-tauri/binaries/plumise-agent-x86_64-pc-windows-msvc.exe
```

## Architecture
- **Shell**: Tauri v2 (Rust + WebView)
- **Frontend**: React + TypeScript + Tailwind CSS v4 + Recharts
- **Agent**: PyInstaller-bundled [plumise-agent](https://github.com/mikusnuz/plumise-agent) (sidecar)
- **Chain**: [Plumise Network](https://plumise.com) (EVM-compatible)

## Tech Stack
| Component | Technology |
|-----------|-----------|
| Desktop Shell | Tauri v2 |
| Frontend | React 19, TypeScript, Tailwind v4 |
| Charts | Recharts |
| Animations | motion/react |
| Backend | Rust (tokio, reqwest, sysinfo) |
| Agent | Python (PyTorch, gRPC, Web3) |
| Build | PyInstaller, GitHub Actions |

## License
MIT

## Links
- [Plumise Chain](https://plumise.com)
- [Plumise Explorer](https://explorer.plumise.com)
- [Plumise Agent](https://github.com/mikusnuz/plumise-agent)
