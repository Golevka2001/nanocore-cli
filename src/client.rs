//! 高层客户端。
//!
//! 除了即发即忘的 [`Nanocore::send`]，本模块还提供 [`Nanocore::transact`]，
//! 它将请求与设备应答关联起来，并在超时时重试。
//!
//! 重试策略镜像自对官方应用的观察（`liblivtra.so`，`FUN_00483094`）：
//!
//! * 初始应答超时 120 ms
//! * 每次重试翻倍，上限 600 ms
//! * 总预算（硬上限）1 s
//! * **仅在超时时**重试 —— 收到格式错误或错误应答时绝不重试

use std::collections::VecDeque;
use std::fmt;
use std::time::{Duration, Instant};

use crate::frame::{Direction, Frame};
use crate::protocol;
use crate::transport::Transport;

/// 官方应用使用的单次尝试初始应答超时。
pub const INITIAL_TIMEOUT: Duration = Duration::from_millis(120);
/// 官方应用使用的单次尝试超时上限。
pub const MAX_TIMEOUT: Duration = Duration::from_millis(600);
/// 一次事务（含全部重试）的总预算。
pub const HARD_LIMIT: Duration = Duration::from_millis(1000);
/// 等待应答时轮询传输层的间隔。
pub const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// [`Nanocore::transact`] 的重试/超时策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryConfig {
    pub initial_timeout: Duration,
    pub max_timeout: Duration,
    pub hard_limit: Duration,
    /// *重试*次数上限（即总共 `max_retries + 1` 次尝试）。
    pub max_retries: u32,
    pub poll_interval: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            initial_timeout: INITIAL_TIMEOUT,
            max_timeout: MAX_TIMEOUT,
            hard_limit: HARD_LIMIT,
            max_retries: 3,
            poll_interval: POLL_INTERVAL,
        }
    }
}

/// [`Nanocore::transact`] 产生的错误。
#[derive(Debug)]
pub enum Error<E> {
    /// 底层传输层失败。
    Transport(E),
    /// 设备在超时预算内始终未应答。
    Timeout { attempts: u32, elapsed: Duration },
}

impl<E: fmt::Display> fmt::Display for Error<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Transport(e) => write!(f, "transport error: {e}"),
            Error::Timeout { attempts, elapsed } => write!(
                f,
                "timed out waiting for response after {attempts} attempt(s) in {elapsed:?}"
            ),
        }
    }
}

impl<E: fmt::Debug + fmt::Display> std::error::Error for Error<E> {}

impl<E> From<E> for Error<E> {
    fn from(e: E) -> Self {
        Error::Transport(e)
    }
}

/// 高层 Nanocore 控制器。包装一个 [`Transport`] 并管理小端序列号计数器。
pub struct Nanocore<T: Transport> {
    transport: T,
    seq: u16,
    config: RetryConfig,
    /// 已接收但尚未被事务消费的帧 —— 主动（非请求）的事件，
    /// 或在其调用方查找之前就已到达的应答。
    pending: VecDeque<Frame>,
}

impl<T: Transport> Nanocore<T> {
    pub fn new(transport: T) -> Self {
        Self::with_config(transport, RetryConfig::default())
    }

    pub fn with_config(transport: T, config: RetryConfig) -> Self {
        Self { transport, seq: 0, config, pending: VecDeque::new() }
    }

    pub fn into_inner(self) -> T {
        self.transport
    }

    pub fn transport(&mut self) -> &mut T {
        &mut self.transport
    }

    /// 覆盖重试/超时策略。
    pub fn set_config(&mut self, config: RetryConfig) {
        self.config = config;
    }

    fn next_seq(&mut self) -> u16 {
        let s = self.seq;
        self.seq = next_seq_after(s);
        s
    }

    /// 以类型 + 载荷发送消息，`flags = 0x00`（普通）。
    ///
    /// 即发即忘：不关联应答，不重试。
    pub fn send(&mut self, kind: u8, payload: Vec<u8>) -> Result<(), T::Error> {
        let seq = self.next_seq();
        let frame = Frame::new(Direction::Tx, 0x00, seq, kind, payload);
        self.transport.send(&frame.encode())
    }

    /// 接收并解码当前可用的所有帧。
    pub fn receive(&mut self) -> Result<Vec<Frame>, T::Error> {
        let bytes = self.transport.receive()?;
        Ok(Frame::split_stream(&bytes))
    }

    /// 从传输层拉取可用字节放入待处理队列，
    /// 并返回刚刚到达的帧。
    fn pump(&mut self) -> Result<Vec<Frame>, T::Error> {
        let frames = self.receive()?;
        self.pending.extend(frames.iter().cloned());
        Ok(frames)
    }

    /// 移除并返回待处理队列中最早的、回显 `seq` 的帧，
    /// 可选择限定为 `kinds` 中的某一类型。
    ///
    /// 设备在应答时回显主机的序列号（已在抓包中验证：TX `seq=0x0344` -> RX `seq=0x0344`）。
    /// 其*主动（非请求）的*状态帧则携带设备自己的计数器，因此永远不会匹配 —— 这正是我们想要的效果。
    fn take_matching(&mut self, seq: u16, kinds: &[u8]) -> Option<Frame> {
        let pos = self.pending.iter().position(|f| {
            f.seq == seq && (kinds.is_empty() || kinds.contains(&f.kind))
        })?;
        self.pending.remove(pos)
    }

    /// 发送命令并等待设备应答。
    ///
    /// 应答指任何回显请求序列号的帧。
    /// 传入非空的 `expect` 可额外要求帧属于给定的消息类型之一。
    /// 按 [`RetryConfig`] 执行超时与重试。
    pub fn transact_expect(
        &mut self,
        kind: u8,
        payload: Vec<u8>,
        expect: &[u8],
    ) -> Result<Frame, Error<T::Error>> {
        // 只需应答；要看 tx 帧头请用 [`Nanocore::transact_with_flags`]。
        self.transact_with_flags(0x00, kind, payload, expect)
            .map(|(_tx, rx)| rx)
    }

    /// 与 [`Nanocore::transact_expect`] 相同，但显式指定 flags 字节。
    ///
    /// 观察到的 flags 值：`0x00` 普通，`0x02` 实时，`0x22` 预设写入。
    ///
    /// 连同实际发出的 [`Frame`] 一起返回：调用方要打印 tx，而线上帧头里的
    /// `flags` 与 `seq` 只有在这里才确定（`seq` 由本方法分配，`client` 侧递增，
    /// 调用方无从预知）。此前 tx 只能打成 `type=0xNN payload[..]`，那两个字节
    /// 在输出里是看不见的。
    pub fn transact_with_flags(
        &mut self,
        flags: u8,
        kind: u8,
        payload: Vec<u8>,
        expect: &[u8],
    ) -> Result<(Frame, Frame), Error<T::Error>> {
        let seq = self.next_seq();
        let tx = Frame::new(Direction::Tx, flags, seq, kind, payload);
        let wire = tx.encode();

        let started = Instant::now();
        let mut timeout = self.config.initial_timeout;
        let mut attempts = 0u32;

        for attempt in 0..=self.config.max_retries {
            self.transport.send(&wire).map_err(Error::Transport)?;
            attempts = attempt + 1;
            let attempt_start = Instant::now();

            loop {
                if let Some(f) = self.take_matching(seq, expect) {
                    return Ok((tx, f));
                }
                if started.elapsed() >= self.config.hard_limit {
                    break;
                }
                if attempt_start.elapsed() >= timeout {
                    break;
                }
                let arrived = self.pump().map_err(Error::Transport)?;
                if arrived.is_empty() {
                    std::thread::sleep(self.config.poll_interval);
                }
            }

            if attempt == self.config.max_retries || started.elapsed() >= self.config.hard_limit {
                break;
            }
            timeout = (timeout * 2).min(self.config.max_timeout);
        }

        Err(Error::Timeout { attempts, elapsed: started.elapsed() })
    }

