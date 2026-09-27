#!/usr/bin/env python3
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DOCS = ROOT / "docs" / "v2" / "plugins" / "wasm"
SDK = ROOT / "crates" / "infrarust-plugin-sdk"

PAGES = [
    "examples", "getting-started", "api-reference", "events", "commands", "services",
    "limbo", "messaging", "bans", "permissions", "codec-filters", "lifecycle", "network",
    "capabilities", "index", "migration-0.3", "threading", "fault-model", "virtual-backend",
    "building", "deploying", "architecture",
]

STD_TYPES = {
    "Duration": "std::time::Duration",
    "SystemTime": "std::time::SystemTime",
    "IpAddr": "std::net::IpAddr",
    "SocketAddr": "std::net::SocketAddr",
    "HashMap": "std::collections::HashMap",
    "HashSet": "std::collections::HashSet",
    "RefCell": "std::cell::RefCell",
    "Cell": "std::cell::Cell",
    "Rc": "std::rc::Rc",
}

NATIVE = "native infrarust / loader host code, not guest SDK code"

SPEC = {
    "getting-started.md: description = \"Logs joins, adds /hello\",": {
        "mode": "items", "prepend": "#[derive(Default)]\nstruct MyPlugin;",
        "replace": [["/* ... */", "fn on_enable(&self, _ctx: &Context) -> Result<(), PluginError> { Ok(()) }"]]},
    "getting-started.md: fn on_enable(&self, ctx: &Context) -> Result<(), PluginError>;": {
        "mode": "plugin_items", "decl_body": "{ Ok(()) }"},
    "events.md: pub fn on<E: GuestEvent>(": {"mode": "decl", "owner": "Context"},
    "commands.md: let registration = ctx": {
        "mode": "enable",
        "extra": "fn warp(_invocation: &CommandInvocation) {}\nfn complete_warps(_partial: &str) -> Vec<String> { Vec::new() }",
        "note": "helpers `warp` and `complete_warps` are not defined on the page; stubs added"},
    "commands.md: pub struct CommandInvocation {": {"mode": "decl"},
    "commands.md: .map(|candidate| Suggestion::new(candidate).with_tooltip(\"greet them\"))": {
        "mode": "enable", "prefix": "ctx.command(\"greet\")", "suffix": ".register()?;"},
    "commands.md: let event = ctx.command(\"event\").handler(|_| start_event()).register()?;": {
        "mode": "enable", "extra": "fn start_event() {}",
        "note": "helper `start_event` is not defined on the page; stub added"},
    "services.md: pub struct Error {": {"mode": "decl"},
    "services.md: impl Players {": {"mode": "decl"},
    "services.md: let bar = player.show_boss_bar(": {"mode": "fn", "params": "player: Player"},
    "services.md: impl Servers {": {"mode": "decl"},
    "services.md: pub enum BanTarget {": {"mode": "decl"},
    "services.md: pub fn get(key: &str) -> Result<Option<String>, Error>;": {"mode": "decl"},
    "services.md: pub fn server_document(server: &ServerId) -> Result<Option<String>, Error>;": {"mode": "decl"},
    "services.md: impl LoadBalancer {": {"mode": "decl"},
    "services.md: impl Messaging {": {"mode": "decl"},
    "services.md: impl Proxy {": {"mode": "decl"},
    "services.md: pub fn delay(&self, after: Duration, task: impl FnOnce() + 'static) -> Result<TaskHandle, Error>;": {
        "mode": "decl", "owner": "Context"},
    "services.md: trace!(\"inbound packet {id}\");": {
        "mode": "fn", "params": "id: i32, state: &str, name: &str, attempt: u32, max: u32, err: Error"},
    "services.md: let message = Component::text(\"Server: \")": {"mode": "fn", "params": "player: Player"},
    "limbo.md: pub trait LimboHandler {": {"mode": "decl"},
    "limbo.md: pub enum HandlerOutcome {": {"mode": "decl"},
    "limbo.md: pub enum TimeoutOutcome {": {"mode": "decl"},
    "limbo.md: .send_message(Component::text(\"Type /continue within 5s\"))": {
        "mode": "impl_items", "trait": "LimboHandler"},
    "limbo.md: pub enum EntryContext {": {"mode": "decl"},
    "limbo.md: struct DelayedGate;": {"mode": "items"},
    "limbo.md: pub enum SessionEndReason {": {"mode": "decl"},
    "limbo.md: struct Boom;": {"mode": "items"},
    "messaging.md: fn register_bungeecord(ctx: &Context) -> Result<(), PluginError> {": {"mode": "items"},
    "permissions.md: let promoted = PermissionSnapshot::new().grant(\"warps.*\");": {
        "mode": "fn", "params": "player: Player"},
    "permissions.md: let groups = Groups::load();": {
        "mode": "plugin_items",
        "extra": "#[derive(Clone, Default)]\nstruct Groups(Rc<RefCell<HashMap<String, PermissionSnapshot>>>);\n\nimpl Groups {\n    fn load() -> Self {\n        Self::default()\n    }\n\n    fn of(&self, username: &str) -> PermissionSnapshot {\n        self.0.borrow().get(username).cloned().unwrap_or_default()\n    }\n}\n\nimpl PermissionProvider for Groups {\n    fn snapshot_for(&self, _subject: &PermissionSubject) -> PermissionSnapshot {\n        PermissionSnapshot::new()\n    }\n}",
        "note": "the `Groups` provider of the page, with a stub `Groups::load`, copied in"},
    "codec-filters.md: struct Counter {": {"mode": "items"},
    "codec-filters.md: pub trait CodecFilter {": {"mode": "decl"},
    "codec-filters.md: packet.id();": {
        "mode": "fn", "params": "packet: &mut Packet, packet_id: i32",
        "scoped_let": "let bytes: Vec<u8> = Vec::new();"},
    "codec-filters.md: ^out.before(Packet::new(0xfe, b\"before\".to_vec()));": {
        "mode": "fn", "params": "out: &mut Injections"},
    "codec-filters.md: impl CodecFilter for OpFilter {": {"mode": "items", "extra": "struct OpFilter;"},
    "codec-filters.md: struct Tally {": {"mode": "items"},
    "lifecycle.md: // AotCache::cache_key in cache.rs": {"mode": "skip", "reason": NATIVE},
    "lifecycle.md: let wit_md = bindings": {"mode": "skip", "reason": NATIVE},
    "lifecycle.md: // load in loader.rs": {"mode": "skip", "reason": NATIVE},
    "lifecycle.md: // InstanceFactory in instance.rs": {"mode": "skip", "reason": NATIVE},
    "lifecycle.md: // SDK guest trait (infrarust-plugin-sdk)": {"mode": "decl"},
    "lifecycle.md: fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {": {"mode": "plugin_items"},
    "network.md: use std::io::{Read, Write};": {"mode": "full", "deps": {"wasip2": "\"1.0\""}},
    "capabilities.md: // crates/infrarust-api/src/permissions.rs": {"mode": "skip", "reason": NATIVE},
    "capabilities.md: pub fn baseline() -> Self {": {"mode": "skip", "reason": NATIVE},
    "capabilities.md: pub fn native_trusted() -> Self {": {"mode": "skip", "reason": NATIVE},
    "capabilities.md: pub fn from_config_strings(": {"mode": "skip", "reason": NATIVE},
    "capabilities.md: pub fn from_config(grants": {"mode": "skip", "reason": NATIVE},
    "capabilities.md: match ban_service::get(": {
        "mode": "skip", "reason": "raw wit-bindgen bindings of the capability-denied fixture, not the SDK"},
    "capabilities.md: // crates/infrarust-loader-wasm/src/imp/store_state.rs": {"mode": "skip", "reason": NATIVE},
    "migration-0.3.md: fn on_enable(&self, ctx: &Context) -> Result<(), String> {": {
        "mode": "plugin_items", "after_marker": "// 0.3.0",
        "note": "only the 0.3.0 half; the 0.2.3 half is deliberately old code"},
    "migration-0.3.md: if let Some(id) = invocation.player": {
        "mode": "fn", "params": "invocation: CommandInvocation, reply: String", "after_marker": "// 0.3.0",
        "note": "only the 0.3.0 half; the 0.2.3 half is deliberately old code"},
    "virtual-backend.md: pub trait VirtualBackendHandler: Send + Sync {": {"mode": "skip", "reason": NATIVE},
    "virtual-backend.md: pub trait VirtualBackendSession: Send + Sync + private::Sealed {": {"mode": "skip", "reason": NATIVE},
    "virtual-backend.md: Capability::to_kebab()": {"mode": "skip", "reason": NATIVE},
    "building.md: #![forbid(unsafe_code)]": {
        "mode": "full",
        "deps": {"infrarust-plugin-stats": "{ path = \"%s\" }" % (ROOT / "plugins" / "infrarust-plugin-stats")}},
    "building.md: // crates/infrarust-loader-wasm/build.rs": {"mode": "skip", "reason": NATIVE},
    "deploying.md: fn metadata(&self) -> PluginMetadata {": {"mode": "plugin_items", "attr": "#[plugin]"},
    "architecture.md: fn on_enable(&self, ctx: &Context) -> Result<(), PluginError>;": {
        "mode": "plugin_items", "decl_body": "{ Ok(()) }"},
}


