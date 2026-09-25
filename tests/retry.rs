//! [`Nanocore::transact`] 的重试 / 超时行为，通过脚本化传输层离线快速运行。

use std::cell::RefCell;
use std::time::Duration;

use nanocore::Transport;
use nanocore::client::{Error, Nanocore, RetryConfig};
use nanocore::frame::{Direction, Frame};
use nanocore::protocol;

/// 只有在累计收到 `reply_after` 次发送之后才会应答的传输层。
///
/// 应答回显最近一次请求的序列号，与真实设备的行为一致。
struct Scripted {
    sent: RefCell<Vec<Vec<u8>>>,
    reply_after: usize,
    reply_kind: u8,
}

impl Scripted {
    fn never() -> Self {
        Self {
            sent: RefCell::new(vec![]),
            reply_after: usize::MAX,
            reply_kind: 0x07,
        }
    }
    fn after(n: usize, reply_kind: u8) -> Self {
        Self {
            sent: RefCell::new(vec![]),
            reply_after: n,
            reply_kind,
        }
    }
    fn sends(&self) -> usize {
        self.sent.borrow().len()
    }
}

impl Transport for Scripted {
    type Error = std::io::Error;

    fn send(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        self.sent.borrow_mut().push(data.to_vec());
        Ok(())
    }

    fn receive(&mut self) -> Result<Vec<u8>, Self::Error> {
        let sent = self.sent.borrow();
        if sent.len() <= self.reply_after {
            return Ok(vec![]);
        }
        let seq = sent
            .last()
            .and_then(|w| Frame::decode(w).ok())
            .map(|(f, _)| f.seq)
            .unwrap_or(0);
        let frame = Frame::new(Direction::Rx, 0x00, seq, self.reply_kind, vec![0x01, 0x02]);
        Ok(frame.encode())
    }
}

/// 带固定收件箱的传输层，用于检查杂散帧的处理方式。
struct Inbox(std::cell::RefCell<Vec<u8>>);

impl Inbox {
    fn with(frame: &Frame) -> Self {
        Self(std::cell::RefCell::new(frame.encode()))
    }
}

impl Transport for Inbox {
    type Error = std::io::Error;
    fn send(&mut self, _d: &[u8]) -> Result<(), Self::Error> {
        Ok(())
    }
    fn receive(&mut self) -> Result<Vec<u8>, Self::Error> {
        Ok(self.0.borrow_mut().drain(..).collect())
    }
}

fn fast_config(retries: u32) -> RetryConfig {
    RetryConfig {
        initial_timeout: Duration::from_millis(20),
        max_timeout: Duration::from_millis(40),
        hard_limit: Duration::from_millis(500),
        max_retries: retries,
        poll_interval: Duration::from_millis(2),
    }
}

#[test]
fn replies_immediately_on_first_attempt() {
    let t = Scripted::after(0, 0x68);
    let mut dev = Nanocore::with_config(t, fast_config(3));
    let (k, p) = protocol::ping();
    let reply = dev.transact_expect(k, p, &[0x68]).unwrap();
    assert_eq!(reply.kind, 0x68);
    assert_eq!(dev.into_inner().sends(), 1);
}

#[test]
fn retries_until_max_then_times_out() {
    let t = Scripted::never();
    let mut dev = Nanocore::with_config(t, fast_config(2));
    let (k, p) = protocol::ping();
    match dev.transact_expect(k, p, &[0x68]) {
        Err(Error::Timeout { attempts, .. }) => assert_eq!(attempts, 3),
        other => panic!("expected timeout, got {other:?}"),
    }
    // 1 次初始尝试 + 2 次重试。
    assert_eq!(dev.into_inner().sends(), 3);
}

#[test]
fn succeeds_on_a_later_attempt() {
    let t = Scripted::after(2, 0x68);
    let mut dev = Nanocore::with_config(t, fast_config(3));
    let (k, p) = protocol::ping();
    let reply = dev.transact_expect(k, p, &[0x68]).unwrap();
    assert_eq!(reply.kind, 0x68);
    assert_eq!(dev.into_inner().sends(), 3);
}

#[test]
fn wrong_reply_type_is_not_accepted() {
    // 设备以 0x65 应答，而我们等待的是 0x68 -> 继续等待。
    let t = Scripted::after(0, 0x65);
    let mut dev = Nanocore::with_config(t, fast_config(1));
    let (k, p) = protocol::ping();
    assert!(matches!(
        dev.transact_expect(k, p, &[0x68]),
        Err(Error::Timeout { .. })
    ));
}

#[test]
fn unsolicited_frames_with_foreign_seq_never_match() {
    // 设备主动上报的状态帧携带其自身的计数器；不得把它们误认为对我们请求的应答。
    let noise = Frame::new(Direction::Rx, 0x00, 0x0F49, 0x65, vec![0x00; 13]);
    let mut dev = Nanocore::with_config(Inbox::with(&noise), fast_config(0));
    let (k, p) = protocol::ping();
    // 我们的请求使用 seq 0，杂散帧为 0x0F49 -> 不匹配。
    assert!(matches!(
        dev.transact_expect(k, p, &[]),
        Err(Error::Timeout { .. })
    ));
    // ……但该帧被缓冲而非丢弃，因此事后仍可检查。
    let pending = dev.drain_pending();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].seq, 0x0F49);
}

#[test]
fn pending_frames_survive_until_consumed() {
    // 在我们读取之前到达的应答不得丢失。
    let expected = Frame::new(Direction::Rx, 0x00, 0, 0x68, vec![0xAA]);
    let mut dev = Nanocore::with_config(Inbox::with(&expected), fast_config(0));
    let (k, p) = protocol::ping();
    let reply = dev.transact_expect(k, p, &[0x68]).unwrap();
    assert_eq!(reply.payload, vec![0xAA]);
}