    /// [`Nanocore::transact_expect`]，接受任意应答类型。
    pub fn transact(&mut self, kind: u8, payload: Vec<u8>) -> Result<Frame, Error<T::Error>> {
        self.transact_expect(kind, payload, &[])
    }

    /// 移除并返回所有从未被消费的帧 —— 主动（非请求）的事件，
    /// 或在没有匹配请求的情况下到达的应答。可用于诊断超时。
    pub fn drain_pending(&mut self) -> Vec<Frame> {
        self.pending.drain(..).collect()
    }

    /// 在传输层上发送原始字节，**不带** SysEx 帧包装。
    ///
    /// 用于标准 MIDI 控制面（Control Change / Program Change），设备将其作为裸 3 字节包接受。
    /// 请先确保传输层处于非 SysEx 模式 ——
    /// 例如对 [`crate::midi::MidiTransport`] 调用 `self.transport().set_sysex(false)`。
    pub fn send_raw(&mut self, bytes: &[u8]) -> Result<(), T::Error> {
        self.transport.send(bytes)
    }

    // ─── 带校验的便捷命令 ────────────

    /// Ping 并返回设备应答（连通性检查）。
    pub fn ping_checked(&mut self) -> Result<Frame, Error<T::Error>> {
        let (k, p) = protocol::ping();
        self.transact_expect(k, p, &[crate::msg_type::PING])
    }

    /// 选择预设并等待确认。
    pub fn select_preset_checked(&mut self, index: u8) -> Result<Frame, Error<T::Error>> {
        let (k, p) = protocol::select_preset(index);
        self.transact(k, p)
    }

    /// 请求快照并返回设备应答帧。
    pub fn read_snapshot(&mut self) -> Result<Frame, Error<T::Error>> {
        let (k, p) = protocol::request_snapshot();
        self.transact_expect(k, p, &[crate::msg_type::SNAPSHOT])
    }

    /// 查询单个音箱 / 驱动槽位的模型名称并返回应答。
    ///
    /// 只读 —— 发送 `0x36` 查询并等待 `0x36` 应答。
    /// 应答载荷由 [`crate::protocol::parse_model_name`] 解析。
    /// 箱体 / IR 槽位由不同的消息枚举，此处不涵盖。
    pub fn query_model(&mut self, index: u8) -> Result<Frame, Error<T::Error>> {
        let (k, p) = protocol::model_query(index);
        self.transact_expect(k, p, &[crate::msg_type::MODEL_LIST])
    }

    /// 使用来自 [`crate::asset::read_plan`] 的读取计划读取完整的音箱模型或 IR 数据块。
    ///
    /// 只读。逐个发送数据块请求，并按 `addr` 升序拼接解码后的应答（设备会乱序应答）。
    ///
    /// 每收到一个应答就调用一次 `on_chunk(done, total)`，以便调用方显示进度；不需要时传入一个空闭包即可。
    ///
    /// 读取计划作为参数传入而非在此推导，
    /// 这样未知区域或槽位会被调用方*在任何 MIDI 发出之前*拒绝，
    /// 而不是表现为一个虚假的传输层错误。
    pub fn read_asset<F>(
        &mut self,
        plan: &[crate::asset::ChunkRequest],
        total: u32,
        mut on_chunk: F,
    ) -> Result<Vec<u8>, Error<T::Error>>
    where
        F: FnMut(usize, usize),
    {
        let n = plan.len();
        let mut chunks = Vec::with_capacity(n);
        for (i, req) in plan.iter().enumerate() {
            let reply = self.transact_expect(req.region, req.payload(), &[req.region])?;
            if let Some(c) = crate::asset::parse_chunk(&reply.payload) {
                chunks.push(c);
            }
            on_chunk(i + 1, n);
        }
        let blob = crate::asset::assemble(&chunks);
        // 计划是按标准数据块大小构建的；裁掉多余的余量。
        Ok(if blob.len() > total as usize {
            blob[..total as usize].to_vec()
        } else {
            blob
        })
    }

    /// 将 IR 数据块写回槽位，并通过重新读取进行验证。
    ///
    /// 针对单个槽位重放应用的导入流程：
    /// `0x56`（查询名称）→ PING → `0x49`（会话）→ PING → `0x50`（同步）→
    /// `0x51`（开始）→ `0x52`×N（数据）→ `0x53`（哈希）→ `0x55`（名称）→
    /// `0x57`（提交）→ 完整回读。
    ///
    /// 恒等哈希**由 `blob` 派生**（`identity_hash`）：设备在提交时重新计算它，不匹配时静默拒绝。
    /// `0x53` 帧的 `B3` 字节由 blob 派生（它承载 CRC 被掩掉的最高位 —— 见 `asset::write_hash`）。
    /// 前导步骤尽力而为：设备的写入会话跨连接保留，会话已打开时 `0x49`/`0x50` 将得不到应答。
    ///
    /// 返回回读的字节，以便调用方与 `blob` 比较。
    pub fn write_ir_slot(
        &mut self,
        slot: u8,
        blob: &[u8],
        name_override: Option<&str>,
        mut on_progress: impl FnMut(&str),
    ) -> Result<Vec<u8>, String> {
        // Flash 操作很慢：设备应答会话/提交帧的时间可能远超默认的 1 s 预算
        // （观察到：一个 0x49 ACK 在整个 1 s 预算耗尽之后才到达）。
        // 整个写入流程使用宽松的策略。
        let saved = self.config;
        self.set_config(crate::client::RetryConfig {
            initial_timeout: Duration::from_millis(500),
            max_timeout: Duration::from_millis(2000),
            hard_limit: Duration::from_secs(10),
            max_retries: 2,
            poll_interval: Duration::from_millis(10),
        });
        let result =
            self.write_ir_slot_inner(slot, blob, name_override, &mut on_progress);
        self.set_config(saved);
        result
    }