def blocks():
    for page in PAGES:
        path = DOCS / f"{page}.md"
        if not path.exists():
            continue
        lines = path.read_text().split("\n")
        i = 0
        while i < len(lines):
            m = re.match(r"^\s*```(\S*)", lines[i])
            if not m:
                i += 1
                continue
            lang = m.group(1)
            j = i + 1
            while j < len(lines) and not re.match(r"^\s*```\s*$", lines[j]):
                j += 1
            if lang == "rust":
                yield f"{page}.md", i + 1, lines[i + 1:j]
            i = j + 1


def slug(key):
    page, line = key.split(":")
    return "doc-" + re.sub(r"[^a-z0-9]+", "-", page[:-3].lower()).strip("-") + "-" + line


class Src:
    def __init__(self):
        self.rows = []

    def add(self, text, doc_line=None):
        for k, row in enumerate(text.split("\n")):
            self.rows.append((row, None if doc_line is None else doc_line + k))

    def add_fixed(self, text, doc_line):
        for row in text.split("\n"):
            self.rows.append((row, doc_line))

    def add_block(self, rows, indent=""):
        for row, doc_line in rows:
            self.rows.append((indent + row if row.strip() else row, doc_line))

    def text(self):
        return "\n".join(r for r, _ in self.rows) + "\n"

    def mapping(self):
        return [d for _, d in self.rows]


