use crate::stack::provisioned::ProvisionedStack;
use crate::{DriverError, Watchdog};
use btmesh_common::{InsufficientBuffer, SeqZero};
use btmesh_device::CompletionToken;
use btmesh_pdu::provisioned::lower::{BlockAck, InvalidBlock};
use btmesh_pdu::provisioned::upper::UpperPDU;
use embassy_executor::time::{Duration, Instant};
use heapless::Vec;

pub struct TransmitQueue<const N: usize = 5> {
    queue: Vec<Option<QueueEntry>, N>,
}

#[derive(Clone)]
enum QueueEntry {
    Nonsegmented(NonsegmentedQueueEntry),
    Segmented(SegmentedQueueEntry),
}

#[derive(Clone)]
struct NonsegmentedQueueEntry {
    upper_pdu: UpperPDU<ProvisionedStack>,
    num_retransmit: u8,
    completion_token: Option<CompletionToken>,
}

#[derive(Clone)]
struct SegmentedQueueEntry {
    upper_pdu: UpperPDU<ProvisionedStack>,
    acked: Acked,
    completion_token: Option<CompletionToken>,
}

impl<const N: usize> Default for TransmitQueue<N> {
    fn default() -> Self {
        let mut queue = Vec::new();
        queue.resize(N, None);
        Self { queue }
    }
}

impl<const N: usize> TransmitQueue<N> {
    pub fn add_segmented(
        &mut self,
        upper_pdu: UpperPDU<ProvisionedStack>,
        num_segments: u8,
        completion_token: Option<CompletionToken>,
        watchdog: &Watchdog,
    ) -> Result<(), InsufficientBuffer> {
        let slot = self.queue.iter_mut().find(|e| e.is_none());

        let seq_zero = upper_pdu.meta().seq().into();

        if let Some(slot) = slot {
            debug!("added to retransmit queue {}", seq_zero);
            slot.replace(QueueEntry::Segmented(SegmentedQueueEntry {
                upper_pdu,
                acked: Acked::new(seq_zero, num_segments),
                completion_token,
            }));
        } else {
            warn!("no space in retransmit queue");
        }

        for slot in self.queue.iter() {
            if let Some(slot) = slot {
                if let QueueEntry::Segmented(slot) = slot {
                    let timeout = Instant::now()
                        + Duration::from_millis(
                            200 + (50 * slot.upper_pdu.meta().ttl().value() as u64),
                        );
                    watchdog.outbound_expiration((timeout, slot.upper_pdu.meta().seq().into()));
                }
            }
        }

        Ok(())
    }

    pub fn add_nonsegmented(
        &mut self,
        upper_pdu: UpperPDU<ProvisionedStack>,
        num_retransmit: u8,
        completion_token: Option<CompletionToken>,
    ) -> Result<(), InsufficientBuffer> {
        let slot = self.queue.iter_mut().find(|e| e.is_none());

        if let Some(slot) = slot {
            slot.replace(QueueEntry::Nonsegmented(NonsegmentedQueueEntry {
                upper_pdu,
                num_retransmit,
                completion_token,
            }));
        } else {
            warn!("no space in retransmit queue");
        }

        Ok(())
    }

    pub fn iter(&mut self) -> impl Iterator<Item = UpperPDU<ProvisionedStack>> + '_ {
        QueueIter {
            inner: self.queue.iter_mut(),
        }
    }

    pub fn expire_outbound(&mut self, seq_zero: SeqZero) {
        for outer in self.queue.iter_mut() {
            if let Some(slot) = outer {
                if let QueueEntry::Segmented(entry) = slot {
                    if SeqZero::from(entry.upper_pdu.meta().seq()) == seq_zero {
                        outer.take();
                    }
                }
            }
        }
    }

    pub fn receive_ack(
        &mut self,
        block_ack: BlockAck,
        watchdog: &Watchdog,
    ) -> Result<(), DriverError> {
        if let Some(slot) = self.queue.iter_mut().find(|e| {
            if let Some(QueueEntry::Segmented(entry)) = e {
                let seq_zero: SeqZero = entry.upper_pdu.meta().seq().into();
                seq_zero == block_ack.seq_zero()
            } else {
                false
            }
        }) {
            if let Some(QueueEntry::Segmented(entry)) = slot {
                let fully_acked = entry.acked.ack(block_ack, watchdog)?;
                if fully_acked {
                    watchdog.clear_outbound_expiration(entry.upper_pdu.meta().seq().into());
                    entry.completion_token.as_ref().map(|token| {
                        token.complete();
                    });
                    slot.take();
                }
            }
        }

        for slot in self.queue.iter() {
            if let Some(slot) = slot {
                if let QueueEntry::Segmented(slot) = slot {
                    let timeout = Instant::now()
                        + Duration::from_millis(
                            200 + (50 * slot.upper_pdu.meta().ttl().value() as u64),
                        );
                    watchdog.outbound_expiration((timeout, slot.upper_pdu.meta().seq().into()));
                }
            }
        }
        Ok(())
    }
}

struct QueueIter<'i, I: Iterator<Item = &'i mut Option<QueueEntry>>> {
    inner: I,
}

impl<'i, I: Iterator<Item = &'i mut Option<QueueEntry>>> Iterator for QueueIter<'i, I> {
    type Item = UpperPDU<ProvisionedStack>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(outer) = self.inner.next() {
            let mut should_take = false;

            let result = if let Some(next) = outer {
                match next {
                    QueueEntry::Nonsegmented(inner) => {
                        inner.num_retransmit -= 1;
                        if inner.num_retransmit == 0 {
                            should_take = true;
                            inner
                                .completion_token
                                .as_ref()
                                .map(|token| token.complete());
                        }
                        Some(inner.upper_pdu.clone())
                    }
                    QueueEntry::Segmented(inner) => Some(inner.upper_pdu.clone()),
                }
            } else {
                None
            };

            if should_take {
                outer.take();
            }

            result
        } else {
            None
        }
    }
}

#[derive(Clone)]
struct Acked {
    num_segments: u8,
    block_ack: BlockAck,
}

impl Acked {
    fn new(seq_zero: SeqZero, num_segments: u8) -> Self {
        Self {
            num_segments,
            block_ack: BlockAck::new(seq_zero),
        }
    }

    fn ack(&mut self, block_ack: BlockAck, _watchdog: &Watchdog) -> Result<bool, InvalidBlock> {
        for ack in block_ack.acked_iter() {
            self.block_ack.ack(ack)?;
        }

        Ok(self.block_ack.is_fully_acked(self.num_segments))
    }
}
