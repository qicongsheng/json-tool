<script setup lang="ts">
import { computed, h, inject, nextTick, onBeforeUnmount, onMounted, ref, type Ref } from "vue";
import { NButton, NCheckbox, NDataTable, NDropdown, NInput, NPopover, createDiscreteApi, darkTheme, type DataTableColumns } from "naive-ui";
import { api, splitTokens, type Locate, type PathValue, type Row, type TableInfo } from "../api";
import { t } from "../i18n";
import { naiveOverrides } from "../naive-theme";

// App 注入的主题：discrete message 脱离组件树，须自带 theme 才跟随明暗
const theme = inject<Ref<"light" | "dark">>("theme")!;
const { message } = createDiscreteApi(["message"], {
  configProviderProps: computed(() => ({
    theme: theme.value === "dark" ? darkTheme : undefined,
  })),
});

// 表格失效类提示（Rust 端 STALE_TABLE「表格已失效…」/「节点不存在」）走黄色警告：
// 数据过期可修复重试，非真正错误；JSON 解析失败等仍由调用方用红色 message.error
function tableErr(e: unknown) {
  const s = String(e);
  if (s.includes("已失效") || s.includes("节点不存在")) message.warning(s);
  else message.error(s);
}

// hasDoc：文档是否已加载——决定细栏常驻引导态的文案（无文档 / 有文档未建表）
defineProps<{ hasDoc: boolean }>();

const CHUNK = 50; // 每次取 50 行（首次与「加载更多」一致）：大表一次拉 500 行渲染卡顿

// 表格主题覆盖：复用全局 naiveOverrides（含 DataTable 密度/配色）。
// 注意：naive 的 themeOverrides 不认 var()/color-mix（解析失败会整体回落浅色默认），
// 色值必须用具体色（见 naive-theme.ts），别改回 CSS 变量。
const tableThemeOverrides = computed(() => naiveOverrides(theme.value));

const table = ref<TableInfo | null>(null);
const rows = ref<Row[]>([]);                          // 浏览态已取回的行（分块缓存，仅未过滤时展示）
const hitRows = ref<Row[]>([]);                       // 过滤态：仅按命中索引拉取的行
const hitSet = ref<Set<number> | null>(null);         // 命中行索引（全表 0-based）；null = 未过滤
const filterCol = ref<string | null>(null);
const input = ref("");
const transposed = ref<PathValue[] | null>(null);
const transposedTitle = ref("");
const searching = ref(false);
const loadingMore = ref(false);             // 手动「加载更多」中

let debounceTimer: ReturnType<typeof setTimeout>;
let filterSeq = 0; // 过滤请求序号，防止乱序覆盖

// 建表时记录的数组路径（$.users 形式）：编辑重解析后按路径重挂表格并重放过滤
let lastPath: string | null = null;

// 下钻栈：单元格「转为表格」时压入当前表全部状态，「← 返回」弹出恢复。
// 编辑重解析只刷新栈顶；文档替换时隐藏面板仅标记失效（下层表 id 已随重解析全部失效）
interface StackEntry {
  info: TableInfo; lastPath: string | null;
  input: string; filterCol: string | null;
  hitSet: Set<number> | null; rows: Row[]; hitRows: Row[];
  colWidths: Record<string, number>;
}
const tableStack = ref<StackEntry[]>([]);

async function create(nodeId: number, path: string) {
  let t: Awaited<ReturnType<typeof api.tableCreate>>;
  try {
    t = await api.tableCreate(nodeId);
  } catch (e) {
    tableErr(e);
    return;
  }
  if (!t) return;
  lastPath = path;
  tableStack.value = []; // 新建主表：清空下钻栈
  filterSeq++; // 作废在途过滤：避免旧表的过滤结果写入新表 hitSet
  table.value = t;
  rows.value = await api.getRows(t.id, 0, CHUNK);
  hitRows.value = [];
  hitSet.value = null;
  input.value = "";
  filterCol.value = null;
  transposed.value = null;
  transposedTitle.value = "";
  colWidths.value = {}; // 新表恢复默认列宽
  selectedCols.value = []; // 新表清空列选中
  visibleCols.value = null; // 列显示筛选同步重置
  selectedRows.value = []; // 新表清空行选中
  stale.value = false; // 新表数据来自当前文档，无失效可言
  if (hidden.value) setHidden(false); // 新表打开时恢复显示与高度（隐藏不销毁状态，换表才复位）
}

// 单元格下钻：压栈当前表 → 内层节点建新表（过滤条件重置）
async function drillTo(nodeId: number) {
  const cur = table.value;
  if (!cur) return;
  tableStack.value.push({
    info: cur, lastPath, input: input.value, filterCol: filterCol.value,
    hitSet: hitSet.value, rows: rows.value, hitRows: hitRows.value,
    colWidths: { ...colWidths.value },
  });
  let t: Awaited<ReturnType<typeof api.tableCreate>>;
  try {
    t = await api.tableCreate(nodeId);
  } catch (e) {
    tableStack.value.pop(); // 建表失败：移除幻影栈项，保持下钻前状态
    tableErr(e);
    return;
  }
  if (!t) { tableStack.value.pop(); return; }
  lastPath = t.path;
  filterSeq++;
  table.value = t;
  rows.value = await api.getRows(t.id, 0, CHUNK);
  hitRows.value = [];
  hitSet.value = null;
  input.value = "";
  filterCol.value = null;
  transposed.value = null;
  transposedTitle.value = "";
  colWidths.value = {};
  selectedCols.value = []; // 下钻新表清空列选中
  visibleCols.value = null; // 列显示筛选同步重置
  selectedRows.value = []; // 下钻新表清空行选中
  stale.value = false;
  if (hidden.value) setHidden(false);
}

// 返回上一层：恢复压栈状态并按恢复后的模式重新取行（浏览态分块 / 过滤态按索引）
async function goBack() {
  const prev = tableStack.value.pop();
  if (!prev || !table.value) return;
  filterSeq++;
  table.value = prev.info;
  lastPath = prev.lastPath;
  input.value = prev.input;
  filterCol.value = prev.filterCol;
  hitSet.value = prev.hitSet;
  colWidths.value = prev.colWidths;
  selectedCols.value = []; // 换表清空列选中
  visibleCols.value = null; // 列显示筛选同步重置
  selectedRows.value = []; // 换表清空行选中
  transposed.value = null;
  transposedTitle.value = "";
  if (prev.hitSet === null) {
    hitRows.value = [];
    try {
      rows.value = await api.getRows(prev.info.id, 0, CHUNK);
    } catch (e) {
      rows.value = [];
      tableErr(e);
      // 旧表 id 已失效（如编辑重解析后返回下层）：隐藏并标记 stale（不销毁状态），
      // 恢复时按 lastPath（下层表路径）重挂重验
      stale.value = true;
      setHidden(true);
      return;
    }
  } else {
    rows.value = prev.rows;
    try {
      hitRows.value = await api.getRowsByIndices(prev.info.id, [...prev.hitSet].slice(0, CHUNK));
    } catch (e) {
      hitRows.value = [];
      tableErr(e);
    }
  }
  if (hidden.value) setHidden(false);
}

