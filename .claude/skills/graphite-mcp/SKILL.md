---
name: graphite-mcp
description: Install, configure, verify, and troubleshoot the Graphite MCP server so an agent host such as Claude Code, Codex, Cursor, VS Code, or Gemini CLI can drive the Graphite editor. Use when the user wants to connect Graphite to their agent, when Graphite MCP tools are missing or failing, or when an MCP handshake and tool list need verifying.
when_to_use: Trigger phrases include "install graphite mcp", "connect graphite to claude code", "add graphite to my mcp servers", "point claude at this repo", "the graphite mcp tools are missing", "why won't the graphite mcp server start", and "verify the graphite mcp server".
argument-hint: "[client] [root-path]"
---

# Graphite MCP setup and diagnosis

`graphite-agent` is an MCP server that exposes the Graphite node-graph editor over stdio.
It is built in this repository under `agent/`. Optional arguments: `$ARGUMENTS`
(an optional client name, then an optional confinement root path).

## The authority

Read `agent/INSTALL.md` before doing anything. It is **generated** from the same table as
the CLI, so it cannot drift, and a test fails if it does. Do not reproduce its content from
memory and do not hand-write a client config: run `graphite-agent print-config` instead.
For the current flags, run `graphite-agent print-config --help`.

The command surface, which `agent/INSTALL.md` explains in full:

- `print-config` — emits a client's configuration and **writes nothing**. Formats: native,
  `json`, `shell`, `path`, and `markdown`.
- `install` — merges into the client's file with an atomic write and a timestamped backup.
  Writes **only** with `--yes`; `--dry-run` previews; `--uninstall` removes only our entry.
- `doctor` — checks the binary, the root, and the node catalog, then starts the real server
  and completes an MCP `initialize` handshake. `--json` for machine-readable output,
  `--no-handshake` to skip starting the server.

## Diagnose before you change anything

1. `graphite-agent doctor --json` and read the result. Judge success only by a passing
   `mcp handshake` check with a non-zero tool count. A well-formed config file is not
   evidence: the handshake is what proves the server starts, links its node catalog, and
   serves tools.
2. If the handshake fails, the usual causes are a `command` path a GUI client cannot
   resolve (use an absolute path), a `--root` that is missing or not writable, or a cold
   start that exceeded the host's startup timeout.
3. Report the raw output and name which check failed and why. Do not paper over an
   environmental failure.

## Install, with confirmation before writing

Building and diagnosing are safe. **Writing client configuration is not**, so never run
`install` without explicit confirmation from the user. Propose the exact command, show the
`--dry-run` output, and wait.

```sh
cargo build -p graphite-agent-cli --release   # never bare `cargo run` at the repo root
graphite-agent print-config --client claude-code --scope project   # read only
graphite-agent install --client claude-code --scope project --dry-run
graphite-agent install --client claude-code --scope project --yes   # only after approval
```

If the user wants this available outside this repository, use `--scope user`. For Claude
Code that delegates to `claude mcp add-json` instead of editing `~/.claude.json`, because
that file holds private state and credentials.

## Hard rules

- **Never hand-edit `~/.claude.json` or any `.toml` file.** `install` refuses both on
  purpose: the first is private state, and this workspace has no TOML parser, so a merge
  could corrupt the file. For Codex, use its own `codex mcp add`, or paste the table that
  `print-config` emits.
- **Never write a config without `--yes`,** and never invent a location for a client whose
  path is not documented. `print-config` reports manual steps for those instead.
- Keep `--root` deliberate. It is the confinement boundary for every file-path tool, so
  say out loud which directory you configured and why.

## Traps that break a working setup

- **VS Code uses `servers`; every other JSON client uses `mcpServers`.** This is the most
  common copy-paste failure.
- **Claude Code connects MCP servers at startup.** A newly added server may not appear
  until the session restarts, and a project-scoped `.mcp.json` server shows as
  `Pending approval` until the workspace is trusted. A cloned repository cannot approve
  its own servers.
- **Codex defaults `startup_timeout_sec` to 10.** Our host builds an `Editor` at startup,
  so the generated Codex table raises those timeouts; do not drop them.
- **Gemini CLI was superseded by Antigravity CLI** (2026-06-18) for unpaid and Google One
  users. Support it only if the user is still on it.
- **The three large-output tools** (`node.list_types`, `graph.list_nodes`, `render.preview`)
  carry a `_meta` size annotation because their text output can exceed a host's result cap.
  If a host truncates or writes results to disk, that annotation is the lever.
- **`--http` does not stream events.** `notifications/message` events are stdio-only.

## Report

Finish with: what changed on disk, the backup path if `install` wrote anything, the raw
`doctor` result, and the exact commands the user must run themselves — including the
restart or approval step and `claude mcp list` as the confirmation.
