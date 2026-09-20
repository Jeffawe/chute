mod config;
mod menus;
pub mod tailscale;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_notification::NotificationExt;
use tokio::io::{AsyncBufReadExt, BufReader};

use config::Config;
use tailscale::Target;

#[derive(Default)]
struct AppState {
    cfg: Mutex<Config>,
    /// Handle to the supervisor task. Dropping it kills the child, because the
    /// receiver is spawned with `kill_on_drop`.
    receiver: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    /// Online device names last written to the shell menus.
    menu_signature: Mutex<Option<String>>,
}

/// Handle the non-GUI invocations used by the shell integration.
///
/// Returns `Some(exit_code)` when the arguments were handled here, so a
/// right-click send never pays the cost of starting a webview.
pub fn run_cli() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--remove-menus") {
        menus::remove();
        return Some(0);
    }

    if args.iter().any(|a| a == "--install-menus") {
        let Ok(rt) = tokio::runtime::Runtime::new() else {
            return Some(1);
        };
        return match rt.block_on(tailscale::targets()) {
            Ok(targets) => {
                let errors = menus::regenerate(&targets);
                for e in &errors {
                    eprintln!("menu error: {e}");
                }
                println!(
                    "wrote entries for {} online device(s)",
                    targets.iter().filter(|t| t.online).count()
                );
                Some(if errors.is_empty() { 0 } else { 1 })
            }
            Err(e) => {
                eprintln!("{e}");
                Some(1)
            }
        };
    }

    let pos = args.iter().position(|a| a == "--send-to")?;
    let Some(device) = args.get(pos + 1).cloned() else {
        eprintln!("--send-to requires a device name");
        return Some(2);
    };
    let files: Vec<PathBuf> = args[pos + 2..].iter().map(PathBuf::from).collect();

    let missing: Option<&PathBuf> = files.iter().find(|p| !p.is_file());
    if files.is_empty() || missing.is_some() {
        let msg = match missing {
            // Inside a Flatpak an existing file can still be unreadable: the
            // sandbox only sees the paths it was granted. Saying "not a file"
            // alone sends people hunting for a problem with the file itself.
            Some(p) if std::env::var_os("FLATPAK_ID").is_some() => format!(
                "Cannot read {} - it is outside the paths this sandbox can see",
                p.display()
            ),
            Some(p) => format!("Not a file: {}", p.display()),
            None => "No files given".to_string(),
        };
        eprintln!("{msg}");
        notify("Chute send failed", &msg);
        return Some(2);
    }

    let Ok(rt) = tokio::runtime::Runtime::new() else {
        return Some(1);
    };
    match rt.block_on(tailscale::send(&files, &device)) {
        Ok(()) => {
            let what = if files.len() == 1 {
                files[0]
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "1 file".into())
            } else {
                format!("{} files", files.len())
            };
            notify("Sent with Chute", &format!("{what} \u{2192} {device}"));
            Some(0)
        }
        Err(e) => {
            eprintln!("{e}");
            notify("Chute send failed", &e.to_string());
            Some(1)
        }
    }
}

/// Desktop notification from the headless path, where no Tauri app exists.
fn notify(title: &str, body: &str) {
    let _ = notify_rust::Notification::new()
        .summary(title)
        .body(body)
        .icon("document-send")
        .show();
}

#[tauri::command]
async fn list_targets(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<Target>, tailscale::Error> {
    let targets = tailscale::targets().await?;

    // The UI polls this every few seconds; only touch the filesystem when the
    // set of reachable devices has actually changed.
    let signature = targets
        .iter()
        .filter(|t| t.online)
        .map(|t| t.name.as_str())
        .collect::<Vec<_>>()
        .join("\u{1f}");

    let enabled = state.cfg.lock().unwrap().shell_menus;
    let changed = {
        let mut last = state.menu_signature.lock().unwrap();
        if last.as_deref() == Some(signature.as_str()) {
            false
        } else {
            *last = Some(signature);
            true
        }
    };
    if enabled && changed {
        for err in menus::regenerate(&targets) {
            eprintln!("menu error: {err}");
            let _ = app.emit("menu-error", err);
        }
    }

    Ok(targets)
}

#[tauri::command]
async fn send_files(paths: Vec<PathBuf>, target: String) -> Result<(), tailscale::Error> {
    tailscale::send(&paths, &target).await
}

#[tauri::command]
fn get_config(state: State<'_, AppState>) -> Config {
    state.cfg.lock().unwrap().clone()
}

#[tauri::command]
fn set_config(app: AppHandle, state: State<'_, AppState>, cfg: Config) -> Result<(), String> {
    let disabling_menus = !cfg.shell_menus;
    config::save(&app, &cfg)?;
    *state.cfg.lock().unwrap() = cfg;
    if disabling_menus {
        menus::remove();
    }
    // Force a rewrite on the next poll either way.
    *state.menu_signature.lock().unwrap() = None;
    restart_receiver(&app);
    Ok(())
}

/// Surface frontend errors in the app's stderr; the webview console is not
/// visible when running outside a dev server.
#[tauri::command]
fn log_js(message: String) {
    eprintln!("[webview] {message}");
}

#[tauri::command]
fn get_autostart(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    let launcher = app.autolaunch();
    if enabled {
        launcher.enable()
    } else {
        launcher.disable()
    }
    .map_err(|e| e.to_string())
}

/// Hand file arguments to the UI for staging.
///
/// This is how the shell integration will talk to a running app: the
/// right-click entry launches the binary with paths, the single-instance hook
/// forwards them here rather than starting a second copy.
fn queue_from_argv(app: &AppHandle, argv: &[String]) {
    let files: Vec<PathBuf> = argv
        .iter()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .collect();
    if !files.is_empty() {
        let _ = app.emit("files-queued", files);
    }
}

/// Bring the window back from the tray.
fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Path of the file recording the running receiver's PID.
fn pidfile(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("receiver.pid"))
}

