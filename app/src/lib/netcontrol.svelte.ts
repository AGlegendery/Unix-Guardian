// Internet control's live state, shared by Monitoring's Traffic and Ping
// tabs: the apps the service counts (every 2 s while a page asks), the
// block and limit rules, and the open limit form.

import { api, errorText, type AppTraffic, type NetControlStatus, type NetRule } from "./api";

export type LimitForm = { app: string; up: number; down: number; when: "always" | "gaming" };

export class NetControl {
  apps = $state<AppTraffic[]>([]);
  rules = $state<NetRule[]>([]);
  status = $state<NetControlStatus | null>(null);
  error = $state<string | null>(null);
  busy = $state(false);
  editing = $state<LimitForm | null>(null);

  async refresh() {
    try {
      [this.apps, this.rules, this.status] = await Promise.all([api.netApps(), api.netRules(), api.netStatus()]);
      this.error = null;
    } catch (e) {
      this.error = errorText(e);
    }
  }

  /** Polls every 2 s while visible; call from an $effect and return the result. */
  poll(): () => void {
    const tick = () => !document.hidden && this.refresh();
    tick();
    const id = setInterval(tick, 2000);
    return () => clearInterval(id);
  }

  ruleFor(name: string): NetRule | null {
    return this.rules.find((r) => r.app.toLowerCase() === name.toLowerCase()) ?? null;
  }

  /** "blocked", "limited" or "" for an app by name (its rule, or the live state). */
  stateOf(name: string): "" | "blocked" | "limited" {
    const live = this.apps.find((a) => a.name.toLowerCase() === name.toLowerCase());
    if (live?.state) return live.state;
    const r = this.ruleFor(name);
    return r ? (r.block ? "blocked" : "limited") : "";
  }

  async act(f: () => Promise<unknown>) {
    this.busy = true;
    try {
      await f();
      await this.refresh();
    } catch (e) {
      this.error = errorText(e);
    } finally {
      this.busy = false;
    }
  }

  block = (name: string) => this.act(() => api.netSetRule(name, true, 0, 0, "always"));
  allow = (name: string) => this.act(() => api.netRemoveRule(name));

  openLimit(name: string) {
    const r = this.ruleFor(name);
    this.editing = { app: name, up: r?.up_kbps || 1000, down: r?.down_kbps || 8000, when: r?.when ?? "gaming" };
  }

  async saveLimit() {
    const e = this.editing;
    if (!e) return;
    await this.act(() => api.netSetRule(e.app, false, e.up, e.down, e.when));
    if (!this.error) this.editing = null;
  }
}
