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
        self.seq = (self.seq + 1) & 0x7FFF;
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
        self.transact_with_flags(0x00, kind, payload, expect)
    }

    /// 与 [`Nanocore::transact_expect`] 相同，但显式指定 flags 字节。
    ///
    /// 观察到的 flags 值：`0x00` 普通，`0x02` 实时，`0x22` 预设写入。
    pub fn transact_with_flags(
        &mut self,
        flags: u8,
        kind: u8,
        payload: Vec<u8>,
        expect: &[u8],
    ) -> Result<Frame, Error<T::Error>> {
        let seq = self.next_seq();
        let wire = Frame::new(Direction::Tx, flags, seq, kind, payload).encode();

        let started = Instant::now();
        let mut timeout = self.config.initial_timeout;
        let mut attempts = 0u32;

        for attempt in 0..=self.config.max_retries {
            self.transport.send(&wire).map_err(Error::Transport)?;
            attempts = attempt + 1;
            let attempt_start = Instant::now();

            loop {
                if let Some(f) = self.take_matching(seq, expect) {
                    return Ok(f);
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

    /// 读取全部预设槽位（0–63）的名称。
    ///
    /// 设备对一次 `0x40` 预设列表查询每条应答返回 8 条固定大小的记录，
    /// 因此 8 次查询即可覆盖全部 64 个槽位。
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
                if f.seq != ack.seq && f.kind == crate::msg_type::NOTIFY {
                    if protocol::parse_chain_notify(&f.payload).is_some() {
                        return Ok(echoed);
                    }
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
