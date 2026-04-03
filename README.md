# 🦀 Kaspa Node Telegram Bot (Enterprise Rust Edition)

![Rust](https://img.shields.io/badge/Rust-1.70+-orange.svg?style=flat-square)
![Kaspa](https://img.shields.io/badge/Kaspa-Network-70D4CB.svg?style=flat-square)
![Maintenance](https://img.shields.io/badge/Maintained%3F-yes-green.svg?style=flat-square)

An enterprise-grade, high-performance Telegram bot built entirely in Rust for the Kaspa ecosystem. Designed specifically for solo miners and node operators, this bot connects directly to your local node via WebSocket to provide **zero-latency** UTXO tracking, mining reward notifications, and real-time network analytics.

Rigorously tested in live solo mining environments to ensure absolute stability, zero missed blocks, and minimal resource consumption.

## ✨ Key Features

* **Zero-Latency Node Integration:** Bypasses public API bottlenecks by connecting directly to your local Kaspa node via WebSocket.
* **Ultra-Low Footprint:** Completely rewritten from legacy Node.js to Rust, virtually eliminating RAM bloat, memory leaks, and garbage collection pauses.
* **Thread-Safe Concurrency:** Powered by `tokio` and `dashmap` to handle thousands of concurrent monitoring requests without race conditions.
* **Smart Memory Cleanup:** Features an asynchronous background garbage collector that actively purges processed TXs (5000+ confirmations) from active memory.
* **Advanced Admin Controls:** Secure your bot with admin-only commands for system diagnostics (`/sys`), remote restarts (`/restart`), and live systemd log fetching (`/logs`).
* **Fallback Resilience:** Graceful degradation to Kaspa public APIs only if the local node fetch fails, ensuring 100% uptime.

## ⚙️ Prerequisites

* A running Kaspa Node (e.g., `kaspad` running locally or remotely).
* [Rust Toolchain](https://rustup.rs/) (v1.70 or higher).
* A Telegram Bot Token (from [@BotFather](https://t.me/botfather)).

## 🚀 Installation & Setup

1. **Clone the repository:**
   ```bash
   git clone https://github.com/KaspaPulse/kaspa-rust-bot.git
   cd kaspa-telegram-bot
   ```

2. **Configure your environment:**
   Create a `.env` file in the root directory and add your specific details:
   ```env
   BOT_TOKEN=your_telegram_bot_token
   ADMIN_ID=your_telegram_user_id
   # Point this to your local node (e.g., Node02)
   WS_URL=ws://127.0.0.1:18110
   RUST_LOG=info
   ```

3. **Build the Enterprise Release:**
   Compile the binary optimized for speed and memory efficiency:
   ```bash
   cargo build --release
   ```

4. **Run the Engine:**
   ```bash
   ./target/release/kaspa-rust-bot
   ```

## 📱 Telegram Commands

### Public Commands
* `/add <address>` - Start tracking a Kaspa wallet for rewards.
* `/remove <address>` - Stop monitoring a wallet.
* `/list` - View your tracked portfolio.
* `/balance` - Check live balances directly from the node.
* `/network` - View current network Hashrate & Difficulty.
* `/fees` - View live Mempool fee estimates.
* `/supply` - Check circulating vs max supply.
* `/dag` - View local BlockDAG details (Blocks & Headers).

### 👑 Admin Exclusive Commands
* `/sys` - View server RAM usage and bot monitoring status.
* `/pause` / `/resume` - Safely disconnect/reconnect the node WebSocket engine.
* `/restart` - Reboot the bot process via systemd.
* `/logs` - Fetch the last 25 lines of system logs directly to Telegram.
* `/broadcast <msg>` - Send an announcement to all tracked users.

## 💖 Support & Donations

This project is continuously tested and optimized to secure solo mining operations and maintain flawless node monitoring. If you find this bot valuable for your infrastructure, consider supporting the ongoing testing and maintenance:

**Kaspa (KAS) Donation Address:**
`kaspa:qz0yqq8z3twwgg7lq2mjzg6w4edqys45w2wslz7tym2tc6s84580vvx9zr44g`

---
*Disclaimer: This project is provided as-is. Always test binaries in your specific node environment before relying on them for critical financial notifications.*