    fn write_ir_slot_inner(
        &mut self,
        slot: u8,
        blob: &[u8],
        name_override: Option<&str>,
        on_progress: &mut dyn FnMut(&str),
    ) -> Result<Vec<u8>, String> {
        use crate::asset;

        // 1. 首先查询槽位名称 —— 趁设备仍处于正常（读取友好）模式。
        //    写入会话一旦打开，设备就只接受写入流程。
        //    恒等哈希不会被复用：设备在提交时从 blob 重新计算它，
        //    不匹配时静默拒绝，因此它必须由被写入的内容派生
        //    （抓包中 16/16 验证通过）。
        let mut name: Vec<u8>;
        match self.transact_expect(
            asset::SLOT_QUERY,
            asset::slot_query(slot),
            &[asset::SLOT_QUERY],
        ) {
            Ok(reply) => {
                let (_old_hash, n) = asset::parse_slot_query(&reply.payload)
                    .ok_or_else(|| {
                        "0x56 reply did not parse as a slot identity".to_string()
                    })
                    .map_err(|e| e.to_string())?;
                name = n;
            }
            Err(_) => {
                // 会话中途设备可能不应答 0x56；
                // 没有显式覆盖名称时我们无法保留旧名称。
                name = match name_override {
                    Some(n) => n.as_bytes().to_vec(),
                    None => return Err("slot query failed and no --name given".into()),
                };
            }
        }
        if let Some(n) = name_override {
            name = n.as_bytes().to_vec();
        }
        on_progress(&format!("slot name {:?}", String::from_utf8_lossy(&name)));

        // 2. 会话打开，抓包顺序：PING -> 0x49 -> PING -> 0x50。
        //    0x49 和 0x50 的应答都回显我们的 seq。
        //    但设备的写入会话跨连接保留：会话已打开时 0x49 会被忽略（无应答），
        //    且一旦打开，设备会直接跳到写入流程。
        //    所以每个前导步骤都是尽力而为；真正起作用的是数据/提交帧。
        let (pk, pp) = crate::protocol::ping();
        on_progress("ping 1");
        self.transact_expect(pk, pp.clone(), &[crate::msg_type::PING])
            .map_err(|e| e.to_string())?;
        on_progress("session open (0x49)");
        let session_ok = self
            .transact_expect(
                asset::SESSION_OPEN,
                asset::session_open(),
                &[asset::SESSION_OPEN],
            )
            .is_ok();
        if session_ok {
            on_progress("session opened");
            // 在应用的抓包中，0x49 之后的 PING 会得到回显，
            // 但设备一旦进入写入模式就可能只应答自己的心跳
            // （flags=0x02，自己的 seq）—— 尽力而为，绝不视为致命错误。
            let _ = self.transact_expect(pk, pp, &[crate::msg_type::PING]);
        } else {
            on_progress("session already open (0x49 ignored)");
        }
        on_progress("sync (0x50)");
        if self
            .transact_expect(0x50, vec![0x00, 0x00, 0x00], &[0x50])
            .is_ok()
        {
            on_progress("synced");
        } else {
            on_progress("sync unanswered (continuing)");
        }

        // 3. 开始。如果设备正处于上一次中断写入的中间状态，
        //    它会忽略 0x51 并等待数据续传 ——
        //    所以这里的超时可以容忍，结果由数据帧决定。
        let total = blob.len() as u32;
        on_progress("begin");
        if self
            .transact_expect(
                asset::WRITE_BEGIN,
                asset::write_begin(slot, total),
                &[asset::WRITE_BEGIN],
            )
            .is_ok()
        {
            on_progress("begin acked");
        } else {
            on_progress("begin unanswered (continuing)");
        }

        // 4. 数据帧。
        let n_chunks = blob.len().div_ceil(asset::WRITE_CHUNK_BYTES);
        for (ci, chunk) in blob.chunks(asset::WRITE_CHUNK_BYTES).enumerate() {
            self.transact_expect(
                asset::WRITE_IR,
                asset::encode_data_frame(chunk, ci, slot),
                &[asset::WRITE_IR],
            ).map_err(|e| {
                on_progress(&format!("FAILED at chunk {}/{}", ci + 1, n_chunks));
                e.to_string()
            })?;
            on_progress(&format!("chunk {}/{}", ci + 1, n_chunks));
        }

        // 5. 哈希（由 blob 派生），6. 名称，7. 提交。
        on_progress("hash");
        self.transact_expect(
            asset::WRITE_HASH,
            asset::write_hash(slot, total, blob),
            &[asset::WRITE_HASH],
        ).map_err(|e| e.to_string())?;
        on_progress("name");
        let name_bytes = asset::encode_name(&name);
        self.transact_expect(
            asset::WRITE_NAME,
            asset::write_name(slot, &name_bytes),
            &[asset::WRITE_NAME],
        ).map_err(|e| e.to_string())?;
        self.transact_expect(
            asset::WRITE_COMMIT,
            asset::write_commit(slot),
            &[asset::WRITE_COMMIT],
        )
        .map_err(|e| e.to_string())?;

        // 应用会在提交与其回读之间重新枚举槽位表；flash 写入不会立即可见。
        // 重新查询槽位，并给设备留出时间后再回读。
        let _ = self.transact_expect(
            asset::SLOT_QUERY,
            asset::slot_query(slot),
            &[asset::SLOT_QUERY],
        );
        // flash 写入是异步落定的：立即回读可能读到部分过期的数据
        // （观察到：提交后数秒，blob 中约 2.4 s 处的一个过期字节范围）。
        // 留出时间，并重试回读直到匹配或预算耗尽。
        std::thread::sleep(std::time::Duration::from_millis(1200));


        // 8. 回读并返回。整个读取重试几次：
        //    提交后立即读取时设备可能返回部分过期的数据。
        let (total, plan) = asset::read_plan(asset::REGION_IR, slot)
            .ok_or_else(|| "no read plan for that slot".to_string()).map_err(|e| e.to_string())?;
        let mut back = Vec::new();
        for attempt in 0..3 {
            let mut chunks = Vec::new();
            for req in &plan {
                let reply = self.transact_expect(req.region, req.payload(), &[req.region]).map_err(|e| e.to_string())?;
                if let Some(c) = asset::parse_chunk(&reply.payload) {
                    chunks.push(c);
                }
            }
            back = asset::assemble(&chunks);
            if back.len() > total as usize {
                back.truncate(total as usize);
            }
            if back == blob {
                if attempt > 0 {
                    on_progress(&format!("read-back stable on attempt {}", attempt + 1));
                }
                break;
            }
            on_progress(&format!("read-back stale (attempt {}), waiting…", attempt + 1));
            std::thread::sleep(std::time::Duration::from_millis(1200));
        }
        Ok(back)
    }

