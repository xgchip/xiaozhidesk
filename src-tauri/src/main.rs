#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod audio;
mod protocol;
mod voice;
mod window_mode;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{sync::atomic::Ordering, time::Duration};
use tauri::{Emitter, Manager, State};
use tokio::sync::{mpsc, watch, Mutex};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

#[derive(Clone, Serialize, Deserialize)]
struct Identity {
    device_id: String,
    client_id: String,
}
#[derive(Clone, Deserialize)]
struct Connection {
    url: String,
    #[serde(default)]
    token: String,
    #[serde(default = "default_version")]
    version: u32,
}
fn default_version() -> u32 {
    1
}
enum Outgoing {
    Begin(u32),
    Audio(u32, Vec<u8>),
    End(u32),
    Abort(u32),
    Text(u32, String),
}
struct Session {
    id: String,
    session_id: String,
    cancel: watch::Sender<bool>,
    audio: audio::Audio,
}
fn wire_messages(
    outgoing: Outgoing,
    latest_epoch: u32,
    active: &mut Option<u32>,
    session_id: &str,
    version: u32,
) -> Vec<Message> {
    let text = |value: Value| Message::Text(value.to_string().into());
    let abort = || text(json!({"type":"abort","session_id":session_id}));
    match outgoing {
        Outgoing::Begin(epoch) if epoch == latest_epoch => {
            *active = Some(epoch);
            vec![
                abort(),
                text(
                    json!({"type":"listen","state":"start","mode":"manual","session_id":session_id}),
                ),
            ]
        }
        Outgoing::Audio(epoch, data) if epoch == latest_epoch && *active == Some(epoch) => {
            vec![Message::Binary(protocol::pack(&data, version).into())]
        }
        Outgoing::End(epoch) if epoch == latest_epoch && *active == Some(epoch) => {
            *active = None;
            vec![text(
                json!({"type":"listen","state":"stop","session_id":session_id}),
            )]
        }
        Outgoing::Abort(epoch) if epoch == latest_epoch => {
            *active = None;
            vec![abort()]
        }
        Outgoing::Text(epoch, content) if epoch == latest_epoch => {
            *active = None;
            vec![
                abort(),
                text(
                    json!({"type":"listen","state":"detect","text":content,"session_id":session_id}),
                ),
            ]
        }
        _ => Vec::new(),
    }
}
struct Inner {
    identity: Identity,
    connection: Option<Connection>,
    session: Option<Session>,
}
struct Device(Mutex<Inner>);

