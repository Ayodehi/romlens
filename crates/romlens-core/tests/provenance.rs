//! Where a byte of VRAM came from (docs/22, P3), end to end: a recorder
//! stream with line writes and a DMA, packed, then traced back.

use std::io::Cursor;

use romlens_core::RomImage;
use romlens_core::SnesAddress;
use romlens_core::fixtures;
use romlens_core::provenance::{Confidence, Target, Writer, last_write};
use romlens_core::recording::lines::{Memory, RegWrite};
use romlens_core::recording::mesen::stream::{
    DMA_LEN, DmaContext, DmaEvent, FrameRecord, LineRecord, PPU_PORTS, Record, SAMPLE_LEN, SAMPLES,
    STREAM_VERSION, StreamHeader, encode,
};
use romlens_core::recording::mesen::{PackOptions, pack};
use romlens_core::recording::{MachineStateSource, RomrecSource};

fn w(line: i16, dot: u16, reg: u8, value: u8) -> RegWrite {
    RegWrite {
        line,
        dot,
        reg,
        value,
    }
}

fn frame(n: u32) -> Record {
    Record::Frame(Box::new(FrameRecord {
        frame: n,
        fields: Vec::new(),
        ports: [0; PPU_PORTS],
        seen: [false; PPU_PORTS],
        dma: [0; DMA_LEN],
        blocks: Default::default(),
    }))
}

/// Three frames. Frame 1: the CPU writes VRAM word $0100 itself, then
/// channel 2 copies 4 bytes from `$81:9000` to word $0200 (mode 1, to
/// VMDATA). Frame 2 writes nothing.
fn recording() -> RomrecSource {
    let rom = RomImage::from_bytes(fixtures::minimal_lorom(), "t.sfc").unwrap();
    let bytes = rom.bytes();
    let size = bytes.len();
    let samples = (0..SAMPLES)
        .flat_map(|i| {
            let at = (size / SAMPLES) * i;
            bytes[at..at + SAMPLE_LEN].to_vec()
        })
        .collect();
    let head = StreamHeader {
        version: STREAM_VERSION,
        producer: "Mesen".to_owned(),
        rom_sha1: String::new(),
        created: 0,
        rom_size: size as u32,
        samples,
        fields: Vec::new(),
        flags: 0,
    };
    let mut registers = [0u8; DMA_LEN];
    let ch = 2 * 16;
    registers[ch] = 0x01; // mode 1
    registers[ch + 1] = 0x18; // VMDATAL
    registers[ch + 2..ch + 4].copy_from_slice(&0x9000u16.to_le_bytes());
    registers[ch + 4] = 0x81;
    registers[ch + 5..ch + 7].copy_from_slice(&4u16.to_le_bytes());
    let mut s = encode::header(&head);
    s.extend(encode::record(&frame(0)));
    s.extend(encode::record(&Record::Dma(Box::new(DmaEvent {
        frame: 1,
        value: 0x04,
        scanline: 232,
        registers,
        context: Some(DmaContext {
            h_clock: 400,
            vram_address: 0x0200,
            k: 0x80,
            pc: 0x8123,
            ..DmaContext::default()
        }),
    }))));
    s.extend(encode::record(&Record::Lines(Box::new(LineRecord {
        frame: 1,
        writes: vec![
            // VMAIN: increment after the high byte.
            w(-35, 0, 0x15, 0x80),
            w(-35, 10, 0x16, 0x00),
            w(-35, 20, 0x17, 0x01),
            w(-35, 30, 0x18, 0x12),
            w(-35, 40, 0x19, 0x34),
            w(-30, 50, 0x16, 0x00),
            w(-30, 60, 0x17, 0x02),
            // The DMA's four bytes, just after it started.
            w(-30, 101, 0x18, 0xA0),
            w(-30, 103, 0x19, 0xA1),
            w(-30, 105, 0x18, 0xA2),
            w(-30, 107, 0x19, 0xA3),
        ],
    }))));
    s.extend(encode::record(&frame(1)));
    s.extend(encode::record(&Record::Lines(Box::new(LineRecord {
        frame: 2,
        writes: Vec::new(),
    }))));
    s.extend(encode::record(&frame(2)));
    s.extend(encode::record(&Record::End { frames: 3 }));
    let mut out = Cursor::new(Vec::new());
    pack(Cursor::new(s), &rom, &mut out, PackOptions::default()).unwrap();
    RomrecSource::from_bytes(out.into_inner(), false).unwrap()
}

#[test]
fn a_dma_byte_names_its_channel_and_its_source() {
    let rec = recording();
    assert_eq!(rec.frame_count(), Some(3));
    // Word $0201's high byte: the DMA's fourth byte, from $81:9003.
    let t = Target {
        memory: Memory::Vram,
        byte: 0x0201 * 2 + 1,
    };
    let f = last_write(&rec, 2, t, 0).unwrap().unwrap();
    assert_eq!((f.frame, f.value), (1, 0xA3));
    let Writer::Dma {
        byte, transfer, pc, ..
    } = f.writer
    else {
        panic!("{:?}", f.writer)
    };
    assert_eq!((byte.channel, byte.index), (2, 3));
    assert_eq!(byte.source, SnesAddress::new(0x81, 0x9003));
    assert_eq!(byte.confidence, Confidence::Exact);
    assert_eq!(transfer.bytes, 4);
    assert_eq!(pc, Some(SnesAddress::new(0x80, 0x8123)));
    // And the replay put the byte there.
    let (_, vram, ..) = romlens_core::recording::lines::frame_replay(&rec, 1)
        .unwrap()
        .unwrap()
        .finish();
    assert_eq!(vram[0x403], 0xA3);
}

#[test]
fn a_cpu_write_is_the_cpus_and_nothing_before_is_found() {
    let rec = recording();
    let t = Target {
        memory: Memory::Vram,
        byte: 0x0100 * 2,
    };
    let f = last_write(&rec, 2, t, 0).unwrap().unwrap();
    assert_eq!((f.frame, f.value, f.writer), (1, 0x12, Writer::Cpu));
    // A byte no write reached.
    let untouched = Target {
        memory: Memory::Vram,
        byte: 0x8000,
    };
    assert_eq!(last_write(&rec, 2, untouched, 0).unwrap(), None);
}
