<script lang="ts">
  import { tick } from "svelte";
  import type { Sound, Pitch } from "./api";
  import { label, conflict } from "./keys";

  let {
    sound = $bindable(),
    view,
    expanded,
    listening,
    failed,
    playing,
    onToggle,
    onListen,
    onDelete,
    onPreview,
    onAudition,
  }: {
    sound: Sound;
    /** Gallery pads play on click; list rows expand on click. */
    view: "list" | "gallery";
    expanded: boolean;
    listening: boolean;
    failed: boolean;
    playing: boolean;
    onToggle: () => void;
    onListen: () => void;
    onDelete: () => void;
    /** Fires the clip for real - monitor and out, like the hotkey. */
    onPreview: () => void;
    /** Monitor bus only, for hearing an offset while dragging it. */
    onAudition: () => void;
  } = $props();

  let renaming = $state(false);
  let nameInput = $state<HTMLInputElement>();

  // failed wins over conflict; both only make sense once a shortcut is bound.
  const conflictMsg = $derived(sound.hotkey ? conflict(sound.hotkey) : "");
  const caution = $derived(!listening && !failed && !!sound.hotkey && !!conflictMsg);
  const chipTitle = $derived(
    failed ? "Another app owns this shortcut. Click to rebind." : caution ? conflictMsg : undefined,
  );

  const volumePct = $derived((sound.volume / 2) * 100);

  const pitchText = $derived.by(() => {
    const p = sound.pitch;
    if (p.mode === "random") return `rnd ${p.min.toFixed(1)}–${p.max.toFixed(1)}`;
    const sign = p.step < 0 ? "−" : p.step > 0 ? "+" : "";
    return `${sign}${Math.abs(p.step).toFixed(2)}`;
  });
  const pitchGlyph = $derived(
    sound.pitch.mode === "step" ? (sound.pitch.step < 0 ? "" : sound.pitch.step > 0 ? "" : "") : "",
  );

  async function startRename(e: MouseEvent) {
    e.stopPropagation();
    e.preventDefault();
    renaming = true;
    await tick();
    nameInput?.focus();
    nameInput?.select();
  }

  /** A pad is a button you hit; a row is a disclosure you open. */
  function press() {
    if (view === "gallery") onPreview();
    else onToggle();
  }

  /**
   * Step's min/max are the floor and ceiling a walk may wander to, so they are
   * deliberately wide. Random picks a fresh rate every single press, and that
   * same span turns every clip into a lottery - so it starts at a gentle
   * +/-0.20 instead of inheriting the walk's limits.
   */
  const RANDOM_MIN = 0.8;
  const RANDOM_MAX = 1.2;

  function setMode(mode: Pitch["mode"]) {
    if (sound.pitch.mode === mode) return;
    const { min, max } = sound.pitch;
    sound.pitch =
      mode === "step"
        ? { mode: "step", step: -0.12, min, max }
        : { mode: "random", min: RANDOM_MIN, max: RANDOM_MAX };
  }

  const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

  const offsetPct = $derived((Math.min(sound.offset, 2) / 2) * 100);

  // Binding a number input straight to the model clamps/reformats on every keystroke,
  // which makes "0.1" -> "0.5" impossible (deleting the "1" leaves "0." and it snaps
  // back). Each field gets its own free-typing draft string instead; only Enter/blur
  // parses, clamps and writes it back, and Escape reverts without touching the model.
  function useDraft(get: () => number, set: (v: number) => void, lo: number, hi: number, onCommit?: () => void) {
    let value = $state(get().toString());
    let focused = false;
    let skipNextBlur = false;

    $effect(() => {
      const current = get();
      if (!focused) value = current.toString();
    });

    function commit() {
      focused = false;
      const n = parseFloat(value);
      if (Number.isFinite(n)) {
        set(clamp(n, lo, hi));
        onCommit?.();
      }
      value = get().toString();
    }

    return {
      get value() {
        return value;
      },
      set value(v: string) {
        value = v;
      },
      onfocus() {
        focused = true;
      },
      onblur() {
        if (skipNextBlur) {
          skipNextBlur = false;
          return;
        }
        commit();
      },
      onkeydown(e: KeyboardEvent) {
        e.stopPropagation();
        if (e.key === "Enter") {
          commit();
          skipNextBlur = true;
          (e.target as HTMLInputElement).blur();
        } else if (e.key === "Escape") {
          value = get().toString();
          skipNextBlur = true;
          (e.target as HTMLInputElement).blur();
        }
      },
    };
  }

  const stepDraft = useDraft(
    () => (sound.pitch.mode === "step" ? sound.pitch.step : 0),
    (v) => {
      if (sound.pitch.mode === "step") sound.pitch.step = v;
    },
    -1,
    1,
  );
  const minDraft = useDraft(
    () => sound.pitch.min,
    (v) => {
      sound.pitch.min = v;
      if (sound.pitch.min > sound.pitch.max) sound.pitch.max = sound.pitch.min;
    },
    0.1,
    6,
  );
  const maxDraft = useDraft(
    () => sound.pitch.max,
    (v) => {
      sound.pitch.max = v;
      if (sound.pitch.max < sound.pitch.min) sound.pitch.min = sound.pitch.max;
    },
    0.1,
    6,
  );
  const offsetDraft = useDraft(
    () => sound.offset,
    (v) => {
      sound.offset = v;
    },
    0,
    30,
    () => onAudition(),
  );
