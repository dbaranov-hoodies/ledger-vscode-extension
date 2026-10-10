export type CommandStatus = "idle" | "running" | "success" | "error";

export interface Action {
  id: string;
  label: string;
  icon?: string;
  taskName?: string;
  commandName?: string;
  tooltip?: string;
  status: CommandStatus;
  disabledOnAllDevices?: boolean;
  disabled: boolean;
  pinned?: boolean;
}

export interface ActionGroupData {
  id: string;
  icon: string;
  mainAction?: Action;
  options: Action[];
  defaultOptions?: Action[];
  disabledOnAllDevices?: boolean;
  expanded?: boolean;
}

export function isActionDisabled(action: Action, allDevices: boolean): boolean {
  return action.disabled || (allDevices && (action.disabledOnAllDevices ?? false));
}
