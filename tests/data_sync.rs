//! 内嵌表与其权威 JSON 来源之间的同步守卫。
//!
//! `data/models.json` 和 `data/params.json` 是从固件/ OEM MIDI CC 指南转录而来的机器可读来源。
//! `*_data.rs` 模块以编译期表的形式嵌入同一份数据，使库在运行时保持零依赖；serde_json 仅为开发依赖。
//!
//! 任何一侧单独被修改时这些测试都会失败。
//! JSON 源变更时，须同时更新 `data/` 中的副本和 `*_data.rs` 表。

use serde_json::Value;

use nanocore::models_data;
use nanocore::protocol;
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

/// 效果类型名表（`params.json` 的 `effect_types`）必须与内嵌表逐项一致。
///
/// 这张表是 `show` / `debug snapshot` 把 `type=N` 翻译成 `Compressor`、`Room`
/// 的唯一依据。两侧漂移不会报错，只会让输出指向错误的类型名 —— 参数表也会
/// 跟着取错（`params(block, Some(N))` 用同一个 ID）。
#[test]
fn effect_type_names_match_params_json() {
    let p: Value = serde_json::from_str(PARAMS_JSON).unwrap();
    let et = p["effect_types"].as_object().expect("effect_types map");
    assert_eq!(
        et.len(),
        params_data::EFFECT_TYPES.len(),
        "effect_types block count drifted"
    );
    for (block, rs_types) in params_data::EFFECT_TYPES {
        let j = et
            .get(*block)
            .unwrap_or_else(|| panic!("{block}: missing from params.json effect_types"));
        let j_rows = j.as_array().unwrap_or_else(|| panic!("{block}: not an array"));
        assert_eq!(
            j_rows.len(),
            rs_types.len(),
            "{block}: type count drifted"
        );
        for (row, (id, name)) in j_rows.iter().zip(rs_types.iter()) {
            assert_eq!(row["id"].as_u64().unwrap(), *id as u64, "{block} type id");
            assert_eq!(row["name"], *name, "{block} type {id} name");
        }
    }
}

