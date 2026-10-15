# JSON Tool

Rust + Tauri 2 桌面端 JSON 工具：加载、格式化、树/原文双向定位、表格化查看与过滤。

## 功能

- **加载**：拖拽文件到窗口任意位置、「打开...」选择、原文编辑器直接粘贴
- **格式化**：单击执行（2 空格缩进），自动去转义（`\"` 等）并**展开嵌套 JSON 字符串**为真实结构；「压缩」输出单行
- **JSON 树**：懒加载 + 虚拟滚动，类型图标，全展开/收缩；点击节点定位原文
- **原文**：可编辑（防抖自动重解析）、只读高亮、点击 key/value 反查树节点、Ctrl+F 搜索
- **表格**：右键任意节点转为表格（数组 = N 行，对象/标量 = 单行）；**模糊过滤**（包含、忽略大小写，命中对象/数组内的 JSON 内容），支持多值（逗号/空格/Tab/换行分隔）与列级过滤；过滤后自动横滚到首个命中列
- **选择与复制**：表头点击选列、行号点击选行（Ctrl/Shift 多选），Ctrl+C 复制 TSV（行列交集，Excel 可直接粘贴）
- **行转列 / 嵌套下钻**：右键行转置为键值视图；嵌套数组/对象单元格可下钻建表，「← 返回」逐层回退
- **导出 Excel**：导出当前筛选结果，数字写数值类型
- 原文/表格非法输入精确到行列提示；明暗主题；面板分隔线可拖拽

## 性能

50MB / 10 万行 JSON（release 构建）：解析 **633ms**、过滤 **71ms**；树与表格虚拟渲染 + 按需分块取数，大文档不卡 UI。

## 构建与运行

依赖：Node.js ≥ 18、Rust stable（MSVC）、Windows 11（WebView2 系统自带）。

```bash
npm install

# 开发模式（热重载）
npm run tauri dev

# 构建发布版 exe
npm run tauri build
# 产物：src-tauri/target/release/json-tool.exe
```

## 测试

```bash
cd src-tauri
cargo test                                            # 全部单元/集成测试（33 个）
cargo test --release --test bench_large -- --ignored --nocapture   # 50MB 性能基准
```

## 项目结构

```
src-tauri/src/
├── parser.rs      # 带 span 的 JSON 解析器（节点树 + 字节区间，一次遍历）
├── formatter.rs   # 格式化 / 压缩 / 去转义 / 嵌套 JSON 递归展开
├── table.rs       # 表格引擎：列并集 / 分块取行 / 模糊过滤 / 行转列 / 导出
├── state.rs       # 会话状态（原文 + 解析树 + 表格缓存）
└── lib.rs         # Tauri 命令层（17 个命令）
src/
├── App.vue        # 布局、主题、拖拽/粘贴加载、编辑防抖
├── api.ts         # Tauri 命令封装（前后端契约）
└── components/    # Toolbar / JsonTree / RawView / TableView
docs/
├── specs/               # 详细设计文档（与实现同步）
├── prototypes/          # UI 交互原型（HTML）
└── samples/             # 复杂 JSON 测试样例
```

设计细节见 [docs/specs/详细设计文档.md](docs/specs/详细设计文档.md)。