    /// 将 amp 模型（完整 `.ead`，12302 B）写入设备 amp 槽位，并回读校验 crc32。
    ///
    /// 会话时序逐帧对齐官方"导入 RESONA"抓包（[`crate::amp_write`] 模块文档）：
    /// PING → `0x30` → `0x31`(长度+槽) → `0x32`×N(数据) → `0x33`(提交+crc)
    /// → `0x35`(模型名) → `0x30` → `0x37`(收尾) → `0x30` → `0x36`×38(型号表刷新)。
    /// 无 IR 写的 `0x49`/`0x50` 会话门控。
    ///
    /// * 数据帧编码直接复用 [`crate::asset::encode_data_frame`]（`0x52`/`0x32` 同构，
    ///   已用两份抓包共 7520 帧全量验证）。
    /// * `0x33` 提交帧完全可推导（38/38 出厂会话逐字节验证，见 [`crate::amp_write`]）。
    /// * `name_override` 为新模型名；缺省时先查询当前槽位名并沿用
    ///   （官方 App 在导入时把新模型名发给 `0x35`，设备以此更新型号表）。
    ///
    /// 与 IR 写一样：写会话有清空预设名表等副作用（见 README），失败后用
    /// `preset restore-factory` 恢复。
    ///
    /// 返回回读的剥壳负载（12242 B），调用方与 `ead[60..]` 比较。
    pub fn write_amp_slot(
        &mut self,
        slot: u8,
        ead: &[u8],
        name_override: Option<&str>,
        mut on_progress: impl FnMut(&str),
    ) -> Result<Vec<u8>, String> {
        use crate::amp_write as aw;

        assert_eq!(ead.len(), aw::EAD_TOTAL, "amp blob must be a full .ead");

        // 宽松预算（同 write_ir_slot）：会话/提交应答可能迟到。
        let saved = self.config;
        self.set_config(crate::client::RetryConfig {
            initial_timeout: Duration::from_millis(500),
            max_timeout: Duration::from_millis(2000),
            hard_limit: Duration::from_secs(10),
            max_retries: 2,
            poll_interval: Duration::from_millis(10),
        });
        let result = self.write_amp_slot_inner(slot, ead, name_override, &mut on_progress);
        self.set_config(saved);
        result
    }

    fn write_amp_slot_inner(
        &mut self,
        slot: u8,
        ead: &[u8],
        name_override: Option<&str>,
        on_progress: &mut dyn FnMut(&str),
    ) -> Result<Vec<u8>, String> {
        use crate::amp_write as aw;
        use crate::asset;
        use crate::msg_type;

        // 1. 先趁设备还在正常（读取友好）模式查当前模型名，作为缺省新名。
        let name: String = match name_override {
            Some(n) => n.to_string(),
            None => {
                let (k, p) = crate::protocol::model_query(slot);
                match self.transact_expect(k, p, &[msg_type::MODEL_LIST]) {
                    Ok(reply) => crate::protocol::parse_model_name(&reply.payload)
                        .map(|m| m.name)
                        .ok_or_else(|| "0x36 reply did not parse as a model name".to_string())?,
                    Err(_) => {
                        return Err(
                            "model name query failed and no --name given".to_string()
                        )
                    }
                }
            }
        };
        on_progress(&format!("model name {name:?}"));

        // 2. PING → 0x30（会话开始，尽力而为）。
        let (pk, pp) = crate::protocol::ping();
        self.transact_expect(pk, pp.clone(), &[msg_type::PING])
            .map_err(|e| e.to_string())?;
        on_progress("xfer begin (0x30)");
        if let Ok(ack) =
            self.transact_expect(aw::XFER_BEGIN, aw::xfer_begin(), &[aw::XFER_BEGIN])
        {
            on_progress(&format!("0x30 ack {}", hex(&ack.payload)));
        } else {
            on_progress("0x30 unanswered");
        }

        // 3. 0x31 长度 + 槽位。
        on_progress("xfer header (0x31)");
        let ack = self
            .transact_expect(aw::XFER_HEADER, aw::xfer_header(slot, ead.len()), &[aw::XFER_HEADER])
            .map_err(|e| format!("0x31 header not acked: {e}"))?;
        on_progress(&format!("0x31 ack {}", hex(&ack.payload)));

        // 4. 数据帧：64 B/帧，与 IR 写同构。
        let n_chunks = ead.len().div_ceil(asset::WRITE_CHUNK_BYTES);
        for (ci, chunk) in ead.chunks(asset::WRITE_CHUNK_BYTES).enumerate() {
            let reply = self.transact_expect(
                aw::XFER_DATA,
                asset::encode_data_frame(chunk, ci, slot),
                &[aw::XFER_DATA],
            )
            .map_err(|e| {
                on_progress(&format!("FAILED at chunk {}/{}", ci + 1, n_chunks));
                format!("0x32 data frame {ci} rejected: {e}")
            })?;
            if ci < 2 || reply.payload.get(1) != Some(&0x00) {
                on_progress(&format!("0x32 ack[{ci}] {}", hex(&reply.payload)));
            }
            if (ci + 1) % 16 == 0 || ci + 1 == n_chunks {
                on_progress(&format!("chunk {}/{}", ci + 1, n_chunks));
            }
        }

        // 5. 0x33 提交（crc32 尾部，38/38 验证）。状态字节非 0 = 设备拒收
        //    （数据不完整 / crc 不符）——立即报错，不要继续。
        on_progress("xfer commit (0x33)");
        let ack = self
            .transact_expect(aw::XFER_COMMIT, aw::xfer_commit(slot, ead.len(), ead), &[aw::XFER_COMMIT])
            .map_err(|e| format!("0x33 commit not acked: {e}"))?;
        on_progress(&format!("0x33 ack {}", hex(&ack.payload)));
        if ack.payload.get(1) != Some(&0x00) {
            return Err(format!(
                "device rejected the commit (status {:02x}): written data did not \
                 verify; the slot content is unchanged",
                ack.payload[1]
            ));
        }

        // 6. 0x35 模型名。5 字节头；设备以状态字节报告是否接受。
        on_progress("model name (0x35)");
        let ack = self
            .transact_expect(
                aw::XFER_META,
                aw::xfer_meta(slot, &aw::encode_model_name(&name)),
                &[aw::XFER_META],
            )
            .map_err(|e| format!("0x35 name not acked: {e}"))?;
        on_progress(&format!("0x35 ack {}", hex(&ack.payload)));
        if ack.payload.get(1) != Some(&0x00) {
            return Err(format!(
                "device rejected the model name (status {:02x})",
                ack.payload[1]
            ));
        }

        // 7. 0x30 → 0x37 → 0x30 收尾节拍（时序对齐官方抓包）。
        //    会话在 0x33+0x35 后已完结：官方流程里 0x37 得到 00，实测我们这边
        //    回 01（空会话上的收尾空操作）——无害，尽力而为即可。
        let _ = self.transact_expect(aw::XFER_BEGIN, aw::xfer_begin(), &[aw::XFER_BEGIN]);
        on_progress("xfer finalize (0x37)");
        if let Ok(ack) =
            self.transact_expect(aw::XFER_FINALIZE, aw::xfer_finalize(slot), &[aw::XFER_FINALIZE])
        {
            on_progress(&format!("0x37 ack {}", hex(&ack.payload)));
        }
        let _ = self.transact_expect(aw::XFER_BEGIN, aw::xfer_begin(), &[aw::XFER_BEGIN]);

        // 8. 验证：0x36 型号名应为新名字（设备内部把模型转为 DDBP 格式存储，
        //    `asset read` 读回的是内部格式而非写入的 .ead 负载，因此不能用
        //    字节比对验证）。flash 落盘异步生效，重试直到名字更新。
        std::thread::sleep(Duration::from_millis(1500));
        let (k, p) = crate::protocol::model_query(slot);
        for attempt in 0..4 {
            if let Ok(reply) = self.transact_expect(k, p.clone(), &[msg_type::MODEL_LIST])
                && let Some(m) = crate::protocol::parse_model_name(&reply.payload) {
                    if m.name == name {
                        on_progress(&format!("model table updated to {name:?}"));
                        break;
                    }
                    if attempt == 3 {
                        return Err(format!(
                            "model table still shows {:?} after write",
                            m.name
                        ));
                    }
                }
            on_progress(&format!("name stale (attempt {}), waiting…", attempt + 1));
            std::thread::sleep(Duration::from_millis(1500));
        }

        // 9. 回读（信息性）：设备内部 DDBP 格式，无法与 .ead 负载逐字节比对，
        //    仅供调用方展示 crc 变化。
        let (_total, plan) = asset::read_plan(asset::REGION_AMP, slot)
            .ok_or_else(|| "no read plan for that amp slot".to_string())?;
        let mut chunks = Vec::new();
        for req in &plan {
            let reply = self
                .transact_expect(req.region, req.payload(), &[req.region])
                .map_err(|e| e.to_string())?;
            if let Some(c) = asset::parse_chunk(&reply.payload) {
                chunks.push(c);
            }
        }
        on_progress("done");
        Ok(asset::assemble(&chunks))
    }

