// 6502 Model
// Cycle accurate model based on observing behavior of a real chip.
//
// The goal of this project is to perfectly recreate all the behavior of the chip,
// not necessarily implement it in the same way.
//
// Representing Signals in Rust:
// - Buses can be represented by the unsigned int of appropriate size.
// - Tri state pins are represented by Option<bool>, and None indicates floating / HighZ.
use std::collections::VecDeque;

mod trace_tests;

// Small internal instructions that perform the work for each
// cycle of a user-facing instruction.
//
// Many instructions have very similar behavior. By examining the cpu
// busses at each cycle, most instructions can be decomposed into a small
// set of simple micro operations.
#[derive(Clone, Copy, Debug)]
enum UOp {
    Nop,
    Fetch,
    ResetRegs,
    ReadPC{first: bool, addr: u16},
    Inc{reg: Register},
    Push{first: bool, val: Register},
    Dec{reg: Register},
    Read{src: Source, reg: Register},   // memory
    Write{dst: Source, val: Register},  // memory
    Set{dst: Register, src: Register},  // reg->reg transfers
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Register {
    Acc,
    X,
    Y,
    Sp,
    Status,
    // Fake scratch registers, used as work space for
    // uops.
    Scratch1,
    Scratch2,
}
#[derive(Clone, Copy, Debug)]
enum Flag {
    // bit 7 to 0
    Negative,
    Overflow,
    Five, // unused bit 5
    Break,
    Decimal,
    Interrupt, // aka irq disable
    Zero,
    Carry,
}

impl Flag {
    pub fn bit(&self) -> u8 {
        match self {
            Flag::Negative => 7,
            Flag::Overflow => 6,
            Flag::Five => 5,
            Flag::Break => 4,
            Flag::Decimal => 3,
            Flag::Interrupt => 2,
            Flag::Zero => 1,
            Flag::Carry => 0,
        }
    }
    pub fn apply(&self, status: &mut u8, val: bool) {
        if val {
            self.set(status);
        } else {
            self.clear(status);
        }
    }
    pub fn set(&self, status: &mut u8) {
        *status |= (1 << self.bit());
    }
    pub fn clear(&self, status: &mut u8) {
        *status &= !(1 << self.bit());
    }
}

#[derive(Clone, Copy, Debug)]
enum Source {
    // A direct address, known at the time of decoding the address.
    Address(u16),
    // RegVal allows uops to use the register value at the time of
    // usage, rather than when the opcode was initially decoded.
    // Consider a zero page instruction:
    // 1. read the operand, which holds a zero page address(u8)
    // 2. read memory based on the value read previously.
    // Step 2 would like to be able to use the result of #1. By reading 1
    // into a register, step 2 can use Source::RegVal as its input to use that value.
    RegVal(Register),
    // Stack, as stored in sp, is a single byte.
    // But the stack is in page 01, so the full stack address = 0x01 .. SP
    Stack,
    // Absolute addresses are built up during extended fetch, and based on temporary
    // data stored into Scratch1/Scratch2
    AddressAbsScratch,
}

pub struct W6502 {
    outputs: Outputs,
    prev_clk: bool,

    //
    // Internal Execution State
    // Analogs of these may or may not not exist in the real chip, but are important for managing
    // execution.
    //

    // Most instructions take several cycles. The queue
    // holds remaining steps for the last fetched
    // instruction.
    queue: VecDeque<UOp>,
    active_uop: UOp,

    //
    // Registers
    // These are real internal state documented in the chip.
    //
    pc: u16,
    acc: u8,
    x: u8,
    y: u8,
    sp: u8,       // The top of stack is 0x0100 + sp
    flags: u8,    // NZCIDV
    // scratch registers for uops
    scratch1: u8,
    scratch2: u8,
}

// Pins read by the 6502
#[derive(Clone, Copy)]
pub struct Inputs {
    pub clk: bool,
    pub n_reset: bool,    // active low reset
    pub data: u8,
}

// Pins set by the 6502.
pub struct Outputs {
    pub address: u16,
    pub data: Option<u8>,   // None if reading, Some if writing.
    pub rwb: bool,          // true for read, false for write
    pub sync: bool,         // true for the cycle of fetching the opcode byte.
}

impl Outputs {
    fn new() -> Outputs {
        Outputs {
            address: 0xFFFF,
            data: None,
            rwb: true,
            sync: false,
        }
    }
    fn zero(&mut self) {
        self.data = None;
        self.rwb = true;
    }
}

impl std::fmt::Debug for Outputs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        // match log format for easier diffing
        let addr = self.address;
        let rwb = self.rwb as usize;
        let sync = self.sync as usize;
        let data = if let Some(d) = self.data {
            format!("d=0x{d:02x} ")
        } else {
            String::new()
        };
        write!(f, "a=0x{addr:04x} rwb={rwb} {data}sync={sync}")
        // f.debug_struct("Outputs")
        //     .field("address", &addr)
        //     .field("data", &self.data)
        //     .field("rwb", &self.rwb)
        //     .field("sync", &self.sync)
        //     .finish()
    }
}

