import { useEffect, useRef, useState } from "react";
import { BookOpenCheck, Bot, CheckCircle2, LoaderCircle, Play, Send, Settings, ShieldAlert, Sparkles, TerminalSquare, Trash2, X } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import type {
  AiActionProposal,
  AiChatMessage,
  AiProposalExecutionResult,
  AiSettings,
  AiToolTrace,
  Server,
} from "../types";

export interface AiPromptPreset {
  id: number;
  prompt: string;
  serverId?: string;
  contextNote?: string;
}

interface Props {
  open: boolean;
  servers: Server[];
  preset?: AiPromptPreset;
  onClose: () => void;
  onOpenSettings: () => void;
}

interface TurnEntry {
  id: string;
  role: "user" | "assistant";
  content: string;
  model?: string;
  toolTraces?: AiToolTrace[];
  proposals?: AiActionProposal[];
}

const quickPrompts = [
  "巡检当前所有服务器，列出异常或临期资产",
  "检查当前服务器的 CPU/内存/磁盘与失败服务",
  "排查 Docker 容器与安全审计风险，给出加固建议",
  "帮我生成一份服务器日常巡检 Runbook",
];

export function AiAssistantDialog({ open, servers, preset, onClose, onOpenSettings }: Props) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const lastPresetId = useRef(0);
  const [settings, setSettings] = useState<AiSettings>();
  const [serverId, setServerId] = useState("");
  const [contextNote, setContextNote] = useState("");
  const [input, setInput] = useState("");
  const [turns, setTurns] = useState<TurnEntry[]>([]);
  const [sending, setSending] = useState(false);
  const [executingId, setExecutingId] = useState("");
  const [proposalResults, setProposalResults] = useState<Record<string, AiProposalExecutionResult>>({});
  const [error, setError] = useState("");

  useEffect(() => {
    if (open && !dialogRef.current?.open) {
      dialogRef.current?.showModal();
      void api.getAiSettings().then(setSettings).catch(() => undefined);
    }
    if (!open && dialogRef.current?.open) {
      dialogRef.current.close();
    }
  }, [open]);

  useEffect(() => {
    if (!open || !preset || preset.id === lastPresetId.current) return;
    lastPresetId.current = preset.id;
    if (preset.serverId) setServerId(preset.serverId);
    setContextNote(preset.contextNote ?? "");
    setInput(preset.prompt);
  }, [open, preset]);

  const sendMessage = async (textOverride?: string) => {
    const text = (textOverride ?? input).trim();
    if (!text || sending) return;
    setError("");
    const userTurn: TurnEntry = {
      id: `u-${Date.now()}`,
      role: "user",
      content: text,
    };
    const nextTurns = [...turns, userTurn];
    setTurns(nextTurns);
    if (!textOverride) setInput("");
    setSending(true);
    try {
      const history: AiChatMessage[] = nextTurns.map((item) => ({
        role: item.role,
        content: item.content,
      }));
      const response = await api.aiChat({
        messages: history,
        contextServerId: serverId || undefined,
        contextNote: contextNote || undefined,
      });
      setTurns((current) => [
        ...current,
        {
          id: `a-${Date.now()}`,
          role: "assistant",
          content: response.reply,
          model: response.model,
          toolTraces: response.toolTraces,
          proposals: response.proposals,
        },
      ]);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setSending(false);
    }
  };

  const executeProposal = async (proposal: AiActionProposal) => {
    if (executingId) return;
    const confirmText = proposal.kind === "save_runbook"
      ? `确认将「${proposal.title}」（${proposal.steps.length} 个步骤）保存到本地 Runbook 库？`
      : proposal.dangerous
        ? `警告：此操作包含高风险命令！\n\n目标：${proposal.server ?? proposal.servers.join("、")}\n命令：${proposal.command}\n\n确认批准并执行？`
        : `确认执行此操作？\n\n目标：${proposal.server ?? proposal.servers.join("、")}\n命令：${proposal.command}`;
    if (!window.confirm(confirmText)) return;
    setExecutingId(proposal.id);
    setError("");
    try {
      const result = await api.executeAiProposal(proposal);
      setProposalResults((current) => ({ ...current, [proposal.id]: result }));
    } catch (reason) {
      setProposalResults((current) => ({
        ...current,
        [proposal.id]: {
          success: false,
          summary: errorMessage(reason),
          output: "",
        },
      }));
    } finally {
      setExecutingId("");
    }
  };

  const clear = () => {
    setTurns([]);
    setProposalResults({});
    setContextNote("");
    setError("");
  };

  return (
    <dialog
      ref={dialogRef}
      className="dialog batch-review"
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
    >
      <div className="dialog-header">
        <div className="flex items-center gap-2">
          <Sparkles size={18} />
          <div>
            <h2>AI 运维助手</h2>
            <span className="table-meta">
              {settings?.enabled
                ? `${settings.model} · ${settings.permissionMode === "observer" ? "只读观察" : "写操作需确认"} · 复用本地 12 项只读巡检工具`
                : "尚未启用模型，请先到「AI 接入」配置 OpenAI 兼容 / Claude / Ollama"}
            </span>
          </div>
        </div>
        <div className="row-actions">
          {turns.length > 0 && (
            <button className="icon-btn" type="button" title="清空会话" aria-label="清空会话" onClick={clear} disabled={sending}>
              <Trash2 size={16} />
            </button>
          )}
          <button className="icon-btn" type="button" title="模型与权限设置" aria-label="模型设置" onClick={() => { onClose(); onOpenSettings(); }}>
            <Settings size={16} />
          </button>
          <button className="icon-btn" type="button" aria-label="关闭 AI 助手" onClick={onClose}>
            <X size={18} />
          </button>
        </div>
      </div>

      <div className="dialog-body">
        {!settings?.enabled ? (
          <div className="empty">
            <div>
              <Bot size={32} />
              <h3>启用应用内 AI 运维助手</h3>
              <p>支持 BYOK（OpenAI / DeepSeek / Claude）或本机 Ollama，API Key 仅保存在系统钥匙串。AI 可自动调用只读巡检工具并在写操作前生成待确认卡片。</p>
              <button className="btn btn-primary" type="button" onClick={() => { onClose(); onOpenSettings(); }}>
                前往「AI 接入」配置模型
              </button>
            </div>
          </div>
        ) : <>
          <div className="filters mb-4">
            <select className="select" aria-label="关注服务器上下文" value={serverId} onChange={(e) => setServerId(e.target.value)}>
              <option value="">全部服务器上下文</option>
              {servers.map((server) => (
                <option key={server.id} value={server.id}>{server.name} · {server.host}</option>
              ))}
            </select>
            {contextNote && (
              <span className="badge badge-cool">已附带诊断上下文</span>
            )}
          </div>

          {turns.length === 0 && (
            <div className="mb-4">
              <p className="table-meta mb-2">快捷指令（快捷键 `⌘K` / `Ctrl+K` 随时唤起）：</p>
              <div className="flex flex-wrap gap-2">
                {quickPrompts.map((prompt) => (
                  <button key={prompt} className="btn" type="button" disabled={sending} onClick={() => void sendMessage(prompt)}>
                    {prompt}
                  </button>
                ))}
              </div>
            </div>
          )}

          {error && <div className="error-banner mb-4" role="alert">{error}</div>}

          <div className="stack">
            {turns.map((turn) => (
              <article key={turn.id} className={turn.role === "user" ? "panel panel-body" : "panel panel-body"}>
                <div className="flex items-center justify-between mb-2">
                  <strong>{turn.role === "user" ? "你" : `AI 助手${turn.model ? ` · ${turn.model}` : ""}`}</strong>
                  {turn.toolTraces && turn.toolTraces.length > 0 && (
                    <span className="badge badge-cool">调用 {turn.toolTraces.length} 个工具</span>
                  )}
                </div>
                {turn.toolTraces && turn.toolTraces.length > 0 && (
                  <details className="mb-3">
                    <summary className="table-meta">查看工具调用轨迹（{turn.toolTraces.length} 步）</summary>
                    <ul className="mt-2 stack">
                      {turn.toolTraces.map((trace, index) => (
                        <li key={`${turn.id}-t-${index}`} className="table-meta">
                          <code>{trace.tool}</code> {trace.target ? `· ${trace.target}` : ""} — {trace.summary}（{trace.durationMs} ms）
                        </li>
                      ))}
                    </ul>
                  </details>
                )}
                {turn.content && (
                  <div style={{ whiteSpace: "pre-wrap", lineHeight: 1.6 }}>{turn.content}</div>
                )}
                {turn.proposals && turn.proposals.length > 0 && (
                  <div className="stack mt-4">
                    {turn.proposals.map((proposal) => {
                      const execResult = proposalResults[proposal.id];
                      return (
                        <section key={proposal.id} className="panel panel-body">
                          <div className="flex items-center justify-between gap-2 mb-2">
                            <div className="flex items-center gap-2">
                              {proposal.kind === "save_runbook" ? <BookOpenCheck size={16} /> : <TerminalSquare size={16} />}
                              <strong>{proposal.title}</strong>
                              {proposal.dangerous && <span className="badge badge-bad"><ShieldAlert size={12} /> 高风险</span>}
                            </div>
                            <span className="badge">
                              {proposal.kind === "save_runbook"
                                ? `Runbook 草稿 · ${proposal.steps.length} 步`
                                : proposal.kind === "run_batch_command"
                                  ? `批量 · ${proposal.servers.join("、")}`
                                  : `单机 · ${proposal.server}`}
                            </span>
                          </div>
                          {proposal.rationale && <p className="table-meta mb-2">{proposal.rationale}</p>}
                          {proposal.command && <pre className="review-command"><code>{proposal.command}</code></pre>}
                          {proposal.steps.length > 0 && (
                            <div className="stack mt-2">
                              {proposal.steps.map((step, idx) => (
                                <div key={`${proposal.id}-s-${idx}`}>
                                  <span className="table-meta">{idx + 1}. {step.name}</span>
                                  <pre className="review-command"><code>{step.command}</code></pre>
                                </div>
                              ))}
                            </div>
                          )}
                          <div className="row-actions mt-3">
                            {!execResult ? (
                              <button
                                className={proposal.dangerous ? "btn btn-danger" : "btn btn-primary"}
                                type="button"
                                disabled={executingId !== ""}
                                onClick={() => void executeProposal(proposal)}
                              >
                                {executingId === proposal.id
                                  ? <LoaderCircle size={14} className="spin" />
                                  : proposal.kind === "save_runbook" ? <BookOpenCheck size={14} /> : <Play size={14} />}
                                {executingId === proposal.id
                                  ? "执行中"
                                  : proposal.kind === "save_runbook" ? "保存为 Runbook" : "批准并执行"}
                              </button>
                            ) : (
                              <span className={execResult.success ? "badge badge-ok" : "badge badge-bad"}>
                                <CheckCircle2 size={12} /> {execResult.summary}
                              </span>
                            )}
                          </div>
                          {execResult?.output && (
                            <details className="mt-2" open={!execResult.success}>
                              <summary className="table-meta">查看执行输出</summary>
                              <pre className="action-output"><code>{execResult.output}</code></pre>
                            </details>
                          )}
                        </section>
                      );
                    })}
                  </div>
                )}
              </article>
            ))}
            {sending && (
              <div className="panel panel-body flex items-center gap-2 table-meta">
                <LoaderCircle size={15} className="spin" />
                正在分析并按需调用只读巡检工具…
              </div>
            )}
          </div>
        </>}
      </div>

      {settings?.enabled && (
        <form
          className="dialog-footer"
          onSubmit={(event) => {
            event.preventDefault();
            void sendMessage();
          }}
        >
          <input
            className="input flex-1"
            placeholder="用中文描述你想排查的问题、想生成的命令或 Runbook…"
            value={input}
            disabled={sending}
            onChange={(e) => setInput(e.target.value)}
            autoFocus
          />
          <button className="btn btn-primary" type="submit" disabled={sending || !input.trim()}>
            <Send size={15} />发送
          </button>
        </form>
      )}
    </dialog>
  );
}
