"""独立核对本机 Kimi 1.5 / Codex 单调累计日志。只输出数字，不保存正文。

运行 python3 scripts/verify_local_usage.py。Rust 使用内存 DB，源文件始终只读。
活跃日志先提取临时 usage-only 快照（不含正文），核对后自动删除。
这是针对当前已知格式的核对器；遇到分叉继承、累计重置或格式不符时停止，不能把未核对当通过。
"""
import datetime as dt
import json
import os
from pathlib import Path
import subprocess
import tempfile

FIELDS = ("inputFresh", "inputCacheRead", "inputCacheWrite", "inputTotal", "outputTotal", "total")

def zero():
    return {**{field: 0 for field in FIELDS}, "events": 0, "known": 0}

def add(target, usage, stamp):
    target["events"] += 1
    if usage is not None:
        target["known"] += 1
        for key, value in zip(FIELDS, usage):
            target[key] += value


def capture_snapshot(snapshot):
    """仅提取计量/格式元数据，绝不复制提示词、回复、工具结果或认证内容。"""
    for agent, home_key, default in [("kimi_code", "KIMI_CODE_HOME", ".kimi-code"), ("codex", "CODEX_HOME", ".codex")]:
        root = Path(os.environ.get(home_key) or Path.home() / default)
        destination = snapshot / agent
        for folder in ("sessions", "archived_sessions"):
            pattern = "wire.jsonl" if agent == "kimi_code" else "rollout-*.jsonl"
            if agent == "codex" and list((root / folder).rglob("*.jsonl.zst")):
                raise RuntimeError("当前独立核对器不验证压缩日志")
            for path in (root / folder).rglob(pattern):
                target = destination / path.relative_to(root)
                target.parent.mkdir(parents=True, exist_ok=True)
                with path.open("rb") as stream, target.open("w") as output:
                    for line in stream:
                        if not line.endswith(b"\n"): continue
                        try: record = json.loads(line)
                        except (ValueError, UnicodeError): continue
                        if agent == "kimi_code":
                            if record.get("type") not in ("metadata", "usage.record", "forked"): continue
                            if record.get("type") == "usage.record":
                                u = record.get("usage", {})
                                record = {key: record[key] for key in ("type", "agentId", "model", "usageScope", "time") if key in record}
                                record["usage"] = {key: u[key] for key in ("inputOther", "inputCacheRead", "inputCacheCreation", "output") if key in u}
                            else:
                                record = {key: record[key] for key in ("type", "protocol_version", "created_at", "time") if key in record}
                        else:
                            payload = record.get("payload") or {}
                            if record.get("type") == "session_meta":
                                payload = {key: payload[key] for key in ("id", "thread_id", "threadId", "forked_from_id", "source") if key in payload}
                                if isinstance(payload.get("source"), dict):
                                    parent = payload["source"].get("subagent", {}).get("thread_spawn", {}).get("parent_thread_id")
                                    payload["source"] = {"subagent": {"thread_spawn": {"parent_thread_id": parent}}}
                            elif record.get("type") == "turn_context":
                                payload = {"model": payload.get("model")}
                            elif record.get("type") == "event_msg" and payload.get("type") == "token_count":
                                payload = {key: payload[key] for key in ("type", "info", "rate_limits") if key in payload}
                                info = payload.get("info")
                                if isinstance(info, dict):
                                    payload["info"] = {key: info[key] for key in ("total_token_usage", "last_token_usage", "model_context_window") if key in info}
                                if isinstance(payload.get("rate_limits"), dict):
                                    payload["rate_limits"] = {"limit_id": payload["rate_limits"].get("limit_id")}
                            else: continue
                            record = {"timestamp": record.get("timestamp"), "type": record["type"], "payload": payload}
                        output.write(json.dumps(record, separators=(",", ":")) + "\n")