// 文档整体替换（新加载 / 格式化 / 压缩 / 编辑重解析）后由 App 调用：
// 面板可见且有 lastPath → 立即按路径重挂并重放过滤（保持打开）；
// 否则（隐藏态或无表）只标记数据失效，恢复显示时再重验，避免用失效 tableId 调后端命令。
// Rust 表 map 已随文档加载清空，重验经 table_reopen 重建——面板状态不销毁
async function onDocReplaced() {
  if (!hidden.value && lastPath) {
    stale.value = false;
    await revalidate();
  } else {
    stale.value = true;
  }
}

// 工具栏「清空」：lastPath 随文档一起失效，重验必失败 → 直接清空数据并隐藏（跳过 revalidate）。
// 面板组件本身常驻：回到「无文档」细栏引导态
function onDocCleared() {
  lastPath = null;
  tableStack.value = [];
  table.value = null;
  rows.value = [];
  hitRows.value = [];
  hitSet.value = null;
  input.value = "";
  filterCol.value = null;
  transposed.value = null;
  transposedTitle.value = "";
  colWidths.value = {};
  selectedCols.value = [];
  selectedRows.value = [];
  visibleCols.value = null;
  stale.value = true;
  setHidden(true); // 隐藏（下次展开回默认高度：窗口高度 1/3）
}

// 文档重解析后的表格保活：只重挂栈顶表（下层表的 tableId 已随重解析全部失效，
// 返回到下层时会因 id 失效报错——语义上可接受，报告注明）；
// 路径失效 → 隐藏并标记 stale（不销毁状态，恢复时可按路径重试）。返回 true = 重验成功
async function revalidate(): Promise<boolean> {
  if (!lastPath) { stale.value = true; setHidden(true); return false; }
  const tokens = splitTokens(input.value);
  const seq = ++filterSeq;
  try {
    const r = await api.tableReopen(lastPath, tokens, filterCol.value);
    if (seq !== filterSeq) return false;
    table.value = r.info;
    transposed.value = null;
    transposedTitle.value = "";
    selectedCols.value = []; // 重挂表清空列选中
    visibleCols.value = null; // 列显示筛选同步重置
    selectedRows.value = []; // 重挂表清空行选中
    if (tokens.length === 0) {
      // 浏览态：重取前 500 行缓存
      hitSet.value = null;
      hitRows.value = [];
      const got = await api.getRows(r.info.id, 0, CHUNK);
      if (seq !== filterSeq) return false;
      rows.value = got;
    } else {
      // 过滤态：重放命中集合并按索引拉取命中行（>10000 行等错误同 onInput 处理）
      hitSet.value = new Set(r.hits);
      rows.value = [];
      try {
        const got = await api.getRowsByIndices(r.info.id, r.hits.slice(0, CHUNK));
        if (seq !== filterSeq) return false;
        hitRows.value = got;
        scrollToFirstHitCol(); // 命中列自动横向滚到视野（附加需求 49）
      } catch (e) {
        hitRows.value = [];
        tableErr(e);
      }
    }
    if (hidden.value) setHidden(false); // 编辑后/恢复时面板展开，让用户看到新数据
    return true;
  } catch (e) {
    if (seq !== filterSeq) return false;
    tableErr(e);
    // 栈顶路径失效：隐藏并标记 stale（状态保留，恢复时按 lastPath 重试；
    // 下层表 id 也已失效，返回下层会报「表格已失效」）
    stale.value = true;
    setHidden(true);
    return false;
  }
}

const shownRows = computed(() =>
  hitSet.value === null ? rows.value : hitRows.value);

function onInput() {
  clearTimeout(debounceTimer);
  debounceTimer = setTimeout(async () => {
    if (!table.value) return;
    const tokens = splitTokens(input.value);
    const seq = ++filterSeq;
    searching.value = true;
    try {
      if (tokens.length === 0) {
        hitSet.value = null; // 清空输入 = 取消过滤（回到浏览态缓存，不重取数据）
        hitRows.value = [];
        return;
      }
      const hit = await api.tableFilter(table.value.id, tokens, filterScope.value);
      if (seq !== filterSeq) return;
      // 零命中保留空 set 语义（显示「命中 0 行」），只有取消过滤才是 null
      hitSet.value = new Set(hit);
      // 命中行分块拉取（与浏览态同款）：宽匹配（如单字母）可能命中数千行，
      // 全量拉取的 IPC 序列化 + 前端物化会卡死界面——先取前 CHUNK 行
      try {
        hitRows.value = hit.length
          ? await api.getRowsByIndices(table.value.id, hit.slice(0, CHUNK))
          : [];
      } catch (e) {
        hitRows.value = [];
        tableErr(e);
        return;
      }
      if (seq !== filterSeq) return;
      scrollToFirstHitCol(); // 命中列自动横向滚到视野（附加需求 49）
    } finally {
      if (seq === filterSeq) {
        searching.value = false;
      }
    }
  }, 500);
}

// ✕ 清空过滤：置空输入并走 onInput 防抖（tokens 为空 → hitSet 回到未过滤态）
function clearFilter() {
  clearTimeout(debounceTimer);
  input.value = "";
  onInput();
}

