//! The glossary (docs/27): the SNES's acronyms and initialisms, each spelt
//! out with a sentence or two on what it is. The tutor's answers and
//! lessons link every term the first time it appears, and a click shows
//! its entry.
//!
//! The general terms are written here; the hardware and S-DSP registers
//! come from the register tables, so their words are the ones the listing
//! and the explanations use.

use crate::explain::{fields, sound};
use crate::model::hardware::all_hardware_registers;

/// What kind of term an entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// An acronym or piece of jargon.
    Term,
    /// A 65816-side hardware register (`$2100`–`$437F`).
    Register,
    /// An S-DSP register, reached through the SPC700's `$F2`/`$F3`.
    DspRegister,
}

/// One entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// As it is written: `DMA`, `VMAIN`.
    pub term: String,
    /// Other spellings that mean the same: `V-blank`, `vblank`.
    pub also: Vec<String>,
    /// The words it stands for.
    pub words: String,
    /// One or two sentences.
    pub about: String,
    pub kind: Kind,
}

struct Term {
    term: &'static str,
    also: &'static [&'static str],
    words: &'static str,
    about: &'static str,
}

const fn t(
    term: &'static str,
    also: &'static [&'static str],
    words: &'static str,
    about: &'static str,
) -> Term {
    Term {
        term,
        also,
        words,
        about,
    }
}