    /// 查询全部 38 个 amp/drive 型号槽，返回被标记为"活动"的槽位
    /// （即当前预设引用的 amp 型号）。
    ///
    /// 恰好一个活动槽才返回 `Some`；0 个或多个（含个别槽位查询失败）返回
    /// `None`，调用方应回退到显式指定。
    pub fn read_active_amp_slot(&mut self) -> Result<Option<u8>, String> {
        let mut active = Vec::new();
        for idx in 0..38u8 {
            let (k, p) = protocol::model_query(idx);
            let reply = self
                .transact_expect(k, p, &[crate::msg_type::MODEL_LIST])
                .map_err(|e| e.to_string())?;
            if let Some(m) = protocol::parse_model_name(&reply.payload)
                && m.active {
                    active.push(m.index);
                }
        }
        Ok(match active.as_slice() {
            [only] => Some(*only),
            _ => None,
        })
    }

    /// 查询全部 30 个 IR 槽（= CAB 型号），返回被标记为「活动」的那个。
    ///
    /// 2026-10-03 真机实测：走 [`crate::preset_restore::preamble_frames`] 的
    /// 前置序列时，其中第 43–72 帧恰好就是 `0x56` 逐槽查询（槽 0..29），每条
    /// 都返回 39 字节，第 7 字节**恰好一个为 `0x01`** —— 那个就是当前预设引用的
    /// CAB 型号。所以这里**复用那 30 条应答，不再发第二轮查询**：第二轮会让
    /// 设备连续吃 60 条 `0x56`，实测随后自己的查询全部落空。
    ///
    /// 与 [`Self::read_active_amp_slot`] 对称：amp 走 `0x36`（38 项，含 FX2
    /// 踏板），cab 走 `0x56`（30 项 IR）。两者都在第 7 字节给出 active 标志。
    pub fn read_active_ir_slot(&mut self) -> Result<Option<crate::asset::IrSlot>, String> {
        let mut active = Vec::new();
        for f in crate::preset_restore::preamble_frames()
            .map_err(|e| e.to_string())?
            .iter()
            // 只重放定位 / 同步这几帧，加上那 30 条 `0x56` 查询；中间的 38 条
            // `0x36`（amp 型号）与 8 条 `0x40`（预设名表）跳过 —— 它们不参与
            // CAB 读取，各省下几十次往返。
            .filter(|f| matches!(f.kind, 0x46 | 0x68 | 0x30 | 0x50))
        {
            // 定位 / 同步帧的应答内容无意义，但设备仍会回；发完即弃。
            let _ = self.transact(f.kind, f.payload.clone());
        }
        for idx in 0..30u8 {
            let reply = self
                .transact_expect(
                    crate::asset::SLOT_QUERY,
                    crate::asset::slot_query(idx),
                    &[crate::asset::SLOT_QUERY],
                )
                .map_err(|e| e.to_string())?;
            if let Some(s) = crate::asset::parse_ir_slot(&reply.payload)
                && s.active
            {
                active.push(s);
            }
        }
        Ok(match active.as_slice() {
            [only] => Some(only.clone()),
            // 0 个或多个：设备状态异常或前置没生效。调用方应回退到「不显示」，
            // 不能猜一个 —— 上一版把恒 0 的 `variant` 印成 `type=1` 就是这么错的。
            _ => None,
        })
    }

    /// 导入若干预设（动态构造 0x43 写组，前置枚举 + 官方节奏重放）。
    ///
    /// 输入为 `(目标槽位, 预设内容)` 对；`0x42`/`0x44` 取官方 restore 数据
    /// 同槽位帧（见 [`crate::preset_restore::build_write_group`]）。前置枚举
    /// 实测是连续提交的前置条件（省略时设备在 ~22-24 组后拒绝提交）。
    pub fn import_presets(
        &mut self,
        presets: &[(u8, crate::preset43::PresetWrite)],
        mut on_progress: impl FnMut(&str),
    ) -> Result<(), String> {
        let saved = self.config;
        self.set_config(crate::client::RetryConfig {
            initial_timeout: Duration::from_millis(500),
            max_timeout: Duration::from_millis(2000),
            hard_limit: Duration::from_secs(10),
            max_retries: 2,
            poll_interval: Duration::from_millis(10),
        });
        let result = self.import_presets_inner(presets, &mut on_progress);
        self.set_config(saved);
        result
    }

    fn import_presets_inner(
        &mut self,
        presets: &[(u8, crate::preset43::PresetWrite)],
        on_progress: &mut dyn FnMut(&str),
    ) -> Result<(), String> {
        use crate::preset_restore as pr;

        // 前置枚举（与 restore 相同；读取事务尽力而为，0x49 进入写会话）。
        let preamble = pr::preamble_frames()?;
        on_progress(&format!("replaying official preamble ({} frames)", preamble.len()));
        for f in &preamble {
            let _ = self.transact_with_flags(f.flags, f.kind, f.payload.clone(), &[f.kind]);
        }

        // 写流：以官方 restore 流为模板（PING 位置 / flags 模式原样保留），
        // 仅替换目标组的 0x43 载荷——合成节奏实测会让设备落表错乱。
        let stream = pr::build_import_stream(presets)
            .map_err(|e| format!("building import stream: {e}"))?;
        on_progress(&format!(
            "replaying {} write frames in {} groups",
            stream.iter().filter(|f| f.kind != pr::KIND_PING).count(),
            presets.len()
        ));

        // 逐组重放（组重放幂等，失败重做一次），0x46 用长等待。
        let mut ack_log: Vec<String> = Vec::new();
        let mut group_frames: Vec<&pr::PresetRestoreFrame> = Vec::new();
        let mut groups: Vec<Vec<&pr::PresetRestoreFrame>> = Vec::new();
        for f in &stream {
            group_frames.push(f);
            if f.kind == pr::KIND_SLOT_SEEK {
                groups.push(std::mem::take(&mut group_frames));
            }
        }
        for (gi, group) in groups.iter().enumerate() {
            let slot = group.last().and_then(|f| f.payload.last().copied()).unwrap_or(0) as usize;
            let mut attempt = 0usize;
            loop {
                attempt += 1;
                match self.replay_group(group, slot, &mut ack_log, on_progress) {
                    Ok(()) => break,
                    Err(e) if attempt < 2 => {
                        on_progress(&format!("group {gi}: {e} — redo group"));
                    }
                    Err(e) => {
                        return Err(format!(
                            "group {gi}: giving up after {attempt} attempts: {e}\n                             ack log: {ack_log:?}"
                        ));
                    }
                }
            }
        }
        on_progress(&format!("all {} groups replayed", groups.len()));
        // 调试统计：0x43 应答分布（status 字节非 0 = 设备拒收该帧）。
        let mut stats: std::collections::BTreeMap<String, usize> = Default::default();
        for l in &ack_log {
            if let Some((g, rest)) = l.split_once(":0x43/") {
                let _ = g;
                stats.entry(rest.to_string()).and_modify(|c| *c += 1).or_insert(1);
            }
        }
        for (shape, count) in &stats {
            on_progress(&format!("0x43 ack shape {shape} x{count}"));
        }
        Ok(())
    }

