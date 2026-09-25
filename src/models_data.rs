//! 内嵌的 Nanocore 音箱 / 箱体模型表。
//!
//! 与 `data/models.json` 保持镜像；
//! 同步由 `tests/data_sync.rs` 强制保证 —— 修改 JSON 时须一并修改本表，
//! 二者不一致时 `cargo test` 会失败。
//! 来源：固件 `ampAssets`/`irAssets`（槽位顺序）、官方用户手册（友好的 "based on" 名称）、
//! 以及 `strings_en.json`（设备槽位数：AMP+Fx2 = 38、IR = 30）。索引均为 0 起始的固件槽位。
#![allow(dead_code)]

/// 单个模型条目。`index` 为 0 起始的固件槽位（设备 UI 显示 `index + 1`）。
pub struct ModelInfo {
    pub index: u8,
    pub code: &'static str,
    /// 友好名称（对手册中品牌参照名的 ASCII 近似写法）。
    pub name: &'static str,
    /// `"amp"` | `"drive"`（FX2 踏板效果）| `"cab"`。
    pub category: &'static str,
}

/// 音箱模拟 + FX2 踏板效果模型（固件 `ampAssets`，38 条，槽位 0-37）。
pub const AMP: &[ModelInfo] = &[
    ModelInfo { index: 0, code: "BogXTC1", name: "Bogner XTC #1", category: "amp" },
    ModelInfo { index: 1, code: "BogXTC2", name: "Bogner XTC #2", category: "amp" },
    ModelInfo { index: 2, code: "ENGP1", name: "ENGL Powerball E645 #1", category: "amp" },
    ModelInfo { index: 3, code: "ENGP2", name: "ENGL Powerball E645 #2", category: "amp" },
    ModelInfo { index: 4, code: "Fd59Bm", name: "Fender 59 Bassman", category: "amp" },
    ModelInfo { index: 5, code: "Fd65Dk", name: "Fender 65 Darkface", category: "amp" },
    ModelInfo { index: 6, code: "Hiw103", name: "Hiwatt DR103", category: "amp" },
    ModelInfo { index: 7, code: "MarDSL1", name: "Marshall DSL50 #1", category: "amp" },
    ModelInfo { index: 8, code: "MarDSL2", name: "Marshall DSL50 #2", category: "amp" },
    ModelInfo { index: 9, code: "Mar8001", name: "Marshall JCM800 #1", category: "amp" },
    ModelInfo { index: 10, code: "Mar8002", name: "Marshall JCM800 #2", category: "amp" },
    ModelInfo { index: 11, code: "MesR1", name: "Mesa Boogie Rectifier #1", category: "amp" },
    ModelInfo { index: 12, code: "MesR2", name: "Mesa Boogie Rectifier #2", category: "amp" },
    ModelInfo { index: 13, code: "MesR3", name: "Mesa Boogie Rectifier #3", category: "amp" },
    ModelInfo { index: 14, code: "OgG120", name: "Orange Graphic 120", category: "amp" },
    ModelInfo { index: 15, code: "Pey51501", name: "Peavey 5150 #1", category: "amp" },
    ModelInfo { index: 16, code: "Pey51502", name: "Peavey 5150 #2", category: "amp" },
    ModelInfo { index: 17, code: "Pey6505", name: "Peavey 6505", category: "amp" },
    ModelInfo { index: 18, code: "RanT21", name: "Randall T2 #1", category: "amp" },
    ModelInfo { index: 19, code: "RanT22", name: "Randall T2 #2", category: "amp" },
    ModelInfo { index: 20, code: "RoJC120", name: "Roland JC-120", category: "amp" },
    ModelInfo { index: 21, code: "SLO1001", name: "Soldano SLO-100 #1", category: "amp" },
    ModelInfo { index: 22, code: "SLO1002", name: "Soldano SLO-100 #2", category: "amp" },
    ModelInfo { index: 23, code: "SLO1003", name: "Soldano SLO-100 #3", category: "amp" },
    ModelInfo { index: 24, code: "VXAC30", name: "VOX AC30", category: "amp" },
    ModelInfo { index: 25, code: "ApSVTVR", name: "Ampeg SVT VR", category: "amp" },
    ModelInfo { index: 26, code: "ApSVTCL", name: "Ampeg SVT CL", category: "amp" },
    ModelInfo { index: 27, code: "Adn900", name: "Ashdown ABM900 EVO II", category: "amp" },
    ModelInfo { index: 28, code: "Hiw320", name: "Hiwatt Jupiter 320", category: "amp" },
    ModelInfo { index: 29, code: "MarSup", name: "Marshall Super Bass", category: "amp" },
    ModelInfo { index: 30, code: "scream", name: "Ibanez TS9 (drive)", category: "drive" },
    ModelInfo { index: 31, code: "Klone", name: "Ryra The Klone (drive)", category: "drive" },
    ModelInfo { index: 32, code: "OCD", name: "Fulltone OCD (drive)", category: "drive" },
    ModelInfo { index: 33, code: "DS2", name: "Boss DS-2 (drive)", category: "drive" },
    ModelInfo { index: 34, code: "PiFUZZ v.2", name: "EHX Big Muff Pi (drive)", category: "drive" },
    ModelInfo { index: 35, code: "Range", name: "Dallas-Arbiter Rangemaster (drive)", category: "drive" },
    ModelInfo { index: 36, code: "AC", name: "Xotic AC Booster (drive)", category: "drive" },
    ModelInfo { index: 37, code: "Fiman", name: "Friedman Marty Friedman (drive)", category: "drive" },
];

