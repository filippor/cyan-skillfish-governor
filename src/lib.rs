mod transport;
mod mailbox;
mod codec;
mod error;
mod api;

pub use error::{SmuError, Result};
pub use codec::{mv_to_vid, vid_to_mv};

use std::collections::HashMap;
use std::sync::Arc;
use transport::Bc250PciTransport;
use mailbox::{Bc250Mailbox, SmuStatus};

const DEFAULT_QUEUE_ADDRS: [(u8, (u32, u32, u32)); 5] = [
    (0, (0x03B10A08, 0x03B10A68, 0x03B10A48)),
    (1, (0x03B10A00, 0x03B10A60, 0x03B10A40)),
    (2, (0x03B10528, 0x03B10564, 0x03B10998)),
    (3, (0x03B10A20, 0x03B10A80, 0x03B10A88)),
    (4, (0x03B10A24, 0x03B10A84, 0x03B10A8C)),
];

pub struct Bc250Smu {
    allow_queue0: bool,
    #[allow(dead_code)]  // Used indirectly through Arc clones in mailboxes
    transport: Arc<Bc250PciTransport>,
    queues: HashMap<u8, Bc250Mailbox>,
}

impl Bc250Smu {
    pub fn new(
        bdf: &str,
        allow_queue0: bool,
        use_flock: bool,
        timeout: u32,
    ) -> Result<Self> {
        let mut transport = Bc250PciTransport::new(bdf, use_flock);
        transport.open()?;
        let transport = Arc::new(transport);

        let mut queues = HashMap::new();
        for (queue, (cmd, rsp, arg)) in DEFAULT_QUEUE_ADDRS {
            queues.insert(
                queue,
                Bc250Mailbox::new(&transport, cmd, rsp, arg, timeout),
            );
        }

        Ok(Self {
            allow_queue0,
            transport,
            queues,
        })
    }

    pub fn close(&mut self) {
        // Can't call close on Arc, but it will be cleaned up on drop
    }

    fn guard_queue(&self, queue: u8) -> Result<()> {
        if queue == 0 && !self.allow_queue0 {
            return Err(SmuError::Queue0Disabled);
        }
        Ok(())
    }

    fn get_queue(&self, queue: u8) -> Result<&Bc250Mailbox> {
        self.queues
            .get(&queue)
            .ok_or(SmuError::QueueNotConfigured(queue))
    }

    pub fn raw_send(&self, queue: u8, msg_id: u32, arg: u32, arg_high: Option<u32>) -> Result<SmuStatus> {
        self.guard_queue(queue)?;
        self.get_queue(queue)?.send(msg_id, arg, arg_high)
    }

    pub fn raw_read(&self, queue: u8) -> Result<u32> {
        self.guard_queue(queue)?;
        self.get_queue(queue)?.read_arg()
    }

    pub fn raw_read_high(&self, queue: u8) -> Result<u32> {
        self.guard_queue(queue)?;
        self.get_queue(queue)?.read_arg_high()
    }

    pub fn send_message(
        &self,
        queue_id: u8,
        msg_id: u32,
        arg: u32,
        arg_high: Option<u32>,
        pack: Option<fn(u32) -> u32>,
        decode: Option<fn(u32) -> u32>,
        check_status: bool,
    ) -> Result<u32> {
        let packed = pack.map(|f| f(arg)).unwrap_or(arg);
        let status = self.raw_send(queue_id, msg_id, packed, arg_high)?;

        if check_status && status != SmuStatus::Ok {
            return Err(SmuError::SmuStatus {
                status: status as u8,
                queue: queue_id,
                msg: msg_id as u8,
            });
        }

        if let Some(decode_fn) = decode {
            let value = self.raw_read(queue_id)?;
            Ok(decode_fn(value))
        } else {
            Ok(status as u32)
        }
    }
}
