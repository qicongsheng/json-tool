<template>
  <div v-if="show" class="detect-mask" @click.self="$emit('close')">
    <div class="detect-panel">
      <div class="detect-head">
        <span class="detect-title">{{ t("detectTitle", { n: candidates.length }) }}</span>
        <span class="detect-sub">{{ t("detectSubtitle") }}</span>
        <button class="detect-close" tabindex="-1" @click="$emit('close')">Esc ✕</button>
      </div>
      <div class="detect-list">
        <div v-for="(c, i) in candidates" :key="i" class="detect-card"
             :class="{ current: i === currentIdx }" @click="$emit('load', i)">
          <span class="detect-badge">{{ c.kind === "obj" ? "{}" : "[]" }}
            <small>{{ c.kind === "obj" ? t("kindObj") : t("kindArr") }}</small></span>
          <span class="detect-meta">
            <span class="detect-info">{{ t("detectMeta", { line: c.line, len: c.charLen, depth: c.depth }) }}</span>
            <!-- preview 先 HTML 转义再包高亮 span，无注入面 -->
            <code class="detect-preview" v-html="highlight(c.preview)"></code>
          </span>
          <span class="detect-go">{{ i === currentIdx ? t("detectLoaded") : t("detectLoad") }}</span>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { onBeforeUnmount, onMounted } from "vue";
import type { Detected } from "../api";
import { t } from "../i18n";

const props = defineProps<{ show: boolean; candidates: Detected[]; currentIdx: number | null }>();
const emit = defineEmits<{ (e: "close"): void; (e: "load", idx: number): void }>();

// 预览高亮：先 HTML 转义再按 key/string/number 包 span（复用树的三色变量）
function esc(s: string): string {
  return s.replace(/[&<>]/g, ch => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[ch] as string);
}
function highlight(p: string): string {
  return esc(p)
    .replace(/"([^"]*)"(\s*:)?/g, (_m, body: string, colon: string | undefined) =>
      colon !== undefined
        ? `<span class="pv-key">"${body}"</span>${colon}`
        : `<span class="pv-str">"${body}"</span>`)
    .replace(/(?<!\w)(-?\d+(?:\.\d+)?)(?!\w)/g, '<span class="pv-num">$1</span>');
}

// Esc 关闭（不载入；可从状态栏入口重开）
function onKey(e: KeyboardEvent) {
  if (!props.show) return;
  if (e.key === "Escape") {
    e.preventDefault();
    emit("close");
  }
}
onMounted(() => window.addEventListener("keydown", onKey));
onBeforeUnmount(() => window.removeEventListener("keydown", onKey));
</script>

<style scoped>
.detect-mask {
  position: fixed; inset: 0; z-index: 1000;
  background: rgba(0, 0, 0, .45);
  display: flex; align-items: center; justify-content: center;
}
.detect-panel {
  width: min(720px, 86vw); max-height: 70vh;
  display: flex; flex-direction: column;
  background: var(--panel); border: 1px solid var(--border); border-radius: 10px;
  box-shadow: 0 12px 40px rgba(0, 0, 0, .5); overflow: hidden;
}
.detect-head { display: flex; align-items: center; gap: 8px; padding: 8px 12px; border-bottom: 1px solid var(--border); flex: none; }
.detect-title { font-weight: 600; font-size: 13px; }
.detect-sub { color: var(--fg-dim); font-size: 11.5px; }
.detect-close {
  margin-left: auto; border: 1px solid var(--border); background: var(--panel2);
  color: var(--fg-dim); border-radius: 4px; padding: 1px 7px; font-size: 11px; cursor: pointer;
}
.detect-close:hover { color: var(--fg); border-color: var(--primary); }
.detect-list { overflow: auto; padding: 8px 12px; }
.detect-card {
  display: flex; gap: 10px; align-items: center;
  border: 1px solid var(--border); border-radius: 8px;
  padding: 7px 10px; margin-bottom: 6px; cursor: pointer; background: var(--panel2);
  transition: border-color .15s, background .15s;
}
.detect-card:hover { border-color: var(--accent); background: color-mix(in srgb, var(--accent) 6%, var(--panel2)); }
.detect-card.current { box-shadow: inset 0 0 0 1px var(--accent); }
.detect-badge {
  flex: none; width: 40px; height: 40px; border-radius: 8px;
  background: var(--accent-dim); color: var(--accent);
  display: flex; flex-direction: column; align-items: center; justify-content: center;
  font-family: Consolas, monospace; font-size: 13px; font-weight: 700; line-height: 1;
}
.detect-badge small { font-size: 9px; font-weight: 400; margin-top: 2px; }
.detect-meta { flex: 1; min-width: 0; }
.detect-info { display: block; color: var(--fg-dim); font-size: 11px; margin-bottom: 2px; }
.detect-preview {
  display: block; font-family: "Cascadia Code", Consolas, monospace; font-size: 11.5px;
  color: var(--fg); white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.pv-key { color: var(--key); }
.pv-str { color: var(--str); }
.pv-num { color: var(--num); }
.detect-go { flex: none; color: var(--fg-dim); font-size: 11.5px; white-space: nowrap; }
.detect-card:hover .detect-go { color: var(--accent); }
</style>
