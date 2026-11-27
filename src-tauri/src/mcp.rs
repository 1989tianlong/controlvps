use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};

use crate::batch;
use crate::db::Database;
use crate::discovery;
use crate::docker;
use crate::error::AppError;
use crate::metrics;
use crate::models::{
    McpCallRecord, McpSettings, PortForwardKind, RunbookExecutionRequest, RunbookTrigger, Server,
};
use crate::runbook::{self, RunbookExecutionRegistry};
use crate::security;
use crate::ssh;
use crate::systemd;

pub const PROTOCOL_VERSION: &str = "2025-06-18";
const SUPPORTED_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
const MAX_CONNECTIONS: usize = 16;
const MAX_OUTPUT_CHARS: usize = 60_000;

const INSTRUCTIONS: &str = "ControlVPS 是运行在用户本机的 VPS 管理应用。你可以通过工具读取服务器清单、采集指标、执行命令、批量运维、诊断连接、巡检 systemd 与 Docker、做安全审计、查看隧道与 Runbook、检索 VPS 优惠和操作记录。\
服务器必须先在应用内添加；遇到未知主机密钥时，需要用户先在应用界面手动连接一次完成确认。\
执行命令、批量命令和运行 Runbook 属于写操作，只有用户在「AI 接入」页开启开关后才可用。\
命令输出会按 60000 字符截断，长任务请使用 timeoutSeconds 控制超时。";

#[derive(Clone)]
struct McpConfig {
    enabled: bool,
    port: u16,
    allow_execute: bool,
    allow_unknown_host: bool,
    token: String,
}

impl From<&McpSettings> for McpConfig {
    fn from(settings: &McpSettings) -> Self {
        Self {
            enabled: settings.enabled,
            port: settings.port,
            allow_execute: settings.allow_execute,
            allow_unknown_host: settings.allow_unknown_host,
            token: settings.token.clone(),
        }
    }
}

pub struct McpState {
    db: Database,
    known_hosts: PathBuf,
    registry: RunbookExecutionRegistry,
    config: Mutex<McpConfig>,
    notify: Option<Arc<dyn Fn(McpCallRecord) + Send + Sync>>,
}

impl McpState {
    fn config(&self) -> McpConfig {
        match self.config.lock() {
            Ok(config) => config.clone(),
            Err(error) => error.into_inner().clone(),
        }
    }

    fn set_config(&self, settings: &McpSettings) {
        if let Ok(mut config) = self.config.lock() {
            *config = McpConfig::from(settings);
        }
    }

    fn allow_unknown_host(&self) -> bool {
        self.config().allow_unknown_host
    }
}