def std_imports(body_text, own_text):
    out = []
    for name, path in STD_TYPES.items():
        if re.search(r"\b%s\b" % name, body_text) and not re.search(r"use\s+std::[^;]*\b%s\b" % name, own_text):
            out.append(f"#[allow(unused_imports)]\nuse {path};")
    return "\n".join(out)


ARROW = "\x01"

PRELUDE = "use infrarust_plugin_sdk::prelude::*;"


def header(src, body_text, spec, with_prelude=True):
    src.add("#![allow(dead_code, unused_variables, unused_mut, unused_imports)]")
    if with_prelude:
        src.add(PRELUDE)
    imports = std_imports(body_text + "\n" + spec.get("extra", ""), body_text if spec["mode"] in ("items", "full") else "")
    if imports:
        src.add(imports)
    src.add("")


def plugin_open(src, attr='#[plugin(id = "doc-snippet")]'):
    src.add("#[derive(Default)]\nstruct DocSnippet;\n")
    src.add(attr)
    src.add("impl Plugin for DocSnippet {")


def split_params(s):
    out, depth, cur = [], 0, ""
    for ch in s:
        if ch in "(<[{":
            depth += 1
        elif ch in ")>]}":
            depth -= 1
        if ch == "," and depth == 0:
            out.append(cur.strip())
            cur = ""
        else:
            cur += ch
    if cur.strip():
        out.append(cur.strip())
    return out


def strip_comments(text):
    return "\n".join(re.sub(r"//.*$", "", l) for l in text.split("\n"))


