import { mount } from "svelte";
import Webview from "./Webview.svelte";
import Shell from "./Shell.svelte";

document.addEventListener("DOMContentLoaded", () => {
  // The extension says which backend it is using: the new panel renders snapshots of the sidecar.
  mount(window.uiMode === "snapshot" ? Shell : Webview, {
    target: document.body,
    props: {
      vscode: acquireVsCodeApi(),
    },
  });
});
