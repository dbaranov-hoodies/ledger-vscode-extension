import * as cp from "child_process";
import * as readline from "readline";
import * as vscode from "vscode";
import type { Notifications, RequestName, Requests } from "./protocol";

/** What the sidecar answered when a request failed. */
export class SidecarError extends Error {
  constructor(message: string, readonly code: number) {
    super(message);
  }
}

interface Pending {
  resolve: (result: any) => void;
  reject: (error: Error) => void;
}

type Handler = (params: any) => void;

/**
 * The process that does the work, and the JSON-RPC 2.0 conversation with it: one JSON value per
 * line in each direction.
 */
export class Sidecar implements vscode.Disposable {
  private readonly child: cp.ChildProcessWithoutNullStreams;
  private nextId = 1;
  private readonly pending = new Map<number, Pending>();
  private readonly handlers = new Map<string, Set<Handler>>();
  private readonly exitEmitter = new vscode.EventEmitter<string>();
  private ended?: string;

  /** Fires once, with the reason, when the process is gone. */
  readonly onDidExit = this.exitEmitter.event;

  constructor(binary: string, private readonly output: vscode.OutputChannel) {
    this.child = cp.spawn(binary, [], { stdio: ["pipe", "pipe", "pipe"] });
    readline.createInterface({ input: this.child.stdout }).on("line", line => this.receive(line));
    this.child.stderr.on("data", data => this.output.append(data.toString()));
    this.child.on("error", error => this.end(`could not start: ${error.message}`));
    this.child.on("exit", (code, signal) => this.end(`exited (${code ?? signal})`));
    // Writing to a process that is gone raises an error on the stream, which has no one to tell.
    this.child.stdin.on("error", () => undefined);
  }

  request<M extends RequestName>(
    method: M,
    params: Requests[M]["params"],
  ): Promise<Requests[M]["result"]> {
    if (this.ended) {
      return Promise.reject(new Error(`The Ledger sidecar ${this.ended}.`));
    }
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.write({ jsonrpc: "2.0", id, method, params });
    });
  }

  on<N extends keyof Notifications>(
    name: N,
    handler: (params: Notifications[N]) => void,
  ): vscode.Disposable {
    const handlers = this.handlers.get(name) ?? new Set<Handler>();
    handlers.add(handler);
    this.handlers.set(name, handlers);
    return new vscode.Disposable(() => handlers.delete(handler));
  }

  dispose(): void {
    this.child.stdin.end();
    // Give it a moment to say what is left, then make sure it is gone.
    const timer = setTimeout(() => this.child.kill(), 1000);
    this.child.once("exit", () => clearTimeout(timer));
    this.exitEmitter.dispose();
  }

  private write(message: object): void {
    this.child.stdin.write(`${JSON.stringify(message)}\n`);
  }

  private receive(line: string): void {
    let message: any;
    try {
      message = JSON.parse(line);
    }
    catch {
      this.output.appendLine(`Unreadable line from the sidecar: ${line}`);
      return;
    }
    if (typeof message.id === "number") {
      this.answer(message);
    }
    else if (typeof message.method === "string") {
      this.handlers.get(message.method)?.forEach(handler => handler(message.params));
    }
  }

  private answer(message: any): void {
    const pending = this.pending.get(message.id);
    if (!pending) {
      return;
    }
    this.pending.delete(message.id);
    if (message.error) {
      pending.reject(new SidecarError(message.error.message, message.error.code));
    }
    else {
      pending.resolve(message.result);
    }
  }

  private end(reason: string): void {
    if (this.ended) {
      return;
    }
    this.ended = reason;
    this.pending.forEach(pending => pending.reject(new Error(`The Ledger sidecar ${reason}.`)));
    this.pending.clear();
    this.exitEmitter.fire(reason);
  }
}