def find_braced(text, start):
    depth = 0
    for k in range(start, len(text)):
        if text[k] == "{":
            depth += 1
        elif text[k] == "}":
            depth -= 1
            if depth == 0:
                return k
    return len(text)


def fn_decls(body):
    out, depth, cur = [], 0, ""
    for ch in body:
        if ch in "(<[{":
            depth += 1
        elif ch in ")>]}":
            depth -= 1
        if ch == ";" and depth == 0:
            if cur.strip():
                out.append(" ".join(cur.split()))
            cur = ""
        else:
            cur += ch
    if cur.strip():
        out.append(" ".join(cur.split()))
    return [d for d in out if re.match(r"(pub\s+)?fn\s", d)]


def gen_fn_check(owner, decl, n):
    m = re.match(r"(?:pub\s+)?fn\s+(\w+)\s*(<[^(]*>)?\s*\((.*)\)\s*(?:\x01\s*(.+))?$", decl)
    if not m:
        return f"compile_error!(\"unparsed declaration: {decl}\");"
    name, generics, params, ret = m.group(1), m.group(2) or "", m.group(3), m.group(4) or "()"
    ps = split_params(params)
    recv = None
    if ps and re.match(r"^(&\s*(mut\s+)?)?self$", ps[0]):
        recv = ps.pop(0)
    names, typed = [], []
    for p in ps:
        pn, pt = p.split(":", 1)
        names.append(pn.strip())
        typed.append(f"{pn.strip()}: {pt.strip()}")
    if recv is None:
        sig = ", ".join(typed)
        call = f"{owner}::{name}({', '.join(names)})"
    else:
        this_ty = {"self": owner, "&self": f"&{owner}", "&mut self": f"&mut {owner}"}[" ".join(recv.split()).replace("& ", "&")]
        sig = ", ".join([f"this: {this_ty}"] + typed)
        call = f"this.{name}({', '.join(names)})"
    code = f"fn check_{n}_{owner.lower()}_{name}{generics}({sig}) -> {ret} {{\n    {call}\n}}"
    if not generics and "impl " not in params:
        types = [p.split(":", 1)[1].strip() for p in ps]
        if recv is not None:
            types.insert(0, this_ty)
        code += f"\n\nfn check_{n}_{owner.lower()}_{name}_exact() {{\n    let _: fn({', '.join(types)}) -> {ret} = {owner}::{name};\n}}"
    return code


def gen_struct_check(name, body, n):
    lines = []
    for f in split_params(body):
        m = re.match(r"(?:pub\s+)?(\w+)\s*:\s*(.+)$", f.strip())
        if m:
            lines.append(f"    let _: &{m.group(2).strip()} = &value.{m.group(1)};")
    return f"fn check_{n}_struct_{name.lower()}(value: &{name}) {{\n" + "\n".join(lines) + "\n}"


def gen_enum_check(name, body, n):
    arms = []
    for v in split_params(body):
        v = v.strip()
        if not v:
            continue
        m = re.match(r"(\w+)\s*(\((.*)\)|\{(.*)\})?$", v, re.S)
        if not m:
            arms.append(f"        compile_error!(\"unparsed variant {v}\")")
            continue
        vn = m.group(1)
        if m.group(3) is not None:
            tys = split_params(m.group(3))
            binds = [f"f{k}" for k in range(len(tys))]
            checks = " ".join(f"let _: &{t} = {b};" for b, t in zip(binds, tys))
            arms.append(f"        {name}::{vn}({', '.join(binds)}) => {{ {checks} }}")
        elif m.group(4) is not None:
            fields = [split for split in split_params(m.group(4))]
            binds, checks = [], []
            for f in fields:
                fn_, ft = f.split(":", 1)
                binds.append(fn_.strip())
                checks.append(f"let _: &{ft.strip()} = {fn_.strip()};")
            arms.append(f"        {name}::{vn} {{ {', '.join(binds)} }} => {{ {' '.join(checks)} }}")
        else:
            arms.append(f"        {name}::{vn} => {{}}")
    arms.append("        #[allow(unreachable_patterns)]\n        _ => {}")
    return f"fn check_{n}_enum_{name.lower()}(value: &{name}) {{\n    match value {{\n" + ",\n".join(arms) + ",\n    }\n}"


