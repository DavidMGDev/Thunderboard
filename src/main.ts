import { mount } from "svelte";
import App from "./App.svelte";
import { loadLayoutMap } from "./lib/keys";
import "./app.css";

// Before mounting, so the first hotkey chip already reads the right keycap.
await loadLayoutMap();

export default mount(App, { target: document.getElementById("app")! });
