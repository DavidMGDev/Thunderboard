<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { open } from "@tauri-apps/plugin-dialog";
  import {
    loadConfig,
    saveConfig,
    importSounds,
    listDevices,
    preview,
    setPitchMod,
    soundsDir,
    syncFolder,
    uid,
    quit,
    type Config,
    type Profile,
  } from "./lib/api";
  import { toShortcut, label } from "./lib/keys";
  import SoundRow from "./lib/SoundRow.svelte";
  import Settings from "./lib/Settings.svelte";

  const win = getCurrentWindow();

  let config = $state<Config | undefined>();
  let devices = $state<string[]>([]);
  let pitchOn = $state(false);
  let failedHotkeys = $state<string[]>([]);
  let playingId = $state<string | null>(null);

  let expandedId = $state<string | null>(null);
  let listeningId = $state<string | null>(null);
  let settingsListening = $state<"pitchHotkey" | "stopHotkey" | "nextProfileHotkey" | null>(null);
  let settingsOpen = $state(false);
  let profileFlyoutOpen = $state(false);
  let renamingProfileId = $state<string | null>(null);
  let dragging = $state(false);

  /** A pending destructive action, or null. One dialog serves all of them. */
  let confirming = $state<{ title: string; body: string; label: string; act: () => void } | null>(
    null,
  );

  /** Set when the mirrored folder could not be read - unplugged drive, rename. */
  let folderMissing = $state(false);

  // Not in config.json: every write there re-registers every global hotkey, and
  // flipping the layout has no business unbinding your board for a frame.
  let view = $state<"list" | "gallery">(
    localStorage.getItem("view") === "gallery" ? "gallery" : "list",
  );

  function toggleView() {
    view = view === "list" ? "gallery" : "list";
    localStorage.setItem("view", view);
  }

  // Bumped whenever Settings closes so the footer re-reads the checklist it wrote.
  let setupVersion = $state(0);

  const profile = $derived.by<Profile | undefined>(() => {
    if (!config) return undefined;
    return config.profiles.find((p) => p.id === config!.active) ?? config.profiles[0];
  });

  const setupComplete = $derived.by(() => {
    setupVersion;
    try {
      const flags: unknown = JSON.parse(localStorage.getItem("setup") ?? "null");
      return Array.isArray(flags) && flags.length === 4 && flags.every((v) => v === true);
    } catch {
      return false;
    }
  });

  // True while a config we just pulled FROM the backend is being written into
  // `config` — that assignment must not immediately bounce back to save_config.
  let suppressSave = true;
  let saveHandle = 0;

  $effect(() => {
    if (!config) return;
    void JSON.stringify(config); // deep-read every field so nested sound edits retrigger this
    if (suppressSave) {
      suppressSave = false;
      return;
    }
    const snapshot = config;
    window.clearTimeout(saveHandle);
    saveHandle = window.setTimeout(async () => {
      await saveConfig(snapshot);
    }, 250);
  });

  async function init() {
    config = await loadConfig();
    devices = await listDevices();
  }

  async function reloadConfig() {
    suppressSave = true;
    config = await loadConfig();
  }

  async function onShown() {
    devices = await listDevices();
    expandedId = null;
    listeningId = null;
    settingsListening = null;
    settingsOpen = false;
    profileFlyoutOpen = false;
    renamingProfileId = null;
    playingId = null;
    dragging = false;
    confirming = null;
    // Opening the window is the cheap moment to catch up with the folder: one
    // directory listing, and only for profiles that actually mirror one.
    if (profile?.folder) void syncNow();
  }

  /** Re-reads the mirrored folder. Rows keep their hotkeys, order and tuning. */
  async function syncNow() {
    const p = profile;
    if (!p?.folder) return;
    try {
      p.sounds = await syncFolder(p.folder, p.sounds);
      folderMissing = false;
    } catch {
      // Keep the rows we have rather than wiping a board over an absent drive.
      folderMissing = true;
    }
  }

  async function pickFolder() {
    const p = profile;
    if (!p) return;
    const dir = await open({ directory: true });
    if (typeof dir !== "string") return;
    p.folder = dir;
    await syncNow();
  }

  /**
   * A folder profile gets the copies put in its folder and then re-scans, so
   * the folder stays the single source of truth for what is on the board.
   */
  async function add(paths: string[]) {
    const p = profile;
    if (!p) return;
    const sounds = await importSounds(paths, p.folder);
    if (p.folder) await syncNow();
    else p.sounds.push(...sounds);
  }

  onMount(() => {
    void init();
    void listen("shown", () => void onShown());
    void listen<boolean>("pitch", (e) => (pitchOn = e.payload));
    void listen<string[]>("hotkeys", (e) => (failedHotkeys = e.payload));
    void listen<{ id: string; speed: number }>("played", (e) => {
      playingId = e.payload.id;
      setTimeout(() => (playingId = null), 400);
    });
    void listen<string>("profile", () => void reloadConfig());
    void getCurrentWebview().onDragDropEvent((e) => {
      if (e.payload.type === "over") dragging = true;
      else if (e.payload.type === "drop") {
        dragging = false;
        void add(e.payload.paths);
      } else dragging = false;
    });
  });

  function removeSound(id: string) {
    const p = profile;
    const sound = p?.sounds.find((s) => s.id === id);
    if (!p || !sound) return;
    confirming = {
      title: `Delete "${sound.name}"?`,
      body: p.folder
        ? "Removes the row and its hotkey. The file stays in the folder, so the next sync brings it back - delete the file itself to be rid of it."
        : "Removes the row and its hotkey. The audio file stays in the sounds folder.",
      label: "Delete",
      act: () => {
        p.sounds = p.sounds.filter((s) => s.id !== id);
        if (expandedId === id) expandedId = null;
        if (listeningId === id) listeningId = null;
      },
    };
  }

  function togglePitch() {
    void setPitchMod(!pitchOn);
  }

  // preview() runs against the backend's saved config, so an offset/volume
  // drag that hasn't hit the 250ms debounce yet would preview the old value.
  // Flush the pending save first so preview matches what's on screen.
  async function previewNow(id: string) {
    if (!config) return;
    window.clearTimeout(saveHandle);
    await saveConfig(config);
    await preview(id);
  }

  async function pickFiles() {
    const picked = await open({
      multiple: true,
      filters: [
        { name: "Audio", extensions: ["mp3", "wav", "ogg", "flac", "m4a", "aac", "opus", "wma"] },
      ],
    });
    if (picked) await add(picked);
  }

  /** Escape hatch for pruning clips by hand - deleting a row keeps the file. */
  async function openSoundsFolder() {
    try {
      const dir = await soundsDir();
      await revealItemInDir(dir);
    } catch {
      /* nothing imported yet - the sounds folder doesn't exist until it is */
    }
  }

  function closeSettings() {
    settingsOpen = false;
    setupVersion++;
  }

  function toggleFlyout() {
    profileFlyoutOpen = !profileFlyoutOpen;
  }

  function selectProfile(id: string) {
    if (!config) return;
    config.active = id;
    profileFlyoutOpen = false;
    folderMissing = false;
    if (profile?.folder) void syncNow();
  }

  function addProfile() {
    if (!config) return;
    const p: Profile = { id: uid(), name: "New profile", sounds: [], folder: "" };
    config.profiles.push(p);
    config.active = p.id;
    renamingProfileId = p.id;
  }

  function removeProfile(p: Profile) {
    // A board with no profiles has nothing to be active, so the last one stays.
    if (!config || config.profiles.length <= 1) return;
    const n = p.sounds.length;
    confirming = {
      title: `Delete profile "${p.name}"?`,
      body: `${n} sound${n === 1 ? "" : "s"} and their hotkeys go with it. The audio files themselves stay on disk.`,
      label: "Delete",
      act: () => {
        if (!config) return;
        config.profiles = config.profiles.filter((x) => x.id !== p.id);
        if (config.active === p.id) config.active = config.profiles[0].id;
      },
    };
  }

  /** Stops a control inside the flyout row from also selecting the profile. */
  function isolate(e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
  }

  function onKeydown(e: KeyboardEvent) {
    if (listeningId || settingsListening) {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        listeningId = null;
        settingsListening = null;
        return;
      }
      const shortcut = toShortcut(e);
      if (!shortcut) return; // bare modifier press - keep listening for the real key
      if (listeningId) {
        const sound = profile?.sounds.find((s) => s.id === listeningId);
        if (sound) sound.hotkey = shortcut;
        listeningId = null;
      } else if (settingsListening && config) {
        config[settingsListening] = shortcut;
        settingsListening = null;
      }
      return;
    }
    if (e.key !== "Escape") return;
    // A focused field owns its own Escape - the number drafts revert on it, and
    // this handler runs in the capture phase, so without the guard the window
    // would hide before the field ever saw the key.
    const t = e.target;
    if (t instanceof HTMLElement && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName))) {
      return;
    }
    if (confirming) confirming = null;
    else if (settingsOpen) closeSettings();
    else if (profileFlyoutOpen) profileFlyoutOpen = false;
    else void win.hide();
  }