impl W6502 {
    pub fn new() -> W6502 {
        W6502 {
            outputs: Outputs::new(),
            prev_clk: false,
            queue: VecDeque::new(),
            active_uop: UOp::Nop,

            // "random" nonzero values before reset
            pc: 0xcafe,
            acc: 0xAA,
            flags: 0xFF,
            sp: 0xfc,
            x: 0xbc,
            y: 0xca,

            scratch1: 0,
            scratch2: 0,
        }
    }

    // Utility, lower and raise the clock for a given
    // input.
    pub fn cycle(&mut self, inputs: &Inputs) -> Result<(), String> {
        let mut inputs = inputs.clone();
        inputs.clk = false;
        self.tick(&inputs)?;
        inputs.clk = true;
        self.tick(&inputs)?;
        Ok(())
    }

    pub fn tick(&mut self, inputs: &Inputs) -> Result<(), String> {
        if !inputs.n_reset {
            // unspecified behavior for 6 cycles, then
            // read the reset vector, then set pc
            self.queue.clear();
            for i in 0 .. 5 {
                self.queue.push_back(UOp::Nop);
            }
            self.queue.push_back(UOp::ResetRegs);
            self.queue.push_back(UOp::ReadPC{first: true, addr: 0xFFFC});
            self.queue.push_back(UOp::ReadPC{first: false, addr: 0xFFFD});
            return Ok(());
        }

        let posedge =!self.prev_clk && inputs.clk;
        self.prev_clk = inputs.clk;
        // start a new uop each positive clock edge.
        let op = if posedge {
            if self.queue.len() > 0 {
                self.outputs.sync = false;
                self.queue.pop_front().unwrap()
            } else {
                // reset outputs
                self.outputs.zero();
                self.outputs.sync = true;
                UOp::Fetch
            }
        } else {
            self.active_uop
        };
        self.active_uop = op;

        println!("uop={op:?} c={}", posedge as u8);

        // Execute uops.
        match op {
            UOp::Nop => {
                // nop reads past the opcode while stalling.
                self.set_addr(self.pc);
            },
            UOp::Write{dst, val} => {
                let dst = self.source(dst);
                self.set_addr(dst);
                let val = *self.mut_reg(val);
                self.set_data(val);
            },
            UOp::Fetch => {
                if posedge {
                    self.set_addr(self.pc);
                } else {
                    self.decode_op(inputs.data)?;
                }
            },
            UOp::Inc {reg} => {
                if posedge {
                    let old = self.reg(reg);
                    *self.mut_reg(reg) = old.wrapping_add(1);
                    self.pc += 1;
                    self.set_addr(self.pc);
                }
            },
            UOp::Dec {reg} => {
                if posedge {
                    let old = self.reg(reg);
                    *self.mut_reg(reg) = old.wrapping_sub(1);
                    self.update_flags(&[Flag::Negative, Flag::Zero], self.reg(reg));
                    self.pc += 1;
                    self.set_addr(self.pc);
                }
            },
            UOp::Read{src, reg} => {
                if posedge {
                    let val = self.source(src);
                    self.set_addr(val);
                } else {
                    *self.mut_reg(reg) = inputs.data;
                }
            },
            UOp::ResetRegs => {
                // TODO: initialize registers for reset
                self.sp = 0xFD;
                self.flags = 0x37;
                // self.status = 0x1FD;
            },
            UOp::ReadPC{first, addr} => {
                if posedge {
                    self.set_addr(addr);
                } else {
                    if first {
                        self.pc = (self.pc & 0xFF00) | (inputs.data as u16);
                    } else {
                        self.pc = (self.pc & 0x00FF) | ((inputs.data as u16) << 8);
                    }
                }
            },
            UOp::Set{dst, src} => {
                if posedge {
                    // while updating, address points to next byte to fetch.
                    self.set_addr(self.pc);
                    *self.mut_reg(dst) = self.reg(src);
                }
            },
            UOp::Push{first, val} => {
                if posedge {
                    if first {
                        self.set_addr(self.pc);
                        self.scratch2 = self.sp.wrapping_sub(1);
                        self.scratch1 = self.reg(val);
                        if val == Register::Status {
                            // pushing status always sets bit 5 and brk
                            self.scratch1 |= (1<<5) | (1 << Flag::Break.bit());
                        }
                    } else {
                        let src = self.source(Source::Stack);
                        self.set_addr(src);
                        self.set_data(self.scratch1);
                        self.sp = self.scratch2;
                    }
                }
            },
        }

        Ok(())
    }
    pub fn outputs(&self) -> &Outputs {
        &self.outputs
    }

