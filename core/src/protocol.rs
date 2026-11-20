use serde::{Deserialize, Serialize};

pub const PROTOCOL: u32 = 4;
pub const MAX_MESSAGE: usize = 64 * 1024;
pub const MAX_PEERS: usize = 16;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClientMessage {
    Join {
        name: String,
        protocol: u32,
    },
    PubOffer {
        sdp: String,
    },
    RestartIce {
        pc: Pc,
    },
    PubRestartAnswer {
        sdp: String,
    },
    SubAnswer {
        sdp: String,
    },
    Ice {
        pc: Pc,
        candidate: String,
        #[serde(rename = "sdpMid")]
        sdp_mid: Option<String>,
        #[serde(rename = "sdpMLineIndex")]
        sdp_m_line_index: Option<u16>,
    },
    State {
        mic: bool,
        cam: bool,
        deaf: bool,
    },
    ClaimModerator {
        key: String,
    },
    SetServerMute {
        id: String,
        muted: bool,
    },
    Leave {},
    Ping {},
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pc {
    Pub,
    Sub,
}

#[derive(Clone, Debug, Serialize)]
pub struct Peer {
    pub id: String,
    pub name: String,
    pub mic: bool,
    pub cam: bool,
    pub deaf: bool,
    #[serde(rename = "serverMuted")]
    pub server_muted: bool,
}

pub fn parse(text: &str) -> Result<ClientMessage, &'static str> {
    if text.len() > MAX_MESSAGE {
        return Err("Message too large");
    }
    serde_json::from_str(text).map_err(|_| "Invalid message")
}

pub fn validate_join(name: &str, protocol: u32) -> Result<(), &'static str> {
    if protocol != PROTOCOL {
        return Err("Unsupported protocol");
    }
    if !(1..=32).contains(&name.chars().count())
        || name.trim().is_empty()
        || name.chars().any(char::is_control)
    {
        return Err("Name must contain 1 to 32 printable characters");
    }
    Ok(())
}

// Limit SDP complexity before handing it to the WebRTC parser.
pub fn validate_sdp(sdp: &str, publish: bool) -> Result<(), &'static str> {
    if sdp.len() > MAX_MESSAGE || sdp.lines().count() > 1024 {
        return Err("SDP too large");
    }
    let media: Vec<_> = sdp.lines().filter(|line| line.starts_with("m=")).collect();
    if publish {
        if media.len() > 2
            || media.iter().filter(|m| m.starts_with("m=audio ")).count() != 1
            || media
                .iter()
                .any(|m| !m.starts_with("m=audio ") && !m.starts_with("m=video "))
        {
            return Err("Expected one audio and at most one video track");
        }
    } else if media.is_empty()
        || media.len() > 2 * (MAX_PEERS - 1)
        || media.iter().filter(|m| m.starts_with("m=audio ")).count() >= MAX_PEERS
        || media.iter().filter(|m| m.starts_with("m=video ")).count() >= MAX_PEERS
        || media
            .iter()
            .any(|m| !m.starts_with("m=audio ") && !m.starts_with("m=video "))
    {
        return Err("Invalid subscription media");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_names_and_protocol() {
        assert!(validate_join(&"ş".repeat(32), PROTOCOL).is_ok());
        for name in [
            "".to_owned(),
            " ".to_owned(),
            "a\nb".to_owned(),
            "a".repeat(33),
        ] {
            assert!(validate_join(&name, PROTOCOL).is_err());
        }
        assert!(validate_join("Alice", 1).is_err());
    }
    #[test]
    fn rejects_bad_messages() {
        for input in [
            r#"{"type":"unknown"}"#,
            r#"{"type":"state","mic":1}"#,
            r#"{"type":"ping","extra":true}"#,
            "[]",
        ] {
            assert!(parse(input).is_err());
        }
        assert!(parse(&" ".repeat(MAX_MESSAGE + 1)).is_err());
        assert!(matches!(
            parse(r#"{"type":"ping"}"#),
            Ok(ClientMessage::Ping {})
        ));
    }
    #[test]
    fn bounds_sdp_media() {
        assert!(
            validate_sdp(
                "m=audio 9 UDP/TLS/RTP/SAVPF 111\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\n",
                true
            )
            .is_ok()
        );
        assert!(validate_sdp("m=application 9 UDP/DTLS/SCTP webrtc-datachannel", true).is_err());
        let mixed = "m=audio 9 RTP/AVP 111\nm=video 9 RTP/AVP 96\n".repeat(15);
        assert!(validate_sdp(&mixed, false).is_ok());
        assert!(validate_sdp(&(mixed + "m=video 9 RTP/AVP 96\n"), false).is_err());
        assert!(validate_sdp(&"m=video 9 RTP/AVP 96\n".repeat(16), false).is_err());
        assert!(validate_sdp(&"m=audio 9 RTP/AVP 111\n".repeat(16), false).is_err());
    }
}
