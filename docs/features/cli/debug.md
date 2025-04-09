# Diagnostic Commands

This page documents commands used for diagnostics and troubleshooting in Infrarust.

## debug

Shows detailed debug information about active actors and tasks.

**Usage:**
```
> debug
```

**Output Example:**
```
=== Actor and Task Debug Information ===

Config hub.minecraft.example.com - 2 actors
  1. Steve - Session: 7f8d3a2e-9c5b-4b1d-8a7c-3d2f6e9a1b5c - Age: 2h 15m 30s - ACTIVE
  2. Alex - Session: 6e7c2b1d-8a5e-3b9f-7c8d-2e1a4b3c5d6e - Age: 45m 12s - ACTIVE

Config survival.minecraft.example.com - 2 actors
  1. Notch - Session: 5d4c3b2a-1e9f-8d7c-6b5e-4a3c2d1e9f8a - Age: 1h 5m 42s - ACTIVE
  2. <status> - Session: 4c3b2a1d-0f9e-8d7c-6b5e-4a3c2d1e9f8a - Age: 3h 20m 15s - ACTIVE #Should never be this much aged

Current process memory usage: 42.75 MB

=== Total Actors: 4 ===
```

**Notes:**
- This command is primarily for diagnostic purposes
- Memory usage information is only available on certain platforms
- `<status>` actors are used for server status checking

## tasks

Shows detailed information about background tasks and their status.

**Usage:**
```
> tasks
```

**Output Example:**
```
=== Task Monitor ===

Summary: 8 total, 5 running, 3 completed, 0 orphaned

Config hub.minecraft.example.com - 4 tasks, 2 actors - Healthy
  Tasks:
    3 running, 1 completed

Config survival.minecraft.example.com - 4 tasks, 2 actors - Healthy
  Tasks:
    2 running, 2 completed
```

**Notes:**
- This command is useful for monitoring the health of background tasks
- If there are orphaned tasks, a warning will be displayed
- For configs with potential issues, detailed task lists will be shown