    pub fn update_flags(&mut self, flags: &[Flag], val: u8) {
        for flag in flags {
            match flag {
                Flag::Negative => {
                    flag.apply(&mut self.flags, val & 0x80 != 0);
                }
                Flag::Zero => {
                    flag.apply(&mut self.flags, val == 0);
                }
                x => { todo!("unimplemented flag: {flag:?}"); }
            }
        }

    }

    //
    // uop helpers
    // several opcodes reuse series of uops. These functions are
    // builders for these larger blocks, and each push multiple uops onto the queue.
    fn uops_push(&mut self, reg: Register) {
        // queue uops to push reg onto the stack.
        // 2 cycles for the instruction body
        // visible outputs:
        // 1. sync = 0, a = pc+1
        // 2. sync = 0, writing value to stack.
        // x. sync = 1, done with instr
        // While it isn't clear from watching the bus, we'll assume
        // 1 computes new sp, saves in scratch. and
        // 2 updates the register
        let mut q = |op: UOp| { self.queue.push_back(op); };
        q(UOp::Push{first: true, val: reg});
        q(UOp::Push{first: false, val: reg});

    }

    // decode_op is called at the end of a fetch, when the
    // cpu has just read the opcode for the next byte.
    //
    // This function is responsible for decoding the opcode byte,
    // and setting up the queue to execute the rest of the instruction.
    // After decoding, PC should point to the next instruction.
    fn decode_op(&mut self, opcode: u8) -> Result<(), String> {
        assert_eq!(0, self.queue.len());
        let mut q = |op: UOp| { self.queue.push_back(op); };
        // Note: the commented cycle counts include the cycle for 'fetch', as
        // this is consistent with the masswerk docs which were the main reference outside
        // of the actual chip.
        match opcode {
            0x08 => {
                // php. 1 byte 3 cycles.
                // push processor status, brk and bit 5 both 1
                self.uops_push(Register::Status);
                self.pc += 1;
            },
            0x48 => {
                // pha. 1 byte 3 cycles.
                self.uops_push(Register::Acc);
                self.pc += 1;
            },
            0x4C => {
                // jmp abs
                q(UOp::ReadPC{first: true, addr: self.pc+1});
                q(UOp::ReadPC{first: false, addr: self.pc+2});
                self.pc += 3;
            },
            0x84 => {
                // sty zpg
                q(UOp::Read{src: Source::Address(self.pc+1), reg: Register::Scratch1});
                q(UOp::Write{dst: Source::RegVal(Register::Scratch1), val: Register::Y});
                self.pc += 2;
            },
            0x85 => {
                // sta zpg
                q(UOp::Read{src: Source::Address(self.pc+1), reg: Register::Scratch1});
                q(UOp::Write{dst: Source::RegVal(Register::Scratch1), val: Register::Acc});
                self.pc += 2;
            },
            0x86 => {
                // stx zpg. 2 bytes 3 cycles.
                q(UOp::Read{src: Source::Address(self.pc+1), reg: Register::Scratch1});
                q(UOp::Write{dst: Source::RegVal(Register::Scratch1), val: Register::X});
                self.pc += 2;
            },
            0x8E => {
                // stx abs. 3 bytes 4 cycles
                q(UOp::Read{src: Source::Address(self.pc+1), reg: Register::Scratch1});
                q(UOp::Read{src: Source::Address(self.pc+2), reg: Register::Scratch2});
                // after reading the two byte dest, store.
                q(UOp::Write{dst: Source::AddressAbsScratch, val: Register::X});
                self.pc += 3;
            },
            0x9A => {
                // txs. 1 byte 2 cycles
                q(UOp::Set{dst: Register::Sp, src: Register::X});
                self.pc += 1;
            },
            0xA0 => {
                // ldy imm
                q(UOp::Read{src: Source::Address(self.pc+1), reg: Register::Y});
                self.pc += 2;
            },
            0xA2 => {
                // ldx immediate
                q(UOp::Read{src: Source::Address(self.pc+1), reg: Register::X});
                self.pc += 2;
            },
            0xA4 => {
                // ldy zpg
                q(UOp::Read{src: Source::Address(self.pc+1), reg: Register::Scratch1});
                q(UOp::Read{src: Source::RegVal(Register::Scratch1), reg: Register::Y});
                self.pc += 2;
            },
            0xA5 => {
                // lda zero page
                q(UOp::Read{src: Source::Address(self.pc+1), reg: Register::Acc});
                q(UOp::Read{src: Source::RegVal(Register::Acc), reg: Register::Acc});
                self.pc += 2;
            },
            0xA6 => {
                // ldx zero page
                q(UOp::Read{src: Source::Address(self.pc+1), reg: Register::X});
                q(UOp::Read{src: Source::RegVal(Register::X), reg: Register::X});
                self.pc += 2;
            },
            0xA9 => {
                // lda immediate
                q(UOp::Read{src: Source::Address(self.pc+1), reg: Register::Acc});
                self.pc += 2;
            },
            0xCA => {
                // dex ; decrease x. 1 byte 2 cycles
                q(UOp::Dec{reg: Register::X});
                // PC moved in instruction impl.
            },
            0xE8 => {
                // inx. 1 byte 2 cycles
                q(UOp::Inc{reg: Register::X});
                // PC moved in instruction impl.
            },
            0xEA => {
                q(UOp::Nop);
                // nop
                self.pc += 1;
            },
            _ => {
                return Err(format!("Unsupported opcode: 0x{opcode:2X}"));
            },
        }
        Ok(())
    }

