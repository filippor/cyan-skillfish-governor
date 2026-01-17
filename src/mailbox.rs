use std::sync::Mutex;
use crate::error::{Result, SmuError};
use crate::transport::Bc250PciTransport;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmuStatus {
    Ok = 0x01,
    Failed = 0xFF,
    UnknownCmd = 0xFE,
    RejectedPrereq = 0xFD,
    RejectedBusy = 0xFC,
}

impl SmuStatus {
    pub fn from_u32(value: u32) -> Option<Self> {
        match value as u8 {
            0x01 => Some(Self::Ok),
            0xFF => Some(Self::Failed),
            0xFE => Some(Self::UnknownCmd),
            0xFD => Some(Self::RejectedPrereq),
            0xFC => Some(Self::RejectedBusy),
            _ => None,
        }
    }
}

pub struct Bc250Mailbox {
    transport: *const Bc250PciTransport,
    cmd_addr: u32,
    rsp_addr: u32,
    arg_addr: u32,
    timeout: u32,
    lock: Mutex<()>,
}

// Safety: The transport pointer is guaranteed to outlive the mailbox
// because mailboxes are owned by Bc250Smu which owns the transport
unsafe impl Send for Bc250Mailbox {}
unsafe impl Sync for Bc250Mailbox {}

impl Bc250Mailbox {
    pub fn new(
        transport: &Bc250PciTransport,
        cmd_addr: u32,
        rsp_addr: u32,
        arg_addr: u32,
        timeout: u32,
    ) -> Self {
        Self {
            transport: transport as *const _,
            cmd_addr,
            rsp_addr,
            arg_addr,
            timeout,
            lock: Mutex::new(()),
        }
    }

    pub fn send(&self, msg_id: u32, arg: u32, arg_high: Option<u32>) -> Result<SmuStatus> {
        let _guard = self.lock.lock().unwrap();
        let transport = unsafe { &*self.transport };

        transport.write_smu_reg(self.rsp_addr, 0)?;
        transport.write_smu_reg(self.arg_addr, arg)?;
        transport.write_smu_reg(self.arg_addr + 4, arg_high.unwrap_or(0))?;
        transport.write_smu_reg(self.cmd_addr, msg_id)?;

        self.wait_done()
    }

    pub fn read_arg(&self) -> Result<u32> {
        let _guard = self.lock.lock().unwrap();
        let transport = unsafe { &*self.transport };
        transport.read_smu_reg(self.arg_addr)
    }

    pub fn read_arg_high(&self) -> Result<u32> {
        let _guard = self.lock.lock().unwrap();
        let transport = unsafe { &*self.transport };
        transport.read_smu_reg(self.arg_addr + 4)
    }

    fn wait_done(&self) -> Result<SmuStatus> {
        let transport = unsafe { &*self.transport };
        let mut remaining = self.timeout;

        while remaining > 0 {
            remaining -= 1;
            let status = transport.read_smu_reg(self.rsp_addr)?;
            
            if let Some(smu_status) = SmuStatus::from_u32(status) {
                return Ok(smu_status);
            }
        }

        Err(SmuError::Timeout)
    }
}