fn event(app: &tauri::AppHandle, value: Value) {
    let _ = app.emit("device-event", value);
}
fn error_string(e: impl std::fmt::Display) -> String {
    e.to_string()
}
type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
async fn send_ws(socket: &mut Socket, message: Message) -> Result<(), String> {
    tokio::time::timeout(Duration::from_secs(10), socket.send(message))
        .await
        .map_err(|_| "网络发送超时".to_string())?
        .map_err(|_| "网络发送失败".to_string())
}
fn valid_url(url: &str, websocket: bool) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|_| "服务地址无效")?;
    let valid = if websocket {
        parsed.scheme() == "wss"
            || (parsed.scheme() == "ws"
                && matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")))
    } else {
        parsed.scheme() == "https"
            || (parsed.scheme() == "http"
                && matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")))
    };
    if !valid
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("远程服务必须使用 TLS；仅本机调试允许 HTTP / WS".into());
    }
    Ok(())
}
fn http(identity: &Identity) -> Result<reqwest::Client, String> {
    use reqwest::header::{HeaderMap, HeaderValue};
    let mut headers = HeaderMap::new();
    for (key, value) in [
        ("activation-version", "1"),
        ("device-id", identity.device_id.as_str()),
        ("client-id", identity.client_id.as_str()),
        ("accept-language", "zh-CN"),
    ] {
        headers.insert(key, HeaderValue::from_str(value).map_err(error_string)?);
    }
    reqwest::Client::builder()
        .default_headers(headers)
        .user_agent(concat!("xiaozhi-desktop/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(error_string)
}

#[tauri::command]
async fn get_device(state: State<'_, Device>) -> Result<Value, String> {
    let inner = state.0.lock().await;
    Ok(
        json!({"identity":inner.identity,"audio":audio::devices(),"connected":inner.session.is_some(),"session_id":inner.session.as_ref().map(|s|s.session_id.as_str()),
        "call_enabled":inner.session.as_ref().is_some_and(|s|s.audio.call_enabled.load(Ordering::Relaxed))}),
    )
}

#[tauri::command]
async fn discover(ota_url: String, state: State<'_, Device>) -> Result<Value, String> {
    valid_url(&ota_url, false)?;
    let mut inner = state.0.lock().await;
    if inner.session.is_some() {
        return Err("请先断开当前连接".into());
    }
    inner.connection = None;
    let id = &inner.identity;
    let response=http(id)?.post(&ota_url).json(&json!({"version":2,"language":"zh-CN","flash_size":16777216,
        "minimum_free_heap_size":8000000,"mac_address":id.device_id,"uuid":id.client_id,"chip_model_name":"esp32s3",
        "chip_info":{"model":9,"cores":2,"revision":0,"features":0},
        "application":{"name":"xiaozhi-desktop","version":"2.0.0","idf_version":"desktop","elf_sha256":""},
        "partition_table":[],"ota":{"label":"ota_0"},"board":{"type":"bread-compact-wifi","name":"xiaozhi-desktop","ssid":"Desktop","rssi":-30,"channel":1,"ip":"127.0.0.1","mac":id.device_id}}))
        .send().await.map_err(|e|e.without_url().to_string())?;
    if !response.status().is_success() {
        return Err(format!("OTA 服务返回 HTTP {}", response.status().as_u16()));
    }
    if response.content_length().unwrap_or(0) > 262144 {
        return Err("OTA 响应过大".into());
    }
    let mut response = response;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "读取 OTA 响应失败")? {
        if bytes.len() + chunk.len() > 262144 {
            return Err("OTA 响应过大".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let body: Value = serde_json::from_slice(&bytes).map_err(|_| "OTA 服务返回的 JSON 无效")?;
    let activation = body.get("activation").cloned().unwrap_or(Value::Null);
    let pending = activation
        .get("code")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty());
    if !pending {
        if let Some(ws) = body.get("websocket") {
            let config: Connection =
                serde_json::from_value(ws.clone()).map_err(|_| "后台 WebSocket 配置无效")?;
            valid_url(&config.url, true)?;
            if !(1..=3).contains(&config.version) {
                return Err("后台协议版本不受支持".into());
            }
            inner.connection = Some(config);
        }
    }
    Ok(
        json!({"activation":activation,"ready":inner.connection.is_some(),"mqtt_only":body.get("mqtt").is_some() && body.get("websocket").is_none()}),
    )
}

#[tauri::command]
async fn activate(ota_url: String, state: State<'_, Device>) -> Result<bool, String> {
    valid_url(&ota_url, false)?;
    let identity = state.0.lock().await.identity.clone();
    let response = http(&identity)?
        .post(format!("{}/activate", ota_url.trim_end_matches('/')))
        .json(&json!({}))
        .send()
        .await
        .map_err(|e| e.without_url().to_string())?;
    match response.status().as_u16() {
        200 => Ok(true),
        202 => Ok(false),
        status => Err(format!("激活接口返回 HTTP {status}，请重新获取激活码")),
    }
}

async fn open_websocket(
    config: &Connection,
    identity: &Identity,
) -> Result<(Socket, Value), String> {
    valid_url(&config.url, true)?;
    let mut request = config
        .url
        .as_str()
        .into_client_request()
        .map_err(|_| "WebSocket 地址无效")?;
    for (key, value) in [
        ("Device-Id", identity.device_id.clone()),
        ("Client-Id", identity.client_id.clone()),
        ("Protocol-Version", config.version.to_string()),
    ] {
        request
            .headers_mut()
            .insert(key, value.parse().map_err(error_string)?);
    }
    if !config.token.is_empty() {
        let token = if config.token.contains(' ') {
            config.token.clone()
        } else {
            format!("Bearer {}", config.token)
        };
        request
            .headers_mut()
            .insert("Authorization", token.parse().map_err(error_string)?);
    }
    let mut socket_config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default();
    socket_config.max_message_size = Some(262144);
    socket_config.max_frame_size = Some(262144);
    let (mut socket, _) = tokio::time::timeout(
        Duration::from_secs(15),
        tokio_tungstenite::connect_async_with_config(request, Some(socket_config), false),
    )
    .await
    .map_err(|_| "连接超时")?
    .map_err(|_| "WebSocket 连接失败，请检查网络或重新激活")?;
    socket
        .send(Message::Text(
            protocol::hello(config.version).to_string().into(),
        ))
        .await
        .map_err(error_string)?;
    let server_hello = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(text))) => {
                    let value: Value =
                        serde_json::from_str(&text).map_err(|_| "后台返回无效 JSON")?;
                    if value["type"] == "hello" {
                        return if value["transport"] == "websocket" {
                            Ok(value)
                        } else {
                            Err("后台传输协议不匹配")
                        };
                    }
                }
                Some(Ok(Message::Ping(p))) => {
                    socket
                        .send(Message::Pong(p))
                        .await
                        .map_err(|_| "握手已中断")?;
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                    return Err("后台在握手时断开连接")
                }
                _ => {}
            }
        }
    })
    .await
    .map_err(|_| "等待后台 hello 超时")??;
    Ok((socket, server_hello))
}

