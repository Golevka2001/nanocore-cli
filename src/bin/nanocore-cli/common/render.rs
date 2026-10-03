//! 展示与格式化：hexdump、帧描述、分节线、链序与快照渲染。
//!
//! 这里只产出文本，不产生 IO 之外的副作用（分节线与 `describe` 走 stdout），
//! 也不依赖设备或事务——所有字节都由调用方传进来。

use nanocore::Frame;
use nanocore::protocol;

pub fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 一行 hexdump 内部的固定宽度（不含两侧的 `|`，也不含分隔用的两个空格）。
///
/// 16 个字节各占 `xx ` 3 字符 = 48，末字节后那个空格由 `pop()` 去掉，
/// 但第 8 字节后还有一个分组空隙，所以净值仍是 48。末行不足 16 字节时
/// 按同一宽度右补空格，ASCII 列才不会左移。
const HEX_COL: usize = 16 * 3;

/// 每行 16 字节的 hexdump：偏移、中间 8+8 分组的 hex、右侧 ASCII。
///
/// 形如：
/// ```text
///    0  00 03 00 54 01 03 06  b6 f3 1d 3f 00 00 00 00  |.. ...T......|
/// ```
///
/// 8 字节处的空隙让一行的两半在视觉上分开（8 字节正好是快照的打包组边界）；
/// ASCII 列让可打印字符直接可读，抓包时省去逐字节换算。不可打印字节一律
/// 打成 `.`，因此 ASCII 列宽度恒等于 16，末行也无需特殊处理。
pub fn dump(data: &[u8], indent: &str) -> String {
    let mut out = String::new();
    for (row, chunk) in data.chunks(16).enumerate() {
        let mut mid = String::new();
        for (i, b) in chunk.iter().enumerate() {
            if i == 8 {
                mid.push(' ');
            }
            mid.push_str(&format!("{b:02x} "));
        }
        // 去掉末尾那个分隔空格再右补，末行同样对齐。
        mid.pop();
        let pad = " ".repeat(HEX_COL.saturating_sub(mid.chars().count()));
        let ascii: String = chunk
            .iter()
            .map(|&b| if (0x20..0x7F).contains(&b) { b as char } else { '.' })
            .collect();
        out.push_str(&format!(
            "{indent}{:>4}  {}{}  |{ascii}|\n",
            row * 16,
            mid,
            pad
        ));
    }
    out
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

/// 打印一帧的帧头、载荷 hexdump，以及能结构化解析出的内容。
///
/// tx 与 rx 共用：两者的线上格式是同一个结构（magic | dir | flags | 常量 |
/// seq | type | payload），只有方向字节不同，所以渲染也该相同。tx 帧过去是
/// 手打的 `type=0xNN payload[..]`，`flags` 与 `seq` 从不显示——而 `seq` 正是
/// 设备在应答里回显、用来配对的那个值。
///
/// `indent` 逐层传递：帧头、hexdump、解析出的条目都按它缩进，否则同一个
/// `describe` 在不同调用点会印出不同深度的块。
///
/// `index` 是本帧在所属流里的序号，`monitor` 用它标出帧与帧的边界——一次
/// 抓取会连着来几十帧，每帧都以 `dir=` 开头，编号是唯一能一眼数出「这是第
/// 几帧」的办法。单帧的命令不需要，传 `None`。
pub fn describe(frame: &Frame, indent: &str, index: Option<usize>) {
    let inner = format!("{indent}  ");
    match index {
        Some(n) => println!(
            "{indent}[{n:>4}] dir={:?} flags=0x{:02x} seq=0x{:04x} type=0x{:02x} payload[{}]",
            frame.direction, frame.flags, frame.seq, frame.kind, frame.payload.len()
        ),
        None => println!(
            "{indent}dir={:?} flags=0x{:02x} seq=0x{:04x} type=0x{:02x} payload[{}]",
            frame.direction, frame.flags, frame.seq, frame.kind, frame.payload.len()
        ),
    }
    print!("{}", dump(&frame.payload, &inner));
    if frame.kind == 0x63
        && protocol::decode_snapshot_state(&frame.payload).is_some()
    {
        // 解码成功才渲染：tx 那种 3 字节请求帧也是 0x63，渲染它的 fallback
        // 只会得到「no structural decode」——对着一帧本就没有内容的载荷说它
        // 解不出来，是噪音而不是信息。
        // 快照自成一块，前后划开。缩进传 `indent`（与帧头同级）而不是
        // `inner`：render_snapshot 内部会自己给转储和字段各加一级，传入已
        // 缩进的量会让两层叠加。
        divider();
        print!("{}", render_snapshot(&frame.payload, indent, None, Bytes::Shown));
        divider();
    }
    if frame.kind == 0x06
        && let Some(ep) = protocol::parse_effect_data(&frame.payload)
    {
        println!("{inner}effect data: slot {} params {:?}", ep.slot, ep.params);
    }
    if frame.kind == 0x40
        && let Some(entries) = protocol::parse_preset_list(&frame.payload)
    {
        // 与快照同构：偏移前置，逐条对应上方 hexdump。`0x40` 的载荷是明文
        // ASCII（无需解包），所以偏移就是线上偏移，与上面那段 hexdump 同基准。
        divider();
        println!("{indent}slots ({}):", entries.len());
        for e in &entries {
            // 记录长度 = 槽位 1 字节 + 标记字节 + 8 字符（含跳过的 NUL），
            // 由下一条的偏移反推，最后一条按载荷末尾算。
            let len = entries
                .iter()
                .find(|x| x.offset > e.offset)
                .map_or(frame.payload.len() - e.offset, |x| x.offset - e.offset);
            let mut name = e.name.clone();
            if !e.named {
                name = "(empty)".to_string();
            }
            print!("{}", field_line("slot", e.offset, len, &name, indent));
        }
        divider();
    }
    if let Some(order) = protocol::parse_chain_order(&frame.payload) {
        println!("{inner}chainOrder: {order:?}");
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

/// 一行字段：偏移区间 + 字段名 + 可读值。
///
/// 偏移区间指向**解包后**数据流（偏移由 [`SnapshotRecord::stream_offset`] 等
/// 从解码游标带出来），字节本身交给上方的 hexdump 呈现——这里再抄一遍字节
/// 只会让同一串 hex 出现两遍，改动时容易只改一处。
///
/// 偏移前置：它是这一行的检索键，贴在行首比夹在字段名与数值之间好找。
///
/// 超过 `FIELD_WIDTH` 就把可读值折到下一行缩进对齐（而不是让它顶出右边）：
/// 一行放不下的字段折行后仍然读得完。
const FIELD_WIDTH: usize = 96;

fn field_line(label: &str, at: usize, len: usize, readable: &str, indent: &str) -> String {
    let range = if len == 1 {
        format!("[{at}]")
    } else {
        format!("[{at}..{}]", at + len)
    };
    let head = format!("{indent}{range:<12}{label:<14}");
    if head.chars().count() + readable.chars().count() <= FIELD_WIDTH {
        format!("{head}{readable}\n")
    } else {
        format!("{head}\n{indent}{:<26}{readable}\n", "")
    }
}

/// 快照的呈现粒度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bytes {
    /// 只给可读字段（`show` 用；日常看状态，不必带字节）。
    None,
    /// 额外给原始载荷与解包后数据流的 hexdump + 每字段的偏移（`debug … --decode` 用）。
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
            // 原始 payload 已在 rx 段 hexdump 过了，这里只给解包后的流——
            // fields 的偏移全部指向它。
            out.push_str(&format!("{indent}unpacked\n"));
            out.push_str(&dump(&data, &format!("{indent}  ")));
            out.push_str(&format!("{indent}fields\n"));
        }
        // 没开字节对照时不列偏移，只给一份紧凑摘要（`show` 的常态）。
        // 偏移由调用点显式给出，不靠 label 反查——那样换个标签就静默指错字节。
        let head = |label: &str, at: usize, readable: String| {
            if show_bytes {
                field_line(label, at, 1, &readable, indent)
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
            let on = if r.enabled != 0 { "on " } else { "off" };
            let params = r
                .params
                .iter()
                .map(|p| format!("{p:.3}"))
                .collect::<Vec<_>>()
                .join(", ");
            if show_bytes {
                // head 三字节逐个说清：enabled / variant / count。count 虽能从
                // params 区间长度反推（4 字节一个 float），但它是这一行的第三个
                // 字节，不写出来就等于留了一个没有解释的字节。
                out.push_str(&field_line(
                    &format!("{role} head"),
                    at,
                    3,
                    &format!(
                        "{on} type={} count={}",
                        r.variant + 1,
                        r.params.len()
                    ),
                    indent,
                ));
                out.push_str(&field_line(
                    &format!("{role} params"),
                    at + 3,
                    r.params.len() * 4,
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
            out.push_str(&field_line(
                "chain count",
                state.chain_count_offset,
                1,
                &protocol::CHAIN_LEN.to_string(),
                indent,
            ));
        }
        if let Some(order) = &chain {
            if show_bytes {
                out.push_str(&field_line(
                    "chain order",
                    state.chain_count_offset + 1,
                    order.len(),
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

#[cfg(test)]
mod hexdump_tests {
    use super::*;

    /// 末行必须右补到与满行同宽，否则 ASCII 列在短行上左移——而末行正是
    /// 读者唯一会去核对最后几个字节的那一行。`HEX_COL` 少算 1 就是这个 bug。
    #[test]
    fn short_rows_pad_to_the_full_hex_column() {
        let out = dump(&(0u8..20).collect::<Vec<_>>(), "");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        let col = |l: &str| l.find('|').expect("ascii column");
        assert_eq!(col(lines[0]), col(lines[1]), "短行 ASCII 列左移了:\n{out}");
    }

    /// 8 字节处的分组空隙落在有意义的边界上（打包组大小）。分隔空格本身也是
    /// 空格，故那里是两格；末行不足 8 字节时不该有空隙。
    #[test]
    fn group_gap_after_eight_bytes_only_when_row_is_full() {
        // 用递增字节：全零会把字面量 `00 01 02` 与输出混为一谈。
        let full = dump(&(0u8..16).collect::<Vec<_>>(), "");
        assert!(
            full.lines().next().unwrap().starts_with("   0  00 01 02 03 04 05 06 07  08 "),
            "{full}"
        );
        let short = dump(&[0u8; 4], "");
        assert!(
            short.lines().next().unwrap().starts_with("   0  00 00 00 00 "),
            "{short}"
        );
    }

    /// ASCII 列只把 `0x20..=0x7e` 当可打印；0x7f (DEL) 与 0x00 一样打成 `.`。
    #[test]
    fn ascii_column_dots_non_printable() {
        let out = dump(b"ab\x00\x7f\x80z", "");
        assert!(out.contains("|ab...z|"), "{out}");
    }
}

#[cfg(test)]
mod describe_decode_tests {
    use super::*;
    use nanocore::frame::MAGIC;

    fn frame63(payload: Vec<u8>) -> Frame {
        let mut wire = Vec::new();
        wire.extend_from_slice(&MAGIC);
        wire.push(0x71);
        wire.push(0x40);
        wire.push(0x02);
        wire.push(0x00);
        wire.push(0x00);
        wire.push(0x63);
        wire.extend_from_slice(&payload);
        Frame::decode(&wire).expect("decode").0
    }

    const SNAP: &str = "00002000000300540103060336731d3f00000002006c51383e295c050f3d381e053e5201381e3f000003000000003f0000003f034d4c4c3f010005031a19193f000000443f1f052b3f0a570c633f4d4c4c3f010c00024d4c0c3f334033333f0000041a1919193e414a213f300000003f1a1919603f0100025c0f420e3e4d4c4c3e010120057b142e3e6c5140383e713d4a3f4d014c4c3e7b142e3e000000040000003f000000003f000000003f0000003f08000001020304050607";

    fn bytes() -> Vec<u8> {
        SNAP.as_bytes().chunks(2)
            .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
            .collect()
    }

    /// monitor 会收到设备主动发出的 0x63（`preset set` 的应答就是这种），
    /// 故 describe 必须能把它解出来，而不只是打印 hexdump。
    #[test]
    fn snapshot_frame_renders_without_panicking() {
        let f = frame63(bytes());
        assert_eq!(f.kind, 0x63);
        // 渲染路径不能 panic，且解码应给出 8 条记录。
        let st = protocol::decode_snapshot_state(&f.payload).expect("0x63 must decode");
        assert_eq!(st.records.len(), 8);
        // monitor 会持续调用 describe，panic 会打断整个抓包。
        describe(&f, "", None);
    }
}
