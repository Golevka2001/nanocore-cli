# nanocore-cli

简体中文 | [English](README.en.md)

---

用于控制 [LIVTRA NanoCore](https://livtramusic.com/products/nanocore) 口袋效果器（TPE-S10）的非官方 Rust CLI。

通过 USB-MIDI 驱动真机。可控制预设切换、8 个效果模块的开关/模型/参数、效果链顺序、USB 音量、调音表，以及音箱模型与 IR 资产的读写。

## CLI

```sh
nanocore-cli ports                        # 枚举 MIDI 端口

nanocore-cli preset list                  # 列出 64 个预设槽
nanocore-cli preset set 5                 # 选预设 5
nanocore-cli preset next / prev           # 切换到下/上一个预设

nanocore-cli module amp off               # 关闭音箱模块
nanocore-cli module amp model 3           # 选音箱模型
nanocore-cli module cab model 8           # 选箱体/IR 模型
nanocore-cli module fx1 on                # 开关一个模块
nanocore-cli module delay params          # 列出模块参数（离线）
nanocore-cli module delay param mix 0.3   # 设参数
nanocore-cli module delay model 2         # 选效果类型

nanocore-cli chain                        # `chain set` / `chain show` 用法
nanocore-cli chain set 4 7 0 1 2 3 5 6    # 写链序（回显校验）

nanocore-cli models / models --live       # 内置模型表 / 真机查询

nanocore-cli show                         # 可读的状态快照

nanocore-cli volume 90                    # USB 输出音量

nanocore-cli tuner on / off               # 调音表（状态读取待做）

nanocore-cli asset read ir 0 out.bin      # 把 0 号槽的 IR blob 拉出来

# 协议 / 调试（未公开的帧格式）：
nanocore-cli debug monitor                # 打印所有入站帧
nanocore-cli debug raw 6d 00 02 00 00 09 01
nanocore-cli debug replay 7d 4e 43 70 ...
nanocore-cli debug probe 1                # 选预设流程 + 3 秒抓取
```

命令面与设备模型一致：一个**预设**（preset）含 8 个效果**模块**（`fx1 fx2 amp cab mod delay reverb eq`），参数从属于各自的模块：

```text
preset  →  preset set/next/prev/list/show
module  →  module <m> on|off|model|params|param
chain   →  chain set/show
```

模块的参数列表若依赖当前效果类型（`delay`/`reverb`/`fx1`/`fx2`/`mod`/`eq`），需传 `--type N`（`nanocore-cli module delay params --type 1`）。快照解码完成后，`--type` 会变成可选（由设备推断），`module <m> param <name>` 不带值时将读取当前值（目前是写入；省略 VALUE 的读取待做）。

两条来自硬件的固有约束：

- `module amp model` / `module cab model` 只接受 **0–29**。8 个 FX2 踏板效果占 amp 表的 30–37 号槽，只能通过 `debug variant` 访问。
- `show` 目前是尽力而为——当前预设、各模块状态、音量、调音表都标着 `(TBD)`，等 `0x63` 快照解码完成。

全局参数：`--in N` `--out N` `--hint NAME` `--timeout MS` `--retries N`（另有协议调试用的隐藏参数，见 `debug` 命令文档）。

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
```

```sh
cargo test
```

## 声明与许可

非官方项目，与 LIVTRA 无关联。研究性质，风险自负。

GPL-3.0-only，见 [LICENSE](LICENSE)。