#[tauri::command]
async fn connect_device(
    app: tauri::AppHandle,
    state: State<'_, Device>,
    input: String,
    output: String,
    volume: u32,
    silence_ms: Option<u32>,
) -> Result<(), String> {
    let silence_ms = voice::validate_silence(silence_ms.unwrap_or(voice::DEFAULT_SILENCE_MS))?;
    let mut inner = state.0.lock().await;
    if inner.session.is_some() {
        return Err("设备已经连接".into());
    }
    let config = inner
        .connection
        .clone()
        .ok_or("请先获取设备配置并完成激活")?;
    let (mut socket, server_hello) = open_websocket(&config, &inner.identity).await?;
    let session_id = server_hello["session_id"].as_str().unwrap_or("").to_owned();
    let (tx, mut rx) = mpsc::channel(64);
    let (cancel, mut cancel_rx) = watch::channel(false);
    let audio_app = app.clone();
    let audio_tx = tx.clone();
    let audio = tauri::async_runtime::spawn_blocking(move || {
        audio::start(
            audio_app,
            audio_tx,
            input,
            output,
            volume.min(100),
            silence_ms,
        )
    })
    .await
    .map_err(error_string)??;
    let id = uuid::Uuid::new_v4().to_string();
    let worker_id = id.clone();
    let (listening, running, speaker_volume, playback, incoming) = (
        audio.listening.clone(),
        audio.running.clone(),
        audio.volume.clone(),
        audio.playback.clone(),
        audio.incoming.clone(),
    );
    let epoch = audio.epoch.clone();
    let suppressed = audio.suppressed.clone();
    let response_expected = audio.response_expected.clone();
    inner.session = Some(Session {
        id,
        session_id: session_id.clone(),
        cancel,
        audio,
    });
    event(
        &app,
        json!({"type":"connected","session_id":session_id,"audio_params":server_hello["audio_params"]}),
    );
    tauri::async_runtime::spawn(async move {
        let mut last_incoming = tokio::time::Instant::now();
        let mut active_epoch = None;
        let mut keepalive = tokio::time::interval(Duration::from_secs(25));
        keepalive.tick().await;
        let result=async {
            loop {
                tokio::select! {
                    _=cancel_rx.changed()=>break,
                    _=keepalive.tick()=>send_ws(&mut socket,Message::Ping(Vec::new().into())).await?,
                    outgoing=rx.recv()=>match outgoing {
                        Some(value)=>for message in wire_messages(value,epoch.load(Ordering::Relaxed),&mut active_epoch,&session_id,config.version) {send_ws(&mut socket,message).await?;},
                        None=>break,
                    },
                    message=async {
                        let result=tokio::time::timeout_at(last_incoming+Duration::from_secs(120),socket.next()).await;
                        if matches!(result,Ok(Some(Ok(_)))) {last_incoming=tokio::time::Instant::now();}
                        result
                    }=>match message {
                        Ok(Some(Ok(Message::Text(text))))=>{
                            if text.len()>262144 {return Err("后台消息过大".to_string());}
                            let value:Value=serde_json::from_str(&text).map_err(|_|"后台返回无效 JSON")?;
                            if value["type"]=="mcp" {
                                let mut vol=speaker_volume.load(Ordering::Relaxed);
                                if let Some(payload)=protocol::mcp_response(&value["payload"],&mut vol) {
                                    speaker_volume.store(vol,Ordering::Relaxed);
                                    event(&app,json!({"type":"volume","value":vol}));
                                    send_ws(&mut socket,Message::Text(json!({"type":"mcp","session_id":session_id,"payload":payload}).to_string().into())).await?;
                                }
                            } else if value["type"]=="goodbye" {break;} else {
                                if value["type"]=="tts" {
                                    if !response_expected.load(Ordering::Relaxed) || listening.load(Ordering::Relaxed) {continue;}
                                    if value["state"]=="start" {suppressed.store(false,Ordering::Relaxed);}
                                }
                                event(&app,value);
                            }
                        }
                        Ok(Some(Ok(Message::Binary(data))))=>{
                            let packet=protocol::unpack(&data,config.version)?;
                            incoming.try_send(packet.to_vec()).map_err(|_|"音频解码队列已满或不可用")?;
                        }
                        Ok(Some(Ok(Message::Ping(p))))=>send_ws(&mut socket,Message::Pong(p)).await?,
                        Ok(Some(Ok(Message::Close(_))))|Ok(None)=>break,
                        Ok(Some(Err(_)))=>return Err("网络连接中断，请重新连接".into()),
                        Err(_)=>return Err("后台响应超时，请重新连接".into()),_=>{}
                    }
                }
            }
            Ok::<(),String>(())
        }.await;
        listening.store(false, Ordering::Relaxed);
        running.store(false, Ordering::Relaxed);
        if let Ok(mut queue) = playback.lock() {
            queue.clear();
        }
        let _ = tokio::time::timeout(Duration::from_secs(2), socket.close(None)).await;
        let state = app.state::<Device>();
        let mut inner = state.0.lock().await;
        if inner.session.as_ref().is_some_and(|s| s.id == worker_id) {
            inner.session = None;
        }
        drop(inner);
        event(&app, json!({"type":"disconnected","message":result.err()}));
    });
    Ok(())
}

