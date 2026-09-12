mod audio;

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::menu::{CheckMenuItem, Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, State, WebviewWindow, Wry};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

// ---------------------------------------------------------------- config ----

/// How a clip's rate walks while pitch mod is engaged. Rate is pitch here, so
/// a downward step also slows the clip down, which is the joke.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", tag = "mode")]
pub enum Pitch {
    /// Nth consecutive press plays at `1.0 + step * n`. Negative walks down.
    Step { step: f32, min: f32, max: f32 },
    /// Every press picks a fresh rate in range; no memory between presses.
    Random { min: f32, max: f32 },
}

impl Default for Pitch {
    fn default() -> Self {
        Pitch::Step { step: -0.12, min: 0.35, max: 2.5 }
    }
}

fn one() -> f32 {
    1.0
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sound {
    pub id: String,
    pub name: String,
    /// Absolute path inside the app's own `sounds` directory.
    pub file: String,
    #[serde(default)]
    pub hotkey: String,
    #[serde(default = "one")]
    pub volume: f32,
    /// Seconds of leading silence to skip. Plenty of downloaded clips open with
    /// a beat of nothing, which reads as lag when the hotkey is the punchline.
    #[serde(default)]
    pub offset: f32,
    #[serde(default)]
    pub pitch: Pitch,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub sounds: Vec<Sound>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    /// Bumped when a release has to overwrite bindings the user already has -
    /// an old config that predates the bump gets `migrate`d exactly once.
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub profiles: Vec<Profile>,
    #[serde(default)]
    pub active: String,
    /// Substring of the device Discord listens to, e.g. "CABLE Input".
    #[serde(default)]
    pub output_device: String,
    /// Substring of your own headphones; empty means you hear nothing.
    #[serde(default)]
    pub monitor_device: String,
    #[serde(default)]
    pub pitch_hotkey: String,
    #[serde(default)]
    pub stop_hotkey: String,
    #[serde(default)]
    pub next_profile_hotkey: String,
    #[serde(default = "one")]
    pub master_volume: f32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            version: CONFIG_VERSION,
            profiles: vec![Profile { id: uid(), name: "Default".into(), sounds: vec![] }],
            active: String::new(),
            output_device: "CABLE Input".into(),
            monitor_device: String::new(),
            // Laptop-safe: no numpad, no Fn row, and nothing in Windows,
            // PowerToys, Discord or a browser claims these two.
            pitch_hotkey: "Ctrl+Shift+Quote".into(),
            stop_hotkey: "Ctrl+Shift+Semicolon".into(),
            next_profile_hotkey: String::new(),
            master_volume: 1.0,
        }
    }
}

/// 2: the numpad-era bindings are dropped for laptop-safe ones.
const CONFIG_VERSION: u32 = 2;

/// The bindings Thunderboard hands out, in order. Both banks work on a laptop
/// with no numpad and no Fn key, and nothing in Windows, PowerToys, Discord or
/// a browser claims them. Kept in step with `SUGGESTED` in src/lib/keys.ts.
fn free_keys() -> Vec<String> {
    let digits = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"]
        .iter()
        .map(|d| format!("Ctrl+Shift+Digit{d}"));
    // Quote and Semicolon are missing on purpose: they are the pitch and stop
    // defaults, and a sound bound over one of them would silently lose.
    let punctuation = [
        "Minus", "Equal", "BracketLeft", "BracketRight", "Backslash", "Comma", "Period", "Slash",
    ]
    .iter()
    .map(|k| format!("Ctrl+Shift+{k}"));
    digits.chain(punctuation).collect()
}