// ── 过滤命中后自动横向滚动到第一个命中列 ──
// 命中判断与单元格 ✓ 标记同款（大小写不敏感 contains × 命中行）；
// 用 NDataTable.scrollTo（表头/表体同步）只动横向；无命中或无横向溢出则不动
const mainTable = ref<InstanceType<typeof NDataTable> | null>(null);
async function scrollToFirstHitCol() {
  await nextTick(); // 等 hitRows 渲染 + 列宽按新数据重估后再量溢出
  // 再等两帧布局稳定：过滤态切换会重建虚拟列表滚动容器，nextTick 立即滚会被重置
  await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));
  if (!table.value || hitSet.value === null || transposed.value) return;
  const tokens = splitTokens(input.value);
  if (!tokens.length) return;
  const cols = displayCols.value; // 只在可见列中找命中列（列筛选生效时隐藏列不可见）
  const wrap = document.querySelector("#tablePanel .table-wrap") as HTMLElement | null;
  if (!wrap || scrollX.value <= wrap.clientWidth) return; // 无横向溢出
  for (let i = 0; i < cols.length; i++) {
    const col = cols[i];
    const hasHit = shownRows.value.some(r =>
      hitSet.value!.has(r.index) &&
      r.cells.some(c => c.col === col &&
        tokens.some(t => c.text.toLowerCase().includes(t.toLowerCase()))));
    if (!hasHit) continue;
    const left = indexColWidth.value +
      cols.slice(0, i).reduce((s, c) => s + (colWidths.value[c] ?? estimateColWidth(c)), 0);
    mainTable.value?.scrollTo({ left, behavior: "auto" } as ScrollToOptions);
    return;
  }
}

function setFilterCol(col: string | null) {
  filterCol.value = col;
  onInput();
}

async function loadMore() {
  const t = table.value;
  if (!t || loadingMore.value) return;
  loadingMore.value = true;
  try {
    if (hitSet.value !== null) {
      // 过滤态：按命中索引继续取下一块（Set 迭代序 = Rust 返回的命中序）
      const hits = [...hitSet.value];
      const next = hits.slice(hitRows.value.length, hitRows.value.length + CHUNK);
      if (next.length) hitRows.value.push(...await api.getRowsByIndices(t.id, next));
    } else {
      const chunk = await api.getRows(t.id, rows.value.length, CHUNK);
      rows.value.push(...chunk);
    }
  } finally {
    loadingMore.value = false;
  }
}

