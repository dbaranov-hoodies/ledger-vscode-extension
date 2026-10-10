<script lang="ts">
  import { Check, X } from "@jis3r/icons";
  import type { CommandStatus } from "../actions";

  interface Props {
    status: CommandStatus;
    small?: boolean;
  }

  let { status, small = false }: Props = $props();

  const size = $derived(small ? 12 : 16);
</script>

{#if status !== "idle"}
  <span class="status" class:small data-status={status}>
    {#if status === "running"}
      <span class="spinner"></span>
      <i class="codicon codicon-debug-stop stop" style="font-size: {size}px"></i>
    {:else if status === "success"}
      <Check {size} animate />
    {:else}
      <X {size} animate />
    {/if}
  </span>
{/if}

<style>
  .status {
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .status[data-status="running"] {
    color: var(--vscode-progressBar-background);
  }

  .status[data-status="success"] {
    color: var(--vscode-testing-iconPassed);
  }

  .status[data-status="error"] {
    color: var(--vscode-testing-iconFailed);
  }

  .spinner {
    width: 14px;
    height: 14px;
    border: 2px solid currentColor;
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  /* Pointing at a running action shows that clicking stops it. */
  .stop {
    display: none;
  }

  :global(.button:hover) .stop {
    display: inline-block;
  }

  :global(.button:hover) .spinner {
    display: none;
  }

  .small .spinner {
    width: 10px;
    height: 10px;
    border-width: 1.5px;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