    fn set_addr(&mut self, value: u16) {
        self.outputs.address = value;
    }
    fn set_data(&mut self, value: u8) {
        self.outputs.data = Some(value);
        self.outputs.rwb = false;
    }
    fn reg(&self, reg: Register) -> u8 {
        match reg {
            Register::Acc => self.acc,
            Register::Sp => self.sp,
            Register::Status => self.flags,
            Register::X => self.x,
            Register::Y => self.y,
            Register::Scratch1 => self.scratch1,
            Register::Scratch2 => self.scratch2,
        }
    }
    fn mut_reg(&mut self, reg: Register) -> &mut u8{
        match reg {
            Register::Acc => &mut self.acc,
            Register::Status => &mut self.flags,
            Register::Sp => &mut self.sp,
            Register::X => &mut self.x,
            Register::Y => &mut self.y,
            Register::Scratch1 => &mut self.scratch1,
            Register::Scratch2 => &mut self.scratch2,
        }
    }

    // Evaluate the source based on the current state of the cpu.
    fn source(&mut self, src: Source) -> u16 {
        match src {
            Source::Address(v) => v,
            Source::RegVal(reg) => *self.mut_reg(reg) as u16,
            Source::Stack => 0x0100 | (self.sp as u16),
            Source::AddressAbsScratch => ((self.scratch2 as u16) << 8) | self.scratch1 as u16,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    const OPCODE_NOP: u8 = 0xEA;

    #[test]
    fn test_reset() {
        // After clocking the chip with reset low, the chip will run for 6 cycles
        // before reading from the reset vector. The chip will then begin executing
        // from the address found.
        //
        // The standard trace tests ignore the trace before the reset vector read, since it is
        // varies based on the previous state of the chip. This is why reset needs a non-trace
        // test.
        // 
        // Reset involves clocking the chip with n_reset held low for two cycles. After 6 cycles,
        // the reset vector will be read from 0xFFFC and 0xFFFD, then the chip will execute
        // from that address.
        let mut cpu = W6502::new();
        const RESET_CYCLES : usize = 2;
        const PRE_VECTOR_CYCLES : usize = 6;

        let mut inputs = Inputs {
            data: 0xFF,
            n_reset: false,
            clk: false,
        };

        for i in 0 .. RESET_CYCLES {
            cpu.cycle(&inputs);
        }

        inputs.n_reset = true;
        // for the next 6 cycles, the cpu should be reading only.
        for i in 0 .. PRE_VECTOR_CYCLES {
            cpu.cycle(&inputs);
            assert_eq!(true, cpu.outputs().rwb);
        }

        // Then it should read the reset vector
        // Vector read 1
        cpu.cycle(&inputs).unwrap();
        assert_eq!(0xFFFC, cpu.outputs().address);
        inputs.data = 0xAD;

        // Vector read 2
        cpu.cycle(&inputs).unwrap();
        assert_eq!(0xFFFD, cpu.outputs().address);
        inputs.data = 0xDE;

        // start reading from target address. feed a few nops,
        // each 2 cycles.
        cpu.cycle(&inputs).unwrap();
        assert_eq!(0xDEAD, cpu.outputs().address);

        inputs.data = OPCODE_NOP;
        cpu.cycle(&inputs).unwrap();
        assert_eq!(0xDEAE, cpu.outputs().address);
        assert_eq!(false, cpu.outputs().sync);
        cpu.cycle(&inputs).unwrap();
        assert_eq!(0xDEAE, cpu.outputs().address);
        assert_eq!(true, cpu.outputs().sync);
        // finished first nop

        cpu.cycle(&inputs).unwrap();
        assert_eq!(0xDEAF, cpu.outputs().address);
        assert_eq!(false, cpu.outputs().sync);
        cpu.cycle(&inputs).unwrap();
        assert_eq!(0xDEAF, cpu.outputs().address);
        assert_eq!(true, cpu.outputs().sync);
        // finished second
    }
}
