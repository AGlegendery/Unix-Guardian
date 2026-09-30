import "@fontsource-variable/vazirmatn";
import "./app.css";
import { mount } from "svelte";
import App from "./App.svelte";
import { applyDocumentLocale } from "./lib/i18n.svelte";

applyDocumentLocale();
mount(App, { target: document.getElementById("app")! });
