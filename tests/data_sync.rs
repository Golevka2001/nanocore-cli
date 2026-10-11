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

/// reverb 的 7 个类型落在**不连续**的 id 上：0,1,2,3,5,6,7，variant 4 不使用；
/// 且**名称顺序与 id 顺序不一致**——这是本表最容易照手册抄错的地方。
///
/// 逐个写 CC47（reverb 的类型 CC）值 0..6，回读 `0x63` 快照的
/// `variant` 字节，得 `0,1,2,3,5,6,7`；写 >=7 被钳在 7。名称由两条独立证据交叉确定：
///
/// * CC 指南「CC47 / ID」表：`4=闪烁(shimmer) 5=云雾(cloud) 6=弹簧(spring)`
/// * 用户在设备/App 上核对：预设 #14 Psych 的 REV 是 `spring`，而它的 `variant` 是 7
///
/// 手册 REV 那栏是**产品顺序**（`5-7 spring`），与存储槽位不同：variant 7 是设备上的
/// 第 5 个类型。`e6b6c19` 曾把产品顺序直接填到按 CC 测出的 id 上，末三项整体错位，
/// 于是 Psych 印成 `Cloud`。本断言挡的就是这个。
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
            (5, "Shimmer"),
            (6, "Cloud"),
            (7, "Spring"),
        ],
        "reverb 类型 id / 名称已变——先在设备上重测 CC47 逐值回读，别照手册产品顺序改"
    );
}

/// reverb 的参数表键同样是**真实 variant**（0,1,2,3,5,6,7），不是手册产品序号 0..6；
/// 且每类只有 5 个参数，CC 指南那 6 个里的第 6 项（CC90）设备并不暴露。
///
/// 7 个 reverb 类型的 `param count` 全是 5。
/// 产品序号 4,5,6（spring/shimmer/cloud）对应的 variant 是 7,5,6。
/// `type=` 印的编号必须恒 ≥ 1，且与 `variant + 1` 同一基准。
///
/// 这条守的是一次真实的回归：`type_number` 的 reverb 映射表写成
/// `[0, 1, 2, 3, 4, 6, 7, 5]`，注释说是「1 起始」，但首项是 `0` —— 于是
/// Room（variant 0）印出 `type=0`，一个设备上不可能存在的编号。其它块走
/// `variant + 1`，`reverb` 走查表，两者基准必须一致。
#[test]
fn type_numbers_are_one_based_and_consistent() {
    for (block, types) in params_data::EFFECT_TYPES {
        for &(variant, name) in *types {
            let n = params_data::type_number(block, variant)
                .unwrap_or_else(|| panic!("{block} type {variant} ({name}) 没有类型编号"));
            assert!(
                n >= 1,
                "{block} variant {variant} ({name}) 印出 type={n} —— 编号必须 ≥ 1"
            );
            // 非 reverb 的块 id 连续，编号就是 variant + 1；这条同时把
            // 「reverb 之外不该有例外」钉住。
            if *block != "reverb" {
                assert_eq!(
                    n,
                    variant + 1,
                    "{block} variant {variant} ({name}) 的编号应等于 variant + 1"
                );
            }
        }
        // amp / cab 的 variant 恒 0，型号在各自型号表里，没有可显示的编号。
        if *block == "amp" || *block == "cab" {
            assert_eq!(
                params_data::type_number(block, 0),
                None,
                "{block} 的 variant 恒 0，不该印出编号"
            );
        }
    }
}

/// reverb 的 `type=` 是手册产品顺序里的位置（1 起始），与 `variant` 错开。
///
/// 写 CC47 = 0..6 回读 variant 得 0,1,2,3,5,6,7；
/// 用户在设备/App 上核对 Psych 的 REV 是 spring 而其 variant 是 7。
/// 手册 `5-7 spring` 说明设备上显示的是产品顺序的位置。
#[test]
fn reverb_type_numbers_follow_the_manual_product_order() {
    // 手册 REV 顺序：room plate hall concert spring shimmer cloud
    let expected = [
        (0u8, 1u8, "Room"),
        (1, 2, "Plate"),
        (2, 3, "Hall"),
        (3, 4, "Concert"),
        (7, 5, "Spring"),
        (5, 6, "Shimmer"),
        (6, 7, "Cloud"),
    ];
    for (variant, want, name) in expected {
        assert_eq!(
            params_data::type_number("reverb", variant),
            Some(want),
            "reverb variant {variant} ({name}) 应显示为 type={want}"
        );
    }
}

