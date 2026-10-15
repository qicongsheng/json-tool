<template>
  <div class="pane-head">
    <span class="tag">{{ t("treeTag") }}</span>
    <span class="spacer"></span>
    <button class="tree-ctl-btn" :title="t('expandAll')" :aria-label="t('expandAll')" @click="expandAll">
      <!-- 展开：两个 chevron 背向张开（上朝上 / 下朝下，↕）-->
      <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6"
           stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M3 6.5 L8 2.5 L13 6.5" />
        <path d="M3 9.5 L8 13.5 L13 9.5" />
      </svg>
    </button>
    <button class="tree-ctl-btn" :title="t('collapseAll')" :aria-label="t('collapseAll')" @click="collapseAll">
      <!-- 收缩：两个 chevron 相向合拢（上朝下 / 下朝上，上下半区不交叉）-->
      <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6"
           stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M3 2.5 L8 6.5 L13 2.5" />
        <path d="M3 13.5 L8 9.5 L13 13.5" />
      </svg>
    </button>
  </div>
  <div ref="bodyEl" class="pane-body" @contextmenu.capture="openMenu">
    <NTree
      ref="treeRef"
      block-line
      :animated="false"
      :data="data"
      virtual-scroll
      selectable
      :selected-keys="selectedKeys"
      :expanded-keys="expandedKeys"
      :node-props="nodeProps"
      :on-load="onLoad"
      :render-label="renderLabel"
      @update:selected-keys="onSelect"
      @update:expanded-keys="(k: (string | number)[]) => (expandedKeys = k)"
      style="height: 100%"
    />
    <NDropdown
      trigger="manual"
      placement="bottom-start"
      :show="menu.show"
      :x="menu.x"
      :y="menu.y"
      :options="menuOptions"
      @select="onMenuSelect"
      @clickoutside="closeMenu"
    />
  </div>
</template>

<script setup lang="ts">
import { computed, h, inject, nextTick, reactive, ref, watch, type VNode } from "vue";
import { NTree, NDropdown, createDiscreteApi, darkTheme, type TreeOption } from "naive-ui";
import type { Ref } from "vue";
import { api, type Locate, type NodeDto } from "../api";
import { t } from "../i18n";

// 独立 message 实例（App 内未包 NMessageProvider，与 Toolbar/TableView 同模式）；
// discrete 弹层脱离组件树，须自带 theme 跟随明暗
const theme = inject<Ref<"light" | "dark">>("theme")!;
const { message } = createDiscreteApi(["message"], {
  configProviderProps: computed(() => ({
    theme: theme.value === "dark" ? darkTheme : undefined,
  })),
});

// ── 注入（setup 顶部）─────────────────────
const locateTo = inject<(l: Locate) => void>("locateTo")!;
const openTable = inject<(nodeId: number, path: string) => void>("openTable")!;

const props = defineProps<{ rootId: number }>();
const emit = defineEmits<{ (e: "select", n: NodeDto): void }>();

const data = ref<Opt[]>([]);
const treeRef = ref<InstanceType<typeof NTree>>();

interface Opt extends TreeOption { node: NodeDto }

// childId → parentId 映射：原文→树选中时沿路径逐层展开
const parentOf = new Map<number, number | null>();
const optOf = new Map<number, Opt>();

function toOption(n: NodeDto, parentId: number | null): Opt {
  const preview = n.hasChildren
    ? (n.kind === "arr" ? ` [${n.len}]` : ` {${n.len}}`)
    : `: ${n.literal ?? "null"}`;
  parentOf.set(n.id, parentId);
  const opt: Opt = {
    key: n.id,
    label: `${n.key ?? "$"}${preview}`,
    isLeaf: !n.hasChildren,
    node: n,
  };
  optOf.set(n.id, opt);
  return opt;
}

const selectedKeys = ref<(string | number)[]>([]);
const expandedKeys = ref<(string | number)[]>([]);

// ── 类型图标（对齐 docs/prototypes/json-tool-ui.html 的 .icon 配色）──
const KIND_ICONS: Record<NodeDto["kind"], string> = {
  obj: "📁", arr: "📊", str: "📄", num: "🔢", bool: "🔘", null: "∅",
};

function renderLabel({ option }: { option: TreeOption }): VNode {
  const opt = option as Opt;
  const n = opt.node;
  if (!n) return h("span", String(option.label ?? ""));
  const preview = n.hasChildren
    ? (n.kind === "arr" ? ` [${n.len}]` : ` {${n.len}}`)
    : `: ${n.literal ?? "null"}`;
  // 悬停显示完整内容：叶子 = 完整 literal（单行省略可能截断长值），容器 = 长度预览
  const fullText = n.hasChildren ? preview : (n.literal ?? "null");
  return h("span", { class: "tree-label", title: fullText }, [
    h("i", { class: `tree-icon k-${n.kind}` }, KIND_ICONS[n.kind]),
    h("span", { class: n.key === null ? "tree-root" : "tree-key" }, n.key ?? "$"),
    h("span", { class: "tree-preview" }, preview),
  ]);
}

