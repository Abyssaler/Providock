import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import AccountsPage from "../src/features/accounts/AccountsPage";

/**
 * 渲染冒烟：确认账户页在无 Tauri 运行时（浏览器/服务端渲染）也能挂载，
 * 初始状态是「读取中…」而不是假数据或零值仪表盘（UI_UX 第 8 节）。
 */
describe("AccountsPage 渲染冒烟", () => {
  it("初始渲染读取中，不显示任何余额数字", () => {
    const html = renderToStaticMarkup(<AccountsPage />);
    expect(html).toContain("账户与额度");
    expect(html).toContain("读取中…");
    expect(html).toContain("添加账户");
    expect(html).toContain("全部刷新");
    expect(html).not.toContain("可用余额");
  });
});