#[test]
fn reverb_params_are_keyed_by_real_variant() {
    for (block, ty, ps) in params_data::TYPED.iter().filter(|(b, _, _)| *b == "reverb") {
        let expected: &[u8] = match *ty {
            0 | 1 | 2 | 3 | 5 | 6 | 7 => &[0, 1, 2, 3, 4],
            other => panic!("reverb 出现未知 variant {other}（只应有 0,1,2,3,5,6,7）"),
        };
        assert_eq!(
            ps.len(),
            expected.len(),
            "{block} type {ty} 的参数个数变了——设备每类固定 5 个"
        );
        for (p, &idx) in ps.iter().zip(expected) {
            assert_eq!(p.index, idx, "{block} type {ty} 的参数 index 应连续 0..4");
        }
    }
    let keys: Vec<u8> = params_data::TYPED
        .iter()
        .filter(|(b, _, _)| *b == "reverb")
        .map(|(_, t, _)| *t)
        .collect();
    assert_eq!(
        keys,
        vec![0, 1, 2, 3, 5, 6, 7],
        "reverb 参数表的键应是真实 variant（0,1,2,3,5,6,7），不是手册产品序号 0..6"
    );
}

/// fx2 的 11 个类型名是设备/App 的 pedal 名，不是 CC 指南的 `Drive 1-5` / `Boost 1-3`。
///
/// 设备上核对过：FX2 `type=5` 显示 `PiFUZZ v.2`，而 CC 指南在那一格写的是
/// `Drive 5`。id 与手册 `N-11` 的产品编号一致（0 起始），CC41 写 0..10
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
/// amp 与 cab 的 `variant` 全是 0，且写 `model 1 / 5 / 25 / 34`
/// 后快照的 variant 都不变。用它当 `models_data` 的索引会印出一个与实际型号
/// 无关的名字。型号只能从 `0x36` 应答的
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

/// fx1 与 eq 的类型名用**手册效果列表的原名**，不是自创的展开写法。
///
/// 此前 CLI 写的是 `Gate` / `Auto Gate` / `Gate + Compressor` / `Compressor` 与
/// `3-band EQ` / `6-band EQ` / `8-band EQ` —— 全是自创的。手册「效果列表」节
/// 写的是 `1-4 gate` / `2-4 auto gate` / `3-4 NG-Comp` / `4-4 compressor` 与
/// `1-3 eq3` / `2-3 eq6` / `3-3 eq8`；用户核对设备与 App 上也是 `EQ3` 与
/// `Threshold` 这种写法。
///
/// 参数名同理：`fx1` 两个类型的首参在手册与设备上都叫 `Threshold`，此前 CLI
/// 给它加了块名前缀（`Gate Threshold`），用户按设备上看到的名字找不到。
///
/// 这条断言的用意是给「名字从哪来」留依据 —— 此前那两条只比字符串列表，
/// 名字被改回自创写法时测试照样绿。
#[test]
fn effect_type_names_come_from_the_manual() {
    let names = |block: &str| -> Vec<(u8, &str)> {
        params_data::EFFECT_TYPES
            .iter()
            .find(|(k, _)| *k == block)
            .map(|(_, ts)| ts.iter().map(|(id, n)| (*id, *n)).collect())
            .unwrap_or_else(|| panic!("{block} in EFFECT_TYPES"))
    };
    assert_eq!(
        names("fx1"),
        vec![(0, "gate"), (1, "auto gate"), (2, "NG-Comp"), (3, "compressor")],
        "fx1 类型名应取手册「1-4 gate / 2-4 auto gate / 3-4 NG-Comp / 4-4 compressor」"
    );
    assert_eq!(
        names("eq"),
        vec![(0, "EQ3"), (1, "EQ6"), (2, "EQ8")],
        "eq 类型名应取手册「1-3 eq3 / 2-3 eq6 / 3-3 eq8」，与设备/App 显示一致"
    );
    // 参数名不带块名前缀。
    let fx1_gate = params_data::params("fx1", Some(0)).expect("fx1 gate");
    assert_eq!(
        fx1_gate.first().map(|p| p.name),
        Some("Threshold"),
        "fx1 gate 首参在手册与设备上都叫 Threshold"
    );
}

