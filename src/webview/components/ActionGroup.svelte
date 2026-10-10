<script lang="ts">
  import autoAnimate from "@formkit/auto-animate";
  import type { Snippet } from "svelte";
  import { isActionDisabled, type Action, type ActionGroupData } from "./actions";
  import ExpandToggle from "./action/ExpandToggle.svelte";
  import MainAction from "./action/MainAction.svelte";
  import OptionRow from "./action/OptionRow.svelte";

  interface Props {
    group: ActionGroupData;
    disabled: boolean;
    allDevices: boolean;
    onExecute: (actionId: string) => void;
    onTogglePin: (actionId: string) => void;
    onToggleExpand?: () => void;
    optionSuffix?: Snippet<[Action]>;
    headerSuffix?: Snippet;
    children?: Snippet;
  }

  let {
    group,
    disabled,
    allDevices,
    onExecute,
    onTogglePin,
    onToggleExpand,
    optionSuffix,
    headerSuffix,
    children,
  }: Props = $props();

  const expanded = $derived(!!group.expanded && !disabled);
</script>

<div class="action-group" class:disabled use:autoAnimate>
  <div class="header">
    <MainAction
      title={group.id}
      icon={group.icon}
      action={group.mainAction}
      {disabled}
      {onExecute}
      {onTogglePin}
    />
    <ExpandToggle {expanded} {disabled} onclick={onToggleExpand} />
  </div>

  {@render headerSuffix?.()}

  {#if expanded}
    <div class="options">
      {#each group.options as option (option.id)}
        <OptionRow
          action={option}
          disabled={isActionDisabled(option, allDevices)}
          {onExecute}
          {onTogglePin}
          suffix={optionSuffix}
        />
      {/each}
      {@render children?.()}
    </div>
  {/if}
</div>

<style>
  .action-group {
    display: flex;
    flex-direction: column;
    transition: opacity 0.2s;
  }

  .action-group.disabled {
    opacity: 0.4;
    pointer-events: none;
  }

  .header {
    display: flex;
  }

  .options {
    display: flex;
    flex-direction: column;
    margin-top: 1px;
    padding: 3px;
    background-color: var(--vscode-sideBar-background);
    border: 1px solid var(--vscode-panel-border);
    border-radius: 3px;
  }
</style>
