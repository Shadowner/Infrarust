
# Common Commands

This page documents the essential commands for day-to-day management of your Infrarust server.

## list

Lists all connected players across all servers.

**Usage:**
```
> list
```

**Output Example:**
```
=== Connected Players ===

Server hub.minecraft.example.com (2 players)
  1. Steve - 192.168.1.10 (session: 7f8d3a2e-9c5b-4b1d-8a7c-3d2f6e9a1b5c)
  2. Alex - 192.168.1.11 (session: 6e7c2b1d-8a5e-3b9f-7c8d-2e1a4b3c5d6e)

Server survival.minecraft.example.com (1 player)
  1. Notch - 192.168.1.12 (session: 5d4c3b2a-1e9f-8d7c-6b5e-4a3c2d1e9f8a)

=== Total Players (3) ===
```

## kick

Kicks a player from the server.

**Usage:**
```
> kick <username> [server-id]
```

**Parameters:**
- `username`: The username of the player to kick
- `server-id` (optional): The specific server configuration ID if multiple servers have players with the same username

**Examples:**
```
> kick Steve
Kicked player 'Steve' from server 'hub.minecraft.example.com'.

> kick Notch survival.minecraft.example.com
Kicked player 'Notch' from server 'survival.minecraft.example.com'.
```

**Notes:**
- If multiple players with the same username exist across different servers, you'll be prompted to specify a server ID.

## configs

Lists all server configurations currently loaded.

**Usage:**
```
> configs
```

**Output Example:**
```
=== Server Configurations (2 total) ===

hub.minecraft.example.com
  Domains: hub.minecraft.example.com
  Addresses: localhost:25566
  Proxy Mode: Passthrough
  Proxy Protocol: Disabled

survival.minecraft.example.com
  Domains: survival.minecraft.example.com
  Addresses: localhost:25567
  Proxy Mode: Offline
  Proxy Protocol: Disabled
```

## help

Shows the help message with all available commands.

**Usage:**
```
> help
```

**Output Example:**
```
=== Available Commands ===

  list - Lists all connected players by server
  kick - Kicks a player from the server. Usage: kick <username> [server-id]
  configs - Lists all server configurations
  ban - Bans a player by IP, username, or UUID. Use --ip/-ip, --username/-u, or --uuid/-id flags.
  unban - Removes a ban by IP address, username, or UUID. Use --ip, --username, or --uuid flags.
  bans - Lists all active bans
  debug - Shows detailed debug information about active actors and tasks
  tasks - Shows detailed information about background tasks and their status
  help - Show this help message
  exit/quit - Exit the program
```

## exit/quit

Exits the Infrarust server.

**Usage:**
```
> exit
```
or
```
> quit
```

**Notes:**
- These commands initiate a graceful shutdown of the server
- All connected players will be disconnected
- Configuration changes will be saved
