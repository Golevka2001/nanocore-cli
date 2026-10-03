//! 共享基础辅助代码：端口列表、设备打开、十六进制转储、帧描述、事务与链序渲染。

use nanocore::client::{Error, Nanocore, RetryConfig};
use nanocore::midi::{self, MidiTransport};
use nanocore::protocol;
use nanocore::{Frame, Transport};

use crate::cli::Conn;

/// 通过真实 MIDI 传输层建立的已打开设备连接。
pub type Dev = Nanocore<MidiTransport>;

/// 命令失败：面向用户的消息，加上它应当产生的进程退出码。
///
/// 处理器只挑选语义，退出码由 `main` 统一定夺——「校验不一致」与「普通失败」
/// 对脚本是不同的结果（见 README 的退出码约定），不该靠字符串或深处的
/// `process::exit` 来表达。
#[derive(Debug)]
pub struct Failure {
    /// 写给用户的消息（`main` 只负责加上 `error: ` 前缀）。
    pub message: String,
    /// 进程退出码。
    pub exit_code: i32,
}

impl Failure {
    /// 普通失败：参数、通信或设备错误 → 退出码 2。
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit_code: 2,
        }
    }

    /// 校验不一致：设备数据与参考文件不同 → 退出码 3。
    pub fn mismatch(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit_code: 3,
        }
    }
}

/// 来自设备/传输层（以及任何以 `String` 报错）的失败都是普通失败，
/// 因此 `?` 无需在每个调用点包装。
impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self::error(message)
    }
}

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

pub fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 一条 60 宽的横线，带换行。
fn divider_line() -> String {
    format!("{}\n", "─".repeat(DIVIDER_WIDTH))
}

/// 分节线（stdout）：隔开事务收发与快照解码。
///
/// 一次 `debug snapshot --decode` 的输出来自三个互不相同的路径：端口回显
/// （`open_device`，走 **stderr**）、事务收发（`transact_reply`）、快照解码
/// （`render_snapshot`）。它们顶格排在一起时读者会把后半段误当成前半段的
/// 续行——尤其 `rx:` 的 payload 十六进制和紧随其后的 `chain:` 行长得毫不相干。
/// 一条横线把边界标出来，就不必再靠空行猜。
pub fn divider() {
    print!("{}", divider_line());
}

/// 分节线（stderr）：紧跟端口回显，把它与随后的 stdout 正文分开。
///
/// 必须走 stderr 才和它所分隔的端口回显落在同一个流里：两个流各自缓冲，
/// 一旦被重定向到同一个文件，混排的先后就不保证，分节线也就指错了位置。
pub fn divider_err() {
    eprint!("{}", divider_line());
}

/// 分隔线宽度：与项目文档里最宽的一行输出（`show` 的 settings 节）同量级，
/// 略短于它以免在窄终端里折行。
const DIVIDER_WIDTH: usize = 60;

