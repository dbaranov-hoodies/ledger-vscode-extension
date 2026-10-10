/**
 * Shared types - used by both extension and webview
 * This file must NOT import any Node.js or VS Code modules
 */

/** How ready the containers of the app are, as the sidecar reports it. */
export enum DevImageStatus {
  running = "running",
  syncing = "syncing",
  stopped = "stopped",
}