def gen_trait_check(name, body, n):
    items = []
    k = 0
    text = body
    while k < len(text):
        m = re.compile(r"fn\s+\w+").search(text, k)
        if not m:
            break
        s = m.start()
        depth = 0
        e = s
        while e < len(text):
            ch = text[e]
            if ch in "(<[":
                depth += 1
            elif ch in ")>]":
                depth -= 1
            elif depth == 0 and ch == ";":
                items.append(text[s:e].strip() + " { unimplemented!() }")
                e += 1
                break
            elif depth == 0 and ch == "{":
                close = find_braced(text, e)
                items.append(text[s:close + 1].strip())
                e = close + 1
                break
            e += 1
        k = e
    stub = f"DocImpl{n}{name}"
    return f"struct {stub};\n\nimpl {name} for {stub} {{\n" + "\n".join("    " + " ".join(i.split()) for i in items) + "\n}"


def gen_decl(text, spec):
    text = strip_comments(text).replace("->", ARROW)
    out, n, pos = [], 0, 0
    pattern = re.compile(r"(?:pub\s+)?(impl|struct|enum|trait)\s+(\w+)[^{;]*\{|((?:pub\s+)?fn\s)")

    def at(offset):
        return text[:offset].count("\n")

    while True:
        m = pattern.search(text, pos)
        if not m:
            break
        n += 1
        if m.group(3):
            depth = 0
            e = m.start()
            while e < len(text):
                ch = text[e]
                if ch in "(<[":
                    depth += 1
                elif ch in ")>]":
                    depth -= 1
                elif depth == 0 and ch == ";":
                    break
                e += 1
            chunk = text[m.start():e]
            out.append((gen_fn_check(spec["owner"], " ".join(chunk.split()), n), at(m.start())))
            pos = e + 1
            continue
        kind, name = m.group(1), m.group(2)
        open_at = m.end() - 1
        close = find_braced(text, open_at)
        body = text[open_at + 1:close]
        if kind == "impl":
            for d in fn_decls(body):
                n += 1
                fname = re.match(r"(?:pub\s+)?fn\s+(\w+)", d).group(1)
                where = re.search(r"fn\s+%s\b" % fname, body)
                out.append((gen_fn_check(name, d, n), at(open_at + 1 + (where.start() if where else 0))))
        elif kind == "struct":
            out.append((gen_struct_check(name, body, n), at(m.start())))
        elif kind == "enum":
            out.append((gen_enum_check(name, body, n), at(m.start())))
        elif kind == "trait":
            out.append((gen_trait_check(name, body, n), at(m.start())))
        pos = close + 1
    return [(code.replace(ARROW, "->"), line) for code, line in out]