#[tauri::command]
async fn control(action: String, state: State<'_, Device>) -> Result<(), String> {
    let inner = state.0.lock().await;
    let session = inner.session.as_ref().ok_or("设备尚未连接")?;
    let command = match action.as_str() {
        "start" => audio::Control::Start,
        "stop" => audio::Control::Stop,
        "abort" => audio::Control::Abort,
        _ => return Err("未知的设备操作".into()),
    };
    session.audio.command(command).await
}

#[tauri::command]
async fn set_call(enabled: bool, silence_ms: u32, state: State<'_, Device>) -> Result<(), String> {
    let silence_ms = voice::validate_silence(silence_ms)?;
    let inner = state.0.lock().await;
    let session = inner.session.as_ref().ok_or("设备尚未连接")?;
    session
        .audio
        .silence_ms
        .store(silence_ms, Ordering::Relaxed);
    session.audio.command(audio::Control::Call(enabled)).await
}

#[tauri::command]
async fn set_voice_config(silence_ms: u32, state: State<'_, Device>) -> Result<(), String> {
    let silence_ms = voice::validate_silence(silence_ms)?;
    let inner = state.0.lock().await;
    if let Some(session) = &inner.session {
        session
            .audio
            .silence_ms
            .store(silence_ms, Ordering::Relaxed);
    }
    Ok(())
}
#[tauri::command]
async fn set_volume(volume: u32, state: State<'_, Device>) -> Result<(), String> {
    let inner = state.0.lock().await;
    if let Some(session) = &inner.session {
        session
            .audio
            .volume
            .store(volume.min(100), Ordering::Relaxed);
    }
    Ok(())
}

