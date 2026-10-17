import "@fontsource-variable/vazirmatn";
import "./app.css";
import { mount } from "svelte";
import App from "./App.svelte";
import { applyDocumentLocale } from "./lib/i18n.svelte";
import { applyMotion } from "./lib/motion.svelte";
import { blockZoom, watchScrolling } from "./lib/viewport";

applyDocumentLocale();
applyMotion();
watchScrolling();
blockZoom();
mount(App, { target: document.getElementById("app")! });
