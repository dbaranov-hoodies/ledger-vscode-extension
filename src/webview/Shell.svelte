<script lang="ts">
  import "@vscode-elements/elements/dist/vscode-icon/index.js";
  import autoAnimate from "@formkit/auto-animate";
  import { Popover } from "bits-ui";
  import Select, { type SelectItem } from "./components/Select.svelte";
  import DeviceSelect from "./components/DeviceSelect.svelte";
  import TestTree from "./components/TestTree.svelte";
  import StatusDot from "./components/StatusDot.svelte";
  import ActionGroup from "./components/ActionGroup.svelte";
  import Toolbar from "./components/Toolbar.svelte";
  import type { Action, ActionGroupData, CommandStatus } from "./components/actions";
  import { DevImageStatus } from "../types";
  import type { ActionView } from "../generated/ActionView";
  import type { DisabledReason } from "../generated/DisabledReason";
  import type { Snapshot } from "../generated/Snapshot";

  /**
   * The panel when the sidecar does the work: it renders the last snapshot and sends back what the
   * user does. What it keeps is only how it looks: what is pinned and expanded.
   */
  let { vscode } = $props();

  let snapshot = $state<Snapshot | null>(null);
  let pinned = $state<string[]>([]);
  let expanded = $state<string[]>([]);
  // The result of an action, shown for a few seconds once it is over.
  let results = $state<Record<string, CommandStatus>>({});

  const app = $derived(snapshot?.app ?? null);
  const apps = $derived<SelectItem[]>(
    (snapshot?.apps ?? []).map((summary) => ({ value: summary.folder, label: summary.name })),
  );
  const buildUseCases = $derived<SelectItem[]>(
    (app?.buildUseCases.items ?? []).map((name) => ({ value: name, label: name })),
  );
  const variants = $derived<SelectItem[]>(
    (app?.variants?.items ?? []).map((name) => ({ value: name, label: name })),
  );
  const aiModels = $derived<SelectItem[]>(
    (app?.aiModels.items ?? []).map((name) => ({ value: name, label: name })),
  );
  const enforcerChecks = $derived<SelectItem[]>(
    (app?.enforcerChecks.items ?? []).map((name) => ({ value: name, label: name })),
  );

  const reasons: Record<DisabledReason, string> = {
    severalDevices: "Select a single device",
    deviceActionsOff: "Actions on a Nano X are turned off in the settings",
    noComposeFile: "The app has no docker-compose.yml",
    noTestSuite: "The app has no test suite",
    noTestSelected: "Select at least one test",
    noAiModel: "No AI model is available",
  };

  function toAction(view: ActionView): Action {
    return {
      id: view.id,
      label: view.label,
      icon: view.icon,
      taskName: view.id,
      tooltip: view.disabledReason
        ? `${view.tooltip} (${reasons[view.disabledReason]})`
        : view.tooltip,
      status: results[view.id] ?? (view.running ? "running" : "idle"),
      disabled: !view.enabled,
      pinned: pinned.includes(view.id),
    };
  }

  const actionGroups = $derived<ActionGroupData[]>(
    (app?.groups ?? []).map((group) => ({
      id: group.group,
      icon: group.icon,
      mainAction: group.main ? toAction(group.main) : undefined,
      options: group.options.map(toAction),
      expanded: expanded.includes(group.group),
    })),
  );

  // A group is greyed out when its main action cannot run. With no test chosen it stays open:
  // the tests are chosen in it, and only the button that runs them is disabled.
  function isGroupDisabled(group: ActionGroupData): boolean {
    const main = app?.groups.find((view) => view.group === group.id)?.main;
    return !!main && !main.enabled && main.disabledReason !== "noTestSelected";
  }

  function request(method: string, params: object = {}) {
    vscode.postMessage({ type: "request", method, params });
  }

  function command(name: string, args: unknown[] = []) {
    vscode.postMessage({ type: "command", command: name, args });
  }

  function saveUiState() {
    vscode.postMessage({
      type: "saveUiState",
      pinned: [...pinned],
      expanded: [...expanded],
    });
  }

  // Clicking an action that runs stops it.
  function run(actionId: string) {
    const running = (app?.groups ?? [])
      .flatMap((group) => [group.main, ...group.options])
      .some((action) => action?.id === actionId && action.running);
    request(running ? "cancelAction" : "runAction", { id: actionId });
  }

  function togglePin(actionId: string) {
    pinned = pinned.includes(actionId)
      ? pinned.filter((id) => id !== actionId)
      : [...pinned, actionId];
    saveUiState();
  }

  function toggleExpand(groupId: string) {
    expanded = expanded.includes(groupId)
      ? expanded.filter((id) => id !== groupId)
      : [...expanded, groupId];
    saveUiState();
  }

  // What the user chose to type for the tests of this app, to install what they need.
  let requirementsOpen = $state(false);
  let requirementsInput = $state("");
  $effect(() => {
    requirementsInput = app?.testRequirements ?? "";
  });

  function saveRequirements() {
    request("setTestRequirements", { text: requirementsInput });
    requirementsOpen = false;
  }

  // The status dot asks for the update the way the former panel did.
  const statusDotApi = {
    postMessage(message: { command?: string; taskName?: string }) {
      if (message.command === "executeTask" && message.taskName) {
        run(message.taskName);
      }
    },
  };

  window.addEventListener("message", (event) => {
    const message = event.data;
    switch (message.type) {
      case "snapshot":
        snapshot = message.snapshot;
        break;
      case "uiState":
        pinned = message.pinned ?? [];
        expanded = message.expanded ?? [];
        break;
      case "actionFinished":
        results[message.id] = message.success ? "success" : "error";
        setTimeout(() => delete results[message.id], 5000);
        break;
    }
  });

  // Tells the extension that the panel can receive the snapshot.
  function notifyReady() {
    vscode.postMessage({ type: "ready" });
  }
  notifyReady();
