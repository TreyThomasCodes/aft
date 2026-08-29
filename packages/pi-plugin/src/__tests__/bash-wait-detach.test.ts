import { describe, expect, test } from "bun:test";
import {
  BASH_WAIT_DETACH_MAGIC_KEYWORD,
  shouldDetachBashWaitOnUserMessage,
  stripUserMessageDetachKeyword,
} from "../bash-wait-detach.js";

describe("bash wait detach helper (Pi)", () => {
  test("default config detaches on a plain user message", () => {
    expect(shouldDetachBashWaitOnUserMessage({}, "please continue")).toBe(true);
  });

  test("opt-out suppresses plain messages and preserves the keyword escape hatch", () => {
    const config = { bash: { detach_on_user_message: false } };
    const plain = "please continue";
    const message = `please ${BASH_WAIT_DETACH_MAGIC_KEYWORD} continue`;

    expect(shouldDetachBashWaitOnUserMessage(config, plain)).toBe(false);
    expect(shouldDetachBashWaitOnUserMessage(config, message)).toBe(true);
    expect(stripUserMessageDetachKeyword(message)).toBe("please continue");
  });

  test("substitutes an honest message when the token is the only user text", () => {
    expect(stripUserMessageDetachKeyword("  &detach  ")).toBe("(requested background detach)");
  });

  test("strips every token and preserves the rest of a message", () => {
    expect(stripUserMessageDetachKeyword("before &detach middle &detach after")).toBe(
      "before middle after",
    );
  });

  test("recognizes standalone tokens at message boundaries", () => {
    const config = { bash: { detach_on_user_message: false } };

    expect(shouldDetachBashWaitOnUserMessage(config, "&detach, continue")).toBe(true);
    expect(shouldDetachBashWaitOnUserMessage(config, "continue &detach")).toBe(true);
    expect(stripUserMessageDetachKeyword("&detach, continue")).toBe(", continue");
    expect(stripUserMessageDetachKeyword("continue &detach")).toBe("continue ");
  });

  test("does not detach or strip when the keyword is part of an identifier", () => {
    const config = { bash: { detach_on_user_message: false } };
    const messages = [
      "Please document &detachment behavior",
      "keep before&detach unchanged",
      "keep &detach_mode unchanged",
      "keep &detaché unchanged",
    ];

    for (const message of messages) {
      expect(shouldDetachBashWaitOnUserMessage(config, message)).toBe(false);
      expect(stripUserMessageDetachKeyword(message)).toBe(message);
    }
  });
});