/// 箱体 / IR 模型（固件 `irAssets`，30 条，槽位 0-29）。
pub const CAB: &[ModelInfo] = &[
    ModelInfo { index: 0, code: "Bog412A", name: "Bogner 4x12 A", category: "cab" },
    ModelInfo { index: 1, code: "Bog412B", name: "Bogner 4x12 B", category: "cab" },
    ModelInfo { index: 2, code: "Eng412A", name: "ENGL E412 ProVintage30 4x12 A", category: "cab" },
    ModelInfo { index: 3, code: "Eng412B", name: "ENGL E412 ProVintage30 4x12 B", category: "cab" },
    ModelInfo { index: 4, code: "FdTR212", name: "Fender Twin Reverb Darkface 2x12", category: "cab" },
    ModelInfo { index: 5, code: "Fd59BM410", name: "Fender 1959 Bassman 4x10", category: "cab" },
    ModelInfo { index: 6, code: "Hiw412", name: "Hiwatt Brilliant 4x12", category: "cab" },
    ModelInfo { index: 7, code: "Mar412A", name: "Marshall V30 4x12 A", category: "cab" },
    ModelInfo { index: 8, code: "Mar412B", name: "Marshall V30 4x12 B", category: "cab" },
    ModelInfo { index: 9, code: "Mar96412A", name: "Marshall 1960B 4x12 A", category: "cab" },
    ModelInfo { index: 10, code: "Mar96412B", name: "Marshall 1960B 4x12 B", category: "cab" },
    ModelInfo { index: 11, code: "Mes412A", name: "Mesa/Boogie Oversized V30 4x12 A", category: "cab" },
    ModelInfo { index: 12, code: "Mes412B", name: "Mesa/Boogie Oversized V30 4x12 B", category: "cab" },
    ModelInfo { index: 13, code: "Mes412C", name: "Mesa/Boogie Oversized V30 4x12 C", category: "cab" },
    ModelInfo { index: 14, code: "Og412", name: "Orange Vintage30 4x12", category: "cab" },
    ModelInfo { index: 15, code: "pey412A", name: "Peavey 5150 4x12 A", category: "cab" },
    ModelInfo { index: 16, code: "pey412B", name: "Peavey 5150 4x12 B", category: "cab" },
    ModelInfo { index: 17, code: "peyVk412", name: "Peavey Valveking 4x12", category: "cab" },
    ModelInfo { index: 18, code: "Ran112A", name: "Randall RD112 1x12 A", category: "cab" },
    ModelInfo { index: 19, code: "Ran112B", name: "Randall RD112 1x12 B", category: "cab" },
    ModelInfo { index: 20, code: "Ran112C", name: "Randall RD112 1x12 C", category: "cab" },
    ModelInfo { index: 21, code: "SLO412A", name: "Soldano SLO412 4x12 A", category: "cab" },
    ModelInfo { index: 22, code: "SLO412B", name: "Soldano SLO412 4x12 B", category: "cab" },
    ModelInfo { index: 23, code: "SLO412C", name: "Soldano SLO412 4x12 C", category: "cab" },
    ModelInfo { index: 24, code: "AC30212", name: "VOX AC30 2x12", category: "cab" },
    ModelInfo { index: 25, code: "AP810", name: "Ampeg 8x10", category: "cab" },
    ModelInfo { index: 26, code: "Ap410", name: "Ampeg SVT 4x10", category: "cab" },
    ModelInfo { index: 27, code: "Adn410", name: "Ashdown 4x10", category: "cab" },
    ModelInfo { index: 28, code: "HiwB410", name: "Hiwatt B410 4x10", category: "cab" },
    ModelInfo { index: 29, code: "Mar412", name: "Marshall 4x12", category: "cab" },
];

/// 按 0 起始的槽位查找音箱 / 踏板效果模型。
pub fn lookup_amp(index: u8) -> Option<&'static ModelInfo> {
    AMP.iter().find(|m| m.index == index)
}

/// 按 0 起始的槽位查找箱体 / IR 模型。
pub fn lookup_cab(index: u8) -> Option<&'static ModelInfo> {
    CAB.iter().find(|m| m.index == index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amp_table_complete_and_contiguous() {
        assert_eq!(AMP.len(), 38);
        for (i, m) in AMP.iter().enumerate() {
            assert_eq!(m.index as usize, i, "amp slot {i} out of order");
        }
    }

    #[test]
    fn cab_table_complete_and_contiguous() {
        assert_eq!(CAB.len(), 30);
        for (i, m) in CAB.iter().enumerate() {
            assert_eq!(m.index as usize, i, "cab slot {i} out of order");
        }
    }

    #[test]
    fn lookups_resolve_known_slots() {
        assert_eq!(lookup_amp(0).unwrap().code, "BogXTC1");
        assert_eq!(lookup_amp(0).unwrap().category, "amp");
        assert_eq!(lookup_amp(30).unwrap().code, "scream");
        assert_eq!(lookup_amp(30).unwrap().category, "drive");
        assert_eq!(lookup_amp(37).unwrap().code, "Fiman");
        assert_eq!(lookup_cab(0).unwrap().code, "Bog412A");
        assert_eq!(lookup_cab(29).unwrap().code, "Mar412");
    }

    #[test]
    fn lookups_reject_out_of_range() {
        assert!(lookup_amp(38).is_none());
        assert!(lookup_cab(30).is_none());
    }
}

