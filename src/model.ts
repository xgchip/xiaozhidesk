export interface ChatMessage {
  id: number;
  role: "user" | "assistant";
  text: string;
  time: string;
}
export interface DeviceEvent {
  type: string;
  state?: string;
  text?: string;
  emotion?: string;
  message?: string;
  session_id?: string;
  value?: number;
  enabled?: boolean;
  audio_params?: { sample_rate?: number; frame_duration?: number };
}
export type Phase =
  | "offline"
  | "connecting"
  | "activating"
  | "idle"
  | "listening"
  | "thinking"
  | "speaking";
export function nextPhase(phase: Phase, event: DeviceEvent): Phase {
  if (event.type === "disconnected") return "offline";
  if (event.type === "connected") return "idle";
  if (
    event.type === "voice_start" &&
    !["offline", "connecting", "activating"].includes(phase)
  )
    return "listening";
  if (event.type === "voice_end" && phase === "listening") return "thinking";
  if (
    (event.type === "voice_cancelled" || event.type === "call_state") &&
    !["offline", "connecting", "activating"].includes(phase)
  )
    return "idle";
  if (event.type === "stt") return phase === "listening" ? phase : "thinking";
  if (event.type === "tts" && phase !== "listening") {
    if (event.state === "start" || event.state === "sentence_start")
      return "speaking";
    if (event.state === "stop") return "idle";
  }
  return phase;
}
export function cleanText(value: unknown): string {
  return typeof value === "string" ? value.slice(0, 16000) : "";
}
