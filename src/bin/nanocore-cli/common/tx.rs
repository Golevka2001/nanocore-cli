//! 事务层：裸 MIDI 发送、SysEx 收发、进度回显与 hex 位置参数解析。

use nanocore::client::Error;
use nanocore::{Frame, Transport};

use crate::cli::{Conn, parse_hex_byte};

use super::failure::{Dev, Failure};
use super::render::describe;

/// 以裸 3 字节包发送标准 MIDI 消息（CC / PC）——不带 SysEx 包装。
/// 发送期间将传输层切出 SysEx 模式。
pub fn send_standard(dev: &mut Dev, bytes: &[u8]) -> Result<(), Failure> {
    dev.transport().set_sysex(false);
    dev.transport()
        .send(bytes)
        .map_err(|e| Failure::error(e.to_string()))
}

/// 写会话的进度回显：每条状态覆写同一行（stderr）。
///
/// `write_ir_slot` / `write_amp_slot` / `import_presets` /
/// `restore_factory_presets` 都按 `FnMut(&str)` 报告进度，命令侧统一用它。
pub fn progress(msg: &str) {
    eprint!("\r{msg}");
}

/// 解析十六进制字节序列（`debug raw` / `debug replay` 的位置参数）。
pub fn parse_hex(parts: &[String]) -> Result<Vec<u8>, Failure> {
    parts
        .iter()
        .map(|h| parse_hex_byte(h).map_err(|_| Failure::error(format!("bad hex byte {h:?}"))))
        .collect()
}

/// 通用 SysEx 事务：发送，等待（按类型过滤的）应答，并输出其描述。
pub fn transact(
    dev: &mut Dev,
    conn: &Conn,
    kind: u8,
    payload: Vec<u8>,
    expect: &[u8],
) -> Result<(), Failure> {
    transact_reply(dev, conn, kind, payload, expect).map(|_| ())
}

/// 与 [`transact`] 相同，但把应答帧交回调用方
/// （`--no-wait` 或不等待的命令路径返回 `None`）。
///
/// 参数写入需要它：设备会用一条 5 字节帧 [`protocol::PARAM_WRITE_REJECTED`]
/// 拒绝 `b6` 超出存储范围的写入，此时原值保持不变 —— 只打印应答会让用户
/// 以为写成功了。
pub fn transact_reply(
    dev: &mut Dev,
    conn: &Conn,
    kind: u8,
    payload: Vec<u8>,
    expect: &[u8],
) -> Result<Option<Frame>, Failure> {
    if payload.iter().any(|b| b & 0x80 != 0) {
        eprintln!(
            "warning: payload contains bytes >= 0x80; they are illegal MIDI data \n\
             bytes and the device will discard the message (7-bit-safe encoding?)"
        );
    }
    if !conn.wait {
        dev.send(kind, payload)
            .map_err(|e| Failure::error(e.to_string()))?;
        println!("sent (no wait)");
        return Ok(None);
    }
    match dev.transact_with_flags(conn.flags, kind, payload, expect) {
        Ok((tx, rx)) => {
            // tx 与 rx 用同一个渲染器：两行并排即可看出 seq 是否被设备回显成
            // 同一个值（这正是配对的依据），也免去手打 tx 字符串漏掉字段。
            describe(&tx, "", false);
            describe(&rx, "", false);
            Ok(Some(rx))
        }
        Err(Error::Timeout { attempts, elapsed }) => Err(Failure::error(format!(
            "timeout after {attempts} attempt(s) in {elapsed:?}"
        ))),
        Err(Error::Transport(e)) => Err(Failure::error(e.to_string())),
    }
}
