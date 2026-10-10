<script lang="ts">
  import autoAnimate from "@formkit/auto-animate";
  import { fade } from "svelte/transition";
  import Select, { type SelectItem } from "./Select.svelte";
  import Switch from "./Switch.svelte";
  import type { DevicesView } from "../../generated/DevicesView";
  import type { TestNodeView } from "../../generated/TestNodeView";
  import type { TestsView } from "../../generated/TestsView";

  interface Props {
    tests: TestsView;
    /** The devices of the app, and which of them are selected. */
    devices: DevicesView;
    /** Lists the tests again. */
    refresh: () => void;
    /** Checks or unchecks a node and everything below it. */
    toggle: (id: string) => void;
    /** Chooses these tests, or every test with `null`. */
    choose: (ids: string[] | null) => void;
    /** Chooses the tests that failed on the selected devices. */
    selectFailed: () => void;
    /** Opens the source of a file, class or test in the editor. */
    open: (id: string) => void;
    chooseSuite: (name: string) => void;
    setVerbose: (verbose: boolean) => void;
  }

  let { tests, devices, refresh, toggle, choose, selectFailed, open, chooseSuite, setVerbose }: Props =
    $props();

  let query = $state("");
  let searchOpen = $state(false);
  let searchInput = $state<HTMLInputElement | null>(null);
  let collapsed = $state<Set<string>>(new Set());
  // Names are links to the source while Ctrl (Cmd on a Mac) is held, as in the editor.
  let linkKey = $state(false);

  function trackLinkKey(event: KeyboardEvent) {
    linkKey = event.ctrlKey || event.metaKey;
  }

  function leaves(nodes: TestNodeView[]): TestNodeView[] {
    return nodes.flatMap((node) => (node.kind === "test" ? [node] : leaves(node.children)));
  }

  const allTests = $derived(leaves(tests.tree));
  const checked = $derived(allTests.filter((test) => test.state === "checked"));
  const allChecked = $derived(allTests.length > 0 && checked.length === allTests.length);
  const needle = $derived(query.trim().toLowerCase());
  const selectedDevices = $derived(new Set(devices.selected));
  const failedOnSelected = $derived(allTests.filter(failing));
  const selectedNames = $derived(
    devices.available
      .filter((device) => selectedDevices.has(device.id))
      .map((device) => device.name)
      .join(", "),
  );
  const suites = $derived<SelectItem[]>(tests.suites.map((name) => ({ value: name, label: name })));

  /** The test failed on a device that is selected, so it fails again if run now. */
  function failing(node: TestNodeView): boolean {
    return node.failedOn.some((id) => selectedDevices.has(id));
  }

  /** The devices the test failed on, in the order of the devices of the app. */
  function failedDevices(node: TestNodeView) {
    return devices.available
      .filter((device) => node.failedOn.includes(device.id))
      .map((device) => ({ ...device, selected: selectedDevices.has(device.id) }));
  }

  function matches(node: TestNodeView): boolean {
    if (!needle) return true;
    if (node.kind === "test") return node.name.toLowerCase().includes(needle);
    return node.children.some(matches);
  }

  const shown = $derived(leaves(tests.tree).filter(matches));
  const allShownChecked = $derived(
    shown.length > 0 && shown.every((test) => test.state === "checked"),
  );
  const allCollapsed = $derived(
    tests.tree.length > 0 &&
      tests.tree.every((node) => node.children.length === 0 || collapsed.has(node.id)),
  );

  function toggleSearch() {
    searchOpen = !searchOpen;
    if (!searchOpen) query = "";
    else setTimeout(() => searchInput?.focus(), 0);
  }

  function toggleCollapsed(id: string) {
    const next = new Set(collapsed);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    collapsed = next;
  }

  function containers(nodes: TestNodeView[]): string[] {
    return nodes.flatMap((node) =>
      node.children.length > 0 ? [node.id, ...containers(node.children)] : [],
    );
  }

  function toggleCollapseAll() {
    collapsed = allCollapsed ? new Set() : new Set(containers(tests.tree));
  }

  function chooseShown(select: boolean) {
    const shownIds = new Set(shown.map((test) => test.id));
    const current = checked.map((test) => test.id);
    const next = select
      ? [...new Set([...current, ...shownIds])]
      : current.filter((id) => !shownIds.has(id));
    choose(next);
  }

  function icon(kind: TestNodeView["kind"]): string {
    switch (kind) {
      case "directory":
        return "folder";
      case "file":
        return "file";
      case "class":
        return "symbol-class";
      case "test":
        return "symbol-method";
    }
  }
</script>

<svelte:window onkeydown={trackLinkKey} onkeyup={trackLinkKey} onblur={() => (linkKey = false)} />

