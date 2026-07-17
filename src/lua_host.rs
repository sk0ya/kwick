use crate::providers::{Action, Item};
use eframe::egui;
use mlua::{Function, Lua, Table};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

/// Hosts user plugins written in Lua.
///
/// Plugin API (see plugins/calc.lua for an example):
/// ```lua
/// kwick.register{
///     name = "myplugin",
///     on_query = function(query)
///         return { { title = "...", subtitle = "...", cmd = "..." } }
///     end,
/// }
/// ```
/// Each result may carry one of:
///   run = function() ... end  -- arbitrary Lua callback
///   cmd = "notepad", args = "..."  -- ShellExecute
///   url = "https://..."            -- open in browser
pub struct LuaHost {
    _lua: Lua,
    plugins: Rc<RefCell<Vec<Table>>>,
    /// `run` callbacks for the items returned by the latest query.
    actions: Vec<Function>,
    pub errors: Vec<String>,
}

impl LuaHost {
    pub fn new(plugin_dir: &Path, egui_ctx: egui::Context) -> Self {
        let lua = Lua::new();
        let plugins: Rc<RefCell<Vec<Table>>> = Rc::new(RefCell::new(Vec::new()));
        let mut errors = Vec::new();

        let setup = || -> mlua::Result<()> {
            let kwick = lua.create_table()?;
            let reg = plugins.clone();
            kwick.set(
                "register",
                lua.create_function(move |_, t: Table| {
                    reg.borrow_mut().push(t);
                    Ok(())
                })?,
            )?;
            kwick.set(
                "copy",
                lua.create_function(move |_, s: String| {
                    egui_ctx.copy_text(s);
                    Ok(())
                })?,
            )?;
            lua.globals().set("kwick", kwick)?;
            Ok(())
        };
        if let Err(e) = setup() {
            errors.push(format!("lua setup: {e}"));
        }

        if let Ok(entries) = std::fs::read_dir(plugin_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("lua") {
                    continue;
                }
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
            errors,
        }
    }

    pub fn query(&mut self, q: &str) -> Vec<Item> {
        self.actions.clear();
        let mut out = Vec::new();
        for plugin in self.plugins.borrow().iter() {
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
            for r in results.sequence_values::<Table>() {
                let Ok(r) = r else { continue };
                let Ok(title) = r.get::<String>("title") else {
                    continue;
                };
                let subtitle: String = r.get("subtitle").unwrap_or_default();
                let action = if let Ok(run) = r.get::<Function>("run") {
                    self.actions.push(run);
                    Action::Lua(self.actions.len() - 1)
                } else if let Ok(cmd) = r.get::<String>("cmd") {
                    Action::Exec {
                        cmd,
                        args: r.get("args").ok(),
                    }
                } else if let Ok(url) = r.get::<String>("url") {
                    Action::Url(url)
                } else {
                    continue;
                };
                out.push(Item::new(title, subtitle, action));
            }
        }
        out
    }

    pub fn run(&self, idx: usize) {
        if let Some(f) = self.actions.get(idx) {
            if let Err(e) = f.call::<()>(()) {
                eprintln!("kwick plugin run error: {e}");
            }
        }
    }
}
