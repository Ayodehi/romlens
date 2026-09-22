//! The 92 instruction mnemonics of the 65816 and what each one does, in one
//! line each, for the inspector.

/// Every 65816 mnemonic. `JML` covers the long jumps (`$5C`, `$DC`); `JMP`
/// the 16-bit ones (`$4C`, `$6C`, `$7C`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[allow(clippy::upper_case_acronyms)]
pub enum Mnemonic {
    ADC,
    AND,
    ASL,
    BCC,
    BCS,
    BEQ,
    BIT,
    BMI,
    BNE,
    BPL,
    BRA,
    BRK,
    BRL,
    BVC,
    BVS,
    CLC,
    CLD,
    CLI,
    CLV,
    CMP,
    COP,
    CPX,
    CPY,
    DEC,
    DEX,
    DEY,
    EOR,
    INC,
    INX,
    INY,
    JML,
    JMP,
    JSL,
    JSR,
    LDA,
    LDX,
    LDY,
    LSR,
    MVN,
    MVP,
    NOP,
    ORA,
    PEA,
    PEI,
    PER,
    PHA,
    PHB,
    PHD,
    PHK,
    PHP,
    PHX,
    PHY,
    PLA,
    PLB,
    PLD,
    PLP,
    PLX,
    PLY,
    REP,
    ROL,
    ROR,
    RTI,
    RTL,
    RTS,
    SBC,
    SEC,
    SED,
    SEI,
    SEP,
    STA,
    STP,
    STX,
    STY,
    STZ,
    TAX,
    TAY,
    TCD,
    TCS,
    TDC,
    TRB,
    TSB,
    TSC,
    TSX,
    TXA,
    TXS,
    TXY,
    TYA,
    TYX,
    WAI,
    WDM,
    XBA,
    XCE,
}

impl Mnemonic {
    pub const COUNT: usize = 92;

