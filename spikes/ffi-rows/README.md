# FFI row-batch spike

Two tiny crates that answer one question: is the FFI between a Rust core and
a Swift shell fast enough for a 120 Hz hex table over a 3 MB ROM?

```
c-abi/    hand-written extern "C" functions, staticlib, Swift bench via bridging header
uniffi/   the same functions through UniFFI 0.31 proc-macros, generated Swift bindings
```

Run (needs cargo and swiftc):

```
cd c-abi && cargo build --release && \
  swiftc -O bench.swift -import-objc-header spike.h -L target/release -lffispike -o bench && \
  ./bench /path/to/rom.sfc

cd uniffi && cargo build --release && \
  cargo run --release --bin uniffi-bindgen -- generate --library target/release/libuffispike.dylib --language swift --out-dir out && \
  swiftc -O main.swift out/uffispike.swift -import-objc-header out/uffispikeFFI.h -L target/release -luffispike -o bench && \
  ./bench /path/to/rom.sfc
```

Results and conclusions are in `docs/10-ffi-spike.md`.
