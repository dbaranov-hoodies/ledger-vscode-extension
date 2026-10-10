<script lang="ts">
  import type { Snippet } from "svelte";
  import type { Action } from "../actions";
  import PinButton from "./PinButton.svelte";
  import StatusIcon from "./StatusIcon.svelte";

  interface Props {
    action: Action;
    disabled: boolean;
    onExecute: (actionId: string) => void;
    onTogglePin: (actionId: string) => void;
    suffix?: Snippet<[Action]>;
  }

  let { action, disabled, onExecute, onTogglePin, suffix }: Props = $props();
</script>

<div class="row">
  <button
    class="button"
    data-status={action.status}
    title={action.status === "running" ? "Click to stop" : action.tooltip}
    disabled={disabled && action.status !== "running"}
    onclick={() => onExecute(action.id)}
  >
    <span class="icon"><i class="codicon codicon-{action.icon}"></i></span>
    <span class="label">{action.label}</span>
    <StatusIcon status={action.status} small />
  </button>
  <PinButton
    pinned={action.pinned}
    concealed={action.status !== "idle"}
    onclick={() => onTogglePin(action.id)}
  />
  {@render suffix?.(action)}
</div>

<style>
  .row {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .row:hover {
    --pin-opacity: 1;
  }

  .button {
    display: flex;
    flex: 1;
    align-items: center;
    gap: 5px;
    padding: 3px 6px;
    border: none;
    border-radius: 2px;
    background-color: transparent;
    color: var(--vscode-foreground);
    text-align: left;
    cursor: pointer;
    transition: all 0.1s;
  }

  .button:hover:not(:disabled) {
    background-color: var(--vscode-list-hoverBackground);
  }

  .button:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .button[data-status="running"] {
    background-color: var(--vscode-list-activeSelectionBackground);
  }

  .icon {
    font-size: 13px;
    color: var(--vscode-descriptionForeground);
  }

  .label {
    flex: 1;
    font-size: 12px;
  }
</style>
