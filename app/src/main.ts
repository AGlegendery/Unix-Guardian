import "@fontsource-variable/vazirmatn";
import "./app.css";
import { mount } from "svelte";
import App from "./App.svelte";
import { applyDocumentLocale } from "./lib/i18n.svelte";
import { applyMotion } from "./lib/motion.svelte";

applyDocumentLocale();
applyMotion();
mount(App, { target: document.getElementById("app")! });