const TERMS: &[Term] = &[
    // The machine.
    t(
        "SNES",
        &[],
        "Super Nintendo Entertainment System",
        "Nintendo's 16-bit console, sold from 1990 in Japan as the Super Famicom.",
    ),
    t(
        "SFC",
        &[],
        "Super Famicom",
        "The Japanese SNES; `.sfc` is also the usual extension for a plain ROM file.",
    ),
    t(
        "SMC",
        &[],
        "Super Magicom",
        "A copier the `.smc` file extension is named after; some `.smc` files start with a 512-byte copier header before the ROM.",
    ),
    t(
        "CPU",
        &[],
        "Central Processing Unit",
        "The SNES's is the Ricoh 5A22: a 65C816 core with DMA, multiply and divide, timers and joypad reading built in. It runs at 3.58, 2.68 or 1.79 MHz depending on what it is reading.",
    ),
    t(
        "65C816",
        &["65816"],
        "WDC 65C816",
        "Western Design Center's 16-bit successor to the 6502, with 24-bit addresses and switchable 8- or 16-bit registers; the core of the SNES's CPU.",
    ),
    t(
        "PPU",
        &[],
        "Picture Processing Unit",
        "The two chips (5C77 and 5C78) that draw the picture line by line from VRAM, OAM and CGRAM. The CPU sets them up through the registers at $2100–$213F.",
    ),
    t(
        "APU",
        &[],
        "Audio Processing Unit",
        "The sound module: the SPC700 CPU, the S-DSP and 64 KB of ARAM. It runs its own program and talks to the main CPU only through four ports, $2140–$2143.",
    ),
    t(
        "SPC700",
        &[],
        "Sony SPC700 (the sound CPU)",
        "The 8-bit CPU in the APU, running at about 1.024 MHz. It runs the game's sound driver out of ARAM and tells the S-DSP what to play.",
    ),
    t(
        "DSP",
        &["S-DSP"],
        "Digital Signal Processor",
        "On the SNES, usually the S-DSP in the APU: it mixes eight voices of BRR samples, with pitch, envelopes, echo and noise, into 32 kHz stereo.",
    ),
    t(
        "DSP-1",
        &[],
        "Digital Signal Processor 1",
        "A maths coprocessor in some cartridges (an NEC µPD77C25) for 3D and Mode 7 projection, as in Pilotwings and Super Mario Kart. Not the S-DSP.",
    ),
    t(
        "SA-1",
        &[],
        "Super Accelerator 1",
        "A cartridge coprocessor: a second 65C816 at 10.74 MHz with its own memory mapping and DMA, as in Super Mario RPG.",
    ),
    t(
        "GSU",
        &["Super FX"],
        "Graphics Support Unit (the Super FX)",
        "A RISC coprocessor in the cartridge that draws polygons and bitmaps into the cartridge's RAM, which the game then copies to VRAM; Star Fox uses it.",
    ),
    t(
        "CIC",
        &[],
        "Checking Integrated Circuit",
        "The lockout chip: one in the console and one in the cartridge must agree, or the console holds the CPU in reset.",
    ),
    t(
        "MSU-1",
        &[],
        "Media Streaming Unit 1",
        "An enhancement chip defined for emulators and flash carts such as the SD2SNES, not by Nintendo; it streams CD-quality audio and data from files.",
    ),
    // Memories.
    t(
        "ROM",
        &[],
        "Read-Only Memory",
        "The cartridge's program and data. The CPU sees it through the mapping (LoROM, HiROM) the cartridge is wired for.",
    ),
    t(
        "RAM",
        &[],
        "Random-Access Memory",
        "Memory that can be read and written. The SNES has WRAM for the CPU, VRAM, OAM and CGRAM for the PPU, and ARAM for the sound CPU.",
    ),
    t(
        "WRAM",
        &[],
        "Work RAM",
        "The CPU's 128 KB of main RAM at $7E:0000–$7F:FFFF. Its first 8 KB also appears at $0000–$1FFF in banks $00–$3F and $80–$BF.",
    ),
    t(
        "SRAM",
        &[],
        "Static RAM (save RAM)",
        "Battery-backed RAM on the cartridge, where a game keeps its saves.",
    ),
    t(
        "VRAM",
        &[],
        "Video RAM",
        "The PPU's 64 KB, holding tiles and tilemaps, arranged as 32K 16-bit words. The CPU writes it through $2118/$2119 (VMDATA), normally only in vertical blank or forced blank.",
    ),
    t(
        "OAM",
        &[],
        "Object Attribute Memory",
        "The PPU's 544-byte sprite table: position, tile, palette, priority and flips for 128 sprites, plus a 32-byte high table with each sprite's X bit 8 and size.",
    ),
    t(
        "CGRAM",
        &[],
        "Color Generator RAM",
        "The PPU's 512-byte palette: 256 colours of 15 bits (5 each of blue, green and red), written two bytes at a time through $2122 (CGDATA).",
    ),
    t(
        "ARAM",
        &[],
        "Audio RAM",
        "The APU's 64 KB, holding the sound driver, the samples and the echo buffer. The main CPU cannot reach it; everything goes through the SPC700 and its four ports.",
    ),
    t(
        "IPL",
        &[],
        "Initial Program Loader",
        "The SPC700's 64-byte boot ROM at $FFC0. It signals the main CPU through the ports and waits for the sound driver to be uploaded into ARAM.",
    ),
    t(
        "MMIO",
        &[],
        "Memory-Mapped Input/Output",
        "Hardware registers read and written as if they were memory, such as the PPU's at $2100–$213F and the CPU's at $4200–$437F.",
    ),
    t(
        "I/O",
        &["IO"],
        "Input/Output",
        "Talking to hardware outside the CPU: the PPU, the APU, the joypads. On the SNES it is all MMIO.",
    ),
    t(
        "A-bus",
        &[],
        "Address bus A",
        "The CPU's main 24-bit address bus, reaching the ROM, WRAM and the cartridge. DMA copies between it and the B-bus.",
    ),
    t(
        "B-bus",
        &[],
        "Address bus B",
        "An 8-bit address bus reaching the registers at $2100–$21FF: the PPU, the APU's ports and WRAM's data port. DMA's destination (or source) is one of these.",
    ),
    t(
        "LoROM",
        &[],
        "Low ROM mapping (mode $20)",
        "The ROM is seen 32 KB at a time, in the upper half ($8000–$FFFF) of each bank. The header is at file offset $7FC0.",
    ),
    t(
        "HiROM",
        &[],
        "High ROM mapping (mode $21)",
        "The ROM is seen 64 KB at a time, whole banks at $C0–$FF, with the upper halves also at $8000–$FFFF of banks $00–$3F. The header is at file offset $FFC0.",
    ),
    t(
        "FastROM",
        &["SlowROM"],
        "Fast ROM access",
        "Reads of the ROM through banks $80–$FF take 6 master cycles instead of 8 when bit 0 of $420D (MEMSEL) is set, if the cartridge's ROM is fast enough.",
    ),
    // Moving bytes and timing.
    t(
        "DMA",
        &[],
        "Direct Memory Access",
        "Hardware copying between the A-bus (ROM or WRAM) and a B-bus register such as VRAM's port, about 8 master cycles a byte, while the CPU waits. Eight channels are set up at $43x0 and started by $420B (MDMAEN).",
    ),
    t(
        "HDMA",
        &[],
        "Horizontal-blank DMA",
        "The DMA channels writing a few bytes to PPU registers at the end of each scanline, from a table, for gradients, waves and window shapes. Started for the frame by $420C (HDMAEN).",
    ),
    t(
        "VBlank",
        &["V-blank", "vblank", "VBLANK"],
        "Vertical blank",
        "The lines after the picture (from line 225, or 240 with overscan, to the frame's end) when the PPU draws nothing, so VRAM, OAM and CGRAM can be written.",
    ),
    t(
        "HBlank",
        &["H-blank", "hblank", "HBLANK"],
        "Horizontal blank",
        "The short gap at the end of each scanline while the beam returns; HDMA writes its bytes here.",
    ),
    t(
        "NMI",
        &[],
        "Non-Maskable Interrupt",
        "On the SNES, the interrupt at the start of vertical blank, when bit 7 of $4200 (NMITIMEN) is set; the CPU's I flag cannot hold it off. Games do their VRAM and OAM updates in its handler.",
    ),
    t(
        "IRQ",
        &[],
        "Interrupt Request",
        "An interrupt the CPU's I flag can hold off. On the SNES it comes from the H/V timer ($4207–$420A, enabled in $4200) or a cartridge chip, for changes mid-screen.",
    ),
    t(
        "ISR",
        &[],
        "Interrupt Service Routine",
        "The code an interrupt vector points at, such as the NMI handler.",
    ),
    t(
        "NTSC",
        &[],
        "National Television System Committee",
        "The TV standard of North America and Japan: 60 frames a second of 262 lines.",
    ),
    t(
        "PAL",
        &[],
        "Phase Alternating Line",
        "The TV standard of Europe and Australia: 50 frames a second of 312 lines, so PAL games run slower unless they adjust.",
    ),
    t(
        "CRT",
        &[],
        "Cathode Ray Tube",
        "The TVs the SNES was made for; the beam's return at each line's end and the frame's end are the H-blank and V-blank.",
    ),
    // The picture.
    t(
        "BG",
        &["BG1", "BG2", "BG3", "BG4"],
        "Background",
        "One of up to four tile layers, BG1 to BG4; the mode in $2105 (BGMODE) sets how many there are and how many colours each has.",
    ),
    t(
        "OBJ",
        &[],
        "Object",
        "The SNES's word for a sprite: up to 128, described in OAM, drawn from tiles in VRAM.",
    ),
    t(
        "BPP",
        &["bpp", "2bpp", "4bpp", "8bpp"],
        "Bits per pixel",
        "A tile's colour depth: 2 bpp gives 4 colours, 4 bpp 16 and 8 bpp 256.",
    ),
    t(
        "Mode 7",
        &["mode 7"],
        "Background mode 7",
        "One 1024×1024-pixel background that the PPU rotates and scales with a matrix (M7A–M7D), line by line; the floor in F-Zero and Super Mario Kart.",
    ),
    // Sound.
    t(
        "BRR",
        &[],
        "Bit Rate Reduction",
        "The S-DSP's sample compression: 9-byte blocks, each a header byte (shift, filter, loop and end) and 16 four-bit samples.",
    ),
    t(
        "ADSR",
        &[],
        "Attack, Decay, Sustain, Release",
        "The shape of a voice's volume over a note, set per voice in the S-DSP's ADSR registers.",
    ),
    // The CPU's registers.
    t(
        "PC",
        &[],
        "Program Counter",
        "The address of the next instruction, within the bank in PBR.",
    ),
    t(
        "PBR",
        &["PB"],
        "Program Bank Register",
        "The bank the CPU is running code from; long jumps and calls change it.",
    ),
    t(
        "DBR",
        &["DB"],
        "Data Bank Register",
        "The bank absolute addresses such as `LDA $1234` read and write in; set with `PLB`.",
    ),
    t(
        "DP",
        &[],
        "Direct Page",
        "The D register's 256-byte window in bank $00 that short addresses such as `LDA $12` read, so often-used variables take fewer bytes and cycles.",
    ),
    t(
        "SP",
        &[],
        "Stack Pointer",
        "Where the next push goes, in bank $00; the stack grows down.",
    ),
    t(
        "BCD",
        &[],
        "Binary-Coded Decimal",
        "Each 4 bits holding a decimal digit. With the D flag set (`SED`), `ADC` and `SBC` add and subtract that way, which suits scores.",
    ),
    t(
        "LSB",
        &[],
        "Least Significant Byte (or bit)",
        "The low byte of a word; the 65816 stores it first.",
    ),
    t(
        "MSB",
        &[],
        "Most Significant Byte (or bit)",
        "The high byte of a word; the 65816 stores it second.",
    ),
];

