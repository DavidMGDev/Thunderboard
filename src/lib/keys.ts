/**
 * Hotkey capture.
 *
 * Tauri's shortcut parser takes `Modifier+...+Code` where Code is a
 * `keyboard-types` name, and those names are the same strings the DOM puts in
 * `KeyboardEvent.code`. So capture is a join, not a lookup table.
 */

const BARE_MODIFIERS = /^(Control|Alt|Shift|Meta)(Left|Right)$/;

/** Returns a Tauri shortcut string, or "" if the event was a modifier alone. */
export function toShortcut(e: KeyboardEvent): string {
  if (BARE_MODIFIERS.test(e.code) || !e.code) return "";
  const parts: string[] = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey) parts.push("Super");
  parts.push(e.code);
  return parts.join("+");
}

/** `KeyboardEvent.code` names that read as punctuation, not as words. */
const GLYPHS: Record<string, string> = {
  Minus: "-",
  Equal: "=",
  BracketLeft: "[",
  BracketRight: "]",
  Backslash: "\\",
  Semicolon: ";",
  Quote: "'",
  Comma: ",",
  Period: ".",
  Slash: "/",
  Backquote: "`",
};

/** "Ctrl+Shift+Quote" -> "Ctrl + Shift + '". Display only. */
export function label(shortcut: string): string {
  if (!shortcut) return "";
  return shortcut
    .split("+")
    .map((part) => {
      if (GLYPHS[part]) return GLYPHS[part];
      return part
        .replace(/^Key/, "")
        .replace(/^Digit/, "")
        .replace(/^Numpad/, "Num ")
        .replace(/^Super$/, "Win")
        .replace(/^PageUp$/, "PgUp")
        .replace(/^PageDown$/, "PgDn")
        .replace(/^ScrollLock$/, "ScrLk");
    })
    .join(" + ");
}

const digits = (prefix: string) =>
  ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"].map((d) => `${prefix}Digit${d}`);

const PUNCTUATION = [
  "Minus",
  "Equal",
  "BracketLeft",
  "BracketRight",
  "Backslash",
  "Semicolon",
  "Quote",
  "Comma",
  "Period",
  "Slash",
];

/**
 * Combos free on a laptop keyboard — no numpad, and nothing that needs Fn.
 *
 * F13-F24 stay first because they are the only single-press options with zero
 * conflicts anywhere: no keyboard sends them, so nothing has claimed them.
 * Remap keys you never use (CapsLock, Menu, Right-Alt) onto them with PowerToys
 * Keyboard Manager, or use a macropad.
 *
 * Everything else is a Ctrl+Shift combo, because a global hotkey outranks the
 * focused app: binding one an app already uses takes it away from that app
 * everywhere. Ctrl+Shift over digits and punctuation is the largest bank that
 * neither Windows, PowerToys, Discord nor a browser wants.
 */
export const SUGGESTED: { group: string; keys: string[]; note: string }[] = [
  {
    group: "F13 - F24",
    keys: Array.from({ length: 12 }, (_, i) => `F${13 + i}`),
    note: "Zero conflicts, single press, survives fullscreen games. No keyboard sends them — remap CapsLock or Menu onto one with PowerToys Keyboard Manager.",
  },
  {
    group: "Ctrl + Shift + number row",
    keys: digits("Ctrl+Shift+"),
    note: "Ten slots on every laptop. Ctrl+number is browser tabs; adding Shift is free.",
  },
  {
    group: "Ctrl + Shift + punctuation",
    keys: PUNCTUATION.map((k) => `Ctrl+Shift+${k}`),
    note: "Ten more. The app's own defaults live here, on ' and ;.",
  },
  {
    group: "Ctrl + Alt + number row",
    keys: digits("Ctrl+Alt+"),
    note: "Ten more — but Ctrl+Alt is AltGr on international layouts, so skip these if you ever type on one.",
  },
  {
    group: "Ctrl + Shift + F-row",
    keys: Array.from({ length: 12 }, (_, i) => `Ctrl+Shift+F${i + 1}`),
    note: "Twelve more. Needs Fn on laptops whose F-row defaults to media keys.",
  },
  {
    group: "Navigation cluster",
    keys: [
      "Ctrl+Shift+Insert",
      "Ctrl+Shift+Home",
      "Ctrl+Shift+End",
      "Ctrl+Shift+PageUp",
      "Ctrl+Shift+PageDown",
      "ScrollLock",
      "Pause",
    ],
    note: "ScrollLock and Pause do nothing on modern Windows, so they work bare.",
  },
];

/**
 * Combos to refuse outright, with the reason. Either registration fails, or it
 * succeeds and quietly steals the combo from an app that needs it.
 *
 * ponytail: a hand-written list of the ones that actually bite, not an
 * exhaustive table of every Windows shortcut. Add entries when one surprises you.
 */
const RESERVED: Record<string, string> = {
  "Super+KeyV": "Windows clipboard history",
  "Super+KeyC": "Windows Copilot",
  "Super+KeyX": "Windows quick link menu",
  "Super+KeyL": "Lock screen",
  "Super+KeyD": "Show desktop",
  "Super+KeyE": "File Explorer",
  "Super+KeyR": "Run dialog",
  "Super+Shift+KeyS": "Snipping Tool",
  "Super+Shift+KeyC": "PowerToys Color Picker",
  "Super+Shift+KeyT": "PowerToys Text Extractor",
  "Super+Ctrl+KeyT": "PowerToys Always on Top",
  "Alt+Space": "PowerToys Run",
  "Alt+Tab": "Task switcher",
  "Ctrl+Alt+Delete": "Windows security screen",
  "Ctrl+Shift+Escape": "Task Manager",
  // These register fine and then break the app you were using.
  "Ctrl+Shift+KeyQ": "Quit, in Firefox and others",
  "Ctrl+Shift+KeyT": "Reopen closed tab",
  "Ctrl+Shift+KeyN": "New private window",
  "Ctrl+Shift+KeyW": "Close window",
  "Ctrl+Shift+KeyV": "Paste as plain text",
  "Ctrl+Shift+Backquote": "New terminal, in VS Code",
};

/** Human-readable reason this combo is a bad idea, or "" if it is fine. */
export function conflict(shortcut: string): string {
  const owner = RESERVED[shortcut];
  if (owner) return `Taken by ${owner}`;
  // Alt+<digit> on the numpad is how alt-codes are typed.
  if (/^Alt\+Numpad\d$/.test(shortcut)) return "Alt + numpad types alt-codes";
  // A bare letter or digit would eat every keystroke system-wide.
  if (/^(Key|Digit)[A-Z0-9]$/.test(shortcut)) return "Needs a modifier — this would eat your typing";
  return "";
}
