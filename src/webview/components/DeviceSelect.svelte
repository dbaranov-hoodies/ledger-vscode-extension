<script lang="ts">
  import { Popover } from "bits-ui";
  import { ChevronDown } from "@jis3r/icons";
  import type { DeviceView } from "../../generated/DeviceView";

  interface Props {
    devices: DeviceView[];
    /** The ids of the selected devices. */
    selected: string[];
    /** What to show for the selection: `All`, or the names of the devices. */
    label: string;
    onchange: (ids: string[]) => void;
    icon?: string;
    groupLabel?: string;
  }

  let { devices, selected, label, onchange, icon, groupLabel }: Props = $props();

  const allSelected = $derived(devices.length > 0 && devices.every((d) => selected.includes(d.id)));

  // At least one device stays selected: there would be nothing to run for.
  function toggle(id: string) {
    const next = selected.includes(id) ? selected.filter((s) => s !== id) : [...selected, id];
    if (next.length > 0) onchange(next);
  }

  function toggleAll() {
    onchange(allSelected ? [devices[0].id] : devices.map((d) => d.id));
  }
</script>

<Popover.Root>
  <Popover.Trigger class="select-trigger" title="Select one or several devices">
    {#if icon}
      <span class="trigger-icon">
        <i class="codicon codicon-{icon}"></i>
      </span>
    {/if}
    <span class="select-value">{label}</span>
    <span class="select-icon">
      <ChevronDown size={14} />
    </span>
  </Popover.Trigger>
  <Popover.Portal>
    <Popover.Content
      class="select-content device-select-content"
      side="bottom"
      align="start"
      sideOffset={4}
    >
      <div class="select-viewport">
        {#if groupLabel}
          <div class="select-group-label">{groupLabel}</div>
        {/if}
        {#if devices.length > 1}
          <label class="select-item">
            <input
              type="checkbox"
              checked={allSelected}
              onclick={(event) => {
                event.preventDefault();
                toggleAll();
              }}
            />
            <span class="select-item-label">All</span>
          </label>
        {/if}
        {#each devices as device (device.id)}
          <label class="select-item">
            <input
              type="checkbox"
              checked={selected.includes(device.id)}
              onclick={(event) => {
                event.preventDefault();
                toggle(device.id);
              }}
            />
            <span class="select-item-label">{device.name}</span>
          </label>
        {/each}
      </div>
    </Popover.Content>
  </Popover.Portal>
</Popover.Root>

<style>
  .trigger-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--vscode-descriptionForeground);
    font-size: 14px;
    flex-shrink: 0;
  }

  .select-value {
    flex: 1;
    text-align: left;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .select-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--vscode-descriptionForeground);
    flex-shrink: 0;
  }

  :global(.device-select-content) {
    min-width: 170px;
  }

  :global(.device-select-content .select-item input[type="checkbox"]) {
    width: 12px;
    height: 12px;
    margin: 0;
    accent-color: var(--vscode-focusBorder);
    flex-shrink: 0;
  }
</style>