    /// 重放官方"仅 DSP 恢复出厂"流程：前置枚举 + 40 组预设写（名字 + 内容，槽位 0–39）。
    ///
    /// 帧序列来自官方 App 抓包（[`crate::preset_restore`]），跨会话逐字节稳定。
    /// 前置枚举（型号 / IR / 名表）实测是写入阶段能一次完成 40 组提交的前置条件。
    /// 开会话（`0x49`）会清空预设表——这正是流程随后要整体重写的对象。
    ///
    /// 注意：只覆盖预设；资产槽（amp/IR）不在此流程内。
    pub fn restore_factory_presets(
        &mut self,
        mut on_progress: impl FnMut(&str),
    ) -> Result<(), String> {
        use crate::preset_restore as pr;

        let frames = pr::frames()?;
        pr::validate(&frames).map_err(|e| format!("internal: {e}"))?;

        // 与 write_ir_slot 相同的宽松预算（会话/落槽应答可能迟到）。
        let saved = self.config;
        self.set_config(crate::client::RetryConfig {
            initial_timeout: Duration::from_millis(500),
            max_timeout: Duration::from_millis(2000),
            hard_limit: Duration::from_secs(10),
            max_retries: 2,
            poll_interval: Duration::from_millis(10),
        });
        let result = self.restore_factory_presets_inner(&frames, &mut on_progress);
        self.set_config(saved);
        result
    }

    fn restore_factory_presets_inner(
        &mut self,
        frames: &[crate::preset_restore::PresetRestoreFrame],
        on_progress: &mut dyn FnMut(&str),
    ) -> Result<(), String> {
        use crate::preset_restore as pr;

        // 前置枚举阶段：官方"仅 DSP 恢复"的 0x49 之前的全部 TX 逐帧重放
        // （0x46 定位、PING、XFER_BEGIN、型号/IR/名表枚举、快照、查询、0x49、PING）。
        // 读取事务尽力而为；`0x49` 之后设备即进入写会话（会清空预设表，随后整体重写）。
        let preamble = pr::preamble_frames()?;
        on_progress(&format!("replaying official preamble ({} frames)", preamble.len()));
        for (i, f) in preamble.iter().enumerate() {
            let label = match f.kind {
                pr::KIND_SLOT_SEEK => "slot-seek (reset)",
                pr::KIND_PING => "ping",
                0x36 => "model enumeration",
                0x56 => "ir-slot enumeration",
                0x63 => "snapshot",
                0x40 => "preset-list read",
                0x41 => "query",
                0x30 => "xfer-begin",
                0x49 => "session open (0x49)",
                0x50 => "sync",
                _ => "frame",
            };
            on_progress(&format!("preamble {}/{}: {}", i + 1, preamble.len(), label));
            let _ = self.transact_with_flags(f.flags, f.kind, f.payload.clone(), &[f.kind]);
        }

        // 按组切分（PING 原位保留在所属片内），逐组重放。
        let slices = pr::group_slices(frames).map_err(|e| format!("internal: {e}"))?;
        let loose = self.config;
        let mut ack_log: Vec<String> = Vec::with_capacity(frames.len());

        for (gi, slice) in slices.iter().enumerate() {
            let slot = gi; // 0-based，与片内 0x46 的末字节一致
            let mut attempt = 0usize;
            loop {
                attempt += 1;
                let outcome = self.replay_group(slice, slot, &mut ack_log, on_progress);
                match outcome {
                    Ok(()) => break,
                    Err(e) if attempt < 2 => {
                        // 组内一次性失败：重做本组（组重放幂等）。不重发 0x49——
                        // 它会清空整张表，把已提交的组毁掉。
                        on_progress(&format!("group {}: {} — redo group", slot + 1, e));
                    }
                    Err(e) => {
                        return Err(format!(
                            "group {}: giving up after {attempt} attempts: {e}\n                             full ack log (frame:kind/payload): {ack_log:?}",
                            slot + 1
                        ));
                    }
                }
            }
        }
        self.set_config(loose);
        on_progress("all 40 groups replayed");
        Ok(())
    }

    /// 重放一个组（含片内 PING 与落槽等待）。失败时返回 `Err`，由调用方决定重试。
    fn replay_group(
        &mut self,
        slice: &[&crate::preset_restore::PresetRestoreFrame],
        slot: usize,
        ack_log: &mut Vec<String>,
        on_progress: &mut dyn FnMut(&str),
    ) -> Result<(), String> {
        use crate::preset_restore::KIND_SLOT_SEEK;

        for (ri, f) in slice.iter().enumerate() {
            on_progress(&format!(
                "group {}/40 frame {}/{} (kind 0x{:02x})",
                slot + 1,
                ri + 1,
                slice.len(),
                f.kind
            ));

            if f.kind == crate::preset_restore::KIND_PING {
                let _ = self.transact_with_flags(f.flags, f.kind, f.payload.clone(), &[f.kind]);
                continue;
            }

            if f.kind == KIND_SLOT_SEEK {
                // 提交可能因后台重建被吞：长等待 + 重发，最多 3 轮。
                let mut ack = None;
                for round in 0..3 {
                    let seq = self.next_seq();
                    let wire =
                        Frame::new(Direction::Tx, f.flags, seq, f.kind, f.payload.clone());
                    self.transport
                        .send(&wire.encode())
                        .map_err(|e| format!("slot-seek send failed: {e}"))?;
                    match self.wait_for_slot_seek_ack(slot, 20_000) {
                        Ok(a) => {
                            ack = Some(a);
                            break;
                        }
                        Err(e) => {
                            on_progress(&format!(
                                "group {}: slot-seek attempt {} failed ({}), waiting 30s",
                                slot + 1,
                                round + 1,
                                e
                            ));
                            std::thread::sleep(Duration::from_millis(30_000));
                        }
                    }
                }
                let ack = ack.ok_or_else(|| {
                    format!("slot-seek ack for group {} never arrived", slot + 1)
                })?;
                let hex: String = ack
                    .payload
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                ack_log.push(format!("g{slot}:0x46/{hex} (settled)"));
                // flash 提交后的沉淀时间。提交触发的扇区擦除窗口内，设备会
                // ack 但丢弃后续 0x43（内容静默丢失、下一组内容占位——
                // 2026-10-01 实测 7/40 组确定性复现）；400ms 不够，2s 全量通过。
                std::thread::sleep(Duration::from_millis(2000));
                continue;
            }

            let (_tx, ack) = self
                .transact_with_flags(f.flags, f.kind, f.payload.clone(), &[f.kind])
                .map_err(|e| format!("kind 0x{:02x}: {e}", f.kind))?;
            let hex: String = ack
                .payload
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join(" ");
            ack_log.push(format!("g{slot}:0x{:02x}/{hex}", f.kind));
            // 0x44 之后的 0x43 特别容易在 flash 提交竞争中丢失（ack 正常但
            // 内容不落表，2026-10-01 实测 7/40 组），加长节拍缓解。
            let settle = if f.kind == crate::preset_restore::KIND_BULK_DATA {
                Duration::from_millis(150)
            } else {
                Duration::from_millis(60)
            };
            std::thread::sleep(settle);
        }
        Ok(())
    }

