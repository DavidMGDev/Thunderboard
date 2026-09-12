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
    /// A folder this profile mirrors. Empty means the profile is hand-built and
    /// its clips live in the app's own `sounds` directory.
    #[serde(default)]
    pub folder: String,
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
            profiles: vec![Profile {
                id: uid(),
                name: "Default".into(),
                sounds: vec![],
                folder: String::new(),
            }],
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

/// Queues a re-bind on the main thread. Never binds inline - see `rebind`.
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
    rebind(app);
}

// ------------------------------------------------------- keyboard layout ----

/// The punctuation keys whose virtual-key code moves with the keyboard layout.
///
/// Each entry is (name `KeyboardEvent.code` gives the key, its scan code, the
/// virtual key a US layout puts there).
///
/// This exists because two layers disagree. `KeyboardEvent.code` names a
/// *physical* key using US labels, so the key that types `|` on a Latin
/// American Spanish keyboard still arrives as "Backquote". `global_hotkey` then
/// turns that name into a virtual key with a hard-coded US table - Backquote
/// becomes 0xC0. But on that layout the physical key reports 0xDC, and 0xC0 is
/// the `ñ` key. So `RegisterHotKey` succeeds, binds a key the user never
/// presses, and the shortcut silently does nothing forever.
///
/// Letters and digits are immune: their virtual keys are identical on every
/// Latin layout. Only this set moves, which is why sound hotkeys on digits
/// worked while stop, which lives on punctuation, never did.
const LAYOUT_KEYS: [(&str, u32, u16); 11] = [
    ("Minus", 0x0C, 0xBD),
    ("Equal", 0x0D, 0xBB),
    ("BracketLeft", 0x1A, 0xDB),
    ("BracketRight", 0x1B, 0xDD),
    ("Semicolon", 0x27, 0xBA),
    ("Quote", 0x28, 0xDE),
    ("Backquote", 0x29, 0xC0),
    ("Backslash", 0x2B, 0xDC),
    ("Comma", 0x33, 0xBC),
    ("Period", 0x34, 0xBE),
    ("Slash", 0x35, 0xBF),
];

/// Rewrites a shortcut's key so the US table downstream lands on the virtual
/// key this layout really produces. `to_vk` maps a scan code to that key.
///
/// Anything but the layout-dependent punctuation is returned untouched, as is
/// a key whose layout puts a virtual code the US table cannot name (`VK_OEM_102`
/// on the extra key some European keyboards have). Better to fail registration
/// visibly than to bind a different key than the one that was pressed.
fn retarget(shortcut: &str, to_vk: impl Fn(u32) -> u16) -> String {
    let (prefix, key) = match shortcut.rfind('+') {
        Some(i) => shortcut.split_at(i + 1),
        None => ("", shortcut),
    };
    let Some(&(_, scan, _)) = LAYOUT_KEYS.iter().find(|(name, _, _)| *name == key) else {
        return shortcut.to_string();
    };
    let vk = to_vk(scan);
    match LAYOUT_KEYS.iter().find(|(_, _, us_vk)| *us_vk == vk) {
        Some((name, _, _)) => format!("{prefix}{name}"),
        None => shortcut.to_string(),
    }
}

/// The scan code's virtual key under the layout this thread is using.
#[cfg(windows)]
fn active_layout_vk(scan: u32) -> u16 {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardLayout, MapVirtualKeyExW, MAPVK_VSC_TO_VK_EX,
    };
    // Layout of our own thread, which is the current input language. Re-binding
    // happens on every save, so switching language and touching settings picks
    // the new one up.
    unsafe { MapVirtualKeyExW(scan, MAPVK_VSC_TO_VK_EX, GetKeyboardLayout(0)) as u16 }
}

#[cfg(not(windows))]
fn active_layout_vk(scan: u32) -> u16 {
    LAYOUT_KEYS
        .iter()
        .find(|(_, s, _)| *s == scan)
        .map(|(_, _, vk)| *vk)
        .unwrap_or(0)
}

// -------------------------------------------------------------- hotkeys -----

/// Re-binds every hotkey, on the main thread, never inline.
///
/// The shortcut plugin holds its registry mutex for the whole of a hotkey
/// callback, and `apply` wants that same mutex in order to unregister. So
/// calling `apply` from inside a callback - or from a worker thread while a
/// callback is running - is a circular wait that hangs the main thread, and
/// with it every global hotkey, until the app is restarted. That is what made
/// the stop key stop firing: one save during a keypress and it never came back.
///
/// Hopping to the main thread via a worker makes the hop a real queued task
/// rather than an inline call, so the mutex is only ever taken by the main
/// thread, and a callback and a re-bind can no longer overlap.
fn rebind(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            apply(&handle);
        });
    });
}

