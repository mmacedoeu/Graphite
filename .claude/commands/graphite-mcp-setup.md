---
description: Install or repair the Graphite MCP server for Claude Code in this repo
argument-hint: "[root-path] [project|user]"
disable-model-invocation: true
---

Set up the Graphite MCP server for Claude Code, working from this repository.

Optional arguments: `$ARGUMENTS` (an optional confinement root path, then an optional
scope of `project` or `user`). If nothing is given, use project scope and the repo default.

Start by reading `agent/INSTALL.md`. That file is generated from the same table as the
CLI, so it cannot go stale — it is the authority for the file each client reads and the
wrapper key each one expects. Run `graphite-agent print-config --help` for the current
flags rather than trusting any flag list quoted in prose.

1. **Build the binary.** Never run bare `cargo run` at the repo root; it invokes
   `tools/cargo-run`. Always pass `-p`.

   ```sh
   cargo build -p graphite-agent-cli --release
   ```

2. **Resolve the command path.** If `graphite-agent` is not on `PATH`, decide between
   `cargo install --path agent/cli` and exporting `GRAPHITE_AGENT_BIN`. A GUI client does
   not inherit your shell `PATH`, so prefer an absolute path. Tell me the exact value.

3. **Prove it works before touching config.**

   ```sh
   graphite-agent doctor --json
   ```

   Show me the raw JSON. Fix every check reported as failed. `doctor` completes a real MCP
   `initialize` handshake, so a passing `mcp handshake` check with a non-zero tool count is
   the only acceptable evidence — do not infer success from a config file that looks right.
   If a failure is environmental rather than something you can fix, say so plainly.

4. **Configure.**

   - *project scope (default):* this repo already commits a `.mcp.json` for project scope.
     Compare it against `graphite-agent print-config --client claude-code --scope project`
     and change it only if it is stale or if I passed a different root.
   - *user scope:* run `graphite-agent install --client claude-code --scope user --yes`,
     which delegates to `claude mcp add-json` rather than editing my private state file.

   If I passed a root path, pass it as `--root`. That directory is the confinement
   boundary for every file-path tool, so state clearly which directory you chose.

5. **Respect the two hard rules.** Never hand-edit `~/.claude.json` or any `.toml` file.
   `install` refuses both deliberately: one is private state, and this workspace has no
   TOML parser. Use `install`, `print-config`, or the client's own `mcp add` command.

6. **Report back.** Do not claim success until the handshake passes. Finish with the exact
   commands I must run myself, including the approval or restart step, and how to confirm:

   ```sh
   claude mcp list
   ```

Expect `graphite  ✔ Connected`. Claude Code connects MCP servers at startup, so a newly
added server may not appear until the session restarts, and a project `.mcp.json` server
shows as `Pending approval` until the workspace is trusted.
