import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import SessionsPage from "../src/features/sessions/SessionsPage";

/**
 * 渲染冒烟：会话页在无 Tauri 运行时（浏览器预览/服务端渲染）也能挂载，
 * 初始状态是筛选器 + 空态说明，不显示假数据或 0 值仪表盘（UI_UX 第 8 节）。
 */
describe("SessionsPage 渲染冒烟", () => {
  it("初始渲染筛选与空态说明，不显示伪正文", () => {
    const html = renderToStaticMarkup(<SessionsPage />);
    expect(html).toContain("会话");
    expect(html).toContain("搜索元数据");
    expect(html).toContain("暂无会话索引");
    expect(html).toContain("隐私显示");
    // 未选中会话时不出现抽屉与正文开关
    expect(html).not.toContain("对话查看未开启");
    expect(html).not.toContain("Token 小计 ·");
  });
});
