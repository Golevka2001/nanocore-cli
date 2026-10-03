//! 端口与设备：端口列表、端口号解析、打开设备与重试配置。

use nanocore::client::{Nanocore, RetryConfig};
use nanocore::midi::{self, MidiTransport};

use crate::cli::Conn;

use super::failure::{Dev, Failure};
use super::render::divider_err;

#[allow(clippy::needless_pass_by_value)]
pub fn cmd_list() {
    match midi::input_ports() {
        Ok(ports) if ports.is_empty() => println!("inputs:  (none)"),
        Ok(ports) => {
            println!("inputs:");
            for (i, n) in &ports {
                println!("  #{} {n}", i + 1);
            }
        }
        Err(e) => println!("inputs:  <error: {e}>"),
    }
    match midi::output_ports() {
        Ok(ports) if ports.is_empty() => println!("outputs: (none)"),
        Ok(ports) => {
            println!("outputs:");
            for (i, n) in &ports {
                println!("  #{} {n}", i + 1);
            }
        }
        Err(e) => println!("outputs: <error: {e}>"),
    }
}

/// 解析一个方向的 MIDI 端口号，优先级：显式端口号 > hint 匹配 > 唯一端口。
///
/// 入参是 1 起的用户编号（与 `ports` 打印的一致），返回 0 起的系统索引。
/// 前两级都无法确定时，只有在恰好只有一个可用端口（无歧义）时才采用它；
/// 否则报错并列出端口——静默回落到第 1 个可能打到无关的 MIDI 设备上。
fn resolve_port(
    explicit: Option<usize>,
    ports: &[(usize, String)],
    hint: &str,
    dir: &str,
    flag: &str,
) -> Result<usize, Failure> {
    if let Some(u) = explicit {
        let i = u.checked_sub(1).ok_or_else(|| {
            Failure::error(format!("{flag} must be 1 or greater (see `ports`)"))
        })?;
        if i >= ports.len() {
            return Err(Failure::error(format!(
                "no {dir} port #{u} ({} available)",
                ports.len()
            )));
        }
        return Ok(i);
    }
    if let Some(i) = midi::find_port(ports, hint) {
        return Ok(i);
    }
    match ports {
        [(i, _)] => Ok(*i),
        [] => Err(Failure::error(format!(
            "no {dir} ports available (hint {hint:?} unmatched)"
        ))),
        _ => {
            let list = ports
                .iter()
                .map(|(i, n)| format!("  #{} {n}", i + 1))
                .collect::<Vec<_>>()
                .join("\n");
            Err(Failure::error(format!(
                "no {dir} port matches hint {hint:?}; pass {flag} N explicitly. available:\n{list}"
            )))
        }
    }
}

pub fn open_device(conn: &Conn) -> Result<Dev, Failure> {
    let ins = midi::input_ports().map_err(|e| e.to_string())?;
    let outs = midi::output_ports().map_err(|e| e.to_string())?;
    let i = resolve_port(conn.in_port, &ins, &conn.hint, "input", "--in-port")?;
    let o = resolve_port(conn.out_port, &outs, &conn.hint, "output", "--out-port")?;
    // 只有走了自动探测才回显选择；显式指定全部端口时保持安静。
    if conn.in_port.is_none() || conn.out_port.is_none() {
        eprintln!(
            "input  #{} {:?}\noutput #{} {:?}",
            i + 1,
            ins.get(i).map(|p| p.1.as_str()).unwrap_or("?"),
            o + 1,
            outs.get(o).map(|p| p.1.as_str()).unwrap_or("?")
        );
        // 端口回显与随后的正文是两件事，这里划开——放在 open_device 是因为
        // 每个命令都要经过它，无需逐个分支重复。走 stderr 与回显同流。
        divider_err();
    }
    let transport = MidiTransport::open(i, o).map_err(|e| e.to_string())?;
    let mut dev = Nanocore::with_config(transport, retry_config(conn));
    if conn.no_sysex {
        dev.transport().set_sysex(false);
    }
    Ok(dev)
}

fn retry_config(conn: &Conn) -> RetryConfig {
    let mut c = RetryConfig::default();
    if let Some(ms) = conn.timeout_ms {
        c.initial_timeout = std::time::Duration::from_millis(ms);
        c.max_timeout = std::time::Duration::from_millis(ms * 4);
        c.hard_limit = std::time::Duration::from_millis(ms * 8);
    }
    if let Some(n) = conn.retries {
        c.max_retries = n;
    }
    c
}
