//! `.npreset`（ZIP 容器）：manifest serde 模型 + 打包 / 解包。
//!
//! 格式：
//! ZIP 内含 `manifest.json` + 引用到的资产（`amp/slot_NN.ead` = 完整 12302 B、
//! `ir/slot_NN.bin` = 4096 B f32le）。manifest 字段 camelCase；
//! `schema = "nanocore.preset.bundle"`、`version = 1`、`deviceKind = "nanocore"`。
//! 资产条目的 `length`/`crc32` 指**剥壳负载**（12242 B / 4096 B）。

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const MANIFEST_SCHEMA: &str = "nanocore.preset.bundle";
pub const DEVICE_KIND: &str = "nanocore";
/// `fx2_slot` 的「无」取值。
pub const FX2_SLOT_NONE: u8 = 255;

/// manifest 顶层。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema: String,
    pub version: u32,
    pub device_kind: String,
    pub exported_at: String,
    /// 完整备份用 `allValid`；单条导出用 `single`。
    pub scope: String,
    pub presets: Vec<ManifestPreset>,
    #[serde(default)]
    pub amp_slots: Vec<AssetRef>,
    #[serde(default)]
    pub ir_slots: Vec<AssetRef>,
}

/// 一条预设条目。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestPreset {
    pub slot: u8,
    pub name: String,
    pub preset: PresetSpec,
}

/// 预设核心参数（与 `crate::preset43::PresetWrite` 同构，另带透传字段）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetSpec {
    /// 恒 7（0x43 逻辑流 const7）。
    pub version: u8,
    pub name: String,
    pub chain_order: [u8; 8],
    pub slots: Vec<SlotSpec>,
    pub amp_slot: u8,
    pub ir_slot: u8,
    /// fx2 踏板槽引用（30-37，`FX2_SLOT_NONE` = 无）。
    pub fx2_slot: u8,
    pub preset_volume: u8,
    #[serde(default)]
    pub expression_mappings: Vec<serde_json::Value>,
    #[serde(default)]
    pub extra_tlvs: Vec<serde_json::Value>,
    #[serde(default)]
    pub runtime_effects: Vec<serde_json::Value>,
}

/// 单条角色记录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotSpec {
    pub effect_id: u8,
    pub variant: u8,
    pub enabled: bool,
    pub params: Vec<f32>,
    #[serde(default)]
    pub tempo_sync: Vec<serde_json::Value>,
}

impl PresetSpec {
    /// 0x43 写不了、会被丢弃的非空透传字段及条目数。
    pub fn passthrough_counts(&self) -> Vec<(&'static str, usize)> {
        let mut v = vec![
            ("fx2Slot", usize::from(self.fx2_slot != FX2_SLOT_NONE)),
            ("expressionMappings", self.expression_mappings.len()),
            ("extraTlvs", self.extra_tlvs.len()),
            ("runtimeEffects", self.runtime_effects.len()),
            ("tempoSync", self.slots.iter().map(|s| s.tempo_sync.len()).sum()),
        ];
        v.retain(|(_, n)| *n > 0);
        v
    }
}

/// 资产引用条目。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetRef {
    pub slot: u8,
    pub name: String,
    /// ZIP 内路径，如 `amp/slot_01.ead`、`ir/slot_00.bin`。
    pub asset: String,
    /// 剥壳负载长度（amp 12242 / ir 4096）。
    pub length: u32,
    pub crc32: u32,
    /// 完整模型用 `ead` / `f32le`；导出的裸负载用 `ead-payload`。
    pub encoding: String,
}

/// ZIP 内资产内容（路径 → 字节）。
pub type BundleAssets = BTreeMap<String, Vec<u8>>;

impl Manifest {
    pub fn new(scope: &str, presets: Vec<ManifestPreset>) -> Self {
        Self {
            schema: MANIFEST_SCHEMA.to_string(),
            version: 1,
            device_kind: DEVICE_KIND.to_string(),
            exported_at: iso8601_now(),
            scope: scope.to_string(),
            presets,
            amp_slots: Vec::new(),
            ir_slots: Vec::new(),
        }
    }
}

/// 读取 `.npreset`：返回 manifest 与全部资产。
pub fn read_bundle(path: &Path) -> Result<(Manifest, BundleAssets), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("cannot open {path:?}: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("not a valid zip: {e}"))?;
    let mut assets = BTreeMap::new();
    let mut manifest_raw = None;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().to_string();
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        if name == "manifest.json" {
            manifest_raw = Some(buf);
        } else {
            assets.insert(name, buf);
        }
    }
    let raw = manifest_raw.ok_or("no manifest.json in the bundle")?;
    let manifest: Manifest =
        serde_json::from_slice(&raw).map_err(|e| format!("manifest.json: {e}"))?;
    Ok((manifest, assets))
}

