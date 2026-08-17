//! `which-key.wasm` — a Rust-authored ekko extension for the cordis WASM ABI.
//!
//! Faithful port of the finix `which-key.lua`: vim-style hjkl nav, `x`
//! kill-session, the stock leader map, a zellij-style one-line status bar
//! (mode ribbon, per-mode key hints), a Ctrl+p pane mode, and a leader-attached
//! centred session-list overlay.
//!
//! # cordis ABI (relaxed for Rust guests)
//!
//! - `scratch() -> i32` — single-value form. The kernel derives a 64 KiB
//!   capacity; the returned pointer is the linear-memory base where the host
//!   writes each dispatch payload. We return the base of a big `SCRATCH`
//!   static.
//! - Every dispatch export is `(ptr, len) -> ()` (void) and returns its JSON
//!   result via `host.ctx_return` (Rust can't emit a clean 2-tuple return).
//! - `mount()` registers units via `host.register_*`.
//! - Draw during a dynamic dispatch is buffered set-3 ops the host drains via
//!   `take_ops`. Every op argument is a validated `(ptr, len)` string, including
//!   coordinates.
//!
//! The guest holds no `&mut` host state: it reads the immutable JSON snapshot
//! and returns action JSON (functional-core).

// The guest is single-threaded and uses fixed static arenas for the cordis ABI
// (raw pointers in/out of wasm). `static_mut_refs` is intentional here.
#![allow(static_mut_refs)]
// Dead `name` on grouper nodes is kept because the guest-visible JSON still carries it.
#![allow(dead_code)]

use serde_json::Value as Json;
use unicode_width::UnicodeWidthStr;

// ─────────────────────────────────────────────────────────────────────────
// Host imports (the cordis `host` module).
// ─────────────────────────────────────────────────────────────────────────

