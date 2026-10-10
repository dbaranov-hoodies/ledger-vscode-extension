<script lang="ts">
  import "@vscode-elements/elements/dist/vscode-icon/index.js";
  import autoAnimate from "@formkit/auto-animate";
  import { ChevronDown, ChevronRight } from "@jis3r/icons";
  import Select, { type SelectItem } from "./components/Select.svelte";
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
  let showActions = $state(true);
  // The result of an action, shown for a few seconds once it is over.
  let results = $state<Record<string, CommandStatus>>({});

  const app = $derived(snapshot?.app ?? null);
  const apps = $derived<SelectItem[]>(
    (snapshot?.apps ?? []).map((summary) => ({ value: summary.folder, label: summary.name })),
  );
  const allDevices = "All";

  const deviceItems = $derived<SelectItem[]>([
    ...((app?.devices.available.length ?? 0) > 1 ? [{ value: allDevices, label: allDevices }] : []),
    ...(app?.devices.available ?? []).map((device) => ({ value: device.id, label: device.name })),
  ]);
  const selectedDevice = $derived(
    app?.devices.all
      ? allDevices
      : app?.devices.selected.length === 1
        ? app.devices.selected[0]
        : "",
  );
  const buildUseCases = $derived<SelectItem[]>(
    (app?.buildUseCases.items ?? []).map((name) => ({ value: name, label: name })),
  );
  const variants = $derived<SelectItem[]>(
    (app?.variants?.items ?? []).map((name) => ({ value: name, label: name })),
  );
  const enforcerChecks = $derived<SelectItem[]>(
    (app?.enforcerChecks.items ?? []).map((name) => ({ value: name, label: name })),
  );

  const reasons: Record<DisabledReason, string> = {
    severalDevices: "Select a single device",
    deviceActionsOff: "Actions on a Nano X are turned off in the settings",
    noComposeFile: "The app has no docker-compose.yml",
    noTestSuite: "The app has no test suite",
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

  function isGroupDisabled(group: ActionGroupData): boolean {
    return !!group.mainAction && group.mainAction.disabled;
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
      showActions,
    });
  }

  function run(actionId: string) {
    request("runAction", { id: actionId });
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

  function toggleActions() {
    showActions = !showActions;
    saveUiState();
  }

  function selectDevice(value: string) {
    const devices =
      value === allDevices ? (app?.devices.available ?? []).map((d) => d.id) : [value];
    request("setDevices", { devices });
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
        showActions = message.showActions ?? true;
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
              <Select
                icon="device-mobile"
                groupLabel="Device"
                items={deviceItems}
                value={selectedDevice}
                placeholder={app.devices.label}
                onchange={selectDevice}
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

      <button class="actions-toggle" onclick={toggleActions}>
        {#if showActions}
          <ChevronDown size={12} />
        {:else}
          <ChevronRight size={12} />
        {/if}
        <span>Actions</span>
      </button>

      <div class="actions-wrapper" use:autoAnimate>
        {#if showActions}
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
                  {/if}
                {/snippet}
              </ActionGroup>
            {/each}
          </div>
        {/if}
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

  .actions-toggle:hover {
    color: var(--vscode-foreground);
    opacity: 1;
  }

  .check-select-wrapper {
    flex: 50%;
    max-width: 200px;
    min-width: 0;
  }
</style>