// rootKind 由后端返回（load_text 的 rootKind），数组根据此显示 [n] 并开放「转为表格」菜单
async function reload(rootId: number, rootKind: NodeDto["kind"] = "obj") {
  parentOf.clear(); optOf.clear();
  const children = await api.getChildren(rootId, 0, 500);
  // 根节点也走 toOption 注册进 optOf：右键菜单靠 optOf 反查节点（缺失时右键回落浏览器原生菜单）
  const rootOpt = toOption(
    { id: rootId, key: null, kind: rootKind, len: children.length, literal: null, hasChildren: true },
    null,
  );
  rootOpt.children = children.map(c => toOption(c, rootId));
  data.value = [rootOpt];
  parentOf.set(rootId, null);
  selectedKeys.value = []; expandedKeys.value = [rootId];
}

async function onLoad(node: TreeOption): Promise<void> {
  if (node.children?.length || node.isLeaf) return;
  const children = await api.getChildren(node.key as number, 0, 500);
  (node as Opt).children = children.map(c => toOption(c, node.key as number));
}

// 清空（工具栏「清空」）：清掉树数据与选中态，回到空态提示（rootId=0 时不重拉）
function clear() {
  data.value = [];
  selectedKeys.value = [];
  expandedKeys.value = [];
}

// ── 全展开 / 全收缩（pane-head 图标按钮）──
// 深度优先把整棵树拉到内存并收集所有可展开（非叶子）节点的 key，一次性全展开。
// 两层防护：
// 1) onlyLoaded（optOf > 2000，防 onLoad 级联拉全树拖垮后端）：直接跳过未加载节点；
// 2) 拉取预算：走树期间新拉取节点数超过 5000 即停止向深层拉取，只展开已加载部分并提示。
// expanding 重入保护：走树在途时忽略再次点击，防止并发两趟 walk 交错拉取。
let expanding = false;
const EXPAND_FETCH_BUDGET = 5000;

async function expandAll() {
  if (expanding || !data.value.length) return;
  expanding = true;
  try {
    const onlyLoaded = optOf.size > 2000;
    const keys: (string | number)[] = [];
    let fetched = 0;      // 本次走树新拉取的节点数（预算计数）
    let overBudget = false;
    async function walk(opts: Opt[]): Promise<void> {
      for (const o of opts) {
        if (o.isLeaf) continue;
        if (!o.children?.length) {
          if (onlyLoaded) continue;
          if (overBudget) continue; // 超预算：不再向深层拉取，只展开已加载的部分
          if (fetched >= EXPAND_FETCH_BUDGET) { overBudget = true; continue; }
          // 未加载节点：主动补齐 children（与 onLoad 同源），才能继续向下收集
          const children = await api.getChildren(o.node.id, 0, 500);
          fetched += children.length;
          o.children = children.map(c => toOption(c, o.node.id));
        }
        if (o.key !== undefined) keys.push(o.key);
        await walk(o.children as Opt[]);
      }
    }
    await walk(data.value);
    expandedKeys.value = keys;
    if (onlyLoaded) message.warning(t("warnPartExpanded"));
    else if (overBudget) message.warning(t("warnPartExpandedHint"));
  } finally {
    expanding = false;
  }
}

function collapseAll() {
  expandedKeys.value = [];
}

function onSelect(_keys: (string | number)[], option: (TreeOption | null)[] | TreeOption | null) {
  const opt = (Array.isArray(option) ? option[0] : option) as Opt | null | undefined;
  if (!opt) return;
  const n = opt.node;
  emit("select", n);
  api.locate(n.id).then(loc => locateTo(loc));
}

