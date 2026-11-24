<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  AudioLines,
  Minus,
  ArrowUp,
  ArrowUpRight,
  ArrowLeft,
  Check,
  ChevronRight,
  Copy,
  Headphones,
  Link2,
  Expand,
  GripHorizontal,
  PawPrint,
  Phone,
  PhoneOff,
  Monitor,
  Power,
  Settings2,
  ShieldCheck,
  Square,
  Terminal,
  Volume2,
  VolumeX,
  X,
} from "@lucide/vue";
import {
  cleanText,
  nextPhase,
  type ChatMessage,
  type DeviceEvent,
  type Phase,
} from "./model";
import RobotCharacter from "./RobotCharacter.vue";
import avatarUrl from "./assets/xiaozhi-avatar.jpg";

async function windowAction(
  action: "minimize" | "maximize" | "close" | "drag",
) {
  if (!native) return;
  try {
    await invoke("window_action", { action });
  } catch (e) {
    fail(e);
  }
}
function dragTitlebar(event: PointerEvent) {
  if (event.button === 0 && !(event.target as HTMLElement).closest("button"))
    void windowAction("drag");
}

const native = isTauri();
const petMode = ref(false),
  petBusy = ref(false);
const muted = ref(false);
let previousVolume = 70;
let petClickTimer: ReturnType<typeof setTimeout> | undefined;
let dragOffset: { x: number; y: number; pointerId: number } | null = null;
let dragFrame: number | undefined;
const page = ref<"chat" | "settings" | "diagnostics">("chat");
const phase = ref<Phase>("offline");
const busy = ref(false),
  controlBusy = ref(false),
  error = ref("");
const deviceId = ref(""),
  clientId = ref(""),
  inputs = ref<string[]>([]),
  outputs = ref<string[]>([]);
function saved(key: string, fallback: string) {
  try {
    return localStorage.getItem(key) ?? fallback;
  } catch {
    return fallback;
  }
}
const otaUrl = ref(
  saved("xiaozhi.ota", "https://api.tenclass.net/xiaozhi/ota/"),
);
const input = ref(saved("xiaozhi.input", "")),
  output = ref(saved("xiaozhi.output", ""));
const volume = ref(
  Math.min(100, Math.max(0, Number(saved("xiaozhi.volume", "70")) || 0)),
);
const silenceMs = ref(
  Math.max(
    300,
    Math.min(
      2000,
      Math.round((Number(saved("xiaozhi.silence", "700")) || 700) / 50) * 50,
    ),
  ),
);
const callWanted = ref(false),
  callEnabled = ref(false);
let reconnectTimer: ReturnType<typeof setTimeout> | undefined;
let reconnectAttempt = 0;
const activation = ref<{
  code?: string;
  message?: string;
  challenge?: string;
  timeout_ms?: number;
} | null>(null);
const messages = ref<ChatMessage[]>([]),
  logs = ref<{ time: string; text: string }[]>([]);
const draft = ref(""),
  level = ref(0),
  emotion = ref("neutral"),
  sessionId = ref(""),
  serverRate = ref(0);
const messageList = ref<HTMLElement>(),
  copied = ref(false);
let sequence = 0,
  activationTimer: ReturnType<typeof setTimeout> | undefined,
  activationDeadline = 0,
  disposed = false;
