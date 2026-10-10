import type { LogNotification } from "./protocol";

const reset = "\x1b[0m";
const bold = "\x1b[1m";
const dim = "\x1b[2m";
const green = "\x1b[32m";
const red = "\x1b[31m";
const yellow = "\x1b[33m";

/** The longest name of a source (`apex_p`), so that the lines of all sources line up. */
const sourceWidth = 6;

/** Each device keeps its colour, so it is easy to follow in a stream of several. */
const sourceColors = new Map<string, number>([
  ["nanos", 37],
  ["nanosp", 32],
  ["nanox", 33],
  ["stax", 36],
  ["flex", 35],
  ["apex_p", 34],
  ["apex_m", 91],
  ["image", 90],
]);

function label(source: string): string {
  const color = sourceColors.get(source) ?? 37;
  return `\x1b[${color}m${source.padEnd(sourceWidth)} │${reset}`;
}

/** The lines to show for an event of the log, each starting with the source it comes from. */
export function formatLog(log: LogNotification): string[] {
  const prefix = label(log.source);
  switch (log.kind) {
    case "started":
      return [`${prefix} ${bold}$ ${log.command}${reset}`];
    case "line":
      // What a program prints may leave a colour on: the reset keeps it from the next line.
      return [`${prefix} ${log.stream === "stderr" ? dim : ""}${log.text}${reset}`];
    case "finished":
      return [`${prefix} ${log.success ? `${green}done` : `${red}failed`}${reset}`];
    case "note":
      return [`${prefix} ${yellow}${log.text}${reset}`];
  }
}