</script>

<!-- Capture phase: while a hotkey is being recorded the keystroke must not reach
     the focused row or input first, and stopPropagation only holds it back on
     the way down. -->
<svelte:window onkeydowncapture={onKeydown} oncontextmenu={(e) => e.preventDefault()} />

<div class="panel">
  {#if pitchOn}
    <div class="caution-strip"></div>
  {/if}

  {#if config && profile}
    <header class="header" data-tauri-drag-region>
      <div class="profile-wrap">
        <button class="profile-btn" onclick={toggleFlyout} title="Switch profile">
          <span class="profile-name">{profile.name}</span>
          <span class="chev" class:open={profileFlyoutOpen}>&#xE70D;</span>
        </button>
        {#if profileFlyoutOpen}
          <div class="flyout">
            {#each config.profiles as p (p.id)}
              <div class="flyout-row">
                {#if renamingProfileId === p.id}
                  <!-- svelte-ignore a11y_autofocus -->
                  <input
                    class="rename"
                    autofocus
                    bind:value={p.name}
                    onclick={isolate}
                    onblur={() => (renamingProfileId = null)}
                    onkeydown={(e) => e.key === "Enter" && (renamingProfileId = null)}
                  />
                {:else}
                  <button
                    class="flyout-name"
                    title="Double-click to rename"
                    onclick={() => selectProfile(p.id)}
                    ondblclick={(e) => {
                      isolate(e);
                      renamingProfileId = p.id;
                    }}
                  >
                    {p.name}
                  </button>
                {/if}
                {#if config.profiles.length > 1}
                  <button
                    class="flyout-icon"
                    title="Delete profile"
                    onclick={(e) => {
                      isolate(e);
                      removeProfile(p);
                    }}>&#xE74D;</button
                  >
                {/if}
              </div>
            {/each}
            <button class="flyout-row flyout-new" onclick={addProfile}>
              <span class="flyout-icon pinned">&#xE710;</span> New profile
            </button>
          </div>
        {/if}
      </div>

      <button class="pitch-pill" class:on={pitchOn} onclick={togglePitch}>
        PITCH {pitchOn ? "ON" : "OFF"} · {label(config.pitchHotkey)}
      </button>

      <div class="header-actions">
        {#if profile.folder}
          <button
            class="icon"
            class:caution={folderMissing}
            title={folderMissing
              ? `Cannot read ${profile.folder}`
              : `Sync with ${profile.folder}`}
            onclick={() => void syncNow()}>&#xE72C;</button
          >
        {/if}
        <button class="icon" title="Add sounds" onclick={pickFiles}>&#xE710;</button>
        <button class="icon" title="Open sounds folder" onclick={openSoundsFolder}>&#xE838;</button>
        <button
          class="icon"
          title={view === "list" ? "Gallery view" : "List view"}
          onclick={toggleView}>{view === "list" ? "" : ""}</button
        >
        <button class="icon" title="Settings" onclick={() => (settingsOpen = true)}>&#xE713;</button>
      </div>

      <!-- Flush into the corner and full header height, the way Windows draws its
           own caption buttons. They are not part of .header-actions: those are
           app controls and these are window controls. -->
      <div class="caption">
        <button class="cap" title="Minimize to tray" onclick={() => void win.hide()}>&#xE921;</button>
        <button
          class="cap close"
          title="Quit Thunderboard"
          onclick={() =>
            (confirming = {
              title: "Quit Thunderboard?",
              body: "Hotkeys stop working until you launch it again. Minimize instead to keep it running in the tray.",
              label: "Quit",
              act: () => void quit(),
            })}>&#xE8BB;</button
        >
      </div>
    </header>

    <div class="body" class:dragging class:gallery={view === "gallery"}>
      {#each profile.sounds as s, i (s.id)}
        <SoundRow
          bind:sound={profile.sounds[i]}
          {view}
          expanded={expandedId === s.id}
          listening={listeningId === s.id}
          failed={failedHotkeys.includes(s.hotkey)}
          playing={playingId === s.id}
          onToggle={() => (expandedId = expandedId === s.id ? null : s.id)}
          onListen={() => {
            listeningId = s.id;
            settingsListening = null;
          }}
          onDelete={() => removeSound(s.id)}
          onPreview={() => void previewNow(s.id)}
        />
      {/each}

      {#if profile.sounds.length === 0}
        <div class="empty">
          <span class="empty-glyph">&#xE8E5;</span>
          {#if profile.folder}
            {folderMissing ? "That folder cannot be read right now." : "No audio in that folder yet."}
            <span class="folder-path" title={profile.folder}>{profile.folder}</span>
            <div class="empty-actions">
              <button class="ghost" onclick={() => void syncNow()}>Sync now</button>
              <button class="ghost" onclick={() => void pickFolder()}>Change folder</button>
            </div>
          {:else}
            Drop audio files here, or press +
            <div class="empty-actions">
              <button class="ghost" onclick={() => void pickFolder()}>Set to folder</button>
            </div>
            <span class="empty-note">
              Mirrors a folder instead: every clip in it joins the board, and stays in step.
            </span>
          {/if}
        </div>
      {/if}

      {#if dragging}
        <div class="drop-label">Drop to add to {profile.name}</div>
      {/if}
    </div>

    <footer class="footer">
      <span class="brand">
        <!-- Same geometry as icons/thunderboard.svg, as one even-odd path so the
             bolt is a hole and the mark survives any single colour. -->
        <svg class="mark" viewBox="0 0 256 256" aria-hidden="true">
          <path
            fill="currentColor"
            fill-rule="evenodd"
            d="M76 16H180A60 60 0 0 1 240 76V180A60 60 0 0 1 180 240H76A60 60 0 0 1 16 180V76A60 60 0 0 1 76 16ZM108 52H168L146 116H184L94 204L112 144H72Z"
          />
        </svg>
        Thunderboard
      </span>
      {#if !setupComplete}
        <button class="setup" onclick={() => (settingsOpen = true)}>Setup incomplete</button>
      {/if}
    </footer>

    {#if settingsOpen}
      <Settings
        bind:config
        {devices}
        failed={failedHotkeys}
        listening={settingsListening}
        onListen={(field: "pitchHotkey" | "stopHotkey" | "nextProfileHotkey") =>
          (settingsListening = field)}
        onClose={closeSettings}
      />
    {/if}

    {#if confirming}
      <!-- Esc (capture-phase, above) and the Cancel button already cover keyboard dismissal. -->
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="confirm-overlay" onclick={() => (confirming = null)}>
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="confirm-panel" onclick={(e) => e.stopPropagation()}>
          <span class="confirm-title">{confirming.title}</span>
          <p class="confirm-body">{confirming.body}</p>
          <div class="confirm-actions">
            <button class="ghost" onclick={() => (confirming = null)}>Cancel</button>
            <!-- svelte-ignore a11y_autofocus -->
            <button
              class="danger-btn"
              autofocus
              onclick={() => {
                confirming?.act();
                confirming = null;
              }}>{confirming.label}</button
            >
          </div>
        </div>
      </div>
    {/if}
  {/if}
</div>

<style>
  .panel {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: #262626;
    overflow: hidden;
  }

  .caution-strip {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 3px;
    background: var(--caution);
    pointer-events: none;
  }

  .icon,
  .chev,
  .flyout-icon,
  .empty-glyph {
    font-family: "Segoe Fluent Icons", "Segoe MDL2 Assets";
    line-height: 1;
  }

  /* Header */
  .header {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 44px;
    padding: 0 0 0 12px;
    border-bottom: 1px solid var(--stroke);
  }

  .profile-wrap {
    position: relative;
  }
  .profile-btn {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px;
    border-radius: var(--r-control);
    font-weight: 600;
    font-size: 13px;
  }
  .profile-btn:hover {
    background: var(--hover);
  }
  .chev {
    font-size: 10px;
    color: var(--fg-3);
    transition: transform 120ms ease;
  }
  .chev.open {
    transform: rotate(180deg);
  }

  .pitch-pill {
    height: 24px;
    padding: 0 12px;
    border-radius: 999px;
    border: 1px solid var(--stroke-strong);
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--fg-3);
    transition: background 120ms ease, border-color 120ms ease, color 120ms ease;
  }
  .pitch-pill.on {
    border-color: transparent;
    background: var(--caution);
    color: #332d00;
  }

  .header-actions {
    display: flex;
    align-items: center;
    gap: 2px;
    margin-left: auto;
  }
  .header-actions .icon {
    width: 26px;
    height: 26px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--r-control);
    color: var(--fg-3);
    font-size: 13px;
    transition: background 120ms ease, color 120ms ease;
  }
  .header-actions .icon:hover {
    background: var(--hover);
    color: var(--fg);
  }
  .header-actions .icon.caution {
    color: var(--caution);
  }
  .caption {
    display: flex;
    align-self: stretch;
    margin-left: 6px;
  }
  .cap {
    width: 44px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-family: "Segoe Fluent Icons", "Segoe MDL2 Assets";
    font-size: 10px;
    line-height: 1;
    color: var(--fg-2);
    transition: background 120ms ease, color 120ms ease;
  }
  .cap:hover {
    background: var(--hover);
  }
  .cap.close:hover {
    background: #c42b1c;
    color: #fff;
  }

  /* Profile flyout */
  .flyout {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    width: 200px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 4px;
    background: #2c2c2c;
    border: 1px solid var(--stroke-strong);
    border-radius: var(--r-surface);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
    z-index: 10;
  }
  .flyout-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px;
    border-radius: var(--r-control);
    font-size: 12px;
  }
  .flyout-row:hover {
    background: var(--hover);
  }
  .flyout-name {
    flex: 1;
    min-width: 0;
    text-align: left;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .flyout-icon {
    width: 20px;
    height: 20px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--r-control);
    font-size: 12px;
    color: var(--fg-3);
    opacity: 0;
  }
  .flyout-row:hover .flyout-icon {
    opacity: 1;
  }
  .flyout-icon.pinned {
    opacity: 1;
  }
  .flyout-icon:hover {
    background: var(--stroke-strong);
    color: var(--fg);
  }
  .flyout-new {
    color: var(--fg-3);
    border-top: 1px solid var(--stroke);
    border-radius: 0;
    margin-top: 2px;
    padding-top: 8px;
  }
  .rename {
    flex: 1;
    background: var(--press);
    border: 1px solid var(--accent);
    border-radius: var(--r-control);
    outline: none;
    padding: 1px 4px;
    user-select: text;
  }

  /* Body */
  .body {
    position: relative;
    flex: 1;
    overflow-y: auto;
    padding: 6px;
  }
  .body.gallery {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
    align-content: start;
    gap: 6px;
  }
  .body.gallery .empty {
    grid-column: 1 / -1;
    margin: 0;
  }
  .body.dragging {
    outline: 1px dashed var(--accent);
    outline-offset: -1px;
    background: var(--accent-bg);
  }

  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    margin: 12px 4px;
    padding: 24px;
    border: 1px dashed var(--stroke-strong);
    border-radius: var(--r-surface);
    color: var(--fg-3);
    font-size: 12px;
    text-align: center;
  }
  .empty-glyph {
    font-size: 16px;
  }
  .empty-actions {
    display: flex;
    gap: 6px;
    margin-top: 2px;
  }
  .empty-note {
    max-width: 260px;
    color: var(--fg-3);
    opacity: 0.8;
    font-size: 11px;
  }
  .folder-path {
    max-width: 100%;
    color: var(--fg-2);
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl; /* keep the leaf folder visible when the path is long */
  }

  .drop-label {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--fg);
    font-size: 13px;
    font-weight: 600;
    pointer-events: none;
  }

  /* Footer */
  .footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 34px;
    padding: 7px 10px;
    border-top: 1px solid var(--stroke);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 5px;
    color: var(--fg-3);
    font-size: 11px;
  }
  .mark {
    width: 12px;
    height: 12px;
    color: var(--accent);
  }
  .setup {
    color: var(--caution);
    font-size: 11px;
    font-weight: 600;
    padding: 3px 6px;
    border-radius: var(--r-control);
  }
  .setup:hover {
    background: var(--caution-bg);
  }

  /* Confirmation - no transition, matches the app's no-motion-on-overlays rule. */
  .confirm-overlay {
    position: absolute;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 20;
  }
  .confirm-panel {
    background: #2c2c2c;
    border: 1px solid var(--stroke-strong);
    border-radius: var(--r-surface);
    padding: 16px;
    max-width: 300px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .confirm-title {
    font-size: 13px;
    font-weight: 600;
  }
  .confirm-body {
    font-size: 12px;
    color: var(--fg-3);
  }
  .confirm-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .ghost,
  .danger-btn {
    padding: 5px 12px;
    font-size: 12px;
    border-radius: var(--r-control);
    border: none;
  }
  .ghost {
    border: 1px solid var(--stroke);
    background: var(--layer);
  }
  .ghost:hover {
    background: var(--hover);
  }
  .danger-btn {
    background: #c42b1c;
    color: #fff;
    font-weight: 600;
  }
  .danger-btn:hover {
    background: #d13c2d;
  }
</style>
