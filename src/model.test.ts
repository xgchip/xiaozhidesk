import { describe, it, expect } from "vitest";
import { nextPhase, cleanText } from "./model";
describe("device conversation state", () => {
  it("follows automatic turns and barge-in without late TTS overwriting speech", () => {
    expect(nextPhase("speaking", { type: "voice_start" })).toBe("listening");
    expect(nextPhase("listening", { type: "tts", state: "stop" })).toBe(
      "listening",
    );
    expect(nextPhase("listening", { type: "voice_end" })).toBe("thinking");
    expect(nextPhase("speaking", { type: "voice_cancelled" })).toBe("idle");
    expect(nextPhase("offline", { type: "voice_start" })).toBe("offline");
  });
  it("follows recognition, speech and disconnection", () => {
    expect(nextPhase("thinking", { type: "stt" })).toBe("thinking");
    expect(nextPhase("thinking", { type: "tts", state: "start" })).toBe(
      "speaking",
    );
    expect(nextPhase("speaking", { type: "tts", state: "stop" })).toBe("idle");
    expect(nextPhase("listening", { type: "disconnected" })).toBe("offline");
  });
  it("does not let late TTS events overwrite recording", () => {
    expect(nextPhase("listening", { type: "tts", state: "stop" })).toBe(
      "listening",
    );
    expect(nextPhase("listening", { type: "stt" })).toBe("listening");
  });
  it("bounds remote text and ignores non-string content", () => {
    expect(cleanText({ text: "untrusted" })).toBe("");
    expect(cleanText("x".repeat(20000)).length).toBe(16000);
  });
});