#[link(wasm_import_module = "host")]
extern "C" {
    fn ctx_return(ptr: i32, len: i32);

    // Set-2 registration.
    fn register_command(np: i32, nl: i32, dp: i32, dl: i32);
    fn register_keybinding(
        np: i32, nl: i32,
        mp: i32, ml: i32,
        dp: i32, dl: i32,
        hp: i32, hl: i32,
    );
    fn register_mode(np: i32, nl: i32, kp: i32, kl: i32, ip: i32, il: i32, rp: i32, rl: i32);
    fn register_surface(np: i32, nl: i32, dock: i32, priority: i32, size: i32);
    fn register_overlay(
        np: i32, nl: i32,
        dp: i32, dl: i32,
        rp: i32, rl: i32,
        kp: i32, kl: i32,
        ip: i32, il: i32,
        ap: i32, al: i32,
    );

    // Set-3 draw ops. Every argument is a (ptr, len) string.
    fn put_text(xp: i32, xl: i32, yp: i32, yl: i32, tp: i32, tl: i32, cp: i32, cl: i32);
    fn fill_rect(
        xp: i32, xl: i32, yp: i32, yl: i32,
        wp: i32, wl: i32, hp: i32, hl: i32,
        cp: i32, cl: i32,
    );
    fn draw_box(
        xp: i32, xl: i32, yp: i32, yl: i32,
        wp: i32, wl: i32, hp: i32, hl: i32,
        cp: i32, cl: i32,
    );
    fn render_scrollbar(
        xp: i32, xl: i32, yp: i32, yl: i32,
        visp: i32, visl: i32, totp: i32, totl: i32,
        fgp: i32, fgl: i32, tgp: i32, tgl: i32,
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Guest memory.
// ─────────────────────────────────────────────────────────────────────────

// Scratch arena the host writes dispatch payloads into; returned by `scratch()`.
static mut SCRATCH: [u8; 65536] = [0; 65536];
// Result buffer for JSON replies (delivered via ctx_return).
static mut RESULT: [u8; 12288] = [0; 12288];
static mut RESULT_LEN: usize = 0;

// A growing string pool for registration names, colors, and text; also holds
// stringified coordinates. Append-only, never cleared (few dozen strings).
static mut POOL: [u8; 8192] = [0; 8192];
static mut POOL_END: usize = 0;

/// Register `s` in the pool, returning `(ptr, len)`.
fn pool(s: &str) -> (i32, i32) {
    unsafe {
        let start = POOL_END;
        POOL[start..start + s.len()].copy_from_slice(s.as_bytes());
        POOL_END = start + s.len();
        (POOL.as_ptr().add(start) as i32, s.len() as i32)
    }
}

/// Stringify an integer into the pool (coordinates are (ptr,len) strings).
fn pool_num(n: i32) -> (i32, i32) {
    pool(&n.to_string())
}

/// Copy `s` into the result buffer and commit it via ctx_return.
fn reply(s: &str) {
    unsafe {
        let base = RESULT.as_ptr();
        core::ptr::copy_nonoverlapping(s.as_ptr(), base as *mut u8, s.len());
        RESULT_LEN = s.len();
        ctx_return(base as i32, RESULT_LEN as i32);
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Exports — the ABI entrypoints.
// ─────────────────────────────────────────────────────────────────────────

/// Single-value scratch: the base of the payload arena.
#[no_mangle]
pub extern "C" fn scratch() -> i32 {
    unsafe { SCRATCH.as_ptr() as i32 }
}

/// Register all extension units (host calls this on mount).
#[no_mangle]
pub fn mount() {
    register_all();
}

/// Required kernel hook.
#[no_mangle]
pub fn on_change(_p: i32, _l: i32) {}

/// Keybinding handler dispatch. `(p,l)` is a `DispatchRequest` JSON in scratch.
#[no_mangle]
pub fn on_key(p: i32, l: i32) {
    let req = read_payload(p, l);
    reply(&handle_key(req));
}

/// Command handler dispatch.
#[no_mangle]
pub fn on_command(p: i32, l: i32) {
    let req = read_payload(p, l);
    reply(&handle_command(req));
}

/// Mode key dispatch (leader/pane fallback).
#[no_mangle]
pub fn on_mode_key(p: i32, l: i32) {
    let req = read_payload(p, l);
    reply(&handle_mode_key(req));
}

/// Mode render dispatch — the modes have no inline render (cursor absent).
#[no_mangle]
pub fn on_mode_render(_p: i32, _l: i32) {
    reply("null");
}

/// Surface draw dispatch — paints the top/bottom status bars.
#[no_mangle]
pub fn on_surface_draw(p: i32, l: i32) {
    let req = read_payload(p, l);
    handle_surface_draw(req);
}

/// Surface wants-tick predicate.
#[no_mangle]
pub fn on_surface_wants_tick(p: i32, l: i32) {
    let req = read_payload(p, l);
    reply(&handle_wants_tick(req));
}

/// Overlay render dispatch — paints the sessions panel.
#[no_mangle]
pub fn on_overlay_render(p: i32, l: i32) {
    let req = read_payload(p, l);
    handle_overlay_render(req);
}

/// Overlay key dispatch.
#[no_mangle]
pub fn on_overlay_key(p: i32, l: i32) {
    let req = read_payload(p, l);
    reply(&handle_overlay_key(req));
}

// ─────────────────────────────────────────────────────────────────────────
// Payload + request decoding.
// ─────────────────────────────────────────────────────────────────────────

/// Read the (p,l) bytes the host wrote at absolute linear address `p`.
fn read_payload(p: i32, l: i32) -> &'static str {
    if p < 0 || l < 0 {
        return "";
    }
    let slice = unsafe { core::slice::from_raw_parts(p as *const u8, l as usize) };
    core::str::from_utf8(slice).unwrap_or("")
}

/// Parse a dispatch request: its `snapshot`, `name`, and `bytes`.
fn request(req: &str) -> (Snapshot, String, Vec<u8>) {
    let v: Json = serde_json::from_str(req).unwrap_or(Json::Null);
    let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let bytes = v
        .get("bytes")
        .and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(|b| b.as_u64()).map(|b| b as u8).collect())
        .unwrap_or_default();
    let snap = v
        .get("snapshot")
        .and_then(|s| serde_json::from_value(s.clone()).ok())
        .unwrap_or_default();
    (snap, name, bytes)
}

// ─────────────────────────────────────────────────────────────────────────
// Snapshot schema (subset of ClientSnapshot).
// ─────────────────────────────────────────────────────────────────────────

use serde::Deserialize;

#[derive(Deserialize, Default)]
struct Snapshot {
    session_name: String,
    #[serde(default)]
    mode: String,
    cols: u16,
    rows: u16,
    #[serde(default)]
    scrollback: u32,
    #[serde(default)]
    projects: Vec<ProjectGroup>,
    #[serde(default)]
    status_note: Option<StatusNote>,
    #[serde(default)]
    keybindings: Vec<KeybindInfo>,
    #[serde(default)]
    now_ms: u64,
}

#[derive(Deserialize, Default)]
struct ProjectGroup {
    name: String,
    #[serde(default)]
    sessions: Vec<SessionEntry>,
}

#[derive(Deserialize, Default)]
struct SessionEntry {
    name: String,
}

#[derive(Deserialize, Default)]
struct StatusNote {
    text: String,
    #[serde(default)]
    kind: String,
}

#[derive(Deserialize, Default)]
struct KeybindInfo {
    chord_text: String,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    description: String,
}

// ─────────────────────────────────────────────────────────────────────────
// Registration (mount).
// ─────────────────────────────────────────────────────────────────────────

const LEADER: &str = "leader";
const PANE: &str = "pane";

fn register_all() {
    // Leader chord in normal + leader scope.
    kb(None, "ctrl+b", "leader", "enter leader mode");
    kb(Some(LEADER), "ctrl+b", "close panel", "close");

    // Stock leader map (non-sticky: act then exit).
    kb(Some(LEADER), "n", "new session", "new_session");
    kb(Some(LEADER), "d", "detach", "detach");
    kb(Some(LEADER), "?", "help", "help");
    kb(Some(LEADER), "s", "scroll", "scroll");
    kb(Some(LEADER), "c", "command mode", "command");

    // Session nav (sticky).
    kb(Some(LEADER), "j", "next session", "next_session");
    kb(Some(LEADER), "k", "prev session", "prev_session");
    kb(Some(LEADER), "x", "kill session", "kill");

    // Pane commands.
    cmd("pane-new", "open a new pane", "pane-new");
    cmd("pane-focus", "focus the neighboring pane in a direction", "pane-focus");
    cmd("pane-close", "close the focused pane", "pane-close");

    // Pane mode (ctrl+p) + its map.
    kb(None, "ctrl+p", "pane", "enter pane mode");
    register_mode_guest(PANE);
    kb(Some(PANE), "n", "new pane", "pane-new");
    kb(Some(PANE), "x", "close pane", "close-pane");
    kb(Some(PANE), "q", "exit pane", "exit-pane");
    for (chord, dir) in [
        ("h", "left"),
        ("left", "left"),
        ("j", "down"),
        ("down", "down"),
        ("k", "up"),
        ("up", "up"),
        ("l", "right"),
        ("right", "right"),
    ] {
        kb(Some(PANE), chord, &format!("focus {dir}"), "focus");
    }

    // Two docked status-bar surfaces.
    surface("user.which-key:top", 2, 0, 1); // dock=2 top
    surface("user.which-key:bottom", 3, 0, 1); // dock=3 bottom

    // Leader-attached sessions overlay.
    overlay("user.which-key:sessions", "sessions", LEADER);
}

fn kb(mode: Option<&str>, chord: &str, desc: &str, tag: &str) {
    let m = mode.unwrap_or("");
    let (np, nl) = pool(chord);
    let (mp, ml) = pool(m);
    let (dp, dl) = pool(desc);
    let (hp, hl) = pool(tag);
    unsafe { register_keybinding(np, nl, mp, ml, dp, dl, hp, hl) };
}

fn cmd(name: &str, desc: &str, _tag: &str) {
    let (np, nl) = pool(name);
    let (dp, dl) = pool(desc);
    unsafe { register_command(np, nl, dp, dl) };
}

fn register_mode_guest(name: &str) {
    let (np, nl) = pool(name);
    let (kp, kl) = pool("");
    let (ip, il) = pool("");
    let (rp, rl) = pool("");
    unsafe { register_mode(np, nl, kp, kl, ip, il, rp, rl) };
}

fn surface(name: &str, dock: i32, priority: i32, size: i32) {
    let (np, nl) = pool(name);
    unsafe { register_surface(np, nl, dock, priority, size) };
}

fn overlay(name: &str, desc: &str, attach: &str) {
    let (np, nl) = pool(name);
    let (dp, dl) = pool(desc);
    let (rp, rl) = pool("");
    let (kp, kl) = pool("");
    let (ip, il) = pool("");
    let (ap, al) = pool(attach);
    unsafe { register_overlay(np, nl, dp, dl, rp, rl, kp, kl, ip, il, ap, al) };
}

// ─────────────────────────────────────────────────────────────────────────
// Handlers.
// ─────────────────────────────────────────────────────────────────────────

fn handle_key(req: &str) -> String {
    let (snap, name, _bytes) = request(req);

    // Mode-scoped dispatch: the same chord means different things in leader
    // vs pane mode. `n`/`x`/`j`/`k` collide, so route by the active mode.
    let mode = snap.mode.as_str();
    if mode == PANE {
        return match name.as_str() {
            "n" => r#"["ExitMode","SplitDown"]"#.into(),
            "x" => r#"["CloseFocusedPane"]"#.into(),
            "q" => r#"["ExitMode"]"#.into(),
            "h" | "left" => pane_focus("Left"),
            "j" | "down" => pane_focus("Down"),
            "k" | "up" => pane_focus("Up"),
            "l" | "right" => pane_focus("Right"),
            _ => r#"[]"#.into(),
        };
    }

    match name.as_str() {
        "ctrl+b" => {
            // Same chord in normal + leader scope: exit leader when already in it.
            if mode == LEADER {
                r#"["ExitMode"]"#.into()
            } else {
                r#"[{"EnterMode":{"name":"leader"}}]"#.into()
            }
        }
        "ctrl+p" => r#"[{"EnterMode":{"name":"pane"}}]"#.into(),
        // Leader stock map and session nav. Serde uses external tagging on
        // UiAction: unit variants are bare strings, struct variants are
        // {"Variant":{"field":...}}.
        "n" => r#"["ExitMode",{"NewSession":{"name":null}}]"#.into(),
        "d" => r#"["ExitMode","Detach"]"#.into(),
        "?" => r#"["ExitMode",{"OpenOverlay":{"name":"ekko:help"}}]"#.into(),
        "s" => r#"[{"EnterMode":{"name":"scroll"}}]"#.into(),
        "c" => r#"[{"EnterMode":{"name":"command"}}]"#.into(),
        "j" => step_session(&snap, 1),
        "k" => step_session(&snap, -1),
        "x" => kill_session(&snap),
        _ => r#"[]"#.into(),
    }
}

/// Build the JSON for a pane-focus action.
fn pane_focus(dir: &str) -> String {
    format!(r#"[{{"FocusPaneDirection":{{"direction":"{dir}"}}}}]"#)
}

fn handle_command(req: &str) -> String {
    let v: Json = serde_json::from_str(req).unwrap_or(Json::Null);
    let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let args = v.get("args").and_then(|x| x.as_str()).unwrap_or("").to_string();
    // A command handler returns a CommandOutput { actions: Vec<UiAction> }.
    match name.as_str() {
        "pane-new" => r#"{"actions":["SplitDown"]}"#.into(),
        "pane-focus" => {
            let dir = if args.is_empty() { "right" } else { &args };
            // PaneDirection is serde-external-tagged as PascalCase variants.
            let d = match dir {
                "left" => "Left",
                "up" => "Up",
                "down" => "Down",
                _ => "Right",
            };
            format!(r#"{{"actions":[{{"FocusPaneDirection":{{"direction":"{d}"}}}}]}}"#)
        }
        "pane-close" => r#"{"actions":["CloseFocusedPane"]}"#.into(),
        _ => r#"{"actions":[]}"#.into(),
    }
}

fn handle_mode_key(req: &str) -> String {
    let (snap, name, bytes) = request(req);
    let _ = &snap;
    if bytes == [0x1b] {
        return r#""Exit""#.into();
    }
    if name == LEADER && bytes.len() == 1 {
        let c = bytes[0];
        if (0x20..=0x7e).contains(&c) {
            let ch = c as char;
            // ModeOutcome::ExitWith(actions) -> external-tagged {"ExitWith":[...]}.
            return format!(
                r#"{{"ExitWith":[{{"SetStatusNote":{{"text":"leader: '{ch}' is unbound","kind":"Info","ttl_ms":2000}}}}]}}"#
            );
        }
    }
    r#"null"#.into()
}

fn handle_wants_tick(req: &str) -> String {
    let v: Json = serde_json::from_str(req).unwrap_or(Json::Null);
    let has_note = v
        .get("snapshot")
        .and_then(|s| s.get("status_note"))
        .map(|n| !n.is_null())
        .unwrap_or(false);
    let scroll = v
        .get("snapshot")
        .and_then(|s| s.get("scrollback"))
        .and_then(|x| x.as_u64())
        .unwrap_or(0)
        > 0;
    (has_note || scroll).to_string()
}

/// Surface draw: route by which docked surface name.
fn handle_surface_draw(req: &str) {
    let (snap, name, _) = request(req);
    if name.ends_with(":top") {
        draw_top(&snap);
    } else if name.ends_with(":bottom") {
        draw_bottom(&snap);
    }
}

fn handle_overlay_render(req: &str) {
    let (snap, _, _) = request(req);
    draw_sessions(&snap);
}

fn handle_overlay_key(req: &str) -> String {
    let (_, _, bytes) = request(req);
    if bytes == [0x1b] {
        return r#""close""#.into();
    }
    r#"null"#.into()
}

// ─────────────────────────────────────────────────────────────────────────
// Session navigation helpers.
// ─────────────────────────────────────────────────────────────────────────

fn session_names(snap: &Snapshot) -> Vec<String> {
    let mut out = Vec::new();
    for p in &snap.projects {
        for s in &p.sessions {
            out.push(s.name.clone());
        }
    }
    out
}

fn current_index(names: &[String], current: &str) -> Option<usize> {
    names.iter().position(|n| n == current)
}

fn step_session(snap: &Snapshot, delta: isize) -> String {
    let names = session_names(snap);
    if names.len() < 2 {
        return r#"[{"SetStatusNote":{"text":"no other session","kind":"Info","ttl_ms":2000}}]"#.into();
    }
    let i = current_index(&names, &snap.session_name).unwrap_or(0);
    let target = &names[((i as isize + delta).rem_euclid(names.len() as isize)) as usize];
    format!(r#"[{{"SwitchSession":{{"name":"{target}"}}}}]"#)
}

fn kill_session(snap: &Snapshot) -> String {
    let names = session_names(snap);
    if names.len() >= 2 {
        let i = current_index(&names, &snap.session_name).unwrap_or(0);
        let target = &names[(i + 1) % names.len()];
        format!(
            r#"["KillCurrentSession",{{"SwitchSession":{{"name":"{target}"}}}},"ExitMode"]"#
        )
    } else {
        r#"["KillCurrentSession","ExitMode"]"#.into()
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Drawing (set-3 ops; every arg is a (ptr,len) string).
// ─────────────────────────────────────────────────────────────────────────

/// `put_text(x, y, text, fg_color)` — normal weight.
fn text(x: i32, y: i32, s: &str, fg: &str) {
    let (xp, xl) = pool_num(x);
    let (yp, yl) = pool_num(y);
    let (tp, tl) = pool(s);
    let (cp, cl) = pool(fg);
    unsafe { put_text(xp, xl, yp, yl, tp, tl, cp, cl) };
}

/// `fill_rect(x, y, w, h, fg)`.
fn rect(x: i32, y: i32, w: i32, h: i32, fg: &str) {
    let (xp, xl) = pool_num(x);
    let (yp, yl) = pool_num(y);
    let (wp, wl) = pool_num(w);
    let (hp, hl) = pool_num(h);
    let (cp, cl) = pool(fg);
    unsafe { fill_rect(xp, xl, yp, yl, wp, wl, hp, hl, cp, cl) };
}

/// `draw_box(x, y, w, h, border)`.
fn box_(x: i32, y: i32, w: i32, h: i32, fg: &str) {
    let (xp, xl) = pool_num(x);
    let (yp, yl) = pool_num(y);
    let (wp, wl) = pool_num(w);
    let (hp, hl) = pool_num(h);
    let (cp, cl) = pool(fg);
    unsafe { draw_box(xp, xl, yp, yl, wp, wl, hp, hl, cp, cl) };
}

/// Top bar: centred mode ribbon, scroll spinner, right session name/note.
fn draw_top(snap: &Snapshot) {
    let cols = snap.cols as i32;
    if cols < 1 {
        return;
    }
    let mode_label = format!(" {} ", snap.mode.to_uppercase());
    let mode_w = UnicodeWidthStr::width(mode_label.as_str());
    let chip_w = mode_w + if snap.mode == "normal" { 0 } else { 1 };
    let start = 0.max((cols - chip_w as i32) / 2);
    let mode_fg = if snap.mode == "normal" { "success" } else { "status_fg" };
    text(start, 0, &mode_label, mode_fg);

    // Scroll/offset spinner in the top-left corner.
    if snap.scrollback > 0 || snap.mode == "scroll" {
        let frames = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let idx = ((snap.now_ms / 80) as usize) % frames.len();
        text(0, 0, &frames[idx].to_string(), "accent");
    }

    // Right: active note or session name.
    let right = snap
        .status_note
        .as_ref()
        .map(|n| format!(" {} ", n.text))
        .unwrap_or_else(|| snap.session_name.clone());
    let w = UnicodeWidthStr::width(right.as_str());
    if w == 0 {
        return;
    }
    let available = cols - (start + chip_w as i32);
    if available <= 0 {
        return;
    }
    let shown = truncate(&right, available as usize);
    let fg = match snap.status_note.as_ref().map(|n| n.kind.as_str()) {
        Some("error") => "error",
        Some("ok") => "success",
        Some(_) => "accent",
        None => "muted",
    };
    text(cols - UnicodeWidthStr::width(shown.as_str()) as i32, 0, &shown, fg);
}

/// Bottom bar: centred per-mode key-hint tokens `[chord] desc`.
fn draw_bottom(snap: &Snapshot) {
    let cols = snap.cols as i32;
    if cols < 1 {
        return;
    }
    // Collect the hints valid for the current mode, deduped by description.
    let mut entries: Vec<(String, String)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for kb in &snap.keybindings {
        let eligible = (snap.mode == "normal" && kb.mode.is_none())
            || (snap.mode != "normal" && kb.mode.as_deref() == Some(snap.mode.as_str()));
        if eligible && kb.description != "close panel" && !kb.description.is_empty() {
            if seen.insert(kb.description.clone()) {
                entries.push((kb.chord_text.clone(), kb.description.clone()));
            }
        }
    }
    if entries.is_empty() {
        return;
    }
    // Prefer the with-description layout; fall back to chords only if it
    // overflows.
    let mut tokens: Vec<String> = Vec::new();
    let mut total = 0i32;
    let mut n = 0usize;
    for (chord, desc) in &entries {
        let t = format!("[{}] {}", chord, desc);
        let tw = UnicodeWidthStr::width(t.as_str()) as i32 + (if n > 0 { 2 } else { 0 });
        if total + tw > cols {
            break;
        }
        total += tw;
        n += 1;
        tokens.push(t);
    }
    if n < entries.len() {
        tokens.clear();
        total = 0;
        n = 0;
        for (chord, _desc) in &entries {
            let t = format!("[{chord}]");
            let tw = UnicodeWidthStr::width(t.as_str()) as i32 + (if n > 0 { 2 } else { 0 });
            if total + tw > cols {
                break;
            }
            total += tw;
            n += 1;
            tokens.push(t);
        }
    }
    let x = 0.max((cols - total) / 2);
    let mut cx = x;
    for (i, t) in tokens.iter().enumerate() {
        text(cx, 0, t, "text");
        cx += UnicodeWidthStr::width(t.as_str()) as i32;
        if i + 1 < tokens.len() {
            cx += 2;
        }
    }
}

/// Leader-attached centred session-list panel.
fn draw_sessions(snap: &Snapshot) {
    let cols = snap.cols as i32;
    let rows = snap.rows as i32;
    if cols < 12 || rows < 5 {
        return;
    }
    let names = session_names(snap);
    let count_usize = names.len();
    let count = count_usize as i32;
    let longest = names.iter().map(|s| UnicodeWidthStr::width(s.as_str())).max().unwrap_or(0);
    let name_w = longest.max(1) as i32;
    let width = cols.min(name_w + 5);
    let name_w = (width - 5).max(1);
    let capacity = (rows - 2).max(1);
    let vis = capacity.min(count.max(1));
    let height = vis + 2;
    let col = (cols - width) / 2;
    let row = (rows - height) / 3;

    box_(col, row, width, height, "border");
    text(col + 2, row, " sessions ", "heading");
    if count == 0 {
        text(col + 3, row + 1, "no sessions", "muted");
        return;
    }
    let cur = current_index(&names, &snap.session_name).unwrap_or(0) as i32;
    let from_top = (0i32).max(cur - vis / 2).min((count - vis).max(0));
    for offset in 0..vis {
        let index = from_top + offset;
        if index >= count {
            break;
        }
        let r = row + 1 + offset;
        let uidx = index as usize;
        let marker = if index == cur { "▌" } else { " " };
        let is_cur = index == cur;
        text(col + 1, r, marker, if is_cur { "accent" } else { "muted" });
        let shown = truncate(&names[uidx], name_w as usize);
        text(col + 3, r, &shown, if is_cur { "text" } else { "muted" });
    }
    if count > vis {
        let (xp, xl) = pool_num(col + width - 1);
        let (yp, yl) = pool_num(row + 1);
        let (vp, vl) = pool_num(vis);
        let (tp, tl) = pool_num(count as i32);
        let (fp, fl) = pool("accent");
        let (gp, gl) = pool("surface_raised");
        unsafe { render_scrollbar(xp, xl, yp, yl, vp, vl, tp, tl, fp, fl, gp, gl) };
    }
}

/// Truncate `s` to at most `max` display cells, appending an ellipsis.
fn truncate(s: &str, max: usize) -> String {
    if max <= 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0usize;
    for ch in s.chars() {
        let cw = UnicodeWidthStr::width(ch.to_string().as_str());
        if w + cw + 1 > max {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}