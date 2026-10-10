<script lang="ts">
  interface Props {
    pinned?: boolean;
    concealed?: boolean;
    onclick: () => void;
  }

  let { pinned = false, concealed = false, onclick }: Props = $props();
</script>

<button
  class="pin"
  class:concealed
  title={pinned ? "Unpin from toolbar" : "Pin to toolbar"}
  {onclick}
>
  <i class="codicon codicon-{pinned ? 'pinned' : 'pin'}"></i>
</button>

<style>
  /* A parent row reveals the pin by setting --pin-opacity, and may resize it with --pin-size. */
  .pin {
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    width: var(--pin-size, 18px);
    height: var(--pin-size, 18px);
    border: none;
    border-radius: 3px;
    background: transparent;
    color: var(--vscode-descriptionForeground);
    cursor: pointer;
    opacity: var(--pin-opacity, 0);
    transition:
      opacity 0.15s,
      background 0.15s,
      color 0.15s;
  }

  .pin:hover {
    background: var(--vscode-toolbar-hoverBackground);
    color: var(--vscode-foreground);
  }

  .pin.concealed {
    opacity: 0;
    pointer-events: none;
  }

  .pin .codicon {
    font-size: 12px;
  }
</style>