/// Every entry: the general terms, then the hardware registers, then the
/// S-DSP's. A register whose name is already a term keeps the term.
pub fn entries() -> Vec<Entry> {
    let mut out: Vec<Entry> = TERMS
        .iter()
        .map(|t| Entry {
            term: t.term.to_string(),
            also: t.also.iter().map(|a| a.to_string()).collect(),
            words: t.words.to_string(),
            about: t.about.to_string(),
            kind: Kind::Term,
        })
        .collect();
    let taken = |out: &Vec<Entry>, name: &str| out.iter().any(|e| e.term == name);
    for r in all_hardware_registers() {
        if taken(&out, r.name) {
            continue;
        }
        out.push(Entry {
            term: r.name.to_string(),
            also: Vec::new(),
            words: r.description.to_string(),
            about: {
                let access = match r.access.as_str() {
                    "R" => "read only",
                    "W" => "write only",
                    _ => "read and write",
                };
                match fields::layout(r.address) {
                    Some(l) if !l.about.is_empty() => {
                        format!(
                            "{} At ${:04X}, {access}.",
                            first_sentence(l.about),
                            r.address
                        )
                    }
                    _ => format!("The hardware register at ${:04X}, {access}.", r.address),
                }
            },
            kind: Kind::Register,
        });
    }
    for reg in 0..0x80u8 {
        let name = sound::dsp_register_name(reg);
        if name.is_empty() || taken(&out, &name) {
            continue;
        }
        let layout = sound::dsp_layout(reg);
        out.push(Entry {
            term: name,
            also: Vec::new(),
            words: format!("S-DSP register ${reg:02X}"),
            about: layout.about.to_string(),
            kind: Kind::DspRegister,
        });
    }
    out
}