/// **任何参数名都不得以块名开头** —— 除非手册里真那么写。
///
/// 此前 CLI 给 5 个参数加了块名前缀：`Amp Gain` / `Amp Level` / `Cab Low Cut` /
/// `Cab High Cut` / `Cab Level`，而设备与 App 上（以及手册的参数说明栏）写的是
/// `Gain` / `Level` / `Low Cut` / `High Cut`。用户按设备上看到的名字找不到，
/// 2026-10-03 指出「参数名也没有 cab」。
///
/// 唯一的例外是 `delay` 的 `Delay`：那是手册 `1-8 bbd` 的真实参数名（其余延迟
/// 类型叫 `Time`），不是块名前缀。
#[test]
fn no_parameter_name_starts_with_its_block_name() {
    // 手册里确实带块名/限定词的那几个。
    const ALLOWED: &[(&str, &str)] = &[("delay", "Delay")];
    let check = |block: &str, name: &str| {
        if ALLOWED.iter().any(|(b, n)| *b == block && *n == name) {
            return;
        }
        assert!(
            !name.to_ascii_lowercase().starts_with(block),
            "{block} 的参数名 `{name}` 以块名开头 —— 设备上写的是不带前缀的那个"
        );
    };
    for (block, _, ps) in params_data::TYPED {
        for p in *ps {
            check(block, p.name);
        }
    }
    for (block, _, ps) in params_data::FIXED {
        for p in *ps {
            check(block, p.name);
        }
    }
    for (block, rs) in params_data::FLAT_RANGES {
        for r in *rs {
            check(block, r.name);
        }
    }
}

/// 参数个数一律等于**设备实存**的个数。
///
/// 此前 16 处按 CC 指南的表多列了一个末位「电平」，设备并不存那个 b6；`cab` 还
/// 多了一个 `Level`(CC67)。写它会被设备拒绝（`not in device storage`），
/// `params` 列表里则是多印一行不存在的东西。
#[test]
fn parameter_counts_match_what_the_device_stores() {
    // 设备上 (块, variant) -> count 的实际取值。
    const STORED: &[(&str, u8, usize)] = &[
        ("amp", 0, 5),
        ("cab", 0, 2),
        ("fx1", 0, 2),
        ("fx1", 3, 6),
        ("fx2", 0, 3),
        ("fx2", 1, 3),
        ("fx2", 2, 3),
        ("fx2", 3, 3),
        ("fx2", 4, 3),
        ("fx2", 8, 3),
        ("fx2", 9, 4),
        ("fx2", 10, 5),
        ("mod", 0, 2),
        ("mod", 1, 3),
        ("mod", 2, 3),
        ("mod", 3, 5),
        ("mod", 4, 4),
        ("delay", 0, 4),
        ("delay", 1, 3),
        ("delay", 2, 4),
        ("delay", 3, 4),
        ("delay", 4, 3),
        ("delay", 5, 5),
        ("delay", 6, 5),
        ("delay", 7, 5),
        ("reverb", 0, 5),
        ("reverb", 1, 5),
        ("reverb", 2, 5),
        ("reverb", 3, 5),
        ("reverb", 5, 5),
        ("reverb", 6, 5),
        ("reverb", 7, 5),
    ];
    // 有几个类型**故意不在**表里，因为手册没给它们的参数说明：
    //   * `fx1` type 1（auto gate）—— 手册「无手动调节参数」，设备仍存 2 个
    //     float
    //   * `fx1` type 2（NG-Comp）—— 手册有 7 个参数说明，但我们未逐一核对过
    //   * `fx2` type 5/6/7（Range/XAC/Fiman）—— 手册只有一条 `Boost`
    // **「手册说无参数」≠「设备不存值」**，两者别混；本断言只覆盖有表的类型。
    for (block, ty, want) in STORED.iter().copied() {
        let got = params_data::params(block, Some(ty))
            .unwrap_or_else(|| panic!("{block} type {ty}"))
            .len();
        assert_eq!(
            got, want,
            "{block} type {ty} 的参数个数应为 {want}（设备实存），表里是 {got}"
        );
    }
    // 不随类型变化的两个块走 FLAT_RANGES / FIXED 的同一份数据。
    assert_eq!(params_data::params("cab", None).unwrap().len(), 2);
    assert_eq!(params_data::params("amp", None).unwrap().len(), 5);
}