async function transpose(rowIndex: number) {
  if (!table.value) return;
  const kv = await api.rowTranspose(table.value.id, rowIndex);
  // 列显示筛选生效时只转可见列的字段（path 首段 = 列名）；纯视图层，导出仍含全部
  const shown = visibleCols.value === null
    ? kv
    : kv.filter(p => displayCols.value.includes(p.path.split(/[.\[]/, 1)[0]));
  // 空结果明确提示，避免渲染成空白表格（如索引越界 / 行无可展平字段）
  if (!shown.length) { message.warning(t("msgNoFields", { n: rowIndex })); return; }
  selectedCols.value = []; // 转置视图无列选中语义
  selectedRows.value = []; // 转置视图无行选中语义
  transposed.value = shown;
  transposedTitle.value = t("transposeTitle", { n: rowIndex, k: shown.length });
}

// 合并后的「← 返回」：转置视图优先返回主表格；否则下钻栈非空时返回上一层
function backOne() {
  if (transposed.value) {
    transposed.value = null;
    transposedTitle.value = "";
    return;
  }
  if (tableStack.value.length) goBack();
}

// ── 导出 Excel（遵循当前过滤 + 列过滤范围）──
async function exportExcel() {
  const tb = table.value; // 局部名避开 i18n 的 t()
  if (!tb) return;
  const { save } = await import("@tauri-apps/plugin-dialog");
  const path = await save({ filters: [{ name: "Excel", extensions: ["xlsx"] }] });
  if (typeof path !== "string" || !path) return;
  try {
    await api.tableExport(tb.id, path, splitTokens(input.value), filterScope.value);
    message.success(t("msgExported", { n: hitSet.value ? hitSet.value.size : tb.rowCount }));
  } catch (e) {
    tableErr(e);
  }
}

// ── 面板显示 / 隐藏（永不销毁表格状态，区别于旧「收起」的销毁语义）──
// hidden 默认 true：启动即细栏态；建表 / 下钻 / 恢复时展开。
// stale：文档被替换后隐藏面板中的数据已过期，恢复时先按 lastPath 重验再显示。
// 无表格数据时细栏常驻显示引导文案（empty 态），点击无效果。
const hidden = ref(true);
const stale = ref(false);
const restoring = ref(false); // stale 恢复重验中（细栏标题提示）

// 面板默认高度 = 窗口高度 1/3（打开表格 / 恢复显示 / 双击复位均回到此值）
function defaultPanelH() {
  return Math.floor(window.innerHeight / 3);
}

function setHidden(v: boolean) {
  const panel = document.getElementById("tablePanel");
  if (!panel) return;
  if (v) {
    selectedCols.value = []; // 隐藏面板取消列选中：防止全局 Ctrl+C 拦截误复制隐藏表格的 TSV
    selectedRows.value = []; // 同步取消行选中（Ctrl+C 行复制语义同理）
    panel.style.height = "32px";
  } else {
    panel.style.height = defaultPanelH() + "px";
  }
  hidden.value = v;
}

function toggleHide() {
  setHidden(!hidden.value);
}

// 细栏点击恢复：stale（文档已被替换）时先重验再显示，重验期间细栏提示「恢复中…」；
// 重验失败 → revalidate 内部 message.error 并保持隐藏 + stale，可修复原文后再试
async function restore() {
  if (!table.value) return; // 无表格数据（空态引导条 / 有文档未建表）：恢复无效果
  if (!stale.value) { setHidden(false); return; }
  restoring.value = true;
  try {
    const ok = await revalidate(); // 成功后内部展开面板
    if (ok) stale.value = false;
  } finally {
    restoring.value = false;
  }
}

// 列宽内容估算：max(表头字符数, 各行该列单元格字符数) × 7.5px/字符 + 24px 左右 padding，
// 夹在 [80, 480]——超长 JSON / 嵌套文本封顶 480（显示端已有省略号 + 悬停全文）。
// 少列时表格总宽 = Σ列宽 < 面板宽 → 右侧留空（不再由最后一列吸收剩余宽度拉满）
const CHAR_W = 7.5;
const COL_MIN_W = 80;
const COL_MAX_W = 480;

function estimateColWidth(col: string): number {
  let chars = col.length; // 表头也算内容：列名很长时不截断表头
  for (const r of shownRows.value) {
    const c = r.cells.find(c => c.col === col);
    if (c && c.text.length > chars) chars = c.text.length;
  }
  return Math.round(Math.min(COL_MAX_W, Math.max(COL_MIN_W, chars * CHAR_W + 24)));
}

// 列宽拖拽结果（用户未拖过的列用内容估算宽）；# 列不可拖（宽度按行号位数自适应）
const colWidths = ref<Record<string, number>>({});

// ── 列显示筛选（多选，纯视图层）──
// visibleCols = null 表示全部展示；勾选集合生效后 columns/横向滚动/复制范围均只含可见列。
// 换表/下钻/重挂时重置；允许全不选（只剩 # 序号列，浮层「全选」恢复）。
// 编辑作用于草稿（draftCols），浮层收起时一次性提交——面板开着时表格不跳动
const visibleCols = ref<Set<string> | null>(null);
const draftCols = ref<Set<string> | null>(null);
const colPickerShow = ref(false); // 受控显隐（「确定」需编程收起）
const colSearch = ref("");
// 打开时锁定列表高度（未过滤内容高，上限 280px），搜索过滤后弹层不再跳变
const colListEl = ref<HTMLElement | null>(null);
const colListH = ref("");
const totalCols = computed(() => table.value?.columns.length ?? 0);
const displayCols = computed(() => {
  const all = table.value?.columns ?? [];
  if (visibleCols.value === null) return all;
  return all.filter(c => visibleCols.value!.has(c));
});
const pickableCols = computed(() => {
  const q = colSearch.value.trim().toLowerCase();
  return (table.value?.columns ?? []).filter(c => !q || c.toLowerCase().includes(q));
});
// 关键字过滤范围：指定列（🔎 单列过滤）> 可见列（列显示筛选生效时只搜展示中的列）> 全部列
const filterScope = computed<string[] | null>(() => {
  if (filterCol.value) return [filterCol.value];
  if (visibleCols.value !== null) return displayCols.value;
  return null;
});
const draftCount = computed(() =>
  draftCols.value === null ? totalCols.value : (table.value?.columns ?? []).filter(c => draftCols.value!.has(c)).length);
function onColPickerShow(v: boolean) {
  colPickerShow.value = v;
  if (v) {
    draftCols.value = visibleCols.value === null ? null : new Set(visibleCols.value);
    colSearch.value = "";
    // 渲染后量取未过滤列表高度并锁定，搜索过滤时弹层尺寸稳定
    nextTick(() => {
      const h = colListEl.value?.scrollHeight ?? 0;
      if (h > 0) colListH.value = Math.min(h, 280) + "px";
    });
  } else {
    // 收起时提交：草稿即生效（点击外部 / 再点按钮 / 点确定都走这里）
    visibleCols.value = draftCols.value;
    colSearch.value = "";
    colListH.value = ""; // 解锁，下次打开按当时内容重测
  }
}
// 「确定」：提交草稿并收起浮层（与点击面板外区域等效）
function confirmCols() {
  visibleCols.value = draftCols.value;
  colSearch.value = "";
  colPickerShow.value = false;
}
function isChecked(c: string) { return draftCols.value === null || draftCols.value.has(c); }
function toggleCol(c: string, on: boolean) {
  const all = table.value?.columns ?? [];
  const cur = draftCols.value === null ? new Set(all) : new Set(draftCols.value);
  if (on) cur.add(c); else cur.delete(c);
  // 允许全不选（只剩 # 序号列，浮层「全选」一键恢复）；全选回到「未筛选」草稿语义
  draftCols.value = cur.size === all.length ? null : cur;
}
function resetCols() { draftCols.value = null; colSearch.value = ""; }
function selectAllCols() { draftCols.value = null; }
function invertCols() {
  const all = table.value?.columns ?? [];
  const cur = draftCols.value === null ? new Set(all) : new Set(draftCols.value);
  // 全选 ⇄ 全不选（0 列只渲染 # 序号列）
  draftCols.value = new Set(all.filter(c => !cur.has(c)));
}

// # 列宽度随行号位数自适应（8px/位 + 8px padding，下限 24）：rowCount 变化时 computed 自动跟随
const indexColWidth = computed(() =>
  Math.max(24, String(table.value?.rowCount ?? 0).length * 8 + 8));

const columns = computed<DataTableColumns<Row>>(() => [
  {
    title: "#", key: "index", width: indexColWidth.value,
    className: "col-index", // th/td 左右 padding 收窄到 4px（见 styles.css），行号贴边盛放
    render: (r) => h("span", { class: "dim" }, String(r.index)),
  },
  ...displayCols.value.map((col) => {
    return {
      title: col,
      key: col,
      className: [
        ...(filterCol.value === col ? ["col-filtering"] : []),
        ...(selectedCols.value.includes(col) ? ["col-selected"] : []), // Excel 式列选中高亮
      ].join(" "),
      // 省略号改 td CSS 实现 + 悬停气泡见 cellTip*：
      // naive 的 ellipsis tooltip 每个单元格渲染时做两次 getBoundingClientRect 溢出测量，
      // 34 列 × 滚动重渲染 = 每帧数百次强制重排（实测滚动长任务 200-300ms/个），故弃用
      // ellipsis: { tooltip: true } as const,
      // 全部列固定宽：内容估算优先，拖拽过的列以拖拽宽为准。
      // 不留无宽列吸收剩余空间 → 列少时表格不满铺面板，右侧留空
      width: colWidths.value[col] ?? estimateColWidth(col),
      render: (r: Row) => {
        const c = r.cells.find(c => c.col === col);
        if (!c) return "—";
        // 命中检测与后端一致：大小写不敏感的 contains；单列过滤时只高亮范围内列
        const tokens = splitTokens(input.value);
        const hit = hitSet.value !== null && hitSet.value.has(r.index) &&
          (filterCol.value === null || filterCol.value === col) &&
          tokens.some(t => c.text.toLowerCase().includes(t.toLowerCase()));
        return h("span", {
          class: hit ? "hit-cell" : "",
          style: "cursor: pointer",
          onClick: (e: MouseEvent) => { e.stopPropagation(); locateField(r.nodeId, col); },
        }, c.text + (hit ? " ✓" : ""));
      },
    };
  }),
]);

// 横向滚动总宽 = # 列 + Σ列宽（内容估算/拖拽宽）。列少时小于面板宽 → 右侧留空；
// 列多时超出面板 → 底部横向滚动条
const scrollX = computed(() =>
  indexColWidth.value + displayCols.value.reduce((s, c) => s + (colWidths.value[c] ?? estimateColWidth(c)), 0));

// ── Excel 式列选择 + Ctrl+C 复制（TSV）─────
// selectedCols 保持点击顺序；# 序号列不可选。单击 = 只选该列；Ctrl+点击 = 增/删选；
// Shift+点击 = 锚点(最近一次单击/Ctrl点击的列)到当前列的范围选择。
const selectedCols = ref<string[]>([]);
let selAnchor: string | null = null;

// 行选择（点 # 序号列）：与 selectedCols 同模式。存全表 0-based 行号，
// Shift 范围按 shownRows 展示序（过滤态/浏览态一致）
const selectedRows = ref<number[]>([]);
let selRowAnchor: number | null = null;

function headClick(e: MouseEvent) {
  if (!table.value || transposed.value) return;
  const th = (e.target as HTMLElement).closest("th");
  if (!th) return;
  const col = thToCol(th);
  if (!col) return; // "#" 序号列不参与
  const rect = th.getBoundingClientRect();
  if (e.clientX >= rect.right - 6) return; // 右缘 resizer 热区不触发选择（留给拖拽）
  const cols = table.value.columns;
  if (e.ctrlKey || e.metaKey) {
    const i = selectedCols.value.indexOf(col);
    if (i >= 0) selectedCols.value = selectedCols.value.filter(c => c !== col);
    else selectedCols.value = [...selectedCols.value, col];
    selAnchor = col;
  } else if (e.shiftKey && selAnchor && cols.includes(selAnchor)) {
    const a = cols.indexOf(selAnchor), b = cols.indexOf(col);
    const [lo, hi] = a <= b ? [a, b] : [b, a];
    selectedCols.value = cols.slice(lo, hi + 1);
  } else {
    selectedCols.value = [col];
    selAnchor = col;
  }
}

// # 序号列点击 → 选整行（Excel 式）：单击 = 只选该行；Ctrl+点击 = 增/删选；
// Shift+点击 = 锚点(最近一次单击/Ctrl点击的行)到当前行的范围选择（按展示序）。
// 由 rowClick 拦截转发（tr 级监听），不再触发 treeSelect 行选中
function rowNumClick(e: MouseEvent, index: number) {
  if (!table.value || transposed.value) return;
  const order = shownRows.value.map(r => r.index);
  if (e.ctrlKey || e.metaKey) {
    const i = selectedRows.value.indexOf(index);
    if (i >= 0) selectedRows.value = selectedRows.value.filter(x => x !== index);
    else selectedRows.value = [...selectedRows.value, index];
    selRowAnchor = index;
  } else if (e.shiftKey && selRowAnchor !== null && order.includes(selRowAnchor)) {
    const a = order.indexOf(selRowAnchor), b = order.indexOf(index);
    if (a >= 0 && b >= 0) {
      const [lo, hi] = a <= b ? [a, b] : [b, a];
      selectedRows.value = order.slice(lo, hi + 1);
      return;
    }
    selectedRows.value = [index]; // 锚点行已被过滤/换页移出展示集：退化为单击语义
    selRowAnchor = index;
  } else {
    selectedRows.value = [index];
    selRowAnchor = index;
    selectedCols.value = []; // Excel 语义：普通点行号取消列选中
  }
}

// 点击表格空白区（非 td/th）取消选中
function wrapClick(e: MouseEvent) {
  const el = e.target as HTMLElement;
  if (!el.closest("td") && !el.closest("th")) {
    selectedCols.value = [];
    selectedRows.value = [];
  }
}

// Ctrl+C：有列/行选中时复制 TSV（首行表头 + 每行数据，Tab 分隔，Excel 直接粘贴成列）。
// 行数据取当前 shownRows 缓存：过滤态 = 筛选结果（与导出语义一致）；
// 浏览态 = 已加载行（未全量加载时复制仅含已加载行）。无选中时不拦截系统默认复制。
// Excel 语义：行 + 列同时选中 → 复制交集（选中行 × 选中列的矩阵）
function onTableKeydown(e: KeyboardEvent) {
  if (e.key === "Escape" && (selectedCols.value.length || selectedRows.value.length)) {
    selectedCols.value = [];
    selectedRows.value = [];
    return;
  }
  if (!(e.ctrlKey || e.metaKey) || e.key.toLowerCase() !== "c") return;
  const el = e.target as HTMLElement | null; // 局部名避开 i18n 的 t()
  if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable)) return;
  // 用户正在选普通文字（如页面其它位置的说明文本）→ 走原生复制，不拦截
  if (window.getSelection()?.toString()) return;
  if (!selectedCols.value.length && !selectedRows.value.length) return;
  e.preventDefault();
  const cellOf = (r: Row, c: string) => r.cells.find(c2 => c2.col === c)?.text ?? "";
  const selRowData = shownRows.value.filter(r => selectedRows.value.includes(r.index));
  let lines: string[];
  let tip: string;
  if (selectedRows.value.length && selectedCols.value.length) {
    // 交集：选中行 × 选中列
    const cols = selectedCols.value;
    lines = [cols.join("\t"), ...selRowData.map(r => cols.map(c => cellOf(r, c)).join("\t"))];
    tip = t("msgCopiedIntersection", { c: cols.length, r: selRowData.length });
  } else if (selectedRows.value.length) {
    // 仅行选中：当前可见列（所见即所复）
    const cols = displayCols.value;
    lines = [cols.join("\t"), ...selRowData.map(r => cols.map(c => cellOf(r, c)).join("\t"))];
    tip = t("msgCopiedRows", { r: selRowData.length });
  } else {
    // 仅列选中：保持原语义（所有 shownRows）
    const cols = selectedCols.value;
    lines = [cols.join("\t"), ...shownRows.value.map(r => cols.map(c => cellOf(r, c)).join("\t"))];
    tip = t("msgCopiedCols", { c: cols.length, r: shownRows.value.length });
  }
  navigator.clipboard.writeText(lines.join("\n"));
  message.success(tip);
}
onMounted(() => window.addEventListener("keydown", onTableKeydown));
onBeforeUnmount(() => window.removeEventListener("keydown", onTableKeydown));

