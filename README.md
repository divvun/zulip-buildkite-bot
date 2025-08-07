# Zulip-Buildkite Bot

A Rust-based webhook server that forwards Buildkite pipeline events to Zulip channels.

## Features

- Receives Buildkite webhook events (build started, finished, job completed)
- Formats events into readable Zulip messages with emojis and links
- **Smart Channel Routing**: Automatically routes messages to appropriate channels
  - `lang-*` pipelines → extract language code (e.g., `lang-sami-x-private` → `sami` channel)
  - `keyboard-*` pipelines → extract keyboard type (e.g., `keyboard-finnish-public` → `finnish` channel)
  - All other pipelines → default configured channel
- Configurable via command line arguments or environment variables
- Built with Rust for performance and reliability

## Setup

### 1. Create a Zulip Bot

1. Go to your Zulip organization's settings
2. Navigate to "Personal settings" > "Bots" > "Add a new bot"
3. **Select "Incoming webhook" as the bot type** (this is the correct type for third-party integrations)
4. Give your bot a name (e.g., "Buildkite Bot") and create it
5. Note down the bot email and API key from the bot's settings

### 2. Configure Environment Variables

```bash
export ZULIP_BOT_EMAIL="your-bot@your-org.zulipchat.com"
export ZULIP_BOT_API_KEY="your-bot-api-key"
export ZULIP_SERVER_URL="https://your-org.zulipchat.com"
export ZULIP_STREAM="buildkite"  # Channel/stream name to post to
```

### 3. Run the Server

```bash
# Using environment variables
cargo run -- server

# Or with command line arguments
cargo run -- server \
  --zulip-bot-email "your-bot@your-org.zulipchat.com" \
  --zulip-bot-api-key "your-bot-api-key" \
  --zulip-server-url "https://your-org.zulipchat.com" \
  --zulip-stream "buildkite" \
  --port 3000
```

### 4. Configure Buildkite Webhook

1. Go to your Buildkite pipeline settings
2. Add a webhook with URL: `https://your-server.com/webhook`
3. Select the events you want to forward (build.started, build.finished, job.finished)

## Supported Events

- **build.started**: Notifies when a build starts
- **build.finished**: Notifies when a build completes (with pass/fail status)
- **job.finished**: Notifies when individual jobs complete

## Message Format

The bot formats messages with:
- Emojis for quick status recognition (🔄 starting, ✅ passed, ❌ failed)
- **Ultra-compact single-line format** for minimal visual noise
- Clickable build numbers and job names that link directly to Buildkite
- **Unique topics per build**: Each build gets its own conversation thread
- Format: `Pipeline Name - Build #123` for easy tracking

### Example Messages

**Build Started (with commit message):**
```
🔄 Build #42 started
> Add new feature for user authentication
```

**Build Started (no commit message):**
```
🔄 Build #42 started
```

**Build Finished:**
```
✅ Build #43 passed
```

**Job Finished:**
```
✅ Job 'Unit Tests' passed
```

**Complete Build Flow in Topic `My Pipeline - Build #42`:**
```
🔄 Build #42 started
> Fix critical security vulnerability

✅ Job 'Unit Tests' passed  
❌ Job 'Linting' failed
❌ Build #42 failed
```

*Note: All build numbers and job names are clickable links to Buildkite*

## Testing

The bot includes a built-in test mode that can send mock webhook events to test your setup without connecting to a real Buildkite pipeline.

### Quick Test

1. Start the server in one terminal:
```bash
# Set your environment variables first
export ZULIP_BOT_EMAIL="your-bot@your-org.zulipchat.com"
export ZULIP_BOT_API_KEY="your-bot-api-key"
export ZULIP_SERVER_URL="https://your-org.zulipchat.com"
export ZULIP_STREAM="test-channel"

cargo run -- server
```

2. Send test events in another terminal:
```bash
# Send a complete build scenario (recommended for first test)
cargo run -- test --build-number 42

# Send specific event types
cargo run -- test --event-type build-started --build-number 100
cargo run -- test --event-type scenario --build-number 200
```

### Test Event Types

- `build-started`: Build starting notification
- `build-passed`: Successful build completion
- `build-failed`: Failed build notification  
- `build-canceled`: Canceled build notification
- `job-passed`: Successful job completion
- `job-failed`: Failed job notification
- `all`: Build start + 2 jobs + successful completion
- `scenario`: Build start + 2 jobs + failed completion (realistic workflow)
- `lang-routing`: Test language pipeline routing (→ `sami` channel)
- `keyboard-routing`: Test keyboard pipeline routing (→ `finnish` channel)

### Test Options

```bash
# Test against a different server
cargo run -- test --server-url http://my-server.com:8080 --build-number 42

# Change delay between events
cargo run -- test --delay 5 --build-number 42

# Test specific routing (these will go to different channels)
cargo run -- test --event-type lang-routing --build-number 100
cargo run -- test --event-type keyboard-routing --build-number 200

# Test specific build number (all events will use same build number)
cargo run -- test --build-number 999
```

**Key Feature**: All events for the same `--build-number` will appear in the same Zulip topic thread, showing you exactly how a complete build flow looks!

## Usage

```
A bot that forwards Buildkite events to Zulip

Usage: zulip-buildkite-bot <COMMAND>

Commands:
  server  Start the webhook server
  test    Send test webhook events to a running server
  help    Print this message or the help of the given subcommand(s)

Options:
  -h, --help  Print help
```

### Server Command

```
Start the webhook server

Usage: zulip-buildkite-bot server [OPTIONS] --zulip-bot-email <ZULIP_BOT_EMAIL> --zulip-bot-api-key <ZULIP_BOT_API_KEY> --zulip-server-url <ZULIP_SERVER_URL> --zulip-stream <ZULIP_STREAM>

Options:
  -p, --port <PORT>                            Port to listen on [default: 3000]
      --zulip-bot-email <ZULIP_BOT_EMAIL>      Zulip bot email [env: ZULIP_BOT_EMAIL=]
      --zulip-bot-api-key <ZULIP_BOT_API_KEY>  Zulip bot API key [env: ZULIP_BOT_API_KEY=]
      --zulip-server-url <ZULIP_SERVER_URL>    Zulip server URL [env: ZULIP_SERVER_URL=]
      --zulip-stream <ZULIP_STREAM>            Zulip stream/channel to post to [env: ZULIP_STREAM=]
  -h, --help                                   Print help
```

### Test Command

```
Send test webhook events to a running server

Usage: zulip-buildkite-bot test [OPTIONS]

Options:
      --server-url <SERVER_URL>      Server URL to send test webhooks to [default: http://localhost:3000]
      --event-type <EVENT_TYPE>      Type of test event to send [default: all]
      --delay <DELAY>                Delay between events in seconds [default: 2]
      --build-number <BUILD_NUMBER>  Build number to use for test events [default: 123]
  -h, --help                         Print help
```