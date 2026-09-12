<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import type { Config } from "./api";
  import { label, conflict, SUGGESTED } from "./keys";

  type HotkeyField = "pitchHotkey" | "stopHotkey" | "nextProfileHotkey";

  let {
    config = $bindable(),
    devices,
    failed,
    listening,
    onListen,
    onClose,
  }: {
    config: Config;
    devices: string[];
    failed: string[];
    listening: HotkeyField | null;
    onListen: (field: HotkeyField) => void;
    onClose: () => void;
  } = $props();

  const volumePct = $derived((config.masterVolume / 2) * 100);

  function deviceMatches(v: string) {
    const q = v.trim().toLowerCase();
    return !q || devices.some((d) => d.toLowerCase().includes(q));
  }

  function clearHotkey(field: HotkeyField) {
    config[field] = "";
  }

  // Persisted the same shape App.svelte reads to decide whether to nag.
  const SETUP_KEY = "setup";
  const SETUP_LABELS = [
    "Install VB-CABLE",
    'Discord › Voice › Input = "CABLE Output"',
    'Windows Sound › mic › Listen › "CABLE Input"',
    "Discord: noise suppression, echo cancel, AGC off",
  ] as const;

  function loadSetup(): boolean[] {
    try {
      const parsed = JSON.parse(localStorage.getItem(SETUP_KEY) ?? "null");
      if (Array.isArray(parsed) && parsed.length === 4) return parsed;
    } catch {
      /* corrupt value: fall back to a fresh checklist */
    }
    return [false, false, false, false];
  }

  let setup = $state(loadSetup());
  const setupDone = $derived(setup.filter(Boolean).length);

  function toggleSetup(i: number) {
    setup[i] = !setup[i];
    localStorage.setItem(SETUP_KEY, JSON.stringify(setup));
  }

  let autostart = $state(false);
  onMount(async () => {
    autostart = await invoke<boolean>("autostart");
  });
  function onAutostartChange() {
    void invoke("autostart", { on: autostart });
  }
</script>

