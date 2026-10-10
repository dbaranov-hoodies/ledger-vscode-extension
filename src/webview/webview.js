import { mount } from "svelte";
import Shell from "./Shell.svelte";

document.addEventListener("DOMContentLoaded", () => {
  mount(Shell, {
    target: document.body,
    props: {
      vscode: acquireVsCodeApi(),
    },
  });
});
