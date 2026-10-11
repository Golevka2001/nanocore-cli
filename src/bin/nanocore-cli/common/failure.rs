//! 失败的表示：面向用户的消息 + 退出码，以及已打开设备的类型别名。

use nanocore::client::Nanocore;
use nanocore::midi::MidiTransport;

/// 通过真实 MIDI 传输层建立的已打开设备连接。
pub type Dev = Nanocore<MidiTransport>;

/// 命令失败：面向用户的消息，加上它应当产生的进程退出码。
///
/// 处理器只挑选语义，退出码由 `main` 统一定夺——「校验不一致」与「普通失败」
/// 对脚本是不同的结果（见 README 的退出码约定）。
#[derive(Debug)]
pub struct Failure {
    /// 写给用户的消息（`main` 只负责加上 `error: ` 前缀）。
    pub message: String,
    /// 进程退出码。
    pub exit_code: i32,
}

impl Failure {
    /// 普通失败：参数、通信或设备错误 → 退出码 2。
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit_code: 2,
        }
    }

    /// 校验不一致：设备数据与参考文件不同 → 退出码 3。
    pub fn mismatch(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit_code: 3,
        }
    }
}

/// 来自设备/传输层（以及任何以 `String` 报错）的失败都是普通失败，
/// 因此 `?` 无需在每个调用点包装。
impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self::error(message)
    }
}