{#snippet hotkeyChip(field: HotkeyField)}
  {@const value = config[field]}
  {@const isListening = listening === field}
  {@const isFailed = !isListening && !!value && failed.includes(value)}
  {@const msg = !isListening && !isFailed && value ? conflict(value) : ""}
  <button
    type="button"
    class="chip"
    class:listening={isListening}
    class:failed={isFailed}
    class:caution={!!msg}
    title={isFailed ? "Another app owns this shortcut. Click to rebind." : msg || undefined}
    onclick={() => onListen(field)}
  >
    {#if isListening}
      Press keys&hellip;
    {:else if isFailed}
      <span class="glyph">&#xE7BA;</span> taken
    {:else if value}
      {label(value)}
    {:else}
      <span class="unbound">Set hotkey</span>
    {/if}
  </button>
  {#if value}
    <button type="button" class="icon clear" title="Clear" onclick={() => clearHotkey(field)}>&#xE74D;</button>
  {/if}
{/snippet}

<div class="overlay">
  <div class="header">
    <span class="title">Settings</span>
    <span class="spacer"></span>
    <button type="button" class="icon" title="Close" onclick={onClose}>&#xE711;</button>
  </div>

  <div>
    <div class="row">
      <span class="label">Output (Discord hears)</span>
      <input class="field" list="output-devices" bind:value={config.outputDevice} placeholder="part of the device name" />
      {#if !deviceMatches(config.outputDevice)}
        <span class="glyph caution-mark" title="No device matches this text">&#xE7BA;</span>
      {/if}
    </div>
    <p class="hint">Substring match. Usually CABLE Input.</p>
  </div>

  <div>
    <div class="row">
      <span class="label">Monitor (you hear)</span>
      <input class="field" list="monitor-devices" bind:value={config.monitorDevice} placeholder="part of the device name" />
      {#if !deviceMatches(config.monitorDevice)}
        <span class="glyph caution-mark" title="No device matches this text">&#xE7BA;</span>
      {/if}
    </div>
    <p class="hint">What you hear. Leave empty for nothing.</p>
  </div>

  <datalist id="output-devices">
    {#each devices as d (d)}<option value={d}></option>{/each}
  </datalist>
  <datalist id="monitor-devices">
    {#each devices as d (d)}<option value={d}></option>{/each}
  </datalist>

  <div class="row">
    <span class="label">Master volume</span>
    <span class="glyph">&#xE767;</span>
    <input class="vol" type="range" min="0" max="2" step="0.05" bind:value={config.masterVolume} style="--fill: {volumePct}%" />
    <span class="volval">{config.masterVolume.toFixed(2)}</span>
  </div>

  <div class="row">
    <span class="label">Pitch mod</span>
    {@render hotkeyChip("pitchHotkey")}
  </div>
  <div class="row">
    <span class="label">Stop all</span>
    {@render hotkeyChip("stopHotkey")}
  </div>
  <div class="row">
    <span class="label">Next profile</span>
    {@render hotkeyChip("nextProfileHotkey")}
  </div>

  <details class="suggest">
    <summary>Shortcuts that are actually free</summary>
    <div class="suggest-body">
      {#each SUGGESTED as g (g.group)}
        <div class="group">
          <p class="group-title">{g.group}</p>
          <p class="group-note">{g.note}</p>
          <div class="keys">
            {#each g.keys as k (k)}
              <span class="key-chip">{label(k)}</span>
            {/each}
          </div>
        </div>
      {/each}
    </div>
  </details>

  <label class="row">
    <input type="checkbox" checked={autostart} onchange={(e) => {
      autostart = (e.currentTarget as HTMLInputElement).checked;
      onAutostartChange();
    }} />
    Start with Windows
  </label>

  <details class="suggest" open={setupDone < 4}>
    <summary>Discord routing ({setupDone} of 4 done)</summary>
    <div class="suggest-body">
      {#each SETUP_LABELS as text, i (i)}
        <label class="row check-row">
          <input type="checkbox" checked={setup[i]} onchange={() => toggleSetup(i)} />
          <span class="box"><span class="glyph">&#xE73E;</span></span>
          {text}
        </label>
      {/each}
    </div>
    <p class="hint">Discord's noise suppression will mangle pitched clips. It is not optional.</p>
  </details>
</div>

<style>
  .glyph,
  .icon {
    font-family: "Segoe Fluent Icons", "Segoe MDL2 Assets";
    font-size: 12px;
    line-height: 1;
  }

  .overlay {
    position: absolute;
    inset: 0;
    background: #202020;
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    overflow-y: auto;
    font-size: 12px;
  }

  .header {
    display: flex;
    align-items: center;
  }
  .title {
    font-size: 13px;
    font-weight: 600;
  }
  .spacer {
    flex: 1;
  }
  .icon {
    flex: none;
    width: 22px;
    height: 22px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--r-control);
    color: var(--fg-3);
  }
  .icon:hover {
    background: var(--stroke-strong);
    color: var(--fg);
  }

  .row {
    display: flex;
    align-items: center;
    min-height: 32px;
    gap: 8px;
  }
  .label {
    flex: none;
    width: 160px;
    color: var(--fg-2);
    font-size: 12px;
  }
  .hint {
    margin: 0 0 0 168px;
    color: var(--fg-3);
    font-size: 11px;
  }

  .field {
    flex: 1;
    min-width: 0;
    background: var(--layer);
    border: 1px solid var(--stroke);
    border-bottom-color: var(--stroke-strong);
    border-radius: var(--r-control);
    outline: none;
    padding: 5px 8px;
    font-size: 12px;
    user-select: text;
  }
  .field:focus {
    border-bottom: 2px solid var(--accent);
    padding-bottom: 4px;
  }
  .caution-mark {
    flex: none;
    color: var(--caution);
  }

  .vol {
    flex: 1;
    -webkit-appearance: none;
    appearance: none;
    height: 12px;
    background: transparent;
    outline: none;
  }
  .vol::-webkit-slider-runnable-track {
    height: 3px;
    border-radius: 2px;
    background: linear-gradient(to right, var(--accent) 0%, var(--accent) var(--fill), var(--stroke-strong) var(--fill), var(--stroke-strong) 100%);
    transition: background 200ms ease;
  }
  .vol::-webkit-slider-thumb {
    -webkit-appearance: none;
    width: 12px;
    height: 12px;
    margin-top: -4.5px;
    border-radius: 50%;
    background: var(--accent);
    cursor: default;
  }
  .volval {
    flex: none;
    width: 40px;
    text-align: right;
    color: var(--fg-3);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }

  .chip {
    flex: none;
    width: 140px;
    height: 22px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 4px;
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    background: var(--layer);
    border: 1px solid transparent;
    border-radius: var(--r-control);
    color: var(--fg-2);
    overflow: hidden;
    white-space: nowrap;
    transition:
      background 120ms ease,
      border-color 120ms ease,
      color 120ms ease;
  }
  .chip .unbound {
    color: var(--fg-3);
    font-style: italic;
  }
  .chip.listening {
    border-color: var(--accent);
    background: var(--accent-bg);
    color: var(--fg);
  }
  .chip.failed {
    color: var(--danger);
    background: var(--danger-bg);
  }
  .chip.caution {
    color: var(--caution);
  }
  .icon.clear {
    width: 18px;
    height: 18px;
  }

  .suggest summary {
    padding: 4px 2px;
    font-size: 12px;
    color: var(--fg-2);
    list-style: none;
    cursor: default;
  }
  .suggest summary::-webkit-details-marker {
    display: none;
  }
  .suggest-body {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 4px 2px 8px;
  }
  .group-title {
    margin: 0;
    font-size: 11px;
    font-weight: 600;
    color: var(--fg-2);
  }
  .group-note {
    margin: 2px 0 4px;
    font-size: 11px;
    color: var(--fg-3);
  }
  .keys {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .key-chip {
    padding: 2px 6px;
    font-size: 10.5px;
    font-variant-numeric: tabular-nums;
    background: var(--layer);
    border-radius: var(--r-control);
    color: var(--fg-2);
  }

  .check-row {
    gap: 6px;
    color: var(--fg-2);
    font-size: 12px;
  }
  .check-row input {
    position: absolute;
    width: 1px;
    height: 1px;
    opacity: 0;
  }
  .box {
    flex: none;
    width: 14px;
    height: 14px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--stroke-strong);
    border-radius: 3px;
  }
  .box .glyph {
    opacity: 0;
    font-size: 10px;
    color: var(--success);
  }
  .check-row input:checked + .box {
    border-color: var(--success);
  }
  .check-row input:checked + .box .glyph {
    opacity: 1;
  }
</style>