    /// 轮询等待落槽（`0x46`）应答中槽位字节追上 `slot`（0-based）。
    ///
    /// 设备的 flash 提交是异步的，应答会滞后若干个组；期间到达的旧槽位 ack
    /// 会被丢弃。seq 不作匹配依据——滞后场景下设备应答的 seq 与请求对不上。
    fn wait_for_slot_seek_ack(
        &mut self,
        slot: usize,
        timeout_ms: u64,
    ) -> Result<Frame, String> {
        let want = [0x00u8, 0x00, 0x01, 0x00, 0x00, slot as u8];
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            if Instant::now() >= deadline {
                return Err(format!("no slot-seek ack for slot {slot} within {timeout_ms} ms"));
            }
            if let Ok(arrived) = self.pump() {
                // pump 已把新帧放入 pending，下面统一在 pending 里找。
                let _ = arrived;
            }
            if let Some(pos) = self
                .pending
                .iter()
                .position(|f| f.kind == crate::preset_restore::KIND_SLOT_SEEK)
                && let Some(f) = self.pending.remove(pos)
                && f.payload == want
            {
                return Ok(f);
                // 旧槽位的迟到 ack：丢弃并继续等。
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }


    /// 轮询等待设备的预设切换事件（`0x01`，载荷 `[00, slot]`）。
    ///
    /// field-0x09 选槽是异步的：设备完成切换后会自发一条 `0x01` 事件
    /// （官方改名抓包：选槽 41 后事件载荷 `00 29`）。
    /// 旧槽位的迟到事件会被丢弃。
    pub fn wait_for_preset_switch_event(
        &mut self,
        slot: u8,
        timeout_ms: u64,
    ) -> Result<Frame, String> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            if Instant::now() >= deadline {
                return Err(format!(
                    "no preset-switch event for slot {slot} within {timeout_ms} ms"
                ));
            }
            if let Ok(_arrived) = self.pump() {
                // pump 已把新帧放入 pending，下面统一在 pending 里找。
            }
            if let Some(pos) = self
                .pending
                .iter()
                .position(|f| f.kind == 0x01 && f.payload == [0x00, slot])
            {
                if let Some(f) = self.pending.remove(pos) {
                    return Ok(f);
                }
                continue;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// 抽干 `ms` 毫秒内的所有到达帧（含先前积压的），返回可读摘要。
    /// 目前仅用于人工诊断（失败路径的手动排查），常态编译为死代码。
    #[allow(dead_code)]
    pub fn drain_frames(&mut self, ms: u64) -> String {
        let deadline = Instant::now() + Duration::from_millis(ms);
        let mut all: Vec<Frame> = self.pending.drain(..).collect();
        while Instant::now() < deadline {
            if let Ok(frames) = self.pump() {
                all.extend(frames);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if all.is_empty() {
            return "(nothing arrived)".to_string();
        }
        all.iter()
            .map(|f| {
                let hex: String =
                    f.payload.iter().take(12).map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ");
                format!(
                    "[{:?} type=0x{:02x} seq=0x{:04x} flags=0x{:02x} len={}] {}",
                    f.direction, f.kind, f.seq, f.flags, f.payload.len(), hex
                )
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// 读取**单个**预设槽位（0–63）的名称。
    ///
    /// 只需要名称时用这个，别用 [`Self::read_preset_names`]：后者为覆盖全部
    /// 64 个槽位要发 8 轮 `0x40`，而这里 1 轮就够——设备按 8 槽一批返回，
    /// 所以查询槽位所在的那一批（`slot / 8 * 8`）即可，`count` 仍是 8。
    /// 代价是返回的 [`protocol::PresetEntry`] 里 `slot` 是**批次起始**槽位
    /// 而非请求的那个，需自行按 `slot` 字段定位。
    pub fn read_preset_name(
        &mut self,
        slot: u8,
    ) -> Result<Option<protocol::PresetEntry>, String> {
        let base = slot / 8 * 8;
        let (k, p) = protocol::query_preset_list(base, 8);
        let reply = self
            .transact_expect(k, p, &[crate::msg_type::PRESET_LIST])
            .map_err(|e| e.to_string())?;
        let entries = protocol::parse_preset_list(&reply.payload)
            .ok_or_else(|| format!("preset-list reply for slots {base}.. did not decode"))?;
        Ok(entries.into_iter().find(|e| e.slot == slot))
    }

    /// 读取全部预设槽位（0–63）的名称。
    ///
    /// 设备对一次 `0x40` 预设列表查询每条应答返回 8 条固定大小的记录，
    /// 因此 8 次查询即可覆盖全部 64 个槽位。只需要当前预设的名称时用
    /// [`Self::read_preset_name`]（1 次往返）。
    /// 解析器是严格的：任何错位都会返回 `None`，整个读取失败，而不是产出移位的名称。
    pub fn read_preset_names(&mut self) -> Result<Vec<protocol::PresetEntry>, String> {
        let mut out = Vec::with_capacity(64);
        for base in (0..64).step_by(8) {
            let (k, p) = protocol::query_preset_list(base, 8);
            let reply = self
                .transact_expect(k, p, &[crate::msg_type::PRESET_LIST])
                .map_err(|e| e.to_string())?;
            let entries = protocol::parse_preset_list(&reply.payload)
                .ok_or_else(|| format!("preset-list reply for slots {base}.. did not decode"))?;
            out.extend(entries);
        }
        Ok(out)
    }

    // ─── 即发即忘的便捷命令 ──────────

    pub fn ping(&mut self) -> Result<(), T::Error> {
        let (k, p) = protocol::ping();
        self.send(k, p)
    }

    pub fn select_preset(&mut self, index: u8) -> Result<(), T::Error> {
        let (k, p) = protocol::select_preset(index);
        self.send(k, p)
    }

    pub fn select_cab(&mut self, index: u8) -> Result<(), T::Error> {
        let (k, p) = protocol::select_cab(index);
        self.send(k, p)
    }

    pub fn select_variant(&mut self, variant: u8) -> Result<(), T::Error> {
        let (k, p) = protocol::select_variant(variant);
        self.send(k, p)
    }

    pub fn set_enabled(&mut self, slot: u8, enabled: bool) -> Result<(), T::Error> {
        let (k, p) = protocol::set_enabled(slot, enabled);
        self.send(k, p)
    }

    pub fn set_usb_volume(&mut self, percent: u8) -> Result<(), T::Error> {
        let (k, p) = protocol::set_usb_volume(percent);
        self.send(k, p)
    }

    pub fn set_preset_volume(&mut self, percent: u8) -> Result<(), T::Error> {
        let (k, p) = protocol::set_preset_volume(percent);
        self.send(k, p)
    }

    /// 查询设置快照（`0x65`；设备也会每 ~250 ms 主动广播）。
    pub fn read_settings(&mut self) -> Result<protocol::DeviceSettings, String> {
        let (k, p) = protocol::query_settings();
        let reply = self
            .transact_expect(k, p, &[crate::msg_type::SETTINGS])
            .map_err(|e| e.to_string())?;
        protocol::decode_settings(&reply.payload)
            .ok_or_else(|| "0x65 settings snapshot did not decode".to_string())
    }

    /// 写一个设置页字段（`0x66`；字段 id 见 [`crate::setting`]）。
    pub fn set_setting(&mut self, setting_field: u8, value: u8) -> Result<(), T::Error> {
        let (k, p) = protocol::set_setting(setting_field, value);
        self.send(k, p)
    }

    pub fn set_name(&mut self, name: &[u8]) -> Result<(), T::Error> {
        let (k, p) = protocol::set_name(name);
        self.send(k, p)
    }

    pub fn load_effect(
        &mut self,
        slot: u8,
        effect_index: u8,
        model_flag: u8,
        params: &[f32],
    ) -> Result<(), T::Error> {
        let (k, p) = protocol::load_effect(slot, effect_index, model_flag, params);
        self.send(k, p)
    }

    pub fn request_snapshot(&mut self) -> Result<(), T::Error> {
        let (k, p) = protocol::request_snapshot();
        self.send(k, p)
    }

    /// 写入效果链序（`0x6D` sub `0x0A`，字段 `0x05`）。
    ///
    /// `order` 是从左到右的链，为 `0..8` 的一个排列，使用 [`protocol::CHAIN_ROLES`] 索引
    /// （`0=FX1 1=FX2 2=AMP 3=CAB 4=DEL 5=MOD 6=REV 7=EQ`）。
    /// 若 `order` 不是排列则 panic —— 调用前请先校验用户输入，
    /// 或使用 [`Nanocore::set_chain_order_checked`]。
    pub fn set_chain_order(&mut self, order: &[u8]) -> Result<(), T::Error> {
        let (k, p) = protocol::chain_write(order)
            .expect("chain order must contain each of 0..8 exactly once");
        self.send(k, p)
    }

    /// 写入效果链序（`0x6D` sub `0x0A`，字段 `0x05`）并验证。
    ///
    /// `order` 是从左到右的链，为 `0..8` 的一个排列，使用 [`protocol::CHAIN_ROLES`] 索引
    /// （`0=FX1 1=FX2 2=AMP 3=CAB 4=DEL 5=MOD 6=REV 7=EQ`）。
    ///
    /// 返回：
    /// * `Ok(None)` —— `order` 不是排列；未发送任何内容。
    /// * `Ok(Some(order))` —— 设备已确认。ACK 会把 9 字节链字段镜像回来（以不同的填充索引重新编码），
    ///   因此这是设备报告其接受的链序。
    ///
    /// 应用抓包中可见的主动（非请求）的 `0x09` 通知，在其出现时还会被额外采集一个短窗口；
    /// 但它携带设备自己的序列号，且**并非每个会话都会到达**，
    /// 因此绝不作为必要条件 —— ACK 才是可靠的回读。
    pub fn write_chain_order(&mut self, order: &[u8]) -> Result<Option<Vec<u8>>, Error<T::Error>> {
        let (kind, payload) = match protocol::chain_write(order) {
            Some(x) => x,
            None => return Ok(None),
        };
        let ack = self.transact(kind, payload)?;
        let echoed = protocol::parse_chain_ack(&ack.payload);

        // 可选：采集一条 `0x09` 通知（它绝不是被匹配的应答）。
        let deadline = Instant::now() + std::time::Duration::from_millis(300);
        loop {
            let mut frames = self.drain_pending();
            if let Ok(more) = self.receive() {
                frames.extend(more);
            }
            for f in frames {
                if f.seq != ack.seq
                    && f.kind == crate::msg_type::NOTIFY
                    && protocol::parse_chain_notify(&f.payload).is_some()
                {
                    return Ok(echoed);
                }
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(self.config.poll_interval);
        }
        Ok(echoed)
    }

    /// 带校验的 [`Nanocore::set_chain_order`]：当 `order` 不是有效排列时
    /// 返回 `Ok(false)` 而不是 panic。
    pub fn set_chain_order_checked(&mut self, order: &[u8]) -> Result<bool, T::Error> {
        match protocol::chain_write(order) {
            Some((k, p)) => self.send(k, p).map(|()| true),
            None => Ok(false),
        }
    }
}

/// `seq` 的下一个值：递增并保持低字节 7-bit 安全（≤ 0x7F，MIDI 数据字节约束）。
///
/// 低字节到达 0x80 时整段跳过到下一个 0x100 边界——官方抓包铁证：
/// seq `0x237f` 之后是 `0x2400`（跳过 `0x2380-0x23ff`）。不跳的话
/// （低字节 = 0x80+），设备将不再应答任何帧——这正是重放在 ~125 帧处
/// 必死的根因。
fn next_seq_after(s: u16) -> u16 {
    let mut next = (s + 1) & 0x7FFF;
    if next & 0xFF >= 0x80 {
        next = (next | 0xFF).wrapping_add(1) & 0x7FFF;
    }
    next
}

/// 载荷的小写十六进制（调试日志用）。
fn hex(payload: &[u8]) -> String {
    payload.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod seq_tests {
    use super::next_seq_after;

    #[test]
    fn seq_skips_the_non_7bit_safe_range() {
        assert_eq!(next_seq_after(0x237e), 0x237f);
        // 0x237f 的下一个不是 0x2380，而是跳到 0x2400
        assert_eq!(next_seq_after(0x237f), 0x2400);
        assert_eq!(next_seq_after(0x2400), 0x2401);
        // 每个低字节 0x7F 之后都跳过一个 0x80-0xFF 段
        assert_eq!(next_seq_after(0x007f), 0x0100);
        assert_eq!(next_seq_after(0x0000), 0x0001);
    }

    #[test]
    fn seq_stays_7bit_safe_forever() {
        let mut s = 0x237fu16;
        for _ in 0..70_000 {
            s = next_seq_after(s);
            assert!(s & 0xFF <= 0x7F, "seq {s:#06x} has a non-7-bit-safe low byte");
        }
    }
}