/// Re-registers every shortcut from scratch and reports the ones Windows
/// refused, which is the only way to find out that something else owns them.
///
/// Main thread only. Everything but first-run setup goes through `rebind`.
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
        // Registered against the key this layout actually produces; reported
        // back to the UI under the name the config stores.
        let target = retarget(key, active_layout_vk);
        let hit = gs.on_shortcut(target.as_str(), move |app, _shortcut, event| {
            // Without this the action runs twice, once per edge.
            if event.state() == ShortcutState::Pressed {
                action(app);
            }
        });
        if hit.is_err() {
            failed.push(key.to_string());
        }
    };

    // The three app-wide controls bind first. Windows gives a combo to whoever
    // asks for it first, so a sound bound over the stop key used to win and
    // leave stop silently dead.
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
    if let Some(profile) = cfg.active_profile() {
        for sound in &profile.sounds {
            let id = sound.id.clone();
            bind(&sound.hotkey, Box::new(move |app| play(app, &id, false)));
        }
    }

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
///
/// Re-binding is queued rather than awaited, so the shortcuts that Windows
/// refuses arrive on the `hotkeys` event instead of as a return value.
#[tauri::command]
fn save_config(app: AppHandle, config: Config) -> Result<(), String> {
    *app.state::<App>().cfg.lock().unwrap() = config;
    save(&app)?;
    rebind(&app);
    Ok(())
}

const EXTS: [&str; 8] = ["mp3", "wav", "ogg", "flac", "m4a", "aac", "opus", "wma"];

fn is_audio(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .is_some_and(|e| EXTS.contains(&e.as_str()))
}

fn stem(path: &std::path::Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("sound")
        .to_string()
}

fn new_sound(path: &std::path::Path) -> Sound {
    Sound {
        id: uid(),
        name: stem(path),
        file: path.to_string_lossy().into_owned(),
        hotkey: String::new(),
        volume: 1.0,
        offset: 0.0,
        pitch: Pitch::default(),
    }
}

/// Windows paths differ only in case, and the same file reached two ways must
/// not turn into two rows.
fn same_file(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Copies files into `dest`, or into the app's own directory when `dest` is
/// None, so a hand-built board keeps working after the originals move.
///
/// A file already sitting in `dest` is adopted where it lies - a folder profile
/// would otherwise grow a copy of every clip it already has.
#[tauri::command]
fn import_sounds(
    app: AppHandle,
    paths: Vec<String>,
    dest: Option<String>,
) -> Result<Vec<Sound>, String> {
    let dir = match dest.filter(|d| !d.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => data_dir(&app)?.join("sounds"),
    };
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let mut imported = Vec::new();
    for path in paths {
        let src = PathBuf::from(&path);
        if !is_audio(&src) {
            continue;
        }
        if src.parent() == Some(dir.as_path()) {
            imported.push(new_sound(&src));
            continue;
        }
        let ext = src
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let name = stem(&src);

        // Two clips both called "airhorn.mp3" would otherwise silently overwrite.
        let mut target = dir.join(format!("{name}.{ext}"));
        let mut n = 1;
        while target.exists() {
            target = dir.join(format!("{name} ({n}).{ext}"));
            n += 1;
        }
        fs::copy(&src, &target).map_err(|e| format!("{path}: {e}"))?;
        imported.push(new_sound(&target));
    }
    Ok(imported)
}

/// Every audio file under `dir`, depth first. Unreadable subfolders are skipped
/// rather than failing the whole scan - one locked directory should not cost
/// you the rest of the board.
fn audio_files(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut found: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            audio_files(&path, out);
        } else if is_audio(&path) {
            found.push(path);
        }
    }
    found.sort();
    out.append(&mut found);
}

/// Reconciles a folder profile against what is actually on disk.
///
/// Rows that still have their file keep their position, hotkey and tuning;
/// rows whose file is gone drop out; anything new lands at the end. Keeping
/// the order stable matters more than it sounds - the hotkeys are muscle
/// memory, and a re-sort would shuffle the whole board under your fingers.
///
/// ponytail: O(n*m) scan. Fine for a folder of clips; reach for a set if
/// someone points this at a music library.
fn merge_folder(existing: &[Sound], files: Vec<PathBuf>) -> Vec<Sound> {
    let found: Vec<String> = files
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();

    let mut out: Vec<Sound> = existing
        .iter()
        .filter(|s| found.iter().any(|f| same_file(f, &s.file)))
        .cloned()
        .collect();

    for path in &files {
        let file = path.to_string_lossy();
        if !out.iter().any(|s| same_file(&s.file, &file)) {
            out.push(new_sound(path));
        }
    }
    out
}

