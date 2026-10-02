//! 已确认消息类型的载荷构造器与解析器。
//!
//! 此处所有字节布局均来自真实 MIDI 抓包。

use crate::field;
use crate::msg_type;
use crate::sub;

/// 将值编码为小端 float32（原始值，非线上安全格式）。
///
/// 仅用于解码/检查。发送参数值请使用 [`param_write`]，
/// 它会同时生成 7 比特安全的字节和还原用的 mask。
pub fn encode_param_float(value: f32) -> [u8; 4] {
    value.to_le_bytes()
}

/// 解码小端 float32。
pub fn decode_param_float(bytes: &[u8]) -> Option<f32> {
    if bytes.len() < 4 {
        return None;
    }
    Some(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// 剥掉 float32 四个字节的最高位 —— **只有这样字节才能合法地走 SysEx**。
///
/// SysEx 的数据字节必须 `< 0x80`，所以每个字节的 bit7 必须去掉。
/// 但**不能只剥不还**：剥掉的位由 [`param_value_mask`] 记录，
/// [`param_write`] 把它放进消息的 mask 字节，设备据此把 bit7 还原，
/// 因此 float32 是**全精度**到达设备的（早期版本误以为设备只能收到 28 位，
/// 进而接受了 `0.3 -> 0.166` 这类塌缩 —— 那是我们没发 mask 的结果，不是硬件限制）。
///
/// 唯一还用到「只剥不还」的是 [`load_effect`]（该消息的第 4 字节是 `model_flag`，
/// 不是 mask）；它目前没有 CLI 调用方。
pub fn encode_param_value(value: f32) -> [u8; 4] {
    let b = value.to_le_bytes();
    [b[0] & 0x7F, b[1] & 0x7F, b[2] & 0x7F, b[3] & 0x7F]
}

/// 还原掩码：`bit(3 + j)` = 第 `j` 个值字节的 bit7 被 [`encode_param_value`] 剥掉了。
///
/// 见 [`param_write`] 中的布局说明（mask 覆盖其后的 7 个数据字节，
/// `bit0..2` 对应恒定的 `01`、`b5`、`b6`，正常情况下恒为 0）。
pub fn param_value_mask(value: f32) -> u8 {
    value
        .to_le_bytes()
        .iter()
        .enumerate()
        .fold(0u8, |mask, (j, &b)| {
            if b & 0x80 != 0 {
                mask | (1 << (3 + j))
            } else {
                mask
            }
        })
}

// ─── 构造器 ───────────────────────────

/// `0x6D` sub `0x02`：设置标量字段（预设 / 箱体 / 变体）。
pub fn set_field(field_id: u8, value: u8) -> (u8, Vec<u8>) {
    (msg_type::DATA, vec![0x00, sub::SET_FIELD, 0x00, 0x00, field_id, value])
}

/// 按 0 起始索引选择预设。
pub fn select_preset(index: u8) -> (u8, Vec<u8>) {
    set_field(field::PRESET, index)
}

/// 按索引选择箱体 / IR。
pub fn select_cab(index: u8) -> (u8, Vec<u8>) {
    set_field(field::CAB, index)
}

/// 选择音箱模型 / 效果变体。
pub fn select_variant(variant: u8) -> (u8, Vec<u8>) {
    set_field(field::VARIANT, variant)
}

/// `0x6D` sub `0x03`：启用 / 禁用效果槽位。
pub fn set_enabled(slot: u8, enabled: bool) -> (u8, Vec<u8>) {
    (msg_type::DATA, vec![0x00, sub::ENABLE, 0x00, 0x00, 0x03, slot, enabled as u8])
}

/// `0x6D` sub `0x07`：写入单个参数。
///
/// 线上布局，来自抓包中全部 805 条 `0x07` 消息（`0..3` 号抓包逐条对账 805/805 命中）：
///
/// ```text
/// 00 07 00 [mask] 01 [b5] [b6] <float32 x 4>
/// ```
///
/// * `mask` —— 覆盖其后 7 个数据字节的**还原掩码**：`bit(3 + j)` 表示第 `j` 个值字节的
///   bit7 被 [`encode_param_value`] 剥掉了，设备收到后把它还回去。
///   `bit0..2` 对应恒定的 `01`、`b5`、`b6`（都 `< 0x80`），因此恒为 0。
///   **所以 float32 是全精度写入的**，不需要牺牲任何位。
/// * `b6` —— 效果内的**参数索引**。由 `noise-gate-tweak.mmon` 验证：
///   噪声门恰好暴露两个参数，`b6` 也恰好只取 `0x00`（阈值）和 `0x01`（释放）两个值。
/// * `b5` —— **槽位 / 效果实例**。fx1 抓包中的每次写入均为 `0x00`，
///   fx2 抓包中的每次写入均为 `0x01`。
///
/// 关于 `mask` 的三重证据：
///
/// * 结构：4 份抓包里 `mask` 只会出现 `0x08/0x10/0x18/0x20/0x28/0x30/0x38`
///   （即 bit3..5 的组合，对应 v0/v1/v2 的 bit7），`bit6`（v3 = 指数高位）恒为 0。
/// * 形状：`scream-knob-sweep.mmon` 里旋钮 0→100% 的一整段，**只有**按 mask 还原后
///   才是单调平滑的 `0.404 -> 1.000`；按裸值读会得到 `0.202 -> 0.500`，
///   并在参数本应到达 1.0 的地方卡在 0.5。
/// * 真机：`0.6 -> mask 0x18 + 1a 19 19 3f` 读回 `0.6`（mask=0 时存成 `0.598`）；
///   `1.0 -> mask 0x20 + 00 00 00 3f` 读回 `1.0`（mask=0 时存成 `0.5`）。
///
/// 设备把存下的值四舍五入到 **0.001 网格**：40 组出厂预设的 1215 个参数值全部落在
/// 该网格上，真机实测发送 `0.5996` 读回 `0.6`、发送 `0.123456789` 读回 `0.123`。
///
/// `b6` 超出该模块设备存储范围时，设备回一条 5 字节错误帧 `00 03 00 00 00`
/// 并**保持原值不变**；`module` 命令据此报错。
pub fn param_write(b5: u8, b6: u8, value: f32) -> (u8, Vec<u8>) {
    let mut p = vec![0x00, sub::PARAM_WRITE, 0x00, param_value_mask(value), 0x01, b5, b6];
    p.extend_from_slice(&encode_param_value(value));
    (msg_type::DATA, p)
}

/// 参数写入被拒绝时设备回的 5 字节载荷：`b6` 超出该模块的设备存储范围。
///
/// 真机实测（cab `b6=2`，设备只存 2 个值）：应答为 `type=0x6d payload[5] 00 03 00 00 00`，
/// 随后读回快照确认**原值未变**。同形状的 `00 01 00 00 00` 出现在载荷含
/// `>= 0x80` 的数据字节或长度不对时（帧解析失败），正常写入则回一条 13 字节的
/// mask 回显。
pub const PARAM_WRITE_REJECTED: [u8; 5] = [0x00, 0x03, 0x00, 0x00, 0x00];

// ─── 效果链序（`chainOrder`） ────────

/// 效果链中的效果数量。
pub const CHAIN_LEN: usize = 8;

/// 链角色表：`index -> (cli 名称, 显示名称)`。
///
/// **此顺序不是 CC 块的顺序。** 标准 MIDI CC 映射把各块列为 `FX1 FX2 AMP CAB MOD DELAY REVERB EQ`
/// （CC25 = Mod，CC26 = Delay），而链中 `DEL` 排在 `MOD` *之前*。
/// 由 `enable-del-eq.mmon` 验证：用户启用了 DEL 和 EQ，设备收到 `ENABLE slot=4` 和 `ENABLE slot=7`，
/// 故 `DEL == 4`、`MOD == 5`、`EQ == 7`。
pub const CHAIN_ROLES: [(&str, &str); CHAIN_LEN] = [
    ("fx1", "FX1"),
    ("fx2", "FX2"),
    ("amp", "AMP"),
    ("cab", "CAB"),
    ("delay", "DEL"),
    ("mod", "MOD"),
    ("reverb", "REV"),
    ("eq", "EQ"),
];

/// 按 CLI/显示名称解析链角色（大小写不敏感）。
///
/// 同时接受短 CLI 名称（`delay`、`reverb`）和设备显示名称（`del`、`rev`），以及常见的别名。
pub fn chain_role_index(name: &str) -> Option<u8> {
    let n = name.trim().to_ascii_lowercase();
    let alias: &str = match n.as_str() {
        "del" | "dly" => "delay",
        "rev" | "r" => "reverb",
        "mod" | "modulation" => "mod",
        "b" | "box" => "reverb",
        other => other,
    };
    CHAIN_ROLES
        .iter()
        .position(|(cli, disp)| *cli == alias || disp.to_ascii_lowercase() == alias)
        .map(|i| i as u8)
}

/// 按链角色索引返回设备显示名称。
pub fn chain_role_name(index: u8) -> Option<&'static str> {
    CHAIN_ROLES.get(index as usize).map(|(_, disp)| *disp)
}

/// 插入该消息布局所需的固定 `0x00` 填充字节。
fn chain_encode(order: &[u8], insert_at: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(CHAIN_LEN + 1);
    v.extend_from_slice(&order[..insert_at]);
    v.push(0x00);
    v.extend_from_slice(&order[insert_at..]);
    v
}

/// 从线上字段中移除固定 `0x00` 填充字节。
///
/// 仅当 `remove_at` 处的字节确实是 `0x00` *且*剩余部分是 `0..CHAIN_LEN` 的一个排列时才返回 `Some`。
/// 这种严格性是有意为之：编码后的字段包含两个 `0x00` 字节（填充字节和合法的 `FX1` 元素），
/// 宽松的解码器可能悄悄产出错误的链。
fn chain_decode(encoded: &[u8], remove_at: usize) -> Option<Vec<u8>> {
    if encoded.len() != CHAIN_LEN + 1 || encoded.get(remove_at) != Some(&0x00) {
        return None;
    }
    let mut v = encoded.to_vec();
    v.remove(remove_at);
    let mut sorted = v.clone();
    sorted.sort_unstable();
    if sorted != (0..CHAIN_LEN as u8).collect::<Vec<_>>() {
        return None;
    }
    Some(v)
}

/// `0x6D` 链**写入**布局使用的填充索引（见 [`chain_write`]）。
pub const CHAIN_WRITE_FILLER: usize = 5;
/// `0x09` 设备**通知**布局使用的填充索引。
pub const CHAIN_NOTIFY_FILLER: usize = 1;
/// `0x43` 预设写入布局使用的填充索引。
pub const CHAIN_PRESET_FILLER: usize = 2;

/// `0x6D` sub `0x0A`，字段 `0x05`：写入效果链序。
///
/// `order` 是 `0..8` 的一个排列，按从左到右的链序使用 [`CHAIN_ROLES`] 索引。线上布局：
///
/// ```text
/// 00 0A 00 00 05 08 <9 bytes>
/// ```
///
/// 9 字节字段是在索引 [`CHAIN_WRITE_FILLER`] 处插入 `0x00` 后的 8 元素 `chainOrder`。若 `order` 不是排列则返回 `None`。
pub fn chain_write(order: &[u8]) -> Option<(u8, Vec<u8>)> {
    if order.len() != CHAIN_LEN {
        return None;
    }
    chain_decode(&chain_encode(order, CHAIN_WRITE_FILLER), CHAIN_WRITE_FILLER)?;
    let mut p = vec![0x00, sub::NAME, 0x00, 0x00, 0x05, CHAIN_LEN as u8];
    p.extend_from_slice(&chain_encode(order, CHAIN_WRITE_FILLER));
    Some((msg_type::DATA, p))
}

/// 解析 `0x6D` 链写入载荷。
pub fn parse_chain_write(payload: &[u8]) -> Option<Vec<u8>> {
    if payload.len() != 15 || payload[1] != sub::NAME || payload[4] != 0x05 {
        return None;
    }
    chain_decode(&payload[6..15], CHAIN_WRITE_FILLER)
}

/// 从 `0x09` 设备通知载荷（`00 08` + 9 字节）中解析 9 字节字段。
pub fn parse_chain_notify(payload: &[u8]) -> Option<Vec<u8>> {
    if payload.len() < 11 || payload[1] != CHAIN_LEN as u8 {
        return None;
    }
    chain_decode(&payload[2..11], CHAIN_NOTIFY_FILLER)
}

/// 设备为链写入返回的 `0x6D` **ACK** 所用的填充索引。
///
/// ACK 镜像请求（先是 `00 00`，然后从第 1 字节起的请求载荷），
/// 但以与写入**不同**的填充索引重新编码 9 字节字段。
/// 已在四个样本上验证 —— 两个抓包 ACK 和两个来自实机的实时 ACK ——
/// 移除索引 4 后得到的恰好是所请求的链序。
/// 注意该字段在孤立状态下天然有歧义（它包含两个 `0x00` 字节），所以这个索引很重要。
pub const CHAIN_ACK_FILLER: usize = 4;

/// 从 `0x6D` ACK 载荷中解析链序。
///
/// ACK 是 `00 00` 后接请求载荷自第二个字节起的内容，
/// 因此子命令 id 位于 `[2]`，字段选择器位于 `[5]`，9 字节链字段位于 `[7..16)`。
/// 对请求 `[4,7,0,1,2,3,5,6]` 观察到的实时载荷：
///
/// ```text
/// 00 00 0a 00 00 05 08 04 07 00 01 00 02 03 05 06
/// ```
pub fn parse_chain_ack(payload: &[u8]) -> Option<Vec<u8>> {
    if payload.len() < 16 || payload[2] != sub::NAME || payload[5] != 0x05 {
        return None;
    }
    chain_decode(&payload[7..16], CHAIN_ACK_FILLER)
}

/// `0x6D` sub `0x0A`：重命名当前预设。
///
/// 注意：官方应用会在较长名称的内部插入一个 NUL 字节（`"preset42"` -> `"prese\0t42"`）；
/// 此处按原样写入给定字节。
pub fn set_name(name: &[u8]) -> (u8, Vec<u8>) {
    let mut p = vec![0x00, sub::NAME, 0x00, 0x00, 0x08, name.len() as u8];
    p.extend_from_slice(name);
    (msg_type::DATA, p)
}

/// 把预设名编码为设备名字区的 9 字节线上形态。
///
/// 规则（由恢复出厂抓包的 40/40 个出厂名逆向，全部吻合）：名字（≤ 8 个
/// 可打印 ASCII 字符）先填充到 8 字节，再在下标 6 处插入一个 `0x00`——
/// 与 [`chain_encode`] 的链序填充同族。设备显示时跳过 NUL 收集可打印字符。
///
/// 例：`"ClnArp"` -> `43 6c 6e 41 72 70 00 00 00`，`"BassFuzz"` ->
/// `42 61 73 73 46 75 00 7a 7a`。
///
/// [`chain_encode`]: crate::protocol::chain_encode
pub fn encode_preset_name_field(name: &str) -> Result<[u8; 9], String> {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > 8 {
        return Err(format!("preset name must be 1-8 characters, got {}", bytes.len()));
    }
    if !bytes.iter().all(|&b| (0x20..=0x7E).contains(&b)) {
        return Err("preset name must be printable ASCII".to_string());
    }
    let mut padded = [0x00u8; 8];
    padded[..bytes.len()].copy_from_slice(bytes);
    let mut out = [0x00u8; 9];
    out[..6].copy_from_slice(&padded[..6]);
    // 下标 6 固定插入 0x00，剩余字符跟在其后。
    out[7..9].copy_from_slice(&padded[6..8]);
    Ok(out)
}

/// `0x6D` sub `0x0A`：以 9 字节名字区形态重命名当前预设。
///
/// 与官方抓包样本（"preset42" -> `count=08` + 9 字节载荷）同构；
/// 名字区编码见 [`encode_preset_name_field`]。
pub fn set_name_field(field: &[u8; 9]) -> (u8, Vec<u8>) {
    let mut p = vec![0x00, sub::NAME, 0x00, 0x00, 0x08, 0x08];
    p.extend_from_slice(field);
    (msg_type::DATA, p)
}

/// `0x6D` 效果类型切换。
///
/// 线上布局（已针对 fx1 验证）：
/// ```text
/// 00 [4*(N+1)] 00 30 [slot] 00 [effect_index] [N] <float32 * N>
/// ```
/// * `effect_index` 是槽位内 0 起始的选项索引（fx1：0 = 噪声门，1 = 自动门，2 = NG+COM，3 = 压缩器）。
/// * `N` 是参数个数。
pub fn load_effect(slot: u8, effect_index: u8, model_flag: u8, params: &[f32]) -> (u8, Vec<u8>) {
    let n = params.len() as u8;
    let sub_byte = 4 * (params.len() as u8 + 1);
    let mut p = vec![0x00, sub_byte, 0x00, model_flag, slot, 0x00, effect_index, n];
    for &v in params {
        p.extend_from_slice(&encode_param_value(v));
    }
    (msg_type::DATA, p)
}

/// `0x66` sub `0x02`：设置 USB 音量（0-100）。
pub fn set_usb_volume(percent: u8) -> (u8, Vec<u8>) {
    (msg_type::USB_VOLUME, vec![0x00, 0x02, 0x00, 0x00, field::USB_VOLUME, percent])
}

/// `0x63`：请求当前预设快照。
///
/// 载荷为 `00 00 00`。在抓包中该命令之后总是紧跟元数据尾（`da 00 0e ...`）；见 [`crate::tail`]。
pub fn request_snapshot() -> (u8, Vec<u8>) {
    (msg_type::SNAPSHOT, vec![0x00, 0x00, 0x00])
}

/// `0x40`：查询从 `start_slot` 开始的 `count` 个槽位的预设名称。
///
/// 线上布局：`00 02 00 00 [start_slot] [count]`。抓包中 `start_slot` 取 `0x00, 0x08, 0x10, ..`，`count = 0x08`。
pub fn query_preset_list(start_slot: u8, count: u8) -> (u8, Vec<u8>) {
    (msg_type::PRESET_LIST, vec![0x00, 0x02, 0x00, 0x00, start_slot, count])
}

/// `0x68`：连通性 ping。
pub fn ping() -> (u8, Vec<u8>) {
    (msg_type::PING, vec![0x00, 0x00, 0x00])
}

/// `0x36`：查询单个音箱 / 驱动槽位的模型名称。
///
/// 观察到的 TX 查询载荷（`resona-import.mmon` 中全部 38 个槽位）：
///
/// ```text
/// 00 01 00 00 [index]
/// ```
///
/// 设备以携带模型名称的 `0x36` 帧应答；见 [`parse_model_name`]。
/// `index` 是 0 起始的固件槽位（音箱 + FX2 驱动踏板共 0-37）。
/// 箱体 / IR 槽位（0-29）使用不同的消息，本查询不枚举它们 —— 见协议笔记。
pub fn model_query(index: u8) -> (u8, Vec<u8>) {
    (msg_type::MODEL_LIST, vec![0x00, 0x01, 0x00, 0x00, index])
}

// ─── 解析器 ───────────────────────────

/// 来自 `0x06` 消息的一组效果参数。
#[derive(Debug, Clone, PartialEq)]
pub struct EffectParams {
    pub slot: u8,
    pub params: Vec<f32>,
}

/// 解析 `0x06` 效果数据载荷：
/// `00 01 [slot] [count] <float32 * count>`。
pub fn parse_effect_data(payload: &[u8]) -> Option<EffectParams> {
    if payload.len() < 4 || payload[0] != 0x00 || payload[1] != 0x01 {
        return None;
    }
    let slot = payload[2];
    let count = payload[3] as usize;
    let body = &payload[4..];
    if body.len() < count * 4 {
        return None;
    }
    let params = (0..count)
        .map(|i| decode_param_float(&body[i * 4..]).unwrap())
        .collect();
    Some(EffectParams { slot, params })
}

/// 来自 `0x40` 应答的带名称预设列表条目。
#[derive(Debug, Clone, PartialEq)]
pub struct PresetEntry {
    pub slot: u8,
    pub named: bool,
    pub name: String,
}

/// 存储的预设名称的字符数。名称以空格填充到此宽度，并且以在分片之间插入 `0x00` 字节的方式存储。
pub const PRESET_NAME_CHARS: usize = 8;

/// 解析 `0x40` 预设列表应答载荷。
///
/// 布局 —— **端到端验证过**，不是推断：
///
/// ```text
/// [0..7]  header: 00 00 52 00 00 BASE COUNT      (BASE = first slot, COUNT = 8)
/// then COUNT records:
///   [slot: 1 byte, MUST equal BASE + k]
///   [1+ bytes < 0x20   -- the first one is 0x01 when the slot holds a name]
///   [name: exactly 8 printable characters]
/// ```
///
/// 名称字段以 **NUL 分隔的分片**形式存储：`0x00` 字节被跳过，且**不**计入 8 个字符。
/// 这就是为什么记录的字节长度会变化（11 或 12），尽管每个名称都是 8 字符宽。
/// 8 个字符之后、下一条记录的槽位索引之前，至多还有一个 `< 0x20` 的字节。
///
/// 验证证据：
/// * `preset-export.mmon` —— 全部 8 个数据块（64 个槽位）解码后每个槽位索引都匹配 `BASE + k`，
///   且每个名称都恰好 8 字符宽。
/// * 以此方式解码出的 40 个出厂名称与出厂预设表 -> `products.nanocore.presets`（独立来源）**逐字节一致**。
///   槽位 41..63 携带设备占位名称 `preset`。
///
/// 每条记录都做交叉校验，任何错位都会返回 `None`，而不是悄悄产出错误名称。
///
/// 当槽位索引之后的前导填充字节**不全为零**时 `named` 为 `true`。
/// 在抓包上验证：全部 40 个出厂槽位加上槽位 40 的用户预设都被标记，
/// 而 24 个占位槽位（41..63，其存储的名称字面上就是 `preset`）的前导字节全为零。
/// 这 1-2 个字节的确切字节级含义尚未完全解释清楚，
/// 因此 `named` 应当被理解为"设备将此槽位标记为持有名称"，而不是一个有文档定义的标志值。
pub fn parse_preset_list(payload: &[u8]) -> Option<Vec<PresetEntry>> {
    // 头部恰好 7 字节，且以 `0x52` 体标记开头。
    if payload.len() < 8 || payload[2] != 0x52 {
        return None;
    }
    let base = payload[5];
    let count = payload[6] as usize;
    if count == 0 || count > 64 {
        return None;
    }

    let mut out = Vec::with_capacity(count);
    let mut i = 7usize;
    for k in 0..count {
        let slot = base.wrapping_add(k as u8);
        // 1. 槽位索引，与预期位置交叉校验。
        if i >= payload.len() || payload[i] != slot {
            return None;
        }
        i += 1;

        // 2. 一个或多个前导字节 < 0x20；`named` = 不全为零。
        if i >= payload.len() || payload[i] >= 0x20 {
            return None;
        }
        let mut named = false;
        while i < payload.len() && payload[i] < 0x20 {
            named |= payload[i] != 0x00;
            i += 1;
        }

        // 3. 恰好 PRESET_NAME_CHARS 个可打印字符，跳过 NUL。
        let mut name = String::with_capacity(PRESET_NAME_CHARS);
        while i < payload.len() && name.len() < PRESET_NAME_CHARS {
            let b = payload[i];
            if b == 0x00 {
                i += 1;
            } else if (0x20..=0x7E).contains(&b) {
                name.push(b as char);
                i += 1;
            } else {
                return None;
            }
        }
        if name.len() != PRESET_NAME_CHARS {
            return None;
        }
        out.push(PresetEntry { slot, named, name: name.trim_end().to_string() });

        // 4. 至多一个尾随字节 < 0x20，但绝不可能是下一个槽位索引。
        //    （被标记的槽位可能持有 < 0x20 的值，所以应与预期的下一个
        //    索引比较，而不是与阈值比较。）
        let next_slot = base.wrapping_add(k as u8).wrapping_add(1);
        if i < payload.len() && payload[i] < 0x20 && payload[i] != next_slot {
            i += 1;
        }
    }
    Some(out)
}

/// 解码原始快照载荷中所有合理的 7 比特安全 float32 值。
///
/// 返回 `[0.0, 1.0]` 范围内值的 `(offset, value)` 对。
pub fn scan_snapshot_floats(payload: &[u8]) -> Vec<(usize, f32)> {
    let mut out = Vec::new();
    if payload.len() < 4 {
        return out;
    }
    for i in 0..=payload.len() - 4 {
        let b = [payload[i], payload[i + 1], payload[i + 2], payload[i + 3]];
        let v = f32::from_le_bytes(b);
        if v.is_finite() && (0.0..=1.0).contains(&v) && v.to_le_bytes() == b {
            out.push((i, v));
        }
    }
    out
}

/// `0x63` 快照载荷尾部链字段所用的填充索引。
///
/// 已在 fw 1.04 真机（USB）上用非恒等链序实验定案：
/// 写入 `[4,0,1,2,3,5,6,7]` 后读快照，末 9 字节为
/// `04 00 01 02 03 05 06 00 07`，只有去掉索引 7 的 `0x00` 才还原出写入的链序
/// （索引 1 的歧义解 `[4,1,2,3,5,6,0,7]` 被真值排除）。
/// 恒等链序快照（当日与 8 天前的 baseline）同形态吻合。
/// 注意：官方 App 抓包（BLE）里的快照尾部形态与此不同且均为恒等链，填充下标
/// 在那里不可观测——本常量仅对真机 USB 快照成立。
pub const CHAIN_SNAPSHOT_FILLER: usize = 7;

/// 从 `0x63` 快照载荷尾部解析 `chainOrder`。
///
/// 真机（USB）快照的链字段位于载荷最末：
///
/// ```text
/// … 3f | 08 | 00 | <9 字节链字段>
///   ^浮动数据  ^计数  ^分隔  ^载荷末 9 字节
/// ```
///
/// `0x08` 是链元素计数，其后固定一个 `0x00` 分隔字节，
/// 末 9 字节是在索引 [`CHAIN_SNAPSHOT_FILLER`] 处插入 `0x00` 后的 8 元素链序。
/// 计数与分隔的锚定加上严格排列校验使误报概率可忽略；
/// 恒等链序的歧义解（如填充索引 1）不满足 `field[7] == 0x00` 时自然被拒。
pub fn parse_snapshot_chain_order(payload: &[u8]) -> Option<Vec<u8>> {
    let n = payload.len();
    if n < CHAIN_LEN + 3 || payload[n - 11] != CHAIN_LEN as u8 || payload[n - 10] != 0x00 {
        return None;
    }
    chain_decode(&payload[n - 9..], CHAIN_SNAPSHOT_FILLER)
}

/// `0x63` 快照载荷经 `[mask][7 data]` 组连续解包后的字节流。
///
/// 2026-09-30 对 40 组出厂快照（`nanocore-reverse/data/snapshots-factory40/`）
/// 字节级定案：载荷从 `[3]` 起（含）是**一条连续打包流**——
/// 每 8 个线上字节为一组，组首字节是掩码（bit *j* 对应组内第 *j* 个数据字节的
/// bit7），其余 7 字节是数据。记录头、活动预设索引、音量、链序计数全部
/// 在流内，仅 `[0..3)`（`00 00 <X>`）留在流外。
///
/// 掩码落在 `[3 + 8k]`；位于掩码位置的载荷字节被"吞掉"（不进入数据流）——
/// 快照尾部的链序分隔字节/填充 `0x00` 恰好落在某些掩码位上，
/// 因此解包后的尾部比原始字节短，这是正常现象（那些字节值恒为 `0x00`）。
pub fn unpack_snapshot_stream(payload: &[u8]) -> Option<Vec<u8>> {
    if payload.len() < 12 || payload[0] != 0x00 || payload[1] != 0x00 || payload[3] != 0x00 {
        return None;
    }
    let mut out = Vec::with_capacity(payload.len() * 7 / 8);
    let mut i = 3;
    while i < payload.len() {
        let mask = payload[i];
        for j in 1..8 {
            if i + j >= payload.len() {
                break;
            }
            let mut b = payload[i + j];
            if mask & (1 << (j - 1)) != 0 {
                b |= 0x80;
            }
            out.push(b);
        }
        i += 8;
    }
    Some(out)
}

/// 解包后的一条链位记录：`[enabled][variant][paramCount] + float32 × count`。
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotRecord {
    /// 链位使能（`0x01` = 开）。
    pub enabled: u8,
    /// 效果类型（variant）索引。
    pub variant: u8,
    /// 全精度 float32 参数（bit7 已由掩码还原）。
    pub params: Vec<f32>,
}

/// `0x63` 快照的完整结构化解码。
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotState {
    /// 活动预设索引（0 起始；与 `preset set` 往返一致）。
    pub preset_index: u8,
    /// 预设音量（0–100）。
    pub preset_volume: u8,
    /// 恰 8 条记录（每条对应 `chainOrder` 的一个链位）。
    pub records: Vec<SnapshotRecord>,
}

/// 完整解码 `0x63` 快照载荷（真机 USB 形态，无 BLE 元数据尾）。
///
/// 数据流结构（掩码还原后）：
///
/// ```text
/// [00][03][presetIndex][presetVolume]
/// × 8: [enabled][variant][count] + float32 × count
/// [08] + 链序（sep/填充 0x00 可能已被掩码位置吞掉）
/// ```
///
/// 校验：前导标记 `[00][03]`、记录数恰为 8、每条 `count ≤ 16`、
/// float 不得为 NaN/Inf（与固件解码器 `FUN_004777bc` 的
/// `(bits & 0x7FFFFFFF) <= 0x7F7FFFFF` 一致）、记录后紧跟链计数 `0x08`。
///
/// 注意：**不要**用 [`parse_snapshot_chain_order`] 做前置校验 —— 它锚定
/// 原始尾部的 `count@len-11` 布局，那只对特定快照长度（197B/165B，
/// 掩码相位恰好把 sep/填充留在原位）成立；187B 等其它长度的原始尾部
/// 字节因掩码吞位而整体移位。链序请继续用 [`parse_snapshot_chain_order`]
/// 单独解（它对已验证的形态可靠），结构化解码以数据流内的 `0x08` 计数为准。
///
/// 已知语义差异（**不是**解码错误，40 组基线上与 manifest 对账得出）：
/// * `fx1`（噪音门）/`fx2`（压缩器）的 `enabled` 在快照里是**实时 DSP 状态**
///   （无输入信号时门/压控报告关闭），manifest 走 `0x41` 静态路径恒为 `1`；
/// * `fx8` 的参数是运行时值，与静态出厂值系统性地不同。
pub fn decode_snapshot_state(payload: &[u8]) -> Option<SnapshotState> {
    let data = unpack_snapshot_stream(payload)?;
    let mut p = 0usize;
    let mut take = |k: usize| -> Option<&[u8]> {
        let r = data.get(p..p + k)?;
        p += k;
        Some(r)
    };
    let hdr = take(4)?;
    if hdr[0] != 0x00 || hdr[1] != 0x03 {
        return None;
    }
    let preset_index = hdr[2];
    let preset_volume = hdr[3];
    let mut records = Vec::with_capacity(CHAIN_LEN);
    for _ in 0..CHAIN_LEN {
        let h = take(3)?;
        let (enabled, variant, count) = (h[0], h[1], h[2]);
        if count > 16 {
            return None;
        }
        let raw = take(count as usize * 4)?;
        let mut params = Vec::with_capacity(count as usize);
        for c in raw.as_chunks::<4>().0 {
            let bits = u32::from_le_bytes(*c);
            // 与固件解码器一致：拒绝 NaN/Inf。
            if bits & 0x7FFF_FFFF > 0x7F7F_FFFF {
                return None;
            }
            params.push(f32::from_bits(bits));
        }
        records.push(SnapshotRecord { enabled, variant, params });
    }
    // 链计数标记（sep/填充可能被吞，但计数 0x08 始终在数据流内）。
    if take(1)?[0] != CHAIN_LEN as u8 {
        return None;
    }
    Some(SnapshotState { preset_index, preset_volume, records })
}

/// 从 `0x43` 预设写入载荷中解析 `chainOrder`。
///
/// 已观察到两种布局：
///
/// * **234 字节预设帧**（`factory-reset.mmon` 中有 40 个，每个出厂预设一个）：
///   链字段位于 `[33..42)`，其前是 `[31]` 处的字段 id `0x03` 和 `[32]` 处的计数 `0x08`。
///   9 字节字段是在索引 [`CHAIN_PRESET_FILLER`] 处插入 `0x00` 后的 8 元素链序。
///   已对照 `.npreset` 的 `manifest.json` -> `presets[].preset.chainOrder` 验证，
///   **40/40** 个预设全部吻合，其中包括一个非恒等链序（`[0,1,2,3,5,4,6,7]`）。
/// * **旧的 356 字节"首消息"**：字段 id 在 `[30]`，计数在 `[31]`，链序从 `[32]` 开始（无填充字节）。为旧分析路径保留。
pub fn parse_chain_order(payload: &[u8]) -> Option<Vec<u8>> {
    // 234 字节预设帧。
    if payload.len() >= 42 && payload[31] == 0x03 && payload[32] == CHAIN_LEN as u8 {
        return chain_decode(&payload[33..42], CHAIN_PRESET_FILLER);
    }
    // 旧的 356 字节布局（无填充字节）。
    if payload.len() < 40 || payload[30] != 0x03 {
        return None;
    }
    let count = payload[31] as usize;
    if count == 0 || count > 16 || payload.len() < 32 + count {
        return None;
    }
    Some(payload[32..32 + count].to_vec())
}

/// 从 `0x36` 应答载荷解析出的模型列表条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelName {
    /// 0 起始的固件槽位（设备 UI 显示 `index + 1`）。
    pub index: u8,
    /// 设备的模型字符串，由 NUL 分隔的分片重组而成。
    pub name: String,
    /// 应答第 7 字节非零时为 `true`。
    ///
    /// **这不是"用户模型"标记。** 本模块的早期版本基于 `resona-import` 抓包做出了相反的假设，
    /// 该抓包中槽位 29（用户导入的 NAM）是唯一被标记的槽位。
    /// `preset-export` 抓包推翻了这一点：它只查询槽位 9，该槽位持有*出厂*模型 `Mar8001`，
    /// 而第 7 字节仍是 `0x01`。
    ///
    /// 与两个抓包都相符的留存假设是：第 7 字节标记模型**当前已加载 / 正在使用**的槽位，
    /// 即在导入抓包中活动音箱是槽位 29 的用户 NAM，在导出抓包中活动音箱是槽位 9 的 `Mar8001`。
    ///
    /// 要识别用户/自定义模型，请改为将 [`ModelName::name`] 与 [`crate::models_data`] 中的出厂表比较 —— 这与标志无关。
    pub active: bool,
}

