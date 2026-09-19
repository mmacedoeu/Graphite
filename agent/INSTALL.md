# Installing the Graphite MCP server

<!-- GENERATED FILE - DO NOT EDIT.
     Regenerate with: cargo run -p graphite-agent-cli -- print-config --client all --format markdown > agent/INSTALL.md
     A test compares this file against that command, so hand edits fail the suite. -->

`graphite-agent` speaks MCP over stdio, so every host CLI can run it. The clients differ only in
where they keep the server list and what they call the wrapper key.

## 1. Get the binary

```sh
cargo build -p graphite-agent-cli --release
# or, to put it on PATH:
cargo install --path agent/cli
```

Use an absolute path in the configuration if the host is a GUI app that does not inherit your
shell `PATH` (Cursor and Claude Desktop are the common cases).

## 2. Let the server print its own configuration

```sh
graphite-agent print-config --client <client> --scope project
```

That command writes nothing. Add `--dry-run` to preview, or `--yes` to write, with:

```sh
graphite-agent install --client <client> --scope project --yes
graphite-agent doctor
```

`install` merges into any existing file, leaves every other server in place, and keeps a
timestamped backup of the previous contents before it writes. It is idempotent, and
`install --uninstall --yes` removes only our entry.

## 3. Per-client configuration

| Client | Wrapper key | Project file | User file |
|---|---|---|---|
| Claude Code | `mcpServers` | `<PROJECT>/.mcp.json` | Command Palette |
| Codex CLI | - | `<PROJECT>/.codex/config.toml` | `<HOME>/.codex/config.toml` |
| VS Code | `servers` | `<PROJECT>/.vscode/mcp.json` | Command Palette |
| Cursor | `mcpServers` | `<PROJECT>/.cursor/mcp.json` | `<HOME>/.cursor/mcp.json` |
| Gemini CLI | `mcpServers` | `<PROJECT>/.gemini/settings.json` | `<HOME>/.gemini/settings.json` |

### Claude Code

**Project scope**

File: `<PROJECT>/.mcp.json`

```json
{
  "mcpServers": {
    "graphite": {
      "args": [
        "--mode",
        "headless",
        "--stdio",
        "--root",
        "/absolute/path/to/art"
      ],
      "command": "/absolute/path/to/graphite-agent",
      "env": {},
      "type": "stdio"
    }
  }
}
```

Or use the client's own command:

```sh
claude mcp add --scope project --transport stdio graphite -- /absolute/path/to/graphite-agent --mode headless --stdio --root /absolute/path/to/art
```

**User scope**

```json
{
  "args": [
    "--mode",
    "headless",
    "--stdio",
    "--root",
    "/absolute/path/to/art"
  ],
  "command": "/absolute/path/to/graphite-agent",
  "env": {},
  "type": "stdio"
}
```

Or use the client's own command:

```sh
claude mcp add-json graphite '{"args":["--mode","headless","--stdio","--root","/absolute/path/to/art"],"command":"/absolute/path/to/graphite-agent","env":{},"type":"stdio"}' --scope user
```

- Run the command above; it appends the server to `~/.claude.json` without touching the rest.

This file is **not** edited by `graphite-agent install`.

### Codex CLI

> We never rewrite `config.toml`; the official `codex mcp add` command does.

**Project scope**

File: `<PROJECT>/.codex/config.toml`

```toml
[mcp_servers.graphite]
command = "/absolute/path/to/graphite-agent"
args = ["--mode", "headless", "--stdio", "--root", "/absolute/path/to/art"]
startup_timeout_sec = 60
tool_timeout_sec = 300
enabled = true
```

Or use the client's own command:

```sh
codex mcp add graphite -- /absolute/path/to/graphite-agent --mode headless --stdio --root /absolute/path/to/art
```

- Codex has no TOML parser here, so we never rewrite the file: run the command, or paste the table into the file shown.

This file is **not** edited by `graphite-agent install`.

**User scope**

File: `<HOME>/.codex/config.toml`

```toml
[mcp_servers.graphite]
command = "/absolute/path/to/graphite-agent"
args = ["--mode", "headless", "--stdio", "--root", "/absolute/path/to/art"]
startup_timeout_sec = 60
tool_timeout_sec = 300
enabled = true
```

Or use the client's own command:

