---
title: WIT API Reference
description: "The infrarust:plugin@0.3.0 WIT contract: package, world, types, events, host imports, and guest exports."
outline: [2, 3]
---

# WIT API Reference

This page documents the low-level WIT contract that defines the host/guest boundary for Infrarust WASM plugins. It lists every interface, type, and signature in the contract.

You rarely call these signatures directly. The SDK generates bindings from this contract and presents typed Rust wrappers on top of them, so the author-facing API lives on the per-feature pages: [Events](./events), [Services](./services), [Commands](./commands), [Limbo](./limbo), and [Codec Filters](./codec-filters). Read this page when you need the exact wire shape behind a wrapper.

::: info
The contract is `infrarust:plugin@0.3.0`. `WORLD_VERSION` in `infrarust-plugin-wit` matches the package declaration in `wit/world.wit`, and a test keeps the two in step. Components built for `infrarust:plugin@0.2.3` are refused at discovery; see [Migrating to 0.3](./migration-0.3).
:::

## Package and world

The contract is one package split across files under `crates/infrarust-plugin-wit/wit/`: `world.wit`, `types.wit`, `events.wit` (types only), `event-bus.wit`, `players.wit`, `text.wit`, `services.wit`, `permissions.wit`, `providers.wit`, `limbo.wit`, `codec-filter.wit` and `guest.wit`.

```wit
package infrarust:plugin@0.3.0;

world plugin {
    import types;
    import events;
    import log;
    import text;
    import event-bus;
    import players;
    import server-manager;
    import ban-service;
    import config-service;
    import command-manager;
    import scheduler;
    import limbo;
    import codec-registry;
    import load-balancer;
    import messaging;
    import proxy-info;
    import plugin-registry;
    import permissions;
    import providers;

    export guest;
    export codec-filter;
}
```

### Imports

| Import | Capability gate | Purpose |
|--------|-----------------|---------|
| `types` | none | Shared ids, errors, profiles, addresses and the text-component arena. |
| `events` | none | The event records, results and the `event` / `event-outcome` variants. Types only. |
| `log` | none | `trace`/`debug`/`info`/`warn`/`error` to the host log. |
| `text` | none | Parse and serialize text components with the proxy's own parser. |
| `event-bus` | `event-bus` (`chat-intercept` for `chat-message` and `command-execute`, `plugin-messaging` for `plugin-message`, `raw-packet` for packets) | Subscribe to event kinds, named events and packets by priority; fire named events. |
| `players` | `player-read`, `player-write` for actions, `raw-packet` for `send-packet` | Look up players by id, name or UUID, and act on them by id. |
| `server-manager` | `server-manage` | Read server state, start and stop backends. |
| `ban-service` | `ban` | Create, remove, look up and page bans. |
| `config-service` | `config-read`, `config-write` for `write-proxy-config-document` | Read config values, server configs and documents; rewrite the proxy config file. |
| `load-balancer` | `config-read`, `server-manage` for `set-drained` and `reset-backend` | Read the balancing strategy and backend health; drain and reset backends. |
| `messaging` | `plugin-messaging` | Register plugin channels and send plugin messages to clients, backends and servers. |
| `proxy-info` | none | The proxy's version, bind address and limits, and the capabilities the plugin holds. |
| `plugin-registry` | none | Read-only list of the loaded plugins. |
| `command-manager` | `command` | Register and unregister proxy commands. |
| `scheduler` | `scheduler` | Schedule one-shot and repeating callbacks. |
| `limbo` | `limbo` for `register-limbo-handler` | Register limbo handlers and act on the session resources. |
| `codec-registry` | `codec-filter` | Register and unregister codec filters. |
| `permissions` | `permission-provider` | Replace or clear the permission snapshot the host holds for a player. |
| `providers` | `ban-provider` for `register-ban-provider`, `permission-provider` for `register-permission-provider` | Become the proxy's ban provider or permission provider. |