/// Reissues every binding an older version handed out. Returns whether anything
/// changed, so a fresh install does not rewrite its own config on launch.
///
/// This deliberately throws away bindings the user may have chosen themselves:
/// the whole point of the bump is that the old bank needed a numpad this
/// machine does not have, so leaving them in place leaves dead keys.
fn migrate(cfg: &mut Config) -> bool {
    if cfg.version >= CONFIG_VERSION {
        return false;
    }
    let defaults = Config::default();
    cfg.pitch_hotkey = defaults.pitch_hotkey;
    cfg.stop_hotkey = defaults.stop_hotkey;
    cfg.next_profile_hotkey.clear();

    // One bank across the whole config: two profiles cannot both own a key, and
    // only the active profile's bindings are ever registered anyway.
    let mut bank = free_keys().into_iter();
    for profile in &mut cfg.profiles {
        for sound in &mut profile.sounds {
            if !sound.hotkey.is_empty() {
                sound.hotkey = bank.next().unwrap_or_default();
            }
        }
    }
    cfg.version = CONFIG_VERSION;
    true
}

impl Config {
    fn active_profile(&self) -> Option<&Profile> {
        self.profiles
            .iter()
            .find(|p| p.id == self.active)
            .or_else(|| self.profiles.first())
    }

    fn sound(&self, id: &str) -> Option<&Sound> {
        self.active_profile()?.sounds.iter().find(|s| s.id == id)
    }
}

/// Monotonic, collision-free, and short enough to read in a JSON file.
fn uid() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0);
    format!("{:x}{:x}", t, N.fetch_add(1, Ordering::Relaxed))
}

// ------------------------------------------------------------ pitch walk ----

#[derive(Default)]
struct PitchState {
    on: bool,
    /// Which clip the walk currently belongs to. A different clip restarts it.
    last: Option<String>,
    n: u32,
}

struct App {
    tx: Sender<audio::Cmd>,
    cfg: Mutex<Config>,
    pitch: Mutex<PitchState>,
    tray: Mutex<Option<TrayIcon<Wry>>>,
}

/// ponytail: nanosecond jitter, not a PRNG. The only consumer is the pitch of a
/// fart noise; swap in `fastrand` if that ever stops being true.
fn rand01() -> f32 {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    // Only the low bits move between two presses, so mix them upward.
    ((n.wrapping_mul(2_654_435_761) >> 8) & 0xffff) as f32 / 65_535.0
}

/// Advances the walk and returns the rate for this press.
///
/// The walk survives repeated presses of the same clip and is reset by a
/// different clip or by disengaging pitch mod - nothing else.
fn next_speed(st: &mut PitchState, sound: &Sound) -> f32 {
    if !st.on {
        st.last = None;
        return 1.0;
    }
    if st.last.as_deref() == Some(sound.id.as_str()) {
        st.n += 1;
    } else {
        st.last = Some(sound.id.clone());
        st.n = 0;
    }
    let speed = match sound.pitch {
        // First press of a run is untouched; the drop starts on the repeat.
        Pitch::Step { step, min, max } => {
            (1.0 + step * st.n as f32).clamp(min.min(max), max.max(min))
        }
        Pitch::Random { min, max } => min.min(max) + rand01() * (max - min).abs(),
    };
    speed.clamp(audio::MIN_SPEED, audio::MAX_SPEED)
}

// -------------------------------------------------------------- triggers ----

fn play(app: &AppHandle, id: &str, monitor_only: bool) {
    let state = app.state::<App>();
    let cfg = state.cfg.lock().unwrap();
    let Some(sound) = cfg.sound(id) else { return };

    let speed = if monitor_only {
        1.0
    } else {
        next_speed(&mut state.pitch.lock().unwrap(), sound)
    };
    let _ = state.tx.send(audio::Cmd::Play {
        path: sound.file.clone(),
        speed,
        volume: sound.volume * cfg.master_volume,
        offset: sound.offset.max(0.0),
        monitor_only,
    });
    let _ = app.emit("played", serde_json::json!({ "id": id, "speed": speed }));
}