const matchInfo = computed(() => {
  if (!table.value) return "";
  const n = splitTokens(input.value).length;
  const scope = filterCol.value ? t("scopeCol", { col: filterCol.value }) : "";
  if (hitSet.value === null) return t("rowsTotal", { n: table.value.rowCount });
  // 分块拉取：未全部载入时注明已载行数
  const loaded = hitRows.value.length < hitSet.value.size
    ? ` · ${t("loadedRows", { m: hitRows.value.length })}` : "";
  return t("hitInfo", { hit: hitSet.value.size, total: table.value.rowCount, n, scope }) + loaded;
});

// ── 行右键菜单（manual NDropdown）──────────
const menuX = ref(0); const menuY = ref(0); const menuShow = ref(false);
const menuRow = ref<Row | null>(null);
// 右键落在数据单元格上时记录（# 序号列 / 非单元格区域为 null），菜单首项提供「复制Key / 复制值」
const menuCell = ref<{ col: string; text: string } | null>(null);
// 右键单元格对应的字段节点 id（有字段节点就提供「转为表格」下钻；标量下钻 = 单行表）
const menuDrillId = ref<number | null>(null);
const menuOptions = computed(() => {
  const base = [] as { label: string; key: string }[];
  if (menuCell.value) {
    base.push(
      { label: t("menuCopyKey"), key: "copy-key" },
      { label: t("menuCopyValue"), key: "copy-cell" },
    );
  }
  base.push(
    { label: t("menuCopyRowJson"), key: "c" },
    { label: t("menuTranspose"), key: "t" },
  );
  if (menuDrillId.value !== null) base.push({ label: t("menuToTable"), key: "drill" });
  return base;
});
async function rowMenu(e: MouseEvent, r: Row) {
  e.preventDefault();
  colMenuShow.value = false;
  // td → 列名：按该行内 td 序号反推（首位是 "#" 序号列），单元格完整文本取自行数据而非 DOM
  let cell: { col: string; text: string } | null = null;
  const td = (e.target as HTMLElement).closest("td");
  if (td && table.value) {
    const tds = Array.from(td.parentElement?.children ?? []).filter(el => el.tagName === "TD");
    const idx = tds.indexOf(td);
    // 表格只渲染可见列：td 序号 → displayCols（列筛选生效时 ≠ table.columns）
    const col = idx > 0 && idx - 1 < displayCols.value.length ? displayCols.value[idx - 1] : undefined;
    const c = col ? r.cells.find(c => c.col === col) : undefined;
    if (col && c) cell = { col, text: c.text };
  }
  menuCell.value = cell;
  menuDrillId.value = null;
  // 有字段节点（任意类型）即可下钻建表；查询失败/无该字段则不显示该项
  if (cell) {
    try {
      const id = await api.locateCellNode(r.nodeId, cell.col);
      if (id !== null) menuDrillId.value = id;
    } catch { /* 节点失效等：不提供下钻 */ }
  }
  menuX.value = e.clientX; menuY.value = e.clientY; menuRow.value = r; menuShow.value = true;
}
function onMenuSelect(key: string) {
  menuShow.value = false;
  if (!menuRow.value) return;
  if (key === "copy-key") {
    if (menuCell.value) navigator.clipboard.writeText(menuCell.value.col);
    return;
  }
  if (key === "copy-cell") {
    if (menuCell.value) navigator.clipboard.writeText(menuCell.value.text);
    return;
  }
  if (key === "drill") {
    if (menuDrillId.value !== null) drillTo(menuDrillId.value);
    return;
  }
  if (key === "t") transpose(menuRow.value.index);
  // 复制该行节点的原始 JSON（按 span 切片，与原文完全一致）
  if (key === "c") api.nodeText(menuRow.value.nodeId).then(t => navigator.clipboard.writeText(t));
}

