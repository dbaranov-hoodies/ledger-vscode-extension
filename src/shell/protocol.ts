/**
 * The messages between the extension and the sidecar. The types come from the Rust side
 * (src/generated, written by `cargo test`); only the pairing of method and type is written here.
 */

import type { ActionFinishedNotification } from "../generated/ActionFinishedNotification";
import type { CancelActionParams } from "../generated/CancelActionParams";
import type { InitializeParams } from "../generated/InitializeParams";
import type { LocateTestParams } from "../generated/LocateTestParams";
import type { LogNotification } from "../generated/LogNotification";
import type { MessageNotification } from "../generated/MessageNotification";
import type { RefreshParams } from "../generated/RefreshParams";
import type { RunActionParams } from "../generated/RunActionParams";
import type { SavedNotification } from "../generated/SavedNotification";
import type { SelectAppParams } from "../generated/SelectAppParams";
import type { SetBuildUseCaseParams } from "../generated/SetBuildUseCaseParams";
import type { SetDevicesParams } from "../generated/SetDevicesParams";
import type { SetAiParams } from "../generated/SetAiParams";
import type { SetTestSuiteParams } from "../generated/SetTestSuiteParams";
import type { SetTestsParams } from "../generated/SetTestsParams";
import type { ToggleTestParams } from "../generated/ToggleTestParams";
import type { SetEnforcerCheckParams } from "../generated/SetEnforcerCheckParams";
import type { SetVariantParams } from "../generated/SetVariantParams";
import type { SettingsInput } from "../generated/SettingsInput";
import type { Snapshot } from "../generated/Snapshot";
import type { TestLocation } from "../generated/TestLocation";

export type { Snapshot, SettingsInput, InitializeParams, LogNotification };

/** What the extension can ask: the parameters, and what comes back. */
export interface Requests {
  initialize: { params: InitializeParams; result: Snapshot };
  refresh: { params: RefreshParams; result: Snapshot };
  updateSettings: { params: SettingsInput; result: Snapshot };
  selectApp: { params: SelectAppParams; result: Snapshot };
  setDevices: { params: SetDevicesParams; result: Snapshot };
  setBuildUseCase: { params: SetBuildUseCaseParams; result: Snapshot };
  setVariant: { params: SetVariantParams; result: Snapshot };
  setEnforcerCheck: { params: SetEnforcerCheckParams; result: Snapshot };
  collectTests: { params: Record<string, never>; result: Snapshot };
  toggleTest: { params: ToggleTestParams; result: Snapshot };
  setTests: { params: SetTestsParams; result: Snapshot };
  selectFailedTests: { params: Record<string, never>; result: Snapshot };
  locateTest: { params: LocateTestParams; result: TestLocation };
  setTestSuite: { params: SetTestSuiteParams; result: Snapshot };
  setAi: { params: SetAiParams; result: Snapshot };
  runAction: { params: RunActionParams; result: null };
  cancelAction: { params: CancelActionParams; result: null };
}

export type RequestName = keyof Requests;

/** What the sidecar tells on its own. */
export interface Notifications {
  snapshot: Snapshot;
  saved: SavedNotification;
  log: LogNotification;
  actionFinished: ActionFinishedNotification;
  message: MessageNotification;
}

/** The requests the webview may send on, which change nothing but the choices of the user. */
export const webviewRequests: readonly RequestName[] = [
  "selectApp",
  "setDevices",
  "setBuildUseCase",
  "setVariant",
  "setEnforcerCheck",
  "collectTests",
  "toggleTest",
  "setTests",
  "selectFailedTests",
  "setTestSuite",
  "runAction",
  "cancelAction",
  "refresh",
];