{#snippet row(node: TestNodeView, depth: number)}
  {#if matches(node)}
    <div class="tree-row" style:padding-left="{depth * 12 + 4}px" title={node.id}>
      {#if node.children.length > 0}
        <button
          class="twisty"
          onclick={() => toggleCollapsed(node.id)}
          aria-label="Expand or collapse"
        >
          <i class="codicon codicon-chevron-{!needle && collapsed.has(node.id) ? 'right' : 'down'}"
          ></i>
        </button>
      {:else}
        <span class="twisty"></span>
      {/if}
      <input
        type="checkbox"
        checked={node.state === "checked"}
        indeterminate={node.state === "partial"}
        onclick={(event) => {
          event.preventDefault();
          toggle(node.id);
        }}
      />
      <i class="codicon codicon-{icon(node.kind)} kind-icon"></i>
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
      <span
        class="name {node.kind}"
        class:failing={failing(node)}
        class:link={linkKey && node.kind !== "directory"}
        title={node.kind === "directory" ? node.id : `${node.id}\nCtrl+click to open`}
        onclick={(event) => {
          if (node.kind !== "directory" && (event.ctrlKey || event.metaKey)) open(node.id);
        }}>{node.name}</span
      >
      {#if node.failedOn.length > 0}
        <span class="failed-devices">
          {#each failedDevices(node) as device (device.id)}
            <span
              class="chip"
              class:selected={device.selected}
              title="Failed on {device.name}{device.selected ? '' : ' (not selected)'}"
              >{device.shortName}</span
            >
          {/each}
        </span>
      {/if}
    </div>
    {#if node.children.length > 0 && (needle || !collapsed.has(node.id))}
      {#each node.children as child (child.id)}
        {@render row(child, depth + 1)}
      {/each}
    {/if}
  {/if}
{/snippet}

<div class="test-selector-embedded">
  <div class="test-selector-divider"></div>
  <div class="verbose-toggle-row">
    <span class="verbose-label">
      <i class="codicon codicon-output"></i>
      Verbose output
    </span>
    <Switch checked={tests.verbose} onCheckedChange={setVerbose} />
  </div>
  {#if suites.length > 1}
    <div class="suite-select">
      <Select
        icon="beaker"
        groupLabel="Test suite"
        items={suites}
        value={tests.suite ?? ""}
        onchange={chooseSuite}
      />
    </div>
  {/if}
  <div class="test-selector-header-inline">
    <span class="test-selector-title">
      <i class="codicon codicon-list-selection"></i>
      Tests to run
    </span>
    <div class="test-header-actions">
      {#if !tests.collecting && allTests.length > 0}
        <button
          class="icon-button"
          onclick={toggleCollapseAll}
          title={allCollapsed ? "Expand all" : "Collapse all"}
        >
          <i class="codicon codicon-{allCollapsed ? 'expand-all' : 'collapse-all'}"></i>
        </button>
        <button
          class="icon-button"
          onclick={() => choose(allChecked ? [] : null)}
          title={allChecked ? "Deselect all" : "Select all"}
        >
          <i class="codicon {allChecked ? 'codicon-close-all' : 'codicon-check-all'}"></i>
        </button>
        <button
          class="icon-button {searchOpen ? 'active' : ''}"
          onclick={toggleSearch}
          title="Filter tests"
        >
          <i class="codicon codicon-search"></i>
        </button>
        <button
          class="icon-button"
          class:failures={failedOnSelected.length > 0}
          onclick={selectFailed}
          disabled={failedOnSelected.length === 0}
          title={failedOnSelected.length > 0
            ? `Select the ${failedOnSelected.length} failed test${failedOnSelected.length === 1 ? "" : "s"} of ${selectedNames}`
            : `No failed tests on ${selectedNames}`}
        >
          <i class="codicon codicon-error"></i>
        </button>
      {/if}
      <button class="icon-button" onclick={refresh} title="Refresh test list">
        <i class="codicon codicon-refresh {tests.collecting ? 'spinning' : ''}"></i>
      </button>
      {#if !tests.collecting}
        <span
          transition:fade={{ duration: 150 }}
          class={allTests.length === 0 ? "test-count-no-tests-error" : "test-count"}
          >{checked.length}/{allTests.length}</span
        >
      {/if}
    </div>
  </div>
  <div class="test-search-wrapper" use:autoAnimate>
    {#if searchOpen && !tests.collecting}
      <div class="test-search">
        <div class="search-input-wrapper">
          <input
            type="text"
            class="search-input"
            placeholder="Filter tests..."
            bind:value={query}
            bind:this={searchInput}
            style:padding-right={query ? "62px" : "4px"}
          />
          {#if query}
            <div class="search-input-actions">
              <button
                class="icon-button"
                onclick={() => chooseShown(!allShownChecked)}
                title={allShownChecked ? "Deselect filtered" : "Select filtered"}
              >
                <i class="codicon {allShownChecked ? 'codicon-dash' : 'codicon-check'}"></i>
              </button>
              <button class="icon-button" onclick={() => (query = "")} title="Clear filter">
                <i class="codicon codicon-x"></i>
              </button>
            </div>
          {/if}
        </div>
      </div>
    {/if}
  </div>
  {#if allTests.length === 0 && !tests.collecting}
    <p class="empty">No tests listed yet. Refresh to list them in the container.</p>
  {:else}
    <div class="test-list-compact">
      {#each tests.tree as node (node.id)}
        {@render row(node, 0)}
      {/each}
    </div>
  {/if}
</div>

<style>
  .test-selector-embedded {
    margin-top: 4px;
  }

  .test-selector-divider {
    height: 1px;
    background-color: var(--vscode-panel-border);
    margin: 8px 0;
  }

  .verbose-toggle-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 10px;
    margin-bottom: 4px;
  }

  .verbose-label {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--vscode-foreground);
  }

  .suite-select {
    padding: 2px 10px 4px 10px;
  }

  .test-selector-header-inline {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 4px 10px;
  }

  .test-selector-title {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    color: var(--vscode-descriptionForeground);
    text-transform: uppercase;
    letter-spacing: 0.4px;
  }

  .test-header-actions {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .icon-button {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    padding: 0;
    border: none;
    background: none;
    color: var(--vscode-descriptionForeground);
    cursor: pointer;
    border-radius: 3px;
  }

  .icon-button:hover:not(:disabled),
  .icon-button.active {
    background-color: var(--vscode-list-hoverBackground);
    color: var(--vscode-foreground);
  }

  .icon-button:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .icon-button.failures {
    color: var(--vscode-testing-iconFailed);
  }

  .icon-button .spinning {
    animation: spin 0.8s linear infinite;
  }

  .test-count {
    font-size: 10px;
    background-color: var(--vscode-badge-background);
    color: var(--vscode-badge-foreground);
    padding: 1px 5px;
    border-radius: 8px;
  }

  .test-count-no-tests-error {
    font-size: 10px;
    background-color: var(--vscode-errorForeground);
    color: var(--vscode-editor-background);
    padding: 1px 5px;
    border-radius: 8px;
  }

  .test-search {
    padding: 4px 10px;
  }

  .search-input-wrapper {
    position: relative;
    display: flex;
    align-items: center;
  }

  .search-input {
    width: 100%;
    background: var(--vscode-input-background);
    color: var(--vscode-input-foreground);
    border: 1px solid var(--vscode-input-border, transparent);
    border-radius: 2px;
    padding: 2px 4px;
    font-size: 11px;
    outline: none;
    box-sizing: border-box;
  }

  .search-input:focus {
    border-color: var(--vscode-focusBorder);
  }

  .search-input::placeholder {
    color: var(--vscode-input-placeholderForeground);
  }

  .search-input-actions {
    position: absolute;
    right: 2px;
    top: 50%;
    transform: translateY(-50%);
    display: flex;
    align-items: center;
    gap: 2px;
  }

  .empty {
    margin: 6px 10px;
    font-size: 11px;
    color: var(--vscode-descriptionForeground);
  }

  .test-list-compact {
    display: flex;
    flex-direction: column;
    max-height: 280px;
    overflow-y: auto;
    padding: 0 4px;
  }

  .tree-row {
    display: flex;
    align-items: center;
    gap: 4px;
    padding-top: 2px;
    padding-bottom: 2px;
    padding-right: 6px;
    border-radius: 2px;
    font-size: 11px;
    white-space: nowrap;
  }

  .tree-row:hover {
    background-color: var(--vscode-list-hoverBackground);
  }

  .twisty {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: 14px;
    padding: 0;
    border: none;
    background: none;
    color: var(--vscode-descriptionForeground);
    font-size: 10px;
    cursor: pointer;
    flex-shrink: 0;
  }

  .tree-row input[type="checkbox"] {
    width: 12px;
    height: 12px;
    margin: 0;
    accent-color: var(--vscode-focusBorder);
    flex-shrink: 0;
  }

  .kind-icon {
    font-size: 11px;
    color: var(--vscode-descriptionForeground);
    flex-shrink: 0;
  }

  .name {
    font-family: var(--vscode-editor-font-family);
    color: var(--vscode-foreground);
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .name.failing {
    color: var(--vscode-testing-iconFailed);
  }

  .name.link {
    cursor: pointer;
  }

  .name.link:hover {
    text-decoration: underline;
  }

  .failed-devices {
    display: flex;
    gap: 2px;
    flex-shrink: 0;
  }

  /* A device the test failed on, dimmed when the device is not selected. */
  .chip {
    padding: 0 3px;
    border: 1px solid color-mix(in srgb, var(--vscode-testing-iconFailed) 45%, transparent);
    border-radius: 3px;
    color: color-mix(in srgb, var(--vscode-testing-iconFailed) 70%, transparent);
    font-size: 9px;
    font-weight: 600;
    line-height: 12px;
  }

  .chip.selected {
    background-color: var(--vscode-testing-iconFailed);
    border-color: var(--vscode-testing-iconFailed);
    color: var(--vscode-editor-background);
  }

  .name.directory,
  .name.file,
  .name.class {
    color: var(--vscode-descriptionForeground);
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