def verify(snapshot):
    now = dt.datetime.now().astimezone()
    start = now.replace(hour=0, minute=0, second=0, microsecond=0)
    env = {**os.environ, "KIMI_CODE_HOME": str(snapshot / "kimi_code"), "CODEX_HOME": str(snapshot / "codex"), "PROVIDOCK_VERIFY_FROM": start.astimezone(dt.timezone.utc).isoformat().replace("+00:00", "Z"), "PROVIDOCK_VERIFY_TO": now.astimezone(dt.timezone.utc).isoformat().replace("+00:00", "Z")}
    expected = {}
    stamps = {}
    for agent, home_key, default in [("kimi_code", "KIMI_CODE_HOME", ".kimi-code"), ("codex", "CODEX_HOME", ".codex")]:
        root = Path(env[home_key])
        files = list((root / "sessions").rglob("wire.jsonl" if agent == "kimi_code" else "rollout-*.jsonl"))
        if agent == "codex":
            files += list((root / "archived_sessions").rglob("rollout-*.jsonl"))
            if list(root.rglob("*.jsonl.zst")):
                raise RuntimeError("当前独立核对器不验证压缩日志；请单独核对该格式")
        total, today = zero(), zero()
        for path in files:
            stat = path.stat(); stamps[path] = (stat.st_size, stat.st_mtime_ns)
            previous = None
            final = None
            today_counters = [0, 0, 0, 0]
            with path.open("rb") as stream:
                for line in stream:
                    if not line.endswith(b"\n"): continue
                    try: record = json.loads(line)
                    except (ValueError, UnicodeError): continue
                    if agent == "kimi_code":
                        if record.get("type") == "metadata" and record.get("protocol_version") != "1.5":
                            raise RuntimeError("Kimi 协议超出独立核对范围")
                        if record.get("type") == "forked": raise RuntimeError("Kimi 分叉需要额外祖先核对")
                        if record.get("type") != "usage.record": continue
                        u = record["usage"]
                        fresh, read, write, output = (u[key] for key in ("inputOther", "inputCacheRead", "inputCacheCreation", "output"))
                        usage = (fresh, read, write, fresh + read + write, output, fresh + read + write + output) if fresh + read + write + output else None
                        stamp = dt.datetime.fromtimestamp(record["time"] / 1000, dt.timezone.utc)
                        add(total, usage, stamp)
                        if start <= stamp < now: add(today, usage, stamp)
                    else:
                        payload = record.get("payload") or {}
                        if record.get("type") == "session_meta" and payload.get("forked_from_id"):
                            raise RuntimeError("Codex fork 需要额外祖先核对")
                        if record.get("type") != "event_msg" or payload.get("type") != "token_count": continue
                        info = payload.get("info")
                        if not isinstance(info, dict): continue
                        cumulative = info.get("total_token_usage")
                        if not isinstance(cumulative, dict): raise RuntimeError("Codex 缺少累计计数，独立核对停止")
                        current = [cumulative.get(key, 0) for key in ("input_tokens", "cached_input_tokens", "cache_write_input_tokens", "output_tokens")]
                        if previous is not None and any(a < b for a, b in zip(current, previous)):
                            raise RuntimeError("Codex 累计量重置，独立核对停止")
                        stamp = dt.datetime.fromisoformat(record["timestamp"].replace("Z", "+00:00"))
                        if previous == current: continue
                        if not isinstance(info.get("last_token_usage"), dict): raise RuntimeError("当前独立核对需要 last 与 total 同时提供")
                        total["events"] += 1; total["known"] += 1
                        if start <= stamp < now:
                            delta = [a - b for a, b in zip(current, previous or [0] * 4)]
                            today_counters = [a + b for a, b in zip(today_counters, delta)]
                            today["events"] += 1; today["known"] += 1
                        previous = current; final = current
            if agent == "codex" and final is not None:
                # 独立依据是会话最终累计量，而不是把 last_usage 再求和。
                for target, values in [(total, final), (today, today_counters)]:
                    input_, read, write, output = values
                    for field, value in zip(FIELDS, (input_ - read - write, read, write, input_, output, input_ + output)):
                        target[field] += value
        expected[agent] = {"files": len(files), "summary": total, "today": today}
    project = Path(__file__).resolve().parents[1]
    run = subprocess.run(["cargo", "run", "--quiet", "--offline", "--locked", "--manifest-path", str(project / "src-tauri/Cargo.toml"), "--example", "verify_local_usage"], cwd=project, env=env, capture_output=True, text=True)
    if run.returncode: raise RuntimeError(run.stderr)
    actual = {item["agent"]: item for item in json.loads(run.stdout)}
    for path, before in stamps.items():
        stat = path.stat()
        if before != (stat.st_size, stat.st_mtime_ns): raise RuntimeError("核对期间源文件发生变化，请重试")
    assert actual.keys() == expected.keys(), "有来源未被发现"
    for agent, want in expected.items():
        got = actual[agent]
        assert got["files"] == want["files"], f"{agent}: 文件数量不一致"
        assert got["deferred"] == 0, f"{agent}: 存在未处理文件"
        assert got["repeatInserted"] == 0 and got["repeatReplaced"] == 0, f"{agent}: 重扫发生数据变化"
        for scope in ("summary", "today"):
            summary = got[scope]; oracle = want[scope]
            assert summary["events"] == oracle["events"], f"{agent}/{scope}: 事件数不一致 {summary['events']} != {oracle['events']}"
            for field in FIELDS:
                value = summary[field]["value"]
                assert (int(value) if value is not None else None) == (oracle[field] if oracle["known"] else None), f"{agent}/{scope}/{field}: {value} != {oracle[field]}"
        assert got["summary"] == got["after"], f"{agent}: 重扫汇总不一致"
        print(json.dumps({"agent": agent, "files": want["files"], "events": want["summary"]["events"], "total": want["summary"]["total"], "todayTotal": want["today"]["total"], "independentComparison": "passed", "repeatScan": "unchanged"}, ensure_ascii=False))

if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="providock-usage-only-") as directory:
        snapshot = Path(directory)
        capture_snapshot(snapshot)
        verify(snapshot)
