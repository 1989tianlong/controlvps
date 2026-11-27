import { useCallback, useEffect, useRef, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { ArrowUp, Download, File, FileQuestion, Folder, FolderOpen, FolderPlus, Home, Link2, Pencil, RefreshCw, Trash2, Upload, X } from "lucide-react";
import { api, errorMessage } from "../lib/api";
import { formatBytes, formatTime } from "../lib/format";
import type { FileListing, RemoteFileEntry, Server, TrustedExecutor } from "../types";
import { TransferQueue } from "./TransferQueue";

interface Props {
  server: Server;
  trusted: TrustedExecutor;
}

type NameDialog = { mode: "mkdir" | "rename"; name: string };

export function FileManager({ server, trusted }: Props) {
  const [listing, setListing] = useState<FileListing>();
  const [pathInput, setPathInput] = useState("");
  const [selectedPath, setSelectedPath] = useState<string>();
  const [busy, setBusy] = useState("list");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [transferRevision, setTransferRevision] = useState(0);
  const [nameDialog, setNameDialog] = useState<NameDialog | null>(null);
  const dialogRef = useRef<HTMLDialogElement>(null);
  const requestId = useRef(0);
  const selected = listing?.entries.find((entry) => entry.path === selectedPath);

  const load = useCallback(async (path?: string) => {
    const currentRequest = ++requestId.current;
    setBusy("list");
    setError("");
    setNotice("");
    try {
      const result = await trusted(server, (allow) => api.listRemoteFiles(server.id, path, allow));
      if (currentRequest !== requestId.current) return;
      setListing(result);
      setPathInput(result.path);
      setSelectedPath(undefined);
    } catch (reason) {
      if (currentRequest === requestId.current) setError(errorMessage(reason));
    } finally {
      if (currentRequest === requestId.current) setBusy("");
    }
  }, [server, trusted]);

  useEffect(() => {
    setListing(undefined);
    void load(".");
    return () => { requestId.current += 1; };
  }, [load]);

  useEffect(() => {
    if (nameDialog && !dialogRef.current?.open) dialogRef.current?.showModal();
    if (!nameDialog && dialogRef.current?.open) dialogRef.current.close();
  }, [nameDialog]);

  const perform = async (action: string, operation: (allow: boolean) => Promise<unknown>, message: string) => {
    if (!listing) return false;
    setBusy(action);
    setError("");
    setNotice("");
    try {
      await trusted(server, operation);
      setNotice(message);
      setNameDialog(null);
      setBusy("");
      await load(listing.path);
      setNotice(message);
      return true;
    } catch (reason) {
      setError(errorMessage(reason));
      setBusy("");
      return false;
    }
  };

  const createDirectory = () => {
    if (!listing || !nameDialog) return;
    const name = nameDialog.name.trim();
    void perform("mkdir", (allow) => api.createRemoteDirectory(server.id, listing.path, name, allow), `已创建目录 ${name}`);
  };

  const rename = () => {
    if (!selected || !nameDialog) return;
    const name = nameDialog.name.trim();
    void perform("rename", (allow) => api.renameRemoteFile(server.id, selected.path, name, allow), `已重命名为 ${name}`);
  };

  const remove = () => {
    if (!selected) return;
    const directory = selected.kind === "directory";
    const warning = directory
      ? `确认递归删除目录及其全部内容？\n\n${selected.path}\n\n此操作无法撤销。`
      : `确认删除远程文件？\n\n${selected.path}\n\n此操作无法撤销。`;
    if (!window.confirm(warning)) return;
    void perform("delete", (allow) => api.deleteRemoteFile(server.id, selected.path, directory, allow), `已删除 ${selected.name}`);
  };

  const upload = async () => {
    if (!listing) return;
    setError("");
    setNotice("");
    try {
      const selection = await open({ title: "选择要上传的文件", multiple: true, directory: false });
      if (!selection) return;
      const localPaths = Array.isArray(selection) ? selection : [selection];
      const files = localPaths.map((localPath) => ({ localPath, fileName: localPath.split(/[\\/]/).pop() })).filter((file): file is { localPath: string; fileName: string } => Boolean(file.fileName));
      const existing = files.filter((file) => listing.entries.some((entry) => entry.name === file.fileName));
      if (existing.length > 0 && !window.confirm(`远程已存在 ${existing.map((file) => file.fileName).join("、")}，确认在传输完成后覆盖？`)) return;
      setBusy("upload");
      for (const file of files) await api.enqueueFileUpload(server.id, file.localPath, listing.path, file.fileName);
      setTransferRevision((value) => value + 1);
      setNotice(`已将 ${files.length} 个文件加入后台传输队列`);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy("");
    }
  };

  const download = async () => {
    if (!selected || selected.kind === "directory") return;
    setError("");
    setNotice("");
    try {
      const localPath = await save({ title: "选择下载位置", defaultPath: selected.name });
      if (!localPath) return;
      setBusy("download");
      await api.enqueueFileDownload(server.id, selected.path, localPath);
      setTransferRevision((value) => value + 1);
      setNotice(`已将 ${selected.name} 加入后台传输队列`);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy("");
    }
  };

  const working = busy !== "";
  return <div className="file-manager">
    <form className="file-pathbar" onSubmit={(event) => { event.preventDefault(); if (pathInput.trim()) void load(pathInput.trim()); }}>
      <Home size={16} aria-hidden="true" />
      <input className="input" aria-label="远程路径" value={pathInput} onChange={(event) => setPathInput(event.target.value)} placeholder="/home/user" disabled={working && busy !== "list"} />
      <button className="btn" type="submit" disabled={working || !pathInput.trim()}>前往</button>
    </form>
    <div className="file-toolbar">
      <div className="row-actions justify-start">
        <button className="icon-btn" type="button" title="主目录" aria-label="打开主目录" disabled={working} onClick={() => void load(".")}><Home size={17} /></button>
        <button className="icon-btn" type="button" title="上一级目录" aria-label="上一级目录" disabled={working || !listing?.parent} onClick={() => void load(listing?.parent)}><ArrowUp size={17} /></button>
        <button className="icon-btn" type="button" title="刷新目录" aria-label="刷新目录" disabled={working} onClick={() => void load(listing?.path)}><RefreshCw size={17} /></button>
        <span className="toolbar-separator" />
        <button className="btn" type="button" disabled={working || !listing} onClick={() => setNameDialog({ mode: "mkdir", name: "" })}><FolderPlus size={16} />新建目录</button>
        <button className="btn" type="button" disabled={working || !listing} onClick={() => void upload()}><Upload size={16} />上传文件</button>
      </div>
      <span className="table-meta">{listing ? `${listing.entries.length} 个项目` : "正在连接 SFTP"}</span>
    </div>
    {error && <div className="error-banner m-4" role="alert"><p>{error}</p><button className="btn" type="button" onClick={() => void load(pathInput || ".")}>重试</button></div>}
    {notice && <div className="file-notice" role="status">{notice}</div>}
    {selected && <div className="file-selection">
      <span className="file-selection-name"><EntryIcon entry={selected} size={16} />{selected.name}</span>
      <div className="row-actions">
        {selected.kind !== "directory" && <button className="btn" type="button" disabled={working} onClick={() => void download()}><Download size={15} />下载</button>}
        <button className="btn" type="button" disabled={working} onClick={() => setNameDialog({ mode: "rename", name: selected.name })}><Pencil size={15} />重命名</button>
        <button className="btn btn-danger" type="button" disabled={working} onClick={remove}><Trash2 size={15} />删除</button>
      </div>
    </div>}
    <div className="file-list" aria-busy={busy === "list"}>
      {listing ? <table className="data-table"><thead><tr><th>名称</th><th>大小</th><th>修改时间</th><th>权限</th></tr></thead><tbody>
        {listing.entries.map((entry) => <tr key={entry.path} className={selectedPath === entry.path ? "file-row is-selected" : "file-row"} onClick={() => setSelectedPath(entry.path)}>
          <td><span className="file-name"><EntryIcon entry={entry} size={18} />{entry.kind === "directory" ? <button className="table-link" type="button" disabled={working} onClick={(event) => { event.stopPropagation(); void load(entry.path); }}>{entry.name}</button> : <span>{entry.name}</span>}{entry.kind === "symlink" && <span className="badge">链接</span>}</span></td>
          <td className="measured">{entry.kind === "directory" ? "—" : formatBytes(entry.size)}</td>
          <td className="log-time">{entry.modifiedAt ? formatTime(entry.modifiedAt) : "—"}</td>
          <td className="measured">{entry.permissions ?? "—"}</td>
        </tr>)}
      </tbody></table> : !error && <div className="skeleton file-skeleton" aria-label="正在读取远程目录" />}
      {listing?.entries.length === 0 && <div className="file-empty"><FolderOpen size={28} /><p>这个目录是空的</p></div>}
    </div>
    <TransferQueue serverId={server.id} revision={transferRevision} onCompleted={() => void load(listing?.path)} />
    {busy && busy !== "list" && <div className="file-progress" role="status">{{ mkdir: "正在创建目录", rename: "正在重命名", delete: "正在删除", upload: "正在验证连接并加入队列", download: "正在验证连接并加入队列" }[busy]}</div>}
    <dialog ref={dialogRef} className="dialog file-dialog" onCancel={(event) => { event.preventDefault(); if (busy !== "mkdir" && busy !== "rename") setNameDialog(null); }}>
      <form onSubmit={(event) => { event.preventDefault(); if (nameDialog?.mode === "mkdir") createDirectory(); else rename(); }}>
        <div className="dialog-header"><h2>{nameDialog?.mode === "mkdir" ? "新建远程目录" : "重命名"}</h2><button className="icon-btn" type="button" aria-label="关闭" disabled={busy === "mkdir" || busy === "rename"} onClick={() => setNameDialog(null)}><X size={18} /></button></div>
        <div className="dialog-body">{error && <div className="error-banner" role="alert">{error}</div>}<div className="field"><label htmlFor="remote-file-name">名称</label><input id="remote-file-name" className="input" autoFocus required value={nameDialog?.name ?? ""} onChange={(event) => setNameDialog((current) => current ? { ...current, name: event.target.value } : null)} disabled={busy === "mkdir" || busy === "rename"} /><p className="field-help">不能包含 /，也不能使用 . 或 .. 作为名称。</p></div></div>
        <div className="dialog-footer"><button className="btn" type="button" onClick={() => setNameDialog(null)} disabled={busy === "mkdir" || busy === "rename"}>取消</button><button className="btn btn-primary" type="submit" disabled={!nameDialog?.name.trim() || busy === "mkdir" || busy === "rename"}>{busy === "mkdir" || busy === "rename" ? "正在处理" : "确认"}</button></div>
      </form>
    </dialog>
  </div>;
}

function EntryIcon({ entry, size }: { entry: RemoteFileEntry; size: number }) {
  if (entry.kind === "directory") return <Folder size={size} aria-hidden="true" />;
  if (entry.kind === "symlink") return <Link2 size={size} aria-hidden="true" />;
  if (entry.kind === "other") return <FileQuestion size={size} aria-hidden="true" />;
  return <File size={size} aria-hidden="true" />;
}
