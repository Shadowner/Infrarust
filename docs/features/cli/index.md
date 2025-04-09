# Infrarust CLI Documentation

## Overview

Infrarust features a built-in command-line interface (CLI) for managing and monitoring your Minecraft proxy. The CLI provides real-time control over connected players, server configurations, and security features without requiring server restarts.

## Table of Contents

- [Common Commands](common.md)
- [Ban System Commands](ban.md)
- [Diagnostic Commands](debug.md)

## Getting Started

When running Infrarust, the CLI is automatically available and provides a prompt (`>`) where you can enter commands:

```
> help
```

To see a list of all available commands, use the `help` command. For detailed information about specific commands, refer to the appropriate command page in this documentation.

## Command Summary

| Command | Description | Documentation |
|---------|-------------|---------------|
| `list` | Lists all connected players by server | [Common Commands](common.md) |
| `kick` | Kicks a player from the server | [Common Commands](common.md) |
| `configs` | Lists all server configurations | [Common Commands](common.md) |
| `ban` | Bans a player | [Ban System Commands](ban.md) |
| `unban` | Removes a ban | [Ban System Commands](ban.md) |
| `bans` | Lists all active bans | [Ban System Commands](ban.md) |
| `debug` | Shows detailed debug information | [Diagnostic Commands](debug.md) |
| `tasks` | Shows background task information | [Diagnostic Commands](debug.md) |
| `help` | Shows help information | [Common Commands](common.md) |
| `exit`/`quit` | Exits the program | [Common Commands](common.md) |

## Colored Output

Infrarust CLI uses colored output to improve readability:
- Green: Success messages, headers
- Cyan: Entity names (players, servers), configuration names
- Yellow: Warnings
- Red: Errors
- Gray: Secondary information, IDs
- Bold: Labels, important information

## Non-Interactive Mode

Infrarust can also accept commands from standard input in non-interactive mode, which is useful for scripting or when running in a container. When running in non-interactive mode, the command prompt `>` is not displayed.

**Example (using echo and pipe):**
```bash
echo "list" | ./infrarust
```

This allows you to automate commands or create management scripts for your Infrarust server.