/// 写出 `.npreset`。
pub fn write_bundle(path: &Path, manifest: &Manifest, assets: &BundleAssets) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("cannot create {path:?}: {e}"))?;
    let mut zip = zip::ZipWriter::new(file);
    let options: zip::write::SimpleFileOptions =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let manifest_json = serde_json::to_vec_pretty(manifest).map_err(|e| e.to_string())?;
    zip.start_file("manifest.json", options)
        .map_err(|e| e.to_string())?;
    zip.write_all(&manifest_json).map_err(|e| e.to_string())?;
    for (name, data) in assets {
        zip.start_file(name.as_str(), options)
            .map_err(|e| e.to_string())?;
        zip.write_all(data).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

/// (effectId, variant) → 0x43 参数数（App 侧管理数）。
///
/// 从 `factory_presets.json`提取的
/// 33 个组合，全部一致。设备对参数数超出该表的 `0x43` 短帧回 `00 03` 拒收
/// （备份 manifest 存的是设备形态计数——eq 恒多 1，
/// 部分预设多 1-2 个——不做截断时 26/40 短帧被拒）。
const APP_PARAM_COUNTS: &[(u8, u8, usize)] = &[
    (1, 0, 5),
    (2, 0, 2),
    (3, 0, 5),
    (3, 1, 5),
    (3, 2, 5),
    (3, 3, 5),
    (3, 5, 5),
    (3, 6, 5),
    (3, 7, 5),
    (4, 0, 2),
    (4, 1, 3),
    (4, 2, 3),
    (4, 3, 5),
    (4, 4, 4),
    (5, 0, 4),
    (5, 1, 3),
    (5, 4, 3),
    (5, 5, 5),
    (6, 0, 3),
    (6, 1, 6),
    (6, 2, 8),
    (7, 0, 1),
    (7, 1, 0),
    (7, 3, 6),
    (8, 0, 3),
    (8, 1, 3),
    (8, 2, 3),
    (8, 3, 3),
    (8, 4, 3),
    (8, 6, 1),
    (8, 8, 4),
    (8, 9, 5),
    (8, 10, 6),
];

/// 查 (effectId, variant) 的 App 参数数；表外组合返回 `None`（不截断）。
fn app_param_count(effect_id: u8, variant: u8) -> Option<usize> {
    APP_PARAM_COUNTS
        .iter()
        .find(|(e, v, _)| *e == effect_id && *v == variant)
        .map(|(_, _, c)| *c)
}

/// 把核心字段折算成 0x43 构造器输入。
///
/// 截断规则：每条角色的参数数按 [`app_param_count`] 截到 App 管理数
/// （表外组合不动）。App 备份 manifest 存的是设备形态计数（eq 恒多 1 个
/// 隐藏参数），直接构造会被设备拒收。
impl PresetSpec {
    pub fn to_preset_write(&self) -> crate::preset43::PresetWrite {
        // 角色序 = `protocol::CHAIN_ROLES` 下标（fx1 fx2 amp cab delay mod reverb eq），
        // `slots` 已按角色序存放，故按下标一一对应即可，无需再查名字。
        crate::preset43::PresetWrite {
            name: self.name.clone(),
            chain_order: self.chain_order,
            preset_volume: self.preset_volume,
            amp_slot: self.amp_slot,
            ir_slot: self.ir_slot,
            slots: self
                .slots
                .iter()
                .map(|s| {
                    let mut params = s.params.clone();
                    if let Some(count) = app_param_count(s.effect_id, s.variant) {
                        // 双向对齐：超出截断（App 备份多存设备隐藏参数），
                        // 不足补 0.5（设备默认值——App 备份缺尾部参数时，
                        // 设备对参数数不匹配的短帧整帧丢弃）。
                        params.resize(count, 0.5);
                    }
                    crate::preset43::SlotWrite {
                        effect_id: s.effect_id,
                        variant: s.variant,
                        enabled: s.enabled,
                        params,
                    }
                })
                .collect(),
        }
    }
}

/// 无外部依赖的 UTC ISO-8601 时间戳（精确到毫秒，形如 App 产物）。
fn iso8601_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // 简化的天数→日期换算（civil-from-days，Howard Hinnant 算法）。
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, rem % 3600 / 60, rem % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}.000Z")
}