```sh
codex mcp add graphite -- /absolute/path/to/graphite-agent --mode headless --stdio --root /absolute/path/to/art
```

- Codex has no TOML parser here, so we never rewrite the file: run the command, or paste the table into the file shown.

This file is **not** edited by `graphite-agent install`.

### VS Code

> VS Code spells the wrapper key `servers`; every other JSON client uses `mcpServers`.

**Project scope**

File: `<PROJECT>/.vscode/mcp.json`

```json
{
  "servers": {
    "graphite": {
      "args": [
        "--mode",
        "headless",
        "--stdio",
        "--root",
        "/absolute/path/to/art"
      ],
      "command": "/absolute/path/to/graphite-agent",
      "env": {},
      "type": "stdio"
    }
  }
}
```

**User scope**

```json
{
  "args": [
    "--mode",
    "headless",
    "--stdio",
    "--root",
    "/absolute/path/to/art"
  ],
  "command": "/absolute/path/to/graphite-agent",
  "env": {},
  "type": "stdio"
}
```

- Run "MCP: Open User Configuration" from the Command Palette, or "MCP: Add Server".
- Choose stdio, then enter the command and arguments shown above.

This file is **not** edited by `graphite-agent install`.

### Cursor

**Project scope**

File: `<PROJECT>/.cursor/mcp.json`

```json
{
  "mcpServers": {
    "graphite": {
      "args": [
        "--mode",
        "headless",
        "--stdio",
        "--root",
        "/absolute/path/to/art"
      ],
      "command": "/absolute/path/to/graphite-agent",
      "env": {},
      "type": "stdio"
    }
  }
}
```

**User scope**

File: `<HOME>/.cursor/mcp.json`

```json
{
  "mcpServers": {
    "graphite": {
      "args": [
        "--mode",
        "headless",
        "--stdio",
        "--root",
        "/absolute/path/to/art"
      ],
      "command": "/absolute/path/to/graphite-agent",
      "env": {},
      "type": "stdio"
    }
  }
}
```

### Gemini CLI

> Superseded upstream by Antigravity CLI; supported here for the users still on it.

**Project scope**

File: `<PROJECT>/.gemini/settings.json`

```json
{
  "mcpServers": {
    "graphite": {
      "args": [
        "--mode",
        "headless",
        "--stdio",
        "--root",
        "/absolute/path/to/art"
      ],
      "command": "/absolute/path/to/graphite-agent",
      "env": {},
      "type": "stdio"
    }
  }
}
```

**User scope**

File: `<HOME>/.gemini/settings.json`

```json
{
  "mcpServers": {
    "graphite": {
      "args": [
        "--mode",
        "headless",
        "--stdio",
        "--root",
        "/absolute/path/to/art"
      ],
      "command": "/absolute/path/to/graphite-agent",
      "env": {},
      "type": "stdio"
    }
  }
}
```

## 4. Replace the paths

- `/absolute/path/to/graphite-agent`: the absolute path to `graphite-agent` (use
  `command -v graphite-agent`, or the path under `target/release/`). Claude Code also
  expands `${GRAPHITE_AGENT_BIN:-graphite-agent}` inside a project `.mcp.json`, which is how
  the committed repo configuration stays machine-independent.
- `/absolute/path/to/art`: the confinement root. Every file-path tool is refused outside it
  (INV-12), so open, restore, and export only touch files under this directory.

## 5. Security notes

- A project-scoped `.mcp.json` is committed, so it is code: Claude Code shows a project server
  as pending until you approve the workspace, and a cloned repository cannot approve itself.
- `--capabilities` narrows the grant set per client. Attached and peer sessions are already
  attenuated to their mode ceiling and refuse anything above it.
- `--root` is the only filesystem boundary. Do not point it at a directory you would not let
  the agent read and write.
- Gemini CLI prompts per tool call unless the server is marked trusted; this guide does not
  set `trust`, because that would silently auto-approve every call.

## 6. When it does not connect

```sh
graphite-agent doctor
```

`doctor` checks the binary is absolute and exists, that the root is a writable directory, that
the generated node catalog is non-empty, and that each client's configuration mentions us. It
then performs a real MCP `initialize` handshake by starting the server, unless you pass
`--no-handshake`. Add `--json` for machine-readable output.