fn set_pitch_mod(app: &AppHandle, on: bool) {
    let state = app.state::<App>();
    {
        let mut p = state.pitch.lock().unwrap();
        p.on = on;
        // Toggling either way starts a fresh walk.
        p.last = None;
        p.n = 0;
    }
    if let Some(tray) = state.tray.lock().unwrap().as_ref() {
        let _ = tray.set_tooltip(Some(if on {
            "Thunderboard - pitch mod ON"
        } else {
            "Thunderboard"
        }));
    }
    let _ = app.emit("pitch", on);
}

fn next_profile(app: &AppHandle) {
    let switched = {
        let state = app.state::<App>();
        let mut cfg = state.cfg.lock().unwrap();
        let Some(i) = cfg.profiles.iter().position(|p| p.id == cfg.active) else {
            return;
        };
        let next = (i + 1) % cfg.profiles.len();
        cfg.active = cfg.profiles[next].id.clone();
        cfg.active.clone()
    };
    let _ = save(app);
    let _ = app.emit("profile", switched);
    apply(app);
}

// -------------------------------------------------------------- hotkeys -----

/// Re-registers every shortcut from scratch and reports the ones Windows
/// refused, which is the only way to find out that something else owns them.
fn apply(app: &AppHandle) -> Vec<String> {
    let cfg = app.state::<App>().cfg.lock().unwrap().clone();

    let _ = app.state::<App>().tx.send(audio::Cmd::Devices {
        out: cfg.output_device.clone(),
        monitor: cfg.monitor_device.clone(),
    });

    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let mut failed = Vec::new();

    let mut bind = |key: &str, action: Box<dyn Fn(&AppHandle) + Send + Sync + 'static>| {
        if key.is_empty() {
            return;
        }
        let hit = gs.on_shortcut(key, move |app, _shortcut, event| {
            // Without this the action runs twice, once per edge.
            if event.state() == ShortcutState::Pressed {
                action(app);
            }
        });
        if hit.is_err() {
            failed.push(key.to_string());
        }
    };

    if let Some(profile) = cfg.active_profile() {
        for sound in &profile.sounds {
            let id = sound.id.clone();
            bind(&sound.hotkey, Box::new(move |app| play(app, &id, false)));
        }
    }
    bind(
        &cfg.pitch_hotkey,
        Box::new(|app| {
            let on = !app.state::<App>().pitch.lock().unwrap().on;
            set_pitch_mod(app, on);
        }),
    );
    bind(
        &cfg.stop_hotkey,
        Box::new(|app| {
            let _ = app.state::<App>().tx.send(audio::Cmd::Stop);
        }),
    );
    bind(&cfg.next_profile_hotkey, Box::new(next_profile));

    let _ = app.emit("hotkeys", &failed);
    failed
}

// ------------------------------------------------------------- commands -----

fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn save(app: &AppHandle) -> Result<(), String> {
    let cfg = app.state::<App>().cfg.lock().unwrap().clone();
    let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    fs::write(data_dir(app)?.join("config.json"), json).map_err(|e| e.to_string())
}

#[tauri::command]
fn load_config(state: State<App>) -> Config {
    state.cfg.lock().unwrap().clone()
}

/// The UI owns the config document; Rust just persists it and re-binds.
/// Returns the shortcuts that could not be registered.
#[tauri::command]
fn save_config(app: AppHandle, config: Config) -> Result<Vec<String>, String> {
    *app.state::<App>().cfg.lock().unwrap() = config;
    save(&app)?;
    Ok(apply(&app))
}