#[tauri::command]
async fn disconnect_device(state: State<'_, Device>) -> Result<(), String> {
    let inner = state.0.lock().await;
    if let Some(session) = &inner.session {
        session.audio.listening.store(false, Ordering::Relaxed);
        session.audio.running.store(false, Ordering::Relaxed);
        let _ = session.cancel.send(true);
    }
    Ok(())
}

#[tauri::command]
async fn send_text(text: String, state: State<'_, Device>) -> Result<(), String> {
    if text.trim().is_empty() || text.chars().count() > 2000 {
        return Err("请输入 1–2000 字的内容".into());
    }
    let inner = state.0.lock().await;
    let session = inner.session.as_ref().ok_or("设备尚未连接")?;
    session.audio.command(audio::Control::Text(text)).await
}
#[tauri::command]
fn open_console() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer.exe")
        .arg("https://xiaozhi.me")
        .spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open")
        .arg("https://xiaozhi.me")
        .spawn();
    #[cfg(target_os = "linux")]
    let result = std::process::Command::new("xdg-open")
        .arg("https://xiaozhi.me")
        .spawn();
    result.map(|_| ()).map_err(error_string)
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let directory = app.path().app_config_dir()?;
            std::fs::create_dir_all(&directory)?;
            let path = directory.join("identity.json");
            let identity: Identity = if path.exists() {
                serde_json::from_slice(&std::fs::read(&path)?)?
            } else {
                let uuid = uuid::Uuid::new_v4();
                let b = uuid.as_bytes();
                let value = Identity {
                    device_id: format!(
                        "02:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                        b[0], b[1], b[2], b[3], b[4]
                    ),
                    client_id: uuid.to_string(),
                };
                std::fs::write(&path, serde_json::to_vec_pretty(&value)?)?;
                value
            };
            app.manage(Device(Mutex::new(Inner {
                identity,
                connection: None,
                session: None,
            })));
            app.manage(window_mode::WindowMode::default());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_device,
            discover,
            activate,
            connect_device,
            control,
            set_call,
            set_voice_config,
            set_volume,
            disconnect_device,
            send_text,
            open_console,
            window_mode::get_window_mode,
            window_mode::set_pet_mode,
            window_mode::drag_pet
        ])
        .run(tauri::generate_context!())
        .expect("无法启动小智桌面客户端");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_keeps_audio_before_stop_and_discards_cancelled_epochs() {
        let mut active = None;
        let begin = wire_messages(Outgoing::Begin(7), 7, &mut active, "session", 1);
        assert_eq!(begin.len(), 2);
        assert_eq!(
            serde_json::from_str::<Value>(begin[0].to_text().unwrap()).unwrap()["type"],
            "abort"
        );
        assert_eq!(
            serde_json::from_str::<Value>(begin[1].to_text().unwrap()).unwrap()["state"],
            "start"
        );
        // Recording may already be false when the writer drains these queued frames.
        assert!(matches!(
            wire_messages(Outgoing::Audio(7, vec![42]), 7, &mut active, "session", 1)[0],
            Message::Binary(_)
        ));
        let end = wire_messages(Outgoing::End(7), 7, &mut active, "session", 1);
        assert_eq!(
            serde_json::from_str::<Value>(end[0].to_text().unwrap()).unwrap()["state"],
            "stop"
        );
        assert!(
            wire_messages(Outgoing::Audio(7, vec![42]), 8, &mut active, "session", 1).is_empty()
        );
        assert!(wire_messages(Outgoing::End(7), 8, &mut active, "session", 1).is_empty());
        assert_eq!(
            wire_messages(Outgoing::Begin(8), 8, &mut active, "session", 1).len(),
            2
        );
        assert_eq!(
            wire_messages(Outgoing::Abort(9), 9, &mut active, "session", 1).len(),
            1
        );
        assert!(
            wire_messages(Outgoing::Audio(8, vec![42]), 9, &mut active, "session", 1).is_empty()
        );
    }

    #[test]
    fn endpoints_require_tls_except_loopback() {
        for (url, ws) in [
            ("https://api.tenclass.net/xiaozhi/ota/", false),
            ("wss://example.com/socket", true),
            ("http://127.0.0.1:8765/ota", false),
            ("ws://localhost:8765", true),
        ] {
            assert!(valid_url(url, ws).is_ok());
        }
        for (url, ws) in [
            ("http://example.com", false),
            ("ws://example.com", true),
            ("https://user:secret@example.com", false),
            ("file:///etc/passwd", false),
            ("https://example.com", true),
        ] {
            assert!(valid_url(url, ws).is_err());
        }
    }

    #[tokio::test]
    async fn websocket_handshake_sends_device_headers_and_negotiates_opus() {
        for version in 1..=3 {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut ws = tokio_tungstenite::accept_hdr_async(
                    stream,
                    move |req: &tokio_tungstenite::tungstenite::handshake::server::Request,
                          response| {
                        assert_eq!(req.headers()["Device-Id"], "02:00:00:00:00:01");
                        assert_eq!(req.headers()["Client-Id"], "test-client");
                        assert_eq!(req.headers()["Authorization"], "Bearer test-token");
                        assert_eq!(req.headers()["Protocol-Version"], version.to_string());
                        Ok(response)
                    },
                )
                .await
                .unwrap();
                let hello = ws.next().await.unwrap().unwrap();
                let value: Value = serde_json::from_str(hello.to_text().unwrap()).unwrap();
                assert_eq!(value, protocol::hello(version));
                ws.send(Message::Ping(vec![1, 2].into())).await.unwrap();
                ws.send(Message::Text(json!({"type":"hello","transport":"websocket","session_id":"integration-session","audio_params":{"sample_rate":24000,"frame_duration":60}}).to_string().into())).await.unwrap();
                while let Some(Ok(message)) = ws.next().await {
                    if let Message::Binary(data) = message {
                        assert_eq!(protocol::unpack(&data, version).unwrap(), [42]);
                        ws.send(Message::Binary(data)).await.unwrap();
                        break;
                    }
                }
            });
            let config = Connection {
                url: format!("ws://{address}"),
                token: "test-token".into(),
                version,
            };
            let identity = Identity {
                device_id: "02:00:00:00:00:01".into(),
                client_id: "test-client".into(),
            };
            let (mut ws, hello) = open_websocket(&config, &identity).await.unwrap();
            assert_eq!(hello["session_id"], "integration-session");
            send_ws(
                &mut ws,
                Message::Binary(protocol::pack(&[42], version).into()),
            )
            .await
            .unwrap();
            loop {
                let message = tokio::time::timeout(Duration::from_secs(3), ws.next())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                if let Message::Binary(data) = message {
                    assert_eq!(protocol::unpack(&data, version).unwrap(), [42]);
                    break;
                }
            }
            server.await.unwrap();
        }
    }
}