struct Listener {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Listener {
    fn shutdown(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[derive(Default)]
struct Runtime {
    listener: Option<Listener>,
    error: Option<String>,
}

#[derive(Clone)]
pub struct McpServer {
    state: Arc<McpState>,
    runtime: Arc<Mutex<Runtime>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolInfo {
    pub name: String,
    pub title: String,
    pub description: String,
    pub write: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    pub enabled: bool,
    pub port: u16,
    pub allow_execute: bool,
    pub allow_unknown_host: bool,
    pub token: String,
    pub running: bool,
    pub endpoint: String,
    pub error: Option<String>,
    pub tools: Vec<McpToolInfo>,
}

impl McpServer {
    pub fn bootstrap(
        db: Database,
        known_hosts: PathBuf,
        registry: RunbookExecutionRegistry,
        settings: &McpSettings,
        notify: Option<Arc<dyn Fn(McpCallRecord) + Send + Sync>>,
    ) -> Self {
        let state = Arc::new(McpState {
            db,
            known_hosts,
            registry,
            config: Mutex::new(McpConfig::from(settings)),
            notify,
        });
        let server = Self {
            state,
            runtime: Arc::new(Mutex::new(Runtime::default())),
        };
        if settings.enabled {
            let _ = server.restart();
        }
        server
    }

    pub fn apply(&self, settings: &McpSettings) -> McpStatus {
        self.state.set_config(settings);
        if settings.enabled {
            let _ = self.restart();
        } else {
            self.stop();
            if let Ok(mut runtime) = self.runtime.lock() {
                runtime.error = None;
            }
        }
        self.status()
    }

    pub fn rotate_token(&self, settings: &McpSettings) -> McpStatus {
        self.state.set_config(settings);
        self.status()
    }

    pub fn status(&self) -> McpStatus {
        let config = self.state.config();
        let (running, error) = match self.runtime.lock() {
            Ok(runtime) => (runtime.listener.is_some(), runtime.error.clone()),
            Err(_) => (false, Some("MCP 服务状态不可用".into())),
        };
        McpStatus {
            enabled: config.enabled,
            port: config.port,
            allow_execute: config.allow_execute,
            allow_unknown_host: config.allow_unknown_host,
            endpoint: format!("http://127.0.0.1:{}/mcp", config.port),
            token: config.token,
            running,
            error,
            tools: tool_specs()
                .into_iter()
                .map(|spec| McpToolInfo {
                    name: spec.name.into(),
                    title: spec.title.into(),
                    description: spec.description.into(),
                    write: spec.write,
                })
                .collect(),
        }
    }

    pub fn shutdown(&self) {
        self.stop();
    }

    fn restart(&self) -> Result<(), String> {
        let config = self.state.config();
        let mut runtime = match self.runtime.lock() {
            Ok(runtime) => runtime,
            Err(error) => error.into_inner(),
        };
        if let Some(listener) = runtime.listener.take() {
            listener.shutdown();
        }
        match spawn_listener(self.state.clone(), config.port) {
            Ok(listener) => {
                runtime.listener = Some(listener);
                runtime.error = None;
                Ok(())
            }
            Err(error) => {
                runtime.listener = None;
                runtime.error = Some(error.clone());
                Err(error)
            }
        }
    }

    fn stop(&self) {
        let listener = match self.runtime.lock() {
            Ok(mut runtime) => runtime.listener.take(),
            Err(error) => error.into_inner().listener.take(),
        };
        if let Some(listener) = listener {
            listener.shutdown();
        }
    }
}

struct ActiveGuard {
    counter: Arc<AtomicUsize>,
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

fn spawn_listener(state: Arc<McpState>, port: u16) -> Result<Listener, String> {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .map_err(|error| format!("无法监听 127.0.0.1:{port}：{error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("MCP 监听端口设置失败：{error}"))?;
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let handle = std::thread::Builder::new()
        .name("controlvps-mcp".into())
        .spawn(move || {
            let active = Arc::new(AtomicUsize::new(0));
            while !flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        if active.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
                            let _ = write_response(&mut stream, 503, "application/json", br#"{"error":"busy"}"#);
                            continue;
                        }
                        active.fetch_add(1, Ordering::Relaxed);
                        let worker_state = state.clone();
                        let worker_active = active.clone();
                        let spawned = std::thread::Builder::new()
                            .name("controlvps-mcp-call".into())
                            .spawn(move || {
                                let _guard = ActiveGuard {
                                    counter: worker_active,
                                };
                                let _ = handle_connection(&worker_state, stream);
                            });
                        if spawned.is_err() {
                            active.fetch_sub(1, Ordering::Relaxed);
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(40));
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(200)),
                }
            }
        })
        .map_err(|error| format!("无法启动 MCP 服务线程：{error}"))?;
    Ok(Listener {
        stop,
        handle: Some(handle),
    })
}

struct HttpRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl HttpRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header == name)
            .map(|(_, value)| value.as_str())
    }
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn read_request(stream: &mut TcpStream) -> Option<HttpRequest> {
    stream.set_read_timeout(Some(Duration::from_secs(20))).ok()?;
    stream.set_write_timeout(Some(Duration::from_secs(20))).ok()?;
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 8192];
    let header_end = loop {
        if let Some(position) = find_header_end(&buffer) {
            break position;
        }
        if buffer.len() > MAX_HEADER_BYTES {
            return None;
        }
        let count = stream.read(&mut chunk).ok()?;
        if count == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..count]);
    };
    let head = std::str::from_utf8(&buffer[..header_end]).ok()?;
    let mut lines = head.split("\r\n");
    let mut parts = lines.next()?.split_whitespace();
    let method = parts.next()?.to_ascii_uppercase();
    let target = parts.next()?.to_string();
    let path = target.split('?').next().unwrap_or("/").to_string();
    let mut headers = Vec::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    let mut content_length: Option<usize> = None;
    for (name, value) in &headers {
        if name == "content-length" {
            let parsed = value.parse::<usize>().ok()?;
            if content_length.is_some_and(|existing| existing != parsed) {
                return None;
            }
            content_length = Some(parsed);
        }
    }
    let length = content_length.unwrap_or(0);
    if length > MAX_BODY_BYTES {
        return None;
    }
    if headers
        .iter()
        .any(|(name, value)| name == "expect" && value.eq_ignore_ascii_case("100-continue"))
    {
        stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").ok()?;
        stream.flush().ok()?;
    }
    let mut body = buffer[header_end + 4..].to_vec();
    while body.len() < length {
        let count = stream.read(&mut chunk).ok()?;
        if count == 0 {
            return None;
        }
        body.extend_from_slice(&chunk[..count]);
        if body.len() > MAX_BODY_BYTES {
            return None;
        }
    }
    body.truncate(length);
    Some(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        503 => "Service Unavailable",
        _ => "OK",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

fn handle_connection(state: &McpState, mut stream: TcpStream) -> std::io::Result<()> {
    let Some(request) = read_request(&mut stream) else {
        return write_response(
            &mut stream,
            400,
            "text/plain; charset=utf-8",
            "MCP 请求格式不受支持".as_bytes(),
        );
    };
    if request.path != "/mcp" && request.path != "/" {
        return write_response(
            &mut stream,
            404,
            "text/plain; charset=utf-8",
            "MCP 端点为 /mcp".as_bytes(),
        );
    }
    if !host_allowed(&request, state.config().port) || !origin_allowed(&request) {
        return write_response(
            &mut stream,
            403,
            "text/plain; charset=utf-8",
            "来源或 Host 不被允许".as_bytes(),
        );
    }
    if !authorized(state, &request) {
        return write_response(
            &mut stream,
            401,
            "text/plain; charset=utf-8",
            "缺少或错误的 MCP 访问令牌".as_bytes(),
        );
    }
    if request.method != "POST" {
        return write_response(
            &mut stream,
            405,
            "text/plain; charset=utf-8",
            "ControlVPS MCP 仅接受 POST /mcp，不提供 SSE 长连接".as_bytes(),
        );
    }
    if request
        .header("transfer-encoding")
        .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"))
    {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": { "code": -32700, "message": "不支持分块传输，请使用 Content-Length" }
        });
        return write_response(
            &mut stream,
            400,
            "application/json",
            payload.to_string().as_bytes(),
        );
    }
    let body: Value = match serde_json::from_slice(&request.body) {
        Ok(body) => body,
        Err(error) => {
            let payload = json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": { "code": -32700, "message": format!("JSON 解析失败：{error}") }
            });
            return write_response(
                &mut stream,
                200,
                "application/json",
                payload.to_string().as_bytes(),
            );
        }
    };
    let (status, response) = handle_payload(state, &body);
    write_response(
        &mut stream,
        status,
        "application/json",
        response.as_bytes(),
    )
}

fn authorized(state: &McpState, request: &HttpRequest) -> bool {
    let expected = state.config().token;
    if expected.is_empty() {
        return false;
    }
    let Some(header) = request.header("authorization") else {
        return false;
    };
    let Some(token) = header.strip_prefix("Bearer ").or_else(|| header.strip_prefix("bearer ")) else {
        return false;
    };
    constant_time_eq(token.trim(), &expected)
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0_u8;
    for (a, b) in left.iter().zip(right) {
        diff |= a ^ b;
    }
    diff == 0
}

fn host_allowed(request: &HttpRequest, port: u16) -> bool {
    let Some(host) = request.header("host") else {
        return true;
    };
    let host = host.trim().to_ascii_lowercase();
    host == "127.0.0.1"
        || host == "localhost"
        || host == "[::1]"
        || host == format!("127.0.0.1:{port}")
        || host == format!("localhost:{port}")
        || host == format!("[::1]:{port}")
}

