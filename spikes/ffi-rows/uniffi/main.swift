import Foundation
let rom = Rom.open(path: CommandLine.arguments[1])
let size = Int(rom.size()); let totalRows = (size + 15) / 16; let batch = 200; let batches = (totalRows + batch - 1) / batch
print("UniFFI: ROM \(size) bytes, \(totalRows) rows, \(batches) batches of \(batch)")
func time(_ label: String, _ body: () -> Int) {
    let t0 = DispatchTime.now().uptimeNanoseconds; let n = body()
    let dt = Double(DispatchTime.now().uptimeNanoseconds - t0)
    print(String(format: "%-46@ total %7.1f ms   per batch %7.1f µs   per row %6.1f ns   (checksum %d)", label as NSString, dt/1e6, dt/Double(batches)/1e3, dt/Double(totalRows), n))
}
time("uniffi: Vec<HexRow> records") { var acc = 0; for b in 0..<batches { let rows = rom.hexRows(startRow: UInt32(b*batch), count: UInt32(batch)); for r in rows { acc &+= Int(r.bytes[0]) &+ r.ascii.utf8.count } }; return acc }
time("uniffi: flat Vec<u8> blob per batch") { var acc = 0; for b in 0..<batches { let blob = rom.hexRowsBlob(startRow: UInt32(b*batch), count: UInt32(batch)); let n = blob.count / 36; for i in 0..<n { acc &+= Int(blob[i*36 + 4]) } }; return acc }
time("uniffi: core-formatted String per batch") { var acc = 0; for b in 0..<batches { let s = rom.hexRowsText(startRow: UInt32(b*batch), count: UInt32(batch)); acc &+= s.utf8.count }; return acc }