</script>

<div class="row-wrap" class:gallery={view === "gallery"}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="row"
    class:expanded
    class:playing
    role="button"
    tabindex="0"
    title={view === "gallery" ? "Play" : undefined}
    onclick={press}
    onkeydown={(e) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        press();
      }
    }}
  >
    <button
      type="button"
      class="icon play"
      title="Play"
      onclick={(e) => {
        e.stopPropagation();
        onPreview();
      }}>&#xE767;</button
    >

    {#if renaming}
      <input
        class="rename"
        bind:this={nameInput}
        bind:value={sound.name}
        onclick={(e) => e.stopPropagation()}
        onblur={() => (renaming = false)}
        onkeydown={(e) => {
          e.stopPropagation();
          if (e.key === "Enter") renaming = false;
        }}
      />
    {:else}
      <!-- A <span>, not a <button>: a nested button swallowed the click that was
           supposed to expand the row, which is most of the row's width. The row
           itself is the button. -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <span class="name" title={sound.name} ondblclick={startRename}>
        {sound.name}
      </span>
    {/if}

    <button type="button" class="icon edit" title="Rename" onclick={startRename}>&#xE70F;</button>

    <button
      type="button"
      class="icon delete"
      title="Delete"
      onclick={(e) => {
        e.stopPropagation();
        onDelete();
      }}>&#xE74D;</button
    >

    <button type="button" class="chip" class:listening class:failed class:caution title={chipTitle} onclick={(e) => { e.stopPropagation(); onListen(); }}>
      {#if listening}
        Press keys&hellip;
      {:else if failed}
        <span class="glyph">&#xE7BA;</span> taken
      {:else if sound.hotkey}
        {label(sound.hotkey)}
      {:else}
        <span class="unbound">Set hotkey</span>
      {/if}
    </button>

    <span class="pitch-summary">
      {#if sound.pitch.mode === "random"}
        rnd {sound.pitch.min.toFixed(1)}&ndash;{sound.pitch.max.toFixed(1)}
      {:else}
        {pitchText}{#if pitchGlyph}<span class="glyph">{pitchGlyph}</span>{/if}
      {/if}
    </span>

    {#if view === "gallery"}
      <!-- The pad's own click is taken by playback, so settings need their own
           target. In list view the row is the disclosure and this is redundant. -->
      <button
        type="button"
        class="icon expand"
        class:open={expanded}
        title={expanded ? "Close settings" : "Settings"}
        onclick={(e) => {
          e.stopPropagation();
          onToggle();
        }}>&#xE70D;</button
      >
    {/if}
  </div>

  {#if expanded}
    <div class="panel">
      <div class="offset-block">
        <div class="line offset-line">
          <span class="offset-label">Offset</span>
          <input
            class="field"
            type="number"
            min="0"
            max="30"
            step="0.005"
            bind:value={offsetDraft.value}
            onfocus={offsetDraft.onfocus}
            onblur={offsetDraft.onblur}
            onkeydown={offsetDraft.onkeydown}
            onclick={(e) => e.stopPropagation()}
          />
          <span class="unit">s</span>
          <span class="offset-hint">skips leading silence</span>
        </div>
        <input
          class="vol offset-slider"
          type="range"
          min="0"
          max="2"
          step="0.005"
          bind:value={sound.offset}
          onchange={onAudition}
          onclick={(e) => e.stopPropagation()}
          style="--fill: {offsetPct}%"
        />
      </div>

      <div class="line">
        <span class="glyph">&#xE767;</span>
        <input class="vol" type="range" min="0" max="2" step="0.05" bind:value={sound.volume} style="--fill: {volumePct}%" />
        <span class="volval">{sound.volume.toFixed(2)}</span>
      </div>

      <div class="line pitch-line">
        <div class="seg">
          <button type="button" class:accent={sound.pitch.mode === "step"} class:ghost={sound.pitch.mode !== "step"} onclick={() => setMode("step")}>Step</button>
          <button type="button" class:accent={sound.pitch.mode === "random"} class:ghost={sound.pitch.mode !== "random"} onclick={() => setMode("random")}>Random</button>
        </div>
        {#if sound.pitch.mode === "step"}
          <label class="num">step<input class="field" type="number" step="0.01" min="-1" max="1" bind:value={stepDraft.value} onfocus={stepDraft.onfocus} onblur={stepDraft.onblur} onkeydown={stepDraft.onkeydown} /></label>
        {/if}
        <label class="num">min<input class="field" type="number" step="0.05" min="0.1" max="6" bind:value={minDraft.value} onfocus={minDraft.onfocus} onblur={minDraft.onblur} onkeydown={minDraft.onkeydown} /></label>
        <label class="num">max<input class="field" type="number" step="0.05" min="0.1" max="6" bind:value={maxDraft.value} onfocus={maxDraft.onfocus} onblur={maxDraft.onblur} onkeydown={maxDraft.onkeydown} /></label>
      </div>

      <p class="hint">
        {sound.pitch.mode === "step"
          ? "Each repeat of this hotkey shifts the rate by step. Rate is pitch, so down is also slower."
          : "Every press picks a fresh rate in range."}
      </p>
    </div>
  {/if}
</div>

<style>
  .glyph,
  .icon {
    font-family: "Segoe Fluent Icons", "Segoe MDL2 Assets";
    font-size: 12px;
    line-height: 1;
  }

  .row {
    position: relative;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 32px;
    padding: 0 8px;
    border-radius: var(--r-control);
    transition: background 120ms ease;
  }
  .row:hover {
    background: var(--hover);
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
  .icon.delete,
  .icon.edit {
    opacity: 0;
    transition: opacity 120ms ease;
  }
  .row:hover .icon.delete,
  .row:hover .icon.edit {
    opacity: 1;
  }
  .icon.delete:hover {
    color: var(--danger);
  }

  .name {
    flex: 1;
    min-width: 0;
    padding: 0;
    text-align: left;
    font-size: 12.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rename {
    flex: 1;
    min-width: 0;
    background: var(--press);
    border: 1px solid var(--accent);
    border-radius: var(--r-control);
    outline: none;
    padding: 1px 4px;
    font-size: 12.5px;
    user-select: text;
  }

  .chip {
    flex: none;
    width: 96px;
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

  .pitch-summary {
    flex: none;
    width: 70px;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 2px;
    color: var(--fg-3);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }

  /* Inline editor panel: no transition, it should just be there. */
  .panel {
    background: var(--layer);
    border-radius: var(--r-control);
    padding: 8px 10px;
    margin: 2px 0 6px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .line {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .pitch-line {
    gap: 12px;
    flex-wrap: wrap;
  }

  .offset-block {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .offset-line {
    gap: 6px;
  }
  .offset-label {
    flex: none;
    color: var(--fg-2);
    font-size: 11px;
  }
  .unit {
    flex: none;
    color: var(--fg-3);
    font-size: 11px;
  }
  .offset-hint {
    margin-left: auto;
    color: var(--fg-3);
    font-size: 11px;
  }
  .offset-slider {
    width: 100%;
    flex: none;
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

  .seg {
    display: flex;
    gap: 4px;
  }
  .ghost,
  .accent {
    padding: 4px 10px;
    font-size: 11px;
    border-radius: var(--r-control);
    border: 1px solid var(--stroke);
    background: var(--layer);
  }
  .ghost:hover {
    background: var(--hover);
  }
  .accent {
    background: var(--accent);
    border-color: transparent;
    color: #003e5c;
    font-weight: 600;
  }
  .accent:hover {
    opacity: 0.9;
  }

  .num {
    display: flex;
    align-items: center;
    gap: 4px;
    color: var(--fg-3);
    font-size: 11px;
  }
  .field {
    width: 64px;
    background: var(--layer);
    border: 1px solid var(--stroke);
    border-bottom-color: var(--stroke-strong);
    border-radius: var(--r-control);
    outline: none;
    padding: 4px 6px;
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    user-select: text;
  }
  .field:focus {
    border-bottom: 2px solid var(--accent);
    padding-bottom: 3px;
  }

  .hint {
    margin: 0;
    color: var(--fg-3);
    font-size: 11px;
  }

  /* Gallery: the same six children, re-placed. `display: contents` promotes the
     pad and its editor to grid items of .body so the editor can span the full
     width; the cost is that pads after the expanded one shuffle down to below
     it. ponytail: measuring the resolved column count in JS would keep the row
     intact - do that if the shuffle actually annoys. */
  .row-wrap.gallery {
    display: contents;
  }
  .row-wrap.gallery .row {
    display: grid;
    grid-template-columns: auto 1fr auto;
    grid-template-areas:
      "play name  name"
      "chip chip  pitch";
    align-content: center;
    height: 76px;
    padding: 8px 10px;
    gap: 6px;
    background: var(--layer);
    border: 1px solid var(--stroke);
    border-radius: var(--r-surface);
  }
  .row-wrap.gallery .row:hover {
    background: var(--hover);
    border-color: var(--stroke-strong);
  }
  .row-wrap.gallery .row:active {
    background: var(--press);
  }
  .row-wrap.gallery .row.expanded {
    border-color: var(--accent);
  }
  .row-wrap.gallery .row.playing {
    border-color: var(--accent);
    background: var(--accent-bg);
  }

  .row-wrap.gallery .play {
    grid-area: play;
    align-self: start;
    width: auto;
    height: auto;
    pointer-events: none; /* the whole pad is the play button here */
  }
  .row-wrap.gallery .row.playing .play {
    color: var(--accent);
  }
  .row-wrap.gallery .name,
  .row-wrap.gallery .rename {
    grid-area: name;
    align-self: start;
  }
  .row-wrap.gallery .name {
    /* Two lines, then clip. Reserves the corner controls' width so revealing
       delete on hover never reflows the title. */
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    white-space: normal;
    overflow-wrap: anywhere;
    line-height: 16px;
    padding-right: 66px;
  }
  .row-wrap.gallery .chip {
    grid-area: chip;
    width: auto;
    max-width: 100%;
    justify-content: flex-start;
    padding: 0 6px;
    align-self: end;
  }
  .row-wrap.gallery .pitch-summary {
    grid-area: pitch;
    width: auto;
    align-self: end;
  }
  .row-wrap.gallery .delete,
  .row-wrap.gallery .edit,
  .row-wrap.gallery .expand {
    position: absolute;
    top: 6px;
    width: 20px;
    height: 20px;
  }
  .row-wrap.gallery .expand {
    right: 6px;
    font-size: 10px;
    opacity: 0.7;
    transition: transform 120ms ease, opacity 120ms ease;
  }
  .row-wrap.gallery .row:hover .expand {
    opacity: 1;
  }
  .row-wrap.gallery .expand.open {
    opacity: 1;
    color: var(--accent);
    transform: rotate(180deg);
  }
  .row-wrap.gallery .delete {
    right: 28px;
  }
  .row-wrap.gallery .edit {
    right: 50px;
  }
  .row-wrap.gallery .panel {
    grid-column: 1 / -1;
    border-top: 2px solid var(--accent);
    border-radius: 0 0 var(--r-surface) var(--r-surface);
    margin: 0 0 4px;
  }

  /* The chevron is the gallery's disclosure; in list view the row itself is. */
  .expand {
    display: none;
  }
</style>