/// Copies dropped files into the app's own directory so the board keeps working
/// after the originals are moved or deleted.
#[tauri::command]
fn import_sounds(app: AppHandle, paths: Vec<String>) -> Result<Vec<Sound>, String> {
    const EXTS: [&str; 8] = ["mp3", "wav", "ogg", "flac", "m4a", "aac", "opus", "wma"];
    let dir = data_dir(&app)?.join("sounds");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let mut imported = Vec::new();
    for path in paths {
        let src = PathBuf::from(&path);
        let ext = src
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if !EXTS.contains(&ext.as_str()) {
            continue;
        }
        let stem = src
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("sound")
            .to_string();

        // Two clips both called "airhorn.mp3" would otherwise silently overwrite.
        let mut dest = dir.join(format!("{stem}.{ext}"));
        let mut n = 1;
        while dest.exists() {
            dest = dir.join(format!("{stem} ({n}).{ext}"));
            n += 1;
        }
        fs::copy(&src, &dest).map_err(|e| format!("{path}: {e}"))?;

        imported.push(Sound {
            id: uid(),
            name: stem,
            file: dest.to_string_lossy().into_owned(),
            hotkey: String::new(),
            volume: 1.0,
            offset: 0.0,
            pitch: Pitch::default(),
        });
    }
    Ok(imported)
}

#[tauri::command]
fn list_devices() -> Vec<String> {
    audio::output_devices()
}

/// Audition on the monitor bus only - never out to the call.
#[tauri::command]
fn preview(app: AppHandle, id: String) {
    play(&app, &id, true);
}

#[tauri::command]
fn stop_all(state: State<App>) {
    let _ = state.tx.send(audio::Cmd::Stop);
}

#[tauri::command]
fn pitch_mod(app: AppHandle, on: bool) {
    set_pitch_mod(&app, on);
}

#[tauri::command]
fn sounds_dir(app: AppHandle) -> Result<String, String> {
    Ok(data_dir(&app)?.join("sounds").to_string_lossy().into_owned())
}

/// Leaves for good. The window's close button confirms first; the tray menu
/// item does not, because a menu you opened deliberately is its own confirmation.
#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

/// Reads the registry rather than a stored flag, so the tray checkbox and the
/// settings checkbox can never disagree.
#[tauri::command]
fn autostart(app: AppHandle, on: Option<bool>) -> bool {
    let launcher = app.autolaunch();
    if let Some(on) = on {
        let _ = if on { launcher.enable() } else { launcher.disable() };
    }
    launcher.is_enabled().unwrap_or(false)
}

// ---------------------------------------------------------------- window ----

/// Win11 does not round undecorated windows on its own, and a CSS shadow would
/// be clipped by the window rect. DWM draws both outside it.
#[cfg(windows)]
fn round_corners(win: &WebviewWindow) {
    use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWCP_ROUND: u32 = 2;

    if let Ok(hwnd) = win.hwnd() {
        let preference: u32 = DWMWCP_ROUND;
        unsafe {
            DwmSetWindowAttribute(
                hwnd.0 as _,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                std::ptr::addr_of!(preference).cast(),
                std::mem::size_of::<u32>() as u32,
            );
        }
    }
}

const GAP: i32 = 14;

/// Opens next to the cursor - which, on a tray click, is the tray. Flips then
/// clamps so it always lands fully inside the monitor the cursor is on.
fn show_near_cursor(win: &WebviewWindow) -> tauri::Result<()> {
    let cursor = win.app_handle().cursor_position()?;
    let monitor = match win.monitor_from_point(cursor.x, cursor.y)? {
        Some(m) => Some(m),
        None => win.primary_monitor()?,
    };

    if let Some(monitor) = monitor {
        let size = win.outer_size()?;
        let (w, h) = (size.width as i32, size.height as i32);
        let origin = monitor.position();
        let area = monitor.size();
        let (max_x, max_y) = (origin.x + area.width as i32, origin.y + area.height as i32);
        let (cx, cy) = (cursor.x as i32, cursor.y as i32);

        let mut x = cx + GAP;
        if x + w > max_x {
            x = cx - GAP - w;
        }
        let mut y = cy + GAP;
        if y + h > max_y {
            y = cy - GAP - h;
        }
        // A monitor smaller than the window would make the clamp range invalid.
        x = x.clamp(origin.x, (max_x - w).max(origin.x));
        y = y.clamp(origin.y, (max_y - h).max(origin.y));
        win.set_position(PhysicalPosition::new(x, y))?;
    }

    // Setting this in setup() does not survive to first paint.
    #[cfg(windows)]
    round_corners(win);

    win.show()?;
    win.set_focus()?;

    // tao caches its own visibility flag and skips the syscall when the flag
    // already says shown. A flag that has drifted out of step with the real
    // window leaves `show()` doing nothing at all, which is what made the tray
    // icon need a second click. Go around it.
    #[cfg(windows)]
    if let Ok(hwnd) = win.hwnd() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SetForegroundWindow, ShowWindow, SW_SHOW,
        };
        unsafe {
            ShowWindow(hwnd.0 as _, SW_SHOW);
            SetForegroundWindow(hwnd.0 as _);
        }
    }

    win.emit("shown", ())?;
    Ok(())
}