/// Re-reads a profile's folder. Cheap enough to run every time the window
/// opens: one directory walk, no decoding, no disk reads beyond the listing.
#[tauri::command]
fn sync_folder(dir: String, existing: Vec<Sound>) -> Result<Vec<Sound>, String> {
    let root = PathBuf::from(&dir);
    if !root.is_dir() {
        return Err(format!("{dir} is not a folder"));
    }
    let mut files = Vec::new();
    audio_files(&root, &mut files);
    Ok(merge_folder(&existing, files))
}

#[tauri::command]
fn list_devices() -> Vec<String> {
    audio::output_devices()
}

/// Fires a clip from the UI.
///
/// `audition` keeps it on the monitor bus, which is what tuning the offset
/// wants - dragging that slider should not blast the call on every step. A
/// plain click on a sound is not an audition though: it is the board being
/// used, and it goes out like a hotkey would. Folder profiles made that
/// distinction matter, because their clips arrive unbound and clicking is the
/// only way to fire them at all.
#[tauri::command]
fn preview(app: AppHandle, id: String, audition: bool) {
    play(&app, &id, audition);
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
            sync_folder,
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
                folder: String::new(),
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
    fn folder_sync_keeps_tuning_and_order() {
        let mut tuned = sound("a", Pitch::default());
        tuned.file = r"C:\clips\air.mp3".into();
        tuned.hotkey = "Ctrl+Shift+Digit1".into();
        tuned.volume = 0.4;
        let mut gone = sound("b", Pitch::default());
        gone.file = r"C:\clips\deleted.mp3".into();

        let on_disk = vec![
            // Same file, different case: Windows, so this is the same clip.
            PathBuf::from(r"C:\CLIPS\air.mp3"),
            PathBuf::from(r"C:\clips\new.wav"),
        ];
        let out = merge_folder(&[tuned.clone(), gone], on_disk.clone());

        // The tuned row survives, in place, with everything it had.
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].id, tuned.id);
        assert_eq!(out[0].hotkey, "Ctrl+Shift+Digit1");
        assert_eq!(out[0].volume, 0.4);
        // The deleted file dropped out and the new one arrived, named by stem.
        assert_eq!(out[1].name, "new");
        assert!(out[1].hotkey.is_empty());
        assert!(!out.iter().any(|s| s.file.contains("deleted")));

        // Syncing again with nothing changed must not churn ids or order.
        let again = merge_folder(&out, on_disk);
        assert_eq!(
            again.iter().map(|s| &s.id).collect::<Vec<_>>(),
            out.iter().map(|s| &s.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn punctuation_retargets_to_the_layout() {
        // Latin American Spanish, as reported by MapVirtualKeyEx for layout
        // 0x080A140A. The physical key the UI calls Backquote types `|` and
        // reports 0xDC; 0xC0 sits on the `ñ` key instead.
        let latam = |scan: u32| match scan {
            0x29 => 0xDC, // Backquote position -> the `|` key
            0x2B => 0xBF, // Backslash position -> `}`
            0x27 => 0xC0, // Semicolon position -> `ñ`
            0x28 => 0xDE, // Quote position     -> `{`
            _ => 0,
        };

        // The bug: stop on the `|` key used to register 0xC0, the `ñ` key.
        // 0xDC is what the US table calls Backslash, so that is what gets
        // registered - and pressing `|` now fires it.
        assert_eq!(retarget("Backquote", latam), "Backslash");
        assert_eq!(retarget("Shift+Backquote", latam), "Shift+Backslash");
        assert_eq!(retarget("Ctrl+Shift+Backquote", latam), "Ctrl+Shift+Backslash");
        // Quote happens to sit on the same virtual key on both layouts.
        assert_eq!(retarget("Ctrl+Shift+Quote", latam), "Ctrl+Shift+Quote");

        // Letters, digits and F-keys never move, so they are passed straight
        // through - which is why the sound hotkeys worked all along.
        for untouched in ["Ctrl+Shift+Digit1", "Ctrl+Shift+KeyQ", "F13", "Ctrl+Alt+Shift+F13"] {
            assert_eq!(retarget(untouched, latam), untouched);
        }

        // A US layout is the identity case: nothing should be rewritten.
        let us = |scan: u32| {
            LAYOUT_KEYS.iter().find(|(_, s, _)| *s == scan).map(|(_, _, vk)| *vk).unwrap_or(0)
        };
        for (name, _, _) in LAYOUT_KEYS {
            assert_eq!(retarget(name, us), name);
            assert_eq!(retarget(&format!("Ctrl+Shift+{name}"), us), format!("Ctrl+Shift+{name}"));
        }

        // A layout that puts something unnameable here is left alone rather
        // than silently bound to the wrong key.
        assert_eq!(retarget("Backquote", |_| 0xE2), "Backquote");
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
