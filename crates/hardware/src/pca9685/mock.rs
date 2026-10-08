//! Bounded, fault-injectable register model. No OS or physical bus access.
use super::{AI, ALL_OFF_H, FULL_OFF, LED0, MODE1, MODE2, PRE_SCALE, SLEEP, TransportError};
use std::collections::VecDeque;

pub(super) trait Transport {
    fn write(&mut self, address: u8, bytes: &[u8]) -> Result<(), TransportError>;
    fn read(&mut self, address: u8, register: u8, bytes: &mut [u8]) -> Result<(), TransportError>;
    fn set_inhibited(&mut self, inhibited: bool) -> Result<(), TransportError>;
    fn wait_us(&mut self, microseconds: u64) -> Result<(), TransportError>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectedFault {
    Fail,
    PartialWrite { accepted_data_bytes: usize },
    CorruptRead { byte: usize, value: u8 },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockEvent {
    Write {
        address: u8,
        bytes: [u8; 65],
        length: usize,
        accepted_data_bytes: usize,
        success: bool,
    },
    Read {
        address: u8,
        register: u8,
        length: usize,
        success: bool,
    },
    Inhibit {
        inhibited: bool,
        success: bool,
    },
    Wait {
        microseconds: u64,
        success: bool,
    },
}
pub const TRACE_CAPACITY: usize = 64;
pub struct MockTransport {
    address: u8,
    registers: [u8; 256],
    inhibited: bool,
    inhibit_available: bool,
    disconnected: bool,
    time_us: u64,
    transaction_latency_us: u64,
    oscillator_started_at: Option<u64>,
    operations: u64,
    fault: Option<(u64, InjectedFault)>,
    trace: VecDeque<MockEvent>,
}
impl MockTransport {
    pub(super) fn new(address: u8) -> Self {
        let mut registers = [0u8; 256];
        registers[usize::from(MODE1)] = SLEEP | 1;
        registers[usize::from(MODE2)] = 4;
        registers[usize::from(PRE_SCALE)] = 30;
        for channel in 0..16 {
            registers[usize::from(LED0) + channel * 4 + 3] = FULL_OFF;
        }
        Self {
            address,
            registers,
            inhibited: true,
            inhibit_available: true,
            disconnected: false,
            time_us: 0,
            transaction_latency_us: 0,
            oscillator_started_at: None,
            operations: 0,
            fault: None,
            trace: VecDeque::with_capacity(TRACE_CAPACITY),
        }
    }
    pub fn trace(&self) -> &VecDeque<MockEvent> {
        &self.trace
    }
    pub fn clear_trace(&mut self) {
        self.trace.clear();
    }
    pub fn operations(&self) -> u64 {
        self.operations
    }
    pub fn inhibited(&self) -> bool {
        self.inhibited
    }
    pub fn elapsed_us(&self) -> u64 {
        self.time_us
    }
    /// Simulated duration per I2C read/write. Pin inhibition remains instantaneous.
    pub fn set_transaction_latency_us(&mut self, microseconds: u64) {
        self.transaction_latency_us = microseconds;
    }
    pub fn register(&self, register: u8) -> u8 {
        self.registers[usize::from(register)]
    }
    /// Alter mock state to simulate another writer or device reset. Never touches hardware.
    pub fn corrupt_register(&mut self, register: u8, value: u8) {
        self.registers[usize::from(register)] = value;
    }
    pub fn disconnect(&mut self, disconnected: bool) {
        self.disconnected = disconnected;
    }
    pub fn set_inhibit_available(&mut self, available: bool) {
        self.inhibit_available = available;
    }
    /// Zero fails the next operation. This injection is consumed once.
    pub fn inject_after(&mut self, operations_before_failure: u64, fault: InjectedFault) {
        self.fault = Some((
            self.operations.saturating_add(operations_before_failure),
            fault,
        ));
    }
    pub fn output_ticks(&self, channel: u8) -> Option<u16> {
        if channel >= 16 || self.inhibited || self.register(MODE1) & SLEEP != 0 {
            return None;
        }
        let offset = usize::from(LED0) + usize::from(channel) * 4;
        if self.registers[offset + 3] & FULL_OFF != 0 {
            return None;
        }
        Some(
            u16::from(self.registers[offset + 2])
                | (u16::from(self.registers[offset + 3] & 15) << 8),
        )
    }
    fn next_fault(&mut self) -> Option<InjectedFault> {
        let index = self.operations;
        self.operations = self.operations.saturating_add(1);
        if self.fault.is_some_and(|(at, _)| at == index) {
            self.fault.take().map(|(_, fault)| fault)
        } else {
            None
        }
    }
    fn log(&mut self, event: MockEvent) {
        if self.trace.len() == TRACE_CAPACITY {
            self.trace.pop_front();
        }
        self.trace.push_back(event);
    }
    fn write_register(&mut self, register: u8, value: u8) -> Result<(), TransportError> {
        if register == PRE_SCALE && self.register(MODE1) & SLEEP == 0 {
            return Err(TransportError::InvalidTransaction);
        }
        if ((LED0..=0x45).contains(&register) || (0xfa..=ALL_OFF_H).contains(&register))
            && self.register(MODE1) & SLEEP == 0
            && self
                .oscillator_started_at
                .is_some_and(|start| self.time_us.saturating_sub(start) < 500)
        {
            return Err(TransportError::InvalidTransaction);
        }
        if register == MODE1 {
            if self.register(MODE1) & SLEEP != 0 && value & SLEEP == 0 {
                self.oscillator_started_at = Some(self.time_us);
            }
            if value & SLEEP != 0 {
                self.oscillator_started_at = None;
            }
            self.registers[0] = (value & 0x7f) | (self.registers[0] & 0x40);
        } else if (0xfa..=ALL_OFF_H).contains(&register) {
            let byte = usize::from(register - 0xfa);
            for channel in 0..16 {
                self.registers[usize::from(LED0) + channel * 4 + byte] = value;
            }
            self.registers[usize::from(register)] = 0;
        } else {
            self.registers[usize::from(register)] = value;
        }
        Ok(())
    }
}
impl Transport for MockTransport {
    fn write(&mut self, address: u8, bytes: &[u8]) -> Result<(), TransportError> {
        self.time_us = self
            .time_us
            .checked_add(self.transaction_latency_us)
            .ok_or(TransportError::InvalidTransaction)?;
        let fault = self.next_fault();
        let mut packet = [0u8; 65];
        let length = bytes.len().min(packet.len());
        packet[..length].copy_from_slice(&bytes[..length]);
        let mut accepted = 0;
        let result = (|| {
            if self.disconnected || address != self.address {
                return Err(TransportError::Disconnected);
            }
            if !(2..=65).contains(&bytes.len()) {
                return Err(TransportError::InvalidTransaction);
            }
            if matches!(
                fault,
                Some(InjectedFault::Fail | InjectedFault::CorruptRead { .. })
            ) {
                return Err(TransportError::Injected);
            }
            let limit = match fault {
                Some(InjectedFault::PartialWrite {
                    accepted_data_bytes,
                }) => accepted_data_bytes.min(bytes.len() - 1),
                _ => bytes.len() - 1,
            };
            let increment = self.register(MODE1) & AI != 0;
            for (offset, value) in bytes[1..].iter().take(limit).enumerate() {
                let register = if increment {
                    bytes[0]
                        .checked_add(offset as u8)
                        .ok_or(TransportError::InvalidTransaction)?
                } else {
                    bytes[0]
                };
                self.write_register(register, *value)?;
                accepted += 1;
            }
            if matches!(fault, Some(InjectedFault::PartialWrite { .. })) {
                return Err(TransportError::PartialWrite(accepted));
            }
            Ok(())
        })();
        self.log(MockEvent::Write {
            address,
            bytes: packet,
            length,
            accepted_data_bytes: accepted,
            success: result.is_ok(),
        });
        result
    }
    fn read(&mut self, address: u8, register: u8, bytes: &mut [u8]) -> Result<(), TransportError> {
        self.time_us = self
            .time_us
            .checked_add(self.transaction_latency_us)
            .ok_or(TransportError::InvalidTransaction)?;
        let fault = self.next_fault();
        let result = (|| {
            if self.disconnected || address != self.address {
                return Err(TransportError::Disconnected);
            }
            if matches!(
                fault,
                Some(InjectedFault::Fail | InjectedFault::PartialWrite { .. })
            ) {
                return Err(TransportError::Injected);
            }
            if bytes.is_empty() || bytes.len() > 64 || usize::from(register) + bytes.len() > 256 {
                return Err(TransportError::InvalidTransaction);
            }
            let increment = self.register(MODE1) & AI != 0;
            for (offset, byte) in bytes.iter_mut().enumerate() {
                *byte = self.registers[usize::from(register) + if increment { offset } else { 0 }];
            }
            if let Some(InjectedFault::CorruptRead { byte, value }) = fault {
                *bytes
                    .get_mut(byte)
                    .ok_or(TransportError::InvalidTransaction)? = value;
            }
            Ok(())
        })();
        self.log(MockEvent::Read {
            address,
            register,
            length: bytes.len(),
            success: result.is_ok(),
        });
        result
    }
    fn set_inhibited(&mut self, inhibited: bool) -> Result<(), TransportError> {
        let fault = self.next_fault();
        let success = fault.is_none() && self.inhibit_available;
        if success {
            self.inhibited = inhibited;
        }
        self.log(MockEvent::Inhibit { inhibited, success });
        if success {
            Ok(())
        } else {
            Err(TransportError::Injected)
        }
    }
    fn wait_us(&mut self, microseconds: u64) -> Result<(), TransportError> {
        let fault = self.next_fault();
        let end = self.time_us.checked_add(microseconds);
        let success = fault.is_none() && end.is_some();
        if success && let Some(end) = end {
            self.time_us = end;
        }
        self.log(MockEvent::Wait {
            microseconds,
            success,
        });
        if success {
            Ok(())
        } else {
            Err(TransportError::Injected)
        }
    }
}