/// Always reveals - never hides.
///
/// Toggling on the tray click meant trusting `is_visible()`, and when that read
/// was stale the click hid an already-hidden window and you had to click twice.
/// Leaving is the titlebar's job now, so the tray only has to do one thing.
fn reveal(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = show_near_cursor(&win);
    }
}

// ------------------------------------------------------------------ run -----

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Must be first. A second launch reveals the window instead of starting
        // a rival copy that would fight over the same hotkeys.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            reveal(app);
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .invoke_handler(tauri::generate_handler![
            load_config,
            save_config,
            import_sounds,
            list_devices,
            preview,
            stop_all,
            pitch_mod,
            sounds_dir,
            autostart,
            quit
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            let path = handle.path().app_data_dir()?.join("config.json");
            let first_run = !path.exists();
            let mut cfg: Config = fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            if cfg.profiles.is_empty() {
                cfg.profiles = Config::default().profiles;
            }
            if cfg.active.is_empty() {
                cfg.active = cfg.profiles[0].id.clone();
            }
            let migrated = migrate(&mut cfg);

            app.manage(App {
                tx: audio::spawn(),
                cfg: Mutex::new(cfg),
                pitch: Mutex::new(PitchState::default()),
                tray: Mutex::new(None),
            });

            // Opt in once; enabling every launch would undo the user turning it off.
            if first_run {
                let _ = handle.autolaunch().enable();
            }
            let autostart_on = handle.autolaunch().is_enabled().unwrap_or(false);

            let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
            let stop = MenuItem::with_id(app, "stop", "Stop sounds", true, None::<&str>)?;
            let startup = CheckMenuItem::with_id(
                app,
                "autostart",
                "Start with Windows",
                true,
                autostart_on,
                None::<&str>,
            )?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let startup_item = startup.clone();

            let tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Thunderboard")
                .menu(&Menu::with_items(app, &[&show, &stop, &startup, &quit])?)
                .show_menu_on_left_click(false)
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "show" => reveal(app),
                    "stop" => {
                        let _ = app.state::<App>().tx.send(audio::Cmd::Stop);
                    }
                    "autostart" => {
                        let launcher = app.autolaunch();
                        let on = launcher.is_enabled().unwrap_or(false);
                        let _ = if on { launcher.disable() } else { launcher.enable() };
                        let _ = startup_item.set_checked(!on);
                    }
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
                        reveal(tray.app_handle());
                    }
                })
                .build(app)?;
            *handle.state::<App>().tray.lock().unwrap() = Some(tray);

            if let Some(win) = app.get_webview_window("main") {
                let hide_target = win.clone();
                win.on_window_event(move |event| {
                    // No titlebar to close with, and a stray close must not kill
                    // the tray. Note there is deliberately no hide-on-blur here:
                    // dragging a file in from Explorer blurs us first.
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = hide_target.hide();
                    }
                });
            }

            if migrated {
                let _ = save(&handle);
            }
            apply(&handle);
            if first_run {
                reveal(&handle);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sound(id: &str, pitch: Pitch) -> Sound {
        Sound {
            id: id.into(),
            name: id.into(),
            file: String::new(),
            hotkey: String::new(),
            volume: 1.0,
            offset: 0.0,
            pitch,
        }
    }

    #[test]
    fn pitch_walk() {
        let down = sound("a", Pitch::Step { step: -0.2, min: 0.4, max: 2.0 });
        let other = sound("b", Pitch::Step { step: -0.2, min: 0.4, max: 2.0 });
        let mut st = PitchState::default();

        // Mod off: always dead flat, and no walk is accumulated.
        assert_eq!(next_speed(&mut st, &down), 1.0);
        assert_eq!(next_speed(&mut st, &down), 1.0);

        st.on = true;
        assert_eq!(next_speed(&mut st, &down), 1.0); // first press is untouched
        assert!((next_speed(&mut st, &down) - 0.8).abs() < 1e-6);
        assert!((next_speed(&mut st, &down) - 0.6).abs() < 1e-6);

        // A different clip restarts the walk...
        assert_eq!(next_speed(&mut st, &other), 1.0);
        // ...and coming back restarts it again rather than resuming.
        assert_eq!(next_speed(&mut st, &down), 1.0);

        // The floor holds no matter how long the run gets.
        for _ in 0..50 {
            assert!(next_speed(&mut st, &down) >= 0.4);
        }

        // Toggling off resets, so the next run starts from flat.
        st.on = false;
        assert_eq!(next_speed(&mut st, &down), 1.0);
        st.on = true;
        assert_eq!(next_speed(&mut st, &down), 1.0);
    }

    #[test]
    fn migration_reissues_old_bindings_once() {
        let mut old = Config {
            version: 0,
            pitch_hotkey: "Ctrl+Alt+Numpad0".into(),
            stop_hotkey: "Ctrl+Alt+NumpadDecimal".into(),
            profiles: vec![Profile {
                id: "p".into(),
                name: "Default".into(),
                sounds: vec![
                    sound("a", Pitch::default()),
                    sound("b", Pitch::default()),
                    sound("c", Pitch::default()),
                ],
            }],
            ..Config::default()
        };
        old.profiles[0].sounds[0].hotkey = "Alt+KeyG".into();
        old.profiles[0].sounds[1].hotkey = "Alt+KeyF".into();
        // Left unbound on purpose - migration must not invent a binding for it.

        assert!(migrate(&mut old));
        assert_eq!(old.pitch_hotkey, "Ctrl+Shift+Quote");
        assert_eq!(old.profiles[0].sounds[0].hotkey, "Ctrl+Shift+Digit1");
        assert_eq!(old.profiles[0].sounds[1].hotkey, "Ctrl+Shift+Digit2");
        assert_eq!(old.profiles[0].sounds[2].hotkey, "");
        assert!(!old.profiles[0].sounds.iter().any(|s| s.hotkey.contains("Numpad")));

        // Idempotent: a second launch leaves the config alone.
        let before = format!("{old:?}");
        assert!(!migrate(&mut old));
        assert_eq!(before, format!("{old:?}"));
    }

    #[test]
    fn random_stays_in_range() {
        let s = sound("a", Pitch::Random { min: 0.5, max: 1.8 });
        let mut st = PitchState { on: true, ..Default::default() };
        for _ in 0..200 {
            let v = next_speed(&mut st, &s);
            assert!((0.5..=1.8).contains(&v), "{v} out of range");
        }
    }

    #[test]
    fn absurd_config_is_clamped_not_trusted() {
        // Hand-edited JSON must not be able to produce a 0x or negative rate.
        let s = sound("a", Pitch::Step { step: -5.0, min: -10.0, max: 500.0 });
        let mut st = PitchState { on: true, ..Default::default() };
        for _ in 0..10 {
            let v = next_speed(&mut st, &s);
            assert!(v >= audio::MIN_SPEED && v <= audio::MAX_SPEED, "{v}");
        }
    }
}
