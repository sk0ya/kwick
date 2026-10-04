use crate::providers::{Action, Item};
use eframe::egui;
use mlua::{Function, Lua, Table, Value};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};

type HttpResult = (u64, Result<crate::http::Response, String>);

/// Hosts user plugins written in Lua.
///
/// Plugin API (see the README and plugins/calc.lua):
/// ```lua
/// kwick.register{
///     name = "myplugin",
///     on_query = function(query)
///         return { { title = "...", subtitle = "...", cmd = "..." } }
///     end,
/// }
/// ```
/// Each result may carry one of:
///   run = function() ... end       -- arbitrary Lua callback
///   cmd = "notepad", args = "..."  -- ShellExecute
///   url = "https://..."            -- open in browser
///   copy = "text"                  -- copy to the clipboard
///   items = { ... } / function() return { ... } end  -- a sub-list (Enter opens it)
/// plus optional `icon` (file whose icon to show) and `actions` (more
/// entries for the action panel, each a table like a result).
pub struct LuaHost {
    _lua: Lua,
    plugins: Rc<RefCell<Vec<Table>>>,
    /// `run` callbacks of the items handed out since the last query.
    actions: Vec<Function>,
    /// Sub-lists (`items`) of the items handed out since the last query.
    pages: Vec<(String, Value)>,
    pub errors: Vec<String>,
    http_rx: Receiver<HttpResult>,
    callbacks: Rc<RefCell<HashMap<u64, Function>>>,
    refresh: Rc<Cell<bool>>,
}

impl LuaHost {
    pub fn new(plugin_dir: &Path, egui_ctx: egui::Context, settings: &toml::Table) -> Self {
        let lua = Lua::new();
        let plugins: Rc<RefCell<Vec<Table>>> = Rc::new(RefCell::new(Vec::new()));
        let callbacks: Rc<RefCell<HashMap<u64, Function>>> = Rc::default();
        let refresh: Rc<Cell<bool>> = Rc::default();
        let (http_tx, http_rx) = mpsc::channel::<HttpResult>();
        let mut errors = Vec::new();

        if let Err(e) = install_api(
            &lua,
            plugins.clone(),
            callbacks.clone(),
            refresh.clone(),
            http_tx,
            egui_ctx,
            settings,
        ) {
            errors.push(format!("lua setup: {e}"));
        }

        if let Ok(entries) = std::fs::read_dir(plugin_dir) {
            let mut paths: Vec<_> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("lua"))
                .collect();
            paths.sort();
            for path in paths {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                match std::fs::read_to_string(&path) {
                    Ok(code) => {
                        if let Err(e) = lua.load(&code).set_name(&name).exec() {
                            errors.push(format!("{name}: {e}"));
                        }
                    }
                    Err(e) => errors.push(format!("{name}: {e}")),
                }
            }
        }

        for err in &errors {
            eprintln!("kwick plugin error: {err}");
        }