    pub const fn as_str(self) -> &'static str {
        use Mnemonic::*;
        match self {
            ADC => "ADC",
            AND => "AND",
            ASL => "ASL",
            BCC => "BCC",
            BCS => "BCS",
            BEQ => "BEQ",
            BIT => "BIT",
            BMI => "BMI",
            BNE => "BNE",
            BPL => "BPL",
            BRA => "BRA",
            BRK => "BRK",
            BRL => "BRL",
            BVC => "BVC",
            BVS => "BVS",
            CLC => "CLC",
            CLD => "CLD",
            CLI => "CLI",
            CLV => "CLV",
            CMP => "CMP",
            COP => "COP",
            CPX => "CPX",
            CPY => "CPY",
            DEC => "DEC",
            DEX => "DEX",
            DEY => "DEY",
            EOR => "EOR",
            INC => "INC",
            INX => "INX",
            INY => "INY",
            JML => "JML",
            JMP => "JMP",
            JSL => "JSL",
            JSR => "JSR",
            LDA => "LDA",
            LDX => "LDX",
            LDY => "LDY",
            LSR => "LSR",
            MVN => "MVN",
            MVP => "MVP",
            NOP => "NOP",
            ORA => "ORA",
            PEA => "PEA",
            PEI => "PEI",
            PER => "PER",
            PHA => "PHA",
            PHB => "PHB",
            PHD => "PHD",
            PHK => "PHK",
            PHP => "PHP",
            PHX => "PHX",
            PHY => "PHY",
            PLA => "PLA",
            PLB => "PLB",
            PLD => "PLD",
            PLP => "PLP",
            PLX => "PLX",
            PLY => "PLY",
            REP => "REP",
            ROL => "ROL",
            ROR => "ROR",
            RTI => "RTI",
            RTL => "RTL",
            RTS => "RTS",
            SBC => "SBC",
            SEC => "SEC",
            SED => "SED",
            SEI => "SEI",
            SEP => "SEP",
            STA => "STA",
            STP => "STP",
            STX => "STX",
            STY => "STY",
            STZ => "STZ",
            TAX => "TAX",
            TAY => "TAY",
            TCD => "TCD",
            TCS => "TCS",
            TDC => "TDC",
            TRB => "TRB",
            TSB => "TSB",
            TSC => "TSC",
            TSX => "TSX",
            TXA => "TXA",
            TXS => "TXS",
            TXY => "TXY",
            TYA => "TYA",
            TYX => "TYX",
            WAI => "WAI",
            WDM => "WDM",
            XBA => "XBA",
            XCE => "XCE",
        }
    }

    /// Parse a mnemonic, case-insensitively.
    pub fn parse(text: &str) -> Option<Self> {
        let upper = text.trim().to_ascii_uppercase();
        Self::all().into_iter().find(|m| m.as_str() == upper)
    }

    pub const fn all() -> [Mnemonic; Self::COUNT] {
        use Mnemonic::*;
        [
            ADC, AND, ASL, BCC, BCS, BEQ, BIT, BMI, BNE, BPL, BRA, BRK, BRL, BVC, BVS, CLC, CLD,
            CLI, CLV, CMP, COP, CPX, CPY, DEC, DEX, DEY, EOR, INC, INX, INY, JML, JMP, JSL, JSR,
            LDA, LDX, LDY, LSR, MVN, MVP, NOP, ORA, PEA, PEI, PER, PHA, PHB, PHD, PHK, PHP, PHX,
            PHY, PLA, PLB, PLD, PLP, PLX, PLY, REP, ROL, ROR, RTI, RTL, RTS, SBC, SEC, SED, SEI,
            SEP, STA, STP, STX, STY, STZ, TAX, TAY, TCD, TCS, TDC, TRB, TSB, TSC, TSX, TXA, TXS,
            TXY, TYA, TYX, WAI, WDM, XBA, XCE,
        ]
    }

    /// One line for the inspector.
    pub const fn describe(self) -> &'static str {
        use Mnemonic::*;
        match self {
            ADC => "Add with carry to the accumulator",
            AND => "AND memory with the accumulator",
            ASL => "Shift left one bit (memory or accumulator)",
            BCC => "Branch if carry clear",
            BCS => "Branch if carry set",
            BEQ => "Branch if equal (zero flag set)",
            BIT => "Test bits of memory against the accumulator",
            BMI => "Branch if minus (negative flag set)",
            BNE => "Branch if not equal (zero flag clear)",
            BPL => "Branch if plus (negative flag clear)",
            BRA => "Branch always",
            BRK => "Software interrupt: push state and jump through the BRK vector",
            BRL => "Branch always, 16-bit displacement",
            BVC => "Branch if overflow clear",
            BVS => "Branch if overflow set",
            CLC => "Clear the carry flag",
            CLD => "Clear the decimal flag",
            CLI => "Clear the interrupt-disable flag (enable IRQ)",
            CLV => "Clear the overflow flag",
            CMP => "Compare memory with the accumulator",
            COP => "Coprocessor interrupt: push state and jump through the COP vector",
            CPX => "Compare memory with X",
            CPY => "Compare memory with Y",
            DEC => "Decrement memory or the accumulator",
            DEX => "Decrement X",
            DEY => "Decrement Y",
            EOR => "Exclusive-OR memory with the accumulator",
            INC => "Increment memory or the accumulator",
            INX => "Increment X",
            INY => "Increment Y",
            JML => "Jump long: set the program bank and program counter",
            JMP => "Jump within the current program bank",
            JSL => "Jump to a subroutine long, pushing the 24-bit return address",
            JSR => "Jump to a subroutine, pushing the 16-bit return address",
            LDA => "Load the accumulator from memory",
            LDX => "Load X from memory",
            LDY => "Load Y from memory",
            LSR => "Logical shift right one bit (memory or accumulator)",
            MVN => "Block move, ascending addresses (A+1 bytes, X source, Y destination)",
            MVP => "Block move, descending addresses (A+1 bytes, X source, Y destination)",
            NOP => "No operation",
            ORA => "OR memory with the accumulator",
            PEA => "Push a 16-bit constant onto the stack",
            PEI => "Push the 16-bit value at a direct-page address",
            PER => "Push a program-counter-relative address",
            PHA => "Push the accumulator",
            PHB => "Push the data bank register",
            PHD => "Push the direct page register",
            PHK => "Push the program bank register",
            PHP => "Push the processor status",
            PHX => "Push X",
            PHY => "Push Y",
            PLA => "Pull the accumulator",
            PLB => "Pull the data bank register",
            PLD => "Pull the direct page register",
            PLP => "Pull the processor status (M and X may change)",
            PLX => "Pull X",
            PLY => "Pull Y",
            REP => "Reset status bits: #$20 makes A 16-bit, #$10 makes X and Y 16-bit",
            ROL => "Rotate left one bit through carry",
            ROR => "Rotate right one bit through carry",
            RTI => "Return from interrupt, restoring the status register",
            RTL => "Return from a long subroutine",
            RTS => "Return from a subroutine",
            SBC => "Subtract with borrow from the accumulator",
            SEC => "Set the carry flag",
            SED => "Set the decimal flag",
            SEI => "Set the interrupt-disable flag (disable IRQ)",
            SEP => "Set status bits: #$20 makes A 8-bit, #$10 makes X and Y 8-bit",
            STA => "Store the accumulator to memory",
            STP => "Stop the clock until reset",
            STX => "Store X to memory",
            STY => "Store Y to memory",
            STZ => "Store zero to memory",
            TAX => "Transfer the accumulator to X",
            TAY => "Transfer the accumulator to Y",
            TCD => "Transfer the 16-bit accumulator to the direct page register",
            TCS => "Transfer the 16-bit accumulator to the stack pointer",
            TDC => "Transfer the direct page register to the 16-bit accumulator",
            TRB => "Test and reset memory bits against the accumulator",
            TSB => "Test and set memory bits against the accumulator",
            TSC => "Transfer the stack pointer to the 16-bit accumulator",
            TSX => "Transfer the stack pointer to X",
            TXA => "Transfer X to the accumulator",
            TXS => "Transfer X to the stack pointer",
            TXY => "Transfer X to Y",
            TYA => "Transfer Y to the accumulator",
            TYX => "Transfer Y to X",
            WAI => "Wait for an interrupt",
            WDM => "Reserved opcode (two-byte no-op)",
            XBA => "Exchange the two bytes of the accumulator",
            XCE => "Exchange carry and emulation flags (switch native/emulation mode)",
        }
    }

    /// Control does not fall through to the next instruction.
    pub const fn is_block_end(self) -> bool {
        use Mnemonic::*;
        matches!(self, RTS | RTL | RTI | JMP | JML | BRA | BRL | BRK | STP)
    }

    /// Pushes a return address and continues after the callee returns.
    pub const fn is_call(self) -> bool {
        matches!(self, Mnemonic::JSR | Mnemonic::JSL)
    }

    /// Conditional branch: both the target and the fall-through are reachable.
    pub const fn is_branch(self) -> bool {
        use Mnemonic::*;
        matches!(self, BCC | BCS | BEQ | BMI | BNE | BPL | BVC | BVS)
    }

    /// Unconditional transfer that does not return (`JMP`, `JML`, `BRA`, `BRL`).
    pub const fn is_jump(self) -> bool {
        matches!(
            self,
            Mnemonic::JMP | Mnemonic::JML | Mnemonic::BRA | Mnemonic::BRL
        )
    }

    /// Writes memory at its operand address.
    pub const fn writes_memory(self) -> bool {
        use Mnemonic::*;
        matches!(self, STA | STX | STY | STZ)
    }

    /// Reads and writes memory at its operand address.
    pub const fn read_modify_writes(self) -> bool {
        use Mnemonic::*;
        matches!(self, ASL | LSR | ROL | ROR | INC | DEC | TRB | TSB)
    }
}

impl std::fmt::Display for Mnemonic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
