<script setup lang="ts">
import type { Phase } from "./model";
withDefaults(defineProps<{ phase: Phase; emotion?: string }>(), {
  emotion: "neutral",
});
</script>

<template>
  <div
    class="character"
    :class="phase"
    :data-emotion="emotion"
    aria-hidden="true"
  >
    <div class="character-aura"></div>
    <div class="character-shadow"></div>
    <div class="character-body">
      <div class="character-antenna"><i></i></div>
      <div class="character-ear ear-left"></div>
      <div class="character-ear ear-right"></div>
      <div class="character-face">
        <i class="character-eye eye-left"></i
        ><i class="character-eye eye-right"></i><i class="character-mouth"></i>
      </div>
      <i class="character-cheek cheek-left"></i
      ><i class="character-cheek cheek-right"></i>
      <span class="character-badge">✦</span>
    </div>
    <span v-if="phase === 'thinking'" class="character-thought">···</span>
  </div>
</template>

<style scoped>
.character {
  position: relative;
  width: 200px;
  height: 180px;
  user-select: none;
  pointer-events: none;
}
.character-aura {
  position: absolute;
  inset: 7px 15px 13px;
  border-radius: 50%;
  background: radial-gradient(ellipse, #dce4cd55, transparent 70%);
}
.character-body {
  position: absolute;
  left: 25px;
  top: 34px;
  width: 150px;
  height: 116px;
  border-radius: 40px 40px 37px 37px;
  background: linear-gradient(140deg, #f4f0e4 0%, #e2dbc8 90%);
  box-shadow:
    inset 0 3px 0 #fffdf3,
    inset 0 -5px 0 #c9c4ae,
    0 9px 16px #353d2115;
  transform: rotate(-4deg);
  animation: drift 4s ease-in-out infinite;
}
.character-face {
  position: absolute;
  inset: 19px 14px 25px;
  background: linear-gradient(135deg, #34483b, #273b33);
  border-radius: 24px;
  box-shadow:
    inset 0 3px 7px #17281e77,
    0 1px 0 #ffffff55;
}
.character-eye {
  position: absolute;
  width: 10px;
  height: 21px;
  top: 20px;
  border-radius: 6px;
  background: #d7e6b5;
  box-shadow: 0 0 9px #d7e6b51a;
  animation: blink 6s infinite;
}
.eye-left {
  left: 32px;
}
.eye-right {
  right: 32px;
}
.character-mouth {
  position: absolute;
  width: 22px;
  height: 10px;
  left: 50px;
  top: 42px;
  border-bottom: 2px solid #c2d8a0;
  border-radius: 0 0 14px 14px;
}
.character-ear {
  position: absolute;
  top: 43px;
  width: 12px;
  height: 30px;
  background: linear-gradient(#dad5c3, #bfbba7);
  border-radius: 6px;
  z-index: -1;
}
.ear-left {
  left: -7px;
}
.ear-right {
  right: -7px;
}
.character-antenna {
  position: absolute;
  width: 5px;
  height: 21px;
  left: 73px;
  top: -18px;
  border-radius: 3px;
  background: #c5c1ad;
}
.character-antenna i {
  position: absolute;
  top: -6px;
  left: -5px;
  width: 15px;
  height: 15px;
  border-radius: 50%;
  background: #d97857;
  border: 3px solid #f0cfb6;
  box-shadow: 0 2px 4px #c7895620;
}
.character-cheek {
  position: absolute;
  top: 88px;
  width: 13px;
  height: 5px;
  border-radius: 50%;
  background: #da8d7250;
}
.cheek-left {
  left: 24px;
}
.cheek-right {
  right: 24px;
}
.character-badge {
  position: absolute;
  left: 71px;
  bottom: 7px;
  font-size: 10px;
  color: #b5ac8f;
}
.character-shadow {
  position: absolute;
  left: 46px;
  top: 156px;
  width: 112px;
  height: 10px;
  border-radius: 50%;
  background: #67714f20;
  filter: blur(5px);
  animation: shadow 4s ease-in-out infinite;
}
.listening .character-aura {
  border: 1px solid #d17b5850;
  animation: listen 1.8s ease-out infinite;
}
.listening .character-antenna i {
  background: #e36645;
  box-shadow: 0 0 0 5px #e3664515;
}
.speaking .character-mouth {
  animation: talk 0.3s ease-in-out infinite alternate;
}
.speaking .character-body {
  animation-duration: 1.5s;
}
.thinking .character-antenna i {
  animation: glow 1s infinite alternate;
}
.character-thought {
  position: absolute;
  right: 9px;
  top: 7px;
  font-size: 25px;
  letter-spacing: 2px;
  color: #8b9b77;
  animation: glow 1s infinite alternate;
}
.character[data-emotion="happy"] .character-eye,
.character[data-emotion="laughing"] .character-eye {
  height: 11px;
  top: 25px;
  background: none;
  border-top: 3px solid #d7e6b5;
  border-radius: 50%;
  animation: none;
}
.character[data-emotion="sad"] .character-mouth {
  transform: rotate(180deg);
  top: 49px;
}
.character[data-emotion="sleepy"] .character-eye {
  height: 4px;
  top: 30px;
  animation: none;
}
@keyframes drift {
  50% {
    transform: translateY(-7px) rotate(2deg);
  }
}
@keyframes shadow {
  50% {
    transform: scaleX(0.85);
    opacity: 0.6;
  }
}
@keyframes blink {
  0%,
  43%,
  47%,
  100% {
    transform: scaleY(1);
  }
  45% {
    transform: scaleY(0.1);
  }
}
@keyframes talk {
  to {
    width: 13px;
    height: 12px;
    left: 54px;
    background: #c2d8a0;
    border-radius: 50%;
  }
}
@keyframes glow {
  to {
    opacity: 0.4;
  }
}
@keyframes listen {
  to {
    transform: scale(1.15);
    opacity: 0;
  }
}
@media (prefers-reduced-motion: reduce) {
  *,
  *::before,
  *::after {
    animation: none !important;
  }
}
</style>