        Self {
            _lua: lua,
            plugins,
            actions: Vec::new(),
            pages: Vec::new(),
            errors,
            http_rx,
            callbacks,
            refresh,
        }
    }

    pub fn query(&mut self, q: &str) -> Vec<Item> {
        self.actions.clear();
        self.pages.clear();
        let mut out = Vec::new();
        let plugins: Vec<Table> = self.plugins.borrow().clone();
        for plugin in plugins {
            let Ok(on_query) = plugin.get::<Function>("on_query") else {
                continue;
            };
            let results: Table = match on_query.call(q) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("kwick plugin on_query error: {e}");
                    continue;
                }
            };
            out.extend(self.items_from(results));
        }
        out
    }

    fn items_from(&mut self, results: Table) -> Vec<Item> {
        results
            .sequence_values::<Table>()
            .flatten()
            .filter_map(|r| self.item_from(&r))
            .collect()
    }

    /// The action a result (or an `actions` entry) describes.
    fn action_from(&mut self, r: &Table, title: &str) -> Option<Action> {
        if let Ok(run) = r.get::<Function>("run") {
            self.actions.push(run);
            let idx = self.actions.len() - 1;
            // keep_open: run without hiding the launcher (results update later).
            if r.get::<bool>("keep_open").unwrap_or(false) {
                Some(Action::LuaKeep(idx))
            } else {
                Some(Action::Lua(idx))
            }
        } else if let Ok(cmd) = r.get::<String>("cmd") {
            Some(Action::Exec {
                cmd,
                args: r.get("args").ok(),
            })
        } else if let Ok(url) = r.get::<String>("url") {
            Some(Action::Url(url))
        } else if let Ok(text) = r.get::<String>("copy") {
            Some(Action::Copy(text))
        } else {
            match r.get::<Value>("items") {
                Ok(v @ (Value::Table(_) | Value::Function(_))) => {
                    self.pages.push((title.to_string(), v));
                    Some(Action::LuaPage(self.pages.len() - 1))
                }
                _ => None,
            }
        }
    }

    fn item_from(&mut self, r: &Table) -> Option<Item> {
        let title: String = r.get("title").ok()?;
        let subtitle: String = r.get("subtitle").unwrap_or_default();
        let action = self.action_from(r, &title)?;
        let mut item = Item::new(title, subtitle, action).transient();
        if let Ok(icon) = r.get::<String>("icon") {
            item.icon_path = Some(icon);
        }
        if let Ok(actions) = r.get::<Table>("actions") {
            for entry in actions.sequence_values::<Table>().flatten() {
                let Ok(label) = entry.get::<String>("title") else { continue };
                if let Some(action) = self.action_from(&entry, &label) {
                    item.extra.push((label, action));
                }
            }
        }
        Some(item)
    }

    /// Open a sub-list: (its title, its items).
    pub fn open_page(&mut self, idx: usize) -> Option<(String, Vec<Item>)> {
        let (title, value) = self.pages.get(idx)?.clone();
        let table = match value {
            Value::Table(t) => t,
            Value::Function(f) => match f.call::<Table>(()) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("kwick plugin items error: {e}");
                    return None;
                }
            },
            _ => return None,
        };
        Some((title, self.items_from(table)))
    }

    pub fn run(&self, idx: usize) {
        if let Some(f) = self.actions.get(idx) {
            if let Err(e) = f.call::<()>(()) {
                eprintln!("kwick plugin run error: {e}");
            }
        }
    }

    /// Deliver finished HTTP requests to their callbacks. Returns true when
    /// a plugin asked for the results to be refreshed (kwick.refresh()).
    pub fn poll(&mut self) -> bool {
        while let Ok((id, result)) = self.http_rx.try_recv() {
            let Some(callback) = self.callbacks.borrow_mut().remove(&id) else {
                continue;
            };
            let response = (|| -> mlua::Result<Table> {
                let t = self._lua.create_table()?;
                match result {
                    Ok(r) => {
                        t.set("status", r.status)?;
                        t.set("ok", (200..300).contains(&r.status))?;
                        t.set("body", self._lua.create_string(&r.body)?)?;
                    }
                    Err(e) => {
                        t.set("status", 0)?;
                        t.set("ok", false)?;
                        t.set("error", e)?;
                    }
                }
                Ok(t)
            })();
            match response {
                Ok(t) => {
                    if let Err(e) = callback.call::<()>(t) {
                        eprintln!("kwick plugin http callback error: {e}");
                    }
                }
                Err(e) => eprintln!("kwick plugin http: {e}"),
            }
        }
        self.refresh.replace(false)
    }
}

/// Console programs write UTF-8 (most modern tools) or the system code page
/// (cmd built-ins, older tools: CP932 on Japanese Windows).
fn decode_output(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_string();
    }
    use windows::Win32::Globalization::{MultiByteToWideChar, CP_ACP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS};
    unsafe {
        let flags = MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0);
        let len = MultiByteToWideChar(CP_ACP, flags, bytes, None);
        if len <= 0 {
            return String::from_utf8_lossy(bytes).into_owned();
        }
        let mut wide = vec![0u16; len as usize];
        MultiByteToWideChar(CP_ACP, flags, bytes, Some(&mut wide));
        String::from_utf16_lossy(&wide)
    }
}

fn toml_to_lua(lua: &Lua, v: &toml::Value) -> mlua::Result<Value> {
    Ok(match v {
        toml::Value::String(s) => Value::String(lua.create_string(s)?),
        toml::Value::Integer(i) => Value::Integer(*i),
        toml::Value::Float(f) => Value::Number(*f),
        toml::Value::Boolean(b) => Value::Boolean(*b),
        toml::Value::Datetime(d) => Value::String(lua.create_string(d.to_string())?),
        toml::Value::Array(a) => {
            let t = lua.create_table()?;
            for (i, v) in a.iter().enumerate() {
                t.raw_set(i + 1, toml_to_lua(lua, v)?)?;
            }
            Value::Table(t)
        }
        toml::Value::Table(m) => {
            let t = lua.create_table()?;
            for (k, v) in m {
                t.set(k.as_str(), toml_to_lua(lua, v)?)?;
            }
            Value::Table(t)
        }
    })
}