/// 解析 `0x36` 模型列表应答载荷。
///
/// 布局（已对照 `resona-import.mmon` 的全部 38 条记录和 `preset-export.mmon` 的单槽位查询验证）：
///
/// ```text
/// 00 00 1e 10 00 [index] 01 [flag] 52 2f 00 [len] 00 [hash x4] 1a 1a 00 00 [name...]
/// ```
///
/// * `index` 位于第 5 字节。
/// * `flag` 位于第 7 字节 —— 见 [`ModelName::active`]；它*不是*用户模型标志。
/// * 模型名称紧随 `1a 1a 00 00` 标记之后，以一段 **NUL 分隔的可打印分片**存储。
///   拼接各分片即可重组设备的模型字符串：
///   * `BogXTC` + `1`        -> `BogXTC1`
///   * `NDSP A` + `rchetyp` + `e:` -> `NDSP Archetype:`
///
/// 不可打印字节和分隔用的 NUL 会被丢弃。
pub fn parse_model_name(payload: &[u8]) -> Option<ModelName> {
    if payload.len() < 6 {
        return None;
    }
    // 定位 `1a 1a 00 00` 名称标记。
    let mut marker = None;
    for i in 0..payload.len().saturating_sub(3) {
        if payload[i] == 0x1a && payload[i + 1] == 0x1a && payload[i + 2] == 0x00 && payload[i + 3] == 0x00 {
            marker = Some(i);
            break;
        }
    }
    let m = marker?;
    let index = payload[5];
    let active = payload.get(7).copied().unwrap_or(0) != 0;
    let mut name = String::new();
    for &b in &payload[m + 4..] {
        if b == 0x00 {
            continue;
        }
        if (0x20..=0x7E).contains(&b) {
            name.push(b as char);
        }
    }
    if name.is_empty() {
        return None;
    }
    Some(ModelName { index, name, active })
}

