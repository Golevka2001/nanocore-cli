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
    // 顺序：先查字符集，再查长度。
    // 设备只能显示键盘上打得出的半角字符；真机逐个试过字母、数字、空格，以及
    // `- _ . , ! ? + = * / : ; ' " ( ) [ ] { } < > @ # $ % ^ & | ~ \\`
    // 都可以，中文/全角标点/控制字符都写不进去。
    // 先查字符集，`中文名` 这类会直接报"字符不允许"，而不是被当成 9 字节说超长。
    if let Some(bad) = name.chars().find(|c| !matches!(c, ' '..='~')) {
        return Err(format!("preset name: {bad:?} is not allowed"));
    }
    let count = name.chars().count();
    if count == 0 {
        return Err("preset name is empty".to_string());
    }
    if count > 8 {
        return Err(format!("preset name: 8 characters max, got {count}"));
    }
    // 全是空格的话，设备读回来和空名字没区别。
    if name.chars().all(|c| c == ' ') {
        return Err("preset name is only spaces".to_string());
    }

    let bytes = name.as_bytes();
    let mut padded = [0x00u8; 8];
    padded[..bytes.len()].copy_from_slice(bytes);
    let mut out = [0x00u8; 9];
    out[..6].copy_from_slice(&padded[..6]);
    // 下标 6 固定插入 0x00，剩余字符跟在其后。
    out[7..9].copy_from_slice(&padded[6..8]);
    Ok(out)
}

/// 改名第一步：`0x6D` sub `0x01`，载荷 3 字节全 0。
///
/// 官方 App 改名时先发这一帧再发 [`set_name_field`]；设备对它回一个 10 字节的应答
/// （`00 00 05 08 00 00 04 7f 03 08`）。**去掉这一步改名不会生效。**
pub fn name_session_begin() -> (u8, Vec<u8>) {
    (msg_type::DATA, vec![0x00, sub::NAME_PREPARE, 0x00, 0x00, 0x00])
}

/// `0x6D` sub `0x0A`：以 9 字节名字区形态重命名**当前**预设。
///
/// 名字区编码见 [`encode_preset_name_field`]。设备应答会把 NUL 重排到别的位置
/// （出厂名表里 NUL 位置本来就每条不同：`Am\0bient`、`T\0ightRhy`、`Epi\0cSolo`），
/// 所以回显不必逐字节相同，按"跳过 NUL 收集可打印字符"比较即可。
pub fn set_name_field(field: &[u8; 9]) -> (u8, Vec<u8>) {
    let mut p = vec![0x00, sub::NAME, 0x00, 0x00, 0x08, 0x08];
    p.extend_from_slice(field);
    (msg_type::DATA, p)
}

/// 改名第三步：`0x46` 载荷 `00 01 00 00 0d` —— 提交。
///
/// 抓包线上是 `46 00 01 00 00 0d`：去掉真类型 `0x46` 后载荷为 `00 01 00 00 0d`。
/// 开头这个 `0x00` 少一个字节，设备就回 `01 7e 00 00 00`（status `0x7e` = 拒收）。
///
/// **这一步是成败关键。** 只发前两帧时设备会回显改名后的名字区，看上去收下了，
/// 但 `0x40` 名表纹丝不动；补上这一帧名字才真正落盘。
pub fn name_commit() -> (u8, Vec<u8>) {
    // 抓包原样：线上是 `46 00 01 00 00 0d`，去掉真类型 `0x46` 后载荷为 `00 01 00 00 0d`。
    // 开头这个 `0x00` 少一个字节，设备就回 `01 7e 00 00 00`（status 0x7e = 拒收）。
    (0x46, vec![0x00, 0x01, 0x00, 0x00, 0x0d])
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

/// 名表里每条名字区占的字节数。
///
/// 8 个字符位 + 1 个分隔位。名字不足 8 字符时设备用 NUL 补齐，但**补齐的位置不固定**，
/// 所以读的时候不能指望尾部对齐，只能按固定长度取可打印字符。
pub const NAME_AREA_BYTES: usize = 9;

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

        // 3. 名字区固定 9 字节，取其中的可打印字符，其余（NUL 及个别控制字节）
        //    一律当分隔符跳过。
        //
        //    设备存进去的字节并不总是干净的文本：改过名的槽位里会混进控制字节，
        //    例如改名成 `TestNm` 后名表里是 `54 10 65 73 74 4e 00 20 20`——第 2
        //    个字节是 `0x10`。遇到非可打印字节就整条判死，会让**整张名表**读不出来
        //    （一个槽位坏了，`preset list` 直接报错）。所以这里只按位置消耗固定
        //    长度，字符另收。
        let mut name = String::with_capacity(PRESET_NAME_CHARS);
        let mut name_nuls = Vec::new();
        for _ in 0..NAME_AREA_BYTES {
            if i >= payload.len() {
                return None;
            }
            let b = payload[i];
            if (0x20..=0x7E).contains(&b) {
                name.push(b as char);
            } else {
                name_nuls.push(i);
            }
            i += 1;
        }
        if name.is_empty() && !named {
            name.clear();
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
