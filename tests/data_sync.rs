//! 内嵌表与其权威 JSON 来源之间的同步守卫。
//!
//! `data/models.json` 和 `data/params.json` 是从固件/ OEM MIDI CC 指南转录而来的机器可读来源。
//! `*_data.rs` 模块以编译期表的形式嵌入同一份数据，使库在运行时保持零依赖；serde_json 仅为开发依赖。
//!
//! 任何一侧单独被修改时这些测试都会失败。
//! JSON 源变更时，须同时更新 `data/` 中的副本和 `*_data.rs` 表。

use serde_json::Value;

use nanocore::models_data;
use nanocore::params_data;

const MODELS_JSON: &str = include_str!("../data/models.json");
const PARAMS_JSON: &str = include_str!("../data/params.json");

/// 参数列表按效果类型变化的块（JSON 中的 `by_type`）。
const BY_TYPE_BLOCKS: [&str; 5] = ["fx1", "fx2", "mod", "delay", "reverb"];

// ─── models ─────────────────────────────────────────────────────────────────

#[test]
fn embedded_model_tables_match_models_json() {
    let m: Value = serde_json::from_str(MODELS_JSON).expect("parse models.json");
    assert_eq!(m["schema"], "nanocore.models/v1", "schema drift");

    let check = |json: &Value, table: &[models_data::ModelInfo], what: &str| {
        let entries = json
            .as_array()
            .unwrap_or_else(|| panic!("{what}: not an array"));
        assert_eq!(
            entries.len(),
            table.len(),
            "{what}: JSON has {} rows, embedded table has {}",
            entries.len(),
            table.len()
        );
        for (j, r) in entries.iter().zip(table) {
            assert_eq!(j["index"].as_u64().unwrap(), r.index as u64, "{what} index");
            assert_eq!(j["code"], r.code, "{what} slot {} code", r.index);
            assert_eq!(j["name"], r.name, "{what} slot {} name", r.index);
            assert_eq!(
                j["category"], r.category,
                "{what} slot {} category",
                r.index
            );
        }
    };

    check(&m["amp"], models_data::AMP, "amp");
    check(&m["cab"], models_data::CAB, "cab");
}

// ─── params ─────────────────────────────────────────────────────────────────

fn json_params(list: &Value) -> Vec<(u64, String, u64)> {
    list.as_array()
        .expect("parameter list")
        .iter()
        .map(|p| {
            (
                p["index"].as_u64().unwrap(),
                p["name"].as_str().unwrap().to_owned(),
                p["cc"].as_u64().unwrap(),
            )
        })
        .collect()
}

fn rs_params(ps: &[params_data::ParamInfo]) -> Vec<(u64, String, u64)> {
    ps.iter()
        .map(|p| (p.index as u64, p.name.to_owned(), p.cc as u64))
        .collect()
}

#[test]
fn block_slots_match_params_json() {
    let p: Value = serde_json::from_str(PARAMS_JSON).expect("parse params.json");
    assert_eq!(p["schema"], "nanocore.params/v2", "schema drift");
    let blocks = p["blocks"].as_object().expect("blocks object");
    assert_eq!(blocks.len(), params_data::BLOCKS.len());
    for (key, slot) in params_data::BLOCKS {
        assert_eq!(
            blocks[*key].as_u64(),
            Some(*slot as u64),
            "block {key}: b5 differs between JSON and BLOCKS"
        );
    }
}

#[test]
fn typed_param_lists_match_params_json() {
    let p: Value = serde_json::from_str(PARAMS_JSON).unwrap();

    for block in BY_TYPE_BLOCKS {
        let by_type = p["params_by_block"][block]["by_type"]
            .as_object()
            .unwrap_or_else(|| panic!("{block}: no by_type map in params.json"));

        // 内嵌表中的每个类型都必须存在于 JSON 中，且参数列表相同、顺序相同（顺序即 b6 寻址）。
        let rs_types: Vec<u8> = params_data::TYPED
            .iter()
            .filter(|(k, _, _)| *k == block)
            .map(|(_, t, _)| *t)
            .collect();
        for (_, t, ps) in params_data::TYPED.iter().filter(|(k, _, _)| *k == block) {
            let j = by_type
                .get(&t.to_string())
                .unwrap_or_else(|| panic!("{block}: type {t} missing from params.json"));
            assert_eq!(
                json_params(j),
                rs_params(ps),
                "{block} type {t}: parameter list drifted"
            );
        }
        // 反向亦然：JSON 中不得出现表所省略的类型 —— 文档记载的空白
        // （fx1 类型 1-2、fx2 类型 5-7）以 Rust 侧的缺席来编码，必须保持缺席。
        // （比较时已排序：JSON map 的键按字典序排列，"10" < "2"。）
        let mut json_types: Vec<u8> = by_type
            .keys()
            .map(|k| k.parse::<u8>().expect("numeric type key"))
            .collect();
        json_types.sort_unstable();
        assert_eq!(
            json_types, rs_types,
            "{block}: effect-type key sets diverged"
        );
    }
}

#[test]
fn fixed_param_lists_match_params_json() {
    let p: Value = serde_json::from_str(PARAMS_JSON).unwrap();
    for (block, slot, ps) in params_data::FIXED {
        let j = &p["params_by_block"][*block]["fixed"];
        assert_eq!(
            json_params(j),
            rs_params(ps),
            "{block} (b5={slot}): fixed list drifted"
        );
    }
}

#[test]
fn eq_band_lists_match_params_json() {
    let p: Value = serde_json::from_str(PARAMS_JSON).unwrap();
    let bands = p["params_by_block"]["eq"]["bands_by_type"]
        .as_object()
        .expect("eq bands_by_type map");
    assert_eq!(bands.len(), params_data::EQ_BANDS.len(), "eq type count");
    for (t, ps) in params_data::EQ_BANDS {
        let j = bands
            .get(&t.to_string())
            .unwrap_or_else(|| panic!("eq: band table for type {t} missing from params.json"));
        assert_eq!(
            json_params(j),
            rs_params(ps),
            "eq type {t}: band list drifted"
        );
    }
}