#[cfg(test)]
mod tests {
    use super::*;

    // 槽位 0（BogXTC1）和槽位 29（RESONA NAM 导入后的 NDSP Archetype:）
    // 的原始 `0x36` RX 载荷，逐字节取自 `resona-import.mmon`。
    const SLOT0: &[u8] = &[
        0x00, 0x00, 0x1e, 0x10, 0x00, 0x00, 0x01, 0x00, 0x52, 0x2f, 0x00, 0x18, 0x00, 0x6a,
        0x33, 0x02, 0x57, 0x1a, 0x1a, 0x00, 0x00, 0x42, 0x6f, 0x67, 0x58, 0x54, 0x43, 0x00,
        0x31, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    const SLOT29: &[u8] = &[
        0x00, 0x00, 0x1e, 0x10, 0x00, 0x1d, 0x01, 0x01, 0x52, 0x2f, 0x00, 0x0e, 0x00, 0x6b,
        0x7c, 0x21, 0x54, 0x1a, 0x1a, 0x00, 0x00, 0x4e, 0x44, 0x53, 0x50, 0x20, 0x41, 0x00,
        0x72, 0x63, 0x68, 0x65, 0x74, 0x79, 0x70, 0x00, 0x65, 0x3a, 0x00,
    ];
    // 来自 `preset-export.mmon` 的单槽位查询：槽位 9 持有*出厂*模型 `Mar8001`，
    // 但第 7 字节却是 0x01。这是证明第 7 字节并非用户模型标志的反例。
    const SLOT9_FACTORY_ACTIVE: &[u8] = &[
        0x00, 0x00, 0x1e, 0x10, 0x00, 0x09, 0x01, 0x01, 0x52, 0x2f, 0x00, 0x06, 0x00, 0x46,
        0x36, 0x31, 0x44, 0x1a, 0x1a, 0x00, 0x00, 0x4d, 0x61, 0x72, 0x38, 0x30, 0x30, 0x31,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn model_query_is_5_bytes_with_index() {
        let (k, p) = model_query(7);
        assert_eq!(k, msg_type::MODEL_LIST);
        assert_eq!(p, vec![0x00, 0x01, 0x00, 0x00, 7]);
    }

    #[test]
    fn parse_model_name_reconstructs_fragments() {
        let a = parse_model_name(SLOT0).expect("slot0 parse");
        assert_eq!(a.index, 0);
        assert_eq!(a.name, "BogXTC1");
        assert!(!a.active);

        let b = parse_model_name(SLOT29).expect("slot29 parse");
        assert_eq!(b.index, 29);
        assert_eq!(b.name, "NDSP Archetype:");
        assert!(b.active);
    }

    /// 针对修正后的第 7 字节语义的回归守卫：*出厂*模型也会置位该标志，
    /// 调用方绝不能把它当作"用户模型"。
    #[test]
    fn byte7_is_not_a_user_model_flag() {
        let m = parse_model_name(SLOT9_FACTORY_ACTIVE).expect("slot9 parse");
        assert_eq!(m.index, 9);
        assert_eq!(m.name, "Mar8001");
        assert!(m.active, "factory model Mar8001 also reports byte7 != 0");
    }

    #[test]
    fn parse_model_name_needs_marker() {
        // 无 `1a 1a 00 00` 锚点 -> None。
        assert!(parse_model_name(&[0x00, 0x00, 0x1e, 0x10, 0x00, 0x05]).is_none());
        // 在标记之前被截断。
        assert!(parse_model_name(&[]).is_none());
    }

    // 来自 `preset-export.mmon` 的真实 `0x40` PRESET_LIST 应答。
    // 下方全部字节逐字节取自抓包；
    // 解码出的名称已与出厂预设表 -> products.nanocore.presets 交叉核对。
    const PRESET_LIST_BASE32: &[u8] = &[
        0x00, 0x00, 0x52, 0x00, 0x00, 0x20, 0x08, 0x20, 0x01, 0x50, 0x77, 0x00, 0x43, 0x68,
        0x6f, 0x72, 0x64, 0x20, 0x21, 0x00, 0x01, 0x52, 0x61, 0x77, 0x4c, 0x65, 0x61, 0x00,
        0x64, 0x20, 0x22, 0x01, 0x53, 0x77, 0x65, 0x00, 0x65, 0x70, 0x20, 0x20, 0x20, 0x23,
        0x01, 0x00, 0x4d, 0x75, 0x74, 0x65, 0x4f, 0x44, 0x20, 0x00, 0x20, 0x24, 0x01, 0x42,
        0x61, 0x73, 0x73, 0x00, 0x46, 0x69, 0x6e, 0x67, 0x25, 0x01, 0x42, 0x00, 0x61, 0x73,
        0x73, 0x53, 0x6c, 0x61, 0x70, 0x00, 0x26, 0x01, 0x42, 0x61, 0x73, 0x73, 0x50, 0x00,
        0x69, 0x63, 0x6b, 0x27, 0x01, 0x42, 0x61, 0x00, 0x73, 0x73, 0x46, 0x75, 0x7a, 0x7a,
    ];
    const PRESET_LIST_BASE40: &[u8] = &[
        0x00, 0x00, 0x52, 0x00, 0x00, 0x28, 0x08, 0x28, 0x01, 0x70, 0x69, 0x00, 0x74, 0x62,
        0x20, 0x20, 0x20, 0x20, 0x29, 0x00, 0x00, 0x70, 0x72, 0x65, 0x73, 0x65, 0x74, 0x00,
        0x20, 0x20, 0x2a, 0x00, 0x70, 0x72, 0x65, 0x00, 0x73, 0x65, 0x74, 0x20, 0x20, 0x2b,
        0x00, 0x00, 0x70, 0x72, 0x65, 0x73, 0x65, 0x74, 0x20, 0x00, 0x20, 0x2c, 0x00, 0x70,
        0x72, 0x65, 0x73, 0x00, 0x65, 0x74, 0x20, 0x20, 0x2d, 0x00, 0x70, 0x00, 0x72, 0x65,
        0x73, 0x65, 0x74, 0x20, 0x20, 0x00, 0x2e, 0x00, 0x70, 0x72, 0x65, 0x73, 0x65, 0x00,
        0x74, 0x20, 0x20, 0x2f, 0x00, 0x70, 0x72, 0x00, 0x65, 0x73, 0x65, 0x74, 0x20, 0x20,
    ];

    #[test]
    fn preset_list_decodes_factory_names() {
        // 槽位 32..39 —— 包含作为可打印字符的槽位字节 0x20..0x27，
        // 所以朴素的"扫描槽位字节"方法在此会失效。
        let got = parse_preset_list(PRESET_LIST_BASE32).expect("base32 parse");
        let names: Vec<&str> = got.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["PwChord", "RawLead", "Sweep", "MuteOD", "BassFing", "BassSlap", "BassPick", "BassFuzz"]
        );
        assert!(got.iter().enumerate().all(|(k, e)| e.slot == 32 + k as u8));
        assert!(got.iter().all(|e| e.named));
    }

    #[test]
    fn preset_list_marks_placeholders_unnamed() {
        // 槽位 40 是用户预设（"pitb"）；41..47 是设备占位槽位，
        // 其存储的名称字面上就是 `preset`。
        let got = parse_preset_list(PRESET_LIST_BASE40).expect("base40 parse");
        assert_eq!(got.len(), 8);
        assert_eq!(got[0].slot, 40);
        assert_eq!(got[0].name, "pitb");
        assert!(got[0].named);
        for e in &got[1..] {
            assert_eq!(e.name, "preset");
            assert!(!e.named, "slot {} should be an unnamed placeholder", e.slot);
        }
    }

    /// 解码器会校验每条记录，因此损坏的槽位索引必须被拒绝，
    /// 而不是悄悄产出移位的名称。
    #[test]
    fn preset_list_rejects_misaligned_input() {
        let mut bad = PRESET_LIST_BASE32.to_vec();
        bad[7] = 0x21; // 首个槽位字节不再等于 BASE
        assert!(parse_preset_list(&bad).is_none());
        assert!(parse_preset_list(&[]).is_none());
        // 不是 `0x40` 体标记。
        assert!(parse_preset_list(&[0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x08]).is_none());
    }

    // ─── 链序 ────────────────────────
    // 下方三帧逐字节取自 `block-reorder.mmon`，
    // 其中用户把 DEL 移到链位 1，把 EQ 移到链位 2。
    // 按角色表（DEL == 4，EQ == 7），链必须解出为 [4,0,1,2,3,5,6,7]，
    // 再解出 [4,7,0,1,2,3,5,6]。

    /// `0x6D` 写入载荷，取自抓包。
    const CHAIN_WRITE_1: &[u8] = &[
        0x00, 0x0A, 0x00, 0x00, 0x05, 0x08, 0x04, 0x00, 0x01, 0x02, 0x03, 0x00, 0x05, 0x06, 0x07,
    ];
    const CHAIN_WRITE_2: &[u8] = &[
        0x00, 0x0A, 0x00, 0x00, 0x05, 0x08, 0x04, 0x07, 0x00, 0x01, 0x02, 0x00, 0x03, 0x05, 0x06,
    ];
    /// 匹配的 `0x09` 设备通知载荷（`00 08` + 9 字节）。
    const CHAIN_NOTIFY_1: &[u8] = &[
        0x00, 0x08, 0x04, 0x00, 0x00, 0x01, 0x02, 0x03, 0x05, 0x06, 0x07,
    ];
    const CHAIN_NOTIFY_2: &[u8] = &[
        0x00, 0x08, 0x04, 0x00, 0x07, 0x00, 0x01, 0x02, 0x03, 0x05, 0x06,
    ];

    #[test]
    fn chain_roles_match_the_enable_evidence() {
        // 用户启用了 DEL 和 EQ；设备收到 ENABLE slot=4 / slot=7。
        assert_eq!(chain_role_index("delay"), Some(4));
        assert_eq!(chain_role_index("del"), Some(4));
        assert_eq!(chain_role_index("mod"), Some(5));
        assert_eq!(chain_role_index("reverb"), Some(6));
        assert_eq!(chain_role_index("rev"), Some(6));
        assert_eq!(chain_role_index("eq"), Some(7));
        assert_eq!(chain_role_index("nope"), None);
        assert_eq!(chain_role_name(4), Some("DEL"));
    }

    #[test]
    fn chain_write_matches_the_capture_and_round_trips() {
        let order = [4u8, 0, 1, 2, 3, 5, 6, 7];
        let (k, p) = chain_write(&order).expect("valid permutation");
        assert_eq!(k, msg_type::DATA);
        assert_eq!(p, CHAIN_WRITE_1, "byte-exact against the capture");
        assert_eq!(parse_chain_write(&p).unwrap(), order.to_vec());

        let order2 = [4u8, 7, 0, 1, 2, 3, 5, 6];
        let (_, p2) = chain_write(&order2).unwrap();
        assert_eq!(p2, CHAIN_WRITE_2);
        assert_eq!(parse_chain_write(&p2).unwrap(), order2.to_vec());
    }

    #[test]
    fn chain_notify_decodes_to_the_same_chain_as_the_write() {
        // 同一链的两个*独立*编码必须一致。
        for (write, notify) in [(CHAIN_WRITE_1, CHAIN_NOTIFY_1), (CHAIN_WRITE_2, CHAIN_NOTIFY_2)] {
            let a = parse_chain_write(write).expect("write parse");
            let b = parse_chain_notify(notify).expect("notify parse");
            assert_eq!(a, b);
        }
        assert_eq!(parse_chain_notify(CHAIN_NOTIFY_2).unwrap(), vec![4, 7, 0, 1, 2, 3, 5, 6]);
    }

    #[test]
    fn chain_order_from_a_234_byte_preset_frame() {
        // `factory-reset.mmon`：`ClnArp` 预设帧的链字段是恒等链序 [0,1,2,3,4,5,6,7]，
        // 0x00 填充位于索引 2。
        let mut frame = vec![0u8; 234];
        frame[31] = 0x03;
        frame[32] = 0x08;
        frame[33..42].copy_from_slice(&[0x00, 0x01, 0x00, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07]);
        assert_eq!(parse_chain_order(&frame).unwrap(), vec![0, 1, 2, 3, 4, 5, 6, 7]);

        // 同一抓包中的判别性（非恒等）样本：
        // `ColdCln` -> [0,1,2,3,5,4,6,7]。
        let mut f2 = vec![0u8; 234];
        f2[31] = 0x03;
        f2[32] = 0x08;
        f2[33..42].copy_from_slice(&[0x00, 0x01, 0x00, 0x02, 0x03, 0x05, 0x04, 0x06, 0x07]);
        assert_eq!(parse_chain_order(&f2).unwrap(), vec![0, 1, 2, 3, 5, 4, 6, 7]);
    }

    #[test]
    fn chain_rejects_non_permutations_and_bad_filler() {
        assert!(chain_write(&[0, 1, 2, 3, 4, 5, 6]).is_none(), "too short");
        assert!(chain_write(&[0, 1, 2, 3, 4, 5, 6, 7, 0]).is_none(), "too long");
        assert!(chain_write(&[0, 0, 2, 3, 4, 5, 6, 7]).is_none(), "duplicate");
        assert!(chain_write(&[0, 1, 2, 3, 4, 5, 6, 9]).is_none(), "out of range");
        // 写入布局中填充字节必须确实位于索引 5。
        let mut bad = CHAIN_WRITE_1.to_vec();
        bad[6] = 0x00;
        bad[11] = 0x04;
        assert!(parse_chain_write(&bad).is_none());
    }

    // ─── 快照尾部链字段 ────────────────
    // 逐字节取自真机（fw 1.04，USB）的 `0x63` 快照载荷尾部。

    /// 恒等链序快照的末 12 字节（写入实验前的原始状态，
    /// 与 8 天前 `data/baseline-20260922/snapshot.txt` 的尾部逐字节一致）。
    const SNAPSHOT_TAIL_IDENTITY: [u8; 12] =
        [0x3f, 0x08, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x00, 0x07];

    /// 写入 `[4,0,1,2,3,5,6,7]`（`chain set 4 0 1 2 3 5 6 7`）后快照的末 12 字节。
    const SNAPSHOT_TAIL_DEL_FIRST: [u8; 12] =
        [0x3f, 0x08, 0x00, 0x04, 0x00, 0x01, 0x02, 0x03, 0x05, 0x06, 0x00, 0x07];

    #[test]
    fn chain_order_from_snapshot_tail() {
        let mut p = vec![0x00u8; 197 - 12];
        p.extend_from_slice(&SNAPSHOT_TAIL_IDENTITY);
        assert_eq!(parse_snapshot_chain_order(&p).unwrap(), vec![0, 1, 2, 3, 4, 5, 6, 7]);

        // 非恒等判别样本：填充索引 7 是唯一还原出所写链序的位置
        // （索引 1 的歧义解 [4,1,2,3,5,6,0,7] 被真值排除）。
        let mut q = vec![0x00u8; 197 - 12];
        q.extend_from_slice(&SNAPSHOT_TAIL_DEL_FIRST);
        assert_eq!(parse_snapshot_chain_order(&q).unwrap(), vec![4, 0, 1, 2, 3, 5, 6, 7]);
    }

    #[test]
    fn snapshot_chain_rejects_bad_anchors_and_filler() {
        // 太短。
        assert!(parse_snapshot_chain_order(&SNAPSHOT_TAIL_IDENTITY.to_vec()[..10]).is_none());
        // 计数字节不是 0x08。
        let mut bad = SNAPSHOT_TAIL_IDENTITY;
        bad[1] = 0x07;
        let mut p = vec![0x00u8; 185];
        p.extend_from_slice(&bad);
        assert!(parse_snapshot_chain_order(&p).is_none());
        // 分隔字节缺失（官方 App BLE 抓包中的形态，末 9 字节的索引 7 不是 0x00）。
        let mut q = vec![0x00u8; 185];
        q.extend_from_slice(&[0x3f, 0x08, 0x00, 0x01, 0x02, 0x00, 0x03, 0x04, 0x05, 0x06, 0x07]);
        assert!(parse_snapshot_chain_order(&q).is_none());
    }

    // ─── 快照完整解码 ────────────────
    // 两个向量逐字节取自 40 组出厂基线
    // （nanocore-reverse/data/snapshots-factory40/slot_{0,1}.bin，
    //   2026-09-30 采集于官方恢复出厂后的原厂状态）。

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// `preset set 0`（ClnArp）后的快照载荷，187 字节。
    const SNAP_CLNARP: &str = "00002000000300540103060336731d3f00000002006c51383e295c050f3d381e053e5201381e3f000003000000003f0000003f034d4c4c3f000005031a19193f000000443f1f052b3f0a570c633f4d4c4c3f000c00024d4c0c3f334033333f0000041a1919193e414a213f300000003f1a1919603f0100025c0f420e3e4d4c4c3e010120057b142e3e6c5140383e713d4a3f4d014c4c3e7b142e3e000000040000003f000000003f000000003f0000003f08000001020304050607";
    /// `preset set 1`（Ambient）后的快照载荷，165 字节。
    const SNAP_AMBIENT: &str = "00000c0000030153010102400000003f1d5a24003e000003000000603f0000003f4d4c004c3f00000566664c663f4d4c0c3f1a1919193f4375283f034d4c4c3f00000202142e073f713d0a303f0101031a19196c3e6065503e1a1901193e000002000000003f0000003f01500605295c0f3d4d194c4c3f1a19193f4000000000666666003e000004000000003f0000003f000000003f0000003f0800000102030405060007";

    #[test]
    fn unpack_stream_restores_masked_high_bits() {
        let d = unpack_snapshot_stream(&unhex(SNAP_AMBIENT)).unwrap();
        // fx7 记录的 p1 = 0.321 = 0x3EA45A1D —— 掩码 bit6 还原了 a4 的高位。
        // （d[0..4) 前导 + d[4..7) 记录头 + p0@7..11 → p1@11..15。）
        assert_eq!(&d[11..15], &0x3EA4_5A1Du32.to_le_bytes());
        // 掩码位置的 0x00（尾部链序分隔/填充）被吞掉：165 - 3 = 162 个线上字节
        // = 21 组（20 完整 + 1 尾组仅 1 数据字节）→ 20×7 + 1 = 141 个数据字节。
        assert_eq!(d.len(), 141);
    }

    #[test]
    fn snapshot_state_decodes_clnarp_exactly() {
        let s = decode_snapshot_state(&unhex(SNAP_CLNARP)).expect("ClnArp decode");
        assert_eq!(s.preset_index, 0);
        assert_eq!(s.preset_volume, 84);
        assert_eq!(s.records.len(), 8);
        let r0 = &s.records[0];
        assert_eq!((r0.enabled, r0.variant), (1, 3));
        assert_eq!(r0.params.len(), 6);
        // 与 manifest 逐位一致（0.617 的 float32 恰为 0x3F1DF3B6）。
        assert_eq!(r0.params[0], f32::from_bits(0x3F1D_F3B6));
        assert_eq!(r0.params[1], 0.0);
        assert_eq!(r0.params[2], f32::from_bits(0x3E38_51EC)); // 0.18
        assert_eq!(r0.params[3], f32::from_bits(0x3D8F_5C29)); // 0.07
        assert_eq!(r0.params[4], f32::from_bits(0x3E05_1EB8)); // 0.13
        assert_eq!(r0.params[5], f32::from_bits(0x3F1E_B852)); // 0.62
        // fx8（rec1）与 fx4（rec5）/fx3（rec6，variant=1）。
        assert_eq!((s.records[1].enabled, s.records[1].params.len()), (0, 3));
        assert_eq!((s.records[5].enabled, s.records[5].variant), (1, 0));
        assert_eq!((s.records[6].enabled, s.records[6].variant), (1, 1));
        assert_eq!(s.records[6].params[2], f32::from_bits(0x3F4A_3D71)); // 0.79
    }

    #[test]
    fn snapshot_state_decodes_ambient_exactly() {
        let s = decode_snapshot_state(&unhex(SNAP_AMBIENT)).expect("Ambient decode");
        assert_eq!(s.preset_index, 1);
        assert_eq!(s.preset_volume, 83);
        let r0 = &s.records[0];
        assert_eq!((r0.enabled, r0.variant), (1, 1));
        assert_eq!(r0.params[0], 0.5);
        assert_eq!(r0.params[1], f32::from_bits(0x3EA4_5A1D)); // 0.321 —— 掩码还原后全精度
        assert_eq!((s.records[4].enabled, s.records[4].variant), (1, 1));
        assert_eq!(s.records[6].variant, 6);
    }

    #[test]
    fn snapshot_state_rejects_corrupt_input() {
        let good = unhex(SNAP_CLNARP);
        // 截断到数据流不足以容纳 8 条记录（-12 字节 → 解包不足 153 字节）。
        assert!(decode_snapshot_state(&good[..good.len() - 12]).is_none());
        // 数据流标记字节 [1] 不是 0x03。
        let mut bad = good.clone();
        bad[5] = 0x04;
        assert!(decode_snapshot_state(&bad).is_none());
        // 记录的参数数超过 16（rec2 的 count 字节 0x05 → 0x11）。
        let mut big = good.clone();
        big[58] = 0x11;
        assert!(decode_snapshot_state(&big).is_none());
    }

    // ─── 链 ACK ──────────────────────
    // 逐字节取自实机（fw 1.04 上的 `nanocore-cli chain …`）。
    // ACK 以索引 4 而非 5 的填充位置重新编码同一链序。

    /// 对请求恒等链序 `[0,1,2,3,4,5,6,7]` 的 ACK。
    const CHAIN_ACK_IDENTITY: &[u8] = &[
        0x00, 0x00, 0x0A, 0x00, 0x00, 0x05, 0x08, 0x00, 0x01, 0x02, 0x03, 0x00, 0x04, 0x05, 0x06,
        0x07,
    ];
    /// 对请求 `[4,7,0,1,2,3,5,6]` 的 ACK。
    const CHAIN_ACK_DEL_EQ: &[u8] = &[
        0x00, 0x00, 0x0A, 0x00, 0x00, 0x05, 0x08, 0x04, 0x07, 0x00, 0x01, 0x00, 0x02, 0x03, 0x05,
        0x06,
    ];

    #[test]
    fn chain_ack_round_trips_the_requested_order() {
        assert_eq!(
            parse_chain_ack(CHAIN_ACK_IDENTITY).unwrap(),
            vec![0, 1, 2, 3, 4, 5, 6, 7]
        );
        assert_eq!(
            parse_chain_ack(CHAIN_ACK_DEL_EQ).unwrap(),
            vec![4, 7, 0, 1, 2, 3, 5, 6]
        );
        // 并且它与 `chain_write` 对同一链序的产物一致 ——
        // 写入和 ACK 使用不同的填充索引，但链相同。
        let (_, req) = chain_write(&[4, 7, 0, 1, 2, 3, 5, 6]).unwrap();
        assert_ne!(&req[6..15], &CHAIN_ACK_DEL_EQ[7..16], "encodings differ");
        assert_eq!(parse_chain_write(&req).unwrap(), parse_chain_ack(CHAIN_ACK_DEL_EQ).unwrap());
    }

    // ── 线上安全参数编码 ────────────────────────────────────────────────────

    #[test]
    fn param_write_matches_the_captured_layout() {
        let (kind, p) = param_write(0x00, 0x01, 0.25);
        assert_eq!(kind, msg_type::DATA);
        assert_eq!(p.len(), 11);
        // 00 07 00 [mask] 01 [b5] [b6] <float32> —— 与 805 条抓包同构。
        assert_eq!(&p[..3], &[0x00, 0x07, 0x00]);
        assert_eq!(&p[4..7], &[0x01, 0x00, 0x01]);
        // 0.25f32 = 0x3E800000 -> LE `00 00 80 3E`；v2 的 bit7 被剥掉 -> mask bit5。
        assert_eq!(p[3], 0x20);
        assert_eq!(&p[7..], &[0x00, 0x00, 0x00, 0x3E]);
    }

    #[test]
    fn param_write_carries_the_stripped_bits_in_the_mask() {
        // 真机验证过的两条编码（cab b5=3 b6=1）。
        let (_, p) = param_write(0x03, 0x01, 0.6);
        assert_eq!(p[3], 0x18);
        assert_eq!(&p[7..], &[0x1a, 0x19, 0x19, 0x3f]);
        let (_, p) = param_write(0x03, 0x01, 1.0);
        assert_eq!(p[3], 0x20);
        assert_eq!(&p[7..], &[0x00, 0x00, 0x00, 0x3f]);
    }

    #[test]
    fn param_write_round_trips_through_the_mask() {
        for v in [0.0f32, 0.1, 0.2, 0.25, 0.3, 0.5, 0.6, 0.75, 1.0, 0.3333333, 0.123_456_79] {
            let (_, p) = param_write(0x03, 0x01, v);
            let mask = p[3];
            assert!(
                p.iter().all(|b| b & 0x80 == 0),
                "{v}: payload must be 7-bit safe"
            );
            let mut raw = [0u8; 4];
            raw.copy_from_slice(&p[7..11]);
            for (j, b) in raw.iter_mut().enumerate() {
                if mask & (1 << (3 + j)) != 0 {
                    *b |= 0x80;
                }
            }
            assert_eq!(f32::from_le_bytes(raw), v, "{v}: mask {mask:#04x}");
        }
    }
}

#[cfg(test)]
mod name_field_tests {
    use super::*;

    #[test]
    fn name_field_matches_all_factory_forms() {
        // 逐字节对照恢复出厂抓包中的出厂名（40/40 的抽样验证）。
        let cases: &[(&str, &str)] = &[
            ("ClnArp", "436c6e417270000000"),
            ("Ambient", "416d6269656e007400"),
            ("RhyOD", "5268794f4400000000"),
            ("EpicSolo", "45706963536f006c6f"),
            ("BassFuzz", "426173734675007a7a"),
            ("Sweep", "537765657000000000"),
            ("ThickCln", "546869636b43006c6e"),
        ];
        for (name, want_hex) in cases {
            let enc = encode_preset_name_field(name).unwrap();
            let hex: String = enc.iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(hex, *want_hex, "name {name:?}");
        }
    }

    #[test]
    fn name_field_rejects_bad_input() {
        assert!(encode_preset_name_field("").is_err(), "empty");
        assert!(encode_preset_name_field("123456789").is_err(), "too long");
        assert!(encode_preset_name_field("ab\0cd").is_err(), "embedded NUL");
    }
}
