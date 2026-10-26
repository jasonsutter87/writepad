/// <reference types="vite/client" />
// In a normal browser (npm run dev, no Tauri), send the app's commands to
// crates/devserver over HTTP, which Vite proxies at /api. That's how
// auto-documentor and screenshots see the app with the demo vault. Plugin
// calls (dialogs, events, updates) get the answer a cancelled dialog or an
// up-to-date app would give. Inside the desktop app this does nothing, and a
// production build drops it.

type Internals = {
  invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
  transformCallback: (cb: (x: unknown) => void, once?: boolean) => number;
  unregisterCallback: (id: number) => void;
  convertFileSrc: (path: string) => string;
  metadata: { currentWindow: { label: string }; currentWebview: { windowLabel: string; label: string } };
};

const w = window as unknown as { __TAURI_INTERNALS__?: Internals; __TAURI_EVENT_PLUGIN_INTERNALS__?: object };

if (import.meta.env.DEV && !w.__TAURI_INTERNALS__) {
  let nextId = 1;
  const plugin: Record<string, unknown> = {
    "plugin:app|version": "0.0.0-preview",
    "plugin:updater|check": null,
    "plugin:dialog|open": null,
    "plugin:dialog|save": null,
    "plugin:event|listen": 0,
    "plugin:event|unlisten": null,
    "plugin:event|emit": null,
  };
  w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  w.__TAURI_INTERNALS__ = {
    async invoke(cmd, args = {}) {
      if (cmd.startsWith("plugin:")) return plugin[cmd] ?? null;
      const res = await fetch(`/api/${cmd}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(args) });
      const body = await res.json();
      if (!res.ok) throw body;
      return body;
    },
    transformCallback: () => nextId++,
    unregisterCallback: () => {},
    convertFileSrc: (path) => path,
    metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
  };
}
