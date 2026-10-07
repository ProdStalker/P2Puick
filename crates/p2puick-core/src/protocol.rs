use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MAGIC: [u8; 4] = *b"P2PK";
pub const PROTOCOL_VERSION: u8 = 1;
pub const CHUNK_SIZE: usize = 64 * 1024;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgType {
    Hello = 1,
    HelloAck = 2,
    Manifest = 3,
    FileStart = 4,
    FileChunk = 5,
    FileEnd = 6,
    Ack = 7,
    Cancel = 8,
    Error = 9,
    Ready = 10,
}

impl MsgType {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            1 => Self::Hello,
            2 => Self::HelloAck,
            3 => Self::Manifest,
            4 => Self::FileStart,
            5 => Self::FileChunk,
            6 => Self::FileEnd,
            7 => Self::Ack,
            8 => Self::Cancel,
            9 => Self::Error,
            10 => Self::Ready,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloPayload {
    pub pairing_code: String,
    pub role: String,
    pub hostname: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub relative_path: String,
    pub size: u64,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub files: Vec<FileEntry>,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileStartPayload {
    pub relative_path: String,
    pub size: u64,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEndPayload {
    pub relative_path: String,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AckPayload {
    pub relative_path: String,
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub message: String,
}

#[derive(Debug, Clone)]
pub enum Message {
    Hello(HelloPayload),
    HelloAck(HelloPayload),
    Manifest(Manifest),
    Ready,
    FileStart(FileStartPayload),
    FileChunk(Vec<u8>),
    FileEnd(FileEndPayload),
    Ack(AckPayload),
    Cancel,
    Error(ErrorPayload),
}

impl Message {
    fn msg_type(&self) -> MsgType {
        match self {
            Self::Hello(_) => MsgType::Hello,
            Self::HelloAck(_) => MsgType::HelloAck,
            Self::Manifest(_) => MsgType::Manifest,
            Self::Ready => MsgType::Ready,
            Self::FileStart(_) => MsgType::FileStart,
            Self::FileChunk(_) => MsgType::FileChunk,
            Self::FileEnd(_) => MsgType::FileEnd,
            Self::Ack(_) => MsgType::Ack,
            Self::Cancel => MsgType::Cancel,
            Self::Error(_) => MsgType::Error,
        }
    }

    fn encode_payload(&self) -> Result<Vec<u8>> {
        Ok(match self {
            Self::Hello(p) | Self::HelloAck(p) => serde_json::to_vec(p)
                .map_err(|e| Error::protocol(format!("serialize hello: {e}")))?,
            Self::Manifest(p) => serde_json::to_vec(p)
                .map_err(|e| Error::protocol(format!("serialize manifest: {e}")))?,
            Self::Ready | Self::Cancel => Vec::new(),
            Self::FileStart(p) => serde_json::to_vec(p)
                .map_err(|e| Error::protocol(format!("serialize file start: {e}")))?,
            Self::FileChunk(data) => data.clone(),
            Self::FileEnd(p) => serde_json::to_vec(p)
                .map_err(|e| Error::protocol(format!("serialize file end: {e}")))?,
            Self::Ack(p) => serde_json::to_vec(p)
                .map_err(|e| Error::protocol(format!("serialize ack: {e}")))?,
            Self::Error(p) => serde_json::to_vec(p)
                .map_err(|e| Error::protocol(format!("serialize error: {e}")))?,
        })
    }

    fn decode(msg_type: MsgType, payload: Vec<u8>) -> Result<Self> {
        Ok(match msg_type {
            MsgType::Hello => Self::Hello(serde_json::from_slice(&payload).map_err(|e| {
                Error::protocol(format!("decode hello: {e}"))
            })?),
            MsgType::HelloAck => Self::HelloAck(serde_json::from_slice(&payload).map_err(|e| {
                Error::protocol(format!("decode hello ack: {e}"))
            })?),
            MsgType::Manifest => Self::Manifest(serde_json::from_slice(&payload).map_err(|e| {
                Error::protocol(format!("decode manifest: {e}"))
            })?),
            MsgType::Ready => Self::Ready,
            MsgType::FileStart => Self::FileStart(serde_json::from_slice(&payload).map_err(
                |e| Error::protocol(format!("decode file start: {e}")),
            )?),
            MsgType::FileChunk => Self::FileChunk(payload),
            MsgType::FileEnd => Self::FileEnd(serde_json::from_slice(&payload).map_err(|e| {
                Error::protocol(format!("decode file end: {e}"))
            })?),
            MsgType::Ack => Self::Ack(serde_json::from_slice(&payload).map_err(|e| {
                Error::protocol(format!("decode ack: {e}"))
            })?),
            MsgType::Cancel => Self::Cancel,
            MsgType::Error => Self::Error(serde_json::from_slice(&payload).map_err(|e| {
                Error::protocol(format!("decode error: {e}"))
            })?),
        })
    }
}

pub async fn write_message<W: AsyncWrite + Unpin>(writer: &mut W, msg: &Message) -> Result<()> {
    let payload = msg.encode_payload()?;
    if payload.len() > u32::MAX as usize {
        return Err(Error::protocol("payload too large"));
    }
    let mut header = [0u8; 10];
    header[0..4].copy_from_slice(&MAGIC);
    header[4] = PROTOCOL_VERSION;
    header[5] = msg.msg_type() as u8;
    header[6..10].copy_from_slice(&(payload.len() as u32).to_be_bytes());
    writer.write_all(&header).await?;
    if !payload.is_empty() {
        writer.write_all(&payload).await?;
    }
    writer.flush().await?;
    Ok(())
}

pub async fn read_message<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Message> {
    let mut header = [0u8; 10];
    reader.read_exact(&mut header).await?;
    if header[0..4] != MAGIC {
        return Err(Error::protocol("invalid magic"));
    }
    if header[4] != PROTOCOL_VERSION {
        return Err(Error::protocol(format!(
            "unsupported protocol version {}",
            header[4]
        )));
    }
    let msg_type = MsgType::from_u8(header[5])
        .ok_or_else(|| Error::protocol(format!("unknown message type {}", header[5])))?;
    let len = u32::from_be_bytes([header[6], header[7], header[8], header[9]]) as usize;
    let mut payload = vec![0u8; len];
    if len > 0 {
        reader.read_exact(&mut payload).await?;
    }
    Message::decode(msg_type, payload)
}