</script>

<div class="container">
  {#if snapshot && !snapshot.docker.running}
    <div class="welcome-view">
      <i class="codicon codicon-error welcome-icon error"></i>
      <p class="welcome-title">Docker is not running</p>
      <p class="welcome-description">
        Please install and start Docker to use the extension, then check again.
      </p>
      <div class="welcome-buttons">
        <a
          class="vscode-button"
          href="https://docs.docker.com/get-docker/"
          target="_blank"
          rel="noopener noreferrer"
        >
          Get Docker
        </a>
        <button
          class="vscode-button secondary"
          onclick={() => request("refresh", { folders: null })}
        >
          Check Again
        </button>
      </div>
      <img class="wordmark" src={(window as any).resourceUris?.wordmark} alt="Ledger" />
    </div>
  {:else if snapshot && !app}
    <div class="welcome-view">
      <i class="codicon codicon-warning welcome-icon"></i>
      <p class="welcome-title">No Ledger app detected</p>
      <p class="welcome-description">
        Open a Ledger app's folder or open a workspace containing Ledger apps to get started.
      </p>
      <div class="welcome-buttons">
        <button class="vscode-button" onclick={() => command("vscode.openFolder")}>
          Open App Folder
        </button>
        <button
          class="vscode-button secondary"
          onclick={() => command("workbench.action.openWorkspace")}
        >
          Open Workspace
        </button>
      </div>
      <img class="wordmark" src={(window as any).resourceUris?.wordmark} alt="Ledger" />
    </div>
  {:else if snapshot && app}
    <div class="main-content">
      <Toolbar
        {actionGroups}
        allDevices={false}
        onExecute={(_groupId: string, actionId: string) => run(actionId)}
        {isGroupDisabled}
      />
      <div class="header-section">
        <div class="config-card">
          <div class="config-header">
            <i class="codicon codicon-settings-gear"></i>
            <h2 class="config-title">Configuration</h2>
            <StatusDot
              vscode={statusDotApi}
              status={snapshot.docker.status as DevImageStatus}
              imageOutdated={snapshot.docker.imageOutdated}
            />
          </div>
          <div class="config-grid">
            <div class="form-group">
              <Select
                icon="file-code"
                groupLabel="Application"
                items={apps}
                value={app.folder}
                onchange={(folder: string) => request("selectApp", { folder })}
              />
            </div>
            <div class="form-group">
              <DeviceSelect
                icon="device-mobile"
                groupLabel="Device"
                devices={app.devices.available}
                selected={app.devices.selected}
                label={app.devices.label}
                onchange={(devices: string[]) => request("setDevices", { devices })}
              />
            </div>
            {#if buildUseCases.length > 1}
              <div class="form-group">
                <Select
                  icon="package"
                  groupLabel="Build Mode"
                  items={buildUseCases}
                  value={app.buildUseCases.selected ?? ""}
                  onchange={(name: string) => request("setBuildUseCase", { name })}
                />
              </div>
            {/if}
            {#if variants.length > 1}
              <div class="form-group">
                <Select
                  icon="symbol-misc"
                  groupLabel="Variant"
                  items={variants}
                  value={app.variants?.selected ?? ""}
                  onchange={(value: string) => request("setVariant", { value })}
                />
              </div>
            {/if}
          </div>
          <div class="hint-wrapper" use:autoAnimate>
            {#if app.notices.includes("severalDevices")}
              <div class="toggle-hint">
                <i class="codicon codicon-info"></i>
                Some actions will be disabled
              </div>
            {:else if app.notices.includes("deviceActionsUnsupported")}
              <div class="toggle-hint warning">
                <i class="codicon codicon-warning"></i>
                Device operations not supported
              </div>
            {/if}
          </div>
        </div>
      </div>

      <div class="actions-section">
        {#each actionGroups as group (group.id)}
          <ActionGroup
            {group}
            disabled={isGroupDisabled(group)}
            allDevices={false}
            onExecute={run}
            onTogglePin={togglePin}
            onToggleExpand={() => toggleExpand(group.id)}
          >
            {#if group.id === "Tests"}
              <div class="option-row">
                <Popover.Root bind:open={requirementsOpen}>
                  <Popover.Trigger openOnHover openDelay={1000} class="option-button">
                    <span class="option-icon">
                      <i class="codicon codicon-type-hierarchy"></i>
                    </span>
                    <span class="option-label">Add Test Dependencies</span>
                  </Popover.Trigger>
                  <Popover.Portal>
                    <Popover.Content class="deps-popover" side="top" sideOffset={4}>
                      <div class="deps-popover-content">
                        <label class="deps-label" for="deps-input">
                          Additional test dependencies
                        </label>
                        <div class="dep-warning-row">
                          <i class="codicon codicon-warning"></i>
                          <button
                            class="dep-warning-link"
                            onclick={() =>
                              command("workbench.action.openSettings", [
                                "ledgerDevTools.openContainerAsRoot",
                              ])}
                          >
                            Enable root on container
                          </button>
                        </div>
                        <input
                          id="deps-input"
                          type="text"
                          class="deps-input"
                          bind:value={requirementsInput}
                          placeholder="e.g. apt install package_name"
                        />
                        <button class="deps-save-button" onclick={saveRequirements}>
                          Save
                        </button>
                      </div>
                    </Popover.Content>
                  </Popover.Portal>
                </Popover.Root>
              </div>
              {#if app.tests.suite}
                <TestTree
                  tests={app.tests}
                  devices={app.devices}
                  refresh={() => request("collectTests")}
                  toggle={(id: string) => request("toggleTest", { id })}
                  choose={(ids: string[] | null) => request("setTests", { ids })}
                  selectFailed={() => request("selectFailedTests")}
                  open={(id: string) => request("openTest", { id })}
                  chooseSuite={(name: string) => request("setTestSuite", { name })}
                  setVerbose={(value: boolean) => request("setVerboseTests", { value })}
                />
              {/if}
            {/if}
            {#snippet optionSuffix(option)}
              {#if option.id === "Enforcer Checks"}
                <div class="check-select-wrapper">
                  <Select
                    items={enforcerChecks}
                    value={app.enforcerChecks.selected ?? ""}
                    placeholder="All"
                    onchange={(name: string) => request("setEnforcerCheck", { name })}
                  />
                </div>
              {:else if option.id === "AI Review" && aiModels.length > 1}
                <div class="check-select-wrapper">
                  <Select
                    items={aiModels}
                    value={app.aiModels.selected ?? ""}
                    placeholder="Select AI Model"
                    onchange={(name: string) => request("setAiModel", { name })}
                  />
                </div>
              {/if}
            {/snippet}
          </ActionGroup>
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
  .welcome-view {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    text-align: center;
    padding: 20px;
    border: 1px solid var(--vscode-panel-border);
    border-radius: 8px;
    background: linear-gradient(
      to top,
      #000000 0%,
      var(--vscode-sideBar-background) 35%,
      var(--vscode-sideBar-background) 100%
    );
    color: var(--vscode-foreground);
  }

  .welcome-icon {
    font-size: 32px;
    color: var(--vscode-editorWarning-foreground);
    margin-bottom: 12px;
  }

  .welcome-icon.error {
    color: var(--vscode-editorError-foreground);
  }

  .welcome-title {
    font-size: var(--vscode-font-size);
    font-weight: 600;
    color: var(--vscode-foreground);
    margin: 0 0 8px 0;
  }

  .welcome-description {
    font-size: var(--vscode-font-size);
    color: var(--vscode-descriptionForeground);
    margin: 0 0 16px 0;
    line-height: 1.4;
  }

  .welcome-buttons {
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: 100%;
  }

  .vscode-button {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 4px 14px;
    box-sizing: border-box;
    text-decoration: none;
    border: none;
    border-radius: 2px;
    font-size: var(--vscode-font-size);
    font-family: var(--vscode-font-family);
    cursor: pointer;
    background-color: var(--vscode-button-background);
    color: var(--vscode-button-foreground);
    width: 100%;
  }

  .vscode-button:hover {
    background-color: var(--vscode-button-hoverBackground);
  }

  .vscode-button.secondary {
    background-color: var(--vscode-button-secondaryBackground);
    color: var(--vscode-button-secondaryForeground);
  }

  .vscode-button.secondary:hover {
    background-color: var(--vscode-button-secondaryHoverBackground);
  }

  .wordmark {
    width: 100%;
    max-width: 100px;
    margin-top: 24px;
  }

  .container {
    color: var(--vscode-editor-foreground);
    font-family: var(--vscode-font-family);
    min-width: 200px;
  }

  .main-content {
    max-width: 600px;
    margin: 0 auto;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .header-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .config-header {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 8px;
    color: var(--vscode-foreground);
  }

  .config-title {
    font-size: 12px;
    font-weight: 600;
    margin: 0;
    flex: 1;
  }

  .config-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(140px, 1fr));
    gap: 8px;
  }

  .form-group {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .toggle-hint {
    margin-top: 8px;
    padding-top: 8px;
    border-top: 1px solid var(--vscode-panel-border);
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: var(--vscode-descriptionForeground);
  }

  .toggle-hint.warning {
    color: var(--vscode-editorWarning-foreground);
  }

  .check-select-wrapper {
    flex: 50%;
    max-width: 200px;
    min-width: 0;
  }

  .dep-warning-row {
    display: flex;
    align-items: center;
    gap: 4px;
    color: var(--vscode-editorWarning-foreground);
    font-size: 11px;
  }

  .dep-warning-link {
    background: none;
    border: none;
    padding: 0;
    color: var(--vscode-editorWarning-foreground);
    font-size: 11px;
    cursor: pointer;
    text-decoration: underline;
  }

  .dep-warning-link:hover {
    opacity: 0.8;
  }

  :global(.option-button) {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    border: none;
    border-radius: 2px;
    background-color: transparent;
    color: var(--vscode-foreground);
    cursor: pointer;
    transition: all 0.1s;
    text-align: left;
    flex: 1;
  }

  :global(.option-button:hover) {
    background-color: var(--vscode-list-hoverBackground);
  }

  .option-icon {
    font-size: 14px;
    color: var(--vscode-descriptionForeground);
  }

  .option-label {
    flex: 1;
    font-size: 12px;
  }

  :global(.deps-popover) {
    background-color: var(--vscode-editorWidget-background);
    border: 1px solid var(--vscode-editorWidget-border);
    border-radius: 4px;
    padding: 12px;
    box-shadow: 0 2px 8px var(--vscode-widget-shadow);
    z-index: 1000;
  }

  :global(.deps-popover-content) {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 220px;
  }

  :global(.deps-label) {
    font-size: 11px;
    color: var(--vscode-descriptionForeground);
  }

  :global(.deps-input) {
    padding: 4px 8px;
    font-size: 12px;
    background-color: var(--vscode-input-background);
    color: var(--vscode-input-foreground);
    border: 1px solid var(--vscode-input-border);
    border-radius: 2px;
    outline: none;
  }

  :global(.deps-input:focus) {
    border-color: var(--vscode-focusBorder);
  }

  :global(.deps-save-button) {
    padding: 4px 10px;
    font-size: 12px;
    background-color: var(--vscode-button-background);
    color: var(--vscode-button-foreground);
    border: none;
    border-radius: 2px;
    cursor: pointer;
  }

  :global(.deps-save-button:hover) {
    background-color: var(--vscode-button-hoverBackground);
  }
</style>