// 点击行 → JSON 树选中该节点（树内部随后联动原文定位）；行点击同时取消列/行选中。
// 点在 # 序号列上 → 转发行号选行（Excel 式），不触发 treeSelect
const treeSelect = inject<(nodeId: number) => void>("treeSelect")!;
function rowClick(e: MouseEvent, r: Row) {
  const td = (e.target as HTMLElement).closest("td");
  if (td?.classList.contains("col-index")) {
    rowNumClick(e, r.index);
    return;
  }
  selectedCols.value = [];
  selectedRows.value = [];
  treeSelect(r.nodeId);
}

// 点击单元格 → 原文定位该字段（仅定位，不触发行选中）
const locateTo = inject<(l: Locate) => void>("locateTo")!;
function locateField(rowNodeId: number, col: string) {
  api.locateCell(rowNodeId, col).then(locateTo).catch(err => message.error(String(err)));
}

function rowProps(r: Row) {
  return {
    onContextmenu: (e: MouseEvent) => rowMenu(e, r),
    onClick: (e: MouseEvent) => rowClick(e, r),
    // Excel 式行选中高亮（styles.css 端 .row-selected td 上淡 primary 底）
    class: selectedRows.value.includes(r.index) ? "row-selected" : "",
    style: "cursor: pointer",
  };
}

// ── 表头右键 → 列级过滤（capture 阶段拦截 th）──
const colMenuShow = ref(false); const colMenuX = ref(0); const colMenuY = ref(0);
const colMenuCol = ref<string | null>(null);
function headMenu(e: MouseEvent) {
  if (!table.value || transposed.value) return; // 转置视图表头不弹列过滤菜单
  const th = (e.target as HTMLElement).closest("th");
  if (!th) return;
  const text = th.textContent?.trim();
  let col = table.value.columns.find(c => c === text);
  if (!col) {
    // 兜底：按 th 在表头行中的序号匹配（首位是 "#" 序号列，无数据列）；只含可见列
    const ths = Array.from(th.parentElement?.children ?? []).filter(el => el.tagName === "TH");
    const idx = ths.indexOf(th);
    if (idx > 0 && idx - 1 < displayCols.value.length) col = displayCols.value[idx - 1];
  }
  if (!col) return; // "#" 序号列不参与
  e.preventDefault();
  e.stopPropagation();
  menuShow.value = false;
  colMenuX.value = e.clientX; colMenuY.value = e.clientY; colMenuCol.value = col;
  colMenuShow.value = true;
}
const colMenuOptions = computed(() => [
  { label: t("colMenuFilterOnly", { col: colMenuCol.value }), key: "f" },
  ...(filterCol.value === colMenuCol.value ? [{ label: t("colMenuClearFilter"), key: "c" }] : []),
]);
function onColMenuSelect(key: string) {
  colMenuShow.value = false;
  setFilterCol(key === "f" ? colMenuCol.value : null);
}

// ── 列宽拖拽（th 右缘 6px 热区；# 列与转置视图不支持）──
// th → 列名：先按文本匹配，兜底按表头行序号（首位 "#" 无数据列）——与 headMenu 同策略
function thToCol(th: HTMLElement): string | null {
  if (!table.value || transposed.value) return null;
  const text = th.textContent?.trim();
  let col = table.value.columns.find(c => c === text) ?? null;
  if (!col) {
    const ths = Array.from(th.parentElement?.children ?? []).filter(el => el.tagName === "TH");
    const idx = ths.indexOf(th);
    if (idx > 0 && idx - 1 < displayCols.value.length) col = displayCols.value[idx - 1];
  }
  return col;
}