fn install_api(
    lua: &Lua,
    plugins: Rc<RefCell<Vec<Table>>>,
    callbacks: Rc<RefCell<HashMap<u64, Function>>>,
    refresh: Rc<Cell<bool>>,
    http_tx: Sender<HttpResult>,
    egui_ctx: egui::Context,
    settings: &toml::Table,
) -> mlua::Result<()> {
    let kwick = lua.create_table()?;
    kwick.set(
        "register",
        lua.create_function(move |_, t: Table| {
            plugins.borrow_mut().push(t);
            Ok(())
        })?,
    )?;
    kwick.set(
        "copy",
        lua.create_function(|_, s: String| Ok(crate::clipboard::set_text(&s)))?,
    )?;
    kwick.set(
        "open",
        lua.create_function(|_, (target, args): (String, Option<String>)| {
            crate::launch::shell_open(&target, args.as_deref());
            Ok(())
        })?,
    )?;
    kwick.set(
        "notify",
        lua.create_function(|_, (title, text): (String, Option<String>)| {
            crate::notify::show(&title, text.as_deref().unwrap_or(""));
            Ok(())
        })?,
    )?;
    kwick.set(
        "refresh",
        lua.create_function(move |_, ()| {
            refresh.set(true);
            Ok(())
        })?,
    )?;
    // kwick.exec("git status") -> stdout, exit code. Blocks: keep it short.
    kwick.set(
        "exec",
        lua.create_function(|_, cmd: String| {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let output = std::process::Command::new("cmd")
                .raw_arg(format!("/D /S /C \"{cmd}\""))
                .creation_flags(CREATE_NO_WINDOW)
                .output()
                .map_err(mlua::Error::external)?;
            Ok((decode_output(&output.stdout), output.status.code().unwrap_or(-1)))
        })?,
    )?;
    // kwick.http({url=, method=, headers={}, body=}, function(res) ... end)
    // res = {ok=, status=, body=, error=}; the callback runs on the UI
    // thread once the response is in. A plain URL string works for GET.
    let next_id = Cell::new(0u64);
    kwick.set(
        "http",
        lua.create_function(move |_, (opts, callback): (Value, Function)| {
            let (url, method, headers, body) = match opts {
                Value::String(s) => (s.to_str()?.to_string(), None, Vec::new(), Vec::new()),
                Value::Table(t) => {
                    let mut headers = Vec::new();
                    if let Ok(h) = t.get::<Table>("headers") {
                        for pair in h.pairs::<String, String>() {
                            let (k, v) = pair?;
                            headers.push(format!("{k}: {v}"));
                        }
                    }
                    let body = t
                        .get::<Option<mlua::String>>("body")?
                        .map(|b| b.as_bytes().to_vec())
                        .unwrap_or_default();
                    (t.get::<String>("url")?, t.get::<Option<String>>("method")?, headers, body)
                }
                _ => return Err(mlua::Error::external("kwick.http: url か表を渡してください")),
            };
            let method = method.unwrap_or_else(|| if body.is_empty() { "GET" } else { "POST" }.into());
            let id = next_id.get();
            next_id.set(id + 1);
            callbacks.borrow_mut().insert(id, callback);
            let tx = http_tx.clone();
            let ctx = egui_ctx.clone();
            std::thread::spawn(move || {
                let result = crate::http::send(&crate::http::Request {
                    method: &method,
                    url: &url,
                    headers: &headers,
                    body: &body,
                });
                let _ = tx.send((id, result));
                ctx.request_repaint();
            });
            Ok(())
        })?,
    )?;
    kwick.set(
        "json_decode",
        lua.create_function(|lua, text: String| match crate::json::decode(lua, &text) {
            Ok(v) => Ok((v, Value::Nil)),
            Err(e) => Ok((Value::Nil, Value::String(lua.create_string(e)?))),
        })?,
    )?;
    kwick.set(
        "json_encode",
        lua.create_function(|_, v: Value| crate::json::encode(&v).map_err(mlua::Error::external))?,
    )?;
    // kwick.settings("myplugin") -> the [plugins.myplugin] table of config.toml
    let settings = toml_to_lua(lua, &toml::Value::Table(settings.clone()))?;
    kwick.set(
        "settings",
        lua.create_function(move |lua, name: String| match &settings {
            Value::Table(t) => Ok(t
                .get::<Option<Table>>(name)?
                .map(Value::Table)
                .unwrap_or(Value::Table(lua.create_table()?))),
            _ => Ok(Value::Nil),
        })?,
    )?;
    lua.globals().set("kwick", kwick)?;
    Ok(())
}
