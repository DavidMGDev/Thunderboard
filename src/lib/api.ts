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

export type Profile = { id: string; name: string; sounds: Sound[] };

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

/** Persists, re-binds every hotkey, and returns the ones Windows refused. */
export const saveConfig = (config: Config) => invoke<string[]>("save_config", { config });

export const importSounds = (paths: string[]) =>
  invoke<Sound[]>("import_sounds", { paths });

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
