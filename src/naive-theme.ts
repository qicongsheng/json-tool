// Naive UI 全局主题覆盖（供 App 的 NConfigProvider 与 TableView 的 table-overrides 复用）。
// 色值与 styles.css 的 CSS 变量同源。注意：naive 的 themeOverrides 不接受 var()/color-mix
// （其内部颜色解析判非法会整体回落默认主题），故这里必须用具体色值——改色需两处同步。
import type { GlobalThemeOverrides } from "naive-ui";

interface Palette {
  border: string;
  panel: string;    // 表格所在面板底色（--panel）
  panel2: string;   // 输入框/按钮/表头底色（--panel2）
  fg: string;
  fgDim: string;
  primary: string;
  primaryHover: string;
  primaryPressed: string;
  striped: string;  // 斑马纹（ naive 不认 color-mix，用具体色）
}

const P: Record<"light" | "dark", Palette> = {
  dark: {
    border: "#2a3038",
    panel: "#1b1f26",
    panel2: "#20252e",
    fg: "#cdd6e0",
    fgDim: "#96a0ae",
    primary: "#4c8fd8",
    primaryHover: "#63a0e0",
    primaryPressed: "#3d7dc4",
    striped: "rgba(255, 255, 255, 0.03)",
  },
  light: {
    border: "#d8dde3",
    panel: "#ffffff",
    panel2: "#eef0f3",
    fg: "#242b33",
    fgDim: "#6b7683",
    primary: "#1a6bb8",
    primaryHover: "#2f7fc9",
    primaryPressed: "#155a9e",
    striped: "#fafbfc",
  },
};

export function naiveOverrides(theme: "light" | "dark"): GlobalThemeOverrides {
  const c = P[theme];
  return {
    common: {
      primaryColor: c.primary,
      primaryColorHover: c.primaryHover,
      primaryColorPressed: c.primaryPressed,
      borderColor: c.border,
      borderRadius: "6px",
    },
    // 滚动条与 CM 编辑器的自绘滚动条同色（默认 border / hover fgDim），两栏观感一致
    Scrollbar: {
      color: c.border,
      colorHover: c.fgDim,
    },
    // 输入框融入面板：panel2 底 + 边框与 .btn 同款（hover/focus 描 primary，不发光晕）
    Input: {
      color: c.panel2,
      colorFocus: c.panel2,
      border: `1px solid ${c.border}`,
      borderHover: `1px solid ${c.primary}`,
      borderFocus: `1px solid ${c.primary}`,
      boxShadowFocus: "none",
      textColor: c.fg,
      placeholderColor: c.fgDim,
    },
    // 默认按钮对齐 .btn 样式：panel2 底、hover 只描边变色
    Button: {
      color: c.panel2,
      colorHover: c.panel2,
      colorPressed: c.panel2,
      colorFocus: c.panel2,
      textColor: c.fg,
      textColorHover: c.primary,
      textColorPressed: c.primary,
      textColorFocus: c.primary,
      border: `1px solid ${c.border}`,
      borderHover: `1px solid ${c.primary}`,
      borderPressed: `1px solid ${c.primary}`,
      borderFocus: `1px solid ${c.primary}`,
    },
    DataTable: {
      borderRadius: "0",
      borderColor: c.border,
      tdColor: c.panel,
      tdColorHover: c.panel2,
      tdColorStriped: c.striped,
      thColor: c.panel2,
      thTextColor: c.fgDim,
      thFontWeight: "600",
      fontSizeSmall: "12.5px",
      thPaddingSmall: "4px 12px",
      tdPaddingSmall: "3px 12px",
    },
  };
}