fn origin_allowed(request: &HttpRequest) -> bool {
    let Some(origin) = request.header("origin") else {
        return true;
    };
    let origin = origin.to_ascii_lowercase();
    origin == "tauri://localhost"
        || origin == "http://tauri.localhost"
        || origin == "https://tauri.localhost"
        || origin == "http://127.0.0.1"
        || origin == "http://localhost"
        || origin == "http://[::1]"
        || origin.starts_with("http://127.0.0.1:")
        || origin.starts_with("http://localhost:")
        || origin.starts_with("http://[::1]:")
}

fn handle_payload(state: &McpState, body: &Value) -> (u16, String) {
    let messages: Vec<&Value> = match body {
        Value::Array(items) => items.iter().collect(),
        other => vec![other],
    };
    let mut responses = Vec::new();
    for message in &messages {
        if let Some(response) = handle_message(state, message) {
            responses.push(response);
        }
    }
    if responses.is_empty() {
        return (202, String::new());
    }
    if matches!(body, Value::Array(_)) {
        (200, Value::Array(responses).to_string())
    } else {
        (200, responses.remove(0).to_string())
    }
}

fn handle_message(state: &McpState, message: &Value) -> Option<Value> {
    let Some(object) = message.as_object() else {
        return Some(error_response(&None, -32600, "请求必须是 JSON 对象"));
    };
    let id = object.get("id").cloned();
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return Some(error_response(&id, -32600, "缺少 method 字段"));
    };
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "initialize" => {
            let requested = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL_VERSION);
            let protocol = if SUPPORTED_VERSIONS.contains(&requested) {
                requested
            } else {
                PROTOCOL_VERSION
            };
            Some(success_response(
                &id,
                json!({
                    "protocolVersion": protocol,
                    "capabilities": {
                        "tools": { "listChanged": false },
                        "resources": { "listChanged": false, "subscribe": false },
                        "prompts": { "listChanged": false }
                    },
                    "serverInfo": {
                        "name": "controlvps",
                        "title": "ControlVPS",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "instructions": INSTRUCTIONS
                }),
            ))
        }
        "ping" => Some(success_response(&id, json!({}))),
        "tools/list" => Some(success_response(
            &id,
            json!({ "tools": tool_specs().into_iter().map(|spec| spec.to_json()).collect::<Vec<_>>() }),
        )),
        "tools/call" => Some(handle_tool_call(state, &id, &params)),
        "resources/list" => Some(success_response(&id, json!({ "resources": [] }))),
        "prompts/list" => Some(success_response(&id, json!({ "prompts": [] }))),
        "logging/setLevel" => Some(success_response(&id, json!({}))),
        "completion/complete" => Some(success_response(
            &id,
            json!({ "completion": { "values": [], "total": 0, "hasMore": false } }),
        )),
        other if other.starts_with("notifications/") => None,
        other => Some(error_response(
            &id,
            -32601,
            &format!("不支持的方法：{other}"),
        )),
    }
}

fn success_response(id: &Option<Value>, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id.clone().unwrap_or(Value::Null), "result": result })
}

fn error_response(id: &Option<Value>, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.clone().unwrap_or(Value::Null),
        "error": { "code": code, "message": message }
    })
}

fn handle_tool_call(state: &McpState, id: &Option<Value>, params: &Value) -> Value {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return error_response(id, -32602, "缺少工具名称");
    };
    if tool_specs().iter().all(|spec| spec.name != name) {
        return error_response(id, -32602, &format!("未知工具：{name}"));
    }
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if !arguments.is_object() && !arguments.is_null() {
        return error_response(id, -32602, "arguments 必须是 JSON 对象");
    }
    let started = Instant::now();
    let outcome = call_tool(state, name, &arguments);
    let duration_ms = started.elapsed().as_millis() as u64;
    if let Ok(record) = state.db.record_mcp_call(
        name,
        &outcome.target,
        !outcome.is_error,
        duration_ms,
        &outcome.summary,
    ) {
        if let Some(notify) = &state.notify {
            notify(record);
        }
    }
    success_response(
        id,
        json!({
            "content": [{ "type": "text", "text": outcome.text }],
            "structuredContent": {
                "tool": name,
                "target": outcome.target,
                "durationMs": duration_ms
            },
            "isError": outcome.is_error
        }),
    )
}

struct ToolSpec {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    write: bool,
    schema: Value,
}

impl ToolSpec {
    fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "title": self.title,
            "description": self.description,
            "inputSchema": self.schema,
            "annotations": {
                "title": self.title,
                "readOnlyHint": !self.write,
                "destructiveHint": self.write
            }
        })
    }
}

fn server_arg_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "server": { "type": "string", "description": "服务器 ID 或名称（名称可唯一前缀匹配）" }
        },
        "required": ["server"],
        "additionalProperties": false
    })
}

fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "list_servers",
            title: "列出服务器",
            description: "列出 ControlVPS 中已管理的服务器，可按分组、标签或关键字过滤。返回 id、名称、地址、分组、标签、成本与到期时间。",
            write: false,
            schema: json!({
                "type": "object",
                "properties": {
                    "group": { "type": "string", "description": "只看某个分组" },
                    "tag": { "type": "string", "description": "只看带某个标签的服务器" },
                    "query": { "type": "string", "description": "按名称、主机或备注模糊匹配" }
                },
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "get_server",
            title: "服务器详情",
            description: "读取单台服务器的完整配置、跳板路由、凭据类型和最近 5 条操作记录。不返回任何密码或私钥。",
            write: false,
            schema: server_arg_schema(),
        },
        ToolSpec {
            name: "collect_metrics",
            title: "采集指标",
            description: "连接服务器采集 CPU、内存、磁盘、负载与网络速率，并把采样写入监控历史。",
            write: false,
            schema: server_arg_schema(),
        },
        ToolSpec {
            name: "diagnose_connection",
            title: "诊断连接",
            description: "逐跳测试网络连通、SSH 握手、主机密钥与认证，返回每步耗时和结论。",
            write: false,
            schema: server_arg_schema(),
        },
        ToolSpec {
            name: "run_command",
            title: "执行命令",
            description: "在单台服务器上执行 shell 命令并返回 stdout、stderr 与退出码。属于写操作，需在应用中开启“允许执行命令与变更”。",
            write: true,
            schema: json!({
                "type": "object",
                "properties": {
                    "server": { "type": "string", "description": "服务器 ID 或名称" },
                    "command": { "type": "string", "description": "要执行的 shell 命令" },
                    "timeoutSeconds": { "type": "integer", "minimum": 1, "maximum": 600, "description": "超时秒数，默认 60" }
                },
                "required": ["server", "command"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "run_batch_command",
            title: "批量命令",
            description: "在多台服务器上并发执行同一条命令。可用 servers 列表，或按 group / tag 选择目标；单次最多 50 台、并发最多 10。属于写操作。",
            write: true,
            schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "要执行的 shell 命令" },
                    "servers": { "type": "array", "items": { "type": "string" }, "description": "服务器 ID 或名称列表" },
                    "group": { "type": "string", "description": "或按分组选择" },
                    "tag": { "type": "string", "description": "或按标签选择" },
                    "concurrency": { "type": "integer", "minimum": 1, "maximum": 10, "description": "并发数，默认 4" }
                },
                "required": ["command"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "list_systemd_services",
            title: "systemd 服务",
            description: "列出服务器上的 systemd 服务、运行状态、开机自启与最近失败的服务。",
            write: false,
            schema: server_arg_schema(),
        },
        ToolSpec {
            name: "list_docker_containers",
            title: "Docker 容器",
            description: "列出服务器上的 Docker 容器、镜像、状态、端口映射和 Compose 项目。",
            write: false,
            schema: server_arg_schema(),
        },
        ToolSpec {
            name: "collect_security_report",
            title: "安全审计",
            description: "只读巡检：系统与内核版本、待安装补丁、SSH 配置、防火墙状态、登录失败与高权限账户。",
            write: false,
            schema: server_arg_schema(),
        },
        ToolSpec {
            name: "list_tunnels",
            title: "隧道规则",
            description: "列出已配置的端口转发 / 动态代理规则及其目标服务器。",
            write: false,
            schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        },
        ToolSpec {
            name: "list_runbooks",
            title: "Runbook 清单",
            description: "列出可执行的 Runbook 及其步骤命令，便于选择后用 run_runbook 触发。",
            write: false,
            schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        },
        ToolSpec {
            name: "run_runbook",
            title: "运行 Runbook",
            description: "对指定服务器启动一个 Runbook 执行（后台并发执行），返回执行 ID，可用 get_runbook_execution 查询进度。属于写操作。",
            write: true,
            schema: json!({
                "type": "object",
                "properties": {
                    "runbook": { "type": "string", "description": "Runbook ID 或名称" },
                    "servers": { "type": "array", "items": { "type": "string" }, "description": "目标服务器 ID 或名称" },
                    "concurrency": { "type": "integer", "minimum": 1, "maximum": 10, "description": "并发数，默认 4" },
                    "timeoutSeconds": { "type": "integer", "minimum": 30, "maximum": 3600, "description": "单台超时秒数，默认 300" }
                },
                "required": ["runbook", "servers"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "get_runbook_execution",
            title: "Runbook 执行详情",
            description: "按执行 ID 查询 Runbook 执行状态、每台服务器的步骤结果和错误信息。",
            write: false,
            schema: json!({
                "type": "object",
                "properties": { "executionId": { "type": "string", "description": "run_runbook 返回的执行 ID" } },
                "required": ["executionId"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "list_deals",
            title: "VPS 优惠",
            description: "抓取最新 VPS 优惠信息（LowEndBox / LowEndTalk），返回价格、配置、机房与链接。会访问外网。",
            write: false,
            schema: json!({
                "type": "object",
                "properties": { "limit": { "type": "integer", "minimum": 1, "maximum": 50, "description": "返回条数，默认 15" } },
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "recent_operations",
            title: "操作记录",
            description: "读取最近的运维操作记录（动作、结果、耗时摘要），可按服务器过滤。",
            write: false,
            schema: json!({
                "type": "object",
                "properties": {
                    "server": { "type": "string", "description": "只看某台服务器" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 100, "description": "返回条数，默认 20" }
                },
                "additionalProperties": false
            }),
        },
    ]
}

struct ToolOutcome {
    text: String,
    is_error: bool,
    target: String,
    summary: String,
}

impl ToolOutcome {
    fn ok(target: impl Into<String>, summary: impl Into<String>, value: &Value) -> Self {
        Self {
            text: render(value),
            is_error: false,
            target: target.into(),
            summary: summary.into(),
        }
    }

    fn failure(target: impl Into<String>, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            text: message.clone(),
            is_error: true,
            target: target.into(),
            summary: truncate(&message, 300),
        }
    }
}

fn percent(used: u64, total: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    used as f64 / total as f64 * 100.0
}

fn render(value: &Value) -> String {
    truncate(
        &serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
        MAX_OUTPUT_CHARS,
    )
}

fn truncate(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    let truncated: String = value.chars().take(limit).collect();
    format!("{truncated}\n…输出已截断")
}

fn call_tool(state: &McpState, name: &str, args: &Value) -> ToolOutcome {
    let result = match name {
        "list_servers" => tool_list_servers(state, args),
        "get_server" => tool_get_server(state, args),
        "collect_metrics" => tool_collect_metrics(state, args),
        "diagnose_connection" => tool_diagnose(state, args),
        "run_command" => tool_run_command(state, args),
        "run_batch_command" => tool_run_batch_command(state, args),
        "list_systemd_services" => tool_list_systemd(state, args),
        "list_docker_containers" => tool_list_docker(state, args),
        "collect_security_report" => tool_security_report(state, args),
        "list_tunnels" => tool_list_tunnels(state),
        "list_runbooks" => tool_list_runbooks(state),
        "run_runbook" => tool_run_runbook(state, args),
        "get_runbook_execution" => tool_get_execution(state, args),
        "list_deals" => tool_list_deals(args),
        "recent_operations" => tool_recent_operations(state, args),
        other => Err(format!("未知工具：{other}")),
    };
    match result {
        Ok(outcome) => outcome,
        Err(message) => ToolOutcome::failure(arg_target(args), message),
    }
}

fn arg_target(args: &Value) -> String {
    args.get("server")
        .and_then(Value::as_str)
        .or_else(|| args.get("runbook").and_then(Value::as_str))
        .unwrap_or("")
        .to_string()
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("缺少参数 {key}"))
}

fn arg_opt_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn arg_usize(args: &Value, key: &str, default: usize, min: usize, max: usize) -> Result<usize, String> {
    let value = match args.get(key) {
        None | Some(Value::Null) => default,
        Some(Value::Number(number)) => number
            .as_u64()
            .ok_or_else(|| format!("{key} 必须是整数"))? as usize,
        Some(_) => return Err(format!("{key} 必须是整数")),
    };
    Ok(value.clamp(min, max))
}

fn resolve_server(servers: &[Server], query: &str) -> Result<Server, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("缺少服务器参数".into());
    }
    if let Some(server) = servers.iter().find(|server| server.id == query) {
        return Ok(server.clone());
    }
    if let Some(server) = servers
        .iter()
        .find(|server| server.name.eq_ignore_ascii_case(query))
    {
        return Ok(server.clone());
    }
    let lowered = query.to_lowercase();
    let matches: Vec<&Server> = servers
        .iter()
        .filter(|server| server.name.to_lowercase().contains(&lowered))
        .collect();
    match matches.len() {
        1 => Ok(matches[0].clone()),
        0 => Err(format!(
            "没有找到「{query}」对应的服务器。已管理：{}",
            if servers.is_empty() {
                "（暂无，请先在应用中添加 VPS）".to_string()
            } else {
                servers
                    .iter()
                    .map(|server| server.name.as_str())
                    .collect::<Vec<_>>()
                    .join("、")
            }
        )),
        _ => Err(format!(
            "「{query}」匹配到多台服务器：{}。请改用服务器 ID。",
            matches
                .iter()
                .map(|server| format!("{}（{}）", server.name, server.id))
                .collect::<Vec<_>>()
                .join("、")
        )),
    }
}

fn require_execute(state: &McpState) -> Result<(), String> {
    if state.config().allow_execute {
        Ok(())
    } else {
        Err("写操作已关闭：请在 ControlVPS「AI 接入」页开启“允许执行命令与变更”后重试。".into())
    }
}

fn error_text(error: AppError) -> String {
    match error {
        AppError::UnknownHostKey { host, fingerprint } => format!(
            "服务器 {host} 的主机密钥尚未确认（指纹 {fingerprint}）。请先在 ControlVPS 中手动连接一次完成确认；也可以在「AI 接入」页开启“自动信任未知主机”，但不建议。"
        ),
        AppError::ChangedHostKey { host, fingerprint } => format!(
            "服务器 {host} 的主机密钥已改变（新指纹 {fingerprint}），已阻止连接。请核实服务器是否重装或被劫持。"
        ),
        other => other.to_string(),
    }
}

fn connect(
    state: &McpState,
    server: &Server,
) -> Result<(Server, ssh::SshConnection), String> {
    let job = runbook::connection_job(&state.db, &server.id).map_err(error_text)?;
    let connection = ssh::connect(
        &job.server,
        &job.credentials,
        &job.jumps,
        &state.known_hosts,
        state.allow_unknown_host(),
    )
    .map_err(error_text)?;
    Ok((job.server, connection))
}

fn server_view(server: &Server, servers: &[Server]) -> Value {
    let route = server
        .jump_chain
        .iter()
        .filter_map(|id| servers.iter().find(|item| &item.id == id))
        .map(|item| item.name.clone())
        .collect::<Vec<_>>();
    json!({
        "id": server.id,
        "name": server.name,
        "group": server.group_name,
        "host": server.host,
        "port": server.port,
        "username": server.username,
        "authMethod": match server.auth_method {
            crate::models::AuthMethod::Password => "password",
            crate::models::AuthMethod::Key => "key",
            crate::models::AuthMethod::Agent => "agent",
        },
        "tags": server.tags,
        "provider": server.provider,
        "region": server.region,
        "cost": server.cost,
        "currency": server.currency,
        "expiresAt": server.expires_at,
        "jumpRoute": route,
        "notes": server.notes,
        "updatedAt": server.updated_at,
    })
}

fn tool_list_servers(state: &McpState, args: &Value) -> Result<ToolOutcome, String> {
    let servers = state.db.list_servers().map_err(error_text)?;
    let group = arg_opt_str(args, "group");
    let tag = arg_opt_str(args, "tag");
    let query = arg_opt_str(args, "query").map(str::to_lowercase);
    let selected: Vec<&Server> = servers
        .iter()
        .filter(|server| group.is_none_or(|group| server.group_name.as_deref() == Some(group)))
        .filter(|server| {
            tag.is_none_or(|tag| server.tags.iter().any(|item| item.eq_ignore_ascii_case(tag)))
        })
        .filter(|server| {
            query.as_deref().is_none_or(|query| {
                server.name.to_lowercase().contains(query)
                    || server.host.to_lowercase().contains(query)
                    || server
                        .notes
                        .as_deref()
                        .is_some_and(|notes| notes.to_lowercase().contains(query))
            })
        })
        .collect();
    let payload = json!({
        "count": selected.len(),
        "servers": selected.iter().map(|server| server_view(server, &servers)).collect::<Vec<_>>(),
    });
    Ok(ToolOutcome::ok(
        "",
        format!("列出 {} 台服务器", selected.len()),
        &payload,
    ))
}

fn tool_get_server(state: &McpState, args: &Value) -> Result<ToolOutcome, String> {
    let servers = state.db.list_servers().map_err(error_text)?;
    let server = resolve_server(&servers, arg_str(args, "server")?)?;
    let operations = state
        .db
        .list_operation_logs(Some(&server.id))
        .unwrap_or_default()
        .into_iter()
        .take(5)
        .map(|log| {
            json!({
                "action": log.action,
                "success": log.success,
                "summary": log.summary,
                "createdAt": log.created_at,
            })
        })
        .collect::<Vec<_>>();
    let payload = json!({
        "server": server_view(&server, &servers),
        "recentOperations": operations,
    });
    Ok(ToolOutcome::ok(
        server.name.clone(),
        format!("读取 {} 的详情", server.name),
        &payload,
    ))
}

fn tool_collect_metrics(state: &McpState, args: &Value) -> Result<ToolOutcome, String> {
    let servers = state.db.list_servers().map_err(error_text)?;
    let server = resolve_server(&servers, arg_str(args, "server")?)?;
    let (server, connection) = connect(state, &server)?;
    let metrics = metrics::collect(&server.id, &connection).map_err(error_text)?;
    let _ = state.db.insert_metric_sample(&metrics);
    let payload = serde_json::to_value(&metrics).map_err(|error| error.to_string())?;
    Ok(ToolOutcome::ok(
        server.name.clone(),
        format!(
            "采集指标：CPU {:.1}%、内存 {:.1}%、磁盘 {:.1}%",
            metrics.cpu_usage,
            percent(metrics.memory_used, metrics.memory_total),
            percent(metrics.disk_used, metrics.disk_total)
        ),
        &payload,
    ))
}

fn tool_diagnose(state: &McpState, args: &Value) -> Result<ToolOutcome, String> {
    let servers = state.db.list_servers().map_err(error_text)?;
    let server = resolve_server(&servers, arg_str(args, "server")?)?;
    let job = runbook::connection_job(&state.db, &server.id).map_err(error_text)?;
    let report = ssh::diagnose(
        &job.server,
        &job.credentials,
        &job.jumps,
        &state.known_hosts,
        state.allow_unknown_host(),
    )
    .map_err(error_text)?;
    let payload = serde_json::to_value(&report).map_err(|error| error.to_string())?;
    Ok(ToolOutcome::ok(
        server.name.clone(),
        format!("诊断 {} 的连接", server.name),
        &payload,
    ))
}

fn tool_run_command(state: &McpState, args: &Value) -> Result<ToolOutcome, String> {
    require_execute(state)?;
    let command = arg_str(args, "command")?;
    if command.len() > 100_000 {
        return Err("命令不能超过 100000 字节".into());
    }
    let timeout = arg_usize(args, "timeoutSeconds", 60, 1, 600)?;
    let servers = state.db.list_servers().map_err(error_text)?;
    let server = resolve_server(&servers, arg_str(args, "server")?)?;
    let (server, connection) = connect(state, &server)?;
    let started = Instant::now();
    let output = ssh::execute_controlled(
        &connection.session,
        command,
        Duration::from_secs(timeout as u64),
        &AtomicBool::new(false),
    )
    .map_err(error_text)?;
    let duration_ms = started.elapsed().as_millis() as u64;
    let result = match output {
        ssh::ControlledExecOutput::Completed(output) => json!({
            "stdout": truncate(&output.stdout, MAX_OUTPUT_CHARS),
            "stderr": truncate(&output.stderr, MAX_OUTPUT_CHARS),
            "exitCode": output.exit_code,
            "timedOut": false,
            "durationMs": duration_ms,
        }),
        ssh::ControlledExecOutput::TimedOut { stdout, stderr } => json!({
            "stdout": truncate(&stdout, MAX_OUTPUT_CHARS),
            "stderr": truncate(&stderr, MAX_OUTPUT_CHARS),
            "exitCode": Value::Null,
            "timedOut": true,
            "durationMs": duration_ms,
        }),
        ssh::ControlledExecOutput::Cancelled { stdout, stderr } => json!({
            "stdout": truncate(&stdout, MAX_OUTPUT_CHARS),
            "stderr": truncate(&stderr, MAX_OUTPUT_CHARS),
            "exitCode": Value::Null,
            "timedOut": false,
            "durationMs": duration_ms,
        }),
    };
    let exit_code = result
        .get("exitCode")
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    let timed_out = result
        .get("timedOut")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let success = exit_code == 0 && !timed_out;
    let cmd_preview = command_preview(command);
    let summary = if timed_out {
        format!("执行超时（{timeout} 秒），耗时 {duration_ms} ms · {cmd_preview}")
    } else {
        format!("命令退出码 {exit_code}，耗时 {duration_ms} ms · {cmd_preview}")
    };
    let _ = state
        .db
        .log_operation(&server.id, "MCP · 执行命令", success, &summary);
    if success {
        Ok(ToolOutcome::ok(server.name.clone(), summary, &result))
    } else {
        Ok(ToolOutcome {
            text: render(&result),
            is_error: true,
            target: server.name.clone(),
            summary,
        })
    }
}

fn tool_run_batch_command(state: &McpState, args: &Value) -> Result<ToolOutcome, String> {
    require_execute(state)?;
    let command = arg_str(args, "command")?;
    if command.len() > 100_000 {
        return Err("命令不能超过 100000 字节".into());
    }
    let concurrency = arg_usize(args, "concurrency", 4, 1, 10)?;
    let targets = resolve_targets(state, args)?;
    if targets.len() > 50 {
        return Err("单次批量执行最多 50 台服务器".into());
    }
    let mut jobs = Vec::with_capacity(targets.len());
    for server in &targets {
        jobs.push(runbook::connection_job(&state.db, &server.id).map_err(error_text)?);
    }
    let result = batch::execute(jobs, command.to_string(), state.known_hosts.clone(), concurrency)
        .map_err(error_text)?;
    let cmd_preview = command_preview(command);
    for item in &result.results {
        let summary = match item.exit_code {
            Some(code) => format!("命令退出码 {code}，耗时 {} ms · {cmd_preview}", item.duration_ms),
            None => item.error.clone().unwrap_or_else(|| "连接失败".into()),
        };
        let _ = state
            .db
            .log_operation(&item.server_id, "MCP · 批量命令", item.success, &summary);
    }
    let payload = serde_json::to_value(&result).map_err(|error| error.to_string())?;
    let summary = format!(
        "批量命令：成功 {} 台，失败 {} 台，耗时 {} ms · {cmd_preview}",
        result.success_count, result.failure_count, result.duration_ms
    );
    if result.failure_count > 0 && result.success_count == 0 {
        Ok(ToolOutcome {
            text: render(&payload),
            is_error: true,
            target: format!("{} 台服务器", targets.len()),
            summary,
        })
    } else {
        Ok(ToolOutcome::ok(
            format!("{} 台服务器", targets.len()),
            summary,
            &payload,
        ))
    }
}

fn tool_list_systemd(state: &McpState, args: &Value) -> Result<ToolOutcome, String> {
    let servers = state.db.list_servers().map_err(error_text)?;
    let server = resolve_server(&servers, arg_str(args, "server")?)?;
    let (server, connection) = connect(state, &server)?;
    let overview = systemd::list(&connection.session).map_err(error_text)?;
    let payload = serde_json::to_value(&overview).map_err(|error| error.to_string())?;
    Ok(ToolOutcome::ok(
        server.name.clone(),
        format!("读取 {} 的 systemd 服务", server.name),
        &payload,
    ))
}

fn tool_list_docker(state: &McpState, args: &Value) -> Result<ToolOutcome, String> {
    let servers = state.db.list_servers().map_err(error_text)?;
    let server = resolve_server(&servers, arg_str(args, "server")?)?;
    let (server, connection) = connect(state, &server)?;
    let overview = docker::list(&connection.session).map_err(error_text)?;
    let payload = serde_json::to_value(&overview).map_err(|error| error.to_string())?;
    Ok(ToolOutcome::ok(
        server.name.clone(),
        format!("读取 {} 的 Docker 容器", server.name),
        &payload,
    ))
}

fn tool_security_report(state: &McpState, args: &Value) -> Result<ToolOutcome, String> {
    let servers = state.db.list_servers().map_err(error_text)?;
    let server = resolve_server(&servers, arg_str(args, "server")?)?;
    let (server, connection) = connect(state, &server)?;
    let report = security::collect(&server.id, &connection.session).map_err(error_text)?;
    let payload = serde_json::to_value(&report).map_err(|error| error.to_string())?;
    Ok(ToolOutcome::ok(
        server.name.clone(),
        format!("完成 {} 的安全审计", server.name),
        &payload,
    ))
}

fn tool_list_tunnels(state: &McpState) -> Result<ToolOutcome, String> {
    let rules = state.db.list_port_forwards().map_err(error_text)?;
    let servers = state.db.list_servers().map_err(error_text)?;
    let items = rules
        .iter()
        .map(|rule| {
            let server_name = servers
                .iter()
                .find(|server| server.id == rule.server_id)
                .map(|server| server.name.clone())
                .unwrap_or_else(|| rule.server_id.clone());
            json!({
                "id": rule.id,
                "name": rule.name,
                "server": server_name,
                "kind": match rule.kind {
                    PortForwardKind::Local => "local",
                    PortForwardKind::Remote => "remote",
                    PortForwardKind::Dynamic => "dynamic",
                },
                "listen": format!("{}:{}", rule.listen_address, rule.listen_port),
                "destination": format!("{}:{}", rule.destination_host, rule.destination_port),
                "updatedAt": rule.updated_at,
            })
        })
        .collect::<Vec<_>>();
    let payload = json!({ "count": items.len(), "rules": items });
    Ok(ToolOutcome::ok(
        "",
        format!("列出 {} 条隧道规则", rules.len()),
        &payload,
    ))
}

fn tool_list_runbooks(state: &McpState) -> Result<ToolOutcome, String> {
    let runbooks = state.db.list_runbooks().map_err(error_text)?;
    let items = runbooks
        .iter()
        .map(|runbook| {
            json!({
                "id": runbook.id,
                "name": runbook.name,
                "description": runbook.description,
                "stepCount": runbook.steps.len(),
                "steps": runbook.steps.iter().map(|step| json!({
                    "name": step.name,
                    "command": step.command,
                })).collect::<Vec<_>>(),
                "updatedAt": runbook.updated_at,
            })
        })
        .collect::<Vec<_>>();
    let payload = json!({ "count": items.len(), "runbooks": items });
    Ok(ToolOutcome::ok(
        "",
        format!("列出 {} 个 Runbook", runbooks.len()),
        &payload,
    ))
}

fn tool_run_runbook(state: &McpState, args: &Value) -> Result<ToolOutcome, String> {
    require_execute(state)?;
    let query = arg_str(args, "runbook")?;
    let runbooks = state.db.list_runbooks().map_err(error_text)?;
    let runbook = runbooks
        .iter()
        .find(|item| item.id == query)
        .or_else(|| {
            runbooks
                .iter()
                .find(|item| item.name.eq_ignore_ascii_case(query))
        })
        .cloned()
        .ok_or_else(|| format!("没有找到 Runbook「{query}」"))?;
    let targets = resolve_targets(state, args)?;
    let concurrency = arg_usize(args, "concurrency", 4, 1, 10)?;
    let timeout_seconds = arg_usize(args, "timeoutSeconds", 300, 30, 3600)? as u64;
    let request = RunbookExecutionRequest {
        runbook: runbook.clone(),
        target_ids: targets.iter().map(|server| server.id.clone()).collect(),
        concurrency,
        canary_count: 0,
        timeout_seconds,
        trigger: RunbookTrigger::Manual,
        schedule_id: None,
        retry_of: None,
    };
    let execution = runbook::start_execution(
        state.db.clone(),
        state.known_hosts.clone(),
        state.registry.clone(),
        request,
    )
    .map_err(error_text)?;
    let payload = json!({
        "executionId": execution.id,
        "runbook": execution.runbook_name,
        "status": execution.status,
        "targets": targets.iter().map(|server| server.name.clone()).collect::<Vec<_>>(),
        "hint": "使用 get_runbook_execution 轮询执行进度",
    });
    Ok(ToolOutcome::ok(
        runbook.name.clone(),
        format!(
            "启动 Runbook「{}」，目标 {} 台",
            runbook.name,
            targets.len()
        ),
        &payload,
    ))
}

fn tool_get_execution(state: &McpState, args: &Value) -> Result<ToolOutcome, String> {
    let id = arg_str(args, "executionId")?;
    let execution = state.db.get_runbook_execution(id).map_err(error_text)?;
    let payload = serde_json::to_value(&execution).map_err(|error| error.to_string())?;
    Ok(ToolOutcome::ok(
        execution.runbook_name.clone(),
        format!(
            "Runbook「{}」状态：{:?}",
            execution.runbook_name, execution.status
        ),
        &payload,
    ))
}

fn tool_list_deals(args: &Value) -> Result<ToolOutcome, String> {
    let limit = arg_usize(args, "limit", 15, 1, 50)?;
    let feed = discovery::fetch_deals().map_err(error_text)?;
    let deals = feed
        .deals
        .iter()
        .take(limit)
        .map(|deal| {
            json!({
                "title": deal.title,
                "source": deal.source_name,
                "price": deal.price,
                "memoryGb": deal.memory_gb,
                "diskGb": deal.disk_gb,
                "cpuCores": deal.cpu_cores,
                "locations": deal.locations,
                "publishedAt": deal.published_at,
                "url": deal.url,
            })
        })
        .collect::<Vec<_>>();
    let payload = json!({
        "fetchedAt": feed.fetched_at,
        "sources": feed.sources.iter().map(|source| json!({
            "name": source.name,
            "ok": source.ok,
            "message": source.message,
            "items": source.item_count,
        })).collect::<Vec<_>>(),
        "deals": deals,
    });
    Ok(ToolOutcome::ok(
        "",
        format!("返回 {} 条 VPS 优惠", deals.len()),
        &payload,
    ))
}

fn tool_recent_operations(state: &McpState, args: &Value) -> Result<ToolOutcome, String> {
    let limit = arg_usize(args, "limit", 20, 1, 100)?;
    let server_id = match arg_opt_str(args, "server") {
        Some(query) => {
            let servers = state.db.list_servers().map_err(error_text)?;
            Some(resolve_server(&servers, query)?.id)
        }
        None => None,
    };
    let logs = state
        .db
        .list_operation_logs(server_id.as_deref())
        .map_err(error_text)?;
    let items = logs
        .iter()
        .take(limit)
        .map(|log| {
            json!({
                "server": log.server_name,
                "action": log.action,
                "success": log.success,
                "summary": log.summary,
                "createdAt": log.created_at,
            })
        })
        .collect::<Vec<_>>();
    let payload = json!({ "count": items.len(), "operations": items });
    Ok(ToolOutcome::ok(
        "",
        format!("返回 {} 条操作记录", items.len()),
        &payload,
    ))
}

fn resolve_targets(state: &McpState, args: &Value) -> Result<Vec<Server>, String> {
    let servers = state.db.list_servers().map_err(error_text)?;
    let mut selected: Vec<Server> = Vec::new();
    if let Some(group) = arg_opt_str(args, "group") {
        selected.extend(
            servers
                .iter()
                .filter(|server| server.group_name.as_deref() == Some(group))
                .cloned(),
        );
    }
    if let Some(tag) = arg_opt_str(args, "tag") {
        selected.extend(
            servers
                .iter()
                .filter(|server| server.tags.iter().any(|item| item.eq_ignore_ascii_case(tag)))
                .cloned(),
        );
    }
    if let Some(list) = args.get("servers").and_then(Value::as_array) {
        for item in list {
            let query = item
                .as_str()
                .ok_or_else(|| "servers 数组元素必须是字符串".to_string())?;
            selected.push(resolve_server(&servers, query)?);
        }
    }
    let mut unique: Vec<Server> = Vec::new();
    for server in selected {
        if !unique.iter().any(|item| item.id == server.id) {
            unique.push(server);
        }
    }
    if unique.is_empty() {
        return Err("没有选中任何服务器：请提供 servers 列表，或 group / tag 过滤条件".into());
    }
    Ok(unique)
}

fn command_preview(command: &str) -> String {
    let single_line: String = command
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let count = single_line.chars().count();
    if count <= 120 {
        single_line
    } else {
        format!("{}…", single_line.chars().take(120).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req_with_headers(headers: Vec<(&str, &str)>) -> HttpRequest {
        HttpRequest {
            method: "POST".into(),
            path: "/mcp".into(),
            headers: headers
                .into_iter()
                .map(|(k, v)| (k.to_ascii_lowercase(), v.to_string()))
                .collect(),
            body: Vec::new(),
        }
    }

    #[test]
    fn origin_rejects_null_and_external_origins() {
        assert!(!origin_allowed(&req_with_headers(vec![("origin", "null")])));
        assert!(!origin_allowed(&req_with_headers(vec![(
            "origin",
            "https://evil.example"
        )])));
        assert!(origin_allowed(&req_with_headers(vec![(
            "origin",
            "http://127.0.0.1:1420"
        )])));
        assert!(origin_allowed(&req_with_headers(vec![(
            "origin",
            "tauri://localhost"
        )])));
        assert!(origin_allowed(&req_with_headers(vec![])));
    }

    #[test]
    fn host_rejects_dns_rebinding_hosts() {
        assert!(!host_allowed(
            &req_with_headers(vec![("host", "rebind.attacker.com:8799")]),
            8799
        ));
        assert!(!host_allowed(
            &req_with_headers(vec![("host", "127.0.0.1:9999")]),
            8799
        ));
        assert!(host_allowed(
            &req_with_headers(vec![("host", "127.0.0.1:8799")]),
            8799
        ));
        assert!(host_allowed(
            &req_with_headers(vec![("host", "localhost:8799")]),
            8799
        ));
    }

    #[test]
    fn constant_time_eq_works() {
        assert!(constant_time_eq("secret-token", "secret-token"));
        assert!(!constant_time_eq("secret-token", "secret-tokeN"));
        assert!(!constant_time_eq("short", "longer"));
    }

    #[test]
    fn mcp_read_only_blocks_execute_tools() {
        let db = Database::in_memory().expect("in-memory db");
        let state = McpState {
            db,
            known_hosts: PathBuf::from("/tmp/nonexistent"),
            registry: RunbookExecutionRegistry::default(),
            config: Mutex::new(McpConfig {
                enabled: true,
                port: 8799,
                allow_execute: false,
                allow_unknown_host: false,
                token: "test-token".into(),
            }),
            notify: None,
        };
        let resp = handle_message(
            &state,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "run_command",
                    "arguments": { "server": "demo", "command": "id" }
                }
            }),
        )
        .expect("response");
        assert_eq!(resp["result"]["isError"], Value::Bool(true));
        assert!(resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("写操作已关闭"));
    }
}