/// Kill a receiver left behind by a previous run.
///
/// PDEATHSIG should make this unnecessary, but a receiver that outlives us
/// silently steals incoming files from the next one, and the cost of checking
/// is a single file read.
fn reap_stale_receiver(app: &AppHandle) {
    let Some(pf) = pidfile(app) else { return };
    let Ok(text) = std::fs::read_to_string(&pf) else { return };
    if let Ok(pid) = text.trim().parse::<u32>() {
        if tailscale::is_receiver(pid) {
            // SAFETY: plain kill(2) on a PID we just confirmed is our own
            // receiver process.
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
        }
    }
    let _ = std::fs::remove_file(&pf);
}

/// Tear down any running receiver and start a fresh one if enabled.
fn restart_receiver(app: &AppHandle) {
    let state = app.state::<AppState>();
    if let Some(handle) = state.receiver.lock().unwrap().take() {
        handle.abort();
    }

    let cfg = state.cfg.lock().unwrap().clone();
    if !cfg.auto_receive {
        return;
    }

    let app2 = app.clone();
    let handle = tauri::async_runtime::spawn(supervise(app2, cfg.download_dir));
    *state.receiver.lock().unwrap() = Some(handle);
}

/// Keep `tailscale file get --loop` alive, emitting an event per arrival.
///
/// The loop is not supposed to exit on its own, so any exit is treated as a
/// fault and retried with backoff. A child that stayed up a while is counted
/// as healthy, which stops a slow crash-loop from escalating to the cap and
/// staying there.
async fn supervise(app: AppHandle, dir: PathBuf) {
    const MIN: Duration = Duration::from_secs(1);
    const MAX: Duration = Duration::from_secs(60);
    let mut backoff = MIN;

    loop {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            let _ = app.emit("receiver-error", format!("{}: {e}", dir.display()));
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(MAX);
            continue;
        }

        let started = Instant::now();
        match tailscale::spawn_receiver(&dir) {
            Ok(mut child) => {
                if let (Some(pf), Some(pid)) = (pidfile(&app), child.id()) {
                    if let Some(parent) = pf.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(&pf, pid.to_string());
                }
                if let Some(stdout) = child.stdout.take() {
                    let mut lines = BufReader::new(stdout).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        if let Some(received) = tailscale::parse_received(&line, &dir) {
                            // Fired from the backend rather than the webview so
                            // arrivals still surface with the window closed.
                            let _ = app
                                .notification()
                                .builder()
                                .title("File received")
                                .body(&received.name)
                                .show();
                            let _ = app.emit("file-received", received);
                        }
                    }
                }
                let _ = child.wait().await;
                if let Some(pf) = pidfile(&app) {
                    let _ = std::fs::remove_file(pf);
                }
            }
            Err(e) => {
                let _ = app.emit("receiver-error", e.to_string());
            }
        }

        if started.elapsed() > Duration::from_secs(30) {
            backoff = MIN;
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Registered first, as the plugin requires. Two copies of the app would
        // mean two receivers draining the same inbox, so this is a correctness
        // guard, not just a UX nicety.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            show_window(app);
            queue_from_argv(app, &argv);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(AppState::default())
        .setup(|app| {
            let handle = app.handle().clone();
            let loaded = config::load(&handle);
            *handle.state::<AppState>().cfg.lock().unwrap() = loaded;
            reap_stale_receiver(&handle);
            restart_receiver(&handle);

            let open_item = MenuItem::with_id(app, "open", "Open Chute", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open_item, &quit_item])?;

            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Chute")
                .menu(&menu)
                // Left click should open the window, not pop the menu.
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_window(tray.app_handle());
                    }
                })
                .build(app)?;

            Ok(())
        })
        // Closing the window only hides it: the receiver has to keep running,
        // and quitting is done deliberately from the tray.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            list_targets,
            send_files,
            get_config,
            set_config,
            get_autostart,
            set_autostart,
            log_js
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
