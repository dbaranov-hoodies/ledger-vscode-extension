<script lang="ts">
  import { Plus } from "@jis3r/icons";

  interface Props {
    expanded: boolean;
    disabled: boolean;
    onclick?: () => void;
  }

  let { expanded, disabled, onclick }: Props = $props();

  let hovered = $state(false);
</script>

<button
  class="toggle"
  class:active={expanded}
  aria-label="Toggle options"
  aria-expanded={expanded}
  {disabled}
  {onclick}
  onmouseenter={() => (hovered = true)}
  onmouseleave={() => (hovered = false)}
>
  <span class="plus" class:rotated={expanded}>
    <Plus size={16} animate={hovered} />
  </span>
</button>

<style>
  .toggle {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    border: 1px solid var(--vscode-input-border);
    border-left: none;
    border-radius: 0 3px 3px 0;
    background-color: transparent;
    color: var(--vscode-descriptionForeground);
    cursor: pointer;
    transition: all 0.15s;
  }

  .toggle:hover:not(:disabled) {
    background-color: var(--vscode-list-hoverBackground);
    color: var(--vscode-foreground);
  }

  .toggle:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .toggle.active {
    background-color: var(--vscode-list-activeSelectionBackground);
    color: var(--vscode-foreground);
  }

  .plus {
    display: flex;
    align-items: center;
    justify-content: center;
    pointer-events: none;
    transition: transform 0.2s ease;
  }

  .plus.rotated {
    transform: rotate(45deg);
  }
</style>
