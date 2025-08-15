# Zulip-Buildkite Bot

A Rust-based webhook server that forwards Buildkite pipeline events to Zulip channels.

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
3. Select the events you want to forward (the bot supports **all Buildkite events** - see supported events section below)

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
