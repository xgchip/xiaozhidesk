use serde_json::{json, Value};

pub fn hello(version: u32) -> Value {
    json!({"type":"hello","version":version,"transport":"websocket","features":{"mcp":true},
        "audio_params":{"format":"opus","sample_rate":16000,"channels":1,"frame_duration":60}})
}

pub fn pack(payload: &[u8], version: u32) -> Vec<u8> {
    let mut result = Vec::with_capacity(payload.len() + 16);
    match version {
        2 => {
            result.extend_from_slice(&[0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            result.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        }
        3 => {
            result.extend_from_slice(&[0, 0]);
            result.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        }
        _ => {}
    }
    result.extend_from_slice(payload);
    result
}

pub fn unpack(data: &[u8], version: u32) -> Result<&[u8], String> {
    let (offset, size) = match version {
        2 => {
            if data.len() < 16 || data[0..4] != [0, 2, 0, 0] {
                return Err("无效的 v2 音频帧".into());
            }
            (
                16,
                u32::from_be_bytes(data[12..16].try_into().unwrap()) as usize,
            )
        }
        3 => {
            if data.len() < 4 || data[0] != 0 {
                return Err("无效的 v3 音频帧".into());
            }
            (
                4,
                u16::from_be_bytes(data[2..4].try_into().unwrap()) as usize,
            )
        }
        _ => (0, data.len()),
    };
    if size == 0 || size > 1275 || data.len() != offset + size {
        return Err("音频帧长度不匹配".into());
    }
    Ok(&data[offset..])
}

pub fn mcp_response(payload: &Value, volume: &mut u32) -> Option<Value> {
    let id = payload.get("id")?.clone();
    let result = match payload.get("method").and_then(Value::as_str).unwrap_or("") {
        "initialize" => {
            json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"xiaozhi-desktop","version":env!("CARGO_PKG_VERSION")}})
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools":[
            {"name":"self.get_device_status","description":"Get desktop speaker status","inputSchema":{"type":"object","properties":{}}},
            {"name":"self.audio_speaker.set_volume","description":"Set speaker volume (0–100)","inputSchema":{"type":"object","properties":{"volume":{"type":"integer","minimum":0,"maximum":100}},"required":["volume"]}}
        ]}),
        "tools/call" => {
            let params = &payload["params"];
            match params["name"].as_str().unwrap_or("") {
                "self.get_device_status" => {
                    json!({"content":[{"type":"text","text":json!({"audio_speaker":{"volume":volume},"platform":"desktop"}).to_string()}],"isError":false})
                }
                "self.audio_speaker.set_volume" => {
                    if let Some(v) = params["arguments"]["volume"].as_u64().filter(|v| *v <= 100) {
                        *volume = v as u32;
                        json!({"content":[{"type":"text","text":"true"}],"isError":false})
                    } else {
                        return Some(
                            json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"volume must be an integer from 0 to 100"}}),
                        );
                    }
                }
                _ => {
                    return Some(
                        json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"Unknown tool"}}),
                    )
                }
            }
        }
        _ => {
            return Some(
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Method not found"}}),
            )
        }
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn framing_matches_firmware() {
        for version in 1..=3 {
            let data = pack(&[10, 20, 30], version);
            assert_eq!(unpack(&data, version).unwrap(), [10, 20, 30]);
        }
        assert_eq!(pack(&[42], 3), [0, 0, 0, 1, 42]);
        assert_eq!(&pack(&[42], 2)[12..], &[0, 0, 0, 1, 42]);
    }
    #[test]
    fn reject_truncated_and_oversized_frames() {
        for version in [2, 3] {
            let mut data = pack(&[42], version);
            data.pop();
            assert!(unpack(&data, version).is_err());
        }
        assert!(unpack(&[0; 1276], 1).is_err());
        assert!(unpack(&[], 2).is_err());
    }
    #[test]
    fn mcp_validates_volume_and_notifications() {
        let mut volume = 50;
        assert!(
            mcp_response(&json!({"method":"notifications/initialized"}), &mut volume).is_none()
        );
        let response=mcp_response(&json!({"id":1,"method":"tools/call","params":{"name":"self.audio_speaker.set_volume","arguments":{"volume":101}}}), &mut volume).unwrap();
        assert!(response.get("error").is_some());
        assert_eq!(volume, 50);
    }
}