/// Up to the first full stop that ends a sentence.
fn first_sentence(text: &str) -> &str {
    let b = text.as_bytes();
    (0..b.len().saturating_sub(2))
        .find(|&i| b[i] == b'.' && b[i + 1] == b' ' && b[i + 2].is_ascii_uppercase())
        .map_or(text, |i| &text[..=i])
}

/// The entry for a term or one of its other spellings.
pub fn lookup(term: &str) -> Option<Entry> {
    let t = term.trim();
    let all = entries();
    all.iter()
        .find(|e| e.term == t || e.also.iter().any(|a| a == t))
        .or_else(|| all.iter().find(|e| e.term.eq_ignore_ascii_case(t)))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_is_spelt_out_and_short() {
        let all = entries();
        assert!(all.len() > 100, "{}", all.len());
        for e in &all {
            assert!(!e.words.is_empty(), "{}", e.term);
            assert!(!e.about.is_empty(), "{}", e.term);
            let sentences = e.about.matches(". ").count() + 1;
            assert!(sentences <= 2, "{}: {}", e.term, e.about);
        }
        let mut names: Vec<&str> = all
            .iter()
            .flat_map(|e| std::iter::once(e.term.as_str()).chain(e.also.iter().map(String::as_str)))
            .collect();
        let n = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), n, "a spelling is in two entries");
    }

    #[test]
    fn terms_registers_and_dsp_registers_are_found() {
        let dma = lookup("DMA").unwrap();
        assert_eq!(dma.words, "Direct Memory Access");
        assert_eq!(lookup("vblank").unwrap().term, "VBlank");
        let vmain = lookup("VMAIN").unwrap();
        assert_eq!(vmain.kind, Kind::Register);
        assert!(
            vmain.about.starts_with("How the VRAM address moves on")
                && vmain.about.ends_with("At $2115, write only."),
            "{}",
            vmain.about
        );
        let kon = lookup("KON").unwrap();
        assert_eq!(kon.kind, Kind::DspRegister);
        assert_eq!(kon.words, "S-DSP register $4C");
        assert!(lookup("NOT_A_TERM").is_none());
    }
}
