# 小智电脑版

让小智来到电脑上。使用 Tauri 2、Vue 3 和 Rust 开发，连接 [xiaozhi.me](https://xiaozhi.me) 后台，支持连续语音通话、开口打断和桌面宠物。

本项目参考虾哥的开源项目 [78/xiaozhi-esp32](https://github.com/78/xiaozhi-esp32)，在电脑上实现兼容的设备身份、激活与 WebSocket/Opus 协议。它是独立的社区桌面客户端，不运行 ESP32 固件，也不需要购买开发板。

## 开始使用

当前版本 **v0.3.0**，已在 Windows 环境验证。安装包见 [Releases](https://github.com/xgchip/xiaozhidesk/releases)。

1. 安装并启动「小智电脑版」。Windows 10/11 需要 WebView2 Runtime。
2. 自行前往 [小智网站](https://xiaozhi.me) 注册或登录账号。
3. 点击「开始通话」，在小智控制台添加设备，输入软件显示的激活码完成配对。
4. 配对后直接说话，停顿后自动发送。回复过程中可直接开口打断，继续下一句话。
5. 点击电话按钮结束通话。重新启动软件不会自动打开麦克风。

**每位用户均需使用自己的账号配对。源码和安装包不包含开发者的设备绑定、账号、令牌或聊天记录。** 首次启动会在本机生成独立设备身份；配对后保存在自己的电脑上。

## 语音通话

- 一次开启，多轮聊天，无需逐句按住说话。
- 「设置 → 通话 → 停顿多久自动发送」可调 **300–2000 毫秒**，默认 **700 毫秒**，立即生效并保存。数值越小提交越快，数值越大越适合句中停顿；检测和网络还会带来额外延迟。
- 本机 WebRTC VAD 检测人声，保留开头短预录，静音达到设置时长后结束当前轮次。
- 使用 Sonora WebRTC AEC3 回声消除与降噪。检测到说话时停止本机播放，并向后台发送打断请求。建议佩戴耳机；外放效果会受声卡、音量和房间环境影响。
- 支持音量、输入和输出设备选择、文字输入。单轮语音最长 45 秒；通话中断后自动重连，挂断取消重试。

## 桌面宠物

点击首页角色或右上角「桌面宠物」，切换为透明、置顶的悬浮小智。点击角色开始或结束通话，开启后直接说话即可。

鼠标移入后可拖动、静音或展开。双击角色、右键、点击气泡或按 Esc 返回完整窗口。角色会响应聆听、思考和说话状态，气泡显示最近一句回复。

项目图片将在后续 UI 优化版本补充。

## 本地开发

需要 Node.js 22.12+、Rust stable、CMake。Windows 还需要 Visual Studio 2022 Build Tools 的「使用 C++ 的桌面开发」、Windows SDK 和 WebView2。其他平台参见 [Tauri 环境准备](https://v2.tauri.app/start/prerequisites/)。当前只验证 Windows，默认打包目标为 NSIS。

```powershell
git clone https://github.com/xgchip/xiaozhidesk.git
cd xiaozhidesk
npm ci
.\scripts\desktop.ps1 dev
```

构建安装包与检查：

```powershell
.\scripts\desktop.ps1 test
.\scripts\desktop.ps1 check
.\scripts\desktop.ps1 build
```

脚本会寻找 CMake，也可通过 `-CmakeBin '你的 CMake bin 目录'` 指定。使用 CMake 4 时脚本会设置 `CMAKE_POLICY_VERSION_MINIMUM=3.5`，以兼容 libopus 构建。

构建输出位于 `src-tauri/target/release/`，安装包位于其 `bundle/nsis/` 下。也可以分别运行：

```sh
npm run build
npm test
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
```

单独运行 `npm run dev` 可预览界面；真实网络与音频功能需要 Tauri 原生进程。macOS/Linux 尚未验证，需安装对应系统依赖并调整打包目标。

## 协议与数据

- 默认 OTA：`https://api.tenclass.net/xiaozhi/ota/`。支持设备激活 v1、WebSocket 二进制协议 v1/v2/v3；暂不支持 MQTT/UDP，也不提供硬件激活 v2 证明。
- 上行音频为单声道 16 kHz、60 ms Opus 帧，网络由 Rust 原生进程处理。
- MCP 支持设备状态与音量工具，不提供文件或 Shell 控制。
- 每位用户的设备身份保存在应用配置目录 `identity.json`，Windows 通常为 `%APPDATA%\me.xiaozhi.desktop\identity.json`。该文件不会随项目分发。删除后会生成新身份，需重新配对。
- 后台令牌仅在进程内存中使用；音量、设备和停顿设置保存在本机。聊天与诊断各最多 200 条，仅在进程内存中保留。
- 开启通话后，语音发送到所配置的小智后台进行识别和对话。软件不保存录音文件；使用前请了解后台服务的数据政策。

自动化测试覆盖语音分段、停顿调整、Opus、回声混合下的打断检测、协议帧与本机 WebSocket 联调。真实语音体验取决于麦克风、声卡、网络和后台服务。

## 致谢与许可

感谢 **虾哥** 和 [xiaozhi-esp32](https://github.com/78/xiaozhi-esp32) 社区开源小智项目及协议实现，让这个电脑版客户端得以完成。推荐了解和支持原项目。

协议参考：[WebSocket 文档](https://github.com/78/xiaozhi-esp32/blob/main/docs/websocket.md)、[MCP 文档](https://github.com/78/xiaozhi-esp32/blob/main/docs/mcp-protocol.md) 和上游源码。测试样本来源见 [样本说明](tests/fixtures/README.md)。

本项目采用 [MIT License](LICENSE)，保留上游版权声明。第三方组件许可见 [THIRD-PARTY-NOTICES.txt](THIRD-PARTY-NOTICES.txt)。