def build(key, fence, rows, spec):
    src = Src()
    body = [(r, fence + 1 + k) for k, r in enumerate(rows)]
    if spec.get("after_marker"):
        idx = next(k for k, (r, _) in enumerate(body) if r.strip() == spec["after_marker"])
        body = body[idx + 1:]
    for old, new in spec.get("replace", []):
        body = [(r.replace(old, new), d) for r, d in body]
    body_text = "\n".join(r for r, _ in body)
    mode = spec["mode"]
    if mode == "full":
        src.add_block(body)
        if spec.get("extra"):
            src.add("")
            src.add(spec["extra"])
        return src
    header(src, body_text, spec)
    if spec.get("extra"):
        src.add(spec["extra"])
        src.add("")
    if spec.get("prepend"):
        src.add(spec["prepend"])
        src.add("")
    if mode == "items":
        src.add_block(body)
    elif mode == "decl":
        for code, rel in gen_decl(body_text, spec):
            src.add_fixed(code, body[0][1] + rel)
            src.add("")
    elif mode == "enable":
        plugin_open(src, spec.get("attr", '#[plugin(id = "doc-snippet")]'))
        src.add("    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {")
        if spec.get("prefix"):
            src.add("        " + spec["prefix"])
        src.add_block(body, "        ")
        if spec.get("suffix"):
            src.add("        " + spec["suffix"])
        src.add("        Ok(())\n    }\n}")
    elif mode == "fn":
        src.add(f"fn doc_snippet({spec.get('params', '')}) -> Result<(), PluginError> {{")
        if spec.get("scoped_let"):
            for r, d in body:
                if r.strip():
                    src.add("    { " + spec["scoped_let"])
                    src.add_block([(r, d)], "        ")
                    src.add("    }")
        else:
            src.add_block(body, "    ")
        src.add("    Ok(())\n}")
    elif mode == "plugin_items":
        plugin_open(src, spec.get("attr", '#[plugin(id = "doc-snippet")]'))
        if spec.get("decl_body"):
            last = max(k for k, (r, _) in enumerate(body) if r.strip())
            r, d = body[last]
            body[last] = (re.sub(r";\s*$", " " + spec["decl_body"], r), d)
        src.add_block(body, "    ")
        if not re.search(r"fn\s+on_enable\s*\(", body_text):
            src.add("    fn on_enable(&self, _ctx: &Context) -> Result<(), PluginError> { Ok(()) }")
        src.add("}")
    elif mode == "impl_items":
        src.add("struct DocSnippet;\n")
        src.add(f"impl {spec['trait']} for DocSnippet {{")
        src.add_block(body, "    ")
        src.add("}")
    return src


def anchored(anchor, rows):
    if anchor.startswith("^"):
        return next((r.strip() for r in rows if r.strip()), "") == anchor[1:]
    return anchor in "\n".join(rows)


def match_specs(found):
    by_page = {}
    for spec_key, spec in SPEC.items():
        page, anchor = spec_key.split(": ", 1)
        by_page.setdefault(page, []).append((anchor, spec_key, spec))
    matched, hits, problems = {}, {}, []
    for page, fence, rows in found:
        key = f"{page}:{fence}"
        specs = [(spec_key, spec) for anchor, spec_key, spec in by_page.get(page, []) if anchored(anchor, rows)]
        for spec_key, _ in specs:
            hits.setdefault(spec_key, []).append(key)
        if len(specs) > 1:
            problems.append(f"{key} matches several SPEC entries: " + ", ".join(repr(k) for k, _ in specs))
        elif specs:
            matched[key] = specs[0][1]
    for spec_key in SPEC:
        where = hits.get(spec_key, [])
        if len(where) != 1:
            problems.append(f"SPEC entry {spec_key!r} matches {len(where)} blocks" + (f": {', '.join(where)}" if where else ""))
    if problems:
        print("stale SPEC entries (each key is `<page>: <text>`, the text found in exactly one rust block of the page, or `<page>: ^<line>` for the block whose first line it is):")
        for problem in problems:
            print("   " + problem)
        sys.exit(2)
    return matched


def gen(out):
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    manifest, members = [], []
    matched = match_specs(list(blocks()))
    for page, fence, rows in blocks():
        key = f"{page}:{fence}"
        spec = matched.get(key)
        auto = spec is None
        if auto:
            joined = "\n".join(rows)
            spec = {"mode": "full"} if re.search(r"#\[plugin[\](]", joined) else {"mode": "enable"}
        entry = {"key": key, "mode": spec["mode"], "auto": auto, "note": spec.get("note"),
                 "reason": spec.get("reason"), "first": next((r.strip() for r in rows if r.strip()), "")}
        if spec["mode"] == "skip":
            manifest.append(entry)
            continue
        name = slug(key)
        crate = out / name
        (crate / "src").mkdir(parents=True, exist_ok=True)
        src = build(key, fence, rows, spec)
        (crate / "src" / "lib.rs").write_text(src.text())
        deps = {"infrarust-plugin-sdk": "{ workspace = true }"}
        deps.update(spec.get("deps", {}))
        dep_text = "\n".join(f"{k} = {v}" for k, v in deps.items())
        (crate / "Cargo.toml").write_text(
            f"[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n"
            f"[lib]\ncrate-type = [\"cdylib\"]\n\n[dependencies]\n{dep_text}\n")
        entry["crate"] = name
        entry["map"] = src.mapping()
        manifest.append(entry)
        members.append(name)
    member_text = "".join(f"    \"{m}\",\n" for m in members)
    (out / "Cargo.toml").write_text(
        "[workspace]\nresolver = \"3\"\nmembers = [\n" + member_text + "]\n\n"
        f"[workspace.dependencies]\ninfrarust-plugin-sdk = {{ path = \"{SDK}\" }}\n\n"
        "[profile.dev]\npanic = \"abort\"\n\n[profile.release]\npanic = \"abort\"\n")
    seed = ROOT / "crates" / "infrarust-loader-wasm" / "tests" / "fixtures" / "Cargo.lock"
    if seed.exists() and not (out / "Cargo.lock").exists():
        (out / "Cargo.lock").write_text(seed.read_text())
    (out / "manifest.json").write_text(json.dumps(manifest, indent=1))
    total = len(manifest)
    skipped = sum(1 for e in manifest if e["mode"] == "skip")
    autos = [e["key"] for e in manifest if e["auto"]]
    print(f"{total} rust blocks, {total - skipped} crates, {skipped} skipped")
    if autos:
        print("auto-classified (not in SPEC): " + ", ".join(autos))


