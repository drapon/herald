# Herald

AI-powered screen capture assistant for macOS.

Herald runs in the background, periodically capturing your desktop screen and storing the images locally. When you need context-aware advice, Herald uses AI (Claude/Gemini) to analyze your recent screenshots and provide relevant suggestions.

## Features

- **Background Screen Capture**: Automatic periodic screenshots (configurable interval)
- **Local Storage**: All data stored locally in `~/.herald/` (privacy-first)
- **AI Integration**: Claude and Gemini support for multimodal analysis
- **Configurable Retention**: Automatic cleanup of old captures

## Requirements

- macOS 14.0+ (Sonoma or later)
- Rust 1.75+
- Screen Recording permission

## Installation

```bash
# Clone the repository
git clone https://github.com/herald/herald.git
cd herald

# Build
cargo build --release

# The binary will be at target/release/herald
```

## Configuration

Herald uses TOML configuration at `~/.herald/config.toml`:

```toml
[capture]
interval_seconds = 60    # Capture interval (default: 60)
image_quality = 6        # PNG compression 0-9 (default: 6)

# Display selection (optional)
# - Omit: Capture all displays combined into one image (default)
# - Single number: Capture only that display (e.g., display = 0)
# - Array: Capture multiple displays as separate files (e.g., display = [0, 1])
# display = 0

[storage]
data_dir = "~/.herald"   # Data directory
retention_seconds = 86400 # Keep for 24 hours (default)

[ai]
default_provider = "claude"  # "claude" or "gemini"
model = "claude-3-5-sonnet-20241022"
```

## AI Setup

Copy `.env.example` to `.env` and add your API keys:

```bash
cp .env.example .env
# Edit .env with your API keys
```

## Usage

```bash
# Start the daemon
herald daemon start

# Stop the daemon
herald daemon stop

# Manual capture
herald capture

# List available displays
herald displays

# Check status
herald status

# Get AI suggestions
herald suggest "How can I improve my workflow?"
```

## Architecture

Herald follows Hexagonal Architecture (Ports & Adapters):

```
herald-cli/       # CLI entry point
herald-core/      # Domain logic and port definitions
herald-adapters/  # Infrastructure implementations
```

## License

MIT License - see [LICENSE](LICENSE) for details.
