mod agent_link;
mod app;
mod bundled;
mod catalog;
mod correction;
mod engine;
mod exports;
mod first_reading;
mod forget;
mod ghost;
mod jobs;
mod local;
mod mirror;
mod models;
mod plan;
mod providers;
mod scroll;
mod seal;
mod seed;
mod sources;
mod store;
mod transport;
mod weights;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use store::*;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Emitter, Manager,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

#[tauri::command]
async fn notebook(
    store: tauri::State<'_, Arc<Store>>,
    operation: String,
    args: serde_json::Value,
) -> Res<serde_json::Value> {
    let s = store.inner().clone();
    tauri::async_runtime::spawn_blocking(move || dispatch(&s, &operation, &args))
        .await
        .msg()?
}
fn dispatch(s: &Arc<Store>, op: &str, args: &serde_json::Value) -> Res<serde_json::Value> {
    if op == "beginFirstReading" {
        first_reading::start(s, Some(args["notify"] == true))?;
        return Ok(serde_json::Value::Null);
    }
    app::action(s, op, args)
}
fn show(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}
fn fresh_sheet(app: &tauri::AppHandle) {
    show(app);
    if app.state::<Arc<Store>>().get("onboarded").ok() == Some(serde_json::json!(true)) {
        let _ = app.emit("fresh-sheet", ());
    }
}
static QUITTING: AtomicBool = AtomicBool::new(false);
pub(crate) fn quitting() -> bool {
    QUITTING.load(Ordering::SeqCst)
}
fn shutdown() {
    QUITTING.store(true, Ordering::SeqCst);
    bundled::reap(true);
}
fn main() {
    let result = run();
    shutdown();
    if let Err(e) = result {
        eprintln!("Sumi: {e}");
        std::process::exit(1)
    }
}
fn run() -> Res<()> {
    let args: Vec<String> = std::env::args().collect();
    let param = |k: &str| {
        args.iter()
            .position(|x| x == k)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let root = param("--notebook")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| sources::home().join("Sumi"));
    if args.iter().any(|x| x == "--mcp") {
        let f = param("--credential").ok_or("Missing agent credential.")?;
        let token = std::fs::read_to_string(f).msg()?;
        if transport::relay(&root, token.trim())? {
            return Ok(());
        }
        return seal::serve(&Store::open(root)?, token.trim());
    }
    #[cfg(debug_assertions)]
    if args.iter().any(|x| x == "--fixture-rpc") {
        return fixture_rpc(root, args.iter().any(|v| v == "--fixture-agents"));
    }
    let s = Arc::new(Store::open(root)?);
    app::setup(&s)?;
    tauri::Builder::default()
        .manage(s.clone())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _, e| {
                    if e.state() == ShortcutState::Pressed {
                        fresh_sheet(app);
                    }
                })
                .build(),
        )
        .setup(move |app| {
            let show_item = MenuItem::with_id(app, "open", "Open Sumi", true, None::<&str>)?;
            let fresh = MenuItem::with_id(app, "fresh", "New Desk Sheet", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Sumi", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &fresh, &quit])?;
            TrayIconBuilder::with_id("sumi")
                .title("◦")
                .tooltip("Sumi")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show(app),
                    "fresh" => fresh_sheet(app),
                    "quit" => {
                        shutdown();
                        app.exit(0)
                    }
                    _ => {}
                })
                .build(app)?;
            s.set(
                "globalShortcutReady",
                &serde_json::json!(app.global_shortcut().register("Alt+Space").is_ok()),
            )?;
            if let Err(e) = transport::start_socket(s.clone()) {
                let _ = s.set("backgroundIssue", &serde_json::json!(e));
            }
            if let Err(e) = transport::start(s.clone()) {
                let _ = s.set("backgroundIssue", &serde_json::json!(e));
            }
            first_reading::start(&s, None).map_err(std::io::Error::other)?;
            jobs::start(s.clone());
            let handle = app.handle().clone();
            let db = s.clone();
            std::thread::spawn(move || {
                let mut last_revision = (0, 0);
                let mut disk_edits = scroll::DiskEdits::default();
                let mut tray_waiting = false;
                let mut arrival = exports::Arrival::default();
                let downloads = sources::home().join("Downloads");
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(15));
                    bundled::reap(false);
                    let _ = arrival.check(&db, &downloads);
                    if let Ok(waiting) = db.agent_waiting() {
                        if waiting != tray_waiting {
                            if let Some(t) = handle.tray_by_id("sumi") {
                                if t.set_title(Some(if waiting { "●" } else { "◦" })).is_ok() {
                                    tray_waiting = waiting;
                                }
                            }
                        }
                    }
                    if let Ok(_g) = db.edit.try_lock() {
                        let _ = disk_edits.check(&db);
                    }
                    let revision = {
                        let c = db.db.lock().unwrap();
                        (
                            c.total_changes(),
                            c.query_row("PRAGMA data_version", [], |r| r.get::<_, i64>(0))
                                .unwrap_or(0),
                        )
                    };
                    if revision != last_revision {
                        last_revision = revision;
                        let _ = handle.emit("notebook-changed", ());
                    }
                }
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![notebook])
        .build(tauri::generate_context!())
        .msg()?
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                shutdown();
            }
            if let tauri::RunEvent::Reopen {
                has_visible_windows: false,
                ..
            } = event
            {
                show(app);
            }
        });
    Ok(())
}
#[cfg(debug_assertions)]
fn fixture_rpc(root: std::path::PathBuf, agents: bool) -> Res<()> {
    use std::io::{BufRead, Write};
    if !root.starts_with(std::env::temp_dir()) && !root.starts_with("/tmp") {
        return Err("Fixture notebooks require a temporary folder.".into());
    }
    let s = Arc::new(Store::with_key(root, &[19; 32])?);
    if std::env::args().any(|v| v == "--fixture-predictions") {
        let predictions: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(s.root.join("predictions.json")).msg()?).msg()?;
        for v in predictions {
            s.put("prediction", &v)?;
        }
    }
    if agents {
        transport::start_socket(s.clone())?;
    }
    app::setup(&s)?;
    first_reading::start(&s, None)?;
    let mut arrival = exports::Arrival::default();
    let downloads = s.root.join("Downloads");
    for line in std::io::stdin().lock().lines() {
        let req: serde_json::Value = serde_json::from_str(&line.msg()?).msg()?;
        let op = req["operation"].as_str().unwrap_or("");
        // Keep a fixture connection inside its temporary notebook even if a UI test
        // clicks a real agent's Connect button.
        if op == "state" {
            arrival.check(&s, &downloads)?;
        }
        let result = if op == "exportLink" {
            exports::request(&s)
        } else if op == "exports" {
            exports::find(&s, &downloads)
        } else if op == "acceptExport" {
            exports::accept(&s, &downloads, req["args"]["path"].as_str().unwrap_or(""))
        } else if op == "connect" {
            seal::connect(
                &s,
                req["args"]["name"].as_str().unwrap_or(""),
                &s.root.join("agent-config"),
                &std::env::current_exe().msg()?,
            )
        } else if op == "discover" {
            Ok(serde_json::json!([]))
        } else if op == "pickSource"
            && req["args"]["kind"] == "export"
            && s.root.join("conversations.json").is_file()
        {
            app::register(
                &s,
                "fixture-export",
                &s.root.join("conversations.json"),
                "Studio journal · example conversations",
            )
        } else {
            dispatch(&s, op, &req["args"])
        };
        writeln!(
            std::io::stdout(),
            "{}",
            match result {
                Ok(v) => serde_json::json!({"ok":v}),
                Err(e) => serde_json::json!({"error":e}),
            }
        )
        .msg()?
    }
    Ok(())
}

#[cfg(test)]
mod checks;