// 原文 → 树：自根向下沿路径展开，路径上有懒加载节点就补齐 children。
// 深层节点（未在树中懒加载过，parentOf 无记录）走后端祖先链重建路径。
async function selectNode(id: number) {
  const rootOpt = data.value[0];
  if (!rootOpt) return;
  let path: number[] = [];
  let cur: number | null = id;
  while (cur !== null && cur !== undefined) { path.unshift(cur); cur = parentOf.get(cur) ?? null; }
  if (path[0] !== rootOpt.key || path[path.length - 1] !== id) {
    // 本地链不完整（深层未加载节点）：向后端取根→目标的完整 id 链
    path = await api.nodeAncestors(id).catch(() => [] as number[]);
    if (path[0] !== rootOpt.key || path[path.length - 1] !== id) return; // 目标不在当前树中
  }
  let opt = rootOpt;
  for (let i = 1; i < path.length; i++) {
    if (!opt.children?.length) {
      const children = await api.getChildren(opt.node.id, 0, 500);
      opt.children = children.map(c => toOption(c, opt.node.id));
    }
    const next = opt.children.find(c => c.key === path[i]) as Opt | undefined;
    if (!next) return;
    opt = next;
  }
  expandedKeys.value = path.slice(0, -1);
  selectedKeys.value = [id];
  await nextTick();
  // 虚拟滚动下视口外的节点根本不在 DOM：rAF 轮询等不到它出现（timeout 空转）。
  // 用 NTree.scrollTo({ key })（内部走 VVirtualList，可按 key 滚到视口外节点），
  // 把节点带进 DOM 后再 scrollIntoView 居中；scrollTo 不可用时才退化为单次 rAF 查 DOM。
  if (typeof treeRef.value?.scrollTo === "function") {
    treeRef.value.scrollTo({ key: id });
    await nextTick();
    requestAnimationFrame(() => {
      document.querySelector(`[data-node-id="${id}"]`)?.scrollIntoView({ block: "center" });
    });
  } else {
    requestAnimationFrame(() => {
      document.querySelector(`[data-node-id="${id}"]`)?.scrollIntoView({ block: "center" });
    });
  }
  // 程序化选中（表格行点击等）同样联动原文光束；树节点点击走 onSelect，不会重复定位
  api.locate(id).then(loc => locateTo(loc));
}

// ── 右键菜单 ─────────────────────────────
const menu = reactive({ show: false, x: 0, y: 0, node: null as NodeDto | null });

// 给每个树节点 DOM 挂 data-node-id，供 contextmenu 反查
function nodeProps(info: { option: TreeOption }) {
  const n = (info.option as Opt).node;
  return n ? { "data-node-id": String(n.id) } : {};
}

const menuOptions = computed(() => {
  const base = [] as { label: string; key: string }[];
  // 根节点无 key，不提供「复制节点名」
  if (menu.node?.key) base.push({ label: t("menuCopyKey"), key: "copy-key" });
  base.push(
    { label: t("menuCopyValue"), key: "copy-json" },
    { label: t("menuCopyPath"), key: "copy-path" },
  );
  // 任意节点都可转为表格（数组 = N 行；对象/标量 = 单行）
  base.push({ label: t("menuToTable"), key: "to-table" });
  return base;
});

function openMenu(e: MouseEvent) {
  const el = (e.target as HTMLElement).closest(".n-tree-node") as HTMLElement | null;
  const id = el?.getAttribute("data-node-id");
  const opt = id !== null && id !== undefined ? optOf.get(Number(id)) : undefined;
  if (!opt) return;
  e.preventDefault();
  menu.x = e.clientX; menu.y = e.clientY;
  menu.node = opt.node;
  menu.show = true;
  // 滚动 / 再次右键时关闭（点击由 NDropdown clickoutside 处理）
  document.addEventListener("scroll", closeMenu, { capture: true, once: true });
  document.addEventListener("contextmenu", closeMenu, { capture: true, once: true });
}

function closeMenu() { menu.show = false; }

// 由 parentOf/optOf 反推 JSONPath（$ / .key / [i]）
// 数组子节点的 key 是解析器合成的下标字符串，段格式由父节点 kind 决定：
// 父为数组 → `[i]`（无点号），父为对象 → `.key`
function pathOf(id: number): string {
  const segs: string[] = [];
  let cur: number | null = id;
  while (cur !== null && cur !== undefined) {
    const opt = optOf.get(cur);
    if (!opt) break;
    const pid: number | null = parentOf.get(cur) ?? null;
    if (pid !== null && opt.node.key != null) {
      const parentKind = optOf.get(pid)?.node.kind;
      segs.unshift(parentKind === "arr" ? `[${opt.node.key}]` : `.${opt.node.key}`);
    }
    cur = pid;
  }
  return `$${segs.join("")}`;
}

async function onMenuSelect(key: string | number) {
  closeMenu();
  const n = menu.node;
  if (!n) return;
  if (key === "copy-key") {
    await navigator.clipboard.writeText(n.key ?? "");
  } else if (key === "copy-json") {
    await navigator.clipboard.writeText(await api.nodeText(n.id));
  } else if (key === "copy-path") {
    await navigator.clipboard.writeText(pathOf(n.id));
  } else if (key === "to-table") {
    openTable(n.id, pathOf(n.id));
  }
}

defineExpose({ reload, selectNode, clear });

// 兜底：仅通过 rootId prop 变化的调用方也能重建树（正常路径走 expose 的 reload）
watch(() => props.rootId, (id) => { if (id) reload(id); });
</script>
