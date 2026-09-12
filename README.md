# Thunderboard

A tray-resident soundboard for Discord. Global hotkeys, per-sound pitch
progressions, profiles. Fluent dark, sized to sit next to PowerToys.

Tauri 2 + Svelte 5 + rodio.

## What it does

- **Global hotkeys.** Fire clips with the window closed, from inside a game.
- **Cut-previous.** A new hotkey stops whatever was playing. No layering.
- **Pitch mod.** A toggle key. While it is on, repeatedly hitting the *same*
  clip's hotkey walks its playback rate — down by a fixed step each press, or to
  a fresh random rate each press, configured per sound. Rate is pitch (rodio
  resamples), so down also means slower. The walk resets when you hit a
  different clip or toggle pitch mod off, and nothing else resets it.
- **Two outputs.** One bus goes to the virtual cable Discord listens to, the
  other to your headphones so you hear what you fired.
- **Profiles.** Independent sets of sounds and bindings; switch from the header
  or with a hotkey.
- **List or gallery.** The same profile as a dense list of rows, or as a grid of
  pads you click to fire. Toggle in the header; the choice sticks.
- **Drag and drop.** Drop audio files on the window, or use the `+` picker.
  Either way they are copied into the app's own directory, so the board survives
  you moving or deleting the originals.
- **Per-clip offset.** Downloaded clips often open with a beat of silence, which
  reads as lag when the hotkey is the punchline. A wide slider trims it; letting
  go of the slider plays the clip so you can hear whether you got it.
- **Tray-resident.** Left-click the tray icon to open. The minimize button puts
  it back; the close button quits and asks first, because quitting kills your
  hotkeys.

## Windows audio setup

Thunderboard plays *into* a device. Getting that device into Discord alongside
your actual voice is a Windows configuration job, not an app feature — the app
deliberately does not try to own your microphone.

1. Install [VB-CABLE](https://vb-audio.com/Cable/).
2. **Thunderboard → Settings**: Output device `CABLE Input`, Monitor device your
   headphones.
3. **Discord → Voice & Video**: Input device `CABLE Output`.
4. **Windows Sound Control Panel → Recording → your mic → Listen tab**: tick
   *Listen to this device*, set *Playback through this device* to `CABLE Input`.
   Your voice and the soundboard now both land in the cable.
5. **Discord → Voice & Video → turn OFF** Noise Suppression (Krisp), Echo
   Cancellation and Automatic Gain Control. Discord classifies pitched clips as
   noise and will gate or mangle them otherwise. This is the single most common
   cause of "my soundboard sounds terrible".

Use headphones, not speakers. With speakers, your mic picks up the soundboard
and feeds it back into the cable.

Step 4 adds ~30ms to your outgoing voice. If that bothers you, replace steps 4
with VoiceMeeter Banana and point Discord at its virtual output instead.

## Hotkeys that are actually free

Registration goes through Win32 `RegisterHotKey`, so a combo another app owns
simply fails — the row turns amber when that happens.

These banks assume **no numpad** — laptop keyboard, nothing needing Fn unless
noted. A global hotkey outranks the focused app, so binding one that an app
already uses takes it away from that app *everywhere*; that is why the list is
almost entirely `Ctrl+Shift`.

| Bank | Slots | Conflicts |
| --- | --- | --- |
| `F13`–`F24` | 12 | None anywhere, and single-press so it survives fullscreen games. No keyboard sends them — remap CapsLock, Menu or Right-Alt onto one with PowerToys Keyboard Manager, or use a macropad. **Best option if you're willing to remap.** |
| `Ctrl+Shift+1` … `0` | 10 | None. `Ctrl+<digit>` is browser tabs; adding Shift is free. |
| `Ctrl+Shift+ - = [ ] \ ; ' , . /` | 10 | None. The app's own defaults live here. |
| `Ctrl+Alt+1` … `0` | 10 | None on US layouts. **Ctrl+Alt is AltGr on international layouts** — skip if you ever type on one. |
| `Ctrl+Shift+F1` … `F12` | 12 | None, but needs Fn on laptops whose F-row defaults to media keys. |
| `Ctrl+Shift+Insert/Home/End/PgUp/PgDn`, `ScrollLock`, `Pause` | 7 | None. ScrollLock and Pause do nothing on modern Windows, so they work bare. |

Defaults: pitch mod `Ctrl+Shift+'`, stop all `Ctrl+Shift+;`.

`config.json` carries a `version`. Installing a build whose version is newer
throws away every binding the old one handed out and reissues them from
`Ctrl+Shift+1` onward — the numpad-era defaults were unreachable on a laptop, so
leaving them in place would have left dead keys. It runs once, then never again.

Avoid: anything `Win+…` (the shell and PowerToys own most of it), `Alt+Space`
(PowerToys Run), `Ctrl+Shift+Q/T/N/W/V` (these register fine and then break your
browser), and `` Ctrl+Shift+` `` (VS Code's new terminal). The app greys these
out with the reason when you try to bind one.

## Known limits

- **Fullscreen-exclusive games swallow `RegisterHotKey`.** Run the game
  borderless windowed. Fixing this properly means a low-level keyboard hook,
  which is a keylogger-shaped thing to install; not doing it by default.
- **Pitching down lengthens a clip.** At 0.35× a 4s clip runs 11s. Cut-previous
  and the stop hotkey are how you get out of it.
- Deleting a sound removes the entry, not the file — another profile may point
  at it. The folder icon in the header opens the sounds folder for hand-pruning.
- The NSIS installer is unsigned, so SmartScreen will warn on first run.

## Develop

```
pnpm install
pnpm tauri dev
pnpm tauri build      # -> src-tauri/target/release/bundle/nsis
cd src-tauri && cargo test   # pitch-walk logic
```

Config and clips live in `%APPDATA%\com.davidmg.thunderboard\`
(`config.json`, `sounds\`).
