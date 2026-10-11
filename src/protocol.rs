//! 已确认消息类型的载荷构造器与解析器。
//!
//! 全部布局直接对应设备在 MIDI 上的实际字节。

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

/// 剥掉 float32 四个字节的最高位 —— SysEx 数据字节必须 `< 0x80`。
///
/// **不能只剥不还**：剥掉的位由 [`param_value_mask`] 记录，
/// [`param_write`] 把它放进消息的 mask 字节，设备据此还原，
/// 因此 float32 是**全精度**到达设备的。
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
/// ```text
/// 00 07 00 [mask] 01 [b5] [b6] <float32 x 4>
/// ```
///
/// * `mask` —— 覆盖其后 7 个数据字节的**还原掩码**：
///   `bit(3 + j)` 表示第 `j` 个值字节的 bit7 被 [`encode_param_value`] 剥掉了，
///   设备收到后把它还回去。
///   `bit0..2` 对应恒定的 `01`、`b5`、`b6`，因此恒为 0。
/// * `b5` —— 槽位 / 效果实例。
/// * `b6` —— 效果内的**参数索引**，不是块内的平铺下标。
///
/// 设备把存入的值四舍五入到 **0.001 网格**，更高精度发不过去。
/// `b6` 超出该模块设备存储范围时，
/// 设备回一条 5 字节错误帧 `00 03 00 00 00` 并**保持原值不变**；
/// 见 [`PARAM_WRITE_REJECTED`]。
pub fn param_write(b5: u8, b6: u8, value: f32) -> (u8, Vec<u8>) {
    let mut p = vec![0x00, sub::PARAM_WRITE, 0x00, param_value_mask(value), 0x01, b5, b6];
    p.extend_from_slice(&encode_param_value(value));
    (msg_type::DATA, p)
}

/// 参数写入被拒绝时设备回的 5 字节载荷：`b6` 超出该模块的设备存储范围。
///
/// 设备不会报错或纠正——被拒绝时原值保持不变。
/// 同形状的 `00 01 00 00 00` 出现在载荷含 `>= 0x80` 的数据字节或长度不对时（帧解析失败）；
/// 正常写入则回一条 13 字节的 mask 回显。
pub const PARAM_WRITE_REJECTED: [u8; 5] = [0x00, 0x03, 0x00, 0x00, 0x00];

// ─── 效果链序（`chainOrder`） ────────

/// 效果链中的效果数量。
pub const CHAIN_LEN: usize = 8;

/// 链角色表：`index -> (cli 名称, 显示名称)`。
///
/// **此顺序不是 CC 块的顺序**：标准 MIDI CC 把各块列为 `FX1 FX2 AMP CAB MOD DELAY
/// REVERB EQ`（CC25 = Mod，CC26 = Delay），而链中 `DEL` 排在 `MOD` *之前*。
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

/// 某槽位当前 `variant` 字节对应的类型名。
///
/// 快照每条记录的第二个字节（渲染成 `type=N`）就是这个 variant，
/// 设备用它决定该槽位套用哪套参数表。
/// 名字来自 [`crate::params_data::EFFECT_TYPES`]（压缩器、Room、EQ8 …）。
///
/// **amp / cab 不在此列**：它们的 `variant` 恒为 0（写任何 `model` 快照都不变），
/// 型号在别处查。未知 ID 同样返回 `None`；两种情况下调用方都只保留原数字。
///
/// 未知 ID 同样返回 `None`：调用方应保留原数字，而不是猜一个。
pub fn variant_name(slot: u8, variant: u8) -> Option<&'static str> {
    let block = CHAIN_ROLES.get(slot as usize)?.0;
    crate::params_data::effect_type_name(block, variant)
}

