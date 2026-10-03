# nanocore-cli

简体中文 | [English](README.en.md)

<p align="center">
  <img src="README.assets/banner.jpeg" alt="nanocore-cli banner" />
</p>

---

用于控制 [LIVTRA NanoCore](https://livtramusic.com/products/nanocore) 口袋效果器（TPE-S10）的非官方 Rust CLI。

通过 USB-MIDI 驱动真机。可控制预设切换、8 个效果模块的开关/模型/参数、效果链顺序、与 IR 资产读写等功能，涵盖官方 App 的大部分操作。

## CLI

> [!NOTE]
> 命令中的编号一律 **1 起**（预设 1-64、链位/模块 1-8、槽位 1-30/38、通道 1-16，与设备屏和 App 同号）；只有 `b5`/`b6`、CC 这类线缆字节参数保持原值。

```sh
nanocore-cli ports                        # 枚举 MIDI 端口

nanocore-cli preset list                  # 列出 64 个预设槽
nanocore-cli preset set 5                 # 选预设 5
nanocore-cli preset next / prev           # 切换到下/上一个预设
nanocore-cli preset volume 80             # 预设音量
nanocore-cli preset rename 5 MyRig        # 重命名预设槽
nanocore-cli preset export 5 --out rig.npreset        # 导出预设为 .npreset
nanocore-cli preset import rig.npreset --with-assets  # 从 .npreset 导入（含 IR/amp 资产）
nanocore-cli preset restore-factory       # 重放官方"仅 DSP 恢复出厂"（覆盖 1-40 槽）

nanocore-cli module amp off               # 关闭音箱模块
nanocore-cli module amp model 3           # 选音箱模型
nanocore-cli module cab model 8           # 选箱体/IR 模型
nanocore-cli module fx1 on                # 开关一个模块
nanocore-cli module delay params          # 列出模块参数（离线；类型依赖时自动读设备）
nanocore-cli module delay param mix 0.3   # 设参数（掩码自动推导）
nanocore-cli module delay param mix       # 读当前值（来自快照）
nanocore-cli module delay model 2         # 选效果类型

nanocore-cli chain                        # `chain set` / `chain show` 用法
nanocore-cli chain set 5 8 1 2 3 4 6 7    # 写链序（回显校验）

nanocore-cli tuner on / off               # 调音表开关

nanocore-cli models / models --live       # 内置模型表 / 真机查询

nanocore-cli show                         # 状态快照：链序、预设、音量、各模块

nanocore-cli settings show                # 设置页全览（音量/无线/回放/增益/通道）
nanocore-cli settings usb-volume 90        # USB 输出音量
nanocore-cli settings bt-volume 80        # 蓝牙音量
nanocore-cli settings wireless off        # 启用/关闭无线
nanocore-cli settings input-gain -- -3    # 输入增益（dB，可为负）
nanocore-cli settings input-channel 2     # 输入通道

nanocore-cli asset read ir 1 --out out.bin   # 把 1 号槽的 IR blob 拉出来
nanocore-cli asset write amp 29 tone.ead     # 把 .ead/.eadl 模型写入 amp 槽

# 协议 / 调试（未公开的帧格式）：
nanocore-cli debug monitor                # 打印所有入站帧
nanocore-cli debug raw 6d 00 02 00 00 09 01
nanocore-cli debug replay 7d 4e 43 70 ...
nanocore-cli debug probe 1                # 选预设流程 + 3 秒抓取
```

命令面与设备模型一致：一个**预设**（preset）含 8 个效果**模块**（`fx1 fx2 amp cab mod delay reverb eq`），参数从属于各自的模块：

```text
preset  →  preset set/next/prev/list/show/volume/rename/export/import/restore-factory
module  →  module <m> on|off|model|params|param
chain   →  chain set/show
```

模块的参数列表若依赖当前效果类型（`delay`/`reverb`/`fx1`/`fx2`/`mod`/`eq`），可以传 `--type N` 指定（`nanocore-cli module delay params --type 1`）；不传时会从设备快照读当前效果类型。`module <m> param <name>` 不带值即读取当前值：值取自当前预设快照，`b6` 直接索引快照里的设备数组。

全局参数：`--in-port N` `--out-port N` `--hint NAME` `--ch 1-16` `--timeout MS` `--retries N`（另有协议调试用的隐藏参数，见 `debug` 命令文档）。

退出码：`0` 成功；`2` 参数或通信错误；`3` 校验不一致。

## Shell 补全

fish:

```sh
 # ~/.config/fish/completions/nanocore-cli.fish
COMPLETE=fish nanocore-cli | source
```

bash:

```sh
# ~/.bashrc
source <(COMPLETE=bash nanocore-cli)
```

zsh:

```sh
# ~/.zshrc
source <(COMPLETE=zsh nanocore-cli)
```

## 构建与测试

```sh
cargo build --features midi
# 不加 `--features midi` 时 cargo 只编译库、不产出可执行文件
```

```sh
cargo test
```

## 声明与许可

非官方项目，与 LIVTRA 无关联。研究性质，风险自负。

GPL-3.0-only，见 [LICENSE](LICENSE)。