let connectionEpoch = 0;
const unlisteners: UnlistenFn[] = [];
const connected = computed(() =>
  ["idle", "listening", "thinking", "speaking"].includes(phase.value),
);
const labels: Record<Phase, string> = {
  offline: "尚未连接",
  connecting: "正在连接",
  activating: "等待激活",
  idle: "准备就绪",
  listening: "正在聆听",
  thinking: "正在思考",
  speaking: "正在说话",
};
const statusLabel = computed(() =>
  callWanted.value && !connected.value
    ? phase.value === "activating"
      ? "等待激活"
      : "连接中"
    : callEnabled.value && phase.value === "idle"
      ? "通话中"
      : labels[phase.value],
);
const headline = computed(
  () =>
    ({
      offline: "你好，我是小智。",
      connecting: "正在与小智建立连接…",
      activating: "只差一步，就能见面。",
      idle: callEnabled.value ? "我在听，直接说就好。" : "今天想聊点什么？",
      listening: "嗯，我在听。",
      thinking: "让我想一想…",
      speaking: "有些话，想说给你听。",
    })[phase.value],
);
const petHint = computed(() =>
  error.value
    ? "点击查看提示"
    : callWanted.value && !connected.value
      ? "连接中 · 点我取消"
      : phase.value === "offline"
        ? "点我开始通话"
        : phase.value === "idle"
          ? callEnabled.value
            ? "通话中 · 点我挂断"
            : "点我开始通话"
          : labels[phase.value],
);
const petBubble = computed(() =>
  error.value
    ? "遇到一点小问题，打开看看？"
    : phase.value === "listening"
      ? "我在听，停顿后会自动发送。"
      : phase.value === "thinking"
        ? "让我想想…"
        : messages.value
            .filter((message) => message.role === "assistant")
            .at(-1)
            ?.text.slice(0, 72) || "我在这里，陪着你。",
);
async function togglePet(enabled: boolean) {
  clearTimeout(petClickTimer);
  if (petBusy.value || petMode.value === enabled) return;
  petBusy.value = true;
  const previous = petMode.value;
  petMode.value = enabled;
  document.documentElement.classList.toggle("pet-window", enabled);
  await nextTick();
  try {
    if (native) await run("set_pet_mode", { enabled });
    if (!enabled) page.value = "chat";
  } catch (e) {
    petMode.value = previous;
    document.documentElement.classList.toggle("pet-window", previous);
    fail(e);
  } finally {
    petBusy.value = false;
  }
}
function dragPet(event: PointerEvent) {
  if (event.button !== 0 || !native) return;
  event.preventDefault();
  dragOffset = {
    x: event.clientX,
    y: event.clientY,
    pointerId: event.pointerId,
  };
  (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
}
function movePet(event: PointerEvent) {
  if (
    !dragOffset ||
    event.pointerId !== dragOffset.pointerId ||
    dragFrame !== undefined
  )
    return;
  dragFrame = requestAnimationFrame(() => {
    dragFrame = undefined;
    if (dragOffset)
      void run("drag_pet", {
        offsetX: dragOffset.x,
        offsetY: dragOffset.y,
      }).catch(fail);
  });
}
function endPetDrag(event: PointerEvent) {
  if (!dragOffset || event.pointerId !== dragOffset.pointerId) return;
  if (dragFrame !== undefined) cancelAnimationFrame(dragFrame);
  dragFrame = undefined;
  if (event.type !== "pointercancel")
    void run("drag_pet", {
      offsetX: dragOffset.x,
      offsetY: dragOffset.y,
    }).catch(fail);
  dragOffset = null;
  (event.currentTarget as HTMLElement).releasePointerCapture(event.pointerId);
}
async function petAction() {
  if (petBusy.value || (busy.value && !callWanted.value)) return;
  if (error.value && !callWanted.value) {
    await togglePet(false);
    return;
  }
  await toggleCall();
}
function petClick() {
  clearTimeout(petClickTimer);
  petClickTimer = setTimeout(() => void petAction(), 230);
}
function toggleMute() {
  if (volume.value > 0) {
    previousVolume = volume.value;
    volume.value = 0;
    muted.value = true;
  } else {
    volume.value = previousVolume || 70;
    muted.value = false;
  }
}
const clock = () =>
  new Date().toLocaleTimeString("zh-CN", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
function log(text: string) {
  logs.value.push({ time: clock(), text });
  if (logs.value.length > 200) logs.value.shift();
}
function fail(e: unknown) {
  error.value = String(e);
  log(error.value);
}
async function run<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (!native)
    throw new Error(
      "当前是网页预览。请启动桌面应用以使用激活、麦克风和小智后台。",
    );
  return invoke<T>(command, args);
}
async function refreshDevices() {
  try {
    const result = await run<{
      identity: { device_id: string; client_id: string };
      audio: { inputs: string[]; outputs: string[] };
      connected: boolean;
      call_enabled: boolean;
    }>("get_device");
    deviceId.value = result.identity.device_id;
    clientId.value = result.identity.client_id;
    inputs.value = result.audio.inputs;
    outputs.value = result.audio.outputs;
    if (result.connected && phase.value === "offline") phase.value = "idle";
    callEnabled.value = result.call_enabled;
    if (result.call_enabled) callWanted.value = true;
  } catch (e) {
    fail(e);
  }
}
function addMessage(role: ChatMessage["role"], text: string) {
  if (!text) return;
  messages.value.push({
    id: ++sequence,
    role,
    text,
    time: clock().slice(0, 5),
  });
  if (messages.value.length > 200) messages.value.shift();
  nextTick(() =>
    messageList.value?.scrollTo({
      top: messageList.value.scrollHeight,
      behavior: "smooth",
    }),
  );
}
function onEvent(event: DeviceEvent) {
  const previousPhase = phase.value;
  phase.value = nextPhase(phase.value, event);
  if (event.type === "connected") {
    reconnectAttempt = 0;
    sessionId.value = event.session_id ?? "";
    serverRate.value = event.audio_params?.sample_rate ?? 0;
  }
  if (event.type === "disconnected") {
    callEnabled.value = false;
    level.value = 0;
    sessionId.value = "";
    if (event.message) {
      if (
        !callWanted.value &&
        ["listening", "thinking", "speaking"].includes(previousPhase)
      )
        fail(event.message);
      else log(event.message);
    }
    scheduleReconnect();
  }
  if (event.type === "call_state") callEnabled.value = event.enabled === true;
  if (event.type === "stt") addMessage("user", cleanText(event.text));
  if (event.type === "tts" && event.state === "sentence_start")
    addMessage("assistant", cleanText(event.text));
  if (event.type === "llm")
    emotion.value = cleanText(event.emotion) || "neutral";
  if (event.type === "volume" && typeof event.value === "number")
    volume.value = event.value;
  if (event.type === "audio_error") {
    fail(event.message ?? "音频设备不可用");
    void disconnect();
  }
  if (event.type === "alert" || event.type === "error")
    fail(event.message ?? event.text ?? "后台返回错误");
  if (event.type !== "llm")
    log(`${event.type}${event.state ? ` · ${event.state}` : ""}`);
}
async function connect() {
  if (busy.value || connected.value) return;
  clearTimeout(activationTimer);
  const epoch = ++connectionEpoch;
  busy.value = true;
  error.value = "";
  phase.value = "connecting";
  try {
    const result = await run<{
      ready: boolean;
      activation: typeof activation.value;
      mqtt_only: boolean;
    }>("discover", { otaUrl: otaUrl.value.trim() });
    if (epoch !== connectionEpoch || disposed) return;
    activation.value = result.activation;
    if (result.ready) await openSession();
    else if (result.activation?.code) {
      phase.value = "activating";
      activationDeadline = Date.now() + 5 * 60 * 1000;
      log("已获取激活码，请在 xiaozhi.me 绑定设备");
      scheduleActivation();
    } else
      throw new Error(
        result.mqtt_only
          ? "后台仅返回 MQTT 配置。此客户端使用 WebSocket，请在后台启用 WebSocket 或更换兼容 OTA 地址。"
          : "后台未返回激活码或 WebSocket 地址，请检查 OTA 设置。",
      );
  } catch (e) {
    phase.value = "offline";
    fail(e);
  } finally {
    busy.value = false;
  }
}
async function openSession() {
  phase.value = "connecting";
  await run("connect_device", {
    input: input.value,
    output: output.value,
    volume: volume.value,
    silenceMs: silenceMs.value,
  });
  activation.value = null;
  if (callWanted.value) await syncCall();
}
function scheduleActivation() {
  activationTimer = setTimeout(pollActivation, 3000);
}
async function pollActivation() {
  if (disposed || phase.value !== "activating") return;
  const epoch = connectionEpoch;
  if (Date.now() > activationDeadline) {
    error.value = "等待激活已超时，点击“重新检查”获取最新状态。";
    return;
  }
  try {
    if (activation.value?.challenge) {
      const done = await run<boolean>("activate", {
        otaUrl: otaUrl.value.trim(),
      });
      if (epoch !== connectionEpoch || disposed) return;
      if (!done) {
        scheduleActivation();
        return;
      }
    }
    const result = await run<{
      ready: boolean;
      activation: typeof activation.value;
    }>("discover", { otaUrl: otaUrl.value.trim() });
    if (epoch !== connectionEpoch || disposed) return;
    if (result.ready) {
      await openSession();
      return;
    }
    activation.value = result.activation;
    scheduleActivation();
  } catch (e) {
    fail(e);
  }
}
async function disconnect() {
  ++connectionEpoch;
  callWanted.value = false;
  callEnabled.value = false;
  clearTimeout(reconnectTimer);
  clearTimeout(activationTimer);
  activation.value = null;
  if (!connected.value) {
    phase.value = "offline";
    return;
  }
  busy.value = true;
  try {
    await run("disconnect_device");
  } catch (e) {
    fail(e);
  } finally {
    busy.value = false;
  }
}
function scheduleReconnect() {
  clearTimeout(reconnectTimer);
  if (!callWanted.value || disposed || !native || phase.value === "activating")
    return;
  const delay = Math.min(10000, 1000 * 2 ** Math.min(reconnectAttempt++, 4));
  reconnectTimer = setTimeout(async () => {
    if (!callWanted.value || disposed) return;
    if (!connected.value && !busy.value) await connect();
    if (!connected.value && phase.value !== "activating") scheduleReconnect();
  }, delay);
}
async function syncCall() {
  if (!connected.value || controlBusy.value) return;
  controlBusy.value = true;
  try {
    while (connected.value && callWanted.value !== callEnabled.value) {
      const enabled = callWanted.value;
      await run("set_call", { enabled, silenceMs: silenceMs.value });
      if (!connected.value) break;
      callEnabled.value = enabled;
      if (!enabled) {
        phase.value = "idle";
        level.value = 0;
      }
    }
  } catch (e) {
    callWanted.value = false;
    fail(e);
  } finally {
    controlBusy.value = false;
  }
}
async function toggleCall() {
  callWanted.value = !callWanted.value;
  error.value = "";
  if (!callWanted.value) {
    clearTimeout(reconnectTimer);
    await syncCall();
    return;
  }
  if (!connected.value) await connect();
  await syncCall();
  if (!connected.value && phase.value !== "activating") scheduleReconnect();
}
async function abort() {
  try {
    await run("control", { action: "abort" });
    phase.value = "idle";
  } catch (e) {
    fail(e);
  }
}
async function sendText() {
  const text = draft.value.trim();
  if (!text || !connected.value || busy.value || phase.value === "listening")
    return;
  try {
    await run("send_text", { text });
    addMessage("user", text);
    draft.value = "";
    phase.value = "thinking";
  } catch (e) {
    fail(e);
  }
}
async function openConsole() {
  try {
    await run("open_console");
  } catch (e) {
    fail(e);
  }
}
async function copyIdentity() {
  try {
    await navigator.clipboard.writeText(deviceId.value);
    copied.value = true;
    setTimeout(() => (copied.value = false), 1800);
  } catch (e) {
    fail(e);
  }
}
function keydown(e: KeyboardEvent) {
  if (e.key === "Escape" && petMode.value) {
    e.preventDefault();
    void togglePet(false);
    return;
  }
  if (
    e.code !== "Space" ||
    e.repeat ||
    (e.target instanceof HTMLElement &&
      (e.target.matches("input, textarea, select, button, a") ||
        e.target.isContentEditable))
  )
    return;
  e.preventDefault();
  void toggleCall();
}
watch([otaUrl, input, output, volume, silenceMs], () => {
  try {
    for (const [key, value] of Object.entries({
      ota: otaUrl.value,
      input: input.value,
      output: output.value,
      volume: String(volume.value),
      silence: String(silenceMs.value),
    }))
      localStorage.setItem(`xiaozhi.${key}`, value);
  } catch {
    /* The current session still works without storage. */
  }
});
watch(silenceMs, (value) => {
  if (connected.value)
    void run("set_voice_config", { silenceMs: value }).catch(fail);
});
watch(volume, (value) => {
  muted.value = value === 0;
  if (connected.value) void run("set_volume", { volume: value }).catch(fail);
});
watch(phase, (value) => {
  if (value === "activating" && petMode.value) void togglePet(false);
});
onMounted(async () => {
  window.addEventListener("keydown", keydown);

  if (native) {
    try {
      petMode.value = await run<boolean>("get_window_mode");
      document.documentElement.classList.toggle("pet-window", petMode.value);
      unlisteners.push(
        await listen<DeviceEvent>("device-event", (e) => onEvent(e.payload)),
        await listen<number>("audio-level", (e) => (level.value = e.payload)),
      );
      await refreshDevices();
      log("桌面设备已就绪");
    } catch (e) {
      fail(e);
    }
  } else log("网页预览模式 · 原生设备功能需在 Tauri 中运行");
});
onUnmounted(() => {
  callWanted.value = false;
  clearTimeout(reconnectTimer);
  if (dragFrame !== undefined) cancelAnimationFrame(dragFrame);
  clearTimeout(petClickTimer);
  disposed = true;
  clearTimeout(activationTimer);
  unlisteners.forEach((fn) => fn());
  window.removeEventListener("keydown", keydown);

  if (connected.value) void run("disconnect_device");
});
</script>

<template>
  <section
    v-if="petMode"
    class="pet-stage"
    aria-label="小智桌面宠物"
    @contextmenu.prevent="togglePet(false)"
  >
    <div class="pet-toolbar">
      <button
        class="pet-grip"
        aria-label="拖动小智"
        title="拖动小智"
        @pointerdown="dragPet"
        @pointermove="movePet"
        @pointerup="endPetDrag"
        @pointercancel="endPetDrag"
      >
        <GripHorizontal :size="16" />
      </button>
      <button
        :aria-label="muted ? '取消静音' : '静音'"
        :title="muted ? '取消静音' : '静音'"
        @click="toggleMute"
      >
        <VolumeX v-if="muted" :size="15" /><Volume2 v-else :size="15" />
      </button>
      <button
        aria-label="返回完整窗口"
        title="返回完整窗口 · Esc"
        :disabled="petBusy"
        @click="togglePet(false)"
      >
        <Expand :size="15" />
      </button>
    </div>
    <button
      class="pet-bubble"
      :title="error || petBubble"
      @click="togglePet(false)"
    >
      {{ petBubble }}
    </button>
    <button
      class="pet-character-button"
      :aria-label="petHint"
      :disabled="((busy || controlBusy) && !callWanted) || petBusy"
      :title="callWanted ? '结束通话' : '开始通话'"
      @click="petClick"
      @dblclick="togglePet(false)"
    >
      <RobotCharacter :phase="phase" :emotion="emotion" />
    </button>
    <div class="pet-caption">
      <span
        class="status-dot"
        :class="{ online: connected, recording: phase === 'listening' }"
      ></span
      >{{ petHint }}
    </div>
  </section>

  <div v-else class="app-shell" :class="{ calling: callEnabled }">
    <div
      class="titlebar"
      @pointerdown="dragTitlebar"
      @dblclick.self="windowAction('maximize')"
    >
      <span class="tb-dot"></span><span class="tb-title">小智 · 桌面通话</span>
      <div class="tb-actions" @dblclick.stop>
        <button
          class="tb-btn"
          aria-label="最小化"
          title="最小化"
          :disabled="!native"
          @click="windowAction('minimize')"
        >
          <Minus :size="10" />
        </button>
        <button
          class="tb-btn"
          aria-label="最大化或还原"
          title="最大化 / 还原"
          :disabled="!native"
          @click="windowAction('maximize')"
        >
          <Square :size="10" />
        </button>
        <button
          class="tb-btn close"
          aria-label="关闭窗口"
          title="关闭"
          :disabled="!native"
          @click="windowAction('close')"
        >
          <X :size="10" />
        </button>
      </div>
    </div>
    <header class="app-header">
      <button
        v-if="page !== 'chat'"
        class="icon-button back-button"
        aria-label="返回聊天"
        @click="page = 'chat'"
      >
        <ArrowLeft :size="20" />
      </button>
      <div class="brand">
        <span class="brand-mark"><AudioLines :size="21" /></span
        ><span>小智</span>
      </div>
      <div class="header-status">
        <span
          class="status-dot"
          :class="{ online: connected, recording: phase === 'listening' }"
        ></span
        >{{ statusLabel }}
      </div>
      <div class="header-actions">
        <button
          class="pet-toggle"
          :disabled="petBusy || phase === 'activating' || busy"
          title="变成桌面宠物"
          @click="togglePet(true)"
        >
          <PawPrint :size="17" /><span>桌面宠物</span>
        </button>
        <button
          class="icon-button"
          :class="{ selected: page === 'settings' }"
          aria-label="设备设置"
          title="设置"
          @click="page = page === 'settings' ? 'chat' : 'settings'"
        >
          <Settings2 :size="20" />
        </button>
      </div>
    </header>
    <div v-if="!native" class="preview-note">
      <Monitor :size="15" /> 网页预览，连接和语音功能请使用桌面应用。
    </div>
    <div v-if="error" class="error-banner" role="alert">
      <span>{{ error }}</span
      ><button class="icon-button" aria-label="关闭提示" @click="error = ''">
        <X :size="16" />
      </button>
    </div>

    <main v-if="page === 'chat'" class="chat-page">
      <div
        v-if="activation?.code && phase === 'activating'"
        class="activation-card"
      >
        <span class="section-icon"><Link2 :size="22" /></span>
        <h1>认识一下，你的桌面小智</h1>
        <p>在小智控制台添加设备，输入下方激活码。</p>
        <div class="activation-code">{{ activation.code }}</div>
        <div class="button-row">
          <button class="primary" @click="openConsole">
            打开控制台 <ArrowUpRight :size="16" /></button
          ><button class="quiet-button" :disabled="busy" @click="connect">
            重新检查
          </button>
        </div>
        <span class="activation-note">绑定后自动连接</span
        ><button class="text-button" @click="disconnect">取消</button>
      </div>

      <template v-else>
        <div class="call-content">
          <div class="avatar-block">
            <div class="halo"></div>
            <div v-for="n in 3" :key="n" class="ring"></div>
            <button
              class="call-avatar"
              aria-label="点击小智变成桌面宠物"
              title="变成桌面宠物"
              :disabled="petBusy || busy"
              @click="togglePet(true)"
            >
              <span class="avatar-frame"
                ><img :src="avatarUrl" alt="小智头像" draggable="false"
              /></span>
              <span
                class="online-badge"
                :class="{ disconnected: !connected }"
                :aria-label="statusLabel"
              ></span>
            </button>
            <div
              class="voice-bars"
              :class="{ active: phase === 'listening' || phase === 'speaking' }"
              aria-hidden="true"
            >
              <span
                v-for="n in 5"
                :key="n"
                :style="
                  phase === 'listening'
                    ? {
                        height: `${6 + Math.min(14, level * 110) * (0.5 + (n % 3) / 4)}px`,
                      }
                    : {}
                "
              ></span>
            </div>
          </div>
          <div ref="messageList" class="stage" aria-live="polite">
            <div v-if="!messages.length" class="intro">
              <h1>{{ headline }}</h1>
              <p>
                {{
                  callEnabled
                    ? "停顿后自动发送，随时开口打断我。"
                    : "像打电话一样，直接聊聊吧。"
                }}
              </p>
            </div>
            <div v-else class="msgs">
              <article
                v-for="message in messages"
                :key="message.id"
                class="msg"
                :class="message.role === 'user' ? 'me' : 'bot'"
              >
                {{ message.text }}
              </article>
              <div
                v-if="phase === 'thinking'"
                class="msg bot typing"
                aria-label="小智正在思考"
              >
                <i></i><i></i><i></i>
              </div>
            </div>
          </div>
          <div class="call-row">
            <button
              class="call-btn"
              :class="{ pending: callWanted && !callEnabled }"
              :disabled="!native || ((busy || controlBusy) && !callWanted)"
              @click="toggleCall"
            >
              <Phone :size="17" /><span>{{
                callWanted
                  ? callEnabled
                    ? "结束通话"
                    : "取消连接"
                  : "开始通话"
              }}</span>
            </button>
          </div>
        </div>
        <div class="call-composer">
          <form class="input-wrap" @submit.prevent="sendText">
            <button
              type="button"
              class="side-btn"
              :disabled="!native || ((busy || controlBusy) && !callWanted)"
              :aria-label="callWanted ? '结束通话' : '开始通话'"
              :title="callWanted ? '结束通话' : '开始通话'"
              @click="toggleCall"
            >
              <PhoneOff v-if="callWanted" :size="18" /><Phone
                v-else
                :size="18"
              />
            </button>
            <input
              v-model="draft"
              aria-label="发送文字"
              :disabled="!connected || phase === 'listening'"
              :placeholder="connected ? '说点什么吧…' : '连接后开始聊天'"
              maxlength="2000"
              autocomplete="off"
            />
            <button
              class="send-btn"
              :class="{
                on: connected && draft.trim() && phase !== 'listening',
              }"
              aria-label="发送文字"
              :disabled="!connected || !draft.trim() || phase === 'listening'"
            >
              <ArrowUp :size="16" />
            </button>
          </form>
          <div class="hint">
            <template v-if="callEnabled"
              ><span
                >{{ phase === "listening" ? "正在聆听" : "随时开口打断" }} ·
                停顿 {{ (silenceMs / 1000).toFixed(2) }} 秒自动发送</span
              ></template
            >
            <template v-else-if="callWanted"
              ><span>正在连接，点击电话可取消</span></template
            >
            <template v-else
              ><kbd>Space</kbd><span>开始 / 结束通话</span></template
            >
            <button
              v-if="phase === 'speaking' || phase === 'thinking'"
              class="call-interrupt"
              @click="abort"
              title="停止回复"
            >
              <Square :size="10" fill="currentColor" />打断
            </button>
          </div>
        </div>
      </template>
    </main>

    <main v-else-if="page === 'settings'" class="settings-page">
      <h1>设置</h1>
      <p class="page-subtitle">让小智更适合你。</p>
      <section class="settings-card call-settings">
        <div class="card-heading">
          <h2><Phone :size="18" /> 通话</h2>
          <span class="pause-value"
            >{{ (silenceMs / 1000).toFixed(2) }} 秒</span
          >
        </div>
        <label for="silence-ms">停顿多久自动发送</label>
        <input
          id="silence-ms"
          v-model.number="silenceMs"
          class="pause-slider"
          type="range"
          min="300"
          max="2000"
          step="50"
          aria-label="停顿多久自动发送"
        />
        <div class="pause-scale">
          <span>响应更快</span><span>留更多思考时间</span>
        </div>
        <p class="field-help">
          数值越小发送越快；调大可以避免话没说完就发送。调整立即生效。
        </p>
      </section>
      <section class="settings-card">
        <div class="card-heading">
          <h2><Headphones :size="18" /> 声音</h2>
          <button
            class="text-button"
            :disabled="!native"
            @click="refreshDevices"
          >
            刷新设备
          </button>
        </div>
        <div class="settings-grid">
          <div>
            <label for="mic">麦克风</label
            ><select id="mic" v-model="input" :disabled="connected || busy">
              <option value="">系统默认</option>
              <option v-for="name in inputs" :key="name" :value="name">
                {{ name }}
              </option>
            </select>
          </div>
          <div>
            <label for="speaker">扬声器</label
            ><select
              id="speaker"
              v-model="output"
              :disabled="connected || busy"
            >
              <option value="">系统默认</option>
              <option v-for="name in outputs" :key="name" :value="name">
                {{ name }}
              </option>
            </select>
          </div>
        </div>
        <div class="volume-row">
          <Volume2 :size="17" /><input
            v-model.number="volume"
            type="range"
            min="0"
            max="100"
            aria-label="播放音量"
          /><span>{{ volume }}%</span>
        </div>
        <p class="field-help">切换设备需重新连接，建议使用耳机。</p>
      </section>
      <button
        class="pet-settings-card"
        :disabled="petBusy || busy || phase === 'activating'"
        @click="togglePet(true)"
      >
        <span class="pet-settings-icon"><PawPrint :size="22" /></span
        ><span
          ><strong>桌面宠物</strong
          ><small>缩成一只小小智，随时陪着你。</small></span
        ><ArrowUpRight :size="18" />
      </button>
      <details class="advanced-settings">
        <summary>高级设置 <ChevronRight :size="16" /></summary>
        <div class="advanced-body">
          <label for="ota">OTA 服务地址</label
          ><input
            id="ota"
            v-model="otaUrl"
            :disabled="connected || busy || phase === 'activating'"
            spellcheck="false"
          />
          <div class="identity-row">
            <span
              >设备 ID <code>{{ deviceId || "—" }}</code></span
            ><button
              class="icon-button"
              :disabled="!deviceId"
              aria-label="复制设备 ID"
              @click="copyIdentity"
            >
              <Check v-if="copied" :size="16" /><Copy v-else :size="16" />
            </button>
          </div>
          <div class="identity-row">
            <span
              >客户端 ID <code>{{ clientId || "—" }}</code></span
            >
          </div>
          <button class="diagnostics-link" @click="page = 'diagnostics'">
            <Terminal :size="16" /> 连接诊断 <ChevronRight :size="15" />
          </button>
        </div>
      </details>
      <div class="settings-footer">
        <button class="quiet-button" @click="openConsole">
          小智控制台 <ArrowUpRight :size="14" /></button
        ><button
          class="quiet-button"
          :disabled="busy || !native"
          @click="
            connected || phase === 'activating' ? disconnect() : connect()
          "
        >
          <Power :size="15" />{{
            connected
              ? "断开连接"
              : phase === "activating"
                ? "取消激活"
                : "连接小智"
          }}
        </button>
      </div>
    </main>

    <main v-else class="settings-page">
      <div class="card-heading">
        <h1>连接诊断</h1>
        <button class="text-button" @click="logs = []">清空日志</button>
      </div>
      <p class="page-subtitle">
        {{ labels[phase]
        }}<span v-if="serverRate"> · {{ serverRate / 1000 }} kHz</span>
      </p>
      <div class="diagnostic-meta">
        <span
          >会话 <code>{{ sessionId || "尚未建立" }}</code></span
        ><span>表情 {{ emotion }}</span>
      </div>
      <div class="log-panel" aria-live="polite">
        <div v-for="(entry, index) in logs" :key="index">
          <time>{{ entry.time }}</time
          ><span>{{ entry.text }}</span>
        </div>
        <p v-if="!logs.length">暂无事件</p>
      </div>
      <p class="field-help">
        <ShieldCheck :size="13" /> 日志不记录令牌和对话正文。
      </p>
    </main>
  </div>
</template>
