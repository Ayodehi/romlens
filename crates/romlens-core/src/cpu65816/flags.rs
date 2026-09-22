//! The processor state the decoder needs: the M, X and E flags decide operand
//! widths; DBR and D decide what an operand address means; the carry matters
//! only because `XCE` swaps it into E.

/// Flag state before or after an instruction. `None` means "not known
/// statically".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FlagState {
    /// Accumulator/memory width: `true` = 8-bit.
    pub m: bool,
    /// Index register width: `true` = 8-bit.
    pub x: bool,
    /// Emulation mode; forces 8-bit widths.
    pub e: bool,
    pub dbr: Option<u8>,
    pub dp: Option<u16>,
    pub c: Option<bool>,
}

impl FlagState {
    /// After a hardware reset: emulation mode, DBR = 0, D = 0.
    pub const RESET: FlagState = FlagState {
        m: true,
        x: true,
        e: true,
        dbr: Some(0),
        dp: Some(0),
        c: None,
    };

    /// Entering a native-mode interrupt handler: M = X = 1 is what the
    /// hardware sets on the way in; DBR and D are whatever the interrupted
    /// code had.
    pub const NATIVE_VECTOR: FlagState = FlagState {
        m: true,
        x: true,
        e: false,
        dbr: None,
        dp: None,
        c: None,
    };

    /// Entering an emulation-mode interrupt handler.
    pub const EMULATION_VECTOR: FlagState = FlagState {
        m: true,
        x: true,
        e: true,
        dbr: None,
        dp: None,
        c: None,
    };

    /// Effective accumulator width: emulation mode forces 8-bit.
    pub const fn eff_m(self) -> bool {
        self.m || self.e
    }

    /// Effective index width.
    pub const fn eff_x(self) -> bool {
        self.x || self.e
    }

    /// Only the two bits that change instruction lengths.
    pub const fn width_key(self) -> u8 {
        (self.eff_m() as u8) | ((self.eff_x() as u8) << 1)
    }

    /// bit0 m, bit1 x, bit2 e, bit3 dbr known, bit4 dp known.
    pub const fn packed(self) -> u8 {
        (self.m as u8)
            | ((self.x as u8) << 1)
            | ((self.e as u8) << 2)
            | ((self.dbr.is_some() as u8) << 3)
            | ((self.dp.is_some() as u8) << 4)
    }

    /// Parse `m1x0e0`, optionally followed by ` dbr=80 dp=0000` style suffixes
    /// (the CLI's `--flags`). Missing letters keep the `NATIVE_VECTOR` values.
    pub fn parse(text: &str) -> Option<FlagState> {
        let mut f = FlagState::NATIVE_VECTOR;
        let mut chars = text.trim().chars().peekable();
        while let Some(c) = chars.next() {
            let bit = match chars.next()? {
                '0' => false,
                '1' => true,
                _ => return None,
            };
            match c.to_ascii_lowercase() {
                'm' => f.m = bit,
                'x' => f.x = bit,
                'e' => f.e = bit,
                _ => return None,
            }
        }
        Some(f)
    }

    /// `m1x0e0` for the CLI's verbose column.
    pub fn short(self) -> String {
        format!("m{}x{}e{}", self.m as u8, self.x as u8, self.e as u8)
    }
}

impl std::fmt::Display for FlagState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "M={} X={} E={} DBR=",
            self.m as u8, self.x as u8, self.e as u8
        )?;
        match self.dbr {
            Some(b) => write!(f, "${b:02X}")?,
            None => f.write_str("?")?,
        }
        f.write_str(" DP=")?;
        match self.dp {
            Some(d) => write!(f, "${d:04X}"),
            None => f.write_str("?"),
        }
    }
}
