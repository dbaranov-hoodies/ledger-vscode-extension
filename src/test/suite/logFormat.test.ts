import * as assert from "assert";
import { formatLog } from "../../shell/logFormat";

const reset = "\x1b[0m";

suite("Log format", () => {
  test("a line starts with the source it comes from, in the colour of that source", () => {
    const [stax] = formatLog({ kind: "line", source: "stax", stream: "stdout", text: "built" });
    const [flex] = formatLog({ kind: "line", source: "flex", stream: "stdout", text: "built" });

    assert.strictEqual(stax, `\x1b[36mstax   │${reset} built${reset}`);
    assert.strictEqual(flex, `\x1b[35mflex   │${reset} built${reset}`);
  });

  test("the sources are padded to the same width, so that the text lines up", () => {
    const widths = ["stax", "apex_p", "image"].map((source) => {
      const [line] = formatLog({ kind: "line", source, stream: "stdout", text: "x" });
      return line.replace(/\x1b\[[0-9;]*m/g, "").indexOf("│");
    });

    assert.deepStrictEqual(widths, [7, 7, 7]);
  });

  test("an error line is dimmed and a colour left on by a program does not reach the next line", () => {
    const [line] = formatLog({ kind: "line", source: "stax", stream: "stderr", text: "\x1b[31mwarning" });

    assert.ok(line.includes(`\x1b[2m\x1b[31mwarning${reset}`), line);
    assert.ok(line.endsWith(reset));
  });

  test("a command, its end and a note are told in the stream of their source", () => {
    const started = formatLog({ kind: "started", source: "image", command: "docker pull x" })[0];
    const done = formatLog({ kind: "finished", source: "stax", success: true })[0];
    const failed = formatLog({ kind: "finished", source: "stax", success: false })[0];
    const note = formatLog({ kind: "note", source: "flex", text: "postponed" })[0];

    assert.ok(started.includes("\x1b[1m$ docker pull x"), started);
    assert.ok(started.startsWith("\x1b[90mimage"), "the image is gray");
    assert.ok(done.includes("\x1b[32mdone"), done);
    assert.ok(failed.includes("\x1b[31mfailed"), failed);
    assert.ok(note.includes("\x1b[33mpostponed"), note);
  });

  test("a source without a colour of its own still gets one", () => {
    const [line] = formatLog({ kind: "line", source: "newdevice", stream: "stdout", text: "x" });

    assert.ok(line.startsWith("\x1b[37m"), line);
  });
});
