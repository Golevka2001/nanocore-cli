//! 传输层抽象。
//!
//! 为你的 MIDI 后端（如 `midir`、`btleplug`）或记录/注入原始字节的测试环境实现本 trait。

use std::error::Error;

/// 携带完整 Nanocore 消息的双向字节传输层。
pub trait Transport {
    type Error: Error + 'static;

    /// 发送原始字节（一个完整编码的 [`crate::Frame`]）。
    fn send(&mut self, data: &[u8]) -> Result<(), Self::Error>;

    /// 接收当前可用的所有字节。可能返回空 vec。
    fn receive(&mut self) -> Result<Vec<u8>, Self::Error>;
}

/// 内存中的传输层，适用于测试和重放既定帧序列。
#[derive(Default)]
pub struct LoopbackTransport {
    pub sent: Vec<Vec<u8>>,
    pub incoming: Vec<u8>,
}

impl LoopbackTransport {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Transport for LoopbackTransport {
    type Error = std::convert::Infallible;

    fn send(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        self.sent.push(data.to_vec());
        Ok(())
    }

    fn receive(&mut self) -> Result<Vec<u8>, Self::Error> {
        Ok(std::mem::take(&mut self.incoming))
    }
}
