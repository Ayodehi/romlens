import Foundation

let path = CommandLine.arguments[1]
guard let rom = spike_open(path) else { fatalError("open failed") }
let size = spike_size(rom)
let totalRows = (size + 15) / 16
let batch = 200
let batches = (totalRows + batch - 1) / batch
print("ROM \(size) bytes, \(totalRows) rows, \(batches) batches of \(batch)")

func time(_ label: String, _ body: () -> Int) {
    let t0 = DispatchTime.now().uptimeNanoseconds
    let n = body()
    let dt = Double(DispatchTime.now().uptimeNanoseconds - t0)
    let perRow = dt / Double(totalRows)
    let perBatch = dt / Double(batches)
    print(String(format: "%-46@ total %7.1f ms   per batch %7.1f µs   per row %6.1f ns   (checksum %d)", label as NSString, dt/1e6, perBatch/1e3, perRow, n))
}

// 1. chatty: one FFI call per byte (what a naive per-cell API would do)
time("chatty: spike_byte per byte") {
    var acc = 0
    for off in 0..<size { acc &+= Int(spike_byte(rom, off)) }
    return acc
}

// 2. chunky into a caller-owned buffer (zero allocation on the Rust side)
time("chunky: rows_into caller buffer") {
    var acc = 0
    var buf = [RowRec](repeating: RowRec(), count: batch)
    for b in 0..<batches {
        let n = buf.withUnsafeMutableBufferPointer { spike_rows_into(rom, b*batch, batch, $0.baseAddress) }
        for i in 0..<n { acc &+= Int(buf[i].bytes.0) &+ Int(buf[i].file_offset & 0xFF) }
    }
    return acc
}

// 3. chunky with a Rust-owned buffer returned and freed (binding-generator style)
time("chunky: rows_alloc + free (Rust-owned)") {
    var acc = 0
    for b in 0..<batches {
        let rb = spike_rows_alloc(rom, b*batch, batch)
        for i in 0..<rb.len { acc &+= Int(rb.ptr[i].bytes.0) }
        spike_rows_free(rb)
    }
    return acc
}

// 4. chunky + shell formats every row into a Swift String (the real table cost)
time("chunky + Swift formats hex strings per row") {
    var acc = 0
    var buf = [RowRec](repeating: RowRec(), count: batch)
    for b in 0..<batches {
        let n = buf.withUnsafeMutableBufferPointer { spike_rows_into(rom, b*batch, batch, $0.baseAddress) }
        for i in 0..<n {
            let rec = buf[i]
            var s = String(format: "%06X  ", rec.file_offset)
            withUnsafeBytes(of: rec.bytes) { p in for k in 0..<16 { s += String(format: "%02X ", p[k]) } }
            acc &+= s.utf8.count
        }
    }
    return acc
}

// 5. core pre-formats text; shell makes one String per batch
time("core formats text, one String per batch") {
    var acc = 0
    for b in 0..<batches {
        var len = 0
        let p = spike_rows_text(rom, b*batch, batch, &len)!
        let s = String(decoding: UnsafeBufferPointer(start: p, count: len), as: UTF8.self)
        acc &+= s.utf8.count
        spike_text_free(p, len)
    }
    return acc
}
spike_close(rom)
