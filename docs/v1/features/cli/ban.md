# Ban System Commands

This page documents commands related to player banning and security management in Infrarust.

## ban

Bans a player by IP address, username, or UUID.

**Usage:**
```
> ban [--ip/-ip <address> | --username/-u <username> | --uuid/-id <uuid>] [--reason <reason>] [--duration <duration>]
```

**Parameters:**
- `--ip` or `-ip`: The IP address to ban
- `--username` or `-u`: The username to ban
- `--uuid` or `-id`: The UUID to ban
- `--reason`: The reason for the ban (optional, defaults to "Banned by administrator")
- `--duration`: The duration of the ban (optional, defaults to permanent)

**Duration Format:**
- `Xs`: X seconds
- `Xm`: X minutes
- `Xh`: X hours
- `Xd`: X days
- `Xw`: X weeks
- `Xmo`: X months
- `Xy`: X years

**Examples:**
```
> ban --ip 192.168.1.10 --reason "Spamming" --duration 2d
Ban applied successfully:
  IP: 192.168.1.10
  Reason: Spamming
  Duration: 2 days

> ban --username Steve --reason "Griefing"
Ban applied successfully:
  Username: Steve
  Reason: Griefing
  Duration: Permanent

> ban --uuid 7f8d3a2e-9c5b-4b1d-8a7c-3d2f6e9a1b5c --duration 1w
Ban applied successfully:
  UUID: 7f8d3a2e-9c5b-4b1d-8a7c-3d2f6e9a1b5c
  Reason: Banned by administrator
  Duration: 1 week
```

**Notes:**
- At least one identifier (IP, username, or UUID) is required
- The ban filter must be enabled in your configuration for this command to work

## unban

Removes a ban by IP address, username, or UUID.

**Usage:**
```
> unban [--ip/-ip <address> | --username/-u <username> | --uuid/-id <uuid>]
```

**Parameters:**
- `--ip` or `-ip`: The IP address to unban
- `--username` or `-u`: The username to unban
- `--uuid` or `-id`: The UUID to unban

**Examples:**
```
> unban --ip 192.168.1.10
Successfully removed ban for IP: 192.168.1.10

> unban --username Steve
Successfully removed ban for username: Steve

> unban --uuid 7f8d3a2e-9c5b-4b1d-8a7c-3d2f6e9a1b5c
Successfully removed ban for UUID: 7f8d3a2e-9c5b-4b1d-8a7c-3d2f6e9a1b5c
```

**Notes:**
- You must specify exactly one identifier (IP, username, or UUID)
- If no ban exists for the specified identifier, you'll receive a warning
- The ban filter must be enabled in your configuration for this command to work

## bans

Lists all active bans.

**Usage:**
```
> bans
```

**Output Example:**
```
=== Active Bans (2) ===

1. IP: 192.168.1.10
   Reason: Spamming
   Banned by: console
   Created: 2 hours ago
   Expires: In 1 day, 22 hours (in 1 day)

2. Username: Griefer123
   Reason: Griefing
   Banned by: console
   Created: 3 days ago
   Expires: Never (permanent ban)
```

**Notes:**
- If no bans are active, you'll see a message indicating that
- The ban filter must be enabled in your configuration for this command to work