pub fn describe(frame: &Frame) {
    println!(
        "  dir={:?} flags=0x{:02x} seq=0x{:04x} type=0x{:02x} payload[{}]",
        frame.direction,
        frame.flags,
        frame.seq,
        frame.kind,
        frame.payload.len()
    );
    println!("  payload: {}", hex(&frame.payload));
    if frame.kind == 0x06
        && let Some(ep) = protocol::parse_effect_data(&frame.payload)
    {
        println!("  effect data: slot {} params {:?}", ep.slot, ep.params);
    }
    if frame.kind == 0x40
        && let Some(entries) = protocol::parse_preset_list(&frame.payload)
    {
        println!("  presets ({}):", entries.len());
        for e in &entries {
            println!(
                "    #{:<2} {:<12} {}",
                e.slot,
                e.name,
                if e.named { "" } else { "(empty)" }
            );
        }
    }
    if let Some(order) = protocol::parse_chain_order(&frame.payload) {
        println!("  chainOrder: {order:?}");
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

/// 以裸 3 字节包发送标准 MIDI 消息（CC / PC）——不带 SysEx 包装。
/// 发送期间将传输层切出 SysEx 模式。
pub fn send_standard(dev: &mut Dev, bytes: &[u8]) -> Result<(), Failure> {
    dev.transport().set_sysex(false);
    dev.transport()
        .send(bytes)
        .map_err(|e| Failure::error(e.to_string()))
}

/// 解析十六进制字节序列（`debug raw` / `debug replay` 的位置参数）。
pub fn parse_hex(parts: &[String]) -> Result<Vec<u8>, Failure> {
    parts
        .iter()
        .map(|h| {
            u8::from_str_radix(h.trim().trim_start_matches("0x"), 16)
                .map_err(|_| Failure::error(format!("bad hex byte {h:?}")))
        })
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
    println!(
        "tx: type=0x{kind:02x} payload[{}] {}",
        payload.len(),
        hex(&payload)
    );
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
        Ok(reply) => {
            println!("rx:");
            describe(&reply);
            Ok(Some(reply))
        }
        Err(Error::Timeout { attempts, elapsed }) => Err(Failure::error(format!(
            "timeout after {attempts} attempt(s) in {elapsed:?}"
        ))),
        Err(Error::Transport(e)) => Err(Failure::error(e.to_string())),
    }
}

/// 使用设备显示名将链序渲染为 `DEL FX1 FX2 ...`。
pub fn render_chain(order: &[u8]) -> String {
    order
        .iter()
        .map(|i| protocol::chain_role_name(*i).unwrap_or("??").to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// 每行 16 字节的十六进制转储，左侧标出该行首字节在流内的偏移。
///
/// 转储的是**解包后**的数据流（[`protocol::unpack_snapshot_stream`]），不是线上
/// 载荷：线上每 8 字节有一个掩码字节被吞，同一个字段的字节会被掩码位劈开、
/// 跨组不连续（187 字节的载荷里 66 个字段有 13 个跨组）。解包后这些都变回连续，
/// 可读字段才能与字节逐段对齐。
fn dump_stream(data: &[u8], indent: &str) -> String {
    let mut out = String::new();
    for (row, chunk) in data.chunks(16).enumerate() {
        out.push_str(&format!("{indent}{:>4}  {}\n", row * 16, hex(chunk)));
    }
    out
}

/// 一行里同时给出「字段名 + 原始字节 + 可读值」，用于逐字段对照。
///
/// 字节取自解包后的流（`slice` 由调用方按 [`SnapshotRecord::stream_offset`] 等
/// 偏移切出），所以每个数字都能在上面的转储里原位找到。
///
/// 超过 `WIDTH` 就把可读值折到下一行缩进对齐（而不是让它顶出右边）——
/// 7 个参数的记录光字节就 28 字符，跟在后面会把这个字段的可读值挤出屏幕。
const FIELD_WIDTH: usize = 96;

fn field_line(label: &str, bytes: &[u8], readable: &str, indent: &str) -> String {
    let pad = " ".repeat(14);
    let head = format!("{indent}{label:<14}{}", hex(bytes));
    if head.chars().count() + readable.chars().count() <= FIELD_WIDTH {
        format!("{head} {readable}\n")
    } else {
        format!("{head}\n{indent}{pad}{readable}\n")
    }
}

/// 快照的呈现粒度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bytes {
    /// 只给可读字段（`show` 用；日常看状态，不必带字节）。
    None,
    /// 额外给解包后的字节流转储 + 每字段的原始字节（`debug snapshot --decode` 用）。
    ///
    /// 转储的是**解包后**的流：线上载荷每 8 字节有一个掩码字节被吞，同一个
    /// float 的 4 个字节会被掩码位劈开、跨组不连续（187 字节载荷的 66 个字段里
    /// 13 个跨组）。解包后字段恢复连续，才谈得上「按偏移逐字段核对」。
    Shown,
}

/// 把一条 `0x63` 快照载荷渲染为多行可读文本（`show` 与 `debug snapshot --decode` 共用）。
///
/// 输出**只写字节本身能支撑的内容**：每个字段都是从载荷里某个确定偏移解出来的，
/// 没有字节出处的推断（成因解释、与出厂值的差异、启发式扫描计数）一律不进这里——
/// 那些属于 `docs/protocol/snapshot.md` 的逆向笔记，不属于「这帧是什么意思」。
///
/// `indent` 是每行的前导空白——`show` 有两格缩进，`debug` 下的调用方用空串。
/// `name` 是当前预设名（`None` 表示未查到或该槽位未命名），显示在编号之后。
/// 无法结构化解码时（例如 BLE 抓包形态）退回经验字段，不静默丢弃内容。
pub fn render_snapshot(
    payload: &[u8],
    indent: &str,
    name: Option<&str>,
    bytes: Bytes,
) -> String {
    let mut out = String::new();
    let decoded = protocol::decode_snapshot_state(payload);
    // 链序优先取数据流解码（对所有长度都成立），旧的尾部锚点只作退回。
    let chain = decoded
        .as_ref()
        .and_then(|st| st.chain.clone())
        .or_else(|| protocol::parse_snapshot_chain_order(payload));
    // 字段一律按**载荷里的真实顺序**输出（preset → volume → 8 条记录 → 链），
    // 不按「链序更适合先说」重排：开了字节对照后，这一节的价值就是每个字段都能
    // 在上面的转储里原位找到，重排会让对照失效。
    if let Some(state) = &decoded
        && let Some(data) = protocol::unpack_snapshot_stream(payload)
    {
        let show_bytes = bytes == Bytes::Shown;
        if show_bytes {
            out.push_str(&format!(
                "{indent}unpacked stream ({} bytes, mask bytes dropped):\n",
                data.len()
            ));
            out.push_str(&dump_stream(&data, &format!("{indent}  ")));
            out.push('\n');
            out.push_str(&format!("{indent}fields (same order as the stream above):\n"));
        }
        // 没开字节对照时不逐字段列偏移，只给一份紧凑摘要（`show` 的常态）。
        // 偏移由调用点显式给出，不靠 label 反查——那样换个标签就静默指错字节。
        let head = |label: &str, at: usize, readable: String| {
            if show_bytes {
                field_line(label, &data[at..at + 1], &readable, indent)
            } else {
                format!("{indent}{label}: {readable}\n")
            }
        };
        out.push_str(&head(
            "preset index",
            state.preset_index_offset,
            format!(
                "#{}{}",
                state.preset_index + 1,
                match name {
                    Some(n) => format!(" {n}"),
                    None => String::new(),
                }
            ),
        ));
        out.push_str(&head(
            "preset volume",
            state.preset_volume_offset,
            state.preset_volume.to_string(),
        ));
        for (i, r) in state.records.iter().enumerate() {
            // 与 `chain:` 行同一套名字（设备显示名），不是 CLI 短名——
            // 两行讲的是同一组角色，叫法不同会让人以为是两套东西。
            let role = protocol::CHAIN_ROLES[i].1;
            let at = r.stream_offset;
            let body = &data[at + 3..at + 3 + r.params.len() * 4];
            let on = if r.enabled != 0 { "on " } else { "off" };
            let params = r
                .params
                .iter()
                .map(|p| format!("{p:.3}"))
                .collect::<Vec<_>>()
                .join(", ");
            if show_bytes {
                out.push_str(&field_line(
                    &format!("{role} head"),
                    &data[at..at + 3],
                    &format!("{on} type={} ({} params)", r.variant + 1, r.params.len()),
                    indent,
                ));
                out.push_str(&field_line(
                    &format!("{role} params"),
                    body,
                    &format!("[{params}]"),
                    indent,
                ));
            } else {
                out.push_str(&format!(
                    "{indent}  {role:<5} {on} type={} params=[{params}]\n",
                    r.variant + 1
                ));
            }
        }
        if show_bytes {
            let at = state.chain_count_offset;
            out.push_str(&field_line(
                "chain count",
                &data[at..at + 1],
                &format!("{} slots", protocol::CHAIN_LEN),
                indent,
            ));
        }
        if let Some(order) = &chain {
            if show_bytes {
                let at = state.chain_count_offset + 1;
                out.push_str(&field_line(
                    "chain order",
                    &data[at..at + order.len()],
                    &render_chain(order),
                    indent,
                ));
            } else {
                out.push_str(&format!("{indent}chain:         {}\n", render_chain(order)));
            }
        }
        return out;
    }

    // ── 无法结构化解码：退回经验字段（BLE 抓包形态）──
    match chain {
        Some(order) => out.push_str(&format!("{indent}chain:         {}\n", render_chain(&order))),
        None => out.push_str(&format!(
            "{indent}chain:         (not decodable from this snapshot)\n"
        )),
    }
    if payload.len() > 6 {
        out.push_str(&format!(
            "{indent}preset:        #{} (empirical — payload[6])\n",
            payload[6] + 1
        ));
    }
    out.push_str(&format!(
        "{indent}(no structural decode for this payload; {} bytes, layout unknown)\n",
        payload.len()
    ));
    out
}