let colResize: { col: string; startX: number; startW: number } | null = null;

function colResizeDown(e: MouseEvent) {
  const th = (e.target as HTMLElement).closest("th") as HTMLElement | null;
  if (!th) return;
  const rect = th.getBoundingClientRect();
  if (e.clientX < rect.right - 6) return;          // 不在右缘热区
  const col = thToCol(th);
  if (!col) return;                                 // "#" 列固定 60
  e.preventDefault();
  e.stopPropagation();
  colResize = { col, startX: e.clientX, startW: colWidths.value[col] ?? estimateColWidth(col) };
  document.addEventListener("mousemove", colResizeMove);
  document.addEventListener("mouseup", colResizeUp);
  document.body.classList.add("col-resizing");
  document.body.style.userSelect = "none";
}

function colResizeMove(e: MouseEvent) {
  if (!colResize) return;
  const w = Math.max(60, colResize.startW + e.clientX - colResize.startX);
  colWidths.value = { ...colWidths.value, [colResize.col]: w };
}

function colResizeUp() {
  colResize = null;
  document.removeEventListener("mousemove", colResizeMove);
  document.removeEventListener("mouseup", colResizeUp);
  document.body.classList.remove("col-resizing");
  document.body.style.userSelect = "";
}

// 双击 th 右缘 = 恢复该列默认宽度
function colResizeReset(e: MouseEvent) {
  const th = (e.target as HTMLElement).closest("th") as HTMLElement | null;
  if (!th) return;
  const rect = th.getBoundingClientRect();
  if (e.clientX < rect.right - 6) return;
  const col = thToCol(th);
  if (!col) return;
  if (colWidths.value[col] === undefined) return;
  const next = { ...colWidths.value };
  delete next[col];
  colWidths.value = next;
}

// hover 右缘热区时给出 col-resize 光标提示
function colHover(e: MouseEvent) {
  const th = (e.target as HTMLElement).closest("th") as HTMLElement | null;
  if (!th || transposed.value) { if (th) th.style.cursor = ""; return; }
  const rect = th.getBoundingClientRect();
  th.style.cursor = e.clientX >= rect.right - 6 ? "col-resize" : "";
}

// ── 单元格悬停全文气泡（替代 naive ellipsis tooltip）──
// 仅在 mouseover 进入单元格时量一次溢出（span.scrollWidth vs clientWidth），
// 溢出才显示；不做渲染期测量。气泡内容取 td.textContent（CSS 省略只影响视觉）
const cellTipShow = ref(false);
const cellTipText = ref("");
const cellTipX = ref(0);
const cellTipY = ref(0);
let cellTipTd: HTMLTableCellElement | null = null;

function cellTipOver(e: MouseEvent) {
  const td = (e.target as HTMLElement).closest("td") as HTMLTableCellElement | null;
  if (!td || td.classList.contains("col-index")) { cellTipHide(); return; }
  if (td === cellTipTd) return;
  cellTipTd = td;
  // td 直接量：inline 子元素 clientWidth 恒为 0，量 td 自身
  const overflowed = td.scrollWidth > td.clientWidth + 1;
  cellTipText.value = overflowed ? (td.textContent ?? "") : "";
  cellTipShow.value = overflowed;
}
function cellTipMove(e: MouseEvent) {
  if (!cellTipShow.value) return;
  cellTipX.value = Math.min(e.clientX + 14, window.innerWidth - 500);
  cellTipY.value = Math.min(e.clientY + 16, window.innerHeight - 60);
}
function cellTipHide() {
  cellTipShow.value = false;
  cellTipTd = null;
}

// ── 面板高度拖拽（面板上边缘；双击复位）─────
function dragHDown(e: MouseEvent) {
  e.preventDefault();
  if (hidden.value) return; // 隐藏态面板高度固定，不允许拖拽
  const panel = document.getElementById("tablePanel");
  if (!panel) return;
  const move = (ev: MouseEvent) => {
    const h = Math.max(100, Math.min(innerHeight - 220, innerHeight - ev.clientY));
    panel.style.height = h + "px";
  };
  const up = () => {
    document.removeEventListener("mousemove", move);
    document.removeEventListener("mouseup", up);
    document.body.classList.remove("row-resizing");
    document.body.style.userSelect = "";
  };
  document.addEventListener("mousemove", move);
  document.addEventListener("mouseup", up);
  document.body.classList.add("row-resizing");
  document.body.style.userSelect = "none";
}
function resetH(e: MouseEvent) {
  e.preventDefault();
  const panel = document.getElementById("tablePanel");
  if (panel) panel.style.height = defaultPanelH() + "px";
}

// 行转列两列：字段列定宽 240（长展平路径不换行省略号），值列占满剩余；
// ellipsis tooltip 与主网格一致（CSS 端 table-layout: fixed 保证列宽不被内容撑破）
const transposeColumns: DataTableColumns<PathValue> = [
  { title: "字段", key: "path", className: "mono", width: 240 },
  { title: "值", key: "value" },
];

defineExpose({ create, revalidate, onDocReplaced, onDocCleared });
</script>

