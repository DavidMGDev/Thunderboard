/** Typed edge against src-tauri. Field names are serde camelCase. */
import { invoke } from "@tauri-apps/api/core";

export type Pitch =
  | { mode: "step"; step: number; min: number; max: number }
  | { mode: "random"; min: number; max: number };

export type Sound = {
  id: string;
  name: string;
  file: string;
  hotkey: string;
  volume: number;
  /** Seconds of leading silence to skip. */
  offset: number;
  pitch: Pitch;
};

export type Profile = {
  id: string;
  name: string;
  sounds: Sound[];
  /** A folder this profile mirrors; empty means it is hand-built. */
  folder: string;
};

export type Config = {
  /** Schema version. Round-trip it untouched: Rust migrates on anything older. */
  version: number;
  profiles: Profile[];
  active: string;
  outputDevice: string;
  monitorDevice: string;
  pitchHotkey: string;
  stopHotkey: string;
  nextProfileHotkey: string;
  masterVolume: number;
};

export const loadConfig = () => invoke<Config>("load_config");

/**
 * Persists and queues a re-bind. The shortcuts Windows refused arrive on the
 * `hotkeys` event rather than as a return value - binding inline used to
 * deadlock the main thread against a keypress and kill every hotkey.
 */
export const saveConfig = (config: Config) => invoke<void>("save_config", { config });

/** Copies into `dest` if given, else into the app's own sounds folder. */
export const importSounds = (paths: string[], dest?: string) =>
  invoke<Sound[]>("import_sounds", { paths, dest: dest || null });

/** Re-reads a folder profile, keeping tuning and order for clips still there. */
export const syncFolder = (dir: string, existing: Sound[]) =>
  invoke<Sound[]>("sync_folder", { dir, existing });

export const listDevices = () => invoke<string[]>("list_devices");

/** Auditions on the monitor bus at normal pitch — never out to the call. */
export const preview = (id: string) => invoke<void>("preview", { id });

export const stopAll = () => invoke<void>("stop_all");

export const setPitchMod = (on: boolean) => invoke<void>("pitch_mod", { on });

export const soundsDir = () => invoke<string>("sounds_dir");

/** Leaves for good — hotkeys stop working. Confirm before calling. */
export const quit = () => invoke<void>("quit");

export const DEFAULT_PITCH: Pitch = { mode: "step", step: -0.12, min: 0.35, max: 2.5 };

export const uid = () => Math.random().toString(36).slice(2, 10);
