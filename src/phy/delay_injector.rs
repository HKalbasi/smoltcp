use alloc::collections::VecDeque;

use crate::phy::{self, Device, DeviceCapabilities, PacketMeta};
use crate::time::{Duration, Instant};

const MTU: usize = 1536;

/// A fault injector device.
///
/// A fault injector is a device that alters packets traversing through it to simulate
/// adverse network conditions (such as random packet loss or corruption), or software
/// or hardware limitations (such as a limited number or size of usable network buffers).
#[derive(Debug)]
pub struct DelayInjector<D: Device> {
    inner: D,
    delay: Duration,
    rx_queue: VecDeque<(Vec<u8>, Instant, PacketMeta)>,
    tx_queue: VecDeque<(Vec<u8>, Instant)>,
}

impl<D: Device> DelayInjector<D> {
    /// Create a fault injector device, using the given random number generator seed.
    pub fn new(inner: D, delay: Duration) -> DelayInjector<D> {
        DelayInjector {
            inner,
            delay,
            rx_queue: VecDeque::new(),
            tx_queue: VecDeque::new(),
        }
    }

    /// Return the underlying device, consuming the fault injector.
    pub fn into_inner(self) -> D {
        self.inner
    }

    pub fn poll(&mut self, timestamp: Instant) {
        if let Some((rx_token, tx_token)) = self.inner.receive(timestamp) {
            let rx_meta = <D::RxToken<'_> as phy::RxToken>::meta(&rx_token);

            super::RxToken::consume(rx_token, |buffer| {
                self.rx_queue
                    .push_back((buffer.to_vec(), timestamp + self.delay, rx_meta));
            });
        }
        while let Some(front) = self.tx_queue.front() {
            if front.1 < timestamp {
                let (buf, _) = self.tx_queue.pop_front().unwrap();
                if buf.is_empty() {
                    continue;
                }
                if let Some(token) = self.inner.transmit(timestamp) {
                    <D::TxToken<'_> as phy::TxToken>::consume(token, buf.len(), |x| {
                        x[..buf.len()].copy_from_slice(&buf);
                    });
                }
            }
        }
    }
}

impl<D: Device> Device for DelayInjector<D> {
    type RxToken<'a> = RxToken
    where
        Self: 'a;
    type TxToken<'a> = TxToken<'a>
    where
        Self: 'a;

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = self.inner.capabilities();
        if caps.max_transmission_unit > MTU {
            caps.max_transmission_unit = MTU;
        }
        caps
    }

    fn receive(&mut self, timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let (_, recv_time, _) = self.rx_queue.front()?;
        if *recv_time > timestamp {
            return None;
        }
        let (buf, _, rx_meta) = self.rx_queue.pop_front().unwrap();

        let rx = RxToken { buf, meta: rx_meta };
        let tx = self.transmit(timestamp)?;
        Some((rx, tx))
    }

    fn transmit(&mut self, timestamp: Instant) -> Option<Self::TxToken<'_>> {
        self.tx_queue.push_back((vec![], timestamp));
        let buf = &mut self.tx_queue.back_mut().unwrap().0;
        Some(TxToken { buf })
    }
}

#[doc(hidden)]
pub struct RxToken {
    buf: Vec<u8>,
    meta: PacketMeta,
}

impl phy::RxToken for RxToken {
    fn consume<R, F>(mut self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        f(&mut self.buf)
    }

    fn meta(&self) -> phy::PacketMeta {
        self.meta
    }
}

#[doc(hidden)]
pub struct TxToken<'a> {
    buf: &'a mut Vec<u8>,
}

impl<'a> phy::TxToken for TxToken<'a> {
    fn consume<R, F>(mut self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        self.buf.extend(std::iter::repeat(0).take(len));
        f(&mut self.buf)
    }

    fn set_meta(&mut self, _meta: PacketMeta) {}
}
