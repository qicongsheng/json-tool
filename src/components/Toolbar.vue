<template>
  <div class="toolbar">
    <span class="logo"><span class="brace">{</span> jsontool <span class="brace">}</span></span>
    <button class="btn" @click="pickFile">{{ t("open") }}</button>
    <button class="btn" :disabled="!hasDoc" @click="onClear">{{ t("clear") }}</button>
    <span class="spacer"></span>
    <!-- 语言切换：显示待切换目标（中文时点 EN 切英文）；放主题切换按钮左边 -->
    <button class="btn" @click="toggleLocale">{{ locale === "zh" ? "English" : "中文" }}</button>
    <button class="btn" @click="$emit('toggle-theme')">{{ theme === "light" ? "🌙" : "☀" }}</button>
  </div>
</template>

<script setup lang="ts">
import { locale, t } from "../i18n";

const props = defineProps<{ hasDoc: boolean; theme: "light" | "dark" }>();
const emit = defineEmits<{
  (e: "toggle-theme"): void;
  (e: "loaded", text: string): void;
  (e: "cleared"): void;
}>();

function toggleLocale() {
  locale.value = locale.value === "zh" ? "en" : "zh";
}

async function pickFile() {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const f = await open({ multiple: false, filters: [{ name: "JSON", extensions: ["json", "txt"] }] });
  if (typeof f === "string") {
    const text = await (await import("@tauri-apps/plugin-fs")).readTextFile(f);
    emit("loaded", text);
  }
}

// 清空：不发后端命令，仅重置前端状态（由 App 处理）
function onClear() {
  emit("cleared");
}
</script>
