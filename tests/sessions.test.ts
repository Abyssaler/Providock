import { describe, expect, it } from "vitest";
import {
  contentReasonLabel,
  displayDomain,
  parseContentError,
  roleLabel,
  splitCodeSegments,
  splitLinks,
} from "../src/features/sessions/contentView";

describe("displayDomain（外部链接只展示目标域名）", () => {
  it("接受 http/https 并返回规范化主机名", () => {
    expect(displayDomain("https://Example.COM/a/b?x=1")).toBe("example.com");
    expect(displayDomain("http://127.0.0.1:8080/path")).toBe("127.0.0.1");
  });

  it("拒绝非 http(s) 协议与非法输入", () => {
    expect(displayDomain("javascript:alert(1)")).toBeNull();
    expect(displayDomain("data:text/html,<b>x</b>")).toBeNull();
    expect(displayDomain("file:///etc/passwd")).toBeNull();
    expect(displayDomain("/local/path")).toBeNull();
    expect(displayDomain("not a url")).toBeNull();
  });
});

describe("splitCodeSegments（白名单渲染：代码块与段落）", () => {
  it("按围栏切分代码块与普通文本", () => {
    const segments = splitCodeSegments("说明\n```\nlet a = 1;\n```\n结尾");
    expect(segments).toEqual([
      { kind: "text", value: "说明" },
      { kind: "code", value: "let a = 1;" },
      { kind: "text", value: "结尾" },
    ]);
  });

  it("未闭合围栏的剩余内容仍按代码块处理，不丢失也不执行", () => {
    const segments = splitCodeSegments("```\nrm -rf /\n");
    expect(segments).toEqual([{ kind: "code", value: "rm -rf /\n" }]);
  });

  it("纯文本不产生空代码块", () => {
    expect(splitCodeSegments("hello\nworld")).toEqual([{ kind: "text", value: "hello\nworld" }]);
    expect(splitCodeSegments("   ")).toEqual([]);
  });
});

describe("splitLinks（点击前展示域名，不生成可点击元素）", () => {
  it("切出链接片段并附域名", () => {
    const parts = splitLinks("见 https://example.com/a 与 javascript:alert(1)");
    expect(parts[0]).toEqual({ kind: "text", value: "见 " });
    expect(parts[1]).toEqual({
      kind: "link",
      value: "https://example.com/a",
      domain: "example.com",
    });
    // javascript: 不是 http(s)，按普通文本保留，不形成链接片段
    expect(parts[2]).toEqual({ kind: "text", value: " 与 javascript:alert(1)" });
  });
});

describe("parseContentError（失败原因分类）", () => {
  it("读取后端错误码与说明", () => {
    expect(parseContentError({ code: "SourceGone", message: "源文件已删除" })).toEqual({
      code: "SourceGone",
      message: "源文件已删除",
    });
    // 有码无文案时回落到分类说明
    expect(parseContentError({ code: "UnsupportedSource" }).message).toContain("尚未实现");
    expect(parseContentError("boom")).toEqual({ code: "Internal", message: "boom" });
    expect(parseContentError(null).code).toBe("Internal");
  });

  it("四类正文缺失原因都有明确文案", () => {
    for (const code of ["SourceGone", "PermissionDenied", "Incompatible", "UnsupportedSource"]) {
      const label = contentReasonLabel(code);
      expect(label.length).toBeGreaterThan(6);
    }
    expect(contentReasonLabel("ContentDisabled")).toContain("未开启");
  });
});

describe("roleLabel", () => {
  it("映射已知角色并保留未知角色名", () => {
    expect(roleLabel("user")).toBe("用户");
    expect(roleLabel("tool_result")).toBe("工具结果");
    expect(roleLabel("other:thinking")).toBe("其它（thinking）");
  });
});