def report(out, log):
    out = Path(out)
    manifest = json.loads((out / "manifest.json").read_text())
    by_crate = {e["crate"]: e for e in manifest if "crate" in e}
    diags, warns = {}, {}
    rx = re.compile(r"^(?:\S*/)?(doc-[a-z0-9-]+)/src/lib\.rs:(\d+):(\d+): (error|warning)(\[\w+\])?: (.*)$")
    for line in Path(log).read_text(errors="replace").split("\n"):
        m = rx.match(line.strip())
        if not m:
            continue
        crate, gl, level, code, msg = m.group(1), int(m.group(2)), m.group(4), m.group(5) or "", m.group(6)
        if crate not in by_crate:
            continue
        if level == "warning" and not re.search(r"must be used|deprecated", msg):
            continue
        mp = by_crate[crate]["map"]
        doc_line = mp[gl - 1] if 0 < gl <= len(mp) else None
        where = f"{by_crate[crate]['key'].split(':')[0]}:{doc_line}" if doc_line else f"generated line {gl}"
        if level == "error":
            diags.setdefault(crate, []).append(f"{where}: error{code}: {msg}")
        else:
            warns.setdefault(crate, []).append(f"{where}: warning: {msg}")
    ok = [e for e in manifest if "crate" in e and e["crate"] not in diags]
    bad = [e for e in manifest if "crate" in e and e["crate"] in diags]
    skipped = [e for e in manifest if e["mode"] == "skip"]
    print(f"blocks: {len(manifest)}  compiled: {len(ok) + len(bad)}  ok: {len(ok)}  failing: {len(bad)}  skipped: {len(skipped)}")
    for e in bad:
        print(f"\n== {e['key']} [{e['mode']}] {e['first'][:70]}")
        if e.get("note"):
            print(f"   scaffold: {e['note']}")
        for d in dict.fromkeys(diags[e["crate"]]):
            print("   " + d)
    if warns:
        print("\nmust-use / deprecation warnings:")
        for e in manifest:
            if e.get("crate") in warns:
                for d in dict.fromkeys(warns[e["crate"]]):
                    print(f"   {e['key']} -> {d}")
    print("\nskipped:")
    for e in skipped:
        print(f"   {e['key']}: {e['reason']}")
    scaff = [e for e in manifest if e.get("note") and e not in bad]
    if scaff:
        print("\nscaffolding notes on passing blocks:")
        for e in scaff:
            print(f"   {e['key']}: {e['note']}")


if __name__ == "__main__":
    if len(sys.argv) >= 3 and sys.argv[1] == "gen":
        gen(sys.argv[2])
    elif len(sys.argv) >= 4 and sys.argv[1] == "report":
        report(sys.argv[2], sys.argv[3])
    else:
        sys.exit("usage: extract.py gen OUT_DIR | extract.py report OUT_DIR LOG")
