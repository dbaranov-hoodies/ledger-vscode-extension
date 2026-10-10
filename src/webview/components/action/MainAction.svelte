<script lang="ts">
  import type { Action } from "../actions";
  import PinButton from "./PinButton.svelte";
  import StatusIcon from "./StatusIcon.svelte";

  interface Props {
    title: string;
    icon: string;
    action?: Action;
    disabled: boolean;
    onExecute: (actionId: string) => void;
    onTogglePin: (actionId: string) => void;
  }

  let { title, icon, action, disabled, onExecute, onTogglePin }: Props = $props();

  const status = $derived(action?.status ?? "idle");

  function execute() {
    if (action) onExecute(action.id);
  }

  function togglePin() {
    if (action) onTogglePin(action.id);
  }
</script>

<div class="main-action">
  <button
    class="button"
    data-status={status}
    title={action?.tooltip}
    disabled={!action || disabled || status === "running"}
    onclick={execute}
  >
    <span class="icon"><i class="codicon codicon-{icon}"></i></span>
    <span class="label">
      <span class="title">{title}</span>
      <span class="subtitle">{action?.label ?? "Not available"}</span>
    </span>
    <StatusIcon {status} />
  </button>
  {#if action}
    <div class="pin-slot">
      <PinButton pinned={action.pinned} concealed={status !== "idle"} onclick={togglePin} />
    </div>
  {/if}
</div>

<style>
  .main-action {
    position: relative;
    display: flex;
    flex: 1;
  }

  .main-action:hover {
    --pin-opacity: 1;
  }

  .button {
    display: flex;
    flex: 1;
    align-items: center;
    gap: 6px;
    padding: 5px 8px;
    border: 1px solid var(--vscode-input-border);
    border-radius: 3px 0 0 3px;
    background-color: transparent;
    color: var(--vscode-foreground);
    cursor: pointer;
    transition: all 0.15s;
  }

  .button:hover:not(:disabled) {
    background-color: var(--vscode-list-hoverBackground);
  }

  .button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .button[data-status="running"] {
    border-color: var(--vscode-focusBorder);
    background-color: var(--vscode-list-activeSelectionBackground);
  }

  .button[data-status="success"] {
    border-color: var(--vscode-testing-iconPassed);
  }

  .button[data-status="error"] {
    border-color: var(--vscode-testing-iconFailed);
  }

  .icon {
    width: 16px;
    font-size: 14px;
    text-align: center;
    color: var(--vscode-descriptionForeground);
  }

  .label {
    display: flex;
    flex: 1;
    flex-direction: column;
    text-align: left;
  }

  .title {
    font-size: 12px;
    font-weight: 500;
  }

  .subtitle {
    font-size: 11px;
    color: var(--vscode-descriptionForeground);
  }

  .pin-slot {
    --pin-size: 20px;
    position: absolute;
    top: 50%;
    right: 4px;
    transform: translateY(-50%);
  }
</style>
