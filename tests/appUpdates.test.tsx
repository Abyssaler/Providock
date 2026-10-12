import { describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import AppUpdates from "../src/features/updates/AppUpdates";
import { UpdateController, type UpdateHandle } from "../src/features/updates/updateController";

function update(): UpdateHandle {
  return {
    version: "2.0.4",
    body: "A newer release",
    close: vi.fn(async () => {}),
    downloadAndInstall: vi.fn(async (onEvent) => {
      onEvent({ event: "Started", data: { contentLength: 100 } });
      onEvent({ event: "Progress", data: { chunkLength: 40 } });
      onEvent({ event: "Finished" });
    }),
  };
}

describe("签名更新控制流程", () => {
  it("发现版本只通知，点击安装后才下载，成功后重启", async () => {
    const next = update();
    const relaunch = vi.fn(async () => {});
    const check = vi.fn(async () => next);
    const client = new UpdateController({ check, relaunch });
    const states: Array<string | number | null> = [];
    client.subscribe((s) => states.push(s.phase, s.progress));
    await client.check();
    expect(client.state.version).toBe("2.0.4");
    expect(next.downloadAndInstall).not.toHaveBeenCalled();
    await client.install();
    expect(next.downloadAndInstall).toHaveBeenCalledOnce();
    expect(states).toContain(40);
    expect(states).toContain("installing");
    expect(client.state.phase).toBe("installed");
    expect(next.close).toHaveBeenCalledOnce();
    expect(relaunch).toHaveBeenCalledOnce();
  });
  it("下载或签名校验失败不重启，并可重试", async () => {
    const next = update();
    vi.mocked(next.downloadAndInstall).mockRejectedValueOnce(new Error("signature invalid"));
    const relaunch = vi.fn(async () => {});
    const client = new UpdateController({ check: async () => next, relaunch });
    await client.check();
    await client.install();
    expect(client.state.phase).toBe("error");
    expect(relaunch).not.toHaveBeenCalled();
    await client.install();
    expect(relaunch).toHaveBeenCalledOnce();
  });
  it("检查失败保留可用更新；新检查无更新时关闭旧资源并清除通知", async () => {
    const next = update();
    const check = vi
      .fn<() => Promise<UpdateHandle | null>>()
      .mockResolvedValueOnce(next)
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValueOnce(null);
    const client = new UpdateController({ check, relaunch: async () => {} });
    await client.check();
    await client.check();
    expect(client.state.version).toBe("2.0.4");
    expect(client.state.error).toContain("检查失败");
    await client.check();
    expect(client.state.version).toBeNull();
    expect(client.state.phase).toBe("idle");
    expect(next.close).toHaveBeenCalledOnce();
  });
  it("并发检查合并，组件卸载后关闭迟到的更新资源", async () => {
    const next = update();
    let resolve!: (value: UpdateHandle) => void;
    const check = vi.fn(
      () =>
        new Promise<UpdateHandle>((r) => {
          resolve = r;
        }),
    );
    const client = new UpdateController({ check, relaunch: async () => {} });
    const pending = client.check();
    await client.check();
    expect(check).toHaveBeenCalledOnce();
    client.dispose();
    resolve(next);
    await pending;
    expect(next.close).toHaveBeenCalledOnce();
  });
  it("重启失败保留已安装状态，可单独重试重启", async () => {
    const relaunch = vi.fn(async () => {}).mockRejectedValueOnce(new Error("blocked"));
    const client = new UpdateController({ check: async () => update(), relaunch });
    await client.check();
    await client.install();
    expect(client.state.phase).toBe("installed");
    expect(client.state.error).toContain("请重启");
    await client.restart();
    expect(relaunch).toHaveBeenCalledTimes(2);
  });
  it("预览页面明确能力边界，不调用原生更新", () => {
    const html = renderToStaticMarkup(<AppUpdates settings />);
    expect(html).toContain("浏览器预览不支持桌面更新");
    expect(html).toContain("每 6 小时");
  });
});