<template>
  <!-- 面板永不销毁：常驻细栏（collapsed=32px）。empty=无表格数据（引导态，点击无效）；
       有表时 hidden 仅折叠细栏，状态保留，恢复时按 stale 先重验 -->
  <div id="tablePanel" :class="{ collapsed: hidden || !table, empty: !table }">
    <div v-show="!hidden && table" class="splitter-h" :title="t('dragHeightTitle')"
         @mousedown="dragHDown" @dblclick="resetH"></div>
    <div class="table-head" :title="table && hidden ? t('clickRestoreTitle') : undefined" @click="hidden && restore()">
      <span class="title">
        <template v-if="!table">{{ t("tableTag") }} · {{ hasDoc ? t("tipRightClickTree") : t("tipLoadFirst") }}</template>
        <template v-else-if="hidden && restoring">{{ t("tableTag") }} · {{ t("restoringTip") }}</template>
        <template v-else-if="hidden && stale">{{ t("tableTag") }} · {{ t("staleTip") }}</template>
        <template v-else-if="hidden">{{ t("tableTitle", { n: table?.rowCount }) }} · {{ t("hiddenTip") }}</template>
        <template v-else>{{ t("tableTitle", { n: table?.rowCount }) }}</template>
      </span>
      <span v-if="filterCol && !hidden" class="col-chip" :title="t('colChipTitle')" @click="setFilterCol(null)">
        {{ t("colChip", { col: filterCol }) }}
      </span>
      <span v-show="!hidden" class="filter-box">
        <NInput v-model:value="input" type="textarea" :rows="1" :autosize="{ minRows: 1, maxRows: 4 }"
                :placeholder="t('filterPlaceholder')" @update:value="onInput" />
        <button v-if="input" class="search-clear" :title="t('clearFilterTitle')"
                tabindex="-1" @mousedown.prevent @click="clearFilter">✕</button>
      </span>
      <template v-if="!hidden">
        <span v-if="searching" class="match-info">{{ t("filtering") }}</span>
        <span v-else-if="transposed" class="match-info">{{ transposedTitle }}</span>
        <span v-else class="match-info">{{ matchInfo }}</span>
      </template>
      <!-- 列显示筛选：勾选要展示的列，至少保留一列；纯视图层（导出仍含全部列） -->
      <NPopover v-if="!hidden && table && !transposed" trigger="click" placement="bottom-end"
                :show="colPickerShow" @update:show="onColPickerShow">
        <template #trigger>
          <NButton size="small" :title="t('colsTitle')">
            {{ t("colsBtn") }}<span v-if="visibleCols !== null" class="col-count mono">{{ displayCols.length }}/{{ totalCols }}</span>
          </NButton>
        </template>
        <div class="col-picker">
          <div class="col-picker-head">
            <span>{{ t("colsTitle") }}</span>
            <span class="mono">{{ draftCount }}/{{ totalCols }}</span>
          </div>
          <NInput v-if="totalCols > 8" v-model:value="colSearch" size="tiny" :placeholder="t('colsSearch')" clearable />
          <div class="col-picker-list" ref="colListEl" :style="colListH ? { height: colListH } : undefined">
            <NCheckbox v-for="c in pickableCols" :key="c"
                       :checked="isChecked(c)"
                       :label="c" @update:checked="(v: boolean) => toggleCol(c, v)" />
          </div>
          <div class="col-picker-foot">
            <NButton text size="tiny" type="primary" @click="selectAllCols">{{ t("colsAll") }}</NButton>
            <NButton text size="tiny" type="primary" @click="invertCols">{{ t("colsInvert") }}</NButton>
            <NButton size="tiny" type="primary" @click="confirmCols">{{ t("colsOk") }}</NButton>
          </div>
        </div>
      </NPopover>
      <NButton v-show="!hidden" size="small" @click="exportExcel">{{ t("exportExcel") }}</NButton>
      <NButton v-show="!hidden" size="small" @click.stop="toggleHide">{{ t("hideBtn") }}</NButton>
      <NButton v-show="!hidden && (transposed || tableStack.length)" size="small" :title="t('backTitle')" @click="backOne">{{ t("backBtn") }}</NButton>
    </div>
    <div v-show="!hidden" class="table-wrap" @contextmenu.capture="headMenu" @click="wrapClick"
         @click.capture="headClick"
         @mousedown.capture="colResizeDown" @dblclick.capture="colResizeReset"
         @mousemove="colHover" @mouseover="cellTipOver" @mousemove.capture="cellTipMove" @mouseleave="cellTipHide">
      <NDataTable ref="mainTable" class="main-grid" v-if="!transposed" size="small" striped virtual-scroll flex-height
                  :columns="columns" :data="shownRows" :row-props="rowProps" :scroll-x="scrollX"
                  :theme-overrides="tableThemeOverrides" />
      <NDataTable class="transpose-grid" v-else size="small" striped flex-height :columns="transposeColumns" :data="transposed"
                  :theme-overrides="tableThemeOverrides" />
      <!-- 加载更多：浏览态按行序续块；过滤态按命中索引续块 -->
      <div v-if="table && !transposed && (hitSet === null
        ? rows.length < table.rowCount
        : hitRows.length < hitSet.size)" class="table-foot">
        <NButton size="tiny" :loading="loadingMore" @click="loadMore">
          {{ hitSet === null
            ? t("loadMore", { a: rows.length, b: table.rowCount })
            : t("loadMore", { a: hitRows.length, b: hitSet.size }) }}
        </NButton>
      </div>
    </div>
    <NDropdown trigger="manual" :show="menuShow" :x="menuX" :y="menuY"
               :options="menuOptions" @select="onMenuSelect" @clickoutside="menuShow = false" />
    <NDropdown trigger="manual" :show="colMenuShow" :x="colMenuX" :y="colMenuY"
               :options="colMenuOptions" @select="onColMenuSelect" @clickoutside="colMenuShow = false" />
    <!-- 单元格悬停全文气泡：pointer-events none，不干扰点击/右键 -->
    <div v-show="cellTipShow" class="cell-tip"
         :style="{ left: cellTipX + 'px', top: cellTipY + 'px' }">{{ cellTipText }}</div>
  </div>
</template>

<style scoped>
/* 过滤框（textarea）容器：相对定位承载右侧 ✕ 清空按钮（仅在有输入时渲染） */
.filter-box {
  position: relative; display: inline-flex;
  flex: 1; max-width: 480px; /* 原本由 .table-head .n-input 承担的宽度约束上移到容器 */
}
.filter-box .n-input { flex: 1; max-width: none; }
.search-clear {
  position: absolute; right: 8px; top: 50%; transform: translateY(-50%);
  width: 16px; height: 16px; padding: 0; border: none; border-radius: 3px;
  background: none; color: var(--fg-dim); font-size: 11px; line-height: 1;
  display: flex; align-items: center; justify-content: center;
  cursor: pointer; user-select: none; z-index: 1;
}
.search-clear:hover { color: var(--accent); }

/* 列显示筛选浮层：头部计数 / 搜索框 / 勾选列表 / 底部操作，跟随明暗主题 */
.col-count { opacity: 0.65; margin-left: 4px; font-size: 11px; }
.col-picker { width: 240px; display: flex; flex-direction: column; gap: 8px; }
.col-picker-head {
  display: flex; justify-content: space-between; align-items: center;
  font-size: 12px; color: var(--fg-dim);
}
.col-picker-list {
  max-height: 280px; overflow: auto;
  display: flex; flex-direction: column; gap: 2px;
  scrollbar-gutter: stable;
}
.col-picker-list :deep(.n-checkbox) { align-items: center; }
.col-picker-list :deep(.n-checkbox__label) {
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  max-width: 190px;
}
.col-picker-foot {
  display: flex; justify-content: flex-end; gap: 12px;
  border-top: 1px solid var(--border); padding-top: 6px;
}
</style>