/// 某槽位当前 `variant` 在设备上显示的类型编号（渲染成 `type=N` 的那个 `N`）。
///
/// 多数块它就是 `variant + 1`；**reverb 不同** —— 手册 REV 那栏是产品顺序，
/// 与存储用的 `variant` 对末三项错开（variant 7 = spring，是设备上的第 5 个类型）。
/// 委托给 [`crate::params_data::type_number`]，不在渲染层写死 `+1`。
///
/// **amp / cab 返回 `None`**：它们的 `variant` 恒为 0，`variant + 1` 得到的
/// `type=1` 是个凭空来的编号——设备上 AMP 走 30 项 amp 型号表、CAB 走 30 项 IR 表，
/// 当前型号都不在 `0x63` 快照里。
pub fn type_number(slot: u8, variant: u8) -> Option<u8> {
    let (block, _) = CHAIN_ROLES.get(slot as usize)?;
    crate::params_data::type_number(block, variant)
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
/// 该字段在孤立状态下天然有歧义（含两个 `0x00`），索引必须取对。
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
/// 名字（≤ 8 个可打印 ASCII 字符）先填充到 8 字节，再在下标 6 处插入一个 `0x00`——
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

/// `0x66` sub `0x02`：设置任意设置页字段（字段 id 见 [`crate::setting`]）。
///
/// 与 `0x6D` 的 [`set_field`] 同形但走 `0x66` 设置通道。
pub fn set_setting(setting_field: u8, value: u8) -> (u8, Vec<u8>) {
    (msg_type::USB_VOLUME, vec![0x00, 0x02, 0x00, 0x00, setting_field, value])
}

/// `0x66` sub `0x02`：设置输入增益（dB，可为负）。
///
/// 负数在 byte3 携带符号标志 `0x02`，值字节为 7-bit 补码（-1 → `0x7F`、
/// -5 → `0x7B`）。0 与正数标志为 `0x00`。
pub fn set_input_gain(db: i8) -> (u8, Vec<u8>) {
    if db < 0 {
        (
            msg_type::USB_VOLUME,
            vec![0x00, 0x02, 0x00, 0x02, crate::setting::INPUT_GAIN, (db as u8) & 0x7F],
        )
    } else {
        (
            msg_type::USB_VOLUME,
            vec![0x00, 0x02, 0x00, 0x00, crate::setting::INPUT_GAIN, db as u8],
        )
    }
}

/// `0x65`：查询设置快照（载荷 `00 00 00`；设备每 ~250 ms 也主动广播一份）。
pub fn query_settings() -> (u8, Vec<u8>) {
    (msg_type::SETTINGS, vec![0x00, 0x00, 0x00])
}

/// `0x6D` SET_FIELD：设置预设音量（0-100）。
///
/// 载荷 `00 02 00 00 04 <val>`，值 0x00–0x53。
/// 设备每写入一步就回一条 `0x01` 事件回显该值。
/// 当前值可由 [`decode_snapshot_state`] 的 `preset_volume` 读回。
pub fn set_preset_volume(percent: u8) -> (u8, Vec<u8>) {
    set_field(field::PRESET_VOLUME, percent)
}

/// `0x63`：请求当前预设快照。
///
/// 载荷为 `00 00 00`。设备在应答后总会紧跟一段元数据尾（`da 00 0e ...`）；
/// 见 [`crate::tail`]。
pub fn request_snapshot() -> (u8, Vec<u8>) {
    (msg_type::SNAPSHOT, vec![0x00, 0x00, 0x00])
}

/// `0x40`：查询从 `start_slot` 开始的 `count` 个槽位的预设名称。
///
/// 线上布局：`00 02 00 00 [start_slot] [count]`。
/// `start_slot` 按组对齐（`0x00`、`0x08`、`0x10` …），`count` 是本组槽位数。
pub fn query_preset_list(start_slot: u8, count: u8) -> (u8, Vec<u8>) {
    (msg_type::PRESET_LIST, vec![0x00, 0x02, 0x00, 0x00, start_slot, count])
}

/// `0x68`：连通性 ping。
pub fn ping() -> (u8, Vec<u8>) {
    (msg_type::PING, vec![0x00, 0x00, 0x00])
}

/// `0x36`：查询单个音箱 / 驱动槽位的模型名称。
///
/// ```text
/// 00 01 00 00 [index]
/// ```
/// 设备以携带模型名称的 `0x36` 帧应答；见 [`parse_model_name`]。
/// `index` 是 0 起始的固件槽位（音箱 + FX2 驱动踏板共 0-37）。
/// 箱体 / IR 槽位（0-29）使用不同的消息，本查询不枚举它们。
pub fn model_query(index: u8) -> (u8, Vec<u8>) {
    (msg_type::MODEL_LIST, vec![0x00, 0x01, 0x00, 0x00, index])
}

// ─── 解析器 ───────────────────────────

/// `0x06`「effect data」—— **语义未定，不提供解析器**。
///
/// 设备切换预设时主动发 3 帧（`dir=Event`，`seq` 恒为 `0x0062`，既不是事务的
/// `seq` 回显也不是心跳 `0x68`），三帧与预设无关、来回切预设也不变：
///
/// ```text
/// [9]  00 01 00 00 00 00 00 3f
/// [10] 00 01 01 00 00 00 00 3f
/// [11] 00 01 02 03 4d 4c 4c 3f
/// ```
///
/// 已确定：载荷恒为 8 字节；`00 01` 前缀；`[2]` 递增 0/1/2；末尾 4 字节是 f32。
/// **未确定：`[3]`**。它不是参数个数（帧 3 的 `3 × 4` 超出剩余的 4 字节）。
/// 语义定下来之前渲染层只保留 hexdump，不给它编字段名。
///
/// 另一条由此确定的结论：**设备不在 MIDI 上报当前 amp / cab 型号**，
/// 型号只存在于 App 侧的预设 manifest（`ampSlot` / `irSlot`），App 走 USB 私有协议读取。
pub const EFFECT_DATA_NOTE: &str = "0x06 effect data: 8 bytes, constant across presets; [3] unknown";

#[cfg(test)]
mod effect_data_tests {
    /// 三帧的真实字节，留作回归基准。
    #[test]
    fn effect_data_is_eight_bytes_with_unknown_third_byte() {
        let frames: [&[u8]; 3] = [
            &[0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3f],
            &[0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x3f],
            &[0x00, 0x01, 0x02, 0x03, 0x4d, 0x4c, 0x4c, 0x3f],
        ];
        // 三帧都是 8 字节，`00 01` 前缀，`[2]` 递增 0/1/2。
        for (i, f) in frames.iter().enumerate() {
            assert_eq!(f.len(), 8, "帧 {i} 长度应为 8");
            assert_eq!(&f[..2], &[0x00, 0x01], "帧 {i} 前缀");
            assert_eq!(f[2], i as u8, "帧 {i} 的 [2]");
        }
        // 旧解析器把 `[3]` 当参数个数：帧 3 要 12 字节 body，实际只剩 4 ——
        // 装不下就是它不是 count 的直接反证。帧 1/2 的 `[3]=0` 会被算成 0 个
        // 参数，于是末尾那 4 字节（0.5 / 0.5）被无声丢弃。
        assert_eq!(frames[2][3] as usize * 4, 12);
        assert!(frames[2].len() - 4 < 12, "[3]=3 按 count 解释装不下");
        assert_eq!(frames[0][3], 0, "[3]=0 会把那 4 字节算成 0 个参数而丢弃");
    }
}

/// 来自 `0x40` 应答的带名称预设列表条目。
#[derive(Debug, Clone, PartialEq)]
pub struct PresetEntry {
    pub slot: u8,
    pub named: bool,
    pub name: String,
    /// 本条记录**槽位索引字节**在载荷中的偏移。
    ///
    /// 渲染层用它把 `slots (N):` 里的每一行与上方 hexdump 原位对齐，
    /// 与 [`SnapshotRecord::stream_offset`] 同一用途。
    /// `0x40` 不需要解包，这里记的是线上载荷的偏移。
    pub offset: usize,
    /// 标记字节（全部 `< 0x20`）所在区间 `[offset+1, marks_end)`。
    pub marks: (usize, usize),
    /// 内嵌 `0x00` 在名字区内的位置——它们不计入 8 个字符。
    ///
    /// ASCII 列里的 `.` 因此有两种可能：真正的不可打印字符，或这里的一颗 NUL。
    pub name_nuls: Vec<usize>,
    /// 记录尾部的填充字节位置（若有），位于名字之后。
    pub filler: Option<usize>,
}

/// 存储的预设名称的字符数。名称以空格填充到此宽度，并且以在分片之间插入 `0x00` 字节的方式存储。
pub const PRESET_NAME_CHARS: usize = 8;

/// 解析 `0x40` 预设列表应答载荷。
///
/// ```text
/// [0..7]  header: 00 00 <10*COUNT+2> 00 00 BASE COUNT
///         （第三个字节随分组大小变化，不是常量）
/// then COUNT records:
///   [slot: 1 byte, must equal BASE + k]
///   [1+ bytes < 0x20   -- the first one is 0x01 when the slot holds a name]
///   [name: exactly 8 printable characters]
/// ```
///
/// 名称以 **NUL 分隔的分片**存储：`0x00` 被跳过且不计入 8 个字符，
/// 所以记录长度会在 11 / 12 之间变化，尽管名称都是 8 字符宽。
/// 8 个字符之后、下一条记录的槽位索引之前，至多还有一个 `< 0x20` 字节。
///
/// 槽位 41..63 携带设备占位名称 `preset`。
/// 每条记录都交叉校验，任何错位都返回 `None`。
///
/// [`PresetEntry::named`] 取自槽位索引之后的前导字节是否不全为零。
/// 那 1–2 个字节的确切语义尚未解释清楚，
/// 它应读作「设备将此槽位标记为持有名称」。
pub fn parse_preset_list(payload: &[u8]) -> Option<Vec<PresetEntry>> {
    if payload.len() < 8 {
        return None;
    }
    let base = payload[5];
    let count = payload[6] as usize;
    if count == 0 || count > 64 {
        return None;
    }
    // `payload[2]` = `10 * count + 2` —— 头部 2 字节 + 每条 10 字节。
    // 它随分组大小变化，不是常量。
    if payload[2] as usize != 10 * count + 2 {
        return None;
    }

    let mut out = Vec::with_capacity(count);
    let mut i = 7usize;
    for k in 0..count {
        let slot = base.wrapping_add(k as u8);
        let offset = i;
        // 1. 槽位索引，与预期位置交叉校验。
        if i >= payload.len() || payload[i] != slot {
            return None;
        }
        i += 1;

        // 2. 一个或多个前导字节 < 0x20；`named` = 不全为零。
        if i >= payload.len() || payload[i] >= 0x20 {
            return None;
        }
        let marks_at = i;
        let mut named = false;
        while i < payload.len() && payload[i] < 0x20 {
            named |= payload[i] != 0x00;
            i += 1;
        }
        let marks = (marks_at, i);

        // 3. 恰好 PRESET_NAME_CHARS 个可打印字符，跳过 NUL。
        let mut name = String::with_capacity(PRESET_NAME_CHARS);
        let mut name_nuls = Vec::new();
        while i < payload.len() && name.len() < PRESET_NAME_CHARS {
            let b = payload[i];
            if b == 0x00 {
                name_nuls.push(i);
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

        // 4. 至多一个尾随字节 < 0x20，但绝不可能是下一个槽位索引。
        //    （被标记的槽位可能持有 < 0x20 的值，所以应与预期的下一个
        //    索引比较，而不是与阈值比较。）
        let next_slot = base.wrapping_add(k as u8).wrapping_add(1);
        let mut filler = None;
        if i < payload.len() && payload[i] < 0x20 && payload[i] != next_slot {
            filler = Some(i);
            i += 1;
        }
        out.push(PresetEntry {
            slot,
            named,
            name: name.trim_end().to_string(),
            offset,
            marks,
            name_nuls,
            filler,
        });
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
/// 走 BLE 的 App 快照尾部形态不同且链序恒等，填充下标在那里不可观测。
pub const CHAIN_SNAPSHOT_FILLER: usize = 7;

/// 从 `0x63` 快照载荷尾部解析 `chainOrder`。
///
/// 链字段位于载荷最末：
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
/// 载荷从 `[3]` 起（含）是**一条连续打包流**：每 8 个线上字节为一组，
/// 组首字节是掩码（bit *j* 对应组内第 *j* 个数据字节的 bit7），其余 7 字节是数据。
/// 记录头、活动预设索引、音量、链序计数全部在流内，仅 `[0..3)`（`00 00 <X>`）留在流外。
///
/// 掩码落在 `[3 + 8k]`，该位置的载荷字节被吞掉、不进入数据流——
/// 快照尾部的链序分隔/填充 `0x00` 恰好落在某些掩码位上，
/// 因此解包后的尾部比原始字节短；那些被吞掉的字节值恒为 `0x00`。
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
    /// 本记录**记录头**（`[enabled][variant][count]`）在解包后数据流中的起始偏移。
    ///
    /// 参数区紧跟其后，故参数占 `[stream_offset+3 .. +3+4*count)`。
    /// 渲染层用它把可读值与字节流逐字段对齐——偏移必须与解码同源，
    /// 否则解码顺序一改，展示就会静默指错字节。
    pub stream_offset: usize,
}

/// `0x63` 快照的完整结构化解码。
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotState {
    /// 活动预设索引（0 起始；与 `preset set` 往返一致）。
    pub preset_index: u8,
    /// 预设音量（0–100）。
    pub preset_volume: u8,
    /// `presetIndex` 在解包后数据流中的偏移（恒为 2，前两个字节是标记）。
    pub preset_index_offset: usize,
    /// `presetVolume` 在解包后数据流中的偏移（恒为 3）。
    pub preset_volume_offset: usize,
    /// 链计数 `0x08` 在解包后数据流中的偏移。
    pub chain_count_offset: usize,
    /// 解包后数据流的总长度。
    pub stream_len: usize,
    /// 恰 8 条记录（**按角色序**：下标 i 就是 `CHAIN_ROLES[i]`，即 `b5` 槽位）。
    pub records: Vec<SnapshotRecord>,
    /// 链序（`chainOrder`）：8 个角色索引的排列，按从左到右的链位排列。
    ///
    /// 数据流尾部通常只剩裸的 8 字节链序（线上分隔/填充 `0x00` 已被掩码位吞掉）；
    /// 解不出来时为 `None`，此时可退回 [`parse_snapshot_chain_order`] 处理
    /// 那些计数锚仍落在原位的旧形态。
    pub chain: Option<Vec<u8>>,
}

/// `0..CHAIN_LEN` 的一个排列？
fn is_chain_permutation(v: &[u8]) -> bool {
    let mut seen = [false; CHAIN_LEN];
    v.iter().all(|&b| {
        (b as usize) < CHAIN_LEN && !std::mem::replace(&mut seen[b as usize], true)
    })
}

/// 从解包后数据流的尾部（链计数 `0x08` **之后**）解出链序。
///
/// 只接受**恰好 `CHAIN_LEN` 字节**的裸排列。线上原始尾部的分隔/填充 `0x00`
/// 会落在掩码位上被吞掉，因此数据流里看到的正是这 8 字节。
///
/// 不接受 9 字节（sep/填充未被吞）的形态：链序里必然含 `0`（FX1），
/// 分隔/填充又是 `0`，两个 `0` 各去掉一次都能得到一个合法排列且互不相同，
/// 没有计数锚点就无法判定。那种形态请退回 [`parse_snapshot_chain_order`]。
fn chain_order_from_stream(tail: &[u8]) -> Option<Vec<u8>> {
    if tail.len() == CHAIN_LEN && is_chain_permutation(tail) {
        return Some(tail.to_vec());
    }
    None
}

/// 完整解码 `0x63` 快照载荷（USB 形态，无 BLE 元数据尾）。
///
/// 数据流结构（掩码还原后）：
///
/// ```text
/// [00][03][presetIndex][presetVolume]
/// × 8: [enabled][variant][count] + float32 × count
/// [08] + 链序（sep/填充 0x00 通常已被掩码位置吞掉 → 恰 8 字节）
/// ```
///
/// 校验：前导标记 `[00][03]`、记录数恰为 8、每条 `count ≤ 16`、
/// float 不得为 NaN/Inf（即 `(bits & 0x7FFFFFFF) <= 0x7F7FFFFF`）、
/// 记录后紧跟链计数 `0x08`。
///
/// 记录下标 == `CHAIN_ROLES` 下标 == `b5` 槽位。
///
/// 注意：**不要**用 [`parse_snapshot_chain_order`] 做前置校验——
/// 它锚定原始尾部的 `count@len-11` 布局，
/// 那只对掩码相位恰好把 sep/填充留在原位的特定长度成立；
/// 其他长度的原始尾部字节因掩码吞位而整体移位。
/// 链序由数据流尾部解出（见 [`chain_order_from_stream`]），
/// [`parse_snapshot_chain_order`] 仅作旧形态的退回。
pub fn decode_snapshot_state(payload: &[u8]) -> Option<SnapshotState> {
    let data = unpack_snapshot_stream(payload)?;
    // 游标放进 `Cell`：闭包 `take` 要用它推进位置，而调用方也要读同一个值来记录字段偏移。
    // 用 `Cell` 共享，比从切片指针反推偏移（`ptr` 相减）干净。
    let p = std::cell::Cell::new(0usize);
    let take = |k: usize| -> Option<&[u8]> {
        let at = p.get();
        let r = data.get(at..at + k)?;
        p.set(at + k);
        Some(r)
    };
    let hdr = take(4)?;
    if hdr[0] != 0x00 || hdr[1] != 0x03 {
        return None;
    }
    // 偏移取自同一个游标：解码读了多少字节，偏移就是多少，不会漂。
    let preset_index_offset = 2;
    let preset_volume_offset = 3;
    let preset_index = hdr[2];
    let preset_volume = hdr[3];
    let mut records = Vec::with_capacity(CHAIN_LEN);
    for _ in 0..CHAIN_LEN {
        let stream_offset = p.get();
        let h = take(3)?;
        let (enabled, variant, count) = (h[0], h[1], h[2]);
        if count > 16 {
            return None;
        }
        let raw = take(count as usize * 4)?;
        let mut params = Vec::with_capacity(count as usize);
        for c in raw.as_chunks::<4>().0 {
            let bits = u32::from_le_bytes(*c);
            // 拒绝 NaN/Inf。
            if bits & 0x7FFF_FFFF > 0x7F7F_FFFF {
                return None;
            }
            params.push(f32::from_bits(bits));
        }
        records.push(SnapshotRecord {
            enabled,
            variant,
            params,
            stream_offset,
        });
    }
    // 链计数标记（sep/填充可能被吞，但计数 0x08 始终在数据流内）。
    let chain_count_offset = p.get();
    if take(1)?[0] != CHAIN_LEN as u8 {
        return None;
    }
    // 其余就是链序本身（不强求：解不出只影响展示，不影响记录解码）。
    let chain = chain_order_from_stream(&data[p.get()..]);
    Some(SnapshotState {
        preset_index,
        preset_volume,
        preset_index_offset,
        preset_volume_offset,
        chain_count_offset,
        stream_len: data.len(),
        records,
        chain,
    })
}

/// 设备设置页快照（`0x65`，13 B 载荷）。
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceSettings {
    /// 无线开关。
    pub wireless: bool,
    /// 回放（loopback）开关：外部音频与吉他信号混合回录。
    pub loopback: bool,
    /// 输入增益（dB，可为负）。
    pub input_gain_db: i8,
    /// USB 音量（0-100）。
    pub usb_volume: u8,
    /// 蓝牙音量（0-100）。
    pub bt_volume: u8,
    /// MIDI 通道（0-16；0 = 全通道。写入 17 会被设备钳到 16 —— 与手册的
    /// `MIDI Channel 0~16` 一致）。
    pub midi_channel: u8,
}

/// MIDI 通道值的显示标签。
///
/// 0 = 全通道（手册：`通道设为 0 时，开启全通道模式`），1-16 = 对应通道。
pub fn midi_channel_label(v: u8) -> String {
    if v == 0 { "omni".into() } else { format!("ch {v}") }
}

/// 解码 `0x65` 设置快照载荷（13 B）。
///
/// 字段映射：`[3]` = 增益符号标志（负 = `0x10`）、`[6]` = 无线、
/// `[7]` = 回放、`[8]` = 输入增益（7-bit 补码，负数 ≥ `0x40`）、`[9]` = USB 音量、
/// `[10]` = 蓝牙音量、`[12]` = MIDI 通道。`[1][4][5][11]` 含义未定。
pub fn decode_settings(payload: &[u8]) -> Option<DeviceSettings> {
    if payload.len() < 13 || payload[0] != 0x00 || payload[2] != 0x07 {
        return None;
    }
    let raw_gain = payload[8];
    let input_gain_db: i8 = if raw_gain >= 0x40 {
        // 7-bit 补码负数：-1 = 0x7F …（符号标志 [3]=0x10 一并核对）。
        (raw_gain as i16 - 128) as i8
    } else {
        raw_gain as i8
    };
    Some(DeviceSettings {
        wireless: payload[6] != 0x00,
        loopback: payload[7] != 0x00,
        input_gain_db,
        usb_volume: payload[9],
        bt_volume: payload[10],
        midi_channel: payload[12],
    })
}

/// 从 `0x43` 预设写入载荷中解析 `chainOrder`。
///
/// 两种布局：
/// * **234 字节预设帧**：链字段位于 `[33..42)`，其前是 `[31]` 处的字段 id `0x03` 和
///   `[32]` 处的计数 `0x08`。9 字节字段是在索引 [`CHAIN_PRESET_FILLER`] 处插入 `0x00`
///   后的 8 元素链序。
/// * **旧的 356 字节首消息**：字段 id 在 `[30]`，计数在 `[31]`，链序从 `[32]` 开始，无
///   填充字节。为兼容旧格式保留。
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
    let order = payload[32..32 + count].to_vec();
    // 必须是 0..7 的完整排列——8 个槽位各出现一次。
    // `is_chain_permutation` 单独用时不看长度（`chain_order_from_stream` 靠
    // 调用方保证），所以这里同时要求长度正好是 CHAIN_LEN。
    //
    // 严格性是必需的：编码后的链字段天然含两个 `0x00`（填充字节与合法的 FX1
    // 元素），宽松的校验会让 `0x40` 预设列表也被读出「链序」——它的第 30 字节
    // 恰好是 `0x03`（EQ 的槽位索引），第 32 字节是 ASCII `E`。
    if order.len() != CHAIN_LEN || !is_chain_permutation(&order) {
        return None;
    }
    Some(order)
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
    /// 这标记的是**当前已加载 / 正在使用**的槽位，**不是**「用户模型」标志——
    /// 设备内置的模型同样会被置位。
    /// 要识别用户或自定义模型，请把 [`ModelName::name`] 与 [`crate::models_data`]
    /// 的表比较，与本标志无关。
    pub active: bool,
    /// 名称标记 `1a 1a 00 00` 在载荷中的偏移，名称紧随其后。
    ///
    /// 渲染层用它把名字与上方 hexdump 原位对齐（同 [`PresetEntry::offset`]），
    /// 这样 `0x36` 与 `0x40` 的显示方式一致。
    pub name_offset: usize,
}

/// 解析 `0x36` 模型列表应答载荷。
///
/// ```text
/// 00 00 1e 10 00 [index] 01 [flag] 52 2f 00 [len] 00 [hash x4] 1a 1a 00 00 [name...]
/// ```
/// * `index` 位于第 5 字节。
/// * `flag` 位于第 7 字节 —— 见 [`ModelName::active`]。
/// * 模型名称紧随 `1a 1a 00 00` 标记之后，以 **NUL 分隔的可打印分片**存储
///   （如 `BogXTC` + `1` -> `BogXTC1`）。不可打印字节和分隔用的 NUL 会被丢弃。
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
    Some(ModelName { index, name, active, name_offset: m + 4 })
}

#[cfg(test)]
mod tests {
    use super::*;

    // 槽位 0（BogXTC1）和槽位 29（RESONA NAM 导入后的 NDSP Archetype:）
    // 的原始 `0x36` RX 载荷。
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
    // 槽位 9 持有设备内置的模型 `Mar8001`，第 7 字节却是 0x01——
    // 证明该字节并非「用户模型」标志。
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
    fn set_preset_volume_matches_capture() {
        // 写入 83 时的载荷。
        let (k, p) = set_preset_volume(83);
        assert_eq!(k, msg_type::DATA);
        assert_eq!(p, vec![0x00, sub::SET_FIELD, 0x00, 0x00, field::PRESET_VOLUME, 83]);
    }

    #[test]
    fn setting_setters_match_captures() {
        // 启用无线。
        let (_k, p) = set_setting(crate::setting::WIRELESS, 1);
        assert_eq!(_k, msg_type::USB_VOLUME);
        assert_eq!(p, vec![0x00, 0x02, 0x00, 0x00, crate::setting::WIRELESS, 1]);
        // 切到 Ch5。
        let (_k, p) = set_setting(crate::setting::INPUT_CHANNEL, 5);
        assert_eq!(_k, msg_type::USB_VOLUME);
        assert_eq!(p, vec![0x00, 0x02, 0x00, 0x00, crate::setting::INPUT_CHANNEL, 5]);
        // 蓝牙音量置 0。
        let (_k, p) = set_setting(crate::setting::BT_VOLUME, 0);
        assert_eq!(p, vec![0x00, 0x02, 0x00, 0x00, crate::setting::BT_VOLUME, 0]);
    }

    #[test]
    fn input_gain_encoding_is_7bit_complement_with_sign_flag() {
        // -1 → 00 02 00 02 04 7f；-5 → …7b；0 → 00…00。
        let (k, p) = set_input_gain(-1);
        assert_eq!(k, msg_type::USB_VOLUME);
        assert_eq!(p, vec![0x00, 0x02, 0x00, 0x02, crate::setting::INPUT_GAIN, 0x7f]);
        assert_eq!(set_input_gain(-5).1, vec![0x00, 0x02, 0x00, 0x02, crate::setting::INPUT_GAIN, 0x7b]);
        assert_eq!(set_input_gain(0).1, vec![0x00, 0x02, 0x00, 0x00, crate::setting::INPUT_GAIN, 0]);
    }

    #[test]
    fn decode_settings_matches_capture_snapshots() {
        // 增益 -1 时的快照（USB=85、BT=100）。
        let s = decode_settings(&[
            0x00, 0x00, 0x07, 0x10, 0x00, 0x01, 0x01, 0x01, 0x7f, 0x55, 0x64, 0x00, 0x00,
        ])
        .expect("decode");
        assert_eq!(
            s,
            DeviceSettings {
                wireless: true,
                loopback: true,
                input_gain_db: -1,
                usb_volume: 85,
                bt_volume: 100,
                midi_channel: 0
            }
        );
        // 关闭无线后：[6] 00。
        let s = decode_settings(&[
            0x00, 0x00, 0x07, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x55, 0x64, 0x00, 0x00,
        ])
        .expect("decode");
        assert!(!s.wireless);
        assert_eq!(s.input_gain_db, 0);
        // 输入通道 Ch5：[12] 05。
        let s = decode_settings(&[
            0x00, 0x00, 0x07, 0x00, 0x00, 0x01, 0x01, 0x01, 0x00, 0x55, 0x64, 0x00, 0x05,
        ])
        .expect("decode");
        assert_eq!(s.midi_channel, 5);
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

    /// `name_offset` 必须落在名称标记之后、真正的名字首字节上。
    ///
    /// 渲染层拿它把名字与上方 hexdump 原位对齐；标记本身是 `1a 1a 00 00`，
    /// 不该被算进名字区间。
    #[test]
    fn model_name_offset_points_past_the_marker() {
        for payload in [SLOT0, SLOT29] {
            let mn = parse_model_name(payload).expect("parse");
            assert_eq!(
                &payload[mn.name_offset..mn.name_offset + 4],
                &mn.name.as_bytes()[..4],
                "偏移应指向名字本身"
            );
            // 标记紧挨在偏移之前。
            assert_eq!(&payload[mn.name_offset - 4..mn.name_offset - 2], &[0x1A, 0x1A]);
        }
    }

    /// 设备内置的模型同样会置位该标志，调用方绝不能把它当作「用户模型」。
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

    // 一条真实的 `0x40` PRESET_LIST 应答。
    /// 98 字节的 `0x40` 应答。
    const PRESET_LIST_MID: &[u8] = &[
        0x00, 0x00, 0x52, 0x00, 0x00, 0x01, 0x08, 0x01, 0x01, 0x41, 0x6d, 0x00, 0x62, 0x69,
        0x65, 0x6e, 0x74, 0x20, 0x02, 0x00, 0x01, 0x52, 0x68, 0x79, 0x4f, 0x44, 0x20, 0x00,
        0x20, 0x20, 0x03, 0x01, 0x45, 0x70, 0x69, 0x00, 0x63, 0x53, 0x6f, 0x6c, 0x6f, 0x04,
        0x01, 0x00, 0x43, 0x6f, 0x6c, 0x64, 0x43, 0x6c, 0x6e, 0x00, 0x20, 0x05, 0x01, 0x50,
        0x69, 0x65, 0x72, 0x00, 0x63, 0x69, 0x6e, 0x67, 0x06, 0x01, 0x54, 0x00, 0x69, 0x67,
        0x68, 0x74, 0x52, 0x68, 0x79, 0x00, 0x07, 0x01, 0x48, 0x65, 0x61, 0x76, 0x79, 0x00,
        0x4d, 0x75, 0x74, 0x08, 0x01, 0x57, 0x61, 0x00, 0x72, 0x6d, 0x43, 0x6c, 0x6e, 0x20,
    ];

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
        // `payload[2]` 必须是 `10 * count + 2`。
        let mut bad = PRESET_LIST_BASE32.to_vec();
        bad[2] = 0x52; // = 10*8+2，改成别的值
        bad[2] = 0x53;
        assert!(parse_preset_list(&bad).is_none());
    }

    /// 分组大小不是 8 时也要能解析。
    #[test]
    fn preset_list_parses_groups_of_any_size() {
        for count in 1..=8usize {
            let mut p = vec![0x00, 0x00, (10 * count + 2) as u8, 0x00, 0x00, 0x00, count as u8];
            for k in 0..count {
                p.push(k as u8); // 槽位索引 = BASE + k
                p.push(0x01); // 标记字节：< 0x20 且非零 -> named
                p.extend_from_slice(format!("n{k}      ").as_bytes());
            }
            let got = parse_preset_list(&p).expect("grouped parse");
            assert_eq!(got.len(), count);
            for (k, e) in got.iter().enumerate() {
                assert_eq!(e.slot, k as u8);
                assert_eq!(e.name, format!("n{k}"));
                // 每条 10 字节：槽位 1 + 标记 1 + 名字 8（含跳过的 NUL 时更长，
                // 但上面构造的名字无 NUL，正好 10）。
                assert_eq!(e.offset, 7 + k * 10, "第 {k} 条的偏移应逐条推进");
            }
        }
    }

    // ─── 链序 ────────────────────────
    // 用户把 DEL 移到链位 1、把 EQ 移到链位 2 时设备发出的帧。
    // 按角色表（DEL == 4，EQ == 7），链必须解出为 [4,0,1,2,3,5,6,7]，
    // 再解出 [4,7,0,1,2,3,5,6]。

    /// `0x6D` 写入载荷。
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
        // `ClnArp` 预设帧：恒等链序 [0,1,2,3,4,5,6,7]，0x00 填充位于索引 2。
        let mut frame = vec![0u8; 234];
        frame[31] = 0x03;
        frame[32] = 0x08;
        frame[33..42].copy_from_slice(&[0x00, 0x01, 0x00, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07]);
        assert_eq!(parse_chain_order(&frame).unwrap(), vec![0, 1, 2, 3, 4, 5, 6, 7]);

        // `ColdCln`：非恒等链序 [0,1,2,3,5,4,6,7]。
        let mut f2 = vec![0u8; 234];
        f2[31] = 0x03;
        f2[32] = 0x08;
        f2[33..42].copy_from_slice(&[0x00, 0x01, 0x00, 0x02, 0x03, 0x05, 0x04, 0x06, 0x07]);
        assert_eq!(parse_chain_order(&f2).unwrap(), vec![0, 1, 2, 3, 5, 4, 6, 7]);
    }

    /// 记录内部各段的偏移必须能对上载荷本身。
    ///
    /// 渲染层逐段标注槽位 / 标记 / 名字 / 填充，任何一段的偏移算错都会指到
    /// 别人的字节上，而且不会有任何报错。
    #[test]
    fn preset_entry_segments_locate_their_own_bytes() {
        for payload in [PRESET_LIST_BASE32, PRESET_LIST_MID, PRESET_LIST_BASE40] {
            let got = parse_preset_list(payload).expect("parse");
            for e in &got {
                // 槽位字节
                assert_eq!(payload[e.offset], e.slot, "槽位字节应等于 slot");
                // 标记段全为 < 0x20，且至少一个字节
                assert!(e.marks.1 > e.marks.0);
                assert!(
                    payload[e.marks.0..e.marks.1].iter().all(|&b| b < 0x20),
                    "标记段必须全是 < 0x20 的字节"
                );
                // named = 标记段不全为零
                assert_eq!(
                    e.named,
                    payload[e.marks.0..e.marks.1].iter().any(|&b| b != 0)
                );
                // 名字段去掉 NUL 后正好 8 个可打印字符，且与 e.name 一致
                // （e.name 已 trim 尾部空格，比较时补回去）。
                let text: String = payload[e.marks.1..]
                    .iter()
                    .filter(|&&b| b != 0)
                    .take(PRESET_NAME_CHARS)
                    .map(|&b| b as char)
                    .collect();
                assert_eq!(text.len(), PRESET_NAME_CHARS);
                assert_eq!(text, format!("{:<8}", e.name));
                // NUL 位置确实落在名字区内
                for n in &e.name_nuls {
                    assert!(*n >= e.marks.1 && *n < e.marks.1 + 16, "NUL 应在名字区内");
                    assert_eq!(payload[*n], 0x00);
                }
                // 填充字节（若有）也 < 0x20
                if let Some(f) = e.filler {
                    assert!(payload[f] < 0x20);
                }
            }
            // 各条记录互不重叠，且整体铺满载荷
            let mut cursor = 7;
            for e in &got {
                assert_eq!(e.offset, cursor, "记录应首尾相接");
                cursor = e.filler.map_or(e.marks.1 + 8 + e.name_nuls.len(), |f| f + 1);
            }
            assert_eq!(cursor, payload.len(), "记录应恰好铺满载荷");
        }
    }

    /// `0x40` 预设列表不得被读出链序。
    ///
    /// 它的第 30 字节恰好是 `0x03`（EQ 的槽位索引），第 32 字节是 ASCII
    /// `E`（`EpicSolo` 首字母）——旧路径只要求 `payload[30] == 0x03`，就把
    /// 这两个任意字节当成了一副链序，打印出 `chainOrder: [69]`。
    #[test]
    fn chain_order_rejects_a_preset_list_payload() {
        // 前提：该帧的第 30 字节确实等于链字段 id 0x03，因此旧路径会接受它。
        assert_eq!(PRESET_LIST_MID[30], 0x03);
        assert_eq!(PRESET_LIST_MID[31], 0x01, "被误读成链序长度");
        assert_eq!(PRESET_LIST_MID[32], 0x45, "是 'EpicSolo' 的首字母 E");
        assert_eq!(parse_chain_order(PRESET_LIST_MID), None, "预设列表不该被读成链序");
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
    // 一条 `0x63` 快照载荷的尾部。

    /// 恒等链序快照的末 12 字节（写入实验前的原始状态，
    /// 尾部字节完全一致）。
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
        // 分隔字节缺失（BLE 形态，末 9 字节的索引 7 不是 0x00）。
        let mut q = vec![0x00u8; 185];
        q.extend_from_slice(&[0x3f, 0x08, 0x00, 0x01, 0x02, 0x00, 0x03, 0x04, 0x05, 0x06, 0x07]);
        assert!(parse_snapshot_chain_order(&q).is_none());
    }

    // ─── 快照完整解码 ────────────────
    // 两份设备内置的预设快照。

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
    /// `slot_13.bin`（Psych）——非恒等链序 `[0,1,5,7,6,4,2,3]`，165 字节。
    /// 用于证明链序确实来自数据流尾部，而不是碰巧恒等。
    const SNAP_PSYCH: &str = "00000c0000030d55000002634d4c4c3e213072003e000403000000603f0000003f4d4c604c3f0000051a191c193f4d4c4c3d001000003f5c0f023f034375683f000002020a57233f295c0f703f0104034d4c4c483d7f6a3c3e0a5701233d010002713d040a3f2e47213f015007050e2d323d4d394c4c3e63251b3e36142e473e1a1919003e000004000000003f0000003f000000003f0000003f0800000105070604020003";

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

    #[test]
    fn snapshot_state_decodes_chain_from_the_data_stream() {
        // 恒等链序：三条 169B 快照（count@len-10，使旧的
        // `parse_snapshot_chain_order` 锚点失效）都以裸 8 字节结束。
        for hex in [SNAP_CLNARP, SNAP_AMBIENT] {
            let s = decode_snapshot_state(&unhex(hex)).expect("decode");
            assert_eq!(s.chain.as_deref(), Some(&[0, 1, 2, 3, 4, 5, 6, 7][..]));
        }
        // 非恒等：Psych 的 `[0,1,5,7,6,4,2,3]`，且与 manifest 的 chainOrder 一致。
        let s = decode_snapshot_state(&unhex(SNAP_PSYCH)).expect("Psych decode");
        assert_eq!(s.chain.as_deref(), Some(&[0, 1, 5, 7, 6, 4, 2, 3][..]));
        // 记录数/参数不受链序影响。
        assert_eq!(s.preset_index, 13);
        assert_eq!(s.records.len(), CHAIN_LEN);
    }

    /// `stream_offset` 必须能真的定位到字节，而不只是「看起来像个偏移」。
    ///
    /// 渲染层（`debug snapshot --decode`）拿这些偏移去切字节、显示每个字段的
    /// 原始字节；一旦偏移与实际布局错位，那张表就会指着错误的字节展示，
    /// 而且**不会报错**——所以在这里钉死。
    #[test]
    fn record_offsets_address_the_bytes_they_claim() {
        for hex in [SNAP_CLNARP, SNAP_AMBIENT, SNAP_PSYCH] {
            let p = unhex(hex);
            let s = decode_snapshot_state(&p).expect("decode");
            let data = unpack_snapshot_stream(&p).expect("unpack");
            assert_eq!(s.stream_len, data.len());

            // 前导 4 字节之后，8 条记录首尾相接、不重叠、不越界。
            let mut cursor = 4;
            for (i, r) in s.records.iter().enumerate() {
                assert_eq!(r.stream_offset, cursor, "record {i} 起始偏移不连续");
                // 记录头的第 3 字节就是参数个数 —— 偏移对了才会等于 params.len()。
                assert_eq!(
                    data[r.stream_offset + 2] as usize,
                    r.params.len(),
                    "record {i} 的 count 字节与参数个数不符"
                );
                cursor = r.stream_offset + 3 + r.params.len() * 4;
            }
            // 记录之后紧跟链计数，链序再之后到流尾。
            assert_eq!(s.chain_count_offset, cursor, "链计数偏移与记录尾部不接续");
            assert_eq!(data[s.chain_count_offset] as usize, CHAIN_LEN);
            if let Some(order) = &s.chain {
                let at = s.chain_count_offset + 1;
                assert_eq!(
                    &data[at..at + order.len()],
                    order.as_slice(),
                    "该偏移处的字节与解出的链序不符"
                );
                assert_eq!(at + order.len(), data.len(), "链序之后还有未解释的字节");
            }
        }
    }

    #[test]
    fn chain_from_stream_rejects_bad_tails() {
        let perm = [0u8, 1, 2, 3, 4, 5, 6, 7];
        assert_eq!(chain_order_from_stream(&perm), Some(perm.to_vec()));
        // 不是排列（越界 / 重复）。
        assert_eq!(chain_order_from_stream(&[0, 1, 2, 3, 4, 5, 6, 8]), None);
        assert_eq!(chain_order_from_stream(&[0, 1, 2, 3, 4, 5, 6, 6]), None);
        // 长度不对（9 字节：sep/填充 + 链序 —— 两个 0 各去一次都是合法排列，无法判定）。
        let mut nine = perm.to_vec();
        nine.insert(3, 0x00);
        assert_eq!(chain_order_from_stream(&nine), None);
        // 截断 / 带 BLE 元数据尾。
        assert_eq!(chain_order_from_stream(&perm[..7]), None);
        let mut long = perm.to_vec();
        long.extend_from_slice(&[0xAA, 0xBB]);
        assert_eq!(chain_order_from_stream(&long), None);
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
        // 00 07 00 [mask] 01 [b5] [b6] <float32>
        assert_eq!(&p[..3], &[0x00, 0x07, 0x00]);
        assert_eq!(&p[4..7], &[0x01, 0x00, 0x01]);
        // 0.25f32 = 0x3E800000 -> LE `00 00 80 3E`；v2 的 bit7 被剥掉 -> mask bit5。
        assert_eq!(p[3], 0x20);
        assert_eq!(&p[7..], &[0x00, 0x00, 0x00, 0x3E]);
    }

    #[test]
    fn param_write_carries_the_stripped_bits_in_the_mask() {
        // cab b5=3 b6=1 的两条编码。
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
        // 逐字节对照设备内置的预设名。
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