/// 参数量程表（`params.json` 的 `ranges`）必须与内嵌表逐项一致。
///
/// 量程错一位不会报错，只会让 `show` 把归一化值印成错误的设备读数 —— 而那正是
/// 读者拿它去对照设备/App 的数字。量程按 `(块, 效果类型)` 索引，同名参数在不同
/// 类型下量程不同（`Decay` 三种、`Rate` 四种），所以粒度必须到类型。
#[test]
fn parameter_ranges_match_params_json() {
    let p: Value = serde_json::from_str(PARAMS_JSON).unwrap();
    let r = &p["ranges"];

    let json_rows = |block: &str, tid: &str| -> Vec<(String, f64, f64, String)> {
        r["by_type"][block][tid]
            .as_array()
            .unwrap_or_else(|| panic!("ranges.by_type.{block}.{tid} missing"))
            .iter()
            .map(|x| {
                (
                    x["name"].as_str().unwrap().to_string(),
                    x["lo"].as_f64().unwrap(),
                    x["hi"].as_f64().unwrap(),
                    x["unit"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    };
    // 量程存成 f32，转 f64 后 `1.1` 变成 1.100000023841858 —— 逐位比会永远失败。
    // 这里比名称与单位（必须全等），数值比相对误差。
    let close = |a: f64, b: f64| (a - b).abs() <= 1e-5 * a.abs().max(b.abs()).max(1.0);
    let compare = |what: &str, j: &[(String, f64, f64, String)], rs: &[params_data::Range]| {
        let rows: Vec<_> = rs
            .iter()
            .map(|x| (x.name.to_string(), x.lo as f64, x.hi as f64, x.unit.to_string()))
            .collect::<Vec<_>>();
        assert_eq!(j.len(), rows.len(), "{what}: entry count drifted");
        for ((jn, jlo, jhi, ju), (rn, rlo, rhi, ru)) in j.iter().zip(rows.iter()) {
            assert_eq!(jn, rn, "{what}: name drifted");
            assert_eq!(ju, ru, "{what} {rn}: unit drifted");
            assert!(close(*jlo, *rlo), "{what} {rn}: lo {jlo} vs {rlo}");
            assert!(close(*jhi, *rhi), "{what} {rn}: hi {jhi} vs {rhi}");
        }
    };

    for (block, tid, table) in params_data::RANGES {
        let j = json_rows(block, &tid.to_string());
        compare(&format!("{block} type {tid}"), &j, table);
    }
    for (block, table) in params_data::FLAT_RANGES {
        let j: Vec<_> = r["flat"][block]
            .as_array()
            .unwrap_or_else(|| panic!("ranges.flat.{block} missing"))
            .iter()
            .map(|x| {
                (
                    x["name"].as_str().unwrap().to_string(),
                    x["lo"].as_f64().unwrap(),
                    x["hi"].as_f64().unwrap(),
                    x["unit"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        compare(block, &j, table);
    }
    // EQ 的量程对每个频段相同，手册对每段重复写了一遍，故只存一份。
    assert_eq!(r["eq_band"]["lo"].as_f64(), Some(params_data::EQ_RANGE.lo as f64));
    assert_eq!(r["eq_band"]["hi"].as_f64(), Some(params_data::EQ_RANGE.hi as f64));
    assert_eq!(r["eq_band"]["unit"], params_data::EQ_RANGE.unit);
}

/// reverb 的 7 个类型落在**不连续**的 id 上：0,1,2,3,5,6,7，variant 4 不使用。
///
/// 2026-10-03 真机实测：逐个写 CC47（reverb 的类型 CC）值 0..6，每次回读
/// `0x63` 快照确认 `variant` 字节，`type` 依次为
/// Room/Plate/Hall/Concert/Spring/Shimmer/Cloud。写 4 时设备落在 variant 5，
/// 写 >=7 被钳在 variant 7（该块只有 7 个类型）。
///
/// 这条断言挡的是「按手册的产品顺序（room..cloud）填 0..6」——手册那栏是
/// 产品编号（第几款），不是存储槽位。照抄会让 4 个类型名错位，且不会报错。
#[test]
fn reverb_effect_type_ids_are_not_contiguous() {
    let reverb: Vec<(u8, &str)> = params_data::EFFECT_TYPES
        .iter()
        .find(|(k, _)| *k == "reverb")
        .map(|(_, ts)| ts.iter().map(|(id, n)| (*id, *n)).collect())
        .expect("reverb in EFFECT_TYPES");
    assert_eq!(
        reverb,
        vec![
            (0, "Room"),
            (1, "Plate"),
            (2, "Hall"),
            (3, "Concert"),
            (5, "Spring"),
            (6, "Shimmer"),
            (7, "Cloud"),
        ],
        "reverb 类型 id / 名称已变——先真机重测 CC47 逐值回读，别照手册顺序改"
    );
}

/// fx2 的 11 个类型名是设备/App 的 pedal 名，不是 CC 指南的 `Drive 1-5` / `Boost 1-3`。
///
/// 设备上核对过：FX2 `type=5` 显示 `PiFUZZ v.2`，而 CC 指南在那一格写的是
/// `Drive 5`。id 与手册 `N-11` 的产品编号一致（0 起始），真机 CC41 写 0..10
/// 回读 `type=1..11` 连续无空洞 —— 错的只是名字。
#[test]
fn fx2_effect_types_use_pedal_names() {
    let fx2: Vec<(u8, &str)> = params_data::EFFECT_TYPES
        .iter()
        .find(|(k, _)| *k == "fx2")
        .map(|(_, ts)| ts.iter().map(|(id, n)| (*id, *n)).collect())
        .expect("fx2 in EFFECT_TYPES");
    assert_eq!(
        fx2,
        vec![
            (0, "Scream"),
            (1, "Klone"),
            (2, "OCD"),
            (3, "DS2"),
            (4, "PiFUZZ"),
            (5, "Range"),
            (6, "XAC"),
            (7, "Fiman"),
            (8, "Pitch"),
            (9, "Envelope Wah"),
            (10, "Wah"),
        ],
        "fx2 类型名已变——CC 指南的 Drive/Boost 命名设备上不显示"
    );
}

/// amp / cab 的 `variant` 恒为 0，型号不在快照里。
///
/// 40 个出厂预设的 amp 与 cab `variant` 全是 0，且写 `model 1 / 5 / 25 / 34`
/// 后快照的 variant 都不变。用它当 `models_data` 的索引会印出一个与实际型号
/// 无关的名字（曾把 VXAC30 印成 `Bogner XTC #1`）。型号只能从 `0x36` 应答的
/// per-model `active` 标记查。
#[test]
fn amp_and_cab_have_no_variant_name() {
    for (slot, block) in [(2u8, "amp"), (3, "cab")] {
        assert_eq!(
            protocol::variant_name(slot, 0),
            None,
            "{block} 的 variant 不是型号索引，不该从快照得出型号名"
        );
    }
}