Every interface is linked for every plugin. A call the plugin lacks the capability for is refused when it is made: it returns a `host-error` of kind `permission-denied`, or a neutral value for the few infallible reads listed under [Players](#players). See [Capabilities](./capabilities).

### Exports

| Export | Purpose |
|--------|---------|
| `guest` | Metadata, lifecycle, the unified event dispatch, and every callback entry point. |
| `codec-filter` | The per-session `filter-instance` resource and its `create` factory. |

### Version check

Before it instantiates a component, the loader reads the name of its `infrarust:plugin/guest@X.Y.Z` export. It accepts `0.3.M` when `M` is at most the host's patch version. Anything else is refused with a message such as:

```text
plugin built for infrarust:plugin@0.2.3; this host supports infrarust:plugin@0.3.x, rebuild it with an infrarust-plugin-sdk that targets infrarust:plugin@0.3.x
```

A component that exports no `guest` interface of this package is refused as "not an Infrarust plugin component". The AOT compile cache key includes the contract version, so a cached artifact never outlives a contract change.

## Core types

`types.wit` holds everything the other interfaces share.

```wit
interface types {
    type player-id = u64;
    type server-id = string;
    type handler-id = u64;
    type listener-handle = u64;
    type task-handle = u64;
    type duration-ms = u64;
    type timestamp-ms = u64;
    type protocol-version = s32;
    type event-priority = u8;

    record uuid {
        hi: u64,
        lo: u64,
    }

    variant ip-address {
        ipv4(tuple<u8, u8, u8, u8>),
        ipv6(tuple<u16, u16, u16, u16, u16, u16, u16, u16>),
    }

    record socket-address {
        ip: ip-address,
        port: u16,
    }

    enum error-kind {
        invalid-argument,
        not-found,
        permission-denied,
        unavailable,
        timeout,
        player-gone,
        conflict,
        invalid-state,
        unsupported,
        internal,
    }

    record host-error {
        kind: error-kind,
        message: string,
    }

    enum capability {
        event-bus,
        player-read,
        player-write,
        raw-packet,
        server-manage,
        ban,
        command,
        scheduler,
        config-read,
        config-write,
        codec-filter,
        transport-filter,
        limbo,
        virtual-backend,
        permission-provider,
        filesystem-extended,
        network,
        chat-intercept,
        ban-provider,
        plugin-messaging,
    }

    enum connection-state { handshake, status, login, configuration, play }
    enum server-state { online, offline, starting, stopping, sleeping, crashed }
    enum proxy-mode { passthrough, zero-copy, client-only, offline, server-only }
    enum packet-direction { serverbound, clientbound }

    record profile-property {
        name: string,
        value: string,
        signature: option<string>,
    }

    record game-profile {
        uuid: uuid,
        username: string,
        properties: list<profile-property>,
    }

    record player-ref {
        id: player-id,
        uuid: uuid,
        username: string,
    }

    record server-address {
        host: string,
        port: u16,
    }

    record channel-id {
        modern: option<string>,
        legacy: option<string>,
    }

    enum chat-mode { enabled, commands-only, hidden }
    enum main-hand { left, right }
    enum particle-status { all, decreased, minimal }

    flags skin-parts {
        cape,
        jacket,
        left-sleeve,
        right-sleeve,
        left-pants,
        right-pants,
        hat,
    }

    record client-settings {
        locale: string,
        view-distance: u8,
        chat-mode: chat-mode,
        chat-colors: bool,
        skin-parts: skin-parts,
        main-hand: main-hand,
        text-filtering: bool,
        allow-listing: bool,
        particle-status: particle-status,
    }

    record style {
        color: option<string>,
        bold: option<bool>,
        italic: option<bool>,
        underlined: option<bool>,
        strikethrough: option<bool>,
        obfuscated: option<bool>,
        font: option<string>,
        insertion: option<string>,
        shadow-color: option<u32>,
    }

    variant click-event {
        open-url(string),
        run-command(string),
        suggest-command(string),
        copy-to-clipboard(string),
        change-page(s32),
    }

    variant hover-event {
        show-text(u32),
    }

    variant node-content {
        text(string),
        translatable(tuple<string, list<u32>, option<string>>),
        keybind(string),
    }

    record component-node {
        content: node-content,
        style: style,
        click: option<click-event>,
        hover: option<hover-event>,
        children: list<u32>,
    }

    record component {
        nodes: list<component-node>,
    }

    record title-data {
        title: component,
        subtitle: component,
        fade-in-ticks: s32,
        stay-ticks: s32,
        fade-out-ticks: s32,
    }

    record raw-packet {
        packet-id: s32,
        data: list<u8>,
    }

    record plugin-dependency {
        id: string,
        optional: bool,
    }

    record plugin-metadata {
        id: string,
        name: string,
        version: string,
        authors: list<string>,
        description: option<string>,
        dependencies: list<plugin-dependency>,
    }
}
```

### Ids, time and addresses

| Type | Shape | Notes |
|------|-------|-------|
| `player-id` | `u64` | The proxy's id for a connected player. Every player call takes one. |
| `server-id` | `string` | A backend server config id. |
| `handler-id` | `u64` | A guest-owned id behind a command, task, limbo handler or codec factory. |
| `listener-handle` | `u64` | Returned by `event-bus.subscribe`, `subscribe-named` and `subscribe-packets`. |
| `task-handle` | `u64` | Returned by `scheduler.delay` and `scheduler.interval`. |
| `duration-ms`, `timestamp-ms` | `u64` | Milliseconds; timestamps count from the Unix epoch. |
| `uuid` | `record { hi, lo }` | The big-endian halves of the UUID, so a malformed UUID cannot exist. |
| `ip-address`, `socket-address` | variant / record | Octets for IPv4, segments for IPv6. |

### Errors

Every fallible host function returns `result<T, host-error>`. The `kind` is the part to branch on; the `message` is for logs.

| Kind | Raised when |
|------|-------------|
| `invalid-argument` | An argument is malformed: an invalid text component, an unparsable IP range, a bad command name. |
| `not-found` | The thing named does not exist, or the plugin does not own the command it unregisters. Also a codec filter id nobody registered. |
| `permission-denied` | The plugin lacks the capability the call needs. The message names it: `missing capability: ban`. |
| `unavailable` | The service could not do it right now, or host services are not available yet (during `metadata()`). |
| `timeout` | The host call ran out of time, either its own timeout or the deadline of the guest call around it. |
| `player-gone` | The player id is not online any more. |
| `conflict` | The name is taken: a command owned by another plugin, a limbo handler name already registered, a codec filter id owned by another plugin or the proxy (on register and on unregister). |
| `invalid-state` | The player is not in a state that allows the action, such as no backend yet, or the call would wait on the player's own session while that session is waiting on the plugin. |
| `unsupported` | The proxy does not offer the service. |
| `internal` | Anything else. |

A host function traps the guest only on a host invariant bug, such as a stale resource handle.

### Text components

WIT has no recursive types, so a text component crosses the boundary as a flat arena: `nodes[0]` is the root and nodes point at each other by index. A node carries its content (`text`, `translatable` with argument indices and an optional fallback, or `keybind`), its style, an optional click and hover event, and the indices of its children. `hover-event::show-text` holds the index of the tooltip's root.

The receiver validates every arena:

- every index a node references is greater than the node's own index, so there are no cycles;
- every node but the root is referenced exactly once, so a tree cannot fan out exponentially;
- at most 4096 nodes, at most 64 levels deep, at most 256 KiB of text across all strings;
- every `color` is a named color (`gold`, `dark_red`, ...) or `#RRGGBB`.

An invalid component passed to a host function is refused with `invalid-argument`. An invalid component inside an event result still applies the decision, with the text replaced by a fallback, and the host logs a warning. The SDK builds valid arenas for you; see [`Component`](./services#text-components).

Contents the contract does not model (score, selector, NBT, object) reach the guest as plain text, and click events of kind `custom` and hover events that show an item or entity are dropped on the way in.

## Events

`events.wit` declares one record per event and, when the event has a result, the result type. Every event follows the same pattern:

- the record is named `<event>-event`;
- a resulted event's record ends with `result: <event>-result`, which holds the **current** result, as set by earlier handlers;
- `event-kind` has one case per event, `event` has one case per event carrying the record, and `event-outcome` has one case per resulted event carrying its result.

`event-outcome` starts with `unchanged`. Returning it leaves the result as it is. Returning any other case sets the result, even when it equals the current one, so a later handler can reset an earlier deny to allowed. A test in `infrarust-plugin-wit` checks this pattern on every build.

```wit
interface events {
    use types.{
        server-id, uuid, protocol-version, socket-address, game-profile, player-ref,
        component, server-state, server-address, player-id, channel-id, client-settings,
        packet-direction, raw-packet,
    };
    use limbo.{limbo-entry-context};
    use ban-service.{ban-entry, ban-source};
    use permissions.{permission-snapshot};

    enum event-kind {
        pre-login,
        post-login,
        disconnect,
        online-auth-failed,
        permissions-setup,
        player-choose-initial-server,
        server-pre-connect,
        server-connected,
        server-post-connect,
        kicked-from-server,
        chat-message,
        proxy-ping,
        proxy-initialize,
        proxy-shutdown,
        config-reload,
        server-state-change,
        backend-health,
        login,
        game-profile-request,
        command-execute,
        connection-handshake,
        connection-rejected,
        limbo-enter,
        limbo-exit,
        player-client-brand,
        player-settings-changed,
        player-channel-register,
        plugin-message,
        ban-issued,
        ban-revoked,
        plugin-enabled,
        plugin-disabled,
        pre-transfer,
        player-resource-pack-status,
        named-event,
        raw-packet,
    }

    record pre-login-event {
        profile: game-profile,
        remote-addr: socket-address,
        protocol: protocol-version,
        server-domain: string,
        %result: pre-login-result,
    }

    variant pre-login-result {
        allowed,
        denied(component),
        force-offline,
        force-online,
    }

    record post-login-event {
        player: player-ref,
        profile: game-profile,
        protocol: protocol-version,
    }

    variant disconnect-cause {
        client-quit,
        kicked(option<component>),
        backend-closed(option<component>),
        shutdown,
        error,
    }

    record disconnect-event {
        player: player-ref,
        last-server: option<server-id>,
        cause: disconnect-cause,
    }

    record online-auth-failed-event {
        username: string,
    }

    record permissions-setup-event {
        player: player-ref,
        online-mode: bool,
        %result: permissions-setup-result,
    }

    variant permissions-setup-result {
        use-default,
        custom(permission-snapshot),
    }

    record player-choose-initial-server-event {
        player: player-ref,
        initial-server: server-id,
        %result: player-choose-initial-server-result,
    }

    variant player-choose-initial-server-result {
        allowed,
        redirect(server-id),
        send-to-limbo(list<string>),
    }

    enum connect-cause {
        initial,
        switch,
        limbo-exit,
        kick-redirect,
        plugin-message,
    }

    record server-pre-connect-event {
        player: player-ref,
        server: server-id,
        previous-server: option<server-id>,
        cause: connect-cause,
        %result: server-pre-connect-result,
    }

    variant server-pre-connect-result {
        allowed,
        connect-to(server-id),
        send-to-limbo(list<string>),
        denied(component),
    }

    record server-connected-event {
        player: player-ref,
        server: server-id,
        previous-server: option<server-id>,
    }

    record server-post-connect-event {
        player: player-ref,
        server: server-id,
        previous-server: option<server-id>,
    }

    variant kick-cause {
        unreachable(string),
        login-refused,
        config-disconnect,
        play-disconnect,
        connection-lost,
    }

    record kicked-from-server-event {
        player: player-ref,
        server: server-id,
        reason: option<component>,
        cause: kick-cause,
        during-connect: bool,
        previous-server: option<server-id>,
        %result: kicked-from-server-result,
    }

    variant kicked-from-server-result {
        disconnect-player(option<component>),
        redirect-to(server-id),
        send-to-limbo(list<string>),
        notify(component),
    }

    record chat-message-event {
        player: player-ref,
        message: string,
        signed: bool,
        server: option<server-id>,
        %result: chat-message-result,
    }

    variant chat-message-result {
        allow,
        deny(option<component>),
        modify(string),
    }

    record ping-player {
        name: string,
        uuid: uuid,
    }

    record proxy-ping-event {
        remote-addr: socket-address,
        server: option<server-id>,
        virtual-host: option<string>,
        protocol: protocol-version,
        legacy: bool,
        %result: proxy-ping-result,
    }

    record proxy-ping-result {
        description: component,
        max-players: s32,
        online-players: s32,
        protocol: protocol-version,
        version-name: string,
        favicon: option<string>,
        player-sample: list<ping-player>,
    }

    record config-reload-event {
        provider: string,
        added: list<server-id>,
        removed: list<server-id>,
        updated: list<server-id>,
    }

    record server-state-change-event {
        server: server-id,
        old-state: server-state,
        new-state: server-state,
    }

    enum backend-state {
        healthy,
        probing,
        unhealthy,
        draining,
    }

    record backend-health-event {
        address: server-address,
        servers: list<server-id>,
        state: backend-state,
    }

    record login-event {
        player: player-ref,
        online-mode: bool,
        %result: login-result,
    }

    variant login-result {
        allowed,
        denied(component),
    }

    record game-profile-request-event {
        original: game-profile,
        online-mode: bool,
        remote-addr: socket-address,
        virtual-host: option<string>,
        protocol: protocol-version,
        %result: game-profile-request-result,
    }

    record game-profile-request-result {
        profile: game-profile,
    }

    record command-execute-event {
        player: player-ref,
        command: string,
        signed: bool,
        server: option<server-id>,
        %result: command-execute-result,
    }

    variant command-execute-result {
        allow,
        deny(option<component>),
        modify(string),
        forward-to-backend,
    }

    enum handshake-intent {
        status,
        login,
        transfer,
    }

    record connection-handshake-event {
        remote-addr: socket-address,
        virtual-host: option<string>,
        raw-host: string,
        port: u16,
        protocol: protocol-version,
        intent: handshake-intent,
        legacy: bool,
        server: option<server-id>,
        %result: connection-handshake-result,
    }

    variant connection-handshake-result {
        allow,
        deny(option<component>),
        drop-silently,
    }

    variant reject-reason {
        ip-filter,
        rate-limit,
        unknown-domain,
        ip-banned,
        banned,
        server-unavailable,
        plugin(option<string>),
    }

    record connection-rejected-event {
        remote-addr: socket-address,
        virtual-host: option<string>,
        reason: reject-reason,
    }

    record limbo-enter-event {
        player: player-ref,
        handlers: list<string>,
        context: limbo-entry-context,
    }

    variant limbo-exit-reason {
        released,
        redirected,
        sent-to-limbo(list<string>),
        kicked(component),
        disconnected,
        timed-out,
        shutdown,
    }

    record limbo-exit-event {
        player: player-ref,
        reason: limbo-exit-reason,
        next-server: option<server-id>,
    }

    record player-client-brand-event {
        player: player-ref,
        brand: string,
    }

    record player-settings-changed-event {
        player: player-ref,
        settings: client-settings,
    }

    record player-channel-register-event {
        player: player-ref,
        channels: list<string>,
        direction: packet-direction,
    }

    variant message-endpoint {
        client,
        backend(server-id),
    }

    enum message-phase {
        configuration,
        play,
    }

    record plugin-message-event {
        player: player-ref,
        source: message-endpoint,
        channel: channel-id,
        raw-channel: string,
        data: list<u8>,
        phase: message-phase,
        %result: plugin-message-result,
    }

    variant plugin-message-result {
        forward,
        handled,
        replace(list<u8>),
    }

    record ban-issued-event {
        entry: ban-entry,
        source: ban-source,
        silent: bool,
    }

    record ban-revoked-event {
        entry: ban-entry,
        source: ban-source,
        silent: bool,
    }

    record plugin-enabled-event {
        plugin-id: string,
        version: string,
    }

    record plugin-disabled-event {
        plugin-id: string,
    }

    enum transfer-origin {
        plugin,
        backend,
    }

    record pre-transfer-event {
        player: player-ref,
        host: string,
        port: u16,
        origin: transfer-origin,
        %result: pre-transfer-result,
    }

    variant pre-transfer-result {
        allowed,
        denied(component),
        redirect(server-address),
    }

    variant resource-pack-status {
        successfully-loaded,
        declined,
        failed-download,
        accepted,
        downloaded,
        invalid-url,
        failed-reload,
        discarded,
        unknown(s32),
    }

    enum resource-pack-origin {
        proxy,
        backend,
    }

    record player-resource-pack-status-event {
        player: player-ref,
        pack-id: option<uuid>,
        status: resource-pack-status,
        origin: resource-pack-origin,
    }

    record named-event-response {
        content-type: string,
        payload: list<u8>,
    }

    record named-event-event {
        name: string,
        source-plugin: string,
        content-type: string,
        payload: list<u8>,
        %result: named-event-result,
    }

    record named-event-result {
        cancelled: bool,
        response: option<named-event-response>,
    }

    record raw-packet-event {
        player: player-id,
        direction: packet-direction,
        packet: raw-packet,
        %result: raw-packet-result,
    }

    variant raw-packet-result {
        pass,
        modify(raw-packet),
        drop,
    }

    variant event {
        pre-login(pre-login-event),
        post-login(post-login-event),
        disconnect(disconnect-event),
        online-auth-failed(online-auth-failed-event),
        permissions-setup(permissions-setup-event),
        player-choose-initial-server(player-choose-initial-server-event),
        server-pre-connect(server-pre-connect-event),
        server-connected(server-connected-event),
        server-post-connect(server-post-connect-event),
        kicked-from-server(kicked-from-server-event),
        chat-message(chat-message-event),
        proxy-ping(proxy-ping-event),
        proxy-initialize,
        proxy-shutdown,
        config-reload(config-reload-event),
        server-state-change(server-state-change-event),
        backend-health(backend-health-event),
        login(login-event),
        game-profile-request(game-profile-request-event),
        command-execute(command-execute-event),
        connection-handshake(connection-handshake-event),
        connection-rejected(connection-rejected-event),
        limbo-enter(limbo-enter-event),
        limbo-exit(limbo-exit-event),
        player-client-brand(player-client-brand-event),
        player-settings-changed(player-settings-changed-event),
        player-channel-register(player-channel-register-event),
        plugin-message(plugin-message-event),
        ban-issued(ban-issued-event),
        ban-revoked(ban-revoked-event),
        plugin-enabled(plugin-enabled-event),
        plugin-disabled(plugin-disabled-event),
        pre-transfer(pre-transfer-event),
        player-resource-pack-status(player-resource-pack-status-event),
        named-event(named-event-event),
        raw-packet(raw-packet-event),
    }

    variant event-outcome {
        unchanged,
        pre-login(pre-login-result),
        permissions-setup(permissions-setup-result),
        player-choose-initial-server(player-choose-initial-server-result),
        server-pre-connect(server-pre-connect-result),
        kicked-from-server(kicked-from-server-result),
        chat-message(chat-message-result),
        proxy-ping(proxy-ping-result),
        login(login-result),
        game-profile-request(game-profile-request-result),
        command-execute(command-execute-result),
        connection-handshake(connection-handshake-result),
        plugin-message(plugin-message-result),
        pre-transfer(pre-transfer-result),
        named-event(named-event-result),
        raw-packet(raw-packet-result),
    }
}
```

`permissions-setup-result` is `use-default` or `custom(permission-snapshot)`: a custom snapshot becomes the player's checker, held by the host so that `permissions.set-snapshot` can change it later (see [Permissions](./permissions)). A custom checker set by a native plugin reaches the guest as `custom` with the snapshot it describes, or an empty one when the native checker cannot describe itself. `proxy-ping-result` is the whole response: returning it replaces the response, and a description that comes back unchanged keeps the native component untouched. `game-profile-request-result` wraps the profile the player gets, and the record's `original` is the profile the proxy started from. `named-event-result` is the pair the listeners leave behind, `cancelled` and `response`, and returning it replaces both. `raw-packet-event` carries only the player id, like the native `RawPacketEvent`.

`ban-issued` and `ban-revoked` reuse `ban-entry` and `ban-source` from `ban-service`, and `limbo-enter` reuses `limbo-entry-context` from `limbo`. A plugin that only reads those events imports the types, not the functions, so it needs neither `ban` nor `limbo`.

### event-bus

```wit
interface event-bus {
    use types.{event-priority, listener-handle, host-error, connection-state, packet-direction};
    use events.{event-kind, named-event-result};

    record packet-filter {
        packet-id: s32,
        state: connection-state,
        direction: packet-direction,
    }

    subscribe: func(kind: event-kind, priority: event-priority) -> result<listener-handle, host-error>;
    unsubscribe: func(handle: listener-handle) -> result<bool, host-error>;
    subscribe-named: func(name: string, priority: event-priority) -> result<listener-handle, host-error>;
    fire-named: func(name: string, content-type: string, payload: list<u8>) -> result<named-event-result, host-error>;
    subscribe-packets: func(filters: list<packet-filter>, priority: event-priority) -> result<listener-handle, host-error>;
}
```

`subscribe` refuses with `permission-denied` when the plugin lacks `event-bus`, for `chat-message` and `command-execute` when it lacks `chat-intercept`, for `plugin-message` when it lacks `plugin-messaging`, and for `raw-packet` when it lacks `raw-packet`; with `raw-packet` granted it still answers `invalid-argument`, because a packet subscription goes through `subscribe-packets` with its filters. `subscribe` with `named-event` receives every named event, `subscribe-named` only the ones with that name. `unsubscribe` answers whether the handle was subscribed, and removes every filter of a packet subscription.

`fire-named` fires a native `NamedEvent` with the plugin as its `source-plugin`, runs every native and WASM listener in priority order, and answers what they decided. It is bounded like the other waiting calls, see [Slow services and deadlines](./services#slow-services-and-deadlines). The host never enters an instance that is running the call the event came from: a listener of the calling plugin, or of any plugin up the chain of calls that led here, gets the event queued behind its current call, and its outcome is ignored. See [Named events](./events#named-events).

`subscribe-packets` needs `raw-packet` and at least one filter. Each filter registers one native packet listener, so the proxy's per-packet check stays constant time and only the listed packets reach the guest.

## Players

Players are addressed by id; there is no player resource. The five reads are the infallible reads of the contract: without `player-read` they answer `none`, an empty list or `0`, and the host logs the refusal. Every other function returns a `host-error`.

```wit
interface players {
    use types.{
        player-id, server-id, uuid, player-ref, game-profile, protocol-version, socket-address,
        timestamp-ms, component, title-data, raw-packet, host-error, client-settings,
        server-address,
    };

    record player-info {
        player: player-ref,
        profile: game-profile,
        protocol: protocol-version,
        remote-addr: socket-address,
        current-server: option<server-id>,
        online-mode: bool,
        connected: bool,
        active: bool,
        connected-at: timestamp-ms,
        virtual-host: option<string>,
        client-brand: option<string>,
        ping-ms: option<u32>,
        settings: option<client-settings>,
        known-channels: list<string>,
    }

    variant connection-result {
        success,
        already-connected,
        denied(component),
        failed(component),
        cancelled,
    }

    enum boss-bar-color { pink, blue, red, green, yellow, purple, white }
    enum boss-bar-overlay { progress, notched6, notched10, notched12, notched20 }

    flags boss-bar-flags {
        darken-screen,
        play-boss-music,
        create-world-fog,
    }

    record boss-bar {
        title: component,
        progress: f32,
        color: boss-bar-color,
        overlay: boss-bar-overlay,
        %flags: boss-bar-flags,
    }

    variant boss-bar-update {
        title(component),
        progress(f32),
        style(tuple<boss-bar-color, boss-bar-overlay>),
        %flags(boss-bar-flags),
    }

    type boss-bar-id = uuid;

    record resource-pack-request {
        id: uuid,
        url: string,
        hash: option<string>,
        required: bool,
        prompt: option<component>,
    }

    get: func(id: player-id) -> option<player-info>;
    get-by-name: func(username: string) -> option<player-info>;
    get-by-uuid: func(id: uuid) -> option<player-info>;
    %list: func(server: option<server-id>) -> list<player-info>;
    count: func(server: option<server-id>) -> u32;

    send-message: func(player: player-id, message: component) -> result<_, host-error>;
    send-title: func(player: player-id, title: title-data) -> result<_, host-error>;
    send-action-bar: func(player: player-id, message: component) -> result<_, host-error>;
    send-packet: func(player: player-id, packet: raw-packet) -> result<_, host-error>;
    disconnect: func(player: player-id, reason: component) -> result<_, host-error>;
    switch-server: func(player: player-id, server: server-id) -> result<_, host-error>;
    has-permission: func(player: player-id, permission: string) -> result<bool, host-error>;

    connect: func(player: player-id, server: server-id) -> result<connection-result, host-error>;
    set-player-list-header-footer: func(player: player-id, header: component, footer: component) -> result<_, host-error>;
    clear-title: func(player: player-id, reset: bool) -> result<_, host-error>;
    show-boss-bar: func(player: player-id, bar: boss-bar) -> result<boss-bar-id, host-error>;
    update-boss-bar: func(bar: boss-bar-id, update: boss-bar-update) -> result<_, host-error>;
    hide-boss-bar: func(bar: boss-bar-id) -> result<_, host-error>;
    send-resource-pack: func(player: player-id, pack: resource-pack-request) -> result<_, host-error>;
    remove-resource-pack: func(player: player-id, id: option<uuid>) -> result<_, host-error>;
    transfer: func(player: player-id, target: server-address) -> result<_, host-error>;
    store-cookie: func(player: player-id, key: string, data: list<u8>) -> result<_, host-error>;
    request-cookie: func(player: player-id, key: string) -> result<option<list<u8>>, host-error>;
    refresh-permissions: func(player: player-id) -> result<_, host-error>;
}
```

| Function | Capability |
|----------|------------|
| `get`, `get-by-name`, `get-by-uuid`, `list`, `count`, `has-permission` | `player-read` |
| `send-message`, `send-title`, `send-action-bar`, `disconnect`, `switch-server`, `connect`, `set-player-list-header-footer`, `clear-title`, `show-boss-bar`, `update-boss-bar`, `hide-boss-bar`, `send-resource-pack`, `remove-resource-pack`, `transfer`, `store-cookie`, `request-cookie`, `refresh-permissions` | `player-write` |
| `send-packet` | `raw-packet` |

`disconnect` returns once the kick is queued. `switch-server` waits for the session to accept the switch, bounded by a short host timeout and by the guest call's deadline. `connect`, `transfer`, `request-cookie` and `refresh-permissions` wait for their outcome, bounded by `host_call_timeout` and the guest call's deadline. `show-boss-bar` answers the bar's id, which `update-boss-bar` and `hide-boss-bar` take; an id the plugin does not own answers `not-found`. `settings` and `known-channels` in `player-info` are what the client sent, empty until it did.

## Text

Always linked. The host uses the proxy's own parser and serializer, so a component the plugin builds serializes exactly like one built natively.

```wit
interface text {
    use types.{component, host-error};

    parse-json: func(json: string) -> result<component, host-error>;
    parse-legacy: func(legacy: string) -> component;
    to-json: func(value: component) -> result<string, host-error>;
    to-plain: func(value: component) -> result<string, host-error>;
}
```

## Host services

```wit
interface log {
    trace: func(message: string);
    debug: func(message: string);
    info: func(message: string);
    warn: func(message: string);
    error: func(message: string);
}

interface server-manager {
    use types.{server-id, server-state, host-error};

    record server-status {
        server: server-id,
        state: server-state,
    }

    get-state: func(server: server-id) -> result<option<server-state>, host-error>;
    start: func(server: server-id) -> result<_, host-error>;
    stop: func(server: server-id) -> result<_, host-error>;
    %list: func() -> result<list<server-status>, host-error>;
}

interface ban-service {
    use types.{uuid, ip-address, duration-ms, timestamp-ms, host-error, server-id, component};

    variant ban-target {
        ip(ip-address),
        ip-range(string),
        username(string),
        uuid(uuid),
    }

    record ban-request {
        target: ban-target,
        reason: option<string>,
        duration-ms: option<duration-ms>,
        kick: bool,
        silent: bool,
    }

    record ban-entry {
        id: string,
        target: ban-target,
        reason: option<string>,
        source: string,
        created-at: timestamp-ms,
        expires-at: option<timestamp-ms>,
    }

    record ban-page {
        entries: list<ban-entry>,
        next-cursor: option<string>,
    }

    record ban-actor {
        uuid: uuid,
        name: string,
    }

    variant ban-source {
        console,
        player(ban-actor),
        plugin(string),
        web-api(option<string>),
        system,
    }

    enum login-stage {
        status,
        pre-auth,
        post-auth,
    }

    record login-attempt {
        stage: login-stage,
        ip: ip-address,
        username: option<string>,
        uuid: option<uuid>,
        uuid-verified: bool,
        virtual-host: option<string>,
        server: option<server-id>,
    }

    record ban-features {
        ip-ranges: bool,
        pagination: bool,
    }

    record ban-record {
        id: string,
        target: ban-target,
        reason: option<string>,
        source: ban-source,
        created-at: timestamp-ms,
        expires-at: option<timestamp-ms>,
    }

    record ban-verdict {
        entry: ban-record,
        kick-message: option<component>,
    }

    record unban-request {
        target: ban-target,
        source: ban-source,
        silent: bool,
    }

    record ban-query {
        cursor: option<string>,
        limit: u32,
    }

    record ban-record-page {
        entries: list<ban-record>,
        next-cursor: option<string>,
    }

    ban: func(request: ban-request) -> result<ban-entry, host-error>;
    unban: func(target: ban-target) -> result<option<ban-entry>, host-error>;
    get: func(target: ban-target) -> result<option<ban-entry>, host-error>;
    %list: func(cursor: option<string>, limit: u32) -> result<ban-page, host-error>;
}

interface config-service {
    use types.{server-id, server-address, proxy-mode, host-error};

    record server-source {
        id: string,
        provider-id: string,
        provider-type: string,
        editable: bool,
    }

    record server-config {
        id: server-id,
        network: option<string>,
        addresses: list<server-address>,
        domains: list<string>,
        proxy-mode: proxy-mode,
        limbo-handlers: list<string>,
        max-players: u32,
        disconnect-message: option<string>,
        send-proxy-protocol: bool,
        has-server-manager: bool,
    }

    get-value: func(key: string) -> result<option<string>, host-error>;
    get-server: func(server: server-id) -> result<option<server-config>, host-error>;
    list-servers: func() -> result<list<server-config>, host-error>;
    get-server-document: func(server: server-id) -> result<option<string>, host-error>;
    list-server-sources: func() -> result<list<server-source>, host-error>;
    get-proxy-config-document: func() -> result<string, host-error>;
    get-effective-proxy-config-document: func() -> result<string, host-error>;
    write-proxy-config-document: func(document: string) -> result<_, host-error>;
}

interface command-manager {
    use types.{handler-id, host-error};

    record command-spec {
        name: string,
        aliases: list<string>,
        description: string,
        usage: option<string>,
        permission: option<string>,
        hidden: bool,
    }

    record command-registration {
        name: string,
        namespaced: string,
        aliases: list<string>,
        rejected-aliases: list<string>,
    }

    register: func(spec: command-spec, handler: handler-id) -> result<command-registration, host-error>;
    unregister: func(name: string) -> result<_, host-error>;
}

interface scheduler {
    use types.{task-handle, handler-id, duration-ms, host-error};

    delay: func(after: duration-ms, handler: handler-id) -> result<task-handle, host-error>;
    interval: func(period: duration-ms, initial-delay: option<duration-ms>, handler: handler-id) -> result<task-handle, host-error>;
    cancel: func(handle: task-handle) -> result<_, host-error>;
}

interface codec-registry {
    use types.{handler-id, host-error};

    enum filter-priority { first, early, normal, late, last }

    record codec-filter-metadata {
        id: string,
        priority: filter-priority,
        after: list<string>,
        before: list<string>,
    }

    register-codec-filter: func(metadata: codec-filter-metadata, factory: handler-id) -> result<_, host-error>;
    unregister-codec-filter: func(id: string) -> result<_, host-error>;
}

interface load-balancer {
    use types.{server-id, server-address, host-error};
    use events.{backend-state};

    record backend-status {
        address: server-address,
        weight: u32,
        effective-weight: u32,
        state: backend-state,
        active-connections: u64,
        healthy-since-secs: option<u64>,
        ejections: u32,
        last-failure-secs-ago: option<u64>,
    }

    strategy: func(server: server-id) -> result<option<string>, host-error>;
    backends: func(server: server-id) -> result<list<backend-status>, host-error>;
    set-drained: func(server: server-id, address: server-address, drained: bool) -> result<_, host-error>;
    reset-backend: func(server: server-id, address: server-address) -> result<_, host-error>;
}

interface messaging {
    use types.{player-id, server-id, channel-id, host-error};

    register-channel: func(channel: channel-id) -> result<_, host-error>;
    unregister-channel: func(channel: channel-id) -> result<bool, host-error>;
    channels: func() -> result<list<channel-id>, host-error>;
    send-to-player: func(player: player-id, channel: channel-id, data: list<u8>) -> result<_, host-error>;
    send-to-backend: func(player: player-id, channel: channel-id, data: list<u8>) -> result<_, host-error>;
    send-to-server: func(server: server-id, channel: channel-id, data: list<u8>) -> result<u32, host-error>;
}

interface proxy-info {
    use types.{capability, socket-address, duration-ms};

    record rate-limit-info {
        max-connections: u32,
        window-ms: duration-ms,
        status-max: u32,
        status-window-ms: duration-ms,
    }

    record status-cache-info {
        ttl-ms: duration-ms,
        max-entries: u64,
    }

    record keepalive-info {
        time-ms: duration-ms,
        interval-ms: duration-ms,
        retries: u32,
    }

    enum unknown-domain-behavior { default-motd, drop }

    record proxy-details {
        version: string,
        bind: socket-address,
        max-connections: u32,
        connect-timeout-ms: duration-ms,
        receive-proxy-protocol: bool,
        worker-threads: u32,
        so-reuseport: bool,
        rate-limit: rate-limit-info,
        status-cache: status-cache-info,
        keepalive: keepalive-info,
        telemetry-enabled: bool,
        docker-enabled: bool,
        web-api-enabled: bool,
        web-ui-enabled: bool,
        unknown-domain-behavior: unknown-domain-behavior,
    }

    details: func() -> proxy-details;
    granted-capabilities: func() -> list<capability>;
}

interface plugin-registry {
    use types.{plugin-dependency};

    record plugin-info {
        id: string,
        name: string,
        version: string,
        authors: list<string>,
        description: option<string>,
        state: string,
        dependencies: list<plugin-dependency>,
    }

    %list: func() -> list<plugin-info>;
    get: func(id: string) -> option<plugin-info>;
}

interface permissions {
    use types.{player-id, game-profile, socket-address, host-error};

    record permission-rule {
        node: string,
        value: bool,
    }

    record permission-snapshot {
        rules: list<permission-rule>,
        admin: bool,
    }

    record player-subject {
        id: player-id,
        profile: game-profile,
        online-mode: bool,
        virtual-host: option<string>,
        remote-addr: socket-address,
    }

    variant permission-subject {
        player(player-subject),
        console,
    }

    set-snapshot: func(player: player-id, snapshot: permission-snapshot) -> result<_, host-error>;
    release: func(player: player-id) -> result<_, host-error>;
}

interface providers {
    use types.{host-error};
    use ban-service.{ban-features};

    register-ban-provider: func(features: ban-features) -> result<_, host-error>;
    register-permission-provider: func() -> result<_, host-error>;
}
```

- `ban-service.ban` records the plugin as the ban's source. `unban` answers the removed entry. `list` pages through bans with the cursor from the previous page.
- `command-manager.register` answers what the host registered, including the aliases it rejected because they are taken. The `handler-id` routes invocations and completions back into `handle-command` and `tab-complete`.
- `scheduler.interval` takes an optional initial delay; without one the first run waits one period. The `handler-id` routes back into `on-scheduled-task`.
- `codec-registry` filter priorities run from `first` to `last`; see [Codec Filters](./codec-filters).
- `config-service` document functions mirror the native `ConfigService` and redact every secret. `write-proxy-config-document` needs `config-write`; a document that does not parse or validate answers `invalid-argument`, a failed write `unavailable`.
- `load-balancer` mirrors the native `LoadBalancerService`; an unknown server or address answers `not-found`.
- `messaging` takes a `channel-id` with a modern id, a legacy name or both, validated by the host (`invalid-argument` otherwise). `send-to-server` answers how many players could carry the message and `unavailable` when none could.
- `proxy-info` and `plugin-registry` are always linked and never refuse: `granted-capabilities` lists what the plugin holds.
- The `ban-service` records from `login-stage` to `ban-record-page` are the provider side of bans: what the host hands a ban provider and what it answers. `ban-record` carries a typed `ban-source` where the consumer-side `ban-entry` carries its display string. See [Bans](./bans).
- `permissions.set-snapshot` replaces the snapshot of a player who holds one from this plugin (answered `not-found` otherwise, `player-gone` when the player is offline, `invalid-argument` above 65,536 rules) and refreshes the player's command tree. `release` clears it back to the node defaults and forgets it; releasing a player the plugin holds nothing for succeeds. See [Permissions](./permissions).
- `providers.register-*` register the plugin as the provider named by `[ban] provider` or `[permissions] provider`. A plugin that is not the selected one is answered `conflict`. Registering again, for example from a recovered instance, keeps the first registration and succeeds.

## Limbo

The host owns the native session and lends it by borrow to the `limbo-on-*` guest callbacks. `register-limbo-handler` is refused with `permission-denied` for a plugin without the `limbo` capability, so no session is ever minted for it.

```wit
interface limbo {
    use types.{player-id, server-id, component, game-profile, title-data, handler-id, duration-ms, host-error};

    variant limbo-entry-context {
        initial-connection(server-id),
        kicked-from-server(tuple<server-id, component>),
        plugin-redirect(option<server-id>),
    }

    variant timeout-outcome {
        accept,
        deny(component),
        redirect(server-id),
        send-to-limbo(list<string>),
    }

    record hold-timeout {
        after-ms: duration-ms,
        on-timeout: timeout-outcome,
    }

    variant handler-result {
        accept,
        deny(component),
        hold,
        hold-with-timeout(hold-timeout),
        redirect(server-id),
        send-to-limbo(list<string>),
    }

    enum session-end-reason {
        disconnected,
        released,
        kicked,
        redirected,
        timed-out,
        shutdown,
    }

    resource limbo-session {
        player-id: func() -> player-id;
        profile: func() -> game-profile;
        entry-context: func() -> limbo-entry-context;
        send-message: func(message: component) -> result<_, host-error>;
        send-title: func(title: title-data) -> result<_, host-error>;
        send-action-bar: func(message: component) -> result<_, host-error>;
        complete: func(outcome: handler-result) -> result<_, host-error>;
        acquire-handle: func() -> limbo-session-handle;
    }

    resource limbo-session-handle {
        player-id: func() -> player-id;
        send-message: func(message: component) -> result<_, host-error>;
        send-title: func(title: title-data) -> result<_, host-error>;
        send-action-bar: func(message: component) -> result<_, host-error>;
        complete: func(outcome: handler-result) -> result<_, host-error>;
        cancelled: func() -> bool;
    }

    register-limbo-handler: func(name: string, handler: handler-id) -> result<_, host-error>;
}
```

`acquire-handle` mints an own-able `limbo-session-handle` the guest can store across dispatches and complete later, from `on-scheduled-task` or an event. `cancelled` reports `true` once the engine has ended the session, so a stored handle's scheduled task knows to stop. A `timeout-outcome` is a terminal-only subset of `handler-result`, so a timed-out hold can never re-arm another hold. `complete` refuses an outcome whose text is invalid. See [Limbo](./limbo).

## The guest export

```wit
interface guest {
    use types.{plugin-metadata, player-id, player-ref, handler-id, listener-handle, component};
    use events.{event, event-outcome};
    use limbo.{limbo-session, handler-result, session-end-reason};
    use permissions.{permission-subject, permission-snapshot};
    use ban-service.{
        login-attempt, ban-verdict, ban-request, ban-source, ban-record, unban-request, ban-target,
        ban-query, ban-record-page,
    };

    record recovery-info {
        attempt: u32,
        cause: string,
    }

    variant enable-reason {
        initial,
        recovered(recovery-info),
    }

    enum disable-reason {
        shutdown,
        unload,
        quarantine,
    }

    variant command-sender {
        console,
        player(player-ref),
    }

    record command-invocation {
        label: string,
        args: list<string>,
        raw: string,
        sender: command-sender,
    }

    record suggestion {
        text: string,
        tooltip: option<component>,
    }

    metadata: func() -> plugin-metadata;
    on-enable: func(reason: enable-reason) -> result<_, string>;
    on-disable: func(reason: disable-reason) -> result<_, string>;

    handle-event: func(listener: listener-handle, ev: event) -> event-outcome;

    handle-command: func(handler: handler-id, invocation: command-invocation);
    tab-complete: func(handler: handler-id, sender: command-sender, args: list<string>, cursor: u32) -> list<suggestion>;
    on-scheduled-task: func(handler: handler-id);

    limbo-on-player-enter: func(handler: handler-id, session: borrow<limbo-session>) -> handler-result;
    limbo-on-command: func(handler: handler-id, session: borrow<limbo-session>, command: string, args: list<string>);
    limbo-on-chat: func(handler: handler-id, session: borrow<limbo-session>, message: string);
    limbo-on-disconnect: func(handler: handler-id, player: player-id);
    limbo-on-session-end: func(handler: handler-id, player: player-id, reason: session-end-reason);

    ban-provider-check: func(attempt: login-attempt) -> result<option<ban-verdict>, string>;
    ban-provider-ban: func(request: ban-request, source: ban-source) -> result<ban-record, string>;
    ban-provider-unban: func(request: unban-request) -> result<option<ban-record>, string>;
    ban-provider-get: func(target: ban-target) -> result<option<ban-record>, string>;
    ban-provider-list: func(query: ban-query) -> result<ban-record-page, string>;

    permission-snapshot-for: func(subject: permission-subject) -> permission-snapshot;
}
```

- `on-enable` receives `initial` the first time and `recovered` when the host replaced a faulted instance; `attempt` counts the recoveries and `cause` describes the fault. Returning `err` fails the enable.
- `on-disable` receives `shutdown` when the proxy stops and `unload` when the plugin alone is disabled. `quarantine` is reserved: a quarantined plugin has no live instance, so the host skips `on-disable` for it.
- `handle-event` receives the `listener-handle` from `subscribe` and answers an `event-outcome`.
- `handle-command` and `tab-complete` receive the command `handler-id` given to `register`; `tab-complete` answers suggestions with optional tooltips.
- The `ban-provider-*` exports answer the host once the plugin registered a ban provider. `ban-provider-ban` receives who issued the ban as a second argument. An `err` answer, a trap or a missed deadline fails the call; for `ban-provider-check` that refuses the login. The SDK answers `err("this plugin provides no bans")` when the plugin provides none.
- `permission-snapshot-for` answers the snapshot for a player or the console once the plugin registered a permission provider. A trap or a missed deadline leaves the subject with the node defaults. The SDK answers an empty snapshot when the plugin provides none.

## The codec-filter export

`codec-filter.wit` exports a per-session resource on the hot path. The host calls `create` once per connection and side, then `filter` for every frame, then drops the instance. The `filter` signature takes `(packet-id, data)` only; the codec context is reconstructed guest-side from `codec-session-init` plus the lifecycle calls.

```wit
interface codec-filter {
    use types.{raw-packet, protocol-version, connection-state, handler-id, socket-address, ip-address};

    enum connection-side { client-side, server-side }

    record codec-session-init {
        client-version: protocol-version,
        connection-id: u64,
        remote-addr: socket-address,
        real-ip: option<ip-address>,
        side: connection-side,
    }

    variant codec-filter-error {
        translation-failed(string),
        malformed-payload,
        unsupported-version(s32),
        internal(string),
    }

    record filter-extras {
        packet: option<raw-packet>,
        inject-before: list<raw-packet>,
        inject-after: list<raw-packet>,
    }

    variant filter-output {
        pass,
        drop,
        pass-modified(filter-extras),
        replace(filter-extras),
        error(codec-filter-error),
    }

    enum filter-verdict { pass, drop, modified }

    resource filter-instance {
        filter: func(packet-id: s32, data: list<u8>) -> filter-verdict;
        take-output: func() -> filter-output;
        on-state-change: func(new-state: connection-state);
        on-compression-change: func(threshold: s32);
        on-encryption-enabled: func();
        on-close: func();
    }

    create: func(factory: handler-id, init: codec-session-init) -> filter-instance;
}
```

`filter` answers `pass` or `drop` on its own. Anything else (a changed frame, injected frames, a `replace`, an error) is answered as `modified`, and the host then calls `take-output` to fetch the `filter-output` the guest set aside. Splitting the two keeps the common `pass` answer free of list cleanup, so it costs one guest call less than returning `filter-output` directly. The boundary has no `&mut`, so a changed frame is returned in `filter-extras`; `packet none` means unchanged. Codec instances run in a separate synchronous store: `log` works there, the other host imports trap. See [Codec Filters](./codec-filters).

## Building against the contract

The SDK embeds `wit-bindgen`, so you compile a plain `cdylib` for `wasm32-wasip2` with no `cargo-component` step.

```bash
rustup target add wasm32-wasip2
cargo build --release --target wasm32-wasip2
```

```toml
[lib]
crate-type = ["cdylib"]
```

## See also

- [Migrating to 0.3](./migration-0.3): what changed from `infrarust:plugin@0.2.3`.
- [Events](./events): the typed SDK event surface.
- [Services](./services): wrappers over `players`, `ban-service`, `text` and the other host imports.
- [Limbo](./limbo): the limbo handler and session-completion model.
- [Codec Filters](./codec-filters): the `filter-instance` trait and chain ordering.
- [Capabilities](./capabilities): baseline and opt-in capability strings.
