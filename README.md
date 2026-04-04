# llama-chat

A Claude Code-like TUI for talking to local and networked LLMs through the OpenAI-compatible API. Built in Rust with ratatui.

```
llama-chat                                         [gemma4:latest] [Spark] [5 tools]
────────────────────────────────────────────────────────────────────────────────────
you: What files are in the current directory?

⚙ list_files {"path":"."} ✓ allowed
Cargo.toml  src/  tests/  README.md

gemma4:latest: You have 4 entries in the current directory: Cargo.toml, src/,
tests/, and README.md.

you: Create a hello world at /tmp/hello.rs

⚙ write_file {"path":"/tmp/hello.rs","content":"fn main() ..."} ✓ allowed
Wrote 42 bytes to /tmp/hello.rs

gemma4:latest: Done — wrote a hello world program to /tmp/hello.rs.
────────────────────────────────────────────────────────────────────────────────────
▸ Type a message...                              /help · /model · /server
```

## Features

- **Multi-server** — configure multiple Ollama/llama.cpp backends, switch with `/server`
- **Streaming** — token-by-token response rendering, live shell output line-by-line
- **Tool calling** — shell, read_file, write_file, edit_file, list_files with permission prompts
- **MCP support** — stdio, SSE, and streamable HTTP transports with auto-detection
- **Thinking mode** — parses `<think>` tags for models that support reasoning (gemma4, etc.)
- **Permissions** — allow/deny/save-always/pattern prompts for shell; filesystem tools auto-allowed per session; `--yolo` to skip all prompts
- **Skills** — markdown files with frontmatter, global (`~/.config/llama-chat/skills/`) and per-project (`.llama-chat/skills/`)
- **Project context** — loads CLAUDE.md, AGENTS.md, Cursor `.cursor/rules/*.mdc`, and `.llama-chat/context.md` as system prompts
- **Themes** — dark/light presets with per-color hex overrides in config

## Install

```sh
cargo install --path .
```

Requires Rust 1.85+ (edition 2024).

## Configuration

Config lives at `~/.config/llama-chat/`:

```toml
# config.toml
[servers.local]
name = "Local Ollama"
url = "http://localhost:11434/v1"

[servers.remote]
name = "GPU Box"
url = "http://gpu-box:8080/v1"
api_key = "sk-your-token-here"

[defaults]
server = "local"
model = "llama3:8b"

[theme]
preset = "dark"  # or "light"

[theme.colors]  # optional overrides
accent = "#818cf8"
tool_ok = "#34d399"
```

### MCP Servers

```json
// ~/.config/llama-chat/mcp.json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/home/user"]
    },
    "remote": {
      "url": "http://mcp-server:3001/sse"
    }
  }
}
```

Transport is auto-detected: `command` field means stdio, `url` field tries streamable HTTP then falls back to SSE. Set `"transport": "sse"` to override.

## Usage

```sh
llama-chat          # normal mode
llama-chat --yolo   # skip all permission prompts
```

### Slash Commands

| Command | Action |
|---------|--------|
| `/model [name]` | Switch model or list available |
| `/server [name]` | Switch server or list configured |
| `/tools` | List active tools |
| `/skills` | List available skills |
| `/init` | Generate AGENTS.md for the project |
| `/clear` | Clear conversation |
| `/help` | Show commands |
| `/exit` | Quit |

Skills are invoked by name: `/review`, `/explain`, etc.

### Keybindings

| Key | Action |
|-----|--------|
| `Enter` | Send message |
| `Ctrl+C` / `Esc` | Quit |
| `A` | Allow tool call |
| `D` | Deny tool call |
| `S` | Save always (persist to permissions.json) |
| `P` | Save a glob pattern |

### Per-Project Config

```
.llama-chat/
├── permissions.json   # saved allow rules
├── context.md         # project-specific system prompt
└── skills/            # project-specific skills
```

The app also reads `CLAUDE.md`, `AGENTS.md`, and `.cursor/rules/*.mdc` from the project root for compatibility with other AI tools.

## Architecture

Event-driven async app with tokio. Three input sources feed a central mpsc channel:

```
Terminal input (crossterm) ──┐
API stream (reqwest SSE)   ──┼──▸ Event Channel ──▸ App State Machine ──▸ ratatui render
MCP clients (stdio/http)   ──┘
```

## License

MIT
